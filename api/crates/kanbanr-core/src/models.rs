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
        self.tasks.iter().filter(|t| t.state == TaskState::Completed).count()
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
    /// Optional due date (e.g. an ISO date).
    #[serde(default)]
    pub due: Option<String>,
    /// Free-form labels/tags.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Other feature codes this one is blocked by (a cross-feature DAG; cycles are rejected).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Persistent todo-lists (newest-first ordering is applied by callers when displaying).
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    pub created_at: String,
    pub updated_at: String,
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
        self.all_tasks().filter(|t| t.state == TaskState::Completed).count()
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
            due: self.due.clone(),
            labels: self.labels.clone(),
            depends_on: self.depends_on.clone(),
            todo_lists: self.todo_lists.clone(),
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
            due: meta.due,
            labels: meta.labels,
            depends_on: meta.depends_on,
            todo_lists: meta.todo_lists,
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
    pub due: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    pub created_at: String,
    pub updated_at: String,
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
