//! The daemon's background worker (FTR-045): docs drift is computed after each board or code change
//! and kept, with the revisions it was computed at, so a page reads the latest result at once instead
//! of waiting the ~49 s a run takes. Before each drift run the worker warms the summary reads the
//! memo keeps (DEF-081, A1 shared API §5).
//!
//! The result lives in this process's memory only: nothing is written, inside or outside the board
//! (ADR-0005), and a restarted daemon answers "computing" until its first run (R-10).

use crate::AppState;
use crate::memo;
use kanbanr_core::Store;
use kanbanr_core::docdrift::{AsOf, Latest, LatestState, NEEDS_REPO, Report};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// How long after the last trigger a run starts: a `kanbanr batch` lands within a second or two.
pub const DEBOUNCE_MS: u64 = 2_000;
/// How often each code repository's HEAD is read for a new commit.
pub const HEAD_POLL_SECS: u64 = 30;

/// What computes drift; the daemon's is [`CoreRunner`], tests pass a fake.
pub trait DriftRunner: Send + Sync + 'static {
    fn run(&self, store: &Store, project: &str, repo: &Path) -> kanbanr_core::Result<Report>;
}

/// The daemon's runner: the board-wide report from the default `since`.
pub struct CoreRunner;

impl DriftRunner for CoreRunner {
    fn run(&self, store: &Store, project: &str, repo: &Path) -> kanbanr_core::Result<Report> {
        kanbanr_core::docdrift::report(store, project, None, None, Some(repo))
    }
}

/// The worker's intervals: the constants in the daemon, shorter in tests.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub debounce: Duration,
    pub poll: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            debounce: Duration::from_millis(DEBOUNCE_MS),
            poll: Duration::from_secs(HEAD_POLL_SECS),
        }
    }
}

/// The canonical dispatch paths warmed before drift on every pass (A1 shared API §5). Other items add
/// lines here.
pub fn warm_routes(project: &str) -> Vec<String> {
    vec![
        format!("/projects/{project}/index"),
        // The Review page's counts (FTR-038), so its first open after a change is warm.
        format!("/projects/{project}/review/summary"),
        // The Overview and the Needs you badge (FTR-048), so the landing page opens warm.
        format!("/projects/{project}/overview"),
        format!("/projects/{project}/attention"),
        // The Plan page's counts (FTR-043), so its tab labels draw warm.
        format!("/projects/{project}/plan/summary"),
        // The Releases page's counts (FTR-041), so its tab labels draw warm.
        format!("/projects/{project}/releases/summary"),
        // The Charter page's progress read (FTR-076); it does not read the code repository.
        format!("/projects/{project}/charter/progress"),
    ]
}

/// One project's background state.
#[derive(Default)]
struct ProjectState {
    repo: Option<PathBuf>,
    last: Option<(Report, AsOf)>,
    running: bool,
    queued: bool,
    due: Option<Instant>,
    /// The last failure's error and time (RFC 3339).
    failure: Option<(String, String)>,
    seen_code_head: String,
}

/// What the worker tasks share.
struct Inner {
    st: AppState,
    runner: Arc<dyn DriftRunner>,
    timing: Timing,
    projects: Mutex<HashMap<String, ProjectState>>,
    notify: Notify,
}

/// The background worker. Dropping it stops its tasks; a blocking run in flight is abandoned and
/// writes nothing.
pub struct Background {
    inner: Arc<Inner>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Background {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl Background {
    /// Spawn the worker on the current tokio runtime. `repos`: the projects with a code repository
    /// (today the daemon's one `code_repo`). It schedules a first pass for every listed project at
    /// start.
    pub fn start(
        st: AppState,
        runner: Arc<dyn DriftRunner>,
        repos: Vec<(String, PathBuf)>,
    ) -> Arc<Background> {
        Self::start_with(st, runner, repos, Timing::default())
    }

    /// `start` with its intervals given, for tests.
    pub fn start_with(
        st: AppState,
        runner: Arc<dyn DriftRunner>,
        repos: Vec<(String, PathBuf)>,
        timing: Timing,
    ) -> Arc<Background> {
        let now = Instant::now();
        let projects = repos
            .into_iter()
            .map(|(project, repo)| {
                let seen_code_head = kanbanr_core::git::head_commit(&repo).unwrap_or_default();
                let state = ProjectState {
                    repo: Some(repo),
                    due: Some(now),
                    seen_code_head,
                    ..ProjectState::default()
                };
                (project, state)
            })
            .collect();
        let inner = Arc::new(Inner {
            st,
            runner,
            timing,
            projects: Mutex::new(projects),
            notify: Notify::new(),
        });
        // Subscribed before any task runs, so no change after start is missed.
        let slices = inner.st.slices.subscribe();
        let tasks = vec![
            tokio::spawn(worker(Arc::clone(&inner))),
            tokio::spawn(head_poll(Arc::clone(&inner))),
            tokio::spawn(on_slices(Arc::clone(&inner), slices)),
        ];
        Arc::new(Background { inner, tasks })
    }

    /// A trigger for `project` (from a slices event or a HEAD move): debounced by DEBOUNCE_MS. A
    /// project the worker does not keep is ignored.
    pub fn poke(&self, project: &str) {
        self.inner.poke(project);
    }

    /// The latest drift for `project`, for the route. Never blocks on a run.
    pub fn latest(&self, project: &str, with_flags: bool) -> Latest {
        let board_head =
            kanbanr_core::git::head_commit(memo::store_for(&self.inner.st, project).data_dir())
                .unwrap_or_default();
        let map = self.inner.lock();
        let Some(ps) = map.get(project).filter(|ps| ps.repo.is_some()) else {
            return Latest {
                state: LatestState::Unavailable,
                as_of: None,
                recomputing: false,
                count: 0,
                report: None,
                reason: NEEDS_REPO.to_string(),
                failed_at: String::new(),
            };
        };
        let (reason, failed_at) = ps.failure.clone().unwrap_or_default();
        match &ps.last {
            None => Latest {
                state: if ps.failure.is_some() {
                    LatestState::Failed
                } else {
                    LatestState::Computing
                },
                as_of: None,
                recomputing: ps.running,
                count: 0,
                report: None,
                reason,
                failed_at,
            },
            Some((report, as_of)) => {
                // Stale while a run is due, queued or going (the schema's `stale`).
                let pending = ps.due.is_some() || ps.queued || ps.running;
                let fresh = as_of.board_rev == board_head
                    && as_of.code_rev == ps.seen_code_head
                    && !pending;
                Latest {
                    state: if fresh {
                        LatestState::Fresh
                    } else {
                        LatestState::Stale
                    },
                    as_of: Some(as_of.clone()),
                    recomputing: ps.running,
                    count: report.flags.len(),
                    report: with_flags.then(|| report.clone()),
                    reason,
                    failed_at,
                }
            }
        }
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ProjectState>> {
        self.projects.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn poke(&self, project: &str) {
        {
            let mut map = self.lock();
            let Some(ps) = map.get_mut(project) else {
                return;
            };
            ps.due = Some(Instant::now() + self.timing.debounce);
        }
        self.notify.notify_one();
    }
}

/// The worker loop: sleep until the earliest due project or a notification, then start a pass for
/// each due project that is not running, and queue one more for each that is.
async fn worker(inner: Arc<Inner>) {
    loop {
        let now = Instant::now();
        let mut start = Vec::new();
        let mut next: Option<Instant> = None;
        {
            let mut map = inner.lock();
            for (project, ps) in map.iter_mut() {
                match ps.due {
                    Some(due) if due <= now => {
                        ps.due = None;
                        if ps.running {
                            ps.queued = true;
                        } else {
                            ps.running = true;
                            start.push(project.clone());
                        }
                    }
                    Some(due) => next = Some(next.map_or(due, |n| n.min(due))),
                    None => {}
                }
            }
        }
        for project in start {
            tokio::spawn(pass(Arc::clone(&inner), project));
        }
        match next {
            Some(at) => {
                tokio::select! {
                    _ = inner.notify.notified() => {}
                    _ = tokio::time::sleep_until(at.into()) => {}
                }
            }
            None => inner.notify.notified().await,
        }
    }
}

/// Every `poll`, read each code repository's HEAD; a new one pokes its project.
async fn head_poll(inner: Arc<Inner>) {
    let mut every = tokio::time::interval(inner.timing.poll);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    every.tick().await;
    loop {
        every.tick().await;
        let repos: Vec<(String, PathBuf)> = inner
            .lock()
            .iter()
            .filter_map(|(p, ps)| ps.repo.clone().map(|r| (p.clone(), r)))
            .collect();
        for (project, repo) in repos {
            let head = tokio::task::spawn_blocking(move || {
                kanbanr_core::git::head_commit(&repo).unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            let moved = {
                let mut map = inner.lock();
                match map.get_mut(&project) {
                    Some(ps) if ps.seen_code_head != head => {
                        ps.seen_code_head = head;
                        true
                    }
                    _ => false,
                }
            };
            if moved {
                inner.poke(&project);
            }
        }
    }
}

/// The tag the worker's own `Slices` carries after a run (A1 shared API §2).
const DRIFT_TAG: &str = "drift";

/// Every `Slices` for a project the worker keeps pokes it (DEF-081's slices). The worker's own
/// event, tagged only `drift`, is ignored, or each pass would start the next. Missed events (the
/// channel lagged) poke every project, since what changed is no longer known.
async fn on_slices(
    inner: Arc<Inner>,
    mut rx: tokio::sync::broadcast::Receiver<crate::watcher::Slices>,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match rx.recv().await {
            Ok(s) => {
                if !s.tags.is_empty() && s.tags.iter().all(|t| t == DRIFT_TAG) {
                    continue;
                }
                inner.poke(&s.project);
            }
            Err(RecvError::Lagged(_)) => {
                let projects: Vec<String> = inner.lock().keys().cloned().collect();
                for p in projects {
                    inner.poke(&p);
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// One pass for `project`: warm, then drift (only with a repo), then finish.
async fn pass(inner: Arc<Inner>, project: String) {
    let repo = inner.lock().get(&project).and_then(|ps| ps.repo.clone());
    let blocking = Arc::clone(&inner);
    let p = project.clone();
    let outcome =
        tokio::task::spawn_blocking(move || run_pass(&blocking, &p, repo.as_deref())).await;
    let outcome = match outcome {
        Ok(o) => o,
        Err(_) => Some(Err("drift run panicked".to_string())),
    };
    let mut finished = false;
    let notify = {
        let mut map = inner.lock();
        let Some(ps) = map.get_mut(&project) else {
            return;
        };
        match outcome {
            Some(Ok((report, as_of))) => {
                ps.last = Some((report, as_of));
                ps.failure = None;
                finished = true;
            }
            Some(Err(e)) => ps.failure = Some((e, kanbanr_core::now_rfc3339())),
            None => {}
        }
        ps.running = false;
        if ps.queued {
            ps.queued = false;
            ps.due = Some(Instant::now());
            true
        } else {
            false
        }
    };
    // Open pages re-read the latest result: the worker alone sends the `drift` tag.
    if finished {
        let _ = inner.st.slices.send(crate::watcher::Slices {
            r#gen: inner.st.gens.of(Some(&project)),
            project,
            tags: vec![DRIFT_TAG.to_string()],
        });
    }
    if notify {
        inner.notify.notify_one();
    }
}

/// The blocking part of a pass. `None` when the project has no repo, so no drift ran.
fn run_pass(
    inner: &Inner,
    project: &str,
    repo: Option<&Path>,
) -> Option<Result<(Report, AsOf), String>> {
    warm(&inner.st, project);
    let repo = repo?;
    let store = memo::store_for(&inner.st, project);
    let board_rev = kanbanr_core::git::head_commit(store.data_dir()).unwrap_or_default();
    let code_rev = kanbanr_core::git::head_commit(repo).unwrap_or_default();
    let start = Instant::now();
    let result = inner.runner.run(store, project, repo);
    let took_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    Some(
        result
            .map(|report| {
                let as_of = AsOf {
                    board_rev,
                    code_rev,
                    finished_at: kanbanr_core::now_rfc3339(),
                    took_ms,
                };
                (report, as_of)
            })
            .map_err(|e| e.to_string()),
    )
}

/// Compute and keep each warm route that the memo does not already hold at its current stamp.
fn warm(st: &AppState, project: &str) {
    for path in warm_routes(project) {
        let key = memo::canonical(&path);
        let stamp = memo::stamp_for(st, &key);
        if st.memo.get(&key, &stamp).is_some() {
            continue;
        }
        let repo = crate::routes::reads_code_repo(&path)
            .then(|| crate::routes::repo_for(st, &path))
            .flatten();
        let store = memo::store_for(st, project);
        match kanbanr_core::dispatch::dispatch_in(store, "GET", &path, None, repo) {
            Ok(body) => st.memo.put(
                &key,
                &stamp,
                body.into(),
                kanbanr_core::dispatch::read_media_type(&path),
            ),
            Err(e) => eprintln!("kanbanr: warm {path}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    const FAST: Timing = Timing {
        debounce: Duration::from_millis(30),
        poll: Duration::from_millis(60),
    };

    /// A scratch folder under the system temporary directory, never under the repository (L-41).
    fn scratch(tag: &str) -> PathBuf {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let dir =
            std::env::temp_dir().join(format!("kanbanr-bg-{tag}-{}", u64::from_le_bytes(bytes)));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str, body: &str) {
        std::fs::write(dir.join(file), body).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", file]);
    }

    /// A git repository with one commit.
    fn code_repo(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        git(&dir, &["init", "-q", "--initial-branch=main"]);
        commit(&dir, "README.md", "# code\n");
        dir
    }

    /// A scratch board with project `demo`, committed, and the state over it.
    fn board(tag: &str) -> (AppState, PathBuf) {
        let dir = scratch(tag);
        kanbanr_core::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        assert_eq!(store.data_dir(), dir.as_path(), "the test's own board");
        store
            .init_project("demo", kanbanr_core::ProjectConfig::default_for("demo"))
            .unwrap();
        git(&dir, &["add", "-A"]);
        let _ = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(["commit", "-q", "-m", "board"])
            .current_dir(&dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output();
        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        let _ = store;
        let boards = Arc::new(kanbanr_core::boards::Boards::single(dir.clone()));
        let st = AppState {
            store: Arc::clone(&boards.first().store),
            boards,
            data_dir: dir.clone(),
            tx,
            allow_writes: false,
            push: crate::PushPolicy::for_board(&dir).0,
            code_repo: None,
            gens: Arc::new(memo::Generations::default()),
            memo: Arc::new(memo::Memo::new(memo::MEMO_CAP_BYTES)),
            slices: tokio::sync::broadcast::channel(4).0,
            background: Arc::new(std::sync::OnceLock::new()),
        };
        (st, dir)
    }

    fn report(flags: usize) -> Report {
        let one: kanbanr_core::docdrift::Flag = serde_json::from_value(serde_json::json!({
            "target": {"kind": "doc", "id": "a.md", "title": "A"},
            "via": [],
            "hits": [],
            "commits": [],
            "target_changed_at": ""
        }))
        .unwrap_or_else(|e| panic!("a flag: {e}"));
        Report {
            generated_at: kanbanr_core::now_rfc3339(),
            since: String::new(),
            flags: vec![one; flags],
        }
    }

    /// A runner whose runs each wait for a release, answering with the next queued result.
    struct Gated {
        runs: AtomicUsize,
        release: Mutex<mpsc::Receiver<kanbanr_core::Result<Report>>>,
    }

    impl Gated {
        fn new() -> (Arc<Gated>, mpsc::Sender<kanbanr_core::Result<Report>>) {
            let (tx, rx) = mpsc::channel();
            let gated = Arc::new(Gated {
                runs: AtomicUsize::new(0),
                release: Mutex::new(rx),
            });
            (gated, tx)
        }
        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    impl DriftRunner for Gated {
        fn run(&self, _: &Store, _: &str, _: &Path) -> kanbanr_core::Result<Report> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let rx = self.release.lock().unwrap();
            rx.recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| Err(kanbanr_core::CoreError::Refused("never released".into())))
        }
    }

    /// Poll `cond` every 10 ms for up to 10 s.
    async fn until(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn start(st: &AppState, runner: Arc<dyn DriftRunner>, repo: &Path) -> Arc<Background> {
        Background::start_with(
            st.clone(),
            runner,
            vec![("demo".into(), repo.to_path_buf())],
            FAST,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_burst_of_triggers_runs_drift_once_more_after_the_current_run() {
        let (st, dir) = board("burst");
        let code = code_repo("burst-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        for _ in 0..5 {
            bg.poke("demo");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Let the debounce pass, so the burst is queued behind the running pass.
        tokio::time::sleep(Duration::from_millis(100)).await;
        release.send(Ok(report(1))).unwrap();
        until("the queued run", || gated.runs() == 2).await;
        release.send(Ok(report(1))).unwrap();
        until("the queued run to finish", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            gated.runs(),
            2,
            "a burst of five pokes queues exactly one more run"
        );
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_code_head_change_triggers_a_run() {
        let (st, dir) = board("head");
        let code = code_repo("head-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release.send(Ok(report(0))).unwrap();
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(gated.runs(), 1, "no run without a change");
        commit(&code, "src.rs", "fn main() {}\n");
        until("a run after the code commit", || gated.runs() == 2).await;
        release.send(Ok(report(0))).unwrap();
        let head = kanbanr_core::git::head_commit(&code).unwrap();
        until("fresh at the new HEAD", || {
            let l = bg.latest("demo", false);
            l.state == LatestState::Fresh && l.as_of.is_some_and(|a| a.code_rev == head)
        })
        .await;
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    /// The tags of a board change, as the watcher sends them.
    fn slices(tags: &[&str]) -> crate::watcher::Slices {
        crate::watcher::Slices {
            project: "demo".into(),
            r#gen: 1,
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_slices_event_for_the_project_triggers_a_run() {
        let (st, dir) = board("slices");
        let code = code_repo("slices-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release.send(Ok(report(0))).unwrap();
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        // Another project's change pokes nothing here.
        let mut other = slices(&["items"]);
        other.project = "elsewhere".into();
        st.slices.send(other).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(gated.runs(), 1, "a change to another project runs nothing");
        st.slices.send(slices(&["item:FTR-001", "items"])).unwrap();
        until("a run after the board change", || gated.runs() == 2).await;
        release.send(Ok(report(0))).unwrap();
        until("fresh again", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_finished_run_sends_a_drift_slices_event() {
        let (st, dir) = board("sends");
        let code = code_repo("sends-code");
        let mut rx = st.slices.subscribe();
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        assert!(
            rx.try_recv().is_err(),
            "nothing is sent before a run finishes"
        );
        release.send(Ok(report(1))).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("a slices event within 10 s")
            .unwrap();
        assert_eq!(got.project, "demo");
        assert_eq!(got.tags, vec!["drift".to_string()]);
        assert_eq!(got.r#gen, st.gens.of(Some("demo")));
        assert_eq!(bg.latest("demo", false).state, LatestState::Fresh);

        // A failed run sends nothing: the result open pages show has not changed.
        bg.poke("demo");
        until("the second run", || gated.runs() == 2).await;
        release
            .send(Err(kanbanr_core::CoreError::Refused("git broke".into())))
            .unwrap();
        until("the failure", || {
            !bg.latest("demo", false).failed_at.is_empty()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err(), "a failed run sends no drift event");
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pass_triggers_no_second_pass_through_its_own_drift_event() {
        let (st, dir) = board("own");
        let code = code_repo("own-code");
        let mut rx = st.slices.subscribe();
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release.send(Ok(report(0))).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the drift event")
            .unwrap();
        assert_eq!(got.tags, vec!["drift".to_string()]);
        // Well past the debounce: the worker's own event started no pass.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(gated.runs(), 1, "the drift event pokes nothing");
        assert_eq!(bg.latest("demo", false).state, LatestState::Fresh);
        // A drift tag beside a board tag is a board change, and runs.
        st.slices.send(slices(&["drift", "items"])).unwrap();
        until("a run after a board change", || gated.runs() == 2).await;
        release.send(Ok(report(0))).unwrap();
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    /// A runner that sleeps, then answers.
    struct Sleepy(Duration);

    impl DriftRunner for Sleepy {
        fn run(&self, _: &Store, _: &str, _: &Path) -> kanbanr_core::Result<Report> {
            std::thread::sleep(self.0);
            Ok(report(2))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_latest_result_carries_its_revisions_and_time() {
        let (st, dir) = board("asof");
        let code = code_repo("asof-code");
        let board_head = kanbanr_core::git::head_commit(&dir).expect("the board has a commit");
        let code_head = kanbanr_core::git::head_commit(&code).unwrap();
        let bg = start(&st, Arc::new(Sleepy(Duration::from_millis(50))), &code);
        until("fresh", || {
            bg.latest("demo", true).state == LatestState::Fresh
        })
        .await;
        let l = bg.latest("demo", true);
        let as_of = l.as_of.expect("as_of");
        assert_eq!(as_of.board_rev, board_head);
        assert_eq!(as_of.code_rev, code_head);
        let at = as_of.finished_at.as_bytes();
        assert!(
            at.len() >= 20
                && at[4] == b'-'
                && at[10] == b'T'
                && at[13] == b':'
                && (as_of.finished_at.ends_with('Z') || at[at.len() - 6] == b'+'),
            "finished_at is RFC 3339: {}",
            as_of.finished_at
        );
        assert!(as_of.took_ms >= 50, "took_ms {}", as_of.took_ms);
        assert_eq!(l.count, 2);
        assert_eq!(l.report.map(|r| r.flags.len()), Some(2));
        assert!(
            bg.latest("demo", false).report.is_none(),
            "no flags when not asked"
        );
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_run_in_progress_is_reported_beside_the_last_result() {
        let (st, dir) = board("beside");
        let code = code_repo("beside-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release.send(Ok(report(3))).unwrap();
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        let first = bg.latest("demo", false).as_of;
        bg.poke("demo");
        until("the second run", || gated.runs() == 2).await;
        let l = bg.latest("demo", false);
        assert!(l.recomputing, "recomputing while the second run blocks");
        assert_eq!(l.state, LatestState::Stale);
        assert_eq!(l.count, 3, "the old count");
        assert_eq!(l.as_of, first, "the old result");
        release.send(Ok(report(1))).unwrap();
        until("fresh again", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn before_the_first_run_finishes_the_state_is_computing() {
        let (st, dir) = board("computing");
        let code = code_repo("computing-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        assert_eq!(bg.latest("demo", true).state, LatestState::Computing);
        until("the first run", || gated.runs() == 1).await;
        let l = bg.latest("demo", true);
        assert_eq!(l.state, LatestState::Computing);
        assert_eq!(l.count, 0);
        assert!(l.report.is_none());
        assert!(l.as_of.is_none());
        let json = serde_json::to_value(&l).unwrap();
        assert!(json.get("report").is_none(), "{json}");
        release.send(Ok(report(0))).unwrap();
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn without_a_code_repository_drift_is_unavailable_and_never_runs() {
        let (st, dir) = board("norepo");
        let (gated, _release) = Gated::new();
        let bg = Background::start_with(st.clone(), gated.clone(), vec![], FAST);
        for _ in 0..3 {
            bg.poke("demo");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let l = bg.latest("demo", true);
        assert_eq!(l.state, LatestState::Unavailable);
        assert_eq!(l.reason, NEEDS_REPO);
        assert_eq!(gated.runs(), 0);
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_failed_run_keeps_the_last_result_and_names_the_error() {
        let (st, dir) = board("failed");
        let code = code_repo("failed-code");
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release.send(Ok(report(2))).unwrap();
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        let first = bg.latest("demo", false).as_of;
        bg.poke("demo");
        until("the second run", || gated.runs() == 2).await;
        release
            .send(Err(kanbanr_core::CoreError::Refused("git broke".into())))
            .unwrap();
        until("the failure", || {
            !bg.latest("demo", false).failed_at.is_empty()
        })
        .await;
        let l = bg.latest("demo", false);
        assert_ne!(l.state, LatestState::Failed, "a result exists");
        assert_eq!(l.as_of, first, "the old result is kept");
        assert_eq!(l.count, 2);
        assert!(l.reason.contains("git broke"), "{}", l.reason);
        drop(bg);

        // With no prior result, the state is failed.
        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        until("the first run", || gated.runs() == 1).await;
        release
            .send(Err(kanbanr_core::CoreError::Refused("git broke".into())))
            .unwrap();
        until("failed", || {
            bg.latest("demo", false).state == LatestState::Failed
        })
        .await;
        let l = bg.latest("demo", false);
        assert!(l.as_of.is_none());
        assert!(l.reason.contains("git broke"), "{}", l.reason);
        assert!(!l.failed_at.is_empty());
        drop(bg);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_run_writes_nothing_to_the_board() {
        let (st, dir) = board("readonly");
        let code = code_repo("readonly-code");
        let head = kanbanr_core::git::head_commit(&dir).expect("the board has a commit");
        let status = git(&dir, &["status", "--porcelain"]);
        assert_eq!(status, "", "the board starts clean");
        let bg = start(&st, Arc::new(CoreRunner), &code);
        until("a result", || bg.latest("demo", false).as_of.is_some()).await;
        let l = bg.latest("demo", false);
        assert!(l.failed_at.is_empty(), "the real run failed: {}", l.reason);
        drop(bg);
        assert_eq!(
            kanbanr_core::git::head_commit(&dir).as_deref(),
            Some(head.as_str())
        );
        assert_eq!(git(&dir, &["status", "--porcelain"]), "");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    /// A runner that, when it runs, records its place in a sequence and whether the memo already
    /// held each warm route at its current stamp.
    struct Witness {
        st: AppState,
        seq: AtomicUsize,
        /// For each run: (its sequence number, every warm route hit in the memo).
        seen: Mutex<Vec<(usize, bool)>>,
    }

    impl DriftRunner for Witness {
        fn run(&self, _: &Store, project: &str, _: &Path) -> kanbanr_core::Result<Report> {
            let n = self.seq.fetch_add(1, Ordering::SeqCst);
            let warmed = warm_routes(project).iter().all(|path| {
                let key = memo::canonical(path);
                let stamp = memo::stamp_for(&self.st, &key);
                self.st.memo.get(&key, &stamp).is_some()
            });
            self.seen.lock().unwrap().push((n, warmed));
            Ok(report(0))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pass_warms_the_summary_routes_before_drift() {
        let (st, dir) = board("warm");
        let code = code_repo("warm-code");
        assert_eq!(
            warm_routes("demo"),
            vec![
                "/projects/demo/index".to_string(),
                "/projects/demo/review/summary".to_string(),
                "/projects/demo/overview".to_string(),
                "/projects/demo/attention".to_string(),
                "/projects/demo/plan/summary".to_string(),
                "/projects/demo/releases/summary".to_string(),
                "/projects/demo/charter/progress".to_string()
            ]
        );
        let key = memo::canonical("/projects/demo/index");
        assert!(
            st.memo.get(&key, &memo::stamp_for(&st, &key)).is_none(),
            "the memo starts empty"
        );
        let witness = Arc::new(Witness {
            st: st.clone(),
            seq: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let bg = start(&st, witness.clone(), &code);
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        drop(bg);
        assert!(
            st.memo.get(&key, &memo::stamp_for(&st, &key)).is_some(),
            "after a pass the memo holds the index at its current stamp"
        );
        let seen = witness.seen.lock().unwrap().clone();
        assert_eq!(
            seen.first(),
            Some(&(0, true)),
            "the first drift run started after the index was put in the memo: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
    }

    /// Every file under `dir`, relative.
    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p.strip_prefix(dir).unwrap().to_path_buf());
                }
            }
        }
        out.sort();
        out
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_new_worker_starts_computing_and_reads_no_earlier_result() {
        let cache = scratch("xdg-cache");
        // SAFETY: nothing else in this test binary reads XDG_CACHE_HOME; it is set so that a cache
        // file written anywhere the platform convention points would land in this scratch folder.
        unsafe { std::env::set_var("XDG_CACHE_HOME", &cache) };
        let (st, dir) = board("restart");
        let code = code_repo("restart-code");
        let before = files(&dir);
        let bg = start(&st, Arc::new(Sleepy(Duration::from_millis(1))), &code);
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        drop(bg);

        let (gated, release) = Gated::new();
        let bg = start(&st, gated.clone(), &code);
        let l = bg.latest("demo", false);
        assert_eq!(l.state, LatestState::Computing);
        assert!(l.as_of.is_none());
        until("its own first run", || gated.runs() == 1).await;
        assert_eq!(bg.latest("demo", false).state, LatestState::Computing);
        release.send(Ok(report(0))).unwrap();
        until("fresh", || {
            bg.latest("demo", false).state == LatestState::Fresh
        })
        .await;
        drop(bg);
        assert_eq!(files(&dir), before, "the board folder holds no new file");
        assert!(files(&cache).is_empty(), "the cache folder holds no file");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&code);
        let _ = std::fs::remove_dir_all(&cache);
    }
}
