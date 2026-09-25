//! Bulk/batch operations: a single bundled request that applies many changes in one call —
//! new feature items, edits (spec/milestone/status), documentation, new todo-lists + items, and
//! task-state updates. Newly-created items can be given a client-chosen `ref` alias that later
//! operations in the same bundle reference (since real codes are server-assigned).

use crate::models::{FeatureDefinition, IssueLink, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "op")]
pub enum BatchOp {
    /// Create a feature item. `ref` aliases the new FEAT code for later ops.
    #[serde(rename = "feature.add")]
    FeatureAdd {
        #[serde(default, rename = "ref")]
        alias: Option<String>,
        title: String,
        /// Milestone code (or a `ref` alias defined earlier in the bundle).
        milestone: String,
        #[serde(default)]
        spec: Option<String>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        due: Option<String>,
        #[serde(default)]
        assignee: Option<String>,
        #[serde(default)]
        team: Option<String>,
        #[serde(default)]
        labels: Option<Vec<String>>,
        /// Feature codes (or `ref` aliases) this one is blocked by.
        #[serde(default)]
        depends_on: Option<Vec<String>>,
        /// Where the item was imported from (FEAT-042). A feature whose source key already exists
        /// in the project is skipped, along with the bundle's ops that target its `ref`.
        #[serde(default)]
        source: Option<Source>,
        /// The item's original text, preserved in the spec under "Imported from".
        #[serde(default)]
        original: Option<String>,
        /// An existing external issue to mirror this feature to (FEAT-043).
        #[serde(default)]
        issue: Option<IssueLink>,
        /// Why this item exists, what must be true, and how it is verified (FEAT-047).
        #[serde(default)]
        definition: Option<FeatureDefinition>,
        /// What a defect cost and where it came from (FEAT-053).
        #[serde(default)]
        defect: Option<crate::models::Defect>,
        /// The item this was sliced out of (FEAT-054).
        #[serde(default)]
        split_from: Option<String>,
    },
    /// Edit a feature item (title / spec / milestone / rename / attrs). Use `feature.move` for status.
    #[serde(rename = "feature.edit")]
    FeatureEdit {
        code: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        spec: Option<String>,
        #[serde(default)]
        milestone: Option<String>,
        #[serde(default)]
        new_code: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        due: Option<String>,
        #[serde(default)]
        assignee: Option<String>,
        #[serde(default)]
        team: Option<String>,
        #[serde(default)]
        labels: Option<Vec<String>>,
        #[serde(default)]
        depends_on: Option<Vec<String>>,
        /// Replace the import provenance (FEAT-042).
        #[serde(default)]
        source: Option<Source>,
        /// Set the mirrored issue link (FEAT-043).
        #[serde(default)]
        issue: Option<IssueLink>,
        /// Replace the definition block (FEAT-047).
        #[serde(default)]
        definition: Option<FeatureDefinition>,
        /// What a defect cost and where it came from (FEAT-053).
        #[serde(default)]
        defect: Option<crate::models::Defect>,
        /// The item this was sliced out of (FEAT-054).
        #[serde(default)]
        split_from: Option<String>,
    },
    /// Move a feature to a new status (validated against the workflow). Entering an active status
    /// also requires a current approval (FEAT-048); `unapproved` records an explicit reason to go
    /// ahead without one.
    #[serde(rename = "feature.move")]
    FeatureMove {
        code: String,
        to: String,
        #[serde(default)]
        unapproved: Option<String>,
    },
    /// Move one test along the TDD lifecycle (FEAT-051): planned | red | green.
    #[serde(rename = "test.state")]
    TestState {
        feature: String,
        requirement: String,
        test: String,
        state: String,
        #[serde(default)]
        checked_rev: Option<String>,
    },
    /// Record a lesson learned (FEAT-055). Saying one that is already recorded affirms it.
    #[serde(rename = "lesson.add")]
    LessonAdd {
        lesson: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        from_item: Option<String>,
        #[serde(default)]
        from_retro: Option<String>,
        #[serde(default)]
        evidence: Option<String>,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        goals: Option<Vec<String>>,
    },
    /// Record agreement to an item's definition as it stands (FEAT-048).
    #[serde(rename = "feature.approve")]
    FeatureApprove {
        code: String,
        #[serde(default)]
        by: Option<String>,
    },
    /// Create a milestone. `ref` aliases the new MS code.
    #[serde(rename = "milestone.add")]
    MilestoneAdd {
        #[serde(default, rename = "ref")]
        alias: Option<String>,
        name: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        depends_on: Option<Vec<String>>,
    },
    /// Add a todo-list to a feature. `ref` aliases the new TL code.
    #[serde(rename = "todo.add")]
    TodoAdd {
        #[serde(default, rename = "ref")]
        alias: Option<String>,
        feature: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        code: Option<String>,
    },
    /// Add a task to a feature's todo-list.
    #[serde(rename = "task.add")]
    TaskAdd {
        feature: String,
        todo: String,
        text: String,
        #[serde(default)]
        key: Option<String>,
    },
    /// Update a task's state (NotStarted | InProgress | Completed).
    #[serde(rename = "task.state")]
    TaskState {
        feature: String,
        todo: String,
        key: String,
        state: String,
    },
    /// Create/update a documentation folder's name + description.
    #[serde(rename = "doc.folder")]
    DocFolder {
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    /// Create/overwrite a documentation markdown file.
    #[serde(rename = "doc.write")]
    DocWrite { path: String, content: String },
}
