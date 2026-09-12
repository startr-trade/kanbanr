//! Core domain models. All entities serialize to/from YAML.

use serde::{Deserialize, Serialize};

/// A status is a configurable label (e.g. "Planned"). Kept as a free string so the
/// workflow can be reconfigured per project without code changes.
pub type Status = String;

/// Tri-state for a feature's todo items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    NotStarted,
    InProgress,
    Completed,
}

impl TaskState {
    pub fn parse(s: &str) -> Option<TaskState> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "notstarted" | "todo" | "new" => Some(TaskState::NotStarted),
            "inprogress" | "wip" | "doing" => Some(TaskState::InProgress),
            "completed" | "done" | "complete" => Some(TaskState::Completed),
            _ => None,
        }
    }
}

/// A todo item belonging to a todo-list. `key` is short and unique within its todo-list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub key: String,
    pub text: String,
    pub state: TaskState,
}

/// A persistent todo-list attached to a feature item. A feature (acting as an epic) can hold
/// many of these — e.g. one added per working session — so the work survives across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoList {
    pub code: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<Task>,
    pub created_at: String,
}

impl TodoList {
    /// A list is fully completed when it has at least one task and all are Completed.
    pub fn fully_completed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.state == TaskState::Completed)
    }
    pub fn done_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.state == TaskState::Completed)
            .count()
    }
}

/// A feature item: the unit Claude develops. Holds the spec and its todo list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureItem {
    pub code: String,
    pub title: String,
    #[serde(default)]
    pub specification: String,
    pub status: Status,
    /// The milestone this feature belongs to (required).
    pub milestone: String,
    /// Work kind (feature / chore / bug / refactor / docs / recurring …); free text.
    #[serde(default)]
    pub kind: Option<String>,
    /// Priority (low / medium / high …); free text.
    #[serde(default)]
    pub priority: Option<String>,
    /// Optional planned start date (an ISO date, e.g. "2026-07-01"). Used by scheduling/Gantt.
    #[serde(default)]
    pub start: Option<String>,
    /// Optional due date (e.g. an ISO date).
    #[serde(default)]
    pub due: Option<String>,
    /// Optional estimated effort, in days (used as the task duration for scheduling/Gantt;
    /// defaults to 1 day when absent). Kept as a float so half-days etc. are expressible.
    #[serde(default)]
    pub estimate_days: Option<f64>,
    /// Optional assignee (the person/agent owning this feature); free text.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Optional owning team; free text.
    #[serde(default)]
    pub team: Option<String>,
    /// Free-form labels/tags.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Other feature codes this one is blocked by (a cross-feature DAG; cycles are rejected).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Persistent todo-lists (newest-first ordering is applied by callers when displaying).
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    /// Where this feature was imported from, if it was (FEAT-042).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The external issue this feature is mirrored to, if any (FEAT-043).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueLink>,
    pub created_at: String,
    pub updated_at: String,
}

/// Where an imported feature came from (FEAT-042). This is a record of history, not a live
/// pointer: the original text is preserved in the feature's spec, so nothing depends on the
/// source still existing (a deleted file, rewritten git history, a removed issue).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// Tracker kind: `file`, `github`, `gitlab`, `jira`, `linear`, …
    pub system: String,
    /// Location in that tracker, for humans: `TODO.md:14`, `owner/repo#123`, `PROJ-45`.
    #[serde(default, rename = "ref")]
    pub reference: String,
    /// Project repo commit at import time (file sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// When it was imported (RFC 3339); stamped by kanbanr.
    #[serde(default)]
    pub imported_at: String,
    /// Stable identity used to skip re-imports; derived by kanbanr when not given.
    #[serde(default)]
    pub key: String,
    /// When the source was found to be gone (set by `kanbanr sources --write`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_since: Option<String>,
}

impl Source {
    /// The re-import identity. Trackers with stable ids (anything but `file`) use
    /// `<system>:<ref>`; files use a hash of the normalized title, so a moved, renumbered or
    /// deleted file doesn't cause a re-import.
    pub fn derive_key(&self, title: &str) -> String {
        let system = self.system.trim().to_lowercase();
        let reference = self.reference.trim();
        if system != "file" && !reference.is_empty() {
            return format!("{system}:{reference}");
        }
        let normalized = title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        format!("{system}:{}", crate::hash::stable_hash(&normalized))
    }
}

/// A link from a feature to the external issue it is mirrored to (FEAT-043).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IssueLink {
    /// Tracker kind; currently `github`.
    pub system: String,
    /// `owner/repo`.
    pub repo: String,
    pub number: u64,
    #[serde(default)]
    pub url: String,
    /// Hash of the issue content kanbanr last pushed; `None` until the first push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,
}

impl FeatureItem {
    /// Iterate over every task across all todo-lists.
    pub fn all_tasks(&self) -> impl Iterator<Item = &Task> {
        self.todo_lists.iter().flat_map(|l| l.tasks.iter())
    }

    /// A feature is complete when it has at least one task and all tasks (across all
    /// todo-lists) are Completed.
    pub fn all_tasks_completed(&self) -> bool {
        let mut any = false;
        for t in self.all_tasks() {
            any = true;
            if t.state != TaskState::Completed {
                return false;
            }
        }
        any
    }

    pub fn done_count(&self) -> usize {
        self.all_tasks()
            .filter(|t| t.state == TaskState::Completed)
            .count()
    }
    pub fn task_count(&self) -> usize {
        self.all_tasks().count()
    }

    pub fn todo_list(&self, code: &str) -> Option<&TodoList> {
        self.todo_lists.iter().find(|l| l.code == code)
    }

    /// Split into the on-disk metadata (yaml) and the specification (separate .md file).
    pub fn meta(&self) -> FeatureMeta {
        FeatureMeta {
            code: self.code.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
            milestone: self.milestone.clone(),
            kind: self.kind.clone(),
            priority: self.priority.clone(),
            start: self.start.clone(),
            due: self.due.clone(),
            estimate_days: self.estimate_days,
            assignee: self.assignee.clone(),
            team: self.team.clone(),
            labels: self.labels.clone(),
            depends_on: self.depends_on.clone(),
            todo_lists: self.todo_lists.clone(),
            source: self.source.clone(),
            issue: self.issue.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }

    /// Reassemble from on-disk metadata and the spec markdown loaded separately.
    pub fn from_meta(meta: FeatureMeta, specification: String) -> FeatureItem {
        FeatureItem {
            code: meta.code,
            title: meta.title,
            specification,
            status: meta.status,
            milestone: meta.milestone,
            kind: meta.kind,
            priority: meta.priority,
            start: meta.start,
            due: meta.due,
            estimate_days: meta.estimate_days,
            assignee: meta.assignee,
            team: meta.team,
            labels: meta.labels,
            depends_on: meta.depends_on,
            todo_lists: meta.todo_lists,
            source: meta.source,
            issue: meta.issue,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
        }
    }
}

/// On-disk metadata for a feature (the `.yaml` file). The specification lives in a separate
/// `feature-specs/<code>.md` file and is not duplicated here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureMeta {
    pub code: String,
    pub title: String,
    pub status: Status,
    /// The milestone this feature belongs to (required).
    pub milestone: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub estimate_days: Option<f64>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueLink>,
    pub created_at: String,
    pub updated_at: String,
}

/// A compact, spec-free per-feature row stored in the per-project `index.yaml` cache (FEAT-033).
/// It carries everything board/list/graph/rollup callers need (status, milestone, attributes, and
/// the todo-list done/total counts) WITHOUT any specification body, so a project's whole feature
/// set can be read from one small file. It is a derivable cache: the status-folder yaml/md files
/// remain the source of truth, and the index is rebuilt from them by `Store::rebuild_index`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub code: String,
    pub status: Status,
    pub milestone: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub estimate_days: Option<f64>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Completed task count across all of the feature's todo-lists.
    #[serde(default)]
    pub done: usize,
    /// Total task count across all of the feature's todo-lists.
    #[serde(default)]
    pub total: usize,
}

impl IndexEntry {
    /// Build an index row from a (fully- or metadata-only-) loaded feature. The spec body is
    /// intentionally ignored — only the cheap, listable attributes are captured.
    pub fn from_feature(f: &FeatureItem) -> IndexEntry {
        IndexEntry {
            code: f.code.clone(),
            status: f.status.clone(),
            milestone: f.milestone.clone(),
            kind: f.kind.clone(),
            priority: f.priority.clone(),
            start: f.start.clone(),
            due: f.due.clone(),
            estimate_days: f.estimate_days,
            assignee: f.assignee.clone(),
            team: f.team.clone(),
            labels: f.labels.clone(),
            depends_on: f.depends_on.clone(),
            done: f.done_count(),
            total: f.task_count(),
        }
    }
}

/// A milestone groups feature items and may depend on other milestones (the DAG).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// NOTE: There is no `Schedule` entity. A "schedule" is a *derived view*: for a given feature
// status, the milestones that contain feature items in that status (ordered by their dependency
// DAG). The UI computes this from features + milestones; nothing is stored on disk.
