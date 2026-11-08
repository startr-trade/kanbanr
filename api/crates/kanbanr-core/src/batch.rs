//! Bulk/batch operations: a single bundled request that applies many changes in one call —
//! new feature items, edits (spec/milestone/status), documentation, new todo-lists + items, and
//! task-state updates. Newly-created items can be given a client-chosen `ref` alias that later
//! operations in the same bundle reference (since real codes are server-assigned).

use crate::models::{FeatureDefinition, IssueLink, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
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

/// Where a sprint is changed, appended to the refusal of a `sprint` field in a batch (DEF-018).
const SPRINT_HINT: &str =
    " — a sprint is changed with `kanbanr sprint plan` and `kanbanr sprint unplan`, not in a batch";

/// The operations of a bundle's `operations` array, each judged on its own so a refusal can name
/// which one. `None` (no `operations` key) is an empty bundle, as before. An operation carrying a
/// field it does not accept is refused rather than ignored (DEF-018).
pub fn parse(operations: Option<&serde_json::Value>) -> Result<Vec<BatchOp>, String> {
    let Some(operations) = operations else {
        return Ok(Vec::new());
    };
    let Some(list) = operations.as_array() else {
        return Err("operations must be a list".to_string());
    };
    list.iter()
        .enumerate()
        .map(|(i, op)| {
            serde_json::from_value::<BatchOp>(op.clone()).map_err(|e| {
                let kind = op["op"].as_str().unwrap_or("?");
                let error = e.to_string();
                let hint = if error.contains("unknown field `sprint`") {
                    SPRINT_HINT
                } else {
                    ""
                };
                format!("operation {i} ({kind}): {error}{hint}")
            })
        })
        .collect()
}

/// A bundle as `POST /projects/{p}/batch` takes it (FEAT-176): a bare array of operations is
/// wrapped as `{ "operations": [...] }`, `message` (when given) is set on it, and each file-sourced
/// `feature.add` that does not say which revision of the file it came from is stamped with
/// `revision()` — asked only when one needs it, since finding it reads the project's repository
/// (FEAT-042).
pub fn normalize(
    bundle: serde_json::Value,
    message: Option<&str>,
    revision: &dyn Fn() -> Option<String>,
) -> serde_json::Value {
    let mut body = if bundle.is_array() {
        serde_json::json!({ "operations": bundle })
    } else {
        bundle
    };
    if let (Some(obj), Some(m)) = (body.as_object_mut(), message) {
        obj.insert("message".to_string(), serde_json::json!(m));
    }
    let Some(ops) = body["operations"].as_array_mut() else {
        return body;
    };
    let needs = |op: &serde_json::Value| {
        op["op"] == "feature.add"
            && op["source"]["system"] == "file"
            && op["source"]["revision"].is_null()
    };
    if !ops.iter().any(needs) {
        return body;
    }
    let Some(rev) = revision() else {
        return body;
    };
    for op in ops.iter_mut().filter(|op| needs(op)) {
        op["source"]["revision"] = serde_json::json!(rev);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// FEAT-176: an array is wrapped, the message set, and only a file source missing its
    /// revision is stamped — the repository is not asked when none needs it.
    #[test]
    fn normalize_wraps_and_stamps_file_sources() {
        let bundle = json!([
            { "op": "feature.add", "title": "a", "milestone": "MS-001",
              "source": { "system": "file", "key": "x.md" } },
            { "op": "feature.add", "title": "b", "milestone": "MS-001",
              "source": { "system": "file", "key": "y.md", "revision": "old" } },
        ]);
        let body = normalize(bundle, Some("import"), &|| Some("abc".into()));
        assert_eq!(body["message"], "import");
        assert_eq!(body["operations"][0]["source"]["revision"], "abc");
        assert_eq!(body["operations"][1]["source"]["revision"], "old");
        let asked = std::cell::Cell::new(false);
        let plain = json!({ "operations": [{ "op": "milestone.add", "title": "m" }] });
        let body = normalize(plain.clone(), None, &|| {
            asked.set(true);
            None
        });
        assert_eq!(body, plain);
        assert!(!asked.get());
    }

    #[test]
    fn an_unknown_field_refuses_the_batch_and_names_it() {
        let ops = json!([
            { "op": "task.add", "feature": "X", "todo": "TL-1", "text": "a" },
            { "op": "task.add", "feature": "X", "todo": "TL-1", "txt": "a" },
        ]);
        let err = parse(Some(&ops)).unwrap_err();
        assert!(err.contains("operation 1 (task.add)"), "{err}");
        assert!(err.contains("unknown field `txt`"), "{err}");
        assert!(
            parse(Some(&json!({})))
                .unwrap_err()
                .contains("must be a list")
        );
        assert!(parse(None).unwrap().is_empty());
    }

    #[test]
    fn the_refusal_for_sprint_points_to_sprint_plan_and_unplan() {
        let ops = json!([{ "op": "feature.edit", "code": "X", "sprint": "" }]);
        let err = parse(Some(&ops)).unwrap_err();
        assert!(err.contains("operation 0 (feature.edit)"), "{err}");
        assert!(err.contains("unknown field `sprint`"), "{err}");
        assert!(err.contains("kanbanr sprint plan"), "{err}");
        assert!(err.contains("kanbanr sprint unplan"), "{err}");
        // Another unknown field gets no sprint hint.
        let other = json!([{ "op": "feature.edit", "code": "X", "ref": "a" }]);
        assert!(!parse(Some(&other)).unwrap_err().contains("sprint"));
    }

    /// The shapes the skill's "Prefer bulk" example, the evidence capture and the mirror send,
    /// plus one op per variant using every field its struct declares.
    #[test]
    fn every_field_in_the_books_batch_examples_is_still_accepted() {
        let src = json!({ "system": "file", "key": "a.md", "revision": "r" });
        let ops = json!([
            { "op": "milestone.add", "ref": "m", "name": "M", "code": "MS-009",
              "description": "d", "depends_on": ["MS-001"] },
            { "op": "feature.add", "ref": "f", "title": "T", "milestone": "m", "spec": "s",
              "code": "FEAT-900", "kind": "feature", "priority": "high", "due": "2026-12-01",
              "assignee": "a", "team": "t", "labels": ["x"], "depends_on": ["FEAT-1"],
              "source": src, "original": "o",
              "issue": { "system": "github", "repo": "o/r", "number": 1 },
              "definition": {}, "defect": {}, "split_from": "FEAT-2" },
            { "op": "feature.edit", "code": "f", "title": "T", "spec": "s", "milestone": "m",
              "new_code": "FEAT-901", "kind": "feature", "priority": "low", "due": "2026-12-02",
              "assignee": "a", "team": "t", "labels": [], "depends_on": [],
              "source": src, "issue": { "system": "github", "repo": "o/r", "number": 1 },
              "definition": {}, "defect": {}, "split_from": "FEAT-2" },
            { "op": "feature.move", "code": "f", "to": "Done", "unapproved": "why" },
            { "op": "feature.approve", "code": "f", "by": "me" },
            { "op": "test.state", "feature": "f", "requirement": "R-1", "test": "t",
              "state": "green", "checked_rev": "abc" },
            { "op": "lesson.add", "lesson": "l", "kind": "k", "from_item": "f",
              "from_retro": "MS-001", "evidence": "e", "tags": [], "goals": [] },
            { "op": "todo.add", "ref": "t", "feature": "f", "description": "d", "code": "TL-1" },
            { "op": "task.add", "feature": "f", "todo": "t", "text": "x", "key": "T1" },
            { "op": "task.state", "feature": "f", "todo": "t", "key": "T1", "state": "Completed" },
            { "op": "doc.folder", "path": "a", "name": "A", "description": "d" },
            { "op": "doc.write", "path": "a/b.md", "content": "c" },
        ]);
        let parsed = parse(Some(&ops));
        assert_eq!(parsed.map(|v| v.len()), Ok(12));
    }
}
