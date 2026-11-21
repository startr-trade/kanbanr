//! The one write recipe (FEAT-176): every change to a board goes through here, from the CLI and
//! from the writes-enabled daemon alike.
//!
//! Under the cross-process advisory lock (FEAT-029): make sure the board is a repository; refuse
//! before anything changes when nobody can author the commit (FEAT-128); dispatch; append the
//! activity entry and record events *before* the commit, so they land in the same commit as the
//! change they describe (FEAT-059); commit; apply the board's push policy (FEAT-142); and only then
//! deliver events, which may touch the network and must never delay or fail a durable write.
//!
//! This used to exist twice — once in the CLI's backend and once in the daemon — and each copy had
//! to be kept in step with the other by hand.

use crate::agentscope::{self, ProcessScope};
use crate::git::{self, PushPolicy};
use crate::origin::{self, DecisionContext, Origin, Taker, Verdict};
use crate::{
    CoreError, Store, activity, decisions, dispatch, eventing, passes, prepared, reviewgroups,
    writejournal,
};
use serde_json::Value;
use std::path::Path;

/// What a write did: the dispatcher's response, and any push that ran and failed (the commit is
/// durable either way; a failed push leaves the board marked unpushed).
#[derive(Debug, Default)]
pub struct Applied {
    pub body: String,
    pub push_failures: Vec<String>,
    /// What the write has to say beside its answer (DEF-102): an earlier write it put back, files
    /// changed outside kanbanr that its commit took. Also in the answer's `warnings`.
    pub warnings: Vec<String>,
}

/// Why a write did not happen. The operation's own failure keeps core's error, so a front end that
/// speaks HTTP answers with [`CoreError::status`] rather than guessing from the message (FEAT-176).
#[derive(Debug)]
pub enum WriteError {
    /// Dispatch refused or failed the operation.
    Core(CoreError),
    /// Refused before dispatch ran: nobody can author the commit, or the lock could not be taken.
    Refused(String),
    /// The board's commit could not be made, so the write was undone (DEF-102).
    NotCommitted(String),
    /// Another kanbanr write held the board for the whole wait (DEF-102).
    Busy(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Core(e) => e.fmt(f),
            WriteError::Refused(why) | WriteError::NotCommitted(why) | WriteError::Busy(why) => {
                f.write_str(why)
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl WriteError {
    /// The HTTP status: core's for an operation that failed, 400 for a write refused up front, 503
    /// for a write that could not commit or could not take the board (DEF-102).
    pub fn status(&self) -> u16 {
        match self {
            WriteError::Core(e) => e.status(),
            WriteError::Refused(_) => 400,
            WriteError::NotCommitted(_) | WriteError::Busy(_) => 503,
        }
    }
}

/// How long a write waits for the board's write lock (DEF-102). `KANBANR_LOCK_WAIT_SECS`
/// overrides it.
pub const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The wait in force: `LOCK_WAIT` unless `KANBANR_LOCK_WAIT_SECS` names another.
fn lock_wait() -> std::time::Duration {
    std::env::var("KANBANR_LOCK_WAIT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(LOCK_WAIT, std::time::Duration::from_secs)
}

/// Who holds the board's write lock, written into the lock file once it is held (DEF-102). It
/// names the method and path only, never a body.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Holder {
    pub pid: u32,
    pub op: String,
    pub since: String,
}

impl Holder {
    /// "pid 1234, `PUT …`, since 23:18:27Z".
    fn describe(&self) -> String {
        let since = self
            .since
            .split_once('T')
            .map_or(self.since.as_str(), |(_, t)| t);
        format!("pid {}, `{}`, since {since}", self.pid, self.op)
    }
}

/// The steps of the write recipe a debug build can be stopped at (DEF-102), in order.
pub(crate) const FAULT_STEPS: [&str; 7] = [
    "locked",
    "journal",
    "files",
    "activity",
    "staged",
    "committed",
    "done",
];

/// A fault point of the write recipe (DEF-102): a debug build with `KANBANR_FAULT=<step>` aborts
/// here, as a killed process would. The release build has no such path.
pub(crate) fn fault(step: &str) {
    debug_assert!(FAULT_STEPS.contains(&step), "unknown fault step {step}");
    if cfg!(debug_assertions) && std::env::var("KANBANR_FAULT").as_deref() == Ok(step) {
        std::process::abort()
    }
}

/// Run `f` holding the board's exclusive advisory write lock (FEAT-029), waiting at most `wait`
/// for it (DEF-102). Once held, the lock file names this write (`op` is its method and path); a
/// write that gives up names the one holding it. The OS releases the lock if the process dies, and
/// the file is never removed. Reads are intentionally not locked.
pub fn with_lock_for<T>(
    data_dir: &Path,
    wait: std::time::Duration,
    op: &str,
    f: impl FnOnce() -> T,
) -> Result<T, WriteError> {
    use fs2::FileExt;
    use std::io::Write as _;
    let lock_path = data_dir.join(git::WRITE_LOCK_FILE);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        // Never truncated before the lock is held: the holder's name is in it.
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| WriteError::Refused(format!("write lock: {e}")))?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if started.elapsed() < wait => {
                let left = wait.saturating_sub(started.elapsed());
                std::thread::sleep(std::time::Duration::from_millis(50).min(left));
            }
            Err(_) => {
                let holder = std::fs::read_to_string(&lock_path)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Holder>(&t).ok())
                    .map_or_else(|| "a kanbanr process".to_string(), |h| h.describe());
                return Err(WriteError::Busy(format!(
                    "the board is busy: another kanbanr write ({holder}) has held it for \
                     {wait:?}; nothing was changed. Run the command again, or check that process"
                )));
            }
        }
    }
    let me = Holder {
        pid: std::process::id(),
        op: op.to_string(),
        since: crate::now_rfc3339(),
    };
    if let Ok(text) = serde_json::to_vec(&me) {
        let _ = file
            .set_len(0)
            .and_then(|()| file.write_all(&text))
            .and_then(|()| file.sync_data());
    }
    let result = f();
    let _ = file.unlock();
    Ok(result)
}

/// Run `f` holding the board's write lock, waiting at most [`LOCK_WAIT`] (for `migrate`, `sync`
/// and `push_pending`). A wait that runs out is `WouldBlock`, carrying the busy message.
pub fn with_lock<T>(data_dir: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
    with_lock_for(data_dir, lock_wait(), "kanbanr", f).map_err(|e| match e {
        WriteError::Busy(why) => std::io::Error::new(std::io::ErrorKind::WouldBlock, why),
        other => std::io::Error::other(other.to_string()),
    })
}

#[cfg(test)]
thread_local! {
    /// A test seam: the commit's wait for git's locks, in place of `COMMIT_WAIT`.
    static COMMIT_WAIT_HERE: std::cell::Cell<Option<std::time::Duration>> =
        const { std::cell::Cell::new(None) };
    /// A test seam: the dispatch fails after it has written its files.
    static FAIL_AFTER_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The paths changed now that were not changed in `before`.
fn changed_since(data_dir: &Path, before: &[String]) -> Vec<String> {
    git::changed_paths(data_dir)
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect()
}

/// Put back what this write changed (every changed path not in `before`) and end its journal.
/// When the files cannot be put back the journal is kept, so the next write puts them back.
fn undo(data_dir: &Path, before: &[String]) -> Result<(), git2::Error> {
    git::restore_to_head(data_dir, &changed_since(data_dir, before))?;
    writejournal::end(data_dir);
    Ok(())
}

/// Commit the write (DEF-102). If the commit cannot be made, the write's own changes are put back
/// and its journal ended, and the answer says nothing was changed; if even that fails, the journal
/// is kept for the next write to finish.
pub fn commit_or_restore(
    data_dir: &Path,
    msg: &str,
    before: &[String],
) -> Result<Option<git2::Oid>, WriteError> {
    #[cfg(test)]
    let committed = match COMMIT_WAIT_HERE.with(std::cell::Cell::get) {
        Some(wait) => git::commit_write_within(data_dir, msg, wait),
        None => git::commit_write(data_dir, msg),
    };
    #[cfg(not(test))]
    let committed = git::commit_write(data_dir, msg);
    committed.map_err(|e| match undo(data_dir, before) {
        Ok(()) => WriteError::NotCommitted(format!(
            "nothing was changed: the board's commit could not be made ({e}). Run the command \
             again"
        )),
        Err(err) => WriteError::NotCommitted(format!(
            "the board's commit could not be made ({e}), and the files could not be put back \
             ({err}): the next write will put them back. Run the command again"
        )),
    })
}

/// Add `warning` to the answer's `warnings` when the answer is a JSON object.
fn push_warning(body: &mut String, warning: &str) {
    if let Ok(Value::Object(mut m)) = serde_json::from_str::<Value>(body) {
        match m.get_mut("warnings") {
            Some(Value::Array(w)) => w.push(Value::String(warning.to_string())),
            _ => {
                m.insert(
                    "warnings".into(),
                    Value::Array(vec![Value::String(warning.to_string())]),
                );
            }
        }
        *body = Value::Object(m).to_string();
    }
}

/// Apply one data-route write: the whole recipe, under the lock. `method` is POST/PUT/PATCH/DELETE;
/// a dry run (a batch with `dry_run: true`) dispatches and records nothing. Errors are the
/// dispatcher's, or the refusal's (no author, no lock, an agent scope), for the front end to render.
///
/// The agent scope this process works under is resolved from the directory it runs in (FEAT-214):
/// every front end writes through here, so every one is held to it.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    origin: Origin,
) -> Result<Applied, WriteError> {
    match std::env::current_dir() {
        Ok(cwd) => apply_from(
            store, data_dir, policy, method, path, body, repo, sender, origin, &cwd,
        ),
        Err(_) => apply_as(
            store,
            data_dir,
            policy,
            method,
            path,
            body,
            repo,
            sender,
            &ProcessScope::Unscoped,
            origin,
        ),
    }
}

/// [`apply`] for a write whose evidence was made in `dir`, not in this process's directory: the
/// scope is resolved from `dir` (MNT-006). A test-capture hook runs in the session that owns the
/// hook, while the tool ran in a build agent's worktree; the write belongs to that worktree's scope.
#[allow(clippy::too_many_arguments)]
pub fn apply_from(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    origin: Origin,
    dir: &Path,
) -> Result<Applied, WriteError> {
    apply_from_as(
        store,
        data_dir,
        policy,
        method,
        path,
        body,
        repo,
        sender,
        origin,
        dir,
        agentscope::Caller::of_process(),
    )
}

/// [`apply_from`] for a given caller (the process's own is read from the environment).
#[allow(clippy::too_many_arguments)]
fn apply_from_as(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    origin: Origin,
    dir: &Path,
    caller: agentscope::Caller,
) -> Result<Applied, WriteError> {
    let scope = agentscope::resolve(store, dir, caller);
    apply_as(
        store, data_dir, policy, method, path, body, repo, sender, &scope, origin,
    )
}

/// Is this the route that binds a scope to a worktree? The one write an unbound agent may make.
fn is_claim(method: &str, path: &str) -> bool {
    let bare = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "scopes", _, "claim"])
}

/// Does this route change task states, so that a finished item could advance?
fn changes_tasks(method: &str, path: &str) -> bool {
    let bare = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    matches!(
        (method.to_ascii_uppercase().as_str(), segs.as_slice()),
        (
            "PUT",
            ["projects", _, "features", _, "todos", _, "tasks", _]
        ) | ("POST", ["projects", _, "batch"])
    )
}

/// The item being worked on (DEF-050): the scope's item when the process is scoped; else the item
/// the code repository's branch names; else none. With none, no spec edit is refused.
pub fn current_item(
    store: &Store,
    project: &str,
    scope: &ProcessScope,
    repo: Option<&Path>,
) -> Option<String> {
    if let ProcessScope::Scoped(s) = scope {
        return Some(s.item.clone());
    }
    let branch = crate::coderepo::git(repo?, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let p = store.load_meta(project).ok()?;
    crate::scm::branch_item(&p, Some(&branch))
}

/// [`apply`] under a scope already resolved (FEAT-214). Under the lock and before anything changes:
/// an agent context with no valid scope is refused (only a claim gets through); a scoped write must
/// be within its scope, on a board with no changes made outside kanbanr, and holds every item it
/// finishes where it is. After dispatch, a scoped write is recorded as the person, by the agent for
/// its item — on the activity entry, the events and the commit — and the board is never pushed.
#[allow(clippy::too_many_arguments)]
pub fn apply_as(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    scope: &ProcessScope,
    origin: Origin,
) -> Result<Applied, WriteError> {
    let op = format!("{method} {path}");
    with_lock_for(data_dir, lock_wait(), &op, || {
        fault("locked");
        git::ensure_repo(data_dir);
        let records = dispatch::is_mutation(method) && !dispatch::is_dry_run(body);
        // A write that died before its commit is finished first, before the scope checks, so its
        // leftovers never get a scoped agent refused (DEF-102).
        let mut warnings: Vec<String> = Vec::new();
        if records {
            match writejournal::recover(data_dir) {
                Ok(writejournal::Recovery::RolledBack { op, paths }) => warnings.push(format!(
                    "an earlier write (`{op}`) stopped before its commit; its changes were put \
                     back: {}",
                    paths.join(", ")
                )),
                Ok(_) => {}
                Err(e) => {
                    return Err(WriteError::Refused(format!(
                        "an earlier write stopped before its commit, and its changes could not \
                         be put back ({e}); nothing was changed"
                    )));
                }
            }
        }
        let mut held_body: Option<Value> = None;
        let scoped = match scope {
            ProcessScope::Scoped(s) => Some(s),
            _ => None,
        };
        if records {
            match scope {
                ProcessScope::Unscoped => {}
                ProcessScope::Refused { why, may_claim } => {
                    if !(*may_claim && is_claim(method, path)) {
                        return Err(WriteError::Refused(why.clone()));
                    }
                }
                ProcessScope::Scoped(s) => {
                    let now = crate::now_rfc3339();
                    // FTR-056: a scope or a write that names an earlier code means the item that
                    // carries it.
                    let (s_now, requested) = match store.load_meta(&s.project) {
                        Ok(board) => (
                            s.resolved(&board),
                            agentscope::requested(method, path, body)
                                .into_iter()
                                .map(|r| r.resolved(&board))
                                .collect::<Vec<_>>(),
                        ),
                        Err(_) => (s.clone(), agentscope::requested(method, path, body)),
                    };
                    agentscope::check(&s_now, &requested, &now)
                        .map_err(|r| WriteError::Refused(r.0))?;
                    if let Err(paths) = git::is_clean(data_dir) {
                        return Err(WriteError::Refused(format!(
                            "refused by agent scope {}: the board has changes not made through \
                             kanbanr ({}). An agent writes the board only through kanbanr; tell \
                             the coordinator.",
                            s.id,
                            paths.join(", ")
                        )));
                    }
                    if changes_tasks(method, path) {
                        let status = store
                            .load_meta(&s.project)
                            .ok()
                            .and_then(|p| p.feature(&s.item).ok().map(|f| f.status.clone()))
                            .unwrap_or_default();
                        let mut b = body
                            .cloned()
                            .unwrap_or_else(|| Value::Object(Default::default()));
                        if let Some(obj) = b.as_object_mut() {
                            obj.insert(
                                "hold_advance".into(),
                                Value::String(format!(
                                    "all of {}'s tasks are done, but it stays in {status}: under \
                                     agent scope {} the coordinator moves it.",
                                    s.item, s.id
                                )),
                            );
                        }
                        held_body = Some(b);
                    }
                }
            }
            git::require_identity(data_dir).map_err(WriteError::Refused)?;
        }
        // What was already changed before this write, kept out of what it undoes; and the journal
        // that lets the next write finish this one if it dies (DEF-102).
        let before = if records {
            let before = git::changed_paths(data_dir);
            writejournal::begin(
                data_dir,
                &writejournal::Journal {
                    version: 1,
                    pid: std::process::id(),
                    started_at: crate::now_rfc3339(),
                    head: git::head_commit(data_dir).unwrap_or_default(),
                    op: op.clone(),
                    dirty_before: before.clone(),
                },
            )
            .map_err(|e| {
                WriteError::Refused(format!(
                    "the write journal could not be written ({e}); nothing was changed"
                ))
            })?;
            fault("journal");
            before
        } else {
            Vec::new()
        };
        let result = (|| -> Result<Applied, WriteError> {
            let body = held_body.as_ref().or(body);
            // A whole batch of prepared records confirmed at once (FTR-037): each record expands into
            // its verdict's own route and goes through as a direct verdict would, all in one commit.
            if let Some((project, batch, action)) = prepared::batch_route(method, path)
                && action == "confirm"
            {
                return confirm_batch(
                    store, data_dir, policy, &project, &batch, path, body, repo, sender, scope,
                    origin, &before,
                );
            }
            // One decision for several rows of a Review group (FTR-038): each row's own verdict,
            // through its own route, all in one commit.
            if let Some(project) = reviewgroups::decide_route(method, path) {
                return decide_group(
                    store, data_dir, policy, &project, path, body, repo, sender, scope, origin,
                    &before,
                );
            }
            // Who may take a verdict is decided here, under the lock, by the one guard, against the
            // project's decision mode and delegations (FTR-028): a revoke and a delegated verdict are
            // ordered by the lock. Answering a prepared record is the person's alone (FTR-034).
            let mut under = if records {
                judge(store, method, path, body, origin)?
            } else {
                None
            };
            // A confirmation becomes the verdict it prepares, before the origin is stamped, so from
            // here on it is exactly what a direct verdict is.
            let mut expanded: Option<(String, prepared::Expanded)> = None;
            if let Some((project, id, action)) = prepared::answer_route(method, path)
                && action == "confirm"
            {
                let e =
                    prepared::expand(store, &project, &id, body.unwrap_or(&Value::Null), origin)
                        .map_err(WriteError::Core)?;
                expanded = Some((project, e));
            }
            // A manual test's result is a person's check (DEF-040): refused to Claude here, for the
            // tests route and for a batch alike, before anything is written.
            if let Some(v) = origin::guarded_verdict(store, method, path, body)
                && let Some(why) = origin::verdict_refusal(
                    v,
                    Taker::of_origin(origin),
                    &DecisionContext {
                        origin: Some(origin),
                        ..DecisionContext::monitor_mode()
                    },
                )
            {
                return Err(WriteError::Refused(format!("refused: {why}")));
            }
            // Another item's approved spec is not Claude's to edit outside a proposed change
            // (DEF-050): refused here, under the lock, before anything is written.
            if records
                && let Some(why) = origin::guarded_spec_edit(
                    store,
                    method,
                    path,
                    body,
                    Taker::of_origin(origin),
                    (Taker::of_origin(origin) == Taker::Claude
                        && (method == "PATCH" || path.ends_with("/batch")))
                    .then(|| project_of(path).and_then(|p| current_item(store, &p, scope, repo)))
                    .flatten()
                    .as_deref(),
                )
            {
                return Err(WriteError::Refused(why));
            }
            let (method, path, body) = match &expanded {
                Some((_, e)) => (e.method.as_str(), e.path.as_str(), Some(&e.body)),
                None => (method, path, body),
            };
            if expanded.is_some() {
                under = judge(store, method, path, body, origin)?;
            }
            // The front end names where a verdict was taken, never the request (DEF-028). A verdict
            // taken under a delegation is Claude's, recorded as delegated and naming its delegation,
            // whatever the request said.
            let mut stamped = origin::stamp(method, path, body, origin);
            let origin = match &under {
                // The person's answer in the conversation (FTR-035): Claude records it, the person
                // decided it; `by` stays the commit identity the request named, and the answer, the
                // pass and the question go on the record.
                Some(Under::Pass(pass)) => {
                    let mut b = stamped.take().unwrap_or_else(|| {
                        body.cloned()
                            .unwrap_or_else(|| Value::Object(Default::default()))
                    });
                    if let Some(m) = b.as_object_mut() {
                        m.insert(
                            "origin".into(),
                            Value::String(Origin::Conversation.as_str().into()),
                        );
                        m.insert(
                            "heard".into(),
                            serde_json::to_value(crate::models::Heard {
                                question: pass.question.clone(),
                                answer: pass.answer.clone(),
                                pass: pass.id.clone(),
                                heard_at: pass.at.clone(),
                                tool_use_id: pass.tool_use_id.clone(),
                                session_id: pass.session_id.clone(),
                                extra: Default::default(),
                            })
                            .unwrap_or(Value::Null),
                        );
                    }
                    stamped = Some(b);
                    Origin::Conversation
                }
                Some(Under::Delegation(delegation)) => {
                    let mut b = stamped.take().unwrap_or_else(|| {
                        body.cloned()
                            .unwrap_or_else(|| Value::Object(Default::default()))
                    });
                    if let Some(m) = b.as_object_mut() {
                        m.insert(
                            "origin".into(),
                            Value::String(Origin::Delegated.as_str().into()),
                        );
                        m.insert("by".into(), Value::String("Claude".into()));
                        m.insert("delegation".into(), Value::String(delegation.clone()));
                    }
                    stamped = Some(b);
                    Origin::Delegated
                }
                None => origin,
            };
            let body = stamped.as_ref().or(body);
            let verdict = origin::is_verdict_route(method, path);
            let out =
                dispatch::dispatch_in(store, method, path, body, repo).map_err(WriteError::Core)?;
            #[cfg(test)]
            if FAIL_AFTER_DISPATCH.with(std::cell::Cell::get) {
                return Err(WriteError::Core(CoreError::Unsupported(
                    "test: the dispatch failed after writing its files".into(),
                )));
            }
            if records {
                fault("files");
            }
            let mut applied = Applied {
                body: out,
                ..Default::default()
            };
            // The pass was used, whether the answer was recorded now or had been already (FTR-035).
            if let Some(Under::Pass(pass)) = &under
                && let Err(e) = passes::consume(store, &pass.id)
            {
                let warning = format!("pass {} could not be removed: {e}", pass.id);
                if let Ok(Value::Object(mut m)) = serde_json::from_str::<Value>(&applied.body) {
                    if let Some(w) = m
                        .entry("warnings")
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()
                    {
                        w.push(Value::String(warning));
                    }
                    applied.body = Value::Object(m).to_string();
                }
            }
            // A verdict that adds nothing to the record leaves no activity entry, event or commit.
            if verdict && applied.body.contains("\"already_recorded\"") {
                let said = serde_json::from_str::<Value>(&applied.body)
                    .ok()
                    .is_some_and(|v| v.get("already_recorded").is_some());
                if said {
                    return Ok(applied);
                }
            }
            // The verdict stands; now keep the person's answer beside what Claude prepared, in the
            // same commit. If that fails the verdict is still theirs, and the record reads stale.
            if let Some((project, e)) = &expanded {
                let mut v: Value = serde_json::from_str(&applied.body)
                    .unwrap_or_else(|_| Value::Object(Default::default()));
                if let Value::Object(m) = &mut v {
                    m.insert("prepared".into(), Value::String(e.resolution.id.clone()));
                    if let Err(err) = prepared::resolve(store, project, &e.resolution) {
                        let warning = Value::String(format!(
                            "{} could not be marked confirmed: {err}",
                            e.resolution.id
                        ));
                        match m.get_mut("warnings") {
                            Some(Value::Array(w)) => w.push(warning),
                            _ => {
                                m.insert("warnings".into(), Value::Array(vec![warning]));
                            }
                        }
                    }
                }
                applied.body = v.to_string();
            }
            let origin = if verdict { origin.as_str() } else { "" };
            if records {
                let msg = dispatch::commit_message_after(method, path, body, &applied.body);
                let by = scoped.map(|s| s.by_agent());
                if let Some(project) = project_of(path) {
                    let actor = git::identity(data_dir)
                        .map(|(n, _)| n)
                        .unwrap_or_else(|| "kanbanr".into());
                    activity::append_with(
                        data_dir,
                        &project,
                        &actor,
                        &msg,
                        item_of(path)
                            .or_else(|| scope_item(path, &applied.body))
                            .as_deref(),
                        by.as_ref(),
                        origin,
                    );
                }
                let events = eventing::record_with(
                    store,
                    data_dir,
                    method,
                    path,
                    &applied.body,
                    by.as_ref(),
                    origin,
                );
                // From an agent's worktree the board is never pushed — under a scope, and by the claim
                // that binds one: the coordinator reviews and pushes.
                let policy = match scope {
                    ProcessScope::Unscoped => policy,
                    _ => PushPolicy::Off,
                };
                // A verdict's commit says where it was taken, and under which delegation (FTR-028).
                let mut trailers: Vec<(&str, &str)> = Vec::new();
                if verdict {
                    trailers.push(("Kanbanr-Origin", origin));
                }
                match &under {
                    Some(Under::Delegation(d)) => trailers.push(("Kanbanr-Delegation", d.as_str())),
                    Some(Under::Pass(p)) => trailers.push(("Kanbanr-Pass", p.id.as_str())),
                    None => {}
                }
                let msg = with_outside_changes(&msg, &before, &mut applied);
                let msg = agentscope::with_trailers(&msg, &trailers);
                let msg = match scoped {
                    Some(s) => agentscope::with_trailer(&msg, s),
                    None => msg,
                };
                // The writer record moves in the write's own commit (FTR-036).
                crate::writer::stamp(data_dir);
                fault("activity");
                // Committed, or undone and said so: nothing is pushed or delivered then (DEF-102).
                let committed = commit_or_restore(data_dir, &msg, &before)?;
                fault("committed");
                writejournal::end(data_dir);
                fault("done");
                if committed.is_some()
                    && let Some(outcome) = git::after_commit(data_dir, policy)
                {
                    applied.push_failures = outcome.failures;
                }
                eventing::deliver_all(data_dir, &events, sender);
            }
            Ok(applied)
        })();
        // A write that failed after its journal began leaves nothing behind (DEF-102). A commit
        // that could not be made has already been undone.
        let result = match result {
            Err(e) if records && !matches!(e, WriteError::NotCommitted(_)) => {
                let _ = undo(data_dir, &before);
                Err(e)
            }
            Ok(applied) => {
                if records {
                    writejournal::end(data_dir);
                }
                Ok(applied)
            }
            other => other,
        };
        result.map(|mut applied| {
            warnings.append(&mut applied.warnings);
            for w in &warnings {
                push_warning(&mut applied.body, w);
            }
            applied.warnings = warnings;
            applied
        })
    })?
}

/// A write's commit message, naming the files changed outside kanbanr that its commit takes with
/// it (DEF-102, Q2 A); the answer's warnings say the same. A scoped write never gets here with any:
/// it was refused before dispatch.
fn with_outside_changes(msg: &str, before: &[String], applied: &mut Applied) -> String {
    if before.is_empty() {
        return msg.to_string();
    }
    let line = format!(
        "Also committed, changed outside this write: {}",
        before.join(", ")
    );
    applied.warnings.push(line.clone());
    format!("{}\n\n{line}", msg.trim_end())
}

/// Confirm every open record of a batch (FTR-037 R-5), under the lock the caller holds: the
/// person's guard first, then every record is expanded before anything is written (a record that
/// cannot be — stale, answered — refuses the whole batch); then [`apply_verdict_ops`] makes each
/// verdict through its own route, one activity entry and one commit for the lot. A verdict the
/// board refuses is reported in that record's `error` and the rest still go through.
#[allow(clippy::too_many_arguments)]
fn confirm_batch(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    project: &str,
    batch: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    scope: &ProcessScope,
    origin: Origin,
    before: &[String],
) -> Result<Applied, WriteError> {
    if let Some(why) = origin::verdict_refusal(
        Verdict::ConfirmPrepared,
        Taker::of_origin(origin),
        &DecisionContext::monitor_mode(),
    ) {
        return Err(WriteError::Refused(format!("refused: {why}")));
    }
    let open = prepared::open_in_batch(store, project, batch, None).map_err(WriteError::Core)?;
    if open.is_empty() {
        return Err(WriteError::Core(CoreError::Unsupported(format!(
            "no prepared record of batch {batch} waits for the person"
        ))));
    }
    let empty = Value::Object(Default::default());
    let body = body.unwrap_or(&empty);
    let mut ops = Vec::with_capacity(open.len());
    for v in &open {
        let e = prepared::expand(store, project, &v.record.id, body, origin)
            .map_err(WriteError::Core)?;
        ops.push(reviewgroups::VerdictOp {
            key: v.record.id.clone(),
            method: e.method,
            path: e.path,
            body: e.body,
            resolution: Some(e.resolution),
        });
    }
    let made: Vec<String> = ops
        .iter()
        .map(|op| {
            op.resolution
                .as_ref()
                .map(|r| r.made.clone())
                .unwrap_or_default()
        })
        .collect();
    apply_verdict_ops(
        store,
        data_dir,
        policy,
        project,
        path,
        ops,
        repo,
        sender,
        scope,
        origin,
        before,
        |outcomes| {
            let results: Vec<prepared::Resolved> = open
                .iter()
                .zip(outcomes)
                .zip(made)
                .map(|((view, o), made)| prepared::Resolved {
                    id: view.record.id.clone(),
                    kind: view.record.kind.clone(),
                    verdict: prepared::Answer::Confirmed,
                    made,
                    error: o.error.clone(),
                })
                .collect();
            let done = results.iter().filter(|r| r.error.is_none()).count();
            let msg = format!(
                "confirm prepared batch {batch} ({done} of {} record(s))",
                results.len()
            );
            (
                serde_json::json!({"batch": batch, "results": results}).to_string(),
                msg,
            )
        },
    )
}

/// What one verdict of a multi-verdict write did.
struct OpOutcome {
    /// The route it made, `POST /projects/…`.
    made: String,
    /// Why it could not be made; the rest still were.
    error: Option<String>,
}

/// Make each verdict in `ops` through its own route, under the lock the caller holds: each is
/// judged by [`judge`] as the person (or under a delegation), stamped with its origin, dispatched,
/// and its prepared record resolved when it has one. A verdict the board refuses is kept in that
/// op's outcome and the rest still go through. `answer` turns the outcomes into the response body
/// and the commit message; then one activity entry and one commit for the lot (FTR-037, FTR-038).
#[allow(clippy::too_many_arguments)]
fn apply_verdict_ops(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    project: &str,
    path: &str,
    ops: Vec<reviewgroups::VerdictOp>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    scope: &ProcessScope,
    origin: Origin,
    before: &[String],
    answer: impl FnOnce(&[OpOutcome]) -> (String, String),
) -> Result<Applied, WriteError> {
    let mut results = Vec::with_capacity(ops.len());
    for e in ops {
        let outcome = match judge(store, &e.method, &e.path, Some(&e.body), origin) {
            Err(why) => Err(why.to_string()),
            Ok(under) => {
                let mut stamped = origin::stamp(&e.method, &e.path, Some(&e.body), origin);
                if let Some(Under::Delegation(d)) = &under
                    && let Some(m) = stamped
                        .get_or_insert_with(|| e.body.clone())
                        .as_object_mut()
                {
                    m.insert(
                        "origin".into(),
                        Value::String(Origin::Delegated.as_str().into()),
                    );
                    m.insert("delegation".into(), Value::String(d.clone()));
                }
                let verdict_body = stamped.as_ref().unwrap_or(&e.body);
                dispatch::dispatch_in(store, &e.method, &e.path, Some(verdict_body), repo)
                    .map_err(|err| err.to_string())
                    .and_then(|_| match &e.resolution {
                        Some(r) => {
                            prepared::resolve(store, project, r).map_err(|err| err.to_string())
                        }
                        None => Ok(()),
                    })
            }
        };
        results.push(OpOutcome {
            made: format!("{} {}", e.method, e.path),
            error: outcome.err(),
        });
    }
    fault("files");
    let (out, msg) = answer(&results);
    let scoped = match scope {
        ProcessScope::Scoped(s) => Some(s),
        _ => None,
    };
    let by = scoped.map(|s| s.by_agent());
    let actor = git::identity(data_dir)
        .map(|(n, _)| n)
        .unwrap_or_else(|| "kanbanr".into());
    activity::append_with(
        data_dir,
        project,
        &actor,
        &msg,
        None,
        by.as_ref(),
        origin.as_str(),
    );
    let events = eventing::record_with(
        store,
        data_dir,
        "POST",
        path,
        &out,
        by.as_ref(),
        origin.as_str(),
    );
    let policy = match scope {
        ProcessScope::Unscoped => policy,
        _ => PushPolicy::Off,
    };
    let mut applied = Applied {
        body: out,
        ..Default::default()
    };
    let msg = with_outside_changes(&msg, before, &mut applied);
    let msg = agentscope::with_trailers(&msg, &[("Kanbanr-Origin", origin.as_str())]);
    let msg = match scoped {
        Some(s) => agentscope::with_trailer(&msg, s),
        None => msg,
    };
    fault("activity");
    // Committed, or undone and said so (DEF-102).
    let committed = commit_or_restore(data_dir, &msg, before)?;
    fault("committed");
    writejournal::end(data_dir);
    fault("done");
    if committed.is_some()
        && let Some(outcome) = git::after_commit(data_dir, policy)
    {
        applied.push_failures = outcome.failures;
    }
    eventing::deliver_all(data_dir, &events, sender);
    Ok(applied)
}

/// One decision for the selected rows of one Review group (FTR-038 R-3, R-4), under the lock the
/// caller holds: refused to Claude in every mode; then [`reviewgroups::ops`] reads the group as it
/// is now and builds each row's own verdict (refusing the whole request, with nothing written,
/// when a row is no longer waiting or the rows would make different verdicts); then
/// [`apply_verdict_ops`] records them, one commit. The answer is `{group, action, results:
/// [{key, made?, error?}]}`.
#[allow(clippy::too_many_arguments)]
fn decide_group(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    project: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
    scope: &ProcessScope,
    origin: Origin,
    before: &[String],
) -> Result<Applied, WriteError> {
    if let Some(why) = origin::verdict_refusal(
        Verdict::GroupDecision,
        Taker::of_origin(origin),
        &DecisionContext::monitor_mode(),
    ) {
        return Err(WriteError::Refused(format!("refused: {why}")));
    }
    let d: reviewgroups::GroupDecision = serde_json::from_value(
        body.cloned()
            .unwrap_or_else(|| Value::Object(Default::default())),
    )
    .map_err(|e| {
        WriteError::Core(CoreError::Unsupported(format!(
            "a group decision is {{group, action, by, target?, items: [{{key, code, rev, \
             spec_rev, …}}]}}: {e}"
        )))
    })?;
    let ops = reviewgroups::ops(store, project, &d, origin, repo).map_err(WriteError::Core)?;
    let keys: Vec<String> = ops.iter().map(|op| op.key.clone()).collect();
    let (group, action) = (d.group.clone(), d.action.trim().to_string());
    apply_verdict_ops(
        store,
        data_dir,
        policy,
        project,
        path,
        ops,
        repo,
        sender,
        scope,
        origin,
        before,
        |outcomes| {
            let results: Vec<Value> = keys
                .iter()
                .zip(outcomes)
                .map(|(key, o)| match &o.error {
                    Some(err) => serde_json::json!({"key": key, "error": err}),
                    None => serde_json::json!({"key": key, "made": o.made}),
                })
                .collect();
            let done = outcomes.iter().filter(|o| o.error.is_none()).count();
            let msg = format!("decide {group}: {action} ({done} of {})", outcomes.len());
            (
                serde_json::json!({"group": group, "action": action, "results": results})
                    .to_string(),
                msg,
            )
        },
    )
}

/// What a verdict the agent takes stands on (FTR-028, FTR-035).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Under {
    /// A delegation in force, `DLG-n`.
    Delegation(String),
    /// The person's answer in the conversation: the pass it was made into.
    Pass(Box<passes::Pass>),
}

/// The one guard over a request (FTR-028): `Err` is its refusal; `Ok(Some(Under::Delegation))`
/// says the verdict is Claude's under that delegation, `Ok(Some(Under::Pass))` that it is the
/// person's answer in the conversation, kept as heard; `Ok(None)` records it as it stands. Only verdict and
/// person-only routes, and a batch that approves, read the project's decisions. A decisions file
/// that does not parse refuses every delegated verdict and person-only route with its error, and
/// judges the rest as `monitor` mode.
pub(crate) fn judge(
    store: &Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    origin: Origin,
) -> Result<Option<Under>, WriteError> {
    let Some(p) = project_of(path) else {
        return Ok(None);
    };
    let batch = method == "POST"
        && path.split('?').next().unwrap_or("").trim_end_matches('/')
            == format!("/projects/{p}/batch");
    let what = origin::verdict_of_route(method, path);
    if what.is_none() && !batch {
        return Ok(None);
    }
    let named = body.map(origin::delegation_from_body).unwrap_or_default();
    let named_pass = body.map(origin::pass_from_body).unwrap_or_default();
    let d = match decisions::load(store, &p) {
        Ok(d) => d,
        Err(e)
            if decisions::is_person_only(method, path)
                || !named.is_empty()
                || !named_pass.is_empty() =>
        {
            return Err(WriteError::Refused(format!(
                "refused: {} could not be read, so no delegation or person-only decision can be \
                 judged: {e}",
                decisions::DECISIONS_FILE
            )));
        }
        Err(_) => decisions::Decisions::default(),
    };
    let project = store.load_meta(&p).ok();
    let now = crate::now_rfc3339();
    let taker = Taker::of_origin(origin);
    let refuse = |why: String| WriteError::Refused(format!("refused: {why}"));
    if batch {
        // A batch carries no delegation: each approval in it is judged as a direct one.
        let ops = body.and_then(|b| {
            if b.is_array() {
                Some(b)
            } else {
                b.get("operations")
            }
        });
        for op in crate::batch::parse(ops).unwrap_or_default() {
            if let crate::batch::BatchOp::FeatureApprove { code, .. } = op {
                let route = format!("/projects/{p}/features/{code}/approve");
                let ctx = DecisionContext {
                    decisions: Some(&d),
                    project: project.as_ref(),
                    method: "POST",
                    path: &route,
                    body: None,
                    origin: Some(origin),
                    now: now.clone(),
                    pass: None,
                };
                if let Some(why) = origin::verdict_refusal(Verdict::Approve, taker, &ctx) {
                    return Err(refuse(why));
                }
            }
        }
        return Ok(None);
    }
    let Some(what) = what else {
        return Ok(None);
    };
    // The pass the body names, when it is there and has not expired (FTR-035).
    let pass = (!named_pass.is_empty())
        .then(|| passes::load(store, &named_pass, &now))
        .flatten();
    let ctx = DecisionContext {
        decisions: Some(&d),
        project: project.as_ref(),
        method,
        path,
        body,
        origin: Some(origin),
        now,
        pass: pass.as_ref(),
    };
    if let Some(why) = origin::verdict_refusal(what, taker, &ctx) {
        return Err(refuse(why));
    }
    // Only an item verdict stands on a pass; the guard has refused it for any other route.
    if taker == Taker::Claude
        && !named_pass.is_empty()
        && decisions::verdict_of(method, path).is_some()
    {
        return Ok(pass.map(|p| Under::Pass(Box::new(p))));
    }
    if taker != Taker::Claude || named.is_empty() {
        return Ok(None);
    }
    // Provenance (T9): only a grant kanbanr wrote, in a committed file, is honoured.
    match decisions::grant_provenance(store, &p, &named) {
        decisions::Provenance::ThroughKanbanr => {}
        decisions::Provenance::Elsewhere => {
            return Err(refuse(format!(
                "{named} was not granted through kanbanr: {} was changed outside it, or the \
                 grant was written into the file by hand. The person grants it again on the \
                 Review page.",
                decisions::DECISIONS_FILE
            )));
        }
        // DEF-080: a grant nobody can check is never honoured, and the refusal says why.
        decisions::Provenance::Unverifiable(why) => {
            return Err(refuse(format!(
                "{named} could not be checked: {why}. Nothing is taken under it until kanbanr \
                 can read the board's history."
            )));
        }
    }
    Ok(Some(Under::Delegation(named)))
}

/// The item a scope route's response is about, for tagging its activity (FEAT-214).
fn scope_item(path: &str, response: &str) -> Option<String> {
    if !path.contains("/scopes") {
        return None;
    }
    let v: Value = serde_json::from_str(response).ok()?;
    v["scope"]["item"]
        .as_str()
        .or_else(|| v["item"].as_str())
        .map(String::from)
}

/// The project id from `/projects/<id>/...`.
pub fn project_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// The feature code from `/projects/<id>/features/<CODE>/...`, for tagging activity.
pub fn item_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next()?;
    if segs.next()? != "features" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn head(dir: &Path) -> String {
        crate::coderepo::git(dir, &["rev-parse", "HEAD"]).unwrap()
    }

    /// FEAT-214 R-3, R-7, R-14: under a scope, a write outside it is refused before dispatch and
    /// the board does not change; an unbound agent is refused; a hand edit to the board refuses
    /// the next write; a write inside it is recorded by the agent, with the trailer, unpushed, and
    /// holds the finished item where it is.
    #[test]
    fn scoped_write_is_refused_before_dispatch() {
        let dir = std::env::temp_dir().join(format!("kanbanr-write-scoped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        let write = |scope: &ProcessScope, m: &str, p: &str, b: Value| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Auto,
                m,
                p,
                Some(&b),
                None,
                &eventing::NullSender,
                scope,
                Origin::Terminal,
            )
        };
        let none = ProcessScope::Unscoped;
        write(&none, "POST", "/projects", json!({"name": "demo"})).unwrap();
        write(
            &none,
            "POST",
            "/projects/demo/milestones",
            json!({"name": "M", "code": "M"}),
        )
        .unwrap();
        write(
            &none,
            "POST",
            "/projects/demo/features",
            json!({"title": "T", "milestone": "M"}),
        )
        .unwrap();
        write(
            &none,
            "POST",
            "/projects/demo/features/FEAT-001/move",
            json!({"to": "In Progress"}),
        )
        .unwrap();
        write(
            &none,
            "POST",
            "/projects/demo/features/FEAT-001/todos",
            json!({"description": "w"}),
        )
        .unwrap();
        write(
            &none,
            "POST",
            "/projects/demo/features/FEAT-001/todos/TL-001/tasks",
            json!({"text": "one"}),
        )
        .unwrap();

        let scope = agentscope::Scope {
            id: "SC-001".into(),
            project: "demo".into(),
            role: agentscope::Role::Build,
            item: "FEAT-001".into(),
            operations: agentscope::Role::Build.operations(),
            branch: Some("feat/FEAT-001-t".into()),
            base: None,
            task: None,
            issued_by: "Ada".into(),
            issued_at: crate::now_rfc3339(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            note: String::new(),
            worktree: Some(".claude/worktrees/agent-1".into()),
            agent_branch: None,
            claimed_at: None,
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            extra: Default::default(),
        };
        let scoped = ProcessScope::Scoped(scope);
        let before = head(&dir);
        let moved = write(
            &scoped,
            "POST",
            "/projects/demo/features/FEAT-001/move",
            json!({"to": "Completed"}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            moved.contains("`move feature FEAT-001` is not one of its operations"),
            "{moved}"
        );
        assert_eq!(head(&dir), before, "nothing committed");
        assert!(git::is_clean(&dir).is_ok(), "nothing changed on disk");
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature("FEAT-001")
                .unwrap()
                .status,
            "In Progress"
        );

        let unbound = ProcessScope::Refused {
            why: "refused: this is an agent worktree".into(),
            may_claim: true,
        };
        let e = write(
            &unbound,
            "PUT",
            "/projects/demo/features/FEAT-001/todos/TL-001/tasks/T1",
            json!({"state": "InProgress"}),
        )
        .unwrap_err();
        assert!(
            e.to_string()
                .starts_with("refused: this is an agent worktree")
        );
        assert_eq!(head(&dir), before);

        std::fs::write(dir.join("projects").join("demo").join("stray.txt"), "x").unwrap();
        let dirty = write(
            &scoped,
            "PUT",
            "/projects/demo/features/FEAT-001/todos/TL-001/tasks/T1",
            json!({"state": "InProgress"}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            dirty.contains("changes not made through kanbanr (projects/demo/stray.txt)"),
            "{dirty}"
        );
        std::fs::remove_file(dir.join("projects").join("demo").join("stray.txt")).unwrap();

        let done = write(
            &scoped,
            "PUT",
            "/projects/demo/features/FEAT-001/todos/TL-001/tasks/T1",
            json!({"state": "Completed"}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&done.body).unwrap();
        assert_eq!(v["status"], "In Progress", "held: {v}");
        assert!(
            v["auto_advance_held"].as_str().unwrap().contains(
                "stays in In Progress: under agent scope SC-001 the coordinator moves it"
            ),
            "{v}"
        );
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(
            log.contains("Agent-Scope: SC-001 (build, by the agent for FEAT-001)"),
            "{log}"
        );
        let entry = &activity::read(&dir, "demo", 1)[0];
        assert_eq!(entry.actor, "Ada");
        assert_eq!(
            entry.by_agent.as_ref().unwrap().to_string(),
            "by the agent for FEAT-001 (SC-001)"
        );
        assert!(
            git::has_unpushed(&dir),
            "under a scope the board is never pushed"
        );
        // The events of a scoped write carry its agent too.
        let by = match &scoped {
            ProcessScope::Scoped(s) => s.by_agent(),
            _ => unreachable!(),
        };
        let events = eventing::record_by(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            r#"{"code": "FEAT-009", "status": "Planned"}"#,
            Some(&by),
        );
        assert_eq!(events[0].by_agent.as_ref(), Some(&by));
        assert_eq!(
            eventing::read(&dir, "demo", 1)[0].by_agent.as_ref(),
            Some(&by)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MNT-006 R-2: a write made for evidence from another directory is held to that directory's
    /// scope: this process stands in an unscoped folder, `dir` is a claimed agent worktree, and the
    /// commit carries the worktree's trailer; from the unscoped folder it carries none.
    #[test]
    fn a_write_applied_from_a_directory_is_recorded_under_that_directory_scope() {
        let base = std::env::temp_dir().join(format!("kanbanr-write-from-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, code) = (base.join("board"), base.join("code"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&code).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        let write = |scope: &ProcessScope, m: &str, p: &str, b: Value| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Auto,
                m,
                p,
                Some(&b),
                None,
                &eventing::NullSender,
                scope,
                Origin::Terminal,
            )
            .unwrap()
        };
        let none = ProcessScope::Unscoped;
        write(&none, "POST", "/projects", json!({"name": "demo"}));
        write(
            &none,
            "POST",
            "/projects/demo/milestones",
            json!({"name": "M", "code": "M"}),
        );
        write(
            &none,
            "POST",
            "/projects/demo/features",
            json!({"title": "T", "milestone": "M"}),
        );
        write(
            &none,
            "POST",
            "/projects/demo/features/FEAT-001/move",
            json!({"to": "In Progress"}),
        );
        write(
            &none,
            "POST",
            "/projects/demo/features/FEAT-001/todos",
            json!({"description": "w"}),
        );
        for text in ["one", "two"] {
            write(
                &none,
                "POST",
                "/projects/demo/features/FEAT-001/todos/TL-001/tasks",
                json!({"text": text}),
            );
        }

        // A code repository with an agent's worktree, and a scope bound to it on the board.
        let g = |args: &[&str]| crate::coderepo::git(&code, args).unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@x",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        g(&[
            "worktree",
            "add",
            "-q",
            ".claude/worktrees/agent-1",
            "-b",
            "agent-1",
        ]);
        let agent_dir = code.join(".claude/worktrees/agent-1");
        let scope = agentscope::Scope {
            id: "SC-001".into(),
            project: "demo".into(),
            role: agentscope::Role::Build,
            item: "FEAT-001".into(),
            operations: agentscope::Role::Build.operations(),
            branch: Some("agent-1".into()),
            base: None,
            task: None,
            issued_by: "Ada".into(),
            issued_at: crate::now_rfc3339(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            note: String::new(),
            worktree: Some(".claude/worktrees/agent-1".into()),
            agent_branch: None,
            claimed_at: Some(crate::now_rfc3339()),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            extra: Default::default(),
        };
        std::fs::write(
            dir.join("projects")
                .join("demo")
                .join(agentscope::SCOPES_FILE),
            crate::yaml::to_string(&vec![scope]).unwrap(),
        )
        .unwrap();
        assert!(git::commit_local(&dir, "bind a scope"));
        let facts = crate::coderepo::worktree_facts(&agent_dir).unwrap();
        agentscope::write_pointer(
            &facts.git_dir,
            &agentscope::PointerFile {
                scope: "SC-001".into(),
                project: "demo".into(),
            },
        )
        .unwrap();

        let task = |dir_of_evidence: &Path, state: &str| {
            apply_from_as(
                &store,
                &dir,
                PushPolicy::Auto,
                "PUT",
                "/projects/demo/features/FEAT-001/todos/TL-001/tasks/T1",
                Some(&json!({"state": state})),
                None,
                &eventing::NullSender,
                Origin::Terminal,
                dir_of_evidence,
                agentscope::Caller::Session,
            )
            .unwrap()
        };
        task(&code, "InProgress");
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(!log.contains("Agent-Scope:"), "unscoped folder: {log}");
        task(&agent_dir, "Completed");
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(
            log.contains("Agent-Scope: SC-001 (build, by the agent for FEAT-001)"),
            "{log}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// DEF-040 R-2: a manual result is the person's verdict. Claude is refused it, through the
    /// tests route and through a batch; a test that is not manual, and setting one back to
    /// planned, are not refused; the person's terminal and the monitor pass.
    #[test]
    fn a_manual_result_from_claude_is_refused() {
        use crate::models::{FeatureDefinition, Requirement, TestRef};
        let dir = std::env::temp_dir().join(format!("kanbanr-write-manual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        let write = |origin: Origin, m: &str, p: &str, b: Value| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Auto,
                m,
                p,
                Some(&b),
                None,
                &eventing::NullSender,
                &ProcessScope::Unscoped,
                origin,
            )
        };
        let t = Origin::Terminal;
        write(t, "POST", "/projects", json!({"name": "demo"})).unwrap();
        write(
            t,
            "POST",
            "/projects/demo/milestones",
            json!({"name": "M", "code": "M"}),
        )
        .unwrap();
        write(
            t,
            "POST",
            "/projects/demo/features",
            json!({"title": "T", "milestone": "M"}),
        )
        .unwrap();
        let test = |name: &str, kind: &str| TestRef {
            name: name.into(),
            kind: kind.into(),
            ..Default::default()
        };
        store
            .set_feature_definition(
                "demo",
                "FEAT-001",
                Some(FeatureDefinition {
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "x".into(),
                        tests: vec![
                            test("manual: looks right", "manual"),
                            test("a::unit", "unit"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // A manual check opens at the build stage (DEF-076).
        let build = store
            .load_meta("demo")
            .unwrap()
            .config
            .role_status(crate::config::Role::Build)
            .unwrap();
        store.move_feature("demo", "FEAT-001", &build).unwrap();
        let route = "/projects/demo/features/FEAT-001/tests/R-1/";
        let manual = format!("{route}manual%3A%20looks%20right");
        for o in [Origin::Conversation, Origin::Delegated] {
            let e = write(o, "PUT", &manual, json!({"state": "green", "by": "x"})).unwrap_err();
            assert!(e.to_string().contains("Review page"), "{o:?}: {e}");
            let batch = json!({"operations": [{"op": "test.state", "feature": "FEAT-001",
                "requirement": "R-1", "test": "manual: looks right", "state": "red"}]});
            let e = write(o, "POST", "/projects/demo/batch", batch).unwrap_err();
            assert!(e.to_string().contains("Review page"), "{o:?} batch: {e}");
        }
        let state_of = |name: &str| {
            let p = store.load_meta("demo").unwrap();
            let f = p.feature("FEAT-001").unwrap();
            f.definition.as_ref().unwrap().requirements[0]
                .tests
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .state
        };
        assert_eq!(
            state_of("manual: looks right"),
            crate::models::TestState::Planned
        );
        // Not a verdict: a test that is not manual, and putting a manual test back to planned.
        write(
            Origin::Conversation,
            "PUT",
            &format!("{route}a%3A%3Aunit"),
            json!({"state": "green"}),
        )
        .unwrap();
        write(
            Origin::Conversation,
            "PUT",
            &manual,
            json!({"state": "planned"}),
        )
        .unwrap();
        // The person's own terminal and the monitor pass.
        write(t, "PUT", &manual, json!({"state": "green", "by": "Ada"})).unwrap();
        write(
            Origin::Monitor,
            "PUT",
            &manual,
            json!({"state": "red", "by": "Ada"}),
        )
        .unwrap();
        assert_eq!(
            state_of("manual: looks right"),
            crate::models::TestState::Red
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_name_their_project_and_item() {
        assert_eq!(
            project_of("/projects/demo/features/FEAT-1/move").as_deref(),
            Some("demo")
        );
        assert_eq!(
            item_of("/projects/demo/features/FEAT-1/move?x=1").as_deref(),
            Some("FEAT-1")
        );
        assert_eq!(item_of("/projects/demo/milestones"), None);
        assert_eq!(project_of("/portfolio/programs"), None);
    }

    /// A board with FEAT-001 (the item being worked on), FEAT-002 (approved) and FEAT-003 (not),
    /// and a code repository on the branch of FEAT-001.
    fn spec_guard_fixture(tag: &str) -> (std::path::PathBuf, Store, std::path::PathBuf) {
        use crate::docdrift::testkit::{code_commit, git_env, scratch};
        let dir =
            std::env::temp_dir().join(format!("kanbanr-write-spec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        for _ in 0..3 {
            store.add_feature("demo", "T", "Spec.", "M", None).unwrap();
        }
        store
            .set_feature_definition(
                "demo",
                "FEAT-002",
                Some(crate::models::FeatureDefinition {
                    statement: "Agreed".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        store.approve_feature("demo", "FEAT-002", "Ada").unwrap();
        let p = store.load("demo").unwrap();
        assert!(
            p.feature("FEAT-002")
                .unwrap()
                .definition
                .as_ref()
                .unwrap()
                .approval_state()
                .is_agreed()
        );
        assert!(p.feature("FEAT-003").unwrap().definition.is_none());
        let repo = scratch(&format!("spec-{tag}"));
        git_env(&repo, &["init", "-q", "-b", "feat/FEAT-001-t"], None);
        code_commit(&repo, "a.txt", "a\n", "a", "2026-11-01T10:00:00+00:00");
        (dir, store, repo)
    }

    fn spec_write(
        store: &Store,
        dir: &Path,
        repo: Option<&Path>,
        origin: Origin,
        target: &str,
        batch: bool,
    ) -> Result<Applied, WriteError> {
        let (method, path, body) = if batch {
            (
                "POST",
                "/projects/demo/batch".to_string(),
                json!({"operations": [{"op": "feature.edit", "code": target, "spec": "New spec.\n"}]}),
            )
        } else {
            (
                "PATCH",
                format!("/projects/demo/features/{target}"),
                json!({"specification": "New spec.\n"}),
            )
        };
        apply_as(
            store,
            dir,
            PushPolicy::Auto,
            method,
            &path,
            Some(&body),
            repo,
            &eventing::NullSender,
            &ProcessScope::Unscoped,
            origin,
        )
    }

    /// DEF-050 R-2: Claude is refused an edit to another item's approved spec, by the route and by
    /// a batch; the person's terminal and the monitor pass.
    #[test]
    fn claude_is_refused_an_edit_to_another_items_approved_spec() {
        let (dir, store, repo) = spec_guard_fixture("refused");
        for origin in [Origin::Conversation, Origin::Delegated] {
            for batch in [false, true] {
                let e = spec_write(&store, &dir, Some(&repo), origin, "FEAT-002", batch)
                    .unwrap_err()
                    .to_string();
                assert!(
                    e.contains("FEAT-002's specification belongs to an approved item"),
                    "{e}"
                );
                assert!(e.contains("Work on FEAT-001 stays on FEAT-001"), "{e}");
            }
        }
        assert_ne!(
            store
                .load("demo")
                .unwrap()
                .feature("FEAT-002")
                .unwrap()
                .specification,
            "New spec.\n"
        );
        for origin in [Origin::Terminal, Origin::Monitor] {
            for batch in [false, true] {
                spec_write(&store, &dir, Some(&repo), origin, "FEAT-002", batch).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-050 R-2: the current item's own spec, and the spec of an item not approved, are Claude's
    /// to edit; a scope names its item as the current one.
    #[test]
    fn an_edit_to_the_current_items_own_spec_or_an_unapproved_items_spec_is_allowed() {
        let (dir, store, repo) = spec_guard_fixture("allowed");
        for batch in [false, true] {
            spec_write(
                &store,
                &dir,
                Some(&repo),
                Origin::Conversation,
                "FEAT-001",
                batch,
            )
            .unwrap();
            spec_write(
                &store,
                &dir,
                Some(&repo),
                Origin::Conversation,
                "FEAT-003",
                batch,
            )
            .unwrap();
        }
        // The scope's item is the current one, whatever branch the checkout is on.
        let scope = ProcessScope::Scoped(agentscope::Scope {
            id: "SC-001".into(),
            project: "demo".into(),
            role: agentscope::Role::Build,
            item: "FEAT-003".into(),
            operations: agentscope::Role::Build.operations(),
            branch: None,
            base: None,
            task: None,
            issued_by: "Ada".into(),
            issued_at: crate::now_rfc3339(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            note: String::new(),
            worktree: None,
            agent_branch: None,
            claimed_at: None,
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            extra: Default::default(),
        });
        assert_eq!(
            current_item(&store, "demo", &scope, Some(&repo)).as_deref(),
            Some("FEAT-003")
        );
        assert_eq!(
            current_item(&store, "demo", &ProcessScope::Unscoped, Some(&repo)).as_deref(),
            Some("FEAT-001")
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-050 R-2: with no current item (no code repository, or a branch that names no item)
    /// nothing is refused: the design stage writes the Design into the item being designed.
    #[test]
    fn with_no_current_item_no_spec_edit_is_refused() {
        let (dir, store, repo) = spec_guard_fixture("none");
        for batch in [false, true] {
            spec_write(&store, &dir, None, Origin::Conversation, "FEAT-002", batch).unwrap();
        }
        crate::docdrift::testkit::git_env(&repo, &["checkout", "-q", "-b", "main"], None);
        assert_eq!(
            current_item(&store, "demo", &ProcessScope::Unscoped, Some(&repo)),
            None
        );
        for batch in [false, true] {
            spec_write(
                &store,
                &dir,
                Some(&repo),
                Origin::Conversation,
                "FEAT-002",
                batch,
            )
            .unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-038 R-3: three approvals in one group decision make three approval records, each at
    /// the revision the page showed and stamped as the monitor's, in exactly one new board commit;
    /// a row whose definition changed after it was shown is reported, and the other two land.
    #[test]
    fn a_group_decision_records_one_verdict_per_item_in_one_commit() {
        let dir = std::env::temp_dir().join(format!("kanbanr-write-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        assert!(
            root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
            "a test board lives in the temp dir (L-41): {}",
            root.display()
        );
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let mut codes = Vec::new();
        for title in ["One", "Two", "Three"] {
            let f = store.add_feature("demo", title, "", "M", None).unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(crate::models::FeatureDefinition {
                        statement: format!("{title} is agreed"),
                        ..Default::default()
                    }),
                )
                .unwrap();
            codes.push(f.code);
        }
        assert!(git::commit_local(&dir, "board"));
        let slice = reviewgroups::slice(&store, "demo", "approve", None).unwrap();
        assert_eq!(slice.rows.len(), 3, "{slice:?}");
        let commits = |dir: &Path| {
            crate::coderepo::git(dir, &["rev-list", "--count", "HEAD"])
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap()
        };
        let before = commits(&dir);
        // The third is shown at a revision its definition has since moved on from.
        let items: Vec<Value> = slice
            .rows
            .iter()
            .map(|r| {
                let rev = if r.code == codes[2] {
                    "0000000".to_string()
                } else {
                    r.rev.clone()
                };
                json!({"key": r.key, "code": r.code, "rev": rev})
            })
            .collect();
        let out = apply_as(
            &store,
            &dir,
            PushPolicy::Off,
            "POST",
            "/projects/demo/review/decide",
            Some(&json!({"group": "approve", "action": "Approve", "by": "Ada", "items": items})),
            None,
            &eventing::NullSender,
            &ProcessScope::Unscoped,
            Origin::Monitor,
        )
        .unwrap();
        let answer: Value = serde_json::from_str(&out.body).unwrap();
        assert_eq!(
            (answer["group"].as_str(), answer["action"].as_str()),
            (Some("approve"), Some("Approve"))
        );
        let results = answer["results"].as_array().unwrap();
        assert_eq!(results.len(), 3, "{answer}");
        for r in results {
            let key = r["key"].as_str().unwrap();
            if key == codes[2] {
                assert!(
                    r["error"]
                        .as_str()
                        .unwrap()
                        .contains("changed after it was shown"),
                    "{r}"
                );
                assert!(r.get("made").is_none(), "{r}");
            } else {
                assert_eq!(
                    r["made"],
                    Value::String(format!("POST /projects/demo/features/{key}/approve")),
                    "{r}"
                );
            }
        }
        assert_eq!(commits(&dir), before + 1, "one press, one commit");
        let project = store.load("demo").unwrap();
        for row in &slice.rows {
            let def = project
                .feature(&row.code)
                .unwrap()
                .definition
                .clone()
                .unwrap();
            if row.code == codes[2] {
                assert!(def.approval.is_none(), "the stale row was approved");
                continue;
            }
            let a = def.approval.expect("approved");
            assert_eq!((a.by.as_str(), a.origin.as_str()), ("Ada", "monitor"));
            assert_eq!(a.rev, row.rev, "each verdict pins its own revision");
        }
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(log.starts_with("decide approve: Approve (2 of 3)"), "{log}");
        assert!(log.contains("Kanbanr-Origin: monitor"), "{log}");

        // Claude is refused it whole, and nothing is written.
        let again = slice.rows[2].clone();
        let before = commits(&dir);
        let e = apply_as(
            &store,
            &dir,
            PushPolicy::Off,
            "POST",
            "/projects/demo/review/decide",
            Some(
                &json!({"group": "approve", "action": "Approve", "by": "Claude",
                         "items": [{"key": again.key, "code": again.code, "rev": again.rev}]}),
            ),
            None,
            &eventing::NullSender,
            &ProcessScope::Unscoped,
            Origin::Conversation,
        )
        .unwrap_err();
        assert!(e.to_string().contains("refused"), "{e}");
        assert_eq!(commits(&dir), before);
        // Outside the write path the route only says where it is taken.
        let e = dispatch::dispatch(
            &store,
            "POST",
            "/projects/demo/review/decide",
            Some(&json!({"group": "approve"})),
        )
        .unwrap_err();
        assert!(e.to_string().contains("write path"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-056 R-7: a recode plan is the person's, in every mode. Claude, in the conversation or
    /// under a delegation, is refused the route and nothing changes; the person's terminal and the
    /// monitor apply it, as one commit with a trailer per new code, and a stale plan is refused.
    #[test]
    fn claude_cannot_apply_a_recode_plan() {
        let dir = std::env::temp_dir().join(format!("kanbanr-write-recode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        let write = |origin: Origin, m: &str, p: &str, b: Value| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Auto,
                m,
                p,
                Some(&b),
                None,
                &eventing::NullSender,
                &ProcessScope::Unscoped,
                origin,
            )
        };
        let t = Origin::Terminal;
        write(t, "POST", "/projects", json!({"name": "demo"})).unwrap();
        write(
            t,
            "POST",
            "/projects/demo/milestones",
            json!({"name": "M", "code": "M"}),
        )
        .unwrap();
        for title in ["one", "two"] {
            write(
                t,
                "POST",
                "/projects/demo/features",
                json!({"title": title, "milestone": "M", "kind": "bug"}),
            )
            .unwrap();
        }
        // Codes are configured after the items exist, so the first is the mismatched one.
        write(
            t,
            "PUT",
            "/projects/demo/config/codes",
            json!({"set": {"bug": "DEF"}}),
        )
        .unwrap();
        write(
            t,
            "POST",
            "/projects/demo/features",
            json!({"title": "three", "milestone": "M", "kind": "bug"}),
        )
        .unwrap();
        let plan = crate::aliases::recode_plan(&store.load("demo").unwrap());
        assert_eq!(plan.renames.len(), 2, "{plan:?}");
        let route = "/projects/demo/recode";
        let before = head(&dir);
        for o in [Origin::Conversation, Origin::Delegated] {
            let e = write(o, "POST", route, json!({"rev": plan.rev})).unwrap_err();
            assert!(
                e.to_string().contains("recode is the person's own"),
                "{o:?}: {e}"
            );
        }
        assert_eq!(head(&dir), before, "refused: no commit");
        assert!(
            store.load("demo").unwrap().feature("DEF-002").is_err(),
            "nothing renamed"
        );
        // A stale plan is refused for the person too.
        assert!(write(t, "POST", route, json!({"rev": "0000"})).is_err());
        assert_eq!(head(&dir), before);
        write(Origin::Monitor, "POST", route, json!({"rev": plan.rev})).unwrap();
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        let to = &plan.renames[0].to;
        assert!(
            log.contains("recode 2 items to their kinds' prefixes"),
            "{log}"
        );
        assert!(log.contains(&format!("kanbanr:{to}")), "{log}");
        assert!(log.contains("Kanbanr-Origin: monitor"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // DEF-102 T4: the write recipe commits or undoes.

    /// A board with project `demo`, milestone `M` and item FEAT-001, written through the recipe.
    fn demo_board(tag: &str) -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("kanbanr-write-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        for (p, b) in [
            ("/projects", json!({"name": "demo"})),
            (
                "/projects/demo/milestones",
                json!({"name": "M", "code": "M"}),
            ),
            (
                "/projects/demo/features",
                json!({"title": "T", "milestone": "M"}),
            ),
        ] {
            unscoped(&store, &dir, "POST", p, b).unwrap();
        }
        assert!(git::changed_paths(&dir).is_empty());
        (dir, store)
    }

    fn unscoped(
        store: &Store,
        dir: &Path,
        m: &str,
        p: &str,
        b: Value,
    ) -> Result<Applied, WriteError> {
        apply_as(
            store,
            dir,
            PushPolicy::Off,
            m,
            p,
            Some(&b),
            None,
            &eventing::NullSender,
            &ProcessScope::Unscoped,
            Origin::Terminal,
        )
    }

    fn commit_count(dir: &Path) -> usize {
        crate::coderepo::git(dir, &["rev-list", "--count", "HEAD"])
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn files_of_head(dir: &Path) -> Vec<String> {
        crate::coderepo::git(dir, &["show", "--name-only", "--format=", "HEAD"])
            .unwrap()
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn hold_index_lock(dir: &Path) -> std::path::PathBuf {
        let lock = git::git_dir(dir).join("index.lock");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .unwrap();
        lock
    }

    fn warnings_of(applied: &Applied) -> Vec<String> {
        let v: Value = serde_json::from_str(&applied.body).unwrap_or(Value::Null);
        v["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w.as_str().map(str::to_string))
            .collect()
    }

    /// DEF-102 R-1: another program holds git's index for a moment; the write waits it out and
    /// lands in exactly one commit holding its files, with the tree clean.
    #[test]
    fn a_write_whose_commit_meets_a_locked_index_waits_and_commits() {
        let (dir, store) = demo_board("locked-waits");
        let before = commit_count(&dir);
        let lock = hold_index_lock(&dir);
        let freed = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::fs::remove_file(lock).unwrap();
        });
        unscoped(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            json!({"title": "U", "milestone": "M"}),
        )
        .expect("the write waits for git and commits");
        freed.join().unwrap();
        assert_eq!(commit_count(&dir), before + 1, "exactly one new commit");
        let files = files_of_head(&dir);
        assert!(
            files.iter().any(|f| f.ends_with("FEAT-002.yaml")),
            "{files:?}"
        );
        assert!(git::changed_paths(&dir).is_empty(), "the tree is clean");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-2: git's index stays held for the whole wait; the write is undone and says so.
    #[test]
    fn a_write_whose_commit_cannot_be_made_leaves_the_board_as_it_was() {
        let (dir, store) = demo_board("locked-gives-up");
        let head_before = head(&dir);
        let lock = hold_index_lock(&dir);
        COMMIT_WAIT_HERE.with(|w| w.set(Some(std::time::Duration::from_millis(200))));
        let err = unscoped(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            json!({"title": "U", "milestone": "M"}),
        )
        .unwrap_err();
        COMMIT_WAIT_HERE.with(|w| w.set(None));
        assert!(matches!(err, WriteError::NotCommitted(_)), "{err:?}");
        assert_eq!(err.status(), 503);
        let text = err.to_string();
        assert!(text.contains("nothing was changed"), "{text}");
        assert!(text.contains("index.lock"), "{text}");
        std::fs::remove_file(&lock).unwrap();
        assert_eq!(head(&dir), head_before, "no commit");
        assert!(
            git::changed_paths(&dir).is_empty(),
            "{:?}",
            git::changed_paths(&dir)
        );
        assert!(writejournal::pending(&dir).is_none(), "the journal ended");
        assert!(store.load("demo").unwrap().feature("FEAT-002").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-2: a dispatch that fails after writing its first file leaves no file changed.
    #[test]
    fn a_dispatch_that_fails_halfway_leaves_the_board_as_it_was() {
        let (dir, store) = demo_board("dispatch-halfway");
        let head_before = head(&dir);
        FAIL_AFTER_DISPATCH.with(|f| f.set(true));
        let err = unscoped(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            json!({"title": "U", "milestone": "M"}),
        )
        .unwrap_err();
        FAIL_AFTER_DISPATCH.with(|f| f.set(false));
        assert!(matches!(err, WriteError::Core(_)), "{err:?}");
        assert_eq!(head(&dir), head_before, "no commit");
        assert!(
            git::changed_paths(&dir).is_empty(),
            "{:?}",
            git::changed_paths(&dir)
        );
        assert!(writejournal::pending(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-4: the board lock is waited for at most its bound, and the refusal names the write
    /// that holds it; once the lock is free a waiting write goes ahead.
    #[test]
    fn a_write_waits_for_the_lock_at_most_its_bound_and_names_the_holder() {
        use fs2::FileExt;
        use std::io::Write as _;
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("kanbanr-write-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut held = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join(git::WRITE_LOCK_FILE))
            .unwrap();
        held.lock_exclusive().unwrap();
        let holder = Holder {
            pid: 4242,
            op: "PUT /projects/demo/features/FEAT-001/todos/TL-001/tasks/T5".into(),
            since: "2026-11-16T23:18:27Z".into(),
        };
        held.set_len(0).unwrap();
        held.write_all(&serde_json::to_vec(&holder).unwrap())
            .unwrap();
        held.sync_data().unwrap();

        let started = Instant::now();
        let err = with_lock_for(&dir, Duration::from_millis(300), "POST /x", || ()).unwrap_err();
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(300), "{waited:?}");
        assert!(waited < Duration::from_secs(2), "{waited:?}");
        assert!(matches!(err, WriteError::Busy(_)), "{err:?}");
        assert_eq!(err.status(), 503);
        let text = err.to_string();
        assert!(text.contains("pid 4242"), "{text}");
        assert!(
            text.contains("PUT /projects/demo/features/FEAT-001"),
            "{text}"
        );
        assert!(text.contains("23:18:27Z"), "{text}");
        assert!(text.contains("nothing was changed"), "{text}");

        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                with_lock_for(&dir, Duration::from_secs(10), "POST /y", || 7)
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        held.unlock().unwrap();
        assert_eq!(
            waiter.join().unwrap().unwrap(),
            7,
            "the waiting call goes ahead"
        );
        let now: Holder =
            serde_json::from_str(&std::fs::read_to_string(dir.join(git::WRITE_LOCK_FILE)).unwrap())
                .unwrap();
        assert_eq!(now.op, "POST /y", "the taker names itself in the lock file");
        assert_eq!(now.pid, std::process::id());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-7: files changed outside kanbanr that an unscoped write picks up are named in its
    /// commit and its answer.
    #[test]
    fn changes_made_outside_kanbanr_are_named_in_the_commit_that_takes_them() {
        let (dir, store) = demo_board("outside");
        std::fs::write(dir.join("by-hand.md"), "edited by hand\n").unwrap();
        let applied = unscoped(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            json!({"title": "U", "milestone": "M"}),
        )
        .unwrap();
        let body = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(
            body.contains("Also committed, changed outside this write: by-hand.md"),
            "{body}"
        );
        let warnings = warnings_of(&applied);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("changed outside this write") && w.contains("by-hand.md")),
            "{warnings:?}"
        );
        assert!(applied.warnings.iter().any(|w| w.contains("by-hand.md")));
        assert!(files_of_head(&dir).contains(&"by-hand.md".to_string()));
        assert!(git::changed_paths(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-5: a write that died before its commit left a journal and files behind; the next
    /// write puts them back, says so, and commits only its own change.
    #[test]
    fn a_write_after_a_dead_one_reports_what_was_put_back() {
        let (dir, store) = demo_board("after-dead");
        let item = crate::coderepo::git(&dir, &["ls-files"])
            .unwrap()
            .lines()
            .find(|l| l.ends_with("FEAT-001.yaml"))
            .map(|l| dir.join(l))
            .expect("FEAT-001 is tracked");
        let item_before = std::fs::read_to_string(&item).unwrap();
        let folder = item.parent().unwrap().to_path_buf();
        // The dead write: its journal, a changed tracked file, a new file and a temporary file.
        writejournal::begin(
            &dir,
            &writejournal::Journal {
                version: 1,
                pid: 1,
                started_at: crate::now_rfc3339(),
                head: git::head_commit(&dir).unwrap(),
                op: "PUT /projects/demo/features/FEAT-001/todos/TL-001/tasks/T5".into(),
                dirty_before: Vec::new(),
            },
        )
        .unwrap();
        std::fs::write(&item, "half written\n").unwrap();
        std::fs::write(folder.join("dead.yaml"), "x\n").unwrap();
        std::fs::write(folder.join(".FEAT-001.yaml.1.2.tmp"), "x").unwrap();

        let applied = unscoped(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            json!({"title": "U", "milestone": "M"}),
        )
        .unwrap();
        let warnings = warnings_of(&applied);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("stopped before its commit")
                    && w.contains("PUT /projects/demo/features/FEAT-001/todos/TL-001/tasks/T5")
                    && w.contains("dead.yaml")),
            "{warnings:?}"
        );
        assert!(
            applied
                .warnings
                .iter()
                .any(|w| w.contains("stopped before its commit"))
        );
        assert!(!warnings.iter().any(|w| w.contains("changed outside")));
        let files = files_of_head(&dir);
        assert!(!files.iter().any(|f| f.ends_with("dead.yaml")), "{files:?}");
        assert!(
            !files.iter().any(|f| f.ends_with("FEAT-001.yaml")),
            "{files:?}"
        );
        assert!(!folder.join("dead.yaml").exists());
        assert!(!folder.join(".FEAT-001.yaml.1.2.tmp").exists());
        assert_eq!(std::fs::read_to_string(&item).unwrap(), item_before);
        assert!(writejournal::pending(&dir).is_none());
        assert!(git::changed_paths(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
