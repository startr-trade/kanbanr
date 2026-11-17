//! `kanbanr build` (FTR-062): agents' builds share one machine. A command run through
//! `kanbanr build run` waits for one of the project's build slots, in priority order (a task on the
//! critical path first), then runs, and its wait and run time go to the repository's build log.
//!
//! The state is the machine's and the repository's, never the board's: slot locks and waiting
//! tickets in [`buildplan::machine_dir`], the log in `<git-common-dir>/kanbanr/builds/`. A slot is a
//! held file lock, so a crashed kanbanr gives its slot back with its process. A child is not killed
//! with kanbanr: it is in the terminal's process group, so Ctrl-C reaches both, but a kanbanr killed
//! alone leaves its child running, unaccounted for.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fs2::FileExt;
use kanbanr_core::Store;
use kanbanr_core::buildplan::{self, Affected, BuildRow, Effective, Priority, RepoShape, Tier};
use kanbanr_core::coderepo;
use serde::{Deserialize, Serialize};

/// What `kanbanr build run` was asked to run.
#[derive(Debug, Clone, Default)]
pub struct RunRequest {
    pub cmd: Vec<String>,
    /// `CODE/TL-001/T3`; absent: the worktree's task scope, if any.
    pub task: Option<String>,
    /// `build`, `check` or `test`.
    pub kind: String,
    /// `task`, `merge` or `line`, for a check.
    pub tier: Option<String>,
}

/// How often a waiting build looks for a free slot.
const POLL: Duration = Duration::from_millis(250);
const SLOTS: &str = "slots";
const QUEUE: &str = "queue";
const TICKET: &str = ".ticket";

/// Everything a run needs, resolved once before it waits.
#[derive(Debug, Clone)]
struct Plan {
    machine: PathBuf,
    /// The repository's build folder; `None` outside a git repository (the run is not recorded).
    repo_dir: Option<PathBuf>,
    effective: Effective,
    priority: Priority,
    task: Option<String>,
    scope: Option<String>,
    worktree: String,
    branch: String,
    /// Whether the load and memory waits apply (Linux only).
    machine_waits: bool,
}

/// Run `req.cmd` in the current directory once a build slot is free, record it in the build log
/// of the repository at `repo`, and return its exit status.
pub fn run(store: &Store, project: &str, repo: &Path, req: &RunRequest) -> anyhow::Result<i32> {
    if req.cmd.is_empty() {
        anyhow::bail!("name the command to run: `kanbanr build run -- cargo test -p …`");
    }
    let plan = plan(store, project, repo, req)?;
    let (code, _) = run_planned(&plan, req, &spawn)?;
    Ok(code)
}

/// Resolve the task, its priority, the settings and where state goes.
fn plan(store: &Store, project: &str, repo: &Path, req: &RunRequest) -> anyhow::Result<Plan> {
    let facts = coderepo::worktree_facts(repo);
    let main_root = facts
        .as_ref()
        .map(|f| f.main_root.clone())
        .unwrap_or_else(|| repo.to_path_buf());
    let config = store
        .load_meta(project)
        .map(|p| p.config.build)
        .unwrap_or_default();
    let effective = config.effective(cpus(), &main_root);

    // The scope bound to this worktree names the task when --task does not.
    let pointer = facts
        .as_ref()
        .and_then(|f| kanbanr_core::agentscope::read_pointer(&f.git_dir));
    let bound = facts.as_ref().and_then(|f| bound_scope(store, f));
    let task = match &req.task {
        Some(t) => Some(task_ref(t, project)?),
        None => bound
            .as_ref()
            .and_then(|s| s.task.as_ref().map(|t| format!("{}/{t}", s.item))),
    };
    let priority = match &task {
        None => Priority::None,
        Some(t) if on_critical_path(store, project, t) => Priority::Critical,
        Some(_) => Priority::Task,
    };
    let repo_dir = git_common_dir(repo).map(|c| buildplan::repo_dir(&c));
    Ok(Plan {
        machine: buildplan::machine_dir(),
        repo_dir,
        effective,
        priority,
        task,
        scope: pointer.map(|p| p.scope),
        worktree: facts
            .as_ref()
            .map(|f| match f.rel() {
                r if r.is_empty() => ".".to_string(),
                r => r,
            })
            .unwrap_or_else(|| repo.display().to_string()),
        branch: facts.and_then(|f| f.branch).unwrap_or_default(),
        machine_waits: cfg!(target_os = "linux"),
    })
}

/// The agent scope bound to the worktree `facts` describes, if any.
fn bound_scope(
    store: &Store,
    facts: &kanbanr_core::agentscope::WorktreeFacts,
) -> Option<kanbanr_core::agentscope::Scope> {
    let p = kanbanr_core::agentscope::read_pointer(&facts.git_dir)?;
    kanbanr_core::agentscope::load(store, &p.project)
        .ok()?
        .into_iter()
        .find(|s| s.id == p.scope)
}

/// `--task` as `CODE/TL-001/T3`; any other form is refused.
fn task_ref(t: &str, project: &str) -> anyhow::Result<String> {
    match kanbanr_core::taskgraph::parse_task_ref(t, project, "", "") {
        Some((p, code, Some(list), key)) if p == project || project.is_empty() => {
            Ok(format!("{code}/{list}/{key}"))
        }
        _ => anyhow::bail!("--task takes a task as CODE/TL-001/T3: got '{t}'"),
    }
}

/// Is `task` (`CODE/TL-001/T3`) on the project's critical path? A graph that cannot be built
/// answers no: the build then waits as an ordinary task.
fn on_critical_path(store: &Store, project: &str, task: &str) -> bool {
    kanbanr_core::taskgraph::TaskGraph::build(store, project, None)
        .map(|g| g.critical_path(project).0.iter().any(|t| t == task))
        .unwrap_or(false)
}

fn cpus() -> u32 {
    std::thread::available_parallelism()
        .map(|n| u32::try_from(n.get()).unwrap_or(u32::MAX))
        .unwrap_or(1)
}

/// The repository's common git directory, absolute.
fn git_common_dir(repo: &Path) -> Option<PathBuf> {
    let out = coderepo::git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    Some(PathBuf::from(out.trim()))
}

/// Run the command itself, inheriting stdio, in the current directory.
fn spawn(cmd: &[String]) -> std::io::Result<i32> {
    let status = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()?;
    Ok(exit_code(status))
}

/// The status a shell would report: the code, or 128 + the signal that ended it.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(c) = status.code() {
        return c;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(s) = status.signal() {
            return 128 + s;
        }
    }
    -1
}

/// Wait for a slot, run `exec`, record the row. Returns the exit status and the row written.
fn run_planned(
    plan: &Plan,
    req: &RunRequest,
    exec: &dyn Fn(&[String]) -> std::io::Result<i32>,
) -> anyhow::Result<(i32, BuildRow)> {
    let asked = Instant::now();
    let at = kanbanr_core::now_rfc3339();
    let slot = wait_for_slot(plan, &req.cmd)?;
    let queued_ms = millis(asked.elapsed());
    let mem_available_mib = plan.machine_waits.then(mem_available_mib).flatten();
    let started = Instant::now();
    let result = exec(&req.cmd);
    let run_ms = millis(started.elapsed());
    let (exit, note) = match &result {
        Ok(c) => (*c, String::new()),
        Err(e) => (127, format!("could not start: {e}")),
    };
    let row = BuildRow {
        at,
        worktree: plan.worktree.clone(),
        branch: plan.branch.clone(),
        task: plan.task.clone(),
        scope: plan.scope.clone(),
        kind: if req.kind.is_empty() {
            "build".into()
        } else {
            req.kind.clone()
        },
        tier: req.tier.clone(),
        cmd: req.cmd.clone(),
        slot: slot.n,
        queued_ms,
        run_ms,
        exit,
        cache_hits: None,
        cache_misses: None,
        mem_available_mib,
        note,
    };
    match &plan.repo_dir {
        // Best effort: a log that cannot be written never changes the command's status.
        Some(dir) => {
            if let Err(e) = buildplan::append_row(dir, &row) {
                eprintln!(
                    "warning: the build log could not be written ({e}); the run is not recorded"
                );
            }
        }
        None => eprintln!("warning: not in a git repository, so the run is not recorded"),
    }
    // The row is written before the slot is given back, so the log is in the order runs ended.
    drop(slot);
    if let Err(e) = result {
        anyhow::bail!("could not run `{}`: {e}", req.cmd[0]);
    }
    Ok((exit, row))
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------------------------
// Slots and tickets
// ---------------------------------------------------------------------------------------------

/// Who holds a slot, or waits for one: written beside the lock, which itself cannot be read on
/// every platform while it is held.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Holder {
    pub pid: u32,
    pub since: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
}

/// A held slot: the lock is released, and its holder file removed, when this is dropped.
struct Slot {
    n: u32,
    holder: PathBuf,
    _lock: File,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.holder);
    }
}

/// A waiting build's ticket: locked by its owner while it waits, removed when dropped.
struct Ticket {
    name: String,
    path: PathBuf,
    info: PathBuf,
    lock: Option<File>,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // Unlock first: Windows refuses to remove a file this process holds open.
        self.lock.take();
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.info);
    }
}

fn slot_lock(machine: &Path, n: u32) -> PathBuf {
    machine.join(SLOTS).join(format!("slot-{n}.lock"))
}

fn slot_holder(machine: &Path, n: u32) -> PathBuf {
    machine.join(SLOTS).join(format!("slot-{n}.json"))
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Arrival times, strictly increasing within this process, so two threads asking in the same
/// clock tick still get distinct, ordered tickets.
fn arrival_ns() -> u128 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let mut prev = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(prev + 1);
        match LAST.compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return u128::from(next),
            Err(p) => prev = p,
        }
    }
}

/// Write and lock this build's ticket. It is made under a temporary name and renamed once
/// locked, so no other build ever sees it unlocked and takes it for stale.
fn take_ticket(plan: &Plan, cmd: &[String]) -> anyhow::Result<Ticket> {
    let queue = plan.machine.join(QUEUE);
    std::fs::create_dir_all(&queue)?;
    let name = buildplan::ticket_name(plan.priority, arrival_ns(), std::process::id());
    let info = queue.join(format!("{name}.json"));
    let holder = Holder {
        pid: std::process::id(),
        since: kanbanr_core::now_rfc3339(),
        worktree: plan.worktree.clone(),
        branch: plan.branch.clone(),
        task: plan.task.clone(),
        cmd: cmd.to_vec(),
    };
    std::fs::write(&info, serde_json::to_vec(&holder)?)?;
    let tmp = queue.join(format!("{name}.new"));
    let file = open_lock(&tmp)?;
    file.lock_exclusive()?;
    let path = queue.join(&name);
    std::fs::rename(&tmp, &path)?;
    Ok(Ticket {
        name,
        path,
        info,
        lock: Some(file),
    })
}

/// The tickets of builds still waiting, sorted. A ticket that can be locked belongs to a build
/// that is gone: it is removed here, by whoever finds it.
fn live_tickets(machine: &Path) -> Vec<String> {
    let queue = machine.join(QUEUE);
    let Ok(entries) = std::fs::read_dir(&queue) else {
        return Vec::new();
    };
    let mut live = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(TICKET) {
            continue;
        }
        let Ok(f) = OpenOptions::new().read(true).write(true).open(e.path()) else {
            continue;
        };
        if f.try_lock_exclusive().is_ok() {
            drop(f);
            let _ = std::fs::remove_file(e.path());
            let _ = std::fs::remove_file(queue.join(format!("{name}.json")));
        } else {
            live.push(name);
        }
    }
    live.sort();
    live
}

/// Every slot that can be locked now, locked.
fn free_slots(machine: &Path, slots: u32) -> anyhow::Result<Vec<(u32, File)>> {
    std::fs::create_dir_all(machine.join(SLOTS))?;
    let mut free = Vec::new();
    for n in 0..slots {
        let f = open_lock(&slot_lock(machine, n))?;
        if f.try_lock_exclusive().is_ok() {
            free.push((n, f));
        }
    }
    Ok(free)
}

/// Why the machine is too busy for another build now, on Linux; `None` when it is not.
fn machine_busy(e: &Effective) -> Option<String> {
    if let Some(load) = load_1min()
        && load > e.max_load
    {
        return Some(format!(
            "the 1-minute load is {load:.1}, above {:.1}",
            e.max_load
        ));
    }
    let want = u64::from(e.reserve_mem_gib) * 1024;
    if let Some(avail) = mem_available_mib()
        && avail < want
    {
        return Some(format!(
            "{avail} MiB of memory available, below the {} GiB reserve",
            e.reserve_mem_gib
        ));
    }
    None
}

fn load_1min() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn mem_available_mib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

/// Wait until this build may take a slot: no more than the free slots' number of waiting builds
/// sort before its ticket, a slot lock is taken, and (Linux) the load and memory allow it.
fn wait_for_slot(plan: &Plan, cmd: &[String]) -> anyhow::Result<Slot> {
    let ticket = take_ticket(plan, cmd)?;
    let mut said = String::new();
    loop {
        let live = live_tickets(&plan.machine);
        let mut free = free_slots(&plan.machine, plan.effective.slots)?;
        let why = if free.is_empty() {
            format!("all {} build slots are busy", plan.effective.slots)
        } else if !buildplan::may_take_slot(&ticket.name, &live, free.len()) {
            let ahead = live.iter().filter(|t| **t < ticket.name).count();
            format!("{ahead} waiting build(s) go first")
        } else if let Some(busy) = plan
            .machine_waits
            .then(|| machine_busy(&plan.effective))
            .flatten()
        {
            busy
        } else {
            let (n, lock) = free.swap_remove(0);
            drop(free);
            let holder = slot_holder(&plan.machine, n);
            let h = Holder {
                pid: std::process::id(),
                since: kanbanr_core::now_rfc3339(),
                worktree: plan.worktree.clone(),
                branch: plan.branch.clone(),
                task: plan.task.clone(),
                cmd: cmd.to_vec(),
            };
            let _ = std::fs::write(&holder, serde_json::to_vec(&h).unwrap_or_default());
            drop(ticket);
            return Ok(Slot {
                n,
                holder,
                _lock: lock,
            });
        };
        drop(free);
        if why != said {
            eprintln!("waiting for a build slot: {why}");
            said = why;
        }
        std::thread::sleep(POLL);
    }
}

// ---------------------------------------------------------------------------------------------
// Affected checks
// ---------------------------------------------------------------------------------------------

/// What `kanbanr build affected` reports, and `build check` runs.
#[derive(Debug, Clone, Serialize)]
pub struct AffectedView {
    /// The revision the change is measured from.
    pub base: String,
    /// The files changed since `base`, committed or not, repository-relative.
    pub changed: Vec<String>,
    #[serde(flatten)]
    pub affected: Affected,
}

/// The checks the change in the worktree at `repo` needs at `tier`: the files changed from `base`
/// (by default the scope's base: see [`default_base`]) to `HEAD`, plus those `git status` lists.
pub fn affected_here(
    store: &Store,
    project: &str,
    repo: &Path,
    tier: Tier,
    base: Option<&str>,
) -> anyhow::Result<AffectedView> {
    let facts = coderepo::worktree_facts(repo)
        .ok_or_else(|| anyhow::anyhow!("not in a git repository: checks follow a git change"))?;
    let root = facts.root.clone();
    let config = store.load_meta(project).ok().map(|p| p.config);
    let base = match base {
        Some(b) => b.to_string(),
        None => default_base(store, project, config.as_ref(), &facts),
    };
    let mut changed = coderepo::changed_files(&root, &base, "HEAD").map_err(|e| {
        anyhow::anyhow!("cannot compare with {base}: {e} (name another with --base REV)")
    })?;
    changed.extend(uncommitted(&root));
    let changed: Vec<String> = changed.into_iter().collect();
    let ws = workspace(&root)?;
    let effective = config
        .as_ref()
        .map(|c| c.build.clone())
        .unwrap_or_default()
        .effective(cpus(), &facts.main_root);
    let shape = repo_shape(&root, effective.line_check.clone());
    let mut affected = buildplan::affected(&changed, &ws, tier, &shape);
    if affected.checks.is_empty() && changed.is_empty() {
        affected.reason = format!("nothing changed since {base}");
    }
    Ok(AffectedView {
        base,
        changed,
        affected,
    })
}

/// The base a check compares with when none is named: under a task scope its item branch; under
/// an item scope the item's line (else main); otherwise main.
fn default_base(
    store: &Store,
    project: &str,
    config: Option<&kanbanr_core::config::ProjectConfig>,
    facts: &kanbanr_core::agentscope::WorktreeFacts,
) -> String {
    let main = match config {
        Some(c) => kanbanr_core::gitmodel::main_of(c, &facts.root),
        None => coderepo::default_branch(&facts.root),
    };
    let Some(scope) = bound_scope(store, facts) else {
        return main;
    };
    if scope.task.is_some() {
        // A task branch is `<item branch>--tl-001-t3`.
        return scope
            .branch
            .as_deref()
            .and_then(|b| b.rsplit_once("--").map(|(item, _)| item.to_string()))
            .unwrap_or(main);
    }
    let line = || -> Option<String> {
        let (model, _) = kanbanr_core::gitmodel::load(store, project, &facts.root).ok()?;
        let item = store.load_feature(project, &scope.item).ok()?;
        let releases = kanbanr_core::releases::load(store, project).unwrap_or_default();
        kanbanr_core::gitmodel::line_for(&model, &item, &releases)
    };
    line().unwrap_or(main)
}

/// The paths `git status` lists as changed or new, repository-relative; a rename lists both.
fn uncommitted(root: &Path) -> Vec<String> {
    // Untrimmed: the first entry's status may start with a space.
    let Ok(raw) = coderepo::git_raw(root, &["status", "--porcelain=v1", "-z", "-uall"]) else {
        return Vec::new();
    };
    let out = String::from_utf8_lossy(&raw);
    let mut paths = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        let Some(path) = f.get(3..) else { continue };
        paths.push(path.to_string());
        // A rename or copy is followed by its original path.
        if matches!(f.as_bytes()[0], b'R' | b'C')
            && let Some(from) = fields.next()
        {
            paths.push(from.to_string());
        }
    }
    paths
}

/// The workspace's crates, from `cargo metadata` in the workspace folder; none without one.
fn workspace(root: &Path) -> anyhow::Result<Vec<buildplan::CrateInfo>> {
    let dir = root.join(buildplan::CARGO_DIR);
    if !dir.join("Cargo.toml").is_file() {
        return Ok(Vec::new());
    }
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&dir)
        .output()
        .map_err(|e| anyhow::anyhow!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(buildplan::workspace_from_metadata(
        &String::from_utf8_lossy(&out.stdout),
        root,
    )?)
}

/// What the repository at `root` holds that the checks call on.
fn repo_shape(root: &Path, line_check: Option<Vec<String>>) -> RepoShape {
    let package = root.join("web").join("package.json");
    let web_checks: Vec<String> = std::fs::read(&package)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["scripts"].as_object().cloned())
        .map(|s| {
            s.keys()
                .filter(|k| k.starts_with("check:"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let docs_checks = ["check:docs", "check:markdown"]
        .iter()
        .filter(|c| web_checks.iter().any(|w| w == *c))
        .map(|c| c.to_string())
        .collect();
    RepoShape {
        test_script: root.join("scripts").join("test.sh").is_file(),
        web: package.is_file(),
        web_checks,
        docs_checks,
        script_tests: root.join("scripts").join("tests").is_dir(),
        line_check,
    }
}

/// Run `cmd` in `dir`, inheriting stdio. A program named by a relative path with a folder in it
/// (`scripts/test.sh`) is found from `dir`.
fn spawn_in(dir: &Path, cmd: &[String]) -> std::io::Result<i32> {
    let program = if cmd[0].contains('/') && Path::new(&cmd[0]).is_relative() {
        dir.join(&cmd[0]).into_os_string()
    } else {
        cmd[0].clone().into()
    };
    let status = std::process::Command::new(program)
        .args(&cmd[1..])
        .current_dir(dir)
        .status()?;
    Ok(exit_code(status))
}

/// Run the checks the change needs at `tier`, cheapest first, each through a build slot and
/// recorded as a `check` of that tier. Every check runs; the status is non-zero when any failed,
/// with a summary line per check.
pub fn check(
    store: &Store,
    project: &str,
    repo: &Path,
    tier: Tier,
    base: Option<&str>,
) -> anyhow::Result<i32> {
    let view = affected_here(store, project, repo, tier, base)?;
    let a = &view.affected;
    println!(
        "{} checks against {}: {}",
        tier.as_str(),
        view.base,
        a.reason
    );
    if a.checks.is_empty() {
        return Ok(0);
    }
    let root = coderepo::worktree_facts(repo)
        .map(|f| f.root)
        .unwrap_or_else(|| repo.to_path_buf());
    let plan = plan(store, project, repo, &RunRequest::default())?;
    let mut results: Vec<(String, i32, u64)> = Vec::new();
    for c in &a.checks {
        let req = RunRequest {
            cmd: c.argv.clone(),
            task: None,
            kind: "check".into(),
            tier: Some(tier.as_str().into()),
        };
        let dir = root.join(&c.dir);
        let line = format!("{} {}", c.name, c.argv.join(" "));
        match run_planned(&plan, &req, &|cmd| spawn_in(&dir, cmd)) {
            Ok((code, row)) => results.push((line, code, row.run_ms)),
            Err(e) => {
                eprintln!("{e}");
                results.push((line, 127, 0));
            }
        }
    }
    let failed = results.iter().filter(|(_, code, _)| *code != 0).count();
    println!();
    for (line, code, ms) in &results {
        let verdict = if *code == 0 {
            "ok  ".to_string()
        } else {
            format!("FAIL (exit {code})")
        };
        println!("{verdict} {line}  [{:.1} s]", *ms as f64 / 1000.0);
    }
    println!(
        "{} of {} checks passed",
        results.len() - failed,
        results.len()
    );
    Ok(i32::from(failed > 0))
}

// ---------------------------------------------------------------------------------------------
// status, log, stats
// ---------------------------------------------------------------------------------------------

/// One slot as `build status` shows it.
#[derive(Debug, Clone, Serialize)]
pub struct SlotView {
    pub slot: u32,
    pub busy: bool,
    /// Who holds it, when it is busy and its holder file could be read.
    pub holder: Option<Holder>,
}

/// One waiting build as `build status` shows it.
#[derive(Debug, Clone, Serialize)]
pub struct WaitingView {
    pub ticket: String,
    pub priority: String,
    pub holder: Option<Holder>,
}

/// What `kanbanr build status` reports.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub machine_dir: String,
    pub effective: Effective,
    pub slots: Vec<SlotView>,
    pub waiting: Vec<WaitingView>,
    /// The waits that apply on this platform, in words.
    pub waits: Vec<String>,
}

/// The project's slots, who holds them, who waits, and which waits apply here.
pub fn status(store: &Store, project: &str, repo: &Path) -> anyhow::Result<Status> {
    let plan = plan(store, project, repo, &RunRequest::default())?;
    status_of(&plan)
}

fn read_holder(path: &Path) -> Option<Holder> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn status_of(plan: &Plan) -> anyhow::Result<Status> {
    let m = &plan.machine;
    std::fs::create_dir_all(m.join(SLOTS))?;
    let mut slots = Vec::new();
    for n in 0..plan.effective.slots {
        let f = open_lock(&slot_lock(m, n))?;
        let busy = f.try_lock_exclusive().is_err();
        drop(f);
        slots.push(SlotView {
            slot: n,
            busy,
            holder: busy.then(|| read_holder(&slot_holder(m, n))).flatten(),
        });
    }
    let waiting = live_tickets(m)
        .into_iter()
        .map(|t| WaitingView {
            priority: match t.split('-').next() {
                Some("0") => "critical",
                Some("1") => "task",
                _ => "none",
            }
            .to_string(),
            holder: read_holder(&m.join(QUEUE).join(format!("{t}.json"))),
            ticket: t,
        })
        .collect();
    let e = &plan.effective;
    let waits = if plan.machine_waits {
        vec![
            format!(
                "a build waits while the 1-minute load is above {:.1}",
                e.max_load
            ),
            format!(
                "a build waits while less than {} GiB of memory is available",
                e.reserve_mem_gib
            ),
        ]
    } else {
        vec!["the load and memory waits are skipped on this platform".to_string()]
    };
    Ok(Status {
        machine_dir: m.display().to_string(),
        effective: e.clone(),
        slots,
        waiting,
        waits,
    })
}

/// `--since` as an instant: a bare date means its start (UTC).
pub fn since_instant(since: &str) -> String {
    let s = since.trim();
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    }
}

/// The repository's build log, oldest first, for `task` (all when `None`) since `since` (all when
/// empty).
pub fn log(repo: &Path, task: Option<&str>, since: &str) -> Vec<BuildRow> {
    let Some(dir) = git_common_dir(repo).map(|c| buildplan::repo_dir(&c)) else {
        return Vec::new();
    };
    filter_rows(buildplan::read_rows(&dir), task, since)
}

fn filter_rows(rows: Vec<BuildRow>, task: Option<&str>, since: &str) -> Vec<BuildRow> {
    let since = since_instant(since);
    rows.into_iter()
        .filter(|r| task.is_none_or(|t| r.task.as_deref() == Some(t)))
        .filter(|r| since.is_empty() || kanbanr_core::cmp_instants(&r.at, &since).is_ge())
        .collect()
}

/// Run time by kind, for `build stats`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KindStats {
    pub kind: String,
    pub runs: usize,
    pub run_p50_ms: u64,
    pub red: usize,
}

/// What `kanbanr build stats` reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    pub runs: usize,
    pub wait_p50_ms: u64,
    pub wait_p90_ms: u64,
    pub red: usize,
    pub kinds: Vec<KindStats>,
}

/// The nearest-rank percentile `p` (0 to 100) of `v`; 0 for none.
fn percentile(v: &mut [u64], p: u64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    let n = v.len() as u64;
    let rank = (p * n).div_ceil(100).max(1);
    v[usize::try_from(rank - 1).unwrap_or(0).min(v.len() - 1)]
}

/// Wait p50 and p90 over every row, and per kind its runs, run p50 and red count.
pub fn stats(rows: &[BuildRow]) -> Stats {
    let mut waits: Vec<u64> = rows.iter().map(|r| r.queued_ms).collect();
    let mut kinds: std::collections::BTreeMap<&str, Vec<&BuildRow>> = Default::default();
    for r in rows {
        kinds.entry(r.kind.as_str()).or_default().push(r);
    }
    Stats {
        runs: rows.len(),
        wait_p50_ms: percentile(&mut waits, 50),
        wait_p90_ms: percentile(&mut waits, 90),
        red: rows.iter().filter(|r| r.exit != 0).count(),
        kinds: kinds
            .into_iter()
            .map(|(k, rs)| {
                let mut run: Vec<u64> = rs.iter().map(|r| r.run_ms).collect();
                KindStats {
                    kind: k.to_string(),
                    runs: rs.len(),
                    run_p50_ms: percentile(&mut run, 50),
                    red: rs.iter().filter(|r| r.exit != 0).count(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A machine folder and a repository folder of this test's own, under the temp dir (L-41).
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "kanbanr-buildrun-{tag}-{}-{}",
            std::process::id(),
            arrival_ns()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn plan_in(dir: &Path, slots: u32, priority: Priority, task: Option<&str>) -> Plan {
        let mut effective = kanbanr_core::config::BuildConfig {
            slots: Some(slots),
            ..Default::default()
        }
        .effective(12, dir);
        // The machine's own load is not this test's business.
        effective.max_load = f64::MAX;
        effective.reserve_mem_gib = 0;
        Plan {
            machine: dir.join("machine"),
            repo_dir: Some(dir.join("repo")),
            effective,
            priority,
            task: task.map(String::from),
            scope: None,
            worktree: "wt".into(),
            branch: "b".into(),
            machine_waits: false,
        }
    }

    /// Sets its flag when dropped.
    struct Release<'a>(&'a Mutex<bool>);

    impl Drop for Release<'_> {
        fn drop(&mut self) {
            *self.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        }
    }

    fn req(cmd: &[&str]) -> RunRequest {
        RunRequest {
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            task: None,
            kind: "build".into(),
            tier: None,
        }
    }

    /// FTR-062 R-1: with 2 slots, 4 builds asked for at once never run more than 2 at a time,
    /// and every one runs.
    #[test]
    fn a_job_waits_for_a_free_slot_and_no_more_than_the_slots_run_at_once() {
        let dir = scratch("slots");
        let plan = plan_in(&dir, 2, Priority::None, None);
        let running = Mutex::new(0usize);
        let most = Mutex::new(0usize);
        let done = Mutex::new(0usize);
        let exec = |_: &[String]| -> std::io::Result<i32> {
            {
                let mut r = running.lock().unwrap();
                *r += 1;
                let mut m = most.lock().unwrap();
                *m = (*m).max(*r);
            }
            std::thread::sleep(Duration::from_millis(400));
            *running.lock().unwrap() -= 1;
            *done.lock().unwrap() += 1;
            Ok(0)
        };
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| run_planned(&plan, &req(&["x"]), &exec).unwrap());
            }
        });
        assert_eq!(*done.lock().unwrap(), 4, "every build ran");
        assert_eq!(
            *most.lock().unwrap(),
            2,
            "never more than the 2 slots at once"
        );
        let rows = buildplan::read_rows(&dir.join("repo"));
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.slot < 2), "{rows:?}");
        assert!(
            rows.iter().filter(|r| r.queued_ms >= 200).count() >= 2,
            "two of them waited: {rows:?}"
        );
        assert!(live_tickets(&plan.machine).is_empty(), "no ticket left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-062 R-2: with the one slot held, a task build asks first and a critical-path build
    /// second; when the slot is freed the critical-path build runs first.
    #[test]
    fn a_critical_path_task_takes_the_next_free_slot_first() {
        let dir = scratch("priority");
        let hold = plan_in(&dir, 1, Priority::None, None);
        let task = plan_in(&dir, 1, Priority::Task, Some("FTR-1/TL-001/T2"));
        let critical = plan_in(&dir, 1, Priority::Critical, Some("FTR-1/TL-001/T1"));
        let order = Mutex::new(Vec::<String>::new());
        let release = Mutex::new(false);
        let tickets = |n: usize| {
            let t0 = Instant::now();
            while live_tickets(&hold.machine).len() < n {
                assert!(
                    t0.elapsed() < Duration::from_secs(10),
                    "tickets never appeared"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        std::thread::scope(|s| {
            s.spawn(|| {
                run_planned(&hold, &req(&["hold"]), &|_| {
                    while !*release.lock().unwrap() {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(0)
                })
                .unwrap()
            });
            // Frees the holder however this closure ends, so a failed assertion fails the test
            // rather than hanging it.
            let release_on_exit = Release(&release);
            // The holder has its slot once its own ticket is gone and a slot is busy.
            let t0 = Instant::now();
            while free_slots(&hold.machine, 1).map(|f| f.len()).unwrap_or(1) == 1 {
                assert!(t0.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            let record = |who: &'static str| {
                let order = &order;
                move |_: &[String]| -> std::io::Result<i32> {
                    order.lock().unwrap().push(who.to_string());
                    Ok(0)
                }
            };
            let task_exec = record("task");
            let critical_exec = record("critical");
            s.spawn(move || run_planned(&task, &req(&["t"]), &task_exec).unwrap());
            tickets(1);
            s.spawn(move || run_planned(&critical, &req(&["c"]), &critical_exec).unwrap());
            tickets(2);
            drop(release_on_exit);
        });
        assert_eq!(*order.lock().unwrap(), ["critical", "task"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-062 R-3: a run under a task writes one row naming the task, its slot, how long it
    /// waited (above 0 when it waited) and its exit status.
    #[test]
    fn each_run_is_recorded_with_its_task_wait_and_run_time() {
        let dir = scratch("rows");
        let plan = plan_in(&dir, 1, Priority::Task, Some("FTR-9/TL-001/T4"));
        // A real command, so the exit status is the child's own.
        let (code, row) = run_planned(&plan, &req(&["git", "--version"]), &spawn).unwrap();
        assert_eq!(code, 0);
        assert_eq!(row.task.as_deref(), Some("FTR-9/TL-001/T4"));
        assert_eq!(row.exit, 0);
        assert_eq!(row.cmd, ["git", "--version"]);
        let (code, _) = run_planned(&plan, &req(&["git", "no-such-subcommand"]), &spawn).unwrap();
        assert_ne!(code, 0, "the child's failure is the run's status");

        // One build holds the only slot for 300 ms; the second waits for it.
        let waited = std::thread::scope(|s| {
            let held = s.spawn(|| {
                run_planned(&plan, &req(&["hold"]), &|_| {
                    std::thread::sleep(Duration::from_millis(300));
                    Ok(0)
                })
                .unwrap()
            });
            let t0 = Instant::now();
            while free_slots(&plan.machine, 1).map(|f| f.len()).unwrap_or(1) == 1 {
                assert!(t0.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(5));
            }
            let (_, row) = run_planned(&plan, &req(&["after"]), &|_| Ok(3)).unwrap();
            held.join().unwrap();
            row
        });
        assert!(waited.queued_ms > 0, "{waited:?}");
        assert_eq!(waited.exit, 3);
        assert_eq!(waited.slot, 0);

        let rows = buildplan::read_rows(&dir.join("repo"));
        assert_eq!(rows.len(), 4, "one row per run: {rows:?}");
        assert!(
            rows.iter()
                .all(|r| r.task.as_deref() == Some("FTR-9/TL-001/T4"))
        );
        assert_eq!(rows.last().unwrap().exit, 3);
        assert_eq!(rows[0].worktree, "wt");
        assert_eq!(rows[0].branch, "b");
        let totals = buildplan::task_totals(&rows, "FTR-9/TL-001/T4", "");
        assert_eq!(totals.runs, 4);
        assert_eq!(totals.red, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-062 R-10: taking a free slot, recording the row and giving the slot back adds under
    /// 200 ms to a command (median of five).
    #[test]
    fn a_free_slot_adds_under_200_ms() {
        let dir = scratch("budget");
        let plan = plan_in(&dir, 2, Priority::None, None);
        let mut took: Vec<u64> = (0..5)
            .map(|_| {
                let t0 = Instant::now();
                run_planned(&plan, &req(&["noop"]), &|_| Ok(0)).unwrap();
                millis(t0.elapsed())
            })
            .collect();
        let median = percentile(&mut took, 50);
        assert!(median < 200, "a free slot added {median} ms: {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
