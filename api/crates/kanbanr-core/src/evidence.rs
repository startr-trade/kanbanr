//! Test evidence and source provenance (FEAT-053, FEAT-042): what a test run proves, which recorded
//! results no longer have a test behind them, and which imported items lost their source.
//!
//! Core decides; the front end collects the facts (a run's output, whether a name is still in the
//! repository, whether a file exists) and applies the batch it is handed (FEAT-176). Each planner
//! returns the rows a front end shows and the batch operations that would correct the board, so
//! the report and the write can never disagree.

use crate::models::{Source, TestState};
use crate::store::Project;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Test name -> passed, parsed from a test runner's output. Handles `cargo test` (`test NAME ...
/// ok`) and the common `✓ NAME` / `✗ NAME` shape used by JS runners. Unknown formats yield
/// nothing, which is the right failure: recording a guess would be worse than recording nothing.
pub fn parse_results(text: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("test ") {
            // cargo: `test module::name ... ok` / `... FAILED`
            if let Some((name, verdict)) = rest.rsplit_once(" ... ") {
                let name = name.trim();
                match verdict.trim() {
                    "ok" => {
                        out.insert(name.to_string(), true);
                    }
                    "FAILED" => {
                        out.insert(name.to_string(), false);
                    }
                    _ => {}
                }
            }
        } else if let Some(name) = line
            .strip_prefix("\u{2713} ")
            .or_else(|| line.strip_prefix("PASS "))
        {
            out.insert(name.trim().to_string(), true);
        } else if let Some(name) = line
            .strip_prefix("\u{2717} ")
            .or_else(|| line.strip_prefix("FAIL "))
        {
            out.insert(name.trim().to_string(), false);
        }
    }
    out
}

/// The `test.state` operations a run's `results` call for, stamped with the project revision the
/// run saw. Matching by test NAME rather than by "whatever item is in progress" is what makes this
/// trustworthy: it records what ran, not what someone meant to run. A result already recorded at
/// this revision is left alone, so a repeated run says nothing.
pub fn plan_capture(
    project: &Project,
    results: &BTreeMap<String, bool>,
    rev: Option<&str>,
) -> Vec<Value> {
    let mut ops = Vec::new();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                if let Some(passed) = results.get(t.name.as_str()) {
                    let state = if *passed { "green" } else { "red" };
                    if t.state == TestState::parse(state).unwrap_or_default()
                        && (!passed || t.checked_rev.as_str() == rev.unwrap_or(""))
                    {
                        continue; // already recorded at this revision; nothing to say
                    }
                    let mut op = json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": t.name, "state": state,
                    });
                    if let Some(rev) = rev {
                        op["checked_rev"] = json!(rev);
                    }
                    ops.push(op);
                }
            }
        }
    }
    ops
}

/// The batch body that records a run's results, or `None` when there is nothing to record.
pub fn capture_batch(ops: Vec<Value>) -> Option<Value> {
    if ops.is_empty() {
        return None;
    }
    let n = ops.len();
    Some(json!({ "operations": ops, "message": format!("record {n} test result(s) from a run") }))
}

/// One tracked test and whether its name is still in the repository.
#[derive(Debug, Clone, Serialize)]
pub struct TrackedTest {
    pub code: String,
    pub requirement: String,
    pub test: String,
    pub state: TestState,
    pub found: bool,
}

/// Every tracked test, and the operations that return a vanished one's recorded result to
/// `planned`.
#[derive(Debug, Clone, Default)]
pub struct StaleTests {
    pub rows: Vec<TrackedTest>,
    pub ops: Vec<Value>,
}

impl StaleTests {
    /// The batch body that applies [`StaleTests::ops`], or `None` when nothing is stale.
    pub fn batch(&self) -> Option<Value> {
        if self.ops.is_empty() {
            return None;
        }
        let n = self.ops.len();
        Some(json!({
            "operations": self.ops,
            "message": format!("tests: {n} result(s) whose test no longer exists are back to planned"),
        }))
    }
}

/// Check that every tracked test name still exists (FEAT-053). A test that has been renamed or
/// deleted leaves its last result behind, and that stale green is the most misleading thing a
/// board can hold: it reports a requirement as proven by something nobody can run. Such a result
/// goes back to `planned` rather than being deleted — the requirement still needs a test, it just
/// no longer has one. `repo_search` answers whether a test name is still in the repository.
pub fn plan_stale_tests(project: &Project, repo_search: &dyn Fn(&str) -> bool) -> StaleTests {
    let mut out = StaleTests::default();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                let name = t.name.trim();
                // A manual check is performed by a person and has no name in the repo to find;
                // treating its absence as rot would push people to stop recording manual gates.
                if name.is_empty() || t.kind.trim().eq_ignore_ascii_case("manual") {
                    continue;
                }
                let found = repo_search(name);
                out.rows.push(TrackedTest {
                    code: f.code.clone(),
                    requirement: r.id.clone(),
                    test: name.to_string(),
                    state: t.state,
                    found,
                });
                // Only recorded evidence is worth correcting: a `planned` test is expected to be
                // absent — that is what planned means.
                if !found && t.state != TestState::Planned {
                    out.ops.push(json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": name, "state": "planned", "checked_rev": "",
                    }));
                }
            }
        }
    }
    out
}

/// One imported item's provenance, and whether its file source is still there.
#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    pub code: String,
    pub system: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub revision: Option<String>,
    pub url: Option<String>,
    pub imported_at: String,
    /// `None` for a source that cannot be checked locally (anything but a file).
    pub present: Option<bool>,
    pub missing_since: Option<String>,
}

/// Every imported item, and the edits that mark a lost source missing (or clear the mark).
#[derive(Debug, Clone, Default)]
pub struct SourcesPlan {
    pub rows: Vec<SourceRow>,
    pub edits: Vec<Value>,
}

impl SourcesPlan {
    /// The batch body that applies [`SourcesPlan::edits`], or `None` when every mark is current.
    pub fn batch(&self) -> Option<Value> {
        (!self.edits.is_empty()).then(
            || json!({"operations": self.edits, "message": "sources: update missing-source marks"}),
        )
    }
}

/// The file a file source names: `TODO.md:14` -> `TODO.md`.
pub fn source_file(src: &Source) -> &str {
    match src.reference.rsplit_once(':') {
        Some((file, line)) if line.chars().all(|c| c.is_ascii_digit()) => file,
        _ => src.reference.as_str(),
    }
}

/// Check imported items' file sources (FEAT-042). `exists` answers whether a path, relative to the
/// project folder, is there; `now` stamps a newly missing source.
pub fn plan_sources(project: &Project, exists: &dyn Fn(&str) -> bool, now: &str) -> SourcesPlan {
    let mut out = SourcesPlan::default();
    for f in &project.features {
        let Some(src) = &f.source else { continue };
        // Only file sources can be checked locally.
        let present = (src.system == "file").then(|| exists(source_file(src)));
        let mut updated = src.clone();
        match present {
            Some(false) if src.missing_since.is_none() => {
                updated.missing_since = Some(now.to_string())
            }
            Some(true) if src.missing_since.is_some() => updated.missing_since = None,
            _ => {}
        }
        if updated != *src {
            out.edits
                .push(json!({"op": "feature.edit", "code": f.code, "source": updated}));
        }
        out.rows.push(SourceRow {
            code: f.code.clone(),
            system: src.system.clone(),
            reference: src.reference.clone(),
            revision: src.revision.clone(),
            url: src.url.clone(),
            imported_at: src.imported_at.clone(),
            present,
            missing_since: updated.missing_since,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_is_read_from_the_formats_a_run_actually_prints() {
        let text = "\
running 3 tests
test hooks::tests::install_merges ... ok
test report::tests::cycle_time ... FAILED
test slow::case ... ignored
✓ cart retains items for 7 days
✗ cart empties on sign-out
PASS src/cart.test.ts
FAIL src/checkout.test.ts
some unrelated line ... ok";
        let results = parse_results(text);
        assert_eq!(results.get("hooks::tests::install_merges"), Some(&true));
        assert_eq!(results.get("report::tests::cycle_time"), Some(&false));
        assert_eq!(results.get("cart retains items for 7 days"), Some(&true));
        assert_eq!(results.get("cart empties on sign-out"), Some(&false));
        assert_eq!(results.get("src/cart.test.ts"), Some(&true));
        assert_eq!(results.get("src/checkout.test.ts"), Some(&false));
        // An ignored test proves nothing, and a line that merely ends in "ok" is not a result.
        assert_eq!(results.get("slow::case"), None);
        assert_eq!(results.len(), 6, "{results:?}");
        // Nothing recognizable: the hook stays quiet rather than guessing.
        assert!(parse_results("cargo build finished in 3.2s").is_empty());
    }

    fn project_with_tests(tests: &str) -> Project {
        let mut p = Project {
            id: "shop".into(),
            config: crate::config::ProjectConfig::default_for("shop"),
            milestones: vec![],
            features: vec![],
        };
        let def: crate::models::FeatureDefinition = crate::yaml::from_str(&format!(
            "requirements:\n  - id: R-1\n    text: x\n    tests:\n{tests}"
        ))
        .unwrap();
        let f: crate::models::FeatureItem = crate::yaml::from_str(
            "code: FEAT-001\ntitle: t\nstatus: Planned\nmilestone: MS-001\n\
             created_at: '2026-01-01T00:00:00Z'\nupdated_at: '2026-01-01T00:00:00Z'\n",
        )
        .unwrap();
        p.features.push(crate::models::FeatureItem {
            definition: Some(def),
            ..f
        });
        p
    }

    #[test]
    fn a_run_records_only_what_changed_at_this_revision() {
        let p = project_with_tests(
            "      - name: a::passes\n        state: green\n        checked_rev: abc\n\
             \x20     - name: b::fails\n        state: green\n        checked_rev: abc\n\
             \x20     - name: c::untracked_run\n",
        );
        let results = parse_results("test a::passes ... ok\ntest b::fails ... FAILED\n");
        let ops = plan_capture(&p, &results, Some("abc"));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0]["test"], "b::fails");
        assert_eq!(ops[0]["state"], "red");
        assert_eq!(ops[0]["checked_rev"], "abc");
        // A green at a new revision is worth stamping.
        let ops = plan_capture(&p, &results, Some("def"));
        assert_eq!(ops.len(), 2);
        assert!(capture_batch(vec![]).is_none());
        assert_eq!(
            capture_batch(ops).unwrap()["message"],
            "record 2 test result(s) from a run"
        );
    }

    #[test]
    fn a_vanished_test_with_a_result_goes_back_to_planned() {
        let p = project_with_tests(
            "      - name: gone::green\n        state: green\n\
             \x20     - name: gone::planned\n\
             \x20     - name: still::here\n        state: red\n\
             \x20     - name: tried by hand\n        kind: manual\n        state: green\n",
        );
        let plan = plan_stale_tests(&p, &|name| name.starts_with("still"));
        assert_eq!(plan.rows.len(), 3, "a manual check is not searched for");
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(plan.ops[0]["test"], "gone::green");
        assert_eq!(plan.ops[0]["state"], "planned");
        assert!(
            plan.batch().unwrap()["message"]
                .as_str()
                .unwrap()
                .starts_with("tests: 1")
        );
    }

    #[test]
    fn a_lost_file_source_is_marked_and_a_found_one_cleared() {
        let mut p = project_with_tests("      - name: x\n");
        let source = |reference: &str, missing: Option<&str>| Source {
            system: "file".into(),
            reference: reference.into(),
            revision: None,
            url: None,
            imported_at: "2026-01-01T00:00:00Z".into(),
            key: String::new(),
            missing_since: missing.map(String::from),
            extra: Default::default(),
        };
        let mut second = p.features[0].clone();
        second.code = "FEAT-002".into();
        second.source = Some(source("BACKLOG.md", Some("2026-01-02T00:00:00Z")));
        p.features[0].source = Some(source("TODO.md:14", None));
        p.features.push(second);
        let plan = plan_sources(&p, &|path| path == "BACKLOG.md", "2026-02-01T00:00:00Z");
        assert_eq!(plan.rows[0].present, Some(false));
        assert_eq!(
            plan.rows[0].missing_since.as_deref(),
            Some("2026-02-01T00:00:00Z")
        );
        assert_eq!(plan.rows[1].missing_since, None);
        assert_eq!(plan.edits.len(), 2);
        assert!(plan.batch().is_some());
        assert_eq!(source_file(&source("TODO.md:14", None)), "TODO.md");
        assert_eq!(source_file(&source("a:b.md", None)), "a:b.md");
    }
}
