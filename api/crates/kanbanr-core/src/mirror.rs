//! Issue mirror (FEAT-043): keep a project's features in step with GitHub issues, **one
//! direction** — kanbanr is the source of truth and issues are a mirror of it.
//!
//! This module is the pure half: the per-project config (`mirror.yaml`), how a feature renders
//! as an issue, a stable content hash for change detection, and the reconcile **plan** (which
//! features need an issue created or updated). Talking to GitHub (via `gh`) lives in the CLI.

use crate::error::Result;
use crate::models::FeatureItem;
use crate::store::Project;
use crate::Store;
use serde::{Deserialize, Serialize};
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
                if terminal || !(all || f.created_at >= mirror.enabled_at) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{IssueLink, Status, Task, TaskState, TodoList};

    fn feature(code: &str, status: &str, created_at: &str) -> FeatureItem {
        FeatureItem {
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
            assignee: None,
            team: None,
            labels: vec!["ui".into(), "api".into(), "ui".into()],
            depends_on: vec![],
            todo_lists: vec![],
            source: None,
            issue: None,
            definition: None,
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
            code: "TL-001".into(),
            description: "session 1".into(),
            tasks: vec![
                Task {
                    key: "T1".into(),
                    text: "cart UI".into(),
                    state: TaskState::Completed,
                },
                Task {
                    key: "T2".into(),
                    text: "tests".into(),
                    state: TaskState::NotStarted,
                },
            ],
            created_at: "2026-09-11T00:00:00Z".into(),
        }];
        let p = project(vec![f]);
        let doc = render_issue(&p, &p.features[0]);
        assert!(doc
            .body
            .starts_with("> Tracked in kanbanr as `shop:FEAT-001`"));
        assert!(doc.body.contains("# Spec\nDo it."));
        assert!(doc
            .body
            .contains("### TL-001 — session 1\n\n- [x] cart UI\n- [ ] tests\n"));
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
        assert!(summary(true)
            .iter()
            .all(|(c, _)| c != "FEAT-003" && c != "FEAT-004" && c != "FEAT-006"));
    }
}
