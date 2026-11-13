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
use crate::origin::{self, Origin, Taker, Verdict};
use crate::{CoreError, Store, activity, dispatch, eventing, prepared};
use serde_json::Value;
use std::path::Path;

/// What a write did: the dispatcher's response, and any push that ran and failed (the commit is
/// durable either way; a failed push leaves the board marked unpushed).
#[derive(Debug, Default)]
pub struct Applied {
    pub body: String,
    pub push_failures: Vec<String>,
}

/// Why a write did not happen. The operation's own failure keeps core's error, so a front end that
/// speaks HTTP answers with [`CoreError::status`] rather than guessing from the message (FEAT-176).
#[derive(Debug)]
pub enum WriteError {
    /// Dispatch refused or failed the operation.
    Core(CoreError),
    /// Refused before dispatch ran: nobody can author the commit, or the lock could not be taken.
    Refused(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Core(e) => e.fmt(f),
            WriteError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for WriteError {}

impl WriteError {
    /// The HTTP status: core's for an operation that failed, 400 for a write refused up front.
    pub fn status(&self) -> u16 {
        match self {
            WriteError::Core(e) => e.status(),
            WriteError::Refused(_) => 400,
        }
    }
}

/// Run `f` holding the board's exclusive advisory write lock (FEAT-029). The OS releases it if the
/// process dies. Reads are intentionally not locked.
pub fn with_lock<T>(data_dir: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
    use fs2::FileExt;
    let lock_path = data_dir.join(git::WRITE_LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        // The lock file holds no content; never truncate it.
        .truncate(false)
        .open(&lock_path)?;
    file.lock_exclusive()?;
    let result = f();
    let _ = file.unlock();
    Ok(result)
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
    with_lock(data_dir, || {
        git::ensure_repo(data_dir);
        let records = dispatch::is_mutation(method) && !dispatch::is_dry_run(body);
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
                    agentscope::check(s, &agentscope::requested(method, path, body), &now)
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
        let body = held_body.as_ref().or(body);
        // Answering a prepared record is the person's alone, refused to Claude by the one guard
        // (FTR-034). A confirmation becomes the verdict it prepares, before the origin is stamped,
        // so from here on it is exactly what a direct verdict is.
        let mut expanded: Option<(String, prepared::Expanded)> = None;
        if let Some((project, id, action)) = prepared::answer_route(method, path) {
            let what = if action == "confirm" {
                Verdict::ConfirmPrepared
            } else {
                Verdict::RejectPrepared
            };
            if let Some(why) = origin::verdict_refusal(what, Taker::of_origin(origin)) {
                return Err(WriteError::Refused(format!("refused: {why}")));
            }
            if action == "confirm" {
                let e =
                    prepared::expand(store, &project, &id, body.unwrap_or(&Value::Null), origin)
                        .map_err(WriteError::Core)?;
                expanded = Some((project, e));
            }
        }
        let (method, path, body) = match &expanded {
            Some((_, e)) => (e.method.as_str(), e.path.as_str(), Some(&e.body)),
            None => (method, path, body),
        };
        // The front end names where a verdict was taken, never the request (DEF-028).
        let stamped = origin::stamp(method, path, body, origin);
        let body = stamped.as_ref().or(body);
        let verdict = origin::is_verdict_route(method, path);
        let out =
            dispatch::dispatch_in(store, method, path, body, repo).map_err(WriteError::Core)?;
        let mut applied = Applied {
            body: out,
            ..Default::default()
        };
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
            let msg = match scoped {
                Some(s) => agentscope::with_trailer(&msg, s),
                None => msg,
            };
            if git::commit_local(data_dir, &msg)
                && let Some(outcome) = git::after_commit(data_dir, policy)
            {
                applied.push_failures = outcome.failures;
            }
            eventing::deliver_all(data_dir, &events, sender);
        }
        Ok(applied)
    })
    .map_err(|e| WriteError::Refused(format!("write lock: {e}")))?
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
            issued_by: "Ada".into(),
            issued_at: crate::now_rfc3339(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            note: String::new(),
            worktree: Some(".claude/worktrees/agent-1".into()),
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
            issued_by: "Ada".into(),
            issued_at: crate::now_rfc3339(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            note: String::new(),
            worktree: Some(".claude/worktrees/agent-1".into()),
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
}
