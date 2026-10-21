//! Issue mirror (FEAT-043): keep a project's features in step with GitHub issues, **one
//! direction** — kanbanr is the source of truth and issues are a mirror of it.
//!
//! This module holds the per-project config (`mirror.yaml`), how a feature renders as an issue, a
//! stable content hash for change detection, the reconcile **plan** (which features need an issue
//! created or updated), and the operations — enable, status, sync, link, pull — over an
//! [`IssueTracker`] (FEAT-176). Talking to GitHub (via `gh`) is the tracker's job and lives in the
//! CLI.

use crate::Store;
use crate::error::Result;
use crate::models::FeatureItem;
use crate::store::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Per-project mirror config file, next to `config.yaml`.
pub const MIRROR_FILE: &str = "mirror.yaml";

/// `projects/<id>/mirror.yaml`. Absent or empty means the mirror is off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MirrorConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubMirror>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GithubMirror {
    /// `owner/repo`.
    pub repo: String,
    /// When the mirror was enabled (RFC 3339). Features created before this only get an issue
    /// with an explicit `mirror sync --all`, so enabling never floods a repo with old items.
    pub enabled_at: String,
}

fn config_path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(MIRROR_FILE)
}

/// Load a project's mirror config (default = off when the file is absent).
pub fn load_config(store: &Store, id: &str) -> Result<MirrorConfig> {
    store.load_meta(id)?; // the project must exist
    match std::fs::read_to_string(config_path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(MirrorConfig::default()),
    }
}

/// Save a project's mirror config; an empty config removes the file.
pub fn save_config(store: &Store, id: &str, config: &MirrorConfig) -> Result<()> {
    store.load_meta(id)?;
    let path = config_path(store, id);
    if config.github.is_none() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    std::fs::write(path, serde_yaml::to_string(config)?)?;
    Ok(())
}

/// A feature rendered as an issue: exactly what gets pushed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueDoc {
    pub title: String,
    pub body: String,
    pub open: bool,
    /// `completed` / `not_planned` when closed; `None` when open.
    pub state_reason: Option<String>,
    /// Sorted, de-duplicated.
    pub labels: Vec<String>,
}

impl IssueDoc {
    /// Stable hash of the content, used to push only features whose issue would change and to
    /// tell whether an issue was edited on the tracker since the last push. Line endings and
    /// trailing whitespace are normalized, because trackers rewrite them.
    pub fn hash(&self) -> String {
        let body = self.body.replace("\r\n", "\n");
        let canonical = format!(
            "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
            self.title.trim(),
            body.trim_end(),
            if self.open { "open" } else { "closed" },
            self.state_reason.as_deref().unwrap_or(""),
            self.labels.join("\n"),
        );
        crate::hash::stable_hash(&canonical)
    }

    /// Normalize fields that came from a tracker so they compare with a rendered doc.
    pub fn normalized(mut self) -> IssueDoc {
        self.labels.sort();
        self.labels.dedup();
        if self.open {
            self.state_reason = None;
        }
        self
    }
}

/// Render a feature as an issue.
pub fn render_issue(project: &Project, feature: &FeatureItem) -> IssueDoc {
    let config = &project.config;
    let closed = crate::graph::is_terminal_status(config, &feature.status);
    let state_reason = closed.then(|| {
        if config.is_no_op(&feature.status) {
            "not_planned".to_string()
        } else {
            "completed".to_string()
        }
    });

    let mut body = format!(
        "> Tracked in kanbanr as `{}:{}` · status **{}** · milestone `{}`\n\
         > This issue mirrors the kanbanr item; edits made here are replaced on the next sync.\n",
        project.id, feature.code, feature.status, feature.milestone
    );
    let spec = feature.specification.trim();
    if !spec.is_empty() {
        body.push('\n');
        body.push_str(spec);
        body.push('\n');
    }
    if !feature.todo_lists.is_empty() {
        body.push_str("\n## Todo-lists\n");
        for list in &feature.todo_lists {
            let desc = if list.description.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", list.description.trim())
            };
            body.push_str(&format!("\n### {}{desc}\n\n", list.code));
            for task in &list.tasks {
                let mark = if task.state == crate::models::TaskState::Completed {
                    "x"
                } else {
                    " "
                };
                body.push_str(&format!("- [{mark}] {}\n", task.text));
            }
        }
    }

    IssueDoc {
        title: feature.title.clone(),
        body,
        open: !closed,
        state_reason,
        labels: feature.labels.clone(),
    }
    .normalized()
}

/// What to do for one feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MirrorActionKind {
    /// No issue yet: create one.
    Create,
    /// Linked issue whose content differs from the last push: update it.
    Update { number: u64 },
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorAction {
    pub code: String,
    pub title: String,
    #[serde(flatten)]
    pub kind: MirrorActionKind,
    #[serde(skip)]
    pub doc: IssueDoc,
    pub hash: String,
}

/// The reconcile plan for a project.
///
/// - A feature linked to an issue in the mirrored repo is updated when its rendered content
///   differs from the last push. Features linked to another repo are left alone.
/// - An unlinked feature gets a new issue when it is not in a terminal state and was created
///   after the mirror was enabled — or regardless of age with `all`.
pub fn plan(project: &Project, mirror: &GithubMirror, all: bool) -> Vec<MirrorAction> {
    let mut actions = Vec::new();
    for f in &project.features {
        let doc = render_issue(project, f);
        let hash = doc.hash();
        let kind = match &f.issue {
            Some(link) if link.system == "github" && link.repo == mirror.repo => {
                if link.synced_hash.as_deref() == Some(hash.as_str()) {
                    continue;
                }
                MirrorActionKind::Update {
                    number: link.number,
                }
            }
            Some(_) => continue,
            None => {
                let terminal = crate::graph::is_terminal_status(&project.config, &f.status);
                if terminal || !(all || !crate::before(&f.created_at, &mirror.enabled_at)) {
                    continue;
                }
                MirrorActionKind::Create
            }
        };
        actions.push(MirrorAction {
            code: f.code.clone(),
            title: f.title.clone(),
            kind,
            doc,
            hash,
        });
    }
    actions
}

// ---- orchestration over a tracker (FEAT-176) -------------------------------------------------
//
// The operations themselves: what to check, what to push, what to record. They talk to the tracker
// through [`IssueTracker`], which a front end implements (the CLI's is `gh`), and write the board
// through the `record` callback it hands in, so the write goes through that front end's own write
// path (lock, commit, push). Errors are text to show as they are: the tracker's own words, or
// the board's.

/// An issue as it currently is on the tracker.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteIssue {
    pub number: u64,
    pub url: String,
    pub doc: IssueDoc,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

/// The operations the mirror needs from an issue tracker.
pub trait IssueTracker {
    /// The tracker CLI is installed and logged in.
    fn check(&self) -> std::result::Result<(), String>;
    fn is_public(&self, repo: &str) -> std::result::Result<bool, String>;
    /// Create an issue (closing it right away if `doc` is closed); returns its number and URL.
    fn create(&self, repo: &str, doc: &IssueDoc) -> std::result::Result<(u64, String), String>;
    fn update(&self, repo: &str, number: u64, doc: &IssueDoc) -> std::result::Result<(), String>;
    fn fetch(&self, repo: &str, number: u64) -> std::result::Result<RemoteIssue, String>;
    fn comments(
        &self,
        repo: &str,
        number: u64,
        since: Option<&str>,
    ) -> std::result::Result<Vec<RemoteComment>, String>;
}

/// Applies one batch body (`{"operations": [...], "message": ...}`) to the project's board, through
/// the front end's write path.
pub type Record<'a> = &'a dyn Fn(Value) -> std::result::Result<(), String>;

fn require_enabled(store: &Store, project: &str) -> std::result::Result<GithubMirror, String> {
    load_config(store, project)
        .map_err(|e| e.to_string())?
        .github
        .ok_or_else(|| {
            format!(
                "the issue mirror is not enabled for '{project}': run `kanbanr mirror enable --repo owner/repo`"
            )
        })
}

/// Turn the mirror on: the config to save. Refuses a public repo unless `allow_public`, because
/// specs and notes would become publicly visible.
pub fn enable(
    tracker: &dyn IssueTracker,
    repo: &str,
    allow_public: bool,
) -> std::result::Result<MirrorConfig, String> {
    let valid = repo
        .split_once('/')
        .is_some_and(|(o, r)| !o.is_empty() && !r.is_empty() && !r.contains('/'));
    if !valid {
        return Err(format!("--repo must look like owner/repo (got '{repo}')"));
    }
    tracker.check()?;
    if tracker.is_public(repo)? && !allow_public {
        return Err(format!(
            "{repo} is public: mirrored specs, todo-lists and notes would be visible to anyone. \
             Re-run with --allow-public if that's intended."
        ));
    }
    Ok(MirrorConfig {
        github: Some(GithubMirror {
            repo: repo.to_string(),
            enabled_at: crate::now_rfc3339(),
        }),
    })
}

/// Where the mirror stands, without touching the tracker.
#[derive(Debug, Clone)]
pub struct MirrorStatus {
    pub mirror: GithubMirror,
    pub actions: Vec<MirrorAction>,
    /// Features linked to an issue in the mirrored repo.
    pub linked: usize,
}

/// The current plan, or `None` when the mirror is off.
pub fn status(store: &Store, project: &str, all: bool) -> Result<Option<MirrorStatus>> {
    let Some(gh) = load_config(store, project)?.github else {
        return Ok(None);
    };
    let p = store.load(project)?;
    let linked = p
        .features
        .iter()
        .filter(|f| f.issue.as_ref().is_some_and(|i| i.repo == gh.repo))
        .count();
    let actions = plan(&p, &gh, all);
    Ok(Some(MirrorStatus {
        mirror: gh,
        actions,
        linked,
    }))
}

#[derive(Debug, Default, Serialize)]
pub struct SyncOutcome {
    pub repo: String,
    pub created: Vec<(String, u64)>,
    pub updated: Vec<(String, u64)>,
    pub failed: Vec<(String, String)>,
}

/// Push every feature whose issue is missing or out of date, then record the new links and
/// content hashes in one batch. Per-feature failures are collected, not fatal.
pub fn sync(
    store: &Store,
    project: &str,
    tracker: &dyn IssueTracker,
    all: bool,
    record: Record,
) -> std::result::Result<SyncOutcome, String> {
    let gh = require_enabled(store, project)?;
    let p = store.load(project).map_err(|e| e.to_string())?;
    let actions = plan(&p, &gh, all);
    let mut outcome = SyncOutcome {
        repo: gh.repo.clone(),
        ..Default::default()
    };
    if actions.is_empty() {
        return Ok(outcome);
    }
    tracker.check()?;

    let now = crate::now_rfc3339();
    let mut edits = Vec::new();
    for action in actions {
        let result = match action.kind {
            MirrorActionKind::Create => tracker.create(&gh.repo, &action.doc),
            MirrorActionKind::Update { number } => {
                let url = p
                    .features
                    .iter()
                    .find(|f| f.code == action.code)
                    .and_then(|f| f.issue.as_ref())
                    .map(|i| i.url.clone())
                    .unwrap_or_default();
                tracker
                    .update(&gh.repo, number, &action.doc)
                    .map(|_| (number, url))
            }
        };
        match result {
            Ok((number, url)) => {
                match action.kind {
                    MirrorActionKind::Create => outcome.created.push((action.code.clone(), number)),
                    MirrorActionKind::Update { .. } => {
                        outcome.updated.push((action.code.clone(), number))
                    }
                }
                edits.push(json!({
                    "op": "feature.edit",
                    "code": action.code,
                    "issue": {
                        "system": "github", "repo": gh.repo, "number": number, "url": url,
                        "synced_hash": action.hash, "synced_at": now,
                    },
                }));
            }
            Err(e) => outcome.failed.push((action.code.clone(), e)),
        }
    }

    if !edits.is_empty() {
        let message = format!("mirror: sync {} issue(s) to {}", edits.len(), gh.repo);
        record(json!({"operations": edits, "message": message})).map_err(|e| {
            let created: Vec<String> = outcome
                .created
                .iter()
                .map(|(c, n)| format!("{c} → #{n}"))
                .collect();
            format!(
                "issues were pushed but recording the links failed ({e}); created: {}. \
                 Link them with `kanbanr mirror link <code> <number>`.",
                created.join(", ")
            )
        })?;
    }
    Ok(outcome)
}

/// Link a feature to an existing issue; returns the issue's URL. The next sync replaces that
/// issue's content with kanbanr's.
pub fn link(
    store: &Store,
    project: &str,
    tracker: &dyn IssueTracker,
    code: &str,
    number: u64,
    record: Record,
) -> std::result::Result<String, String> {
    let gh = require_enabled(store, project)?;
    tracker.check()?;
    let remote = tracker.fetch(&gh.repo, number)?;
    record(json!({
        "operations": [{
            "op": "feature.edit", "code": code,
            "issue": {"system": "github", "repo": gh.repo, "number": number, "url": remote.url},
        }],
        "message": format!("mirror: link {code} to {}#{number}", gh.repo),
    }))?;
    Ok(remote.url)
}

/// What changed on the tracker since the last push, for a deliberate, manual bring-back.
#[derive(Debug, Serialize)]
pub struct PullReport {
    pub code: String,
    pub repo: String,
    pub remote: RemoteIssue,
    /// The issue's content differs from what kanbanr last pushed.
    pub edited_on_tracker: bool,
    /// The feature changed in kanbanr since the last push (a sync would overwrite the issue).
    pub changed_in_kanbanr: bool,
    pub last_synced_at: Option<String>,
    pub comments: Vec<RemoteComment>,
}

/// Compare a linked feature with its issue, and fetch the comments since the last push.
pub fn pull(
    store: &Store,
    project: &str,
    tracker: &dyn IssueTracker,
    code: &str,
) -> std::result::Result<PullReport, String> {
    let p = store.load(project).map_err(|e| e.to_string())?;
    let feature = p
        .features
        .iter()
        .find(|f| f.code == code)
        .ok_or_else(|| format!("feature '{code}' not found"))?;
    let link = feature
        .issue
        .clone()
        .ok_or_else(|| format!("{code} is not linked to an issue"))?;
    tracker.check()?;
    let remote = tracker.fetch(&link.repo, link.number)?;
    let comments = tracker.comments(&link.repo, link.number, link.synced_at.as_deref())?;
    let local_hash = render_issue(&p, feature).hash();
    Ok(PullReport {
        code: code.to_string(),
        repo: link.repo.clone(),
        edited_on_tracker: link.synced_hash.as_deref() != Some(remote.doc.hash().as_str()),
        changed_in_kanbanr: link.synced_hash.as_deref() != Some(local_hash.as_str()),
        last_synced_at: link.synced_at.clone(),
        remote,
        comments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{IssueLink, Status, Task, TaskState, TodoList};

    fn feature(code: &str, status: &str, created_at: &str) -> FeatureItem {
        FeatureItem {
            extra: Default::default(),
            code: code.into(),
            title: format!("Title {code}"),
            specification: "# Spec\nDo it.".into(),
            status: Status::from(status),
            milestone: "MS-001".into(),
            kind: None,
            priority: None,
            start: None,
            due: None,
            estimate_days: None,
            points: None,
            sprint: None,
            release: None,
            assignee: None,
            team: None,
            labels: vec!["ui".into(), "api".into(), "ui".into()],
            depends_on: vec![],
            todo_lists: vec![],
            source: None,
            issue: None,
            definition: None,
            defect: None,
            split_from: None,
            history: Vec::new(),
            created_at: created_at.into(),
            updated_at: created_at.into(),
        }
    }

    fn project(features: Vec<FeatureItem>) -> Project {
        Project {
            id: "shop".into(),
            config: ProjectConfig::default_for("shop"),
            milestones: vec![],
            features,
        }
    }

    fn mirror() -> GithubMirror {
        GithubMirror {
            repo: "acme/shop".into(),
            enabled_at: "2026-09-10T00:00:00Z".into(),
        }
    }

    #[test]
    fn renders_header_spec_checklists_state_and_sorted_labels() {
        let mut f = feature("FEAT-001", "Completed", "2026-09-11T00:00:00Z");
        f.todo_lists = vec![TodoList {
            extra: Default::default(),
            code: "TL-001".into(),
            description: "session 1".into(),
            tasks: vec![
                Task {
                    extra: Default::default(),
                    key: "T1".into(),
                    text: "cart UI".into(),
                    state: TaskState::Completed,
                },
                Task {
                    extra: Default::default(),
                    key: "T2".into(),
                    text: "tests".into(),
                    state: TaskState::NotStarted,
                },
            ],
            created_at: "2026-09-11T00:00:00Z".into(),
        }];
        let p = project(vec![f]);
        let doc = render_issue(&p, &p.features[0]);
        assert!(
            doc.body
                .starts_with("> Tracked in kanbanr as `shop:FEAT-001`")
        );
        assert!(doc.body.contains("# Spec\nDo it."));
        assert!(
            doc.body
                .contains("### TL-001 — session 1\n\n- [x] cart UI\n- [ ] tests\n")
        );
        assert!(!doc.open);
        assert_eq!(doc.state_reason.as_deref(), Some("completed"));
        assert_eq!(doc.labels, vec!["api", "ui"]);

        let mut dropped = feature("FEAT-002", "Out-of-Scope", "2026-09-11T00:00:00Z");
        dropped.labels.clear();
        let p = project(vec![dropped]);
        let doc = render_issue(&p, &p.features[0]);
        assert_eq!(doc.state_reason.as_deref(), Some("not_planned"));
    }

    #[test]
    fn hash_ignores_crlf_and_trailing_whitespace_but_not_content() {
        let p = project(vec![feature("FEAT-001", "Planned", "2026-09-11T00:00:00Z")]);
        let doc = render_issue(&p, &p.features[0]);
        let mut crlf = doc.clone();
        crlf.body = doc.body.replace('\n', "\r\n") + "  \n";
        assert_eq!(doc.hash(), crlf.hash());
        let mut edited = doc.clone();
        edited.title.push('!');
        assert_ne!(doc.hash(), edited.hash());
        let mut reopened = doc.clone();
        reopened.state_reason = Some("reopened".into());
        assert_eq!(doc.hash(), reopened.normalized().hash());
    }

    #[test]
    fn plan_creates_new_open_items_and_updates_only_changed_links() {
        let old = feature("FEAT-001", "Planned", "2026-09-01T00:00:00Z");
        let new = feature("FEAT-002", "Planned", "2026-09-11T00:00:00Z");
        let new_done = feature("FEAT-003", "Completed", "2026-09-11T00:00:00Z");
        let mut linked_same = feature("FEAT-004", "Planned", "2026-09-01T00:00:00Z");
        let mut linked_changed = feature("FEAT-005", "In Progress", "2026-09-01T00:00:00Z");
        let mut other_repo = feature("FEAT-006", "Planned", "2026-09-11T00:00:00Z");

        let p = project(vec![linked_same.clone()]);
        let same_hash = render_issue(&p, &p.features[0]).hash();
        let link = |number, hash: Option<String>| IssueLink {
            extra: Default::default(),
            system: "github".into(),
            repo: "acme/shop".into(),
            number,
            url: format!("https://github.com/acme/shop/issues/{number}"),
            synced_hash: hash,
            synced_at: None,
        };
        linked_same.issue = Some(link(4, Some(same_hash)));
        linked_changed.issue = Some(link(5, Some("stale".into())));
        other_repo.issue = Some(IssueLink {
            repo: "acme/other".into(),
            ..link(6, None)
        });

        let p = project(vec![
            old,
            new,
            new_done,
            linked_same,
            linked_changed,
            other_repo,
        ]);
        let summary = |all| {
            plan(&p, &mirror(), all)
                .into_iter()
                .map(|a| (a.code, a.kind))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(false),
            vec![
                ("FEAT-002".to_string(), MirrorActionKind::Create),
                (
                    "FEAT-005".to_string(),
                    MirrorActionKind::Update { number: 5 }
                ),
            ]
        );
        // `all` backfills old open items, still never terminal ones.
        assert_eq!(summary(true).len(), 3);
        assert!(
            summary(true)
                .iter()
                .all(|(c, _)| c != "FEAT-003" && c != "FEAT-004" && c != "FEAT-006")
        );
    }

    /// An in-memory tracker that records calls.
    #[derive(Default)]
    struct FakeTracker {
        issues: std::cell::RefCell<std::collections::BTreeMap<u64, IssueDoc>>,
        calls: std::cell::RefCell<Vec<String>>,
        public: bool,
    }

    impl IssueTracker for FakeTracker {
        fn check(&self) -> std::result::Result<(), String> {
            Ok(())
        }
        fn is_public(&self, _repo: &str) -> std::result::Result<bool, String> {
            Ok(self.public)
        }
        fn create(&self, repo: &str, doc: &IssueDoc) -> std::result::Result<(u64, String), String> {
            let n = self.issues.borrow().len() as u64 + 1;
            self.issues.borrow_mut().insert(n, doc.clone());
            self.calls.borrow_mut().push(format!("create #{n}"));
            Ok((n, format!("https://github.com/{repo}/issues/{n}")))
        }
        fn update(
            &self,
            _repo: &str,
            number: u64,
            doc: &IssueDoc,
        ) -> std::result::Result<(), String> {
            self.issues.borrow_mut().insert(number, doc.clone());
            self.calls.borrow_mut().push(format!("update #{number}"));
            Ok(())
        }
        fn fetch(&self, repo: &str, number: u64) -> std::result::Result<RemoteIssue, String> {
            let doc = self
                .issues
                .borrow()
                .get(&number)
                .cloned()
                .ok_or("no such issue")?;
            Ok(RemoteIssue {
                number,
                url: format!("https://github.com/{repo}/issues/{number}"),
                doc,
                updated_at: String::new(),
            })
        }
        fn comments(
            &self,
            _repo: &str,
            _number: u64,
            _since: Option<&str>,
        ) -> std::result::Result<Vec<RemoteComment>, String> {
            Ok(vec![])
        }
    }

    /// A board in the system temp folder with project `shop` and milestone `MS-001`; removed on
    /// drop.
    struct Board(Store, PathBuf);
    impl Drop for Board {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn board() -> Board {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-core-mirror-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let store = Store::new(dir.clone());
        let run = |m: &str, p: &str, b: Value| {
            crate::dispatch::dispatch(&store, m, p, Some(&b)).unwrap();
        };
        run("POST", "/projects", json!({"name": "shop"}));
        run(
            "POST",
            "/projects/shop/batch",
            json!({"operations": [{"op": "milestone.add", "name": "M", "code": "MS-001"}]}),
        );
        Board(store, dir)
    }

    fn apply(store: &Store) -> impl Fn(Value) -> std::result::Result<(), String> + '_ {
        move |body| {
            crate::dispatch::dispatch(store, "POST", "/projects/shop/batch", Some(&body))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }

    #[test]
    fn enable_checks_the_repo_and_refuses_a_public_one() {
        let public = FakeTracker {
            public: true,
            ..Default::default()
        };
        assert!(
            enable(&public, "not-a-repo", true)
                .unwrap_err()
                .contains("owner/repo")
        );
        assert!(
            enable(&public, "acme/shop", false)
                .unwrap_err()
                .contains("is public")
        );
        let config = enable(&public, "acme/shop", true).unwrap();
        assert_eq!(config.github.unwrap().repo, "acme/shop");
    }

    #[test]
    fn sync_records_links_then_status_and_pull_agree() {
        let b = board();
        let store = &b.0;
        let t = FakeTracker::default();
        assert!(status(store, "shop", false).unwrap().is_none());
        let err = sync(store, "shop", &t, false, &apply(store)).unwrap_err();
        assert!(err.contains("not enabled"), "{err}");

        save_config(store, "shop", &enable(&t, "acme/shop", false).unwrap()).unwrap();
        crate::dispatch::dispatch(
            store,
            "POST",
            "/projects/shop/batch",
            Some(&json!({"operations": [{"op": "feature.add", "title": "Cart", "milestone": "MS-001"}]})),
        )
        .unwrap();
        let pending = status(store, "shop", true).unwrap().unwrap();
        assert_eq!((pending.actions.len(), pending.linked), (1, 0));

        let out = sync(store, "shop", &t, true, &apply(store)).unwrap();
        assert_eq!(out.created, vec![("FEAT-001".to_string(), 1)]);
        let now = status(store, "shop", true).unwrap().unwrap();
        assert_eq!((now.actions.len(), now.linked), (0, 1));

        let report = pull(store, "shop", &t, "FEAT-001").unwrap();
        assert!(!report.edited_on_tracker && !report.changed_in_kanbanr);
        t.issues.borrow_mut().get_mut(&1).unwrap().title = "Edited".into();
        assert!(
            pull(store, "shop", &t, "FEAT-001")
                .unwrap()
                .edited_on_tracker
        );

        // Linking points the item at another issue; the next sync overwrites it.
        let copy = t.issues.borrow()[&1].clone();
        t.issues.borrow_mut().insert(7, copy);
        let url = link(store, "shop", &t, "FEAT-001", 7, &apply(store)).unwrap();
        assert!(url.ends_with("/issues/7"));
        let out = sync(store, "shop", &t, false, &apply(store)).unwrap();
        assert_eq!(out.updated, vec![("FEAT-001".to_string(), 7)]);
    }

    #[test]
    fn a_failed_record_names_what_was_pushed() {
        let b = board();
        let store = &b.0;
        let t = FakeTracker::default();
        save_config(store, "shop", &enable(&t, "acme/shop", false).unwrap()).unwrap();
        crate::dispatch::dispatch(
            store,
            "POST",
            "/projects/shop/batch",
            Some(&json!({"operations": [{"op": "feature.add", "title": "Cart", "milestone": "MS-001"}]})),
        )
        .unwrap();
        let err = sync(store, "shop", &t, true, &|_| Err("disk full".into())).unwrap_err();
        assert!(
            err.contains("recording the links failed (disk full)"),
            "{err}"
        );
        assert!(err.contains("FEAT-001 → #1"), "{err}");
    }
}
