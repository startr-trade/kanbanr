//! Build co-ordination on one machine (FTR-062): the pure part. What a project's build settings
//! resolve to, where machine and repository build state lives, the order waiting builds take a
//! slot in, and the build log's rows.
//!
//! Nothing here touches the board: slots, tickets, caches and the build log live on the machine
//! (its cache folder) and in the repository's git directory, never in the data folder. The CLI's
//! `buildrun` holds the locks and runs the commands.

use std::collections::BTreeSet;
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

// ---------------------------------------------------------------------------------------------
// The build environment and the compiler cache
// ---------------------------------------------------------------------------------------------

/// The environment a build runs with, and what it had to go without.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BuildEnv {
    /// Variables set on the command, in this order.
    pub set: Vec<(String, String)>,
    /// A tool the settings ask for that is not here, in words.
    pub notes: Vec<String>,
}

/// The rustflags variables `mold` is set through, one per Linux target it is used on.
const MOLD_VARS: &[&str] = &[
    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
    "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
];

/// The environment a build gets from settings `e`: always the job and test-thread caps; the
/// project's sccache server when sccache is on and found (`cache_dir` its cache); mold when it is
/// configured, found and this is Linux; the cached web bundle when there is one (`ui_dist`).
pub fn build_env(
    e: &Effective,
    sccache_found: bool,
    mold_found: bool,
    linux: bool,
    cache_dir: &Path,
    ui_dist: Option<&Path>,
) -> BuildEnv {
    let mut env = BuildEnv::default();
    let mut set = |k: &str, v: String| env.set.push((k.to_string(), v));
    let jobs = e.jobs.to_string();
    set("CARGO_BUILD_JOBS", jobs.clone());
    set("NEXTEST_TEST_THREADS", jobs.clone());
    set("RUST_TEST_THREADS", jobs);
    if e.sccache && sccache_found {
        set("RUSTC_WRAPPER", "sccache".into());
        set("CARGO_INCREMENTAL", "0".into());
        set("SCCACHE_SERVER_PORT", e.sccache_port.to_string());
        set("SCCACHE_DIR", cache_dir.display().to_string());
        set("SCCACHE_CACHE_SIZE", format!("{}G", e.cache_gib));
    }
    if e.mold && mold_found && linux {
        for k in MOLD_VARS {
            set(k, "-C link-arg=-fuse-ld=mold".into());
        }
    }
    if let Some(d) = ui_dist {
        set("KANBANR_UI_DIST", d.display().to_string());
    }
    if e.sccache && !sccache_found {
        env.notes
            .push("sccache not found: compiling without the shared cache".into());
    }
    if e.mold && !linux {
        env.notes
            .push("mold is used on Linux only: linking with the default linker".into());
    } else if e.mold && !mold_found {
        env.notes
            .push("mold not found: linking with the default linker".into());
    }
    env
}

/// The project's sccache cache folder on this machine: `<machine>/sccache/<project>-<hash8>`,
/// the hash of its main checkout's path, so two checkouts of one project keep their own.
pub fn sccache_dir(machine: &Path, project: &str, main_root: &Path) -> PathBuf {
    let hex = crate::hash::stable_hash(&main_root.to_string_lossy());
    let short = hex.get(..8).unwrap_or(&hex);
    machine.join("sccache").join(format!("{project}-{short}"))
}

/// The sccache server's base directories: the main checkout and every worktree, absolute (a
/// relative worktree is taken from `main_root`), sorted, each once. With them a path inside any
/// checkout hashes the same, so one worktree's compile serves the others.
pub fn base_dirs(main_root: &Path, worktrees: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::iter::once(main_root.to_path_buf())
        .chain(worktrees.iter().map(|w| main_root.join(w)))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// What to do about the project's cache server before a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Restart {
    /// It runs with the base directories wanted.
    No,
    /// Start it, or restart it with the base directories wanted.
    Yes,
    /// Its base directories are out of date, but another build is using it: leave it.
    Deferred,
}

/// Whether the cache server must be (re)started: none running (`running` is `None`) → `Yes`;
/// running with the base directories `wanted` → `No`; with others → `Yes`, or `Deferred` while
/// another build uses it.
pub fn needs_restart(
    running: Option<&[PathBuf]>,
    wanted: &[PathBuf],
    others_building: bool,
) -> Restart {
    let Some(running) = running else {
        return Restart::Yes;
    };
    let set = |d: &[PathBuf]| d.iter().cloned().collect::<BTreeSet<PathBuf>>();
    if set(running) == set(wanted) {
        Restart::No
    } else if others_building {
        Restart::Deferred
    } else {
        Restart::Yes
    }
}

/// The value of the line `label` of `sccache --show-stats`: the text after the label and its
/// padding.
pub fn stats_value<'a>(stats: &'a str, label: &str) -> Option<&'a str> {
    stats.lines().find_map(|l| {
        let rest = l.strip_prefix(label)?;
        // `Cache size` must not match `Cache size (…)`-like longer labels.
        rest.starts_with(char::is_whitespace).then(|| rest.trim())
    })
}

/// The base directories a running sccache server reports in `sccache --show-stats` (0.17: one
/// `Base directories` line, comma-separated, each with a trailing `/`; `(none)` for none).
/// `None` when the text has no such line.
pub fn parse_base_dirs(stats: &str) -> Option<Vec<PathBuf>> {
    let v = stats_value(stats, "Base directories")?;
    if v.is_empty() || v == "(none)" {
        return Some(Vec::new());
    }
    Some(
        v.split(", ")
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(|d| {
                let t = d.trim_end_matches(['/', '\\']);
                PathBuf::from(if t.is_empty() { d } else { t })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Affected checks
// ---------------------------------------------------------------------------------------------

/// How far a check reaches: a task's own change, a branch merged into an item branch, or the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The changed crates' lint and the changed modules' tests; dependents only compiled.
    Task,
    /// The changed crates and every crate that depends on one, tested.
    Merge,
    /// The full suite: the merge into the line.
    Line,
}

impl Tier {
    /// `task`, `merge` or `line`: the build log's `tier`.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Task => "task",
            Tier::Merge => "merge",
            Tier::Line => "line",
        }
    }

    /// `task`, `merge` or `line`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Tier> {
        match s {
            "task" => Some(Tier::Task),
            "merge" => Some(Tier::Merge),
            "line" => Some(Tier::Line),
            _ => None,
        }
    }
}

/// One workspace crate, from `cargo metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrateInfo {
    pub name: String,
    /// Its folder, relative to the repository root, `/`-separated.
    pub dir: String,
    /// The crates it depends on by path (any kind: normal, dev, build).
    pub deps: Vec<String>,
    /// Its binary target's name, for a binary crate.
    pub bin: Option<String>,
    /// It has a library target (a target of kind `lib`): its modules are tested through it.
    pub has_lib: bool,
}

/// The workspace's crates out of `cargo metadata --no-deps --format-version 1`: each package's
/// name, its folder (its manifest's, relative to `repo_root`), its path dependencies, its
/// binary target and whether it has a library target.
pub fn workspace_from_metadata(json: &str, repo_root: &Path) -> crate::Result<Vec<CrateInfo>> {
    let bad = |why: String| crate::CoreError::Unsupported(format!("cargo metadata: {why}"));
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| bad(e.to_string()))?;
    let packages = v["packages"]
        .as_array()
        .ok_or_else(|| bad("no packages".into()))?;
    let canonical_root = std::fs::canonicalize(repo_root).ok();
    let mut out = Vec::new();
    for p in packages {
        let name = p["name"]
            .as_str()
            .ok_or_else(|| bad("a package has no name".into()))?
            .to_string();
        let manifest = PathBuf::from(
            p["manifest_path"]
                .as_str()
                .ok_or_else(|| bad(format!("{name} has no manifest_path")))?,
        );
        let folder = manifest.parent().unwrap_or(Path::new(""));
        let rel = folder
            .strip_prefix(repo_root)
            .ok()
            .or_else(|| {
                canonical_root
                    .as_deref()
                    .and_then(|r| folder.strip_prefix(r).ok())
            })
            .ok_or_else(|| {
                bad(format!(
                    "{name} lies outside the repository at {}",
                    repo_root.display()
                ))
            })?;
        let dir = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let mut deps: Vec<String> = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d.get("path").is_some_and(|x| !x.is_null()))
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect();
        deps.sort();
        deps.dedup();
        let of_kind = |kind: &str| {
            p["targets"].as_array().into_iter().flatten().find(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == kind))
            })
        };
        let bin = of_kind("bin").and_then(|t| t["name"].as_str().map(String::from));
        let has_lib = of_kind("lib").is_some();
        out.push(CrateInfo {
            name,
            dir,
            deps,
            bin,
            has_lib,
        });
    }
    Ok(out)
}

/// One command a check runs: `argv` in `dir` (relative to the repository root).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What kind of check: `fmt`, `clippy`, `check`, `test`, `scripts`, `web` or `docs`.
    pub name: String,
    pub dir: String,
    pub argv: Vec<String>,
}

/// The checks a change needs, cheapest first, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Affected {
    pub tier: Tier,
    /// The full suite: at the line, or because a path every crate builds with changed.
    pub full: bool,
    pub reason: String,
    /// The crates checked (the changed ones, and at the merge tier their dependents), sorted.
    pub crates: Vec<String>,
    pub checks: Vec<Check>,
}

/// What the repository holds that the checks call on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoShape {
    /// `scripts/test.sh` exists (MNT-010): the merge and line tiers test through it.
    pub test_script: bool,
    /// `web/package.json` exists.
    pub web: bool,
    /// Every `check:*` script in `web/package.json`.
    pub web_checks: Vec<String>,
    /// The docs checks among them: `check:docs` and `check:markdown`, those present.
    pub docs_checks: Vec<String>,
    /// `scripts/tests/` exists.
    pub script_tests: bool,
    /// The project's configured line check, which replaces the test run of the full suite.
    pub line_check: Option<Vec<String>>,
}

/// Where cargo runs: the workspace's folder.
pub const CARGO_DIR: &str = "api";

/// Paths every crate builds with: a change to one runs the full suite at any tier.
const FULL_PATHS: &[&str] = &[
    "api/Cargo.toml",
    "api/Cargo.lock",
    "api/.config/nextest.toml",
];

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Does a change to `p` run the full suite: the workspace manifest or lock, cargo's or nextest's
/// config, the toolchain, or any build script?
fn forces_full(p: &str) -> bool {
    FULL_PATHS.contains(&p)
        || p.starts_with("api/.cargo/")
        || matches!(file_name(p), "rust-toolchain.toml" | "build.rs")
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// `head`, then `-p <crate>` for each crate, then `tail`.
fn per_crate(head: &[&str], crates: &[String], tail: &[&str]) -> Vec<String> {
    let mut argv = strings(head);
    for c in crates {
        argv.push("-p".into());
        argv.push(c.clone());
    }
    argv.extend(strings(tail));
    argv
}

fn check(name: &str, dir: &str, argv: Vec<String>) -> Check {
    Check {
        name: name.into(),
        dir: dir.into(),
        argv,
    }
}

/// Every crate that depends on one in `start`, transitively, with `start` itself.
fn reverse_closure(ws: &[CrateInfo], start: &BTreeSet<String>) -> BTreeSet<String> {
    let mut set = start.clone();
    loop {
        let before = set.len();
        for c in ws {
            if !set.contains(&c.name) && c.deps.iter().any(|d| set.contains(d)) {
                set.insert(c.name.clone());
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// The crate a path lies in: the one with the longest folder holding it.
fn crate_of<'a>(ws: &'a [CrateInfo], path: &str) -> Option<&'a CrateInfo> {
    ws.iter()
        .filter(|c| {
            !c.dir.is_empty()
                && path
                    .strip_prefix(c.dir.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|c| c.dir.len())
}

/// The test run a changed file of crate `c` needs at the task tier, if any: a module's unit
/// tests, a test target, or every test target when a shared test helper changed.
fn task_test(c: &CrateInfo, path: &str) -> Option<Vec<String>> {
    let rel = path.get(c.dir.len() + 1..)?;
    let base = ["cargo", "test", "--locked", "-p", c.name.as_str()];
    let mut argv = strings(&base);
    // `main.rs` is tested through the binary; a module through the library when the crate has
    // one, else through its binary (a binary-only crate).
    let binary: Vec<String> = match &c.bin {
        Some(b) => strings(&["--bin", b]),
        None => strings(&["--lib"]),
    };
    let target: Vec<String> = if c.has_lib {
        strings(&["--lib"])
    } else {
        binary.clone()
    };
    if let Some(src) = rel.strip_prefix("src/") {
        match src {
            "lib.rs" => argv.push("--lib".into()),
            "main.rs" => argv.extend(binary),
            _ => {
                let (module, nested) = match src.split_once('/') {
                    Some((m, _)) => (m, true),
                    None => (src.strip_suffix(".rs")?, false),
                };
                if module == "bin" || module.is_empty() || (!nested && module.contains('.')) {
                    return None;
                }
                argv.extend(target);
                argv.push(format!("{module}::"));
            }
        }
        return Some(argv);
    }
    let t = rel.strip_prefix("tests/")?;
    if t.starts_with("common/") || t.starts_with("fixtures/") {
        argv.extend(strings(&["--test", "*"]));
        return Some(argv);
    }
    let f = t.strip_suffix(".rs").filter(|f| !f.contains('/'))?;
    argv.extend(strings(&["--test", f]));
    Some(argv)
}

/// The bundle, then every `check:*` script of the web, sorted.
fn web_checks(repo: &RepoShape) -> Vec<Check> {
    let mut names = repo.web_checks.clone();
    names.sort();
    names.dedup();
    std::iter::once("build".to_string())
        .chain(names)
        .map(|s| check("web", ".", strings(&["npm", "--prefix", "web", "run", &s])))
        .collect()
}

fn docs_checks(repo: &RepoShape) -> Vec<Check> {
    repo.docs_checks
        .iter()
        .map(|s| check("docs", ".", strings(&["npm", "--prefix", "web", "run", s])))
        .collect()
}

/// The checks a change to `changed` (repository-relative paths) needs at `tier`, cheapest first:
/// fmt, clippy, check, tests, scripts, web, docs.
pub fn affected(changed: &[String], ws: &[CrateInfo], tier: Tier, repo: &RepoShape) -> Affected {
    let mut paths: Vec<&str> = changed
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() {
        return Affected {
            tier,
            full: false,
            reason: "nothing changed".into(),
            crates: Vec::new(),
            checks: Vec::new(),
        };
    }
    let web = repo.web && paths.iter().any(|p| p.starts_with("web/"));
    let docs = repo.web
        && paths.iter().any(|p| {
            p.starts_with("docs/")
                || p.starts_with("skill/")
                || (!p.contains('/') && p.ends_with(".md"))
        });
    let scripts = repo.script_tests && paths.iter().any(|p| p.starts_with("scripts/"));
    let scripts_check = || {
        check(
            "scripts",
            ".",
            strings(&[
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "scripts/tests",
            ]),
        )
    };
    // Web then docs, each check once.
    let push_web_and_docs = |checks: &mut Vec<Check>, web: bool, docs: bool| {
        if web {
            checks.extend(web_checks(repo));
        }
        if docs {
            for d in docs_checks(repo) {
                if !checks.iter().any(|c| c.argv == d.argv) {
                    checks.push(d);
                }
            }
        }
    };

    let trigger = paths.iter().find(|p| forces_full(p));
    if tier == Tier::Line || trigger.is_some() {
        let reason = match trigger {
            Some(p) => format!("{p} changed and every crate builds with it: the full suite"),
            None => "the merge into the line: the full suite".to_string(),
        };
        let mut checks = vec![
            check(
                "fmt",
                CARGO_DIR,
                strings(&["cargo", "fmt", "--all", "--check"]),
            ),
            check(
                "clippy",
                CARGO_DIR,
                strings(&[
                    "cargo",
                    "clippy",
                    "--locked",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ]),
            ),
            match &repo.line_check {
                Some(cmd) if !cmd.is_empty() => check("test", ".", cmd.clone()),
                _ if repo.test_script => check("test", ".", strings(&["scripts/test.sh"])),
                _ => check(
                    "test",
                    CARGO_DIR,
                    strings(&["cargo", "test", "--locked", "--workspace"]),
                ),
            },
        ];
        if scripts {
            checks.push(scripts_check());
        }
        push_web_and_docs(&mut checks, repo.web, repo.web);
        let mut crates: Vec<String> = ws.iter().map(|c| c.name.clone()).collect();
        crates.sort();
        return Affected {
            tier,
            full: true,
            reason,
            crates,
            checks,
        };
    }

    let changed_crates: BTreeSet<String> = paths
        .iter()
        .filter_map(|p| crate_of(ws, p).map(|c| c.name.clone()))
        .collect();
    let closure = reverse_closure(ws, &changed_crates);
    let mut checks = Vec::new();
    let crates: Vec<String> = match tier {
        Tier::Merge => closure.iter().cloned().collect(),
        _ => changed_crates.iter().cloned().collect(),
    };
    if !crates.is_empty() {
        checks.push(check(
            "fmt",
            CARGO_DIR,
            per_crate(&["cargo", "fmt", "--check"], &crates, &[]),
        ));
        checks.push(check(
            "clippy",
            CARGO_DIR,
            per_crate(
                &["cargo", "clippy", "--locked"],
                &crates,
                &["--all-targets", "--", "-D", "warnings"],
            ),
        ));
    }
    let reason = match tier {
        Tier::Merge => {
            if crates.is_empty() {
                format!("{} changed file(s) outside the crates", paths.len())
            } else {
                format!(
                    "{} changed file(s): crates {} and their dependents, tested",
                    paths.len(),
                    changed_crates
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        _ => {
            let dependents: Vec<String> = closure.difference(&changed_crates).cloned().collect();
            if !dependents.is_empty() {
                checks.push(check(
                    "check",
                    CARGO_DIR,
                    per_crate(&["cargo", "check", "--locked", "--tests"], &dependents, &[]),
                ));
            }
            for p in &paths {
                if let Some(c) = crate_of(ws, p)
                    && let Some(argv) = task_test(c, p)
                    && !checks.iter().any(|x| x.argv == argv)
                {
                    checks.push(check("test", CARGO_DIR, argv));
                }
            }
            if crates.is_empty() {
                format!("{} changed file(s) outside the crates", paths.len())
            } else if dependents.is_empty() {
                format!(
                    "{} changed file(s): crates {}, their changed modules tested",
                    paths.len(),
                    crates.join(", ")
                )
            } else {
                format!(
                    "{} changed file(s): crates {}, their changed modules tested; dependents {} compiled",
                    paths.len(),
                    crates.join(", "),
                    dependents.join(", ")
                )
            }
        }
    };
    if tier == Tier::Merge && !crates.is_empty() {
        checks.push(if repo.test_script {
            check("test", ".", per_crate(&["scripts/test.sh"], &crates, &[]))
        } else {
            check(
                "test",
                CARGO_DIR,
                per_crate(&["cargo", "test", "--locked"], &crates, &[]),
            )
        });
    }
    if scripts {
        checks.push(scripts_check());
    }
    push_web_and_docs(&mut checks, web, docs);
    Affected {
        tier,
        full: false,
        reason,
        crates,
        checks,
    }
}

// ---------------------------------------------------------------------------------------------
// The merge queue
// ---------------------------------------------------------------------------------------------

/// Where a queued branch is in the merge queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueState {
    /// Waiting for its batch.
    #[default]
    Queued,
    /// Merged onto the target's tip in the queue's worktree, its check running.
    Checking,
    /// Integrated: the target moved to a commit that holds it.
    Merged,
    /// Not integrated, for `reason`: a conflict or a red check.
    Refused,
    /// Taken out of the queue by a person or agent.
    Removed,
}

/// One branch waiting to be integrated into `into` (`<git-common-dir>/kanbanr/queue.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// `MQ-n`.
    pub id: String,
    pub branch: String,
    /// The branch it is integrated into: an item branch, or the line.
    pub into: String,
    /// The item the branch builds, `CODE`.
    #[serde(default)]
    pub item: String,
    /// The task, `CODE/TL/T`, for a task branch.
    #[serde(default)]
    pub task: Option<String>,
    /// 0 when the task (or an open task of the item) is on the critical path, else higher.
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub added_at: String,
    #[serde(default)]
    pub state: QueueState,
    /// Checked on its own: set when a batch it was in went red.
    #[serde(default)]
    pub alone: bool,
    #[serde(default)]
    pub reason: String,
    /// Its last check's log file.
    #[serde(default)]
    pub log: String,
    /// The target's commit after it was integrated.
    #[serde(default)]
    pub merged: String,
}

/// An entry id's number (`MQ-12` → 12), so ids sort as numbers.
fn id_number(id: &str) -> Option<u64> {
    id.rsplit('-').next().and_then(|n| n.parse().ok())
}

/// The order queued entries are taken in: priority, then when they were added, then id.
fn queue_order(a: &QueueEntry, b: &QueueEntry) -> std::cmp::Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| crate::cmp_instants(&a.added_at, &b.added_at))
        .then_with(|| match (id_number(&a.id), id_number(&b.id)) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => a.id.cmp(&b.id),
        })
        .then_with(|| a.id.cmp(&b.id))
}

/// The next batch to integrate, as indices into `entries`: the queued entries in queue order, the
/// head's target, and the entries after it with the same target, up to `max` — stopping at the
/// first with another target or one to be checked alone. A head to be checked alone is a batch of
/// one. Nothing queued: empty.
pub fn next_batch(entries: &[QueueEntry], max: usize) -> Vec<usize> {
    let mut queued: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].state == QueueState::Queued)
        .collect();
    queued.sort_by(|&a, &b| queue_order(&entries[a], &entries[b]));
    let Some(&head) = queued.first() else {
        return Vec::new();
    };
    if entries[head].alone {
        return vec![head];
    }
    queued
        .iter()
        .copied()
        .take_while(|&i| entries[i].into == entries[head].into && !entries[i].alone)
        .take(max.max(1))
        .collect()
}

/// A batch whose check went red: when it held more than one entry, each is marked to be checked
/// alone and goes back to Queued, in its place, so the queue finds which one is red. A batch of one
/// is left for the caller to refuse.
pub fn split_red(entries: &mut [QueueEntry], batch: &[usize]) {
    if batch.len() < 2 {
        return;
    }
    for &i in batch {
        if let Some(e) = entries.get_mut(i) {
            e.alone = true;
            e.state = QueueState::Queued;
        }
    }
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

    /// `sccache --show-stats` as 0.17 prints it, cut to the lines around the ones read.
    const STATS_0_17: &str = "Compile requests                 296942
Cache hits                       193750
Non-cacheable reasons:
crate-type                         7268

Cache location                  Local disk: \"/home/dev/.cache/sccache-kanbanr\"
Base directories                /work/kanbanr/, /work/kanbanr/.claude/worktrees/def-034/
Use direct/preprocessor mode?   yes
Version (client)                0.17.0
Cache size                           10 GiB
Max cache size                       10 GiB
";

    fn get<'a>(env: &'a BuildEnv, k: &str) -> Option<&'a str> {
        env.set
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn the_build_env_points_cargo_at_the_projects_compiler_cache_with_every_worktree_as_a_base_dir()
    {
        let root = Path::new("/work/kanbanr");
        let e = BuildConfig::default().effective(12, root);
        let cache = Path::new("/m/sccache/kanbanr-0123abcd");

        // sccache on and found: the wrapper and the project's server.
        let env = build_env(&e, true, false, true, cache, None);
        assert_eq!(get(&env, "CARGO_BUILD_JOBS"), Some("5"));
        assert_eq!(get(&env, "NEXTEST_TEST_THREADS"), Some("5"));
        assert_eq!(get(&env, "RUST_TEST_THREADS"), Some("5"));
        assert_eq!(get(&env, "RUSTC_WRAPPER"), Some("sccache"));
        assert_eq!(get(&env, "CARGO_INCREMENTAL"), Some("0"));
        assert_eq!(
            get(&env, "SCCACHE_DIR"),
            Some("/m/sccache/kanbanr-0123abcd")
        );
        assert_eq!(get(&env, "SCCACHE_CACHE_SIZE"), Some("10G"));
        // The port comes from the main checkout's path, the same every time.
        let port = derived_port(root);
        assert!((PORT_BASE..PORT_BASE + PORT_SPAN as u16).contains(&port));
        assert_eq!(
            get(&env, "SCCACHE_SERVER_PORT"),
            Some(port.to_string().as_str())
        );
        assert_eq!(
            BuildConfig::default().effective(4, root).sccache_port,
            port,
            "the port does not depend on the machine"
        );
        assert!(env.notes.is_empty(), "{env:?}");
        assert_eq!(get(&env, "KANBANR_UI_DIST"), None);

        // sccache not found: no wrapper, a note; the caps stay.
        let env = build_env(&e, false, false, true, cache, None);
        assert_eq!(get(&env, "RUSTC_WRAPPER"), None);
        assert_eq!(get(&env, "SCCACHE_SERVER_PORT"), None);
        assert_eq!(get(&env, "CARGO_BUILD_JOBS"), Some("5"));
        assert_eq!(
            env.notes,
            ["sccache not found: compiling without the shared cache"]
        );

        // sccache off: none of it, and nothing to say.
        let off = BuildConfig {
            sccache: Some("off".into()),
            jobs_per_slot: Some(3),
            ..Default::default()
        }
        .effective(12, root);
        let env = build_env(
            &off,
            true,
            true,
            true,
            cache,
            Some(Path::new("/m/web/k/dist")),
        );
        assert!(
            env.set
                .iter()
                .all(|(k, _)| !k.starts_with("SCCACHE") && k != "RUSTC_WRAPPER")
        );
        assert_eq!(get(&env, "RUST_TEST_THREADS"), Some("3"));
        assert_eq!(get(&env, "KANBANR_UI_DIST"), Some("/m/web/k/dist"));
        assert!(env.notes.is_empty());

        // The cache folder: per project and main checkout.
        let m = Path::new("/m");
        let d = sccache_dir(m, "kanbanr", root);
        assert_eq!(d.parent(), Some(Path::new("/m/sccache")));
        let name = d.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("kanbanr-") && name.len() == "kanbanr-".len() + 8,
            "{name}"
        );
        assert_ne!(
            d,
            sccache_dir(m, "kanbanr", Path::new("/elsewhere/kanbanr"))
        );

        // Base directories: the main checkout and every worktree, absolute, sorted, once.
        let dirs = base_dirs(
            root,
            &[
                PathBuf::from("/work/kanbanr/.claude/worktrees/b"),
                PathBuf::from(".claude/worktrees/a"),
                PathBuf::from("/work/kanbanr"),
                PathBuf::from("/work/kanbanr/.claude/worktrees/b"),
            ],
        );
        assert_eq!(
            dirs,
            [
                PathBuf::from("/work/kanbanr"),
                PathBuf::from("/work/kanbanr/.claude/worktrees/a"),
                PathBuf::from("/work/kanbanr/.claude/worktrees/b"),
            ]
        );

        // Restart: none running → yes; the same set → no; another set → yes, or deferred while
        // another build runs.
        let one = [PathBuf::from("/work/kanbanr")];
        assert_eq!(needs_restart(None, &dirs, false), Restart::Yes);
        assert_eq!(needs_restart(None, &dirs, true), Restart::Yes);
        let shuffled = [dirs[2].clone(), dirs[0].clone(), dirs[1].clone()];
        assert_eq!(needs_restart(Some(&shuffled), &dirs, true), Restart::No);
        assert_eq!(needs_restart(Some(&one), &dirs, false), Restart::Yes);
        assert_eq!(needs_restart(Some(&one), &dirs, true), Restart::Deferred);

        // The stats text of sccache 0.17.
        assert_eq!(
            parse_base_dirs(STATS_0_17),
            Some(vec![
                PathBuf::from("/work/kanbanr"),
                PathBuf::from("/work/kanbanr/.claude/worktrees/def-034"),
            ])
        );
        assert_eq!(
            needs_restart(
                parse_base_dirs(STATS_0_17).as_deref(),
                &base_dirs(root, &[PathBuf::from(".claude/worktrees/def-034/")]),
                false
            ),
            Restart::No,
            "a reported trailing / is the same folder"
        );
        assert_eq!(
            parse_base_dirs("Cache location   x\nBase directories                (none)\n"),
            Some(vec![])
        );
        assert_eq!(parse_base_dirs("Compile requests 3\n"), None);
        assert_eq!(stats_value(STATS_0_17, "Cache size"), Some("10 GiB"));
        assert_eq!(stats_value(STATS_0_17, "Max cache size"), Some("10 GiB"));
    }

    #[test]
    fn mold_is_used_only_where_configured_and_present() {
        let root = Path::new("/work/kanbanr");
        let cache = Path::new("/m/c");
        let mold = BuildConfig {
            linker: Some("mold".into()),
            ..Default::default()
        }
        .effective(12, root);
        let flags = |env: &BuildEnv| {
            env.set
                .iter()
                .filter(|(k, _)| k.starts_with("CARGO_TARGET_") && k.ends_with("_RUSTFLAGS"))
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
        };
        let env = build_env(&mold, true, true, true, cache, None);
        assert_eq!(
            flags(&env),
            [
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-C link-arg=-fuse-ld=mold",
                "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-C link-arg=-fuse-ld=mold",
            ]
        );
        assert!(env.notes.is_empty());
        // Not found: none, and a note.
        let env = build_env(&mold, true, false, true, cache, None);
        assert!(flags(&env).is_empty());
        assert_eq!(
            env.notes,
            ["mold not found: linking with the default linker"]
        );
        // Not Linux: none, and a note.
        let env = build_env(&mold, true, true, false, cache, None);
        assert!(flags(&env).is_empty());
        assert_eq!(
            env.notes,
            ["mold is used on Linux only: linking with the default linker"]
        );
        // Not configured (the default linker): none, whatever is installed.
        for linker in [None, Some("default")] {
            let e = BuildConfig {
                linker: linker.map(String::from),
                ..Default::default()
            }
            .effective(12, root);
            let env = build_env(&e, true, true, true, cache, None);
            assert!(flags(&env).is_empty(), "{linker:?}");
            assert!(env.notes.is_empty());
        }
    }

    /// Four crates as `cargo metadata --no-deps` reports them: `lint` ← `core` ← `server` ←
    /// `cli` (a binary, `kanbanr`), `cli` also on `core` directly, plus a registry dependency.
    const METADATA: &str = r#"{
      "packages": [
        {"name": "lint", "manifest_path": "/repo/api/crates/lint/Cargo.toml",
         "dependencies": [{"name": "serde", "kind": null}],
         "targets": [{"kind": ["lib"], "name": "lint"}]},
        {"name": "core", "manifest_path": "/repo/api/crates/core/Cargo.toml",
         "dependencies": [{"name": "lint", "path": "/repo/api/crates/lint", "kind": null},
                          {"name": "serde", "kind": null}],
         "targets": [{"kind": ["lib"], "name": "core"}]},
        {"name": "server", "manifest_path": "/repo/api/crates/server/Cargo.toml",
         "dependencies": [{"name": "core", "path": "/repo/api/crates/core", "kind": null}],
         "targets": [{"kind": ["lib"], "name": "server"},
                     {"kind": ["custom-build"], "name": "build-script-build"}]},
        {"name": "cli", "manifest_path": "/repo/api/crates/cli/Cargo.toml",
         "dependencies": [{"name": "core", "path": "/repo/api/crates/core", "kind": null},
                          {"name": "server", "path": "/repo/api/crates/server", "kind": null},
                          {"name": "core", "path": "/repo/api/crates/core", "kind": "dev"}],
         "targets": [{"kind": ["bin"], "name": "kanbanr"}]}
      ],
      "workspace_root": "/repo/api"
    }"#;

    fn ws() -> Vec<CrateInfo> {
        workspace_from_metadata(METADATA, Path::new("/repo")).unwrap()
    }

    fn shape() -> RepoShape {
        RepoShape {
            test_script: true,
            web: true,
            web_checks: vec![
                "check:ui".into(),
                "check:docs".into(),
                "check:markdown".into(),
                "check:api-types".into(),
            ],
            docs_checks: vec!["check:docs".into(), "check:markdown".into()],
            script_tests: true,
            line_check: None,
        }
    }

    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    /// Each check as one line, `<dir>$ <argv>`, to compare whole lists.
    fn lines(a: &Affected) -> Vec<String> {
        a.checks
            .iter()
            .map(|c| format!("{}$ {}", c.dir, c.argv.join(" ")))
            .collect()
    }

    #[test]
    fn a_crate_with_a_library_runs_its_modules_through_the_library() {
        // `app` has a library and a binary; `tool` is binary-only.
        let json = r#"{
          "packages": [
            {"name": "app", "manifest_path": "/repo/api/crates/app/Cargo.toml",
             "dependencies": [],
             "targets": [{"kind": ["lib"], "name": "app"}, {"kind": ["bin"], "name": "app-cli"}]},
            {"name": "tool", "manifest_path": "/repo/api/crates/tool/Cargo.toml",
             "dependencies": [],
             "targets": [{"kind": ["bin"], "name": "tool"}]}
          ]
        }"#;
        let ws = workspace_from_metadata(json, Path::new("/repo")).unwrap();
        assert!(ws[0].has_lib && ws[0].bin.as_deref() == Some("app-cli"));
        assert!(!ws[1].has_lib && ws[1].bin.as_deref() == Some("tool"));
        let a = affected(
            &paths(&[
                "api/crates/app/src/store.rs",
                "api/crates/app/src/main.rs",
                "api/crates/tool/src/walk.rs",
            ]),
            &ws,
            Tier::Task,
            &shape(),
        );
        let l = lines(&a);
        assert!(
            l.contains(&"api$ cargo test --locked -p app --lib store::".to_string()),
            "a module of a crate with a library runs through the library: {l:?}"
        );
        assert!(
            l.contains(&"api$ cargo test --locked -p app --bin app-cli".to_string()),
            "main.rs runs through the binary: {l:?}"
        );
        assert!(
            l.contains(&"api$ cargo test --locked -p tool --bin tool walk::".to_string()),
            "a binary-only crate's module runs through its binary: {l:?}"
        );
        assert!(
            !l.iter().any(|c| c.contains("--bin app-cli store::")),
            "{l:?}"
        );
    }

    #[test]
    fn a_task_check_runs_the_changed_modules_tests_and_checks_dependents() {
        let ws = ws();
        assert_eq!(
            ws.iter()
                .map(|c| (c.name.as_str(), c.dir.as_str()))
                .collect::<Vec<_>>(),
            [
                ("lint", "api/crates/lint"),
                ("core", "api/crates/core"),
                ("server", "api/crates/server"),
                ("cli", "api/crates/cli")
            ]
        );
        assert_eq!(ws[3].deps, ["core", "server"], "path deps only, once each");
        assert!(ws[1].deps == ["lint"] && ws[0].deps.is_empty());
        assert_eq!(ws[3].bin.as_deref(), Some("kanbanr"));
        assert_eq!(ws[2].bin, None);

        // A module of core: its own lint and module tests; its dependents only compiled.
        let a = affected(
            &paths(&[
                "api/crates/core/src/store.rs",
                "api/crates/core/src/store/index.rs",
            ]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert!(!a.full, "{a:?}");
        assert_eq!(a.tier, Tier::Task);
        assert_eq!(a.crates, ["core"]);
        assert_eq!(
            lines(&a),
            [
                "api$ cargo fmt --check -p core",
                "api$ cargo clippy --locked -p core --all-targets -- -D warnings",
                "api$ cargo check --locked --tests -p cli -p server",
                "api$ cargo test --locked -p core --lib store::",
            ]
        );
        assert_eq!(
            a.checks.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["fmt", "clippy", "check", "test"]
        );

        // An integration test file: that test target alone; cli has no dependents.
        let a = affected(
            &paths(&["api/crates/cli/tests/local_mode.rs"]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert_eq!(
            lines(&a),
            [
                "api$ cargo fmt --check -p cli",
                "api$ cargo clippy --locked -p cli --all-targets -- -D warnings",
                "api$ cargo test --locked -p cli --test local_mode",
            ]
        );

        // lib.rs: the whole library; a binary crate's main.rs and modules: its binary.
        let a = affected(
            &paths(&[
                "api/crates/lint/src/lib.rs",
                "api/crates/cli/src/main.rs",
                "api/crates/cli/src/buildrun.rs",
            ]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert_eq!(a.crates, ["cli", "lint"]);
        let l = lines(&a);
        assert!(
            l.contains(&"api$ cargo check --locked --tests -p core -p server".to_string()),
            "lint's dependents, transitively, less the changed crates: {l:?}"
        );
        assert!(l.contains(&"api$ cargo test --locked -p lint --lib".to_string()));
        assert!(l.contains(&"api$ cargo test --locked -p cli --bin kanbanr".to_string()));
        assert!(
            l.contains(&"api$ cargo test --locked -p cli --bin kanbanr buildrun::".to_string())
        );

        // A shared test helper: every integration test of the crate.
        let a = affected(
            &paths(&["api/crates/cli/tests/common/mod.rs"]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert!(
            lines(&a).contains(&"api$ cargo test --locked -p cli --test *".to_string()),
            "{a:?}"
        );
        let a = affected(
            &paths(&["api/crates/cli/tests/fixtures/board.yaml"]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert!(lines(&a).contains(&"api$ cargo test --locked -p cli --test *".to_string()));

        // Nothing changed: nothing to run.
        let a = affected(&[], &ws, Tier::Task, &shape());
        assert!(a.checks.is_empty() && !a.full);
        assert!(a.reason.starts_with("nothing changed"), "{}", a.reason);
    }

    #[test]
    fn a_merge_check_tests_the_changed_crates_and_their_dependents() {
        let ws = ws();
        let a = affected(
            &paths(&["api/crates/core/src/store.rs"]),
            &ws,
            Tier::Merge,
            &shape(),
        );
        assert!(!a.full);
        assert_eq!(a.crates, ["cli", "core", "server"], "the reverse closure");
        assert_eq!(
            lines(&a),
            [
                "api$ cargo fmt --check -p cli -p core -p server",
                "api$ cargo clippy --locked -p cli -p core -p server --all-targets -- -D warnings",
                ".$ scripts/test.sh -p cli -p core -p server",
            ]
        );

        // Without scripts/test.sh, cargo test itself.
        let plain = RepoShape {
            test_script: false,
            ..shape()
        };
        let a = affected(
            &paths(&["api/crates/lint/src/rules.rs"]),
            &ws,
            Tier::Merge,
            &plain,
        );
        assert_eq!(a.crates, ["cli", "core", "lint", "server"]);
        assert_eq!(
            lines(&a).last().unwrap(),
            "api$ cargo test --locked -p cli -p core -p lint -p server"
        );

        // A leaf: itself alone.
        let a = affected(
            &paths(&["api/crates/cli/src/main.rs"]),
            &ws,
            Tier::Merge,
            &shape(),
        );
        assert_eq!(a.crates, ["cli"]);
        assert_eq!(lines(&a).last().unwrap(), ".$ scripts/test.sh -p cli");
    }

    #[test]
    fn a_lockfile_toolchain_or_build_script_change_runs_the_full_suite() {
        let ws = ws();
        let full_with_web = [
            "api$ cargo fmt --all --check",
            "api$ cargo clippy --locked --workspace --all-targets -- -D warnings",
            ".$ scripts/test.sh",
            ".$ npm --prefix web run build",
            ".$ npm --prefix web run check:api-types",
            ".$ npm --prefix web run check:docs",
            ".$ npm --prefix web run check:markdown",
            ".$ npm --prefix web run check:ui",
        ];
        for trigger in [
            "api/Cargo.toml",
            "api/Cargo.lock",
            "api/.cargo/config.toml",
            "rust-toolchain.toml",
            "api/crates/server/build.rs",
            "api/.config/nextest.toml",
        ] {
            for tier in [Tier::Task, Tier::Merge] {
                let a = affected(
                    &paths(&[trigger, "api/crates/core/src/store.rs"]),
                    &ws,
                    tier,
                    &shape(),
                );
                assert!(a.full, "{trigger} at {tier:?}: {a:?}");
                assert!(
                    a.reason.contains(trigger),
                    "the reason names {trigger}: {}",
                    a.reason
                );
                assert_eq!(lines(&a), full_with_web, "{trigger}");
            }
        }

        // The line is always full, whatever changed.
        let a = affected(
            &paths(&["api/crates/core/src/store.rs"]),
            &ws,
            Tier::Line,
            &shape(),
        );
        assert!(a.full);
        assert!(a.reason.contains("line"), "{}", a.reason);
        assert_eq!(lines(&a), full_with_web);

        // A configured line check replaces the test run; without test.sh, cargo test; no web.
        let a = affected(
            &paths(&["api/Cargo.lock"]),
            &ws,
            Tier::Line,
            &RepoShape {
                line_check: Some(vec!["make".into(), "ci".into()]),
                web: false,
                ..shape()
            },
        );
        assert_eq!(
            lines(&a),
            [
                "api$ cargo fmt --all --check",
                "api$ cargo clippy --locked --workspace --all-targets -- -D warnings",
                ".$ make ci",
            ]
        );
        let a = affected(
            &paths(&["api/Cargo.lock"]),
            &ws,
            Tier::Task,
            &RepoShape {
                test_script: false,
                web: false,
                ..shape()
            },
        );
        assert_eq!(
            lines(&a).last().unwrap(),
            "api$ cargo test --locked --workspace"
        );
    }

    #[test]
    fn web_and_docs_changes_run_their_checks_only() {
        let ws = ws();
        // The web alone: the bundle, then every check:* script, sorted; no cargo.
        let a = affected(
            &paths(&["web/src/App.tsx", "web/package-lock.json"]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert!(!a.full && a.crates.is_empty());
        assert_eq!(
            lines(&a),
            [
                ".$ npm --prefix web run build",
                ".$ npm --prefix web run check:api-types",
                ".$ npm --prefix web run check:docs",
                ".$ npm --prefix web run check:markdown",
                ".$ npm --prefix web run check:ui",
            ]
        );
        assert!(a.checks.iter().all(|c| c.name == "web"));

        // Docs, the skill or a top-level page: the two docs checks.
        for p in [
            "docs/guide/using/with-claude.md",
            "skill/kanbanr/SKILL.md",
            "CHANGELOG.md",
        ] {
            let a = affected(&paths(&[p]), &ws, Tier::Merge, &shape());
            assert_eq!(
                lines(&a),
                [
                    ".$ npm --prefix web run check:docs",
                    ".$ npm --prefix web run check:markdown",
                ],
                "{p}"
            );
            assert!(a.checks.iter().all(|c| c.name == "docs"));
        }

        // Web and docs together: each check once.
        let a = affected(
            &paths(&["web/src/App.tsx", "docs/guide/index.md"]),
            &ws,
            Tier::Task,
            &shape(),
        );
        assert_eq!(lines(&a).len(), 5, "{:?}", lines(&a));

        // The scripts: their unit tests.
        let a = affected(&paths(&["scripts/ci_local.py"]), &ws, Tier::Task, &shape());
        assert_eq!(
            lines(&a),
            [".$ python3 -m unittest discover -s scripts/tests"]
        );
        assert_eq!(a.checks[0].name, "scripts");

        // A nested page is not a top-level one; with no web folder, no docs checks to run.
        let a = affected(&paths(&["api/README.md"]), &ws, Tier::Task, &shape());
        assert!(a.checks.is_empty(), "{a:?}");
        let a = affected(
            &paths(&["docs/guide/index.md"]),
            &ws,
            Tier::Task,
            &RepoShape {
                web: false,
                ..shape()
            },
        );
        assert!(a.checks.is_empty(), "{a:?}");
    }

    fn entry(id: &str, into: &str, priority: u8, added_at: &str) -> QueueEntry {
        QueueEntry {
            id: id.into(),
            branch: format!("feat/{id}"),
            into: into.into(),
            item: "FTR-1".into(),
            priority,
            added_at: added_at.into(),
            ..Default::default()
        }
    }

    fn ids(entries: &[QueueEntry], batch: &[usize]) -> Vec<String> {
        batch.iter().map(|&i| entries[i].id.clone()).collect()
    }

    #[test]
    fn the_queue_takes_entries_by_priority_and_batches_one_target() {
        assert!(next_batch(&[], 4).is_empty());

        let line = "sprint/SP-4";
        let mut q = vec![
            entry("MQ-1", line, 1, "2026-11-17T09:00:00Z"),
            entry("MQ-2", "feat/FTR-9", 1, "2026-11-17T09:01:00Z"),
            entry("MQ-3", line, 0, "2026-11-17T09:02:00Z"),
            entry("MQ-4", line, 1, "2026-11-17T09:03:00Z"),
            entry("MQ-5", line, 1, "2026-11-17T09:04:00Z"),
            entry("MQ-6", line, 1, "2026-11-17T09:05:00Z"),
            entry("MQ-7", line, 1, "2026-11-17T09:06:00Z"),
        ];
        // Critical path first, then arrival; one target per batch: MQ-2 ends it.
        assert_eq!(ids(&q, &next_batch(&q, 4)), ["MQ-3", "MQ-1"]);

        // MQ-2 targets an item branch; with it out of the way, the line batch is capped at 4.
        q[1].state = QueueState::Merged;
        assert_eq!(
            ids(&q, &next_batch(&q, 4)),
            ["MQ-3", "MQ-1", "MQ-4", "MQ-5"]
        );
        assert_eq!(ids(&q, &next_batch(&q, 2)), ["MQ-3", "MQ-1"]);
        assert_eq!(ids(&q, &next_batch(&q, 0)), ["MQ-3"], "at least one");

        // Only queued entries are taken.
        q[2].state = QueueState::Refused;
        q[0].state = QueueState::Removed;
        q[3].state = QueueState::Checking;
        assert_eq!(ids(&q, &next_batch(&q, 4)), ["MQ-5", "MQ-6", "MQ-7"]);

        // A head to be checked alone goes alone; an alone entry behind the head ends the batch.
        q[4].alone = true;
        assert_eq!(ids(&q, &next_batch(&q, 4)), ["MQ-5"]);
        q[4].alone = false;
        q[5].alone = true;
        assert_eq!(ids(&q, &next_batch(&q, 4)), ["MQ-5"]);

        // The same priority and time: ids in number order, MQ-9 before MQ-10.
        let same = vec![
            entry("MQ-10", line, 1, "2026-11-17T09:00:00Z"),
            entry("MQ-9", line, 1, "2026-11-17T09:00:00Z"),
        ];
        assert_eq!(ids(&same, &next_batch(&same, 4)), ["MQ-9", "MQ-10"]);
        // Times compare as instants, not text: .5 s is after .25 s.
        let times = vec![
            entry("MQ-1", line, 1, "2026-11-17T09:00:00.5Z"),
            entry("MQ-2", line, 1, "2026-11-17T09:00:00.25Z"),
        ];
        assert_eq!(ids(&times, &next_batch(&times, 4)), ["MQ-2", "MQ-1"]);
    }

    #[test]
    fn a_red_batch_is_retried_one_entry_at_a_time() {
        let line = "main";
        let mut q = vec![
            entry("MQ-1", line, 1, "2026-11-17T09:00:00Z"),
            entry("MQ-2", line, 0, "2026-11-17T09:01:00Z"),
            entry("MQ-3", line, 1, "2026-11-17T09:02:00Z"),
            entry("MQ-4", line, 1, "2026-11-17T09:03:00Z"),
        ];
        let batch = next_batch(&q, 3);
        assert_eq!(ids(&q, &batch), ["MQ-2", "MQ-1", "MQ-3"]);
        for &i in &batch {
            q[i].state = QueueState::Checking;
        }

        split_red(&mut q, &batch);
        for &i in &batch {
            assert!(q[i].alone, "{} is checked alone", q[i].id);
            assert_eq!(q[i].state, QueueState::Queued);
        }
        assert!(!q[3].alone, "an entry outside the batch is untouched");
        assert_eq!(
            q.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["MQ-1", "MQ-2", "MQ-3", "MQ-4"],
            "order unchanged"
        );

        // Retried one at a time, in queue order.
        let one = next_batch(&q, 3);
        assert_eq!(ids(&q, &one), ["MQ-2"]);
        q[one[0]].state = QueueState::Merged;
        let one = next_batch(&q, 3);
        assert_eq!(ids(&q, &one), ["MQ-1"]);

        // A red batch of one is left as it is, for the caller to refuse.
        q[one[0]].state = QueueState::Checking;
        split_red(&mut q, &one);
        assert_eq!(q[0].state, QueueState::Checking);
    }
}
