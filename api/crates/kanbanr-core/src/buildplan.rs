//! Build co-ordination on one machine (FTR-062): the pure part. What a project's build settings
//! resolve to, where machine and repository build state lives, the order waiting builds take a
//! slot in, and the build log's rows.
//!
//! Nothing here touches the board: slots, tickets, caches and the build log live on the machine
//! (its cache folder) and in the repository's git directory, never in the data folder. The CLI's
//! `buildrun` holds the locks and runs the commands.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::BuildConfig;

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/// The build settings a project's builds use: its [`BuildConfig`] with every absent value filled.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Effective {
    pub slots: u32,
    pub jobs: u32,
    pub reserve_cpus: u32,
    pub reserve_mem_gib: u32,
    pub max_load: f64,
    pub sccache: bool,
    pub sccache_port: u16,
    pub cache_gib: u32,
    pub mold: bool,
    pub web_cache: bool,
    pub queue_batch: u32,
    pub line_check: Option<Vec<String>>,
}

/// Jobs per slot when the project sets none.
pub const DEFAULT_JOBS_PER_SLOT: u32 = 5;
/// vCPUs kept free for the desktop when the project sets none (the person's rule).
pub const DEFAULT_RESERVE_CPUS: u32 = 2;
/// GiB of available memory a build waits for when the project sets none.
pub const DEFAULT_RESERVE_MEM_GIB: u32 = 6;
/// The project's compiler cache in GiB when the project sets none.
pub const DEFAULT_CACHE_GIB: u32 = 10;
/// Branches in one line check when the project sets none.
pub const DEFAULT_QUEUE_BATCH: u32 = 4;
/// The first port a project's sccache server may get; ports run over [`PORT_SPAN`] from here.
pub const PORT_BASE: u16 = 4300;
/// How many ports the derived sccache port is spread over.
pub const PORT_SPAN: u64 = 600;

impl BuildConfig {
    /// The settings a build on a machine with `cpus` vCPUs uses, for the project whose main
    /// checkout is `main_root` (which derives the sccache port). The reserve is capped so at least
    /// one vCPU builds; jobs are capped at the vCPUs left after the reserve, so the reserve holds
    /// whatever was configured.
    pub fn effective(&self, cpus: u32, main_root: &Path) -> Effective {
        let cpus = cpus.max(1);
        let reserve_cpus = self
            .reserve_cpus
            .unwrap_or(DEFAULT_RESERVE_CPUS)
            .min(cpus - 1);
        let usable = cpus - reserve_cpus;
        let jobs = self
            .jobs_per_slot
            .unwrap_or(DEFAULT_JOBS_PER_SLOT)
            .clamp(1, usable);
        let slots = self.slots.unwrap_or_else(|| (usable / jobs).max(1)).max(1);
        Effective {
            slots,
            jobs,
            reserve_cpus,
            reserve_mem_gib: self.reserve_mem_gib.unwrap_or(DEFAULT_RESERVE_MEM_GIB),
            max_load: self.max_load.unwrap_or(f64::from(usable)),
            sccache: self.sccache.as_deref() != Some("off"),
            sccache_port: self.sccache_port.unwrap_or_else(|| derived_port(main_root)),
            cache_gib: self.cache_gib.unwrap_or(DEFAULT_CACHE_GIB),
            mold: self.linker.as_deref() == Some("mold"),
            web_cache: self.web_cache.unwrap_or(true),
            queue_batch: self.queue_batch.unwrap_or(DEFAULT_QUEUE_BATCH).max(1),
            line_check: self.line_check.clone(),
        }
    }

    /// How many builds may run at once on a machine with `cpus` vCPUs (FTR-061 reads it).
    pub fn effective_slots(&self, cpus: u32) -> u32 {
        self.effective(cpus, Path::new("")).slots
    }
}

/// The sccache port a project gets when it configures none: FNV-1a of its main checkout's path,
/// spread over [`PORT_SPAN`] ports from [`PORT_BASE`], so two projects rarely share one.
fn derived_port(main_root: &Path) -> u16 {
    let hex = crate::hash::stable_hash(&main_root.to_string_lossy());
    let h = u64::from_str_radix(&hex, 16).unwrap_or(0);
    PORT_BASE + (h % PORT_SPAN) as u16
}

// ---------------------------------------------------------------------------------------------
// Where build state lives
// ---------------------------------------------------------------------------------------------

/// The machine's build folder: slots, tickets, the web cache and the sccache caches.
/// `$KANBANR_BUILD_DIR`, else `$XDG_CACHE_HOME/kanbanr/build`, else the platform's cache folder:
/// `~/.cache` (Linux and other Unix), `~/Library/Caches` (macOS), `%LOCALAPPDATA%` (Windows).
pub fn machine_dir() -> PathBuf {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(d) = var("KANBANR_BUILD_DIR") {
        return d;
    }
    if let Some(d) = var("XDG_CACHE_HOME") {
        return d.join("kanbanr").join("build");
    }
    let platform = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        crate::project::home_dir().map(|h| h.join("Library").join("Caches"))
    } else {
        crate::project::home_dir().map(|h| h.join(".cache"))
    };
    platform
        .unwrap_or_else(std::env::temp_dir)
        .join("kanbanr")
        .join("build")
}

/// The repository's build folder inside its git directory: the build log, the merge queue.
pub fn repo_dir(git_common_dir: &Path) -> PathBuf {
    git_common_dir.join("kanbanr")
}

// ---------------------------------------------------------------------------------------------
// Waiting for a slot
// ---------------------------------------------------------------------------------------------

/// Which waiting build takes the next free slot: a task on the critical path, then any other task,
/// then a command for no task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    Critical = 0,
    Task = 1,
    None = 2,
}

/// A waiting build's ticket file name: `<priority>-<arrival ns, 39 digits>-<pid>.ticket`. The
/// arrival is zero-padded to the width of `u128::MAX`, so names sort by priority, then arrival.
pub fn ticket_name(p: Priority, arrival_ns: u128, pid: u32) -> String {
    format!("{}-{arrival_ns:039}-{pid}.ticket", p as u8)
}

/// Whether the build holding ticket `own` may take a slot now: fewer than `free_slots` live
/// tickets sort before it.
pub fn may_take_slot(own: &str, live: &[String], free_slots: usize) -> bool {
    let ahead = live
        .iter()
        .filter(|t| t.as_str() != own && t.as_str() < own)
        .count();
    ahead < free_slots
}

// ---------------------------------------------------------------------------------------------
// The build log
// ---------------------------------------------------------------------------------------------

/// One command run through `kanbanr build run`: one line of the build log.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildRow {
    /// When the command was asked for (RFC 3339, UTC); its date names the day's file.
    pub at: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub branch: String,
    /// The task it ran for, `CODE/TL/T`.
    #[serde(default)]
    pub task: Option<String>,
    /// The agent scope it ran under, `SC-n`.
    #[serde(default)]
    pub scope: Option<String>,
    /// `build`, `check`, `test` or `bench`.
    #[serde(default)]
    pub kind: String,
    /// `task`, `merge` or `line`, for a check.
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub slot: u32,
    #[serde(default)]
    pub queued_ms: u64,
    #[serde(default)]
    pub run_ms: u64,
    #[serde(default)]
    pub exit: i32,
    #[serde(default)]
    pub cache_hits: Option<u64>,
    #[serde(default)]
    pub cache_misses: Option<u64>,
    /// The machine's available memory when the command started (Linux).
    #[serde(default)]
    pub mem_available_mib: Option<u64>,
    #[serde(default)]
    pub note: String,
}

/// What a task's builds cost, summed from the build log (FTR-061 copies it to the task log).
/// FTR-061 moves this type to `tasklog`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildTotals {
    pub runs: u32,
    pub red: u32,
    pub run_ms: u64,
    pub wait_ms: u64,
}

/// The build log's folder inside the repository's build folder.
const BUILDS: &str = "builds";

/// The day a row belongs to: the date of its `at`, else today (UTC).
fn day_of(at: &str) -> String {
    let d = at.get(..10).unwrap_or("");
    let b = d.as_bytes();
    let is_date = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if is_date {
        d.to_string()
    } else {
        crate::now_rfc3339().chars().take(10).collect()
    }
}

/// Append `row` as one JSON line to its day's file, `<repo_dir>/builds/<YYYY-MM-DD>.jsonl`,
/// creating the folders. The log is never trimmed.
pub fn append_row(repo_dir: &Path, row: &BuildRow) -> std::io::Result<()> {
    let dir = repo_dir.join(BUILDS);
    std::fs::create_dir_all(&dir)?;
    let mut line = serde_json::to_string(row).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{}.jsonl", day_of(&row.at))))?;
    // One write per row, so concurrent appenders never interleave inside a line.
    f.write_all(line.as_bytes())
}

/// Every row of the build log, oldest day first and in written order within a day. A line that
/// does not parse is skipped; a missing log is empty.
pub fn read_rows(repo_dir: &Path) -> Vec<BuildRow> {
    let Ok(entries) = std::fs::read_dir(repo_dir.join(BUILDS)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|l| serde_json::from_str::<BuildRow>(l).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What `task`'s builds since `since` cost: how many ran, how many ended non-zero, and their run
/// and wait time. `since` empty counts every row.
pub fn task_totals(rows: &[BuildRow], task: &str, since: &str) -> BuildTotals {
    rows.iter()
        .filter(|r| r.task.as_deref() == Some(task))
        .filter(|r| since.is_empty() || crate::cmp_instants(&r.at, since).is_ge())
        .fold(BuildTotals::default(), |mut t, r| {
            t.runs += 1;
            t.red += u32::from(r.exit != 0);
            t.run_ms += r.run_ms;
            t.wait_ms += r.queued_ms;
            t
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-buildplan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_slots_leave_two_vcpus_free() {
        let none = BuildConfig::default();
        let root = Path::new("/work/shop");

        let e = none.effective(12, root);
        assert_eq!((e.slots, e.jobs, e.reserve_cpus), (2, 5, 2));
        assert_eq!(e.max_load, 10.0);
        assert_eq!(e.reserve_mem_gib, 6);
        assert_eq!(e.cache_gib, 10);
        assert_eq!(e.queue_batch, 4);
        assert!(e.sccache && e.web_cache && !e.mold);
        assert_eq!(e.line_check, None);
        assert!((PORT_BASE..PORT_BASE + PORT_SPAN as u16).contains(&e.sccache_port));
        assert_eq!(
            e.sccache_port,
            none.effective(4, root).sccache_port,
            "the port follows the root"
        );
        assert_eq!(none.effective_slots(12), 2);

        let e = none.effective(4, root);
        assert_eq!((e.slots, e.jobs, e.reserve_cpus), (1, 2, 2));
        assert_eq!(e.max_load, 2.0);

        let e = none.effective(1, root);
        assert_eq!((e.slots, e.jobs, e.reserve_cpus), (1, 1, 0));
        // An unknown count is one vCPU.
        assert_eq!(none.effective(0, root).slots, 1);

        let set = BuildConfig {
            slots: Some(3),
            jobs_per_slot: Some(3),
            reserve_cpus: Some(1),
            reserve_mem_gib: Some(10),
            max_load: Some(7.5),
            sccache: Some("off".into()),
            sccache_port: Some(4999),
            cache_gib: Some(20),
            linker: Some("mold".into()),
            web_cache: Some(false),
            queue_batch: Some(2),
            line_check: Some(vec!["make".into(), "ci".into()]),
            ..Default::default()
        };
        let e = set.effective(12, root);
        assert_eq!((e.slots, e.jobs, e.reserve_cpus), (3, 3, 1));
        assert_eq!(e.reserve_mem_gib, 10);
        assert_eq!(e.max_load, 7.5);
        assert!(!e.sccache && !e.web_cache && e.mold);
        assert_eq!((e.sccache_port, e.cache_gib, e.queue_batch), (4999, 20, 2));
        assert_eq!(
            e.line_check,
            Some(vec!["make".to_string(), "ci".to_string()])
        );
        assert_eq!(set.effective_slots(12), 3);

        // A reserve alone moves the slots; `auto` and `default` are the defaults spelled out.
        let reserve = BuildConfig {
            reserve_cpus: Some(4),
            sccache: Some("auto".into()),
            linker: Some("default".into()),
            ..Default::default()
        };
        let e = reserve.effective(12, root);
        assert_eq!((e.slots, e.jobs, e.reserve_cpus), (1, 5, 4));
        assert!(e.sccache && !e.mold);
    }

    #[test]
    fn waiting_jobs_are_served_by_priority_then_arrival() {
        let task_first = ticket_name(Priority::Task, 9, 300);
        let critical_later = ticket_name(Priority::Critical, 10, 100);
        let task_later = ticket_name(Priority::Task, 1_000_000_000_000, 200);
        let none_first = ticket_name(Priority::None, 1, 400);
        assert_eq!(
            critical_later,
            format!("0-{}10-100.ticket", "0".repeat(37)),
            "zero-padded to 39 digits"
        );
        assert_eq!(
            ticket_name(Priority::None, u128::MAX, 1).len(),
            "2-".len() + 39 + "-1.ticket".len()
        );

        let mut live = vec![
            none_first.clone(),
            task_later.clone(),
            critical_later.clone(),
            task_first.clone(),
        ];
        live.sort();
        assert_eq!(
            live,
            vec![
                critical_later.clone(),
                task_first.clone(),
                task_later.clone(),
                none_first.clone()
            ]
        );

        // One free slot: only the head may take it.
        assert!(may_take_slot(&critical_later, &live, 1));
        assert!(!may_take_slot(&task_first, &live, 1));
        assert!(!may_take_slot(&none_first, &live, 1));
        // Two free slots: the first two.
        assert!(may_take_slot(&task_first, &live, 2));
        assert!(!may_take_slot(&task_later, &live, 2));
        // No free slot: nobody.
        assert!(!may_take_slot(&critical_later, &live, 0));
        // A ticket not yet listed still counts the ones ahead of it.
        let mine = ticket_name(Priority::Task, 50, 500);
        assert!(!may_take_slot(&mine, &live, 2));
        assert!(may_take_slot(&mine, &live, 3));
    }

    #[test]
    fn build_rows_are_appended_and_read_back_by_task() {
        let dir = scratch("rows");
        let row = |at: &str, task: Option<&str>, exit: i32, run: u64, queued: u64| BuildRow {
            at: at.into(),
            worktree: "/w/ftr-1".into(),
            branch: "feat/FTR-1".into(),
            task: task.map(String::from),
            kind: "check".into(),
            cmd: vec!["cargo".into(), "test".into()],
            slot: 1,
            queued_ms: queued,
            run_ms: run,
            exit,
            ..Default::default()
        };
        let t = "FTR-1/TL-001/T1";
        // Appended out of day order: read back oldest day first.
        append_row(&dir, &row("2026-11-17T09:00:00Z", Some(t), 0, 1_000, 0)).unwrap();
        append_row(&dir, &row("2026-11-16T08:00:00Z", Some(t), 1, 500, 200)).unwrap();
        append_row(&dir, &row("2026-11-17T10:00:00Z", Some(t), 101, 2_000, 50)).unwrap();
        append_row(
            &dir,
            &row("2026-11-17T11:00:00Z", Some("FTR-2/TL-001/T1"), 0, 9_000, 9),
        )
        .unwrap();
        append_row(&dir, &row("2026-11-17T12:00:00Z", None, 0, 7_000, 7)).unwrap();
        assert!(dir.join("builds").join("2026-11-16.jsonl").is_file());
        assert!(dir.join("builds").join("2026-11-17.jsonl").is_file());

        // A bad line is skipped, the rest still read.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("builds").join("2026-11-17.jsonl"))
            .unwrap();
        writeln!(f, "{{not json").unwrap();
        drop(f);
        append_row(&dir, &row("2026-11-17T13:00:00Z", Some(t), 0, 10, 0)).unwrap();

        let rows = read_rows(&dir);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].at, "2026-11-16T08:00:00Z");
        assert_eq!(rows[0], row("2026-11-16T08:00:00Z", Some(t), 1, 500, 200));
        assert_eq!(rows[5].at, "2026-11-17T13:00:00Z");

        assert_eq!(
            task_totals(&rows, t, ""),
            BuildTotals {
                runs: 4,
                red: 2,
                run_ms: 3_510,
                wait_ms: 250
            }
        );
        assert_eq!(
            task_totals(&rows, t, "2026-11-17T09:00:00Z"),
            BuildTotals {
                runs: 3,
                red: 1,
                run_ms: 3_010,
                wait_ms: 50
            }
        );
        assert_eq!(
            task_totals(&rows, "FTR-9/TL-001/T1", ""),
            BuildTotals::default()
        );
        assert!(read_rows(&dir.join("nothing-here")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
