//! `kanbanr build` (FTR-062): agents' builds share one machine. A command run through
//! `kanbanr build run` waits for one of the project's build slots, in priority order (a task on the
//! critical path first), then runs, and its wait and run time go to the repository's build log.
//!
//! The state is the machine's and the repository's, never the board's: slot locks and waiting
//! tickets in [`buildplan::machine_dir`], the log in `<git-common-dir>/kanbanr/builds/`. A slot is a
//! held file lock, so a crashed kanbanr gives its slot back with its process. A child is not killed
//! with kanbanr: it is in the terminal's process group, so Ctrl-C reaches both, but a kanbanr killed
//! alone leaves its child running, unaccounted for.
//!
//! A command runs with [`buildplan::build_env`]: the job and test-thread caps, and the project's
//! own sccache server, whose base directories are the main checkout and every worktree, so a
//! compile in one worktree serves the others. Before a cargo command the server is (re)started
//! when its base directories are out of date, but only when no other build holds a slot.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fs2::FileExt;
use kanbanr_core::Store;
use kanbanr_core::buildplan::{
    self, Affected, BuildRow, Effective, Priority, RepoShape, Restart, Tier,
};
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
    /// The project, which names its compiler cache folder.
    project: String,
    /// The main checkout: the cache server's first base directory, and where worktrees are listed.
    main_root: PathBuf,
    /// `sccache`, when it is on `PATH`.
    sccache: Option<PathBuf>,
    /// Whether `mold` is on `PATH`.
    mold_found: bool,
}

/// The environment a command runs with: name, value.
type Env = [(String, String)];

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
    let path = std::env::var_os("PATH").unwrap_or_default();
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
        project: project.to_string(),
        main_root,
        sccache: find_tool("sccache", &path),
        mold_found: find_tool("mold", &path).is_some(),
    })
}

/// The program `name` in one of the folders of `path` (a `PATH` value), if any.
fn find_tool(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|d| {
            let mut c = vec![d.join(name)];
            if cfg!(windows) {
                c.push(d.join(format!("{name}.exe")));
            }
            c
        })
        .find(|p| p.is_file())
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

/// Run the command itself with `env` added, inheriting stdio, in the current directory.
fn spawn(cmd: &[String], env: &Env) -> std::io::Result<i32> {
    let status = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .envs(env.iter().map(|(k, v)| (k, v)))
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

/// Wait for a slot, make sure of the cache server (a cargo command), run `exec` with the build
/// environment, record the row. Returns the exit status and the row written.
fn run_planned(
    plan: &Plan,
    req: &RunRequest,
    exec: &dyn Fn(&[String], &Env) -> std::io::Result<i32>,
) -> anyhow::Result<(i32, BuildRow)> {
    let asked = Instant::now();
    let at = kanbanr_core::now_rfc3339();
    let slot = wait_for_slot(plan, &req.cmd)?;
    let queued_ms = millis(asked.elapsed());
    let mem_available_mib = plan.machine_waits.then(mem_available_mib).flatten();
    let env = buildplan::build_env(
        &plan.effective,
        plan.sccache.is_some(),
        plan.mold_found,
        cfg!(target_os = "linux"),
        &cache_dir(plan),
        None,
    );
    let mut notes = env.notes.clone();
    if is_cargo(&req.cmd)
        && let Some(n) = ensure_cache_server(plan, slot.n)
    {
        notes.push(n);
    }
    let started = Instant::now();
    let result = exec(&req.cmd, &env.set);
    let run_ms = millis(started.elapsed());
    let exit = match &result {
        Ok(c) => *c,
        Err(e) => {
            notes.push(format!("could not start: {e}"));
            127
        }
    };
    let note = notes.join("; ");
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
// The compiler cache server
// ---------------------------------------------------------------------------------------------

/// Is `cmd` a cargo command (the program's file name is `cargo`)?
fn is_cargo(cmd: &[String]) -> bool {
    cmd.first()
        .and_then(|c| Path::new(c).file_stem())
        .is_some_and(|s| s == "cargo")
}

fn cache_dir(plan: &Plan) -> PathBuf {
    buildplan::sccache_dir(&plan.machine, &plan.project, &plan.main_root)
}

/// The repository's checkouts, from `git worktree list` in the main checkout: absolute paths.
fn worktree_roots(main_root: &Path) -> Vec<PathBuf> {
    coderepo::git(main_root, &["worktree", "list", "--porcelain"])
        .map(|out| {
            out.lines()
                .filter_map(|l| l.strip_prefix("worktree "))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Is a slot other than `held` taken by another build?
fn others_building(machine: &Path, slots: u32, held: u32) -> bool {
    (0..slots).filter(|n| *n != held).any(|n| {
        open_lock(&slot_lock(machine, n))
            .map(|f| f.try_lock_exclusive().is_err())
            .unwrap_or(false)
    })
}

/// The project's sccache server's settings, as environment variables for `sccache` itself.
fn server_env(plan: &Plan, base_dirs: &std::ffi::OsStr) -> Vec<(String, std::ffi::OsString)> {
    let e = &plan.effective;
    vec![
        (
            "SCCACHE_SERVER_PORT".into(),
            e.sccache_port.to_string().into(),
        ),
        ("SCCACHE_DIR".into(), cache_dir(plan).into_os_string()),
        (
            "SCCACHE_CACHE_SIZE".into(),
            format!("{}G", e.cache_gib).into(),
        ),
        ("SCCACHE_BASEDIRS".into(), base_dirs.to_os_string()),
    ]
}

/// `sccache <arg>` against the project's server, with no terminal attached: its output when it
/// succeeded. The server's settings go with every call, so a server that `--show-stats` starts on
/// its own starts with them.
fn sccache_call(sccache: &Path, env: &[(String, std::ffi::OsString)], arg: &str) -> Option<String> {
    let out = std::process::Command::new(sccache)
        .arg(arg)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Make sure the project's sccache server runs with the main checkout and every worktree as its
/// base directories, before a cargo build that holds slot `slot_held`. A server with other base
/// directories is restarted only when no other build holds a slot; otherwise it is left, and the
/// note returned says so (it goes in the build's row).
fn ensure_cache_server(plan: &Plan, slot_held: u32) -> Option<String> {
    let sccache = plan.sccache.as_deref().filter(|_| plan.effective.sccache)?;
    let wanted = buildplan::base_dirs(&plan.main_root, &worktree_roots(&plan.main_root));
    let joined = match std::env::join_paths(&wanted) {
        Ok(j) => j,
        Err(e) => return Some(format!("the cache server's base directories: {e}")),
    };
    let env = server_env(plan, &joined);
    let running =
        sccache_call(sccache, &env, "--show-stats").and_then(|s| buildplan::parse_base_dirs(&s));
    let others = others_building(&plan.machine, plan.effective.slots, slot_held);
    match buildplan::needs_restart(running.as_deref(), &wanted, others) {
        Restart::No => None,
        Restart::Deferred => Some(format!(
            "the cache server was not restarted for the {} checkouts now: another build is \
             running, so a new worktree misses the shared cache until a build finds the slots free",
            wanted.len()
        )),
        Restart::Yes => {
            let _ = sccache_call(sccache, &env, "--stop-server");
            sccache_call(sccache, &env, "--start-server")
                .is_none()
                .then(|| "the cache server could not be started".to_string())
        }
    }
}

/// The project's compiler cache as `build status` shows it.
#[derive(Debug, Clone, Serialize)]
pub struct CacheView {
    /// sccache is on in the project's settings.
    pub enabled: bool,
    /// sccache is on `PATH`.
    pub found: bool,
    pub port: u16,
    pub dir: String,
    /// The server answered.
    pub running: bool,
    /// The base directories the server runs with.
    pub base_dirs: Vec<String>,
    /// The checkouts it should run with: the main checkout and every worktree.
    pub wanted: Vec<String>,
    pub size: Option<String>,
    pub max_size: Option<String>,
}

fn cache_view(plan: &Plan) -> CacheView {
    let e = &plan.effective;
    let wanted = buildplan::base_dirs(&plan.main_root, &worktree_roots(&plan.main_root));
    let stats = plan
        .sccache
        .as_deref()
        .filter(|_| e.sccache)
        .and_then(|sc| {
            let joined = std::env::join_paths(&wanted).ok()?;
            sccache_call(sc, &server_env(plan, &joined), "--show-stats")
        });
    let text = stats.as_deref().unwrap_or("");
    let shown = |l: &str| buildplan::stats_value(text, l).map(String::from);
    CacheView {
        enabled: e.sccache,
        found: plan.sccache.is_some(),
        port: e.sccache_port,
        dir: cache_dir(plan).display().to_string(),
        running: stats.is_some(),
        base_dirs: buildplan::parse_base_dirs(text)
            .unwrap_or_default()
            .iter()
            .map(|d| d.display().to_string())
            .collect(),
        wanted: wanted.iter().map(|d| d.display().to_string()).collect(),
        size: shown("Cache size"),
        max_size: shown("Max cache size"),
    }
}

/// The compiler cache's lines of `build status`.
pub fn cache_lines(c: &CacheView) -> Vec<String> {
    if !c.enabled {
        return vec!["compiler cache: off in the project's build settings".into()];
    }
    if !c.found {
        return vec!["compiler cache: sccache not found, so builds compile without it".into()];
    }
    if !c.running {
        return vec![format!(
            "compiler cache: sccache on port {}, not running (the next cargo build starts it); cache in {}",
            c.port, c.dir
        )];
    }
    let mut lines = vec![format!(
        "compiler cache: sccache on port {}, {} of {} in {}",
        c.port,
        c.size.as_deref().unwrap_or("?"),
        c.max_size.as_deref().unwrap_or("?"),
        c.dir
    )];
    let same = {
        let set = |v: &[String]| {
            v.iter()
                .map(PathBuf::from)
                .collect::<std::collections::BTreeSet<_>>()
        };
        set(&c.base_dirs) == set(&c.wanted)
    };
    lines.push(format!(
        "  base directories ({}){}:",
        c.base_dirs.len(),
        if same {
            String::new()
        } else {
            format!(
                ", out of date: {} checkouts now, restarted when no build runs",
                c.wanted.len()
            )
        }
    ));
    lines.extend(c.base_dirs.iter().map(|d| format!("    {d}")));
    lines
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

/// Run `cmd` in `dir` with `env` added, inheriting stdio. A program named by a relative path with a folder in it
/// (`scripts/test.sh`) is found from `dir`.
fn spawn_in(dir: &Path, cmd: &[String], env: &Env) -> std::io::Result<i32> {
    let program = if cmd[0].contains('/') && Path::new(&cmd[0]).is_relative() {
        dir.join(&cmd[0]).into_os_string()
    } else {
        cmd[0].clone().into()
    };
    let status = std::process::Command::new(program)
        .args(&cmd[1..])
        .envs(env.iter().map(|(k, v)| (k, v)))
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
        match run_planned(&plan, &req, &|cmd, env| spawn_in(&dir, cmd, env)) {
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
    /// The project's compiler cache server.
    pub cache: CacheView,
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
        cache: cache_view(plan),
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
            project: "p".into(),
            main_root: dir.to_path_buf(),
            // No compiler cache unless a test brings its own.
            sccache: None,
            mold_found: false,
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
        let exec = |_: &[String], _: &Env| -> std::io::Result<i32> {
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
                run_planned(&hold, &req(&["hold"]), &|_, _| {
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
                move |_: &[String], _: &Env| -> std::io::Result<i32> {
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
                run_planned(&plan, &req(&["hold"]), &|_, _| {
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
            let (_, row) = run_planned(&plan, &req(&["after"]), &|_, _| Ok(3)).unwrap();
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

    /// `git -c … <args>` in `dir`, which must succeed.
    #[cfg(unix)]
    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// FTR-062 R-4: a stub `sccache` logs each call. With no server, the first cargo build starts
    /// one with the checkouts as base directories; a new worktree stops and restarts it with both
    /// roots; nothing new leaves it alone; with another slot held a new worktree defers the
    /// restart, with a note.
    #[cfg(unix)]
    #[test]
    fn the_cache_server_is_restarted_for_new_worktrees_only_when_no_build_runs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::fs::canonicalize(scratch("sccache")).unwrap();
        let main = dir.join("main");
        std::fs::create_dir_all(&main).unwrap();
        git_ok(&main, &["init", "-q"]);
        git_ok(&main, &["commit", "-q", "--allow-empty", "-m", "start"]);

        // The stub keeps the running server's base directories in a file, prints them as 0.17
        // does, and logs every call with the base directories it was given.
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stub = bin.join("sccache");
        let log = dir.join("sccache.log");
        let state = dir.join("server");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\n\
                 echo \"$1 $SCCACHE_SERVER_PORT $SCCACHE_BASEDIRS\" >> '{log}'\n\
                 case \"$1\" in\n\
                 --show-stats) [ -f '{state}' ] || exit 2\n\
                   echo \"Base directories   $(sed 's#:#/, #g; s#$#/#' '{state}')\"\n\
                   echo 'Cache size          1 GiB' ;;\n\
                 --start-server) printf '%s\\n' \"$SCCACHE_BASEDIRS\" > '{state}' ;;\n\
                 --stop-server) rm -f '{state}' ;;\n\
                 esac\n",
                log = log.display(),
                state = state.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut plan = plan_in(&dir, 2, Priority::None, None);
        plan.main_root = main.clone();
        plan.sccache = find_tool("sccache", bin.as_os_str());
        assert_eq!(
            plan.sccache.as_deref(),
            Some(stub.as_path()),
            "found on PATH"
        );
        let port = plan.effective.sccache_port.to_string();
        let calls = || -> Vec<String> {
            std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        };
        // The base directories a call was given, as paths.
        let dirs_of = |line: &str| -> Vec<PathBuf> {
            let v = line.splitn(3, ' ').nth(2).unwrap_or("");
            std::env::split_paths(v).collect()
        };
        let calls_since = |n: usize| calls().split_off(n);

        // No server: the cargo build starts one, and runs with the cache's environment.
        let seen = Mutex::new(Vec::<(String, String)>::new());
        let (code, row) = run_planned(&plan, &req(&["cargo", "build"]), &|_, env| {
            *seen.lock().unwrap() = env.to_vec();
            Ok(0)
        })
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(row.note, "", "{row:?}");
        let env = seen.lock().unwrap().clone();
        let has = |k: &str, v: &str| env.iter().any(|(a, b)| a == k && b == v);
        assert!(has("RUSTC_WRAPPER", "sccache"), "{env:?}");
        assert!(has("SCCACHE_SERVER_PORT", &port), "{env:?}");
        assert!(has("CARGO_BUILD_JOBS", &plan.effective.jobs.to_string()));
        let c = calls();
        assert_eq!(
            c.iter()
                .map(|l| l.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            ["--show-stats", "--stop-server", "--start-server"],
            "{c:?}"
        );
        assert!(c.iter().all(|l| l.split(' ').nth(1) == Some(port.as_str())));
        assert_eq!(dirs_of(&c[2]), std::slice::from_ref(&main));

        // A command that is not cargo leaves the server alone.
        let n = calls().len();
        run_planned(&plan, &req(&["npm", "run", "build"]), &|_, _| Ok(0)).unwrap();
        assert!(calls_since(n).is_empty());

        // A new worktree: stopped, then started with both roots.
        let wt1 = dir.join("wt1");
        git_ok(
            &main,
            &["worktree", "add", "-q", "--detach", wt1.to_str().unwrap()],
        );
        let n = calls().len();
        assert_eq!(ensure_cache_server(&plan, 0), None);
        let c = calls_since(n);
        assert_eq!(
            c.iter()
                .map(|l| l.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            ["--show-stats", "--stop-server", "--start-server"],
            "{c:?}"
        );
        let mut want = vec![main.clone(), wt1.clone()];
        want.sort();
        assert_eq!(dirs_of(&c[2]), want);

        // Nothing new: only asked.
        let n = calls().len();
        assert_eq!(ensure_cache_server(&plan, 0), None);
        assert_eq!(calls_since(n).len(), 1, "only --show-stats");

        // Another build holds slot 1: a new worktree does not restart the server.
        let wt2 = dir.join("wt2");
        git_ok(
            &main,
            &["worktree", "add", "-q", "--detach", wt2.to_str().unwrap()],
        );
        std::fs::create_dir_all(plan.machine.join(SLOTS)).unwrap();
        let other = open_lock(&slot_lock(&plan.machine, 1)).unwrap();
        other.lock_exclusive().unwrap();
        let n = calls().len();
        let note = ensure_cache_server(&plan, 0).expect("a note for the deferred restart");
        assert!(note.contains("another build is running"), "{note}");
        let c = calls_since(n);
        assert!(
            c.iter().all(|l| l.starts_with("--show-stats")),
            "no restart while another build runs: {c:?}"
        );
        // The cargo build that defers says so in its row.
        let n = calls().len();
        let (_, row) = run_planned(&plan, &req(&["cargo", "test"]), &|_, _| Ok(0)).unwrap();
        assert!(row.note.contains("another build is running"), "{row:?}");
        assert!(calls_since(n).iter().all(|l| l.starts_with("--show-stats")));

        // Once the other build is done, the next one restarts it with all three.
        drop(other);
        let n = calls().len();
        assert_eq!(ensure_cache_server(&plan, 0), None);
        let c = calls_since(n);
        let mut want = vec![main.clone(), wt1, wt2];
        want.sort();
        assert_eq!(dirs_of(c.last().unwrap()), want, "{c:?}");

        // status shows the server and its base directories.
        let st = status_of(&plan).unwrap();
        assert!(st.cache.running && st.cache.found && st.cache.enabled);
        assert_eq!(st.cache.port, plan.effective.sccache_port);
        assert_eq!(st.cache.size.as_deref(), Some("1 GiB"));
        assert_eq!(
            st.cache
                .base_dirs
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
            want
        );
        let lines = cache_lines(&st.cache);
        assert!(lines[0].contains(&format!("port {port}")), "{lines:?}");
        assert_eq!(lines.len(), 2 + 3, "{lines:?}");
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
                run_planned(&plan, &req(&["noop"]), &|_, _| Ok(0)).unwrap();
                millis(t0.elapsed())
            })
            .collect();
        let median = percentile(&mut took, 50);
        assert!(median < 200, "a free slot added {median} ms: {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
