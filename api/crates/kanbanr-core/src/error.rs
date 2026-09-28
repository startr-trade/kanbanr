//! Error type shared by the CLI and server.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// The board was written by a newer kanbanr than this one (FEAT-072). Refusing is the point:
    /// a reader that cannot find the files reports an empty board, and an empty board reads as lost
    /// data. Saying so names the remedy instead.
    #[error(
        "this board was written by a newer kanbanr (schema {found}; this build understands \
         {understood}) — upgrade the binary (`cargo install --path api/crates/kanbanr-cli`) and \
         restart anything long-running, such as `kanbanr serve`"
    )]
    SchemaTooNew { found: u32, understood: u32 },

    #[error("project '{0}' not found")]
    ProjectNotFound(String),
    #[error("project '{0}' already exists")]
    ProjectExists(String),
    #[error("feature '{0}' not found")]
    FeatureNotFound(String),
    #[error("feature '{0}' already exists")]
    FeatureExists(String),
    #[error("a milestone is required for a feature item")]
    MilestoneRequired,
    #[error("milestone '{0}' not found")]
    MilestoneNotFound(String),
    #[error("milestone '{0}' already exists")]
    MilestoneExists(String),
    #[error("milestone '{0}' is in use by {1} feature item(s) and cannot be deleted")]
    MilestoneInUse(String, usize),
    #[error("status '{0}' is in use by {1} feature item(s) and cannot be removed")]
    StatusInUse(String, usize),
    #[error("project '{0}' is not empty ({1} feature(s), {2} milestone(s)) and cannot be deleted")]
    ProjectNotEmpty(String, usize, usize),
    #[error("todo-list '{0}' not found on feature '{1}'")]
    TodoListNotFound(String, String),
    #[error("todo-list '{0}' already exists on feature '{1}'")]
    TodoListExists(String, String),
    #[error("task '{0}' not found in todo-list '{1}'")]
    TaskNotFound(String, String),
    #[error("task '{0}' already exists in todo-list '{1}'")]
    TaskExists(String, String),
    #[error("unknown status '{0}'")]
    UnknownStatus(String),
    #[error("at least one status is required")]
    NoStatuses,
    #[error("status '{0}' is a no-op state and cannot be displayed")]
    DisplayedNoOp(String),
    #[error("transition from '{from}' to '{to}' is not allowed")]
    TransitionNotAllowed { from: String, to: String },
    #[error("'{0}' is not a valid task state (use NotStarted | InProgress | Completed)")]
    InvalidTaskState(String),
    #[error("batch operation #{0} failed: {1}")]
    BatchOpFailed(usize, String),
    #[error("dependency cycle detected involving '{0}'")]
    DependencyCycle(String),
    #[error("dependency '{0}' does not exist")]
    UnknownDependency(String),
    #[error("invalid name '{0}': use letters, numbers, '-' or '_'")]
    InvalidName(String),
    #[error("invalid document path '{0}': must be a relative path without '..'")]
    InvalidDocPath(String),
    #[error("document '{0}' not found")]
    DocNotFound(String),
    #[error("invalid Mermaid state diagram: {0}")]
    InvalidMermaid(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
