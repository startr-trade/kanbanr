//! Eventing / notifications on state change (FEAT-036).
//!
//! A **best-effort, non-blocking, opt-in** tail step that runs after a successful mutating write +
//! commit. With no configuration it does nothing observable beyond appending to a per-project
//! events log (cheap, local, never networked). Eventing must NEVER fail or slow a write: every
//! entry point here swallows its own errors (logging to stderr at most) and is invoked from the
//! write path only *after* the mutation is already durable.
//!
//! Two outputs:
//!  - a per-project **events log** (`projects/<id>/events.yaml`), mirroring [`crate::activity`] — a
//!    capped, newest-first list the monitor (or any viewer) can read; safe to commit.
//!  - opt-in **webhook delivery**: if a webhook URL is configured (root `events.config.yaml` or the
//!    `KANBANR_WEBHOOK_URL` env var), each emitted event is POSTed as JSON, best-effort. Delivery is
//!    dependency-injected (see [`emit`]'s `deliver`), so this crate stays HTTP-free — the CLI/daemon
//!    supply the actual sender.
//!
//! The headline notification is [`EventKind::DependentReady`]: when a feature reaches a terminal
//! status, we recompute the dependency graph and emit one event per dependent that *became* ready —
//! the cross-team handoff signal.

use crate::Store;
use crate::graph::{self, DependencyView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

/// Keep the most recent N events per project (older history still lives in git).
/// The folder holding one file per day (FEAT-066). Nothing is trimmed; see [`crate::daylog`].
const LOG: &str = "events";

/// The opt-in webhook config file at the data-dir root. May contain endpoint URLs, so it is
/// gitignored (see `git.rs`) and never pushed.
pub const CONFIG_FILE: &str = "events.config.yaml";

/// Env var holding a single webhook URL (an alternative to the config file). Takes effect in
/// addition to any URLs in the config file.
pub const WEBHOOK_ENV: &str = "KANBANR_WEBHOOK_URL";

/// What happened. Serialized lowercase-ish via the explicit `kind` string in [`Event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A feature was added.
    FeatureAdded,
    /// A feature changed status (any move).
    FeatureMoved,
    /// A feature reached a terminal status (Completed / no-op / declared terminal).
    FeatureCompleted,
    /// A dependent feature became ready because one of its dependencies completed. The
    /// cross-team handoff signal — `feature` is the now-ready dependent.
    DependentReady,
    /// The last open item of a milestone reached a terminal status (FEAT-054). The moment a wave
    /// ends is the only moment its lessons are still fresh, so this is what prompts the retro.
    MilestoneCompleted,
}

/// A single notification event. Mirrors the shape of an activity entry, plus structured detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// RFC3339 timestamp.
    pub time: String,
    /// The project the change is scoped to (qualified ids in `detail` may cross projects).
    pub project: String,
    /// What kind of change this is.
    pub kind: EventKind,
    /// The feature code this event is about, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// A short human-readable description.
    pub message: String,
    /// Free-form structured payload (e.g. new status, the dependency that unblocked this one).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

impl Event {
    /// Construct an event stamped with the current time.
    pub fn new(
        project: &str,
        kind: EventKind,
        feature: Option<String>,
        message: impl Into<String>,
        detail: Value,
    ) -> Self {
        Event {
            time: crate::now_rfc3339(),
            project: project.to_string(),
            kind,
            feature,
            message: message.into(),
            detail,
        }
    }
}

impl crate::daylog::Dated for Event {
    fn at(&self) -> &str {
        &self.time
    }
}

/// The most recent `limit` events for a project, newest first.
pub fn read(data_dir: &Path, project: &str, limit: usize) -> Vec<Event> {
    crate::daylog::read(data_dir, project, LOG, limit)
}

/// Append an event to today's file. Nothing is trimmed (FEAT-066). Best-effort: I/O errors are
/// ignored so eventing never disturbs the write path.
pub fn append(data_dir: &Path, project: &str, event: &Event) {
    crate::daylog::append(data_dir, project, LOG, event.clone());
}

/// Webhook delivery configuration: zero or more endpoint URLs. Empty means delivery is off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Endpoint URLs to POST each event to.
    #[serde(default)]
    pub webhooks: Vec<String>,
}

impl WebhookConfig {
    /// Load the opt-in config: the root `events.config.yaml` (if present) plus the
    /// `KANBANR_WEBHOOK_URL` env var (if set). Order: config-file URLs, then the env URL. Missing
    /// file / unset env / parse errors all yield no extra endpoints (delivery stays off).
    pub fn load(data_dir: &Path) -> WebhookConfig {
        Self::load_with_env(data_dir, std::env::var(WEBHOOK_ENV).ok().as_deref())
    }

    /// The pure half of [`WebhookConfig::load`], taking the env value rather than reading it.
    /// Tests use this: mutating the process environment races with every other thread (which is
    /// why edition 2024 made `set_var`/`remove_var` unsafe), and a seam avoids the hazard instead
    /// of asserting it away.
    pub fn load_with_env(data_dir: &Path, env_url: Option<&str>) -> WebhookConfig {
        let mut cfg: WebhookConfig = std::fs::read_to_string(data_dir.join(CONFIG_FILE))
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or_default();
        if let Some(url) = env_url {
            let url = url.trim();
            if !url.is_empty() && !cfg.webhooks.iter().any(|u| u == url) {
                cfg.webhooks.push(url.to_string());
            }
        }
        cfg.webhooks.retain(|u| !u.trim().is_empty());
        cfg
    }

    /// Whether any webhook endpoint is configured.
    pub fn is_enabled(&self) -> bool {
        !self.webhooks.is_empty()
    }
}

/// A best-effort webhook sender: POSTs the JSON `body` to `url`. Dependency-injected so this crate
/// stays HTTP-free (the CLI and daemon supply a real implementation, e.g. backed by `ureq`).
/// Implementations MUST be best-effort: a delivery failure is ignored, never surfaced into a write.
pub trait WebhookSender {
    /// POST `body` (JSON) to `url`. Errors are the sender's to swallow; the return value is unused.
    fn post(&self, url: &str, body: &str);
}

/// A no-op sender (log-only mode): used when nothing should be delivered over the network. Also the
/// safe default for hermetic tests.
pub struct NullSender;
impl WebhookSender for NullSender {
    fn post(&self, _url: &str, _body: &str) {}
}

/// Deliver one event to every configured webhook, best-effort. No-op when delivery is disabled.
pub fn deliver(config: &WebhookConfig, sender: &dyn WebhookSender, event: &Event) {
    if !config.is_enabled() {
        return;
    }
    let body = match serde_json::to_string(event) {
        Ok(b) => b,
        Err(_) => return,
    };
    for url in &config.webhooks {
        sender.post(url, &body);
    }
}

/// Compute the events implied by a successful mutating dispatch, append them to the log, and deliver
/// each over any configured webhooks. **Best-effort and infallible**: any internal error is
/// swallowed so this can be the final tail step of a write path without ever failing or blocking a
/// commit (the only network is the webhook POST, itself best-effort and skipped when unconfigured).
///
/// `result` is the dispatch response body (JSON for the structured feature ops). `path` and
/// `method` identify the operation. `sender` performs webhook delivery (use [`NullSender`] to log
/// only / for tests).
pub fn emit(
    store: &Store,
    data_dir: &Path,
    method: &str,
    path: &str,
    result: &str,
    sender: &dyn WebhookSender,
) {
    let events = record(store, data_dir, method, path, result);
    deliver_all(data_dir, &events, sender);
}

/// Write the events for a mutation to the log, and return them. Called **before** the commit
/// (FEAT-059), so the log entry is part of the same commit as the change it describes — otherwise
/// the last events of a session sit uncommitted until some later write sweeps them up, and the
/// board repo is left dirty after every session's final operation.
///
/// Appending is local file I/O and cannot fail the write: errors are swallowed as they always were.
pub fn record(
    store: &Store,
    data_dir: &Path,
    method: &str,
    path: &str,
    result: &str,
) -> Vec<Event> {
    let events = compute_events(store, method, path, result);
    for event in &events {
        append(data_dir, &event.project, event);
    }
    events
}

/// Deliver events to a configured webhook. Called **after** the commit, because delivery talks to
/// the network: it must never delay or fail a durable write, which is why it is separated from
/// recording rather than done in the same step.
pub fn deliver_all(data_dir: &Path, events: &[Event], sender: &dyn WebhookSender) {
    if events.is_empty() {
        return;
    }
    let config = WebhookConfig::load(data_dir);
    for event in events {
        deliver(&config, sender, event);
    }
}

/// The project id from `/projects/<id>/...`.
fn project_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(String::from)
}

/// Classify the route: `(project, segments-after-project)`.
fn segments(path: &str) -> Vec<String> {
    let bare = path.split('?').next().unwrap_or(path);
    bare.split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Derive the events for a mutating write. Pure (no I/O side effects) so it is unit-testable; the
/// only "I/O" is reading the store to recompute readiness, which is the same read the graph routes
/// do. Returns an empty vec for ops that don't warrant a notification.
pub fn compute_events(store: &Store, method: &str, path: &str, result: &str) -> Vec<Event> {
    // Only POST/PUT/PATCH on a project route can produce a feature event.
    let Some(project) = project_of(path) else {
        return Vec::new();
    };
    let segs = segments(path);
    let parsed: Value = serde_json::from_str(result).unwrap_or(Value::Null);
    let m = method.to_uppercase();

    // Identify a feature add / move / edit from the route.
    // segs: ["projects", <id>, "features", <code?>, <verb?>]
    let after: Vec<&str> = segs.iter().skip(2).map(String::as_str).collect();
    let mut events = Vec::new();

    match after.as_slice() {
        // Add a feature: POST /projects/<id>/features
        ["features"] if m == "POST" => {
            if let Some(code) = parsed.get("code").and_then(|v| v.as_str()) {
                let status = parsed
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                events.push(Event::new(
                    &project,
                    EventKind::FeatureAdded,
                    Some(code.to_string()),
                    format!("feature {code} added"),
                    json!({ "status": status }),
                ));
            }
        }
        // Move a feature: POST /projects/<id>/features/<code>/move
        ["features", code, "move"] if m == "POST" => {
            let status = parsed
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            push_move_events(store, &mut events, &project, code, &status);
        }
        _ => {}
    }

    events
}

/// Emit the move event, and — if the feature is now terminal — a `DependentReady` event per
/// dependent that just became ready. Uses the post-mutation on-disk state via the store.
fn push_move_events(
    store: &Store,
    events: &mut Vec<Event>,
    project: &str,
    code: &str,
    status: &str,
) {
    // Build the current dependency view (reflects the just-committed move).
    let view = match DependencyView::build(store, None) {
        Ok(v) => v,
        Err(_) => {
            // Can't compute readiness; still emit the basic move event so nothing is lost.
            events.push(Event::new(
                project,
                EventKind::FeatureMoved,
                Some(code.to_string()),
                format!("feature {code} moved to {status}"),
                json!({ "status": status }),
            ));
            return;
        }
    };
    let qid = graph::qualify(project, code);
    let terminal = view.nodes.get(&qid).map(|n| n.terminal).unwrap_or(false);

    let kind = if terminal {
        EventKind::FeatureCompleted
    } else {
        EventKind::FeatureMoved
    };
    let verb = if terminal { "completed" } else { "moved to" };
    events.push(Event::new(
        project,
        kind,
        Some(code.to_string()),
        format!("feature {code} {verb} {status}"),
        json!({ "status": status, "terminal": terminal }),
    ));

    // A wave that just ended: every item of this feature's milestone is now terminal.
    if terminal
        && let Ok(loaded) = store.load_meta(project)
        && let Some(milestone) = loaded
            .features
            .iter()
            .find(|f| f.code == code)
            .map(|f| f.milestone.clone())
    {
        let items: Vec<&crate::FeatureItem> = loaded
            .features
            .iter()
            .filter(|f| f.milestone == milestone)
            .collect();
        if !items.is_empty()
            && items
                .iter()
                .all(|f| graph::is_terminal_status(&loaded.config, &f.status))
        {
            events.push(Event::new(
                project,
                EventKind::MilestoneCompleted,
                Some(code.to_string()),
                format!("milestone {milestone} is complete — write the retro while it is fresh"),
                json!({ "milestone": milestone, "items": items.len() }),
            ));
        }
    }

    if !terminal {
        return;
    }

    // Cross-team handoff: which features depending (directly) on this one are now Ready? Since this
    // feature just became terminal, a direct dependent flips to Ready exactly when all of ITS other
    // deps are also terminal. We report dependents that are now Ready and that list this feature as
    // a dependency — i.e. those this completion could have unblocked.
    for node in view.nodes.values() {
        if !node.depends_on.iter().any(|d| d == &qid) {
            continue;
        }
        if view.readiness(&node.id) == Some(graph::Readiness::Ready) {
            events.push(Event::new(
                &node.project,
                EventKind::DependentReady,
                Some(node.code.clone()),
                format!("{} is now ready (dependency {} completed)", node.id, qid),
                json!({ "ready": node.id, "unblocked_by": qid }),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "kanbanr-eventing-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(p.join("projects")).unwrap();
        p
    }

    fn new_project(store: &Store, id: &str) {
        store
            .init_project(id, ProjectConfig::default_for(id))
            .unwrap();
        store
            .add_milestone(id, "M", "", vec![], Some("M".into()))
            .unwrap();
    }

    #[test]
    fn events_log_round_trips() {
        let dir = temp_dir();
        // The project dir must exist for append to write under it.
        std::fs::create_dir_all(dir.join("projects").join("demo")).unwrap();
        let e1 = Event::new(
            "demo",
            EventKind::FeatureAdded,
            Some("FEAT-001".into()),
            "feature FEAT-001 added",
            json!({ "status": "Planned" }),
        );
        let e2 = Event::new(
            "demo",
            EventKind::FeatureMoved,
            Some("FEAT-001".into()),
            "feature FEAT-001 moved to Scheduled",
            json!({ "status": "Scheduled" }),
        );
        append(&dir, "demo", &e1);
        append(&dir, "demo", &e2);

        let read_back = read(&dir, "demo", 10);
        assert_eq!(read_back.len(), 2, "both events persisted");
        // Newest first.
        assert_eq!(read_back[0].kind, EventKind::FeatureMoved);
        assert_eq!(read_back[1].kind, EventKind::FeatureAdded);
        assert_eq!(read_back[0].feature.as_deref(), Some("FEAT-001"));

        // limit truncates.
        assert_eq!(read(&dir, "demo", 1).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn completing_a_feature_emits_dependent_ready() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap(); // FEAT-001
        let b = store.add_feature("demo", "B", "", "M", None).unwrap(); // FEAT-002 deps on A
        store
            .set_feature_attrs(
                "demo",
                &b.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![a.code.clone()]),
            )
            .unwrap();

        // Move A to terminal (Planned -> Scheduled -> Completed), as the write path would.
        store.move_feature("demo", &a.code, "Scheduled").unwrap();
        let moved = store.move_feature("demo", &a.code, "Completed").unwrap();
        let result = serde_json::to_string(&moved).unwrap();

        let events = compute_events(
            &store,
            "POST",
            &format!("/projects/demo/features/{}/move", a.code),
            &result,
        );

        // A FeatureCompleted for A, and a DependentReady for B.
        assert!(
            events.iter().any(|e| e.kind == EventKind::FeatureCompleted
                && e.feature.as_deref() == Some(a.code.as_str())),
            "expected FeatureCompleted for A: {events:?}"
        );
        let dep = events
            .iter()
            .find(|e| e.kind == EventKind::DependentReady)
            .expect("expected a DependentReady event");
        assert_eq!(dep.feature.as_deref(), Some(b.code.as_str()));
        assert_eq!(dep.project, "demo");
        assert_eq!(
            dep.detail.get("unblocked_by").and_then(|v| v.as_str()),
            Some(graph::qualify("demo", &a.code).as_str())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finishing_the_last_item_completes_the_milestone() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();

        let finish = |code: &str| {
            store.move_feature("demo", code, "Scheduled").unwrap();
            let moved = store.move_feature("demo", code, "Completed").unwrap();
            compute_events(
                &store,
                "POST",
                &format!("/projects/demo/features/{code}/move"),
                &serde_json::to_string(&moved).unwrap(),
            )
        };

        // One item done, one still open: the wave has not ended.
        let events = finish(&a.code);
        assert!(
            !events
                .iter()
                .any(|e| e.kind == EventKind::MilestoneCompleted),
            "B is still open: {events:?}"
        );

        // The last one closes the wave, and the event says which milestone and how big it was.
        let events = finish(&b.code);
        let done = events
            .iter()
            .find(|e| e.kind == EventKind::MilestoneCompleted)
            .expect("the wave ended");
        assert_eq!(
            done.detail.get("milestone").and_then(|v| v.as_str()),
            Some("M")
        );
        assert_eq!(done.detail.get("items").and_then(|v| v.as_u64()), Some(2));
        assert!(done.message.contains("retro"), "{}", done.message);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_terminal_move_emits_only_feature_moved() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let moved = store.move_feature("demo", &a.code, "Scheduled").unwrap();
        let result = serde_json::to_string(&moved).unwrap();

        let events = compute_events(
            &store,
            "POST",
            &format!("/projects/demo/features/{}/move", a.code),
            &result,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::FeatureMoved);
        assert!(events[0].message.contains("Scheduled"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn adding_a_feature_emits_feature_added() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let result = serde_json::to_string(&a).unwrap();
        let events = compute_events(&store, "POST", "/projects/demo/features", &result);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::FeatureAdded);
        assert_eq!(events[0].feature.as_deref(), Some(a.code.as_str()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn emit_writes_log_and_null_sender_is_noop() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let result = serde_json::to_string(&a).unwrap();
        // No webhook config + NullSender => log-only, never networks.
        emit(
            &store,
            &dir,
            "POST",
            "/projects/demo/features",
            &result,
            &NullSender,
        );
        let log = read(&dir, "demo", 10);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, EventKind::FeatureAdded);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn webhook_config_is_off_without_file_or_env() {
        let dir = temp_dir();
        // No file and no env value: delivery stays off. Passing the env in explicitly keeps the
        // test from mutating the process environment, which races with other tests.
        let cfg = WebhookConfig::load_with_env(&dir, None);
        assert!(!cfg.is_enabled(), "no config + no env => delivery off");
        let cfg = WebhookConfig::load_with_env(&dir, Some("https://example.test/hook"));
        assert!(cfg.is_enabled(), "an env URL turns delivery on");
        std::fs::remove_dir_all(&dir).ok();
    }
}
