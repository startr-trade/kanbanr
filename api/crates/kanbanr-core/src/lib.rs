//! kanbanr-core: domain models, YAML store, validation, and export.
//! Shared by the CLI (the only writer) and the read-only server.

pub mod activity;
pub mod adr;
pub mod batch;
pub mod charter;
pub mod claude;
pub mod config;
pub mod daylog;
pub mod dispatch;
pub mod docs;
pub mod doctor;
/// EARS classification and the ISO 25010 vocabulary, re-exported from the standalone
/// `ears-classifier` crate. It carries nothing of kanbanr in it and is published on its own; this
/// alias keeps every `crate::ears::…` call site unchanged.
pub use ears_classifier as ears;
pub mod error;
pub mod eventing;
pub mod export;
pub mod gantt;
pub mod git;
pub mod graph;
pub mod hash;
pub mod lessons;
pub mod mermaid;
pub mod mirror;
pub mod models;
pub mod portfolio;
pub mod project;
pub mod query;
pub mod readiness;
pub mod releases;
pub mod report;
pub mod retro;
pub mod scm;
pub mod sprints;
pub mod store;
pub mod trace;
pub mod validate;

pub use charter::{Charter, Goal, Stakeholder};
pub use config::ProjectConfig;
pub use error::{CoreError, Result};
pub use models::{
    FeatureItem, IndexEntry, IssueLink, Milestone, Source, Status, Task, TaskState, TodoList,
};
pub use store::{Project, Store};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Current UTC time as an RFC3339 string (used for created_at/updated_at).
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::TaskState;

    fn temp_store() -> (Store, tempdir::TempDirLike) {
        let dir = tempdir::TempDirLike::new();
        (Store::new(dir.path.clone()), dir)
    }

    // Minimal temp-dir helper so we don't pull an external dev-dependency.
    mod tempdir {
        use std::path::PathBuf;
        pub struct TempDirLike {
            pub path: PathBuf,
        }
        impl TempDirLike {
            pub fn new() -> Self {
                let mut path = std::env::temp_dir();
                let unique = format!(
                    "kanbanr-test-{}-{}",
                    std::process::id(),
                    COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                );
                path.push(unique);
                std::fs::create_dir_all(&path).unwrap();
                TempDirLike { path }
            }
        }
        impl Drop for TempDirLike {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    }

    fn new_project(store: &Store, id: &str) {
        store
            .init_project(id, ProjectConfig::default_for(id))
            .unwrap();
        // Features require a milestone, so seed one for the feature-oriented tests.
        store
            .add_milestone(id, "M", "", vec![], Some("M".into()))
            .unwrap();
    }

    #[test]
    fn code_generation_is_sequential() {
        let existing = vec!["FEAT-001".to_string(), "FEAT-003".to_string()];
        assert_eq!(validate::next_code("FEAT", &existing), "FEAT-004");
        assert_eq!(validate::next_code("FEAT", &[]), "FEAT-001");
    }

    #[test]
    fn transition_validation_respects_config() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Login", "spec", "M", None)
            .unwrap();
        // Default state is the declared default ("Planned").
        assert_eq!(f.status, "Planned");
        // Planned -> Completed is not allowed.
        let err = store
            .move_feature("demo", &f.code, "Completed")
            .unwrap_err();
        assert!(matches!(err, CoreError::TransitionNotAllowed { .. }));
        // Planned -> Scheduled is allowed.
        let f = store.move_feature("demo", &f.code, "Scheduled").unwrap();
        assert_eq!(f.status, "Scheduled");
    }

    #[test]
    fn new_feature_uses_declared_default_state() {
        let (store, _d) = temp_store();
        let mut cfg = ProjectConfig::default_for("demo");
        cfg.default_state = "Scheduled".to_string();
        store.init_project("demo", cfg).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let f = store.add_feature("demo", "X", "", "M", None).unwrap();
        assert_eq!(f.status, "Scheduled");
    }

    #[test]
    fn all_tasks_done_autocompletes_feature() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store.add_feature("demo", "Login", "", "M", None).unwrap();
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        // Tasks live in todo-lists now; auto-complete spans ALL of a feature's lists.
        let tl1 = store
            .add_todo_list("demo", &f.code, "session 1", None)
            .unwrap();
        let tl2 = store
            .add_todo_list("demo", &f.code, "session 2", None)
            .unwrap();
        assert_eq!(tl1.code, "TL-001");
        assert_eq!(tl2.code, "TL-002");
        let t1 = store
            .add_task("demo", &f.code, &tl1.code, "do a", None)
            .unwrap();
        store
            .add_task("demo", &f.code, &tl2.code, "do b", None)
            .unwrap();
        assert_eq!(t1.key, "T1");
        // Completing only list 1's task does not complete the feature (list 2 still open).
        store
            .set_task_state("demo", &f.code, &tl1.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(
            store.load("demo").unwrap().feature(&f.code).unwrap().status,
            "Scheduled"
        );
        // Completing the last open task (in list 2) auto-completes the feature.
        let updated = store
            .set_task_state("demo", &f.code, &tl2.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(updated.status, "Completed");
    }

    /// FEAT-061: auto-completion is how most items actually finish. When it left no transition
    /// behind, every flow number was blind to the normal path and reported only on items someone
    /// had moved by hand.
    /// FEAT-075: the gate treated everything non-terminal as starting work, so parking an item in
    /// `Deferred` was refused for want of an approval. The only ways past were to approve work
    /// nobody intended to start, or to record an escape for a non-event — both of which teach that
    /// the escape is routine, which is what the gate exists to prevent.
    #[test]
    fn the_start_gate_fires_on_starting_work_and_not_on_parking() {
        use crate::models::FeatureDefinition;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        // The gate is dormant without a charter, so adopt one.
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Agree the reasoning before building.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Nothing is built on reasoning nobody agreed to".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let f = store
            .add_feature("demo", "Work", "spec", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Something worth agreeing to".into(),
                    ..Default::default()
                }),
            )
            .unwrap();

        // Parking it is not starting it: `Deferred` is not a column the board displays.
        assert!(
            !store
                .load("demo")
                .unwrap()
                .config
                .displayed_states
                .contains(&"Deferred".to_string()),
            "this test's premise: Deferred is not displayed"
        );
        let parked = store
            .move_feature_approved("demo", &f.code, "Deferred", None)
            .unwrap();
        assert_eq!(parked.status, "Deferred", "parking needs no approval");

        // Starting it still does.
        store.move_feature("demo", &f.code, "Planned").unwrap();
        let refused = store
            .move_feature_approved("demo", &f.code, "Scheduled", None)
            .unwrap_err();
        assert!(
            refused.to_string().contains("not approved"),
            "starting work is still gated: {refused}"
        );

        // And the recorded escape still works, for the case it was built for.
        let started = store
            .move_feature_approved(
                "demo",
                &f.code,
                "Scheduled",
                Some("shipping under a deadline"),
            )
            .unwrap();
        assert_eq!(started.status, "Scheduled");
        assert_eq!(
            started.definition.unwrap().started_unapproved,
            "shipping under a deadline"
        );
    }

    /// FEAT-072: an older binary reading a newer board found no files and reported an empty board.
    /// The user saw a dashboard with nothing on it — including no Completed items — and reasonably
    /// read it as data loss. Refusing by name is the whole point.
    #[test]
    fn a_board_newer_than_the_reader_is_refused_by_name() {
        use crate::config::CURRENT_SCHEMA_VERSION;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        store
            .add_feature("demo", "Work", "# Work", "M", None)
            .unwrap();
        let config_path = d.path.join("projects/demo/config.yaml");

        // A board this build wrote: readable, and stamped with the version its content needs — the
        // base layout, since it declares no gates (FEAT-113).
        let config = store.load("demo").unwrap().config;
        assert_eq!(config.schema_version, crate::config::BASE_SCHEMA_VERSION);

        // A board from the future: refused, naming both versions and the remedy.
        let raw = std::fs::read_to_string(&config_path).unwrap();
        std::fs::write(
            &config_path,
            raw.replace(
                &format!("schema_version: {}", crate::config::BASE_SCHEMA_VERSION),
                &format!("schema_version: {}", CURRENT_SCHEMA_VERSION + 7),
            ),
        )
        .unwrap();
        let err = store.load("demo").unwrap_err();
        assert!(
            matches!(err, CoreError::SchemaTooNew { found, understood }
                if found == CURRENT_SCHEMA_VERSION + 7 && understood == CURRENT_SCHEMA_VERSION),
            "{err:?}"
        );
        let message = err.to_string();
        assert!(message.contains("newer kanbanr"), "{message}");
        assert!(
            message.contains("restart anything long-running"),
            "{message}"
        );
        // Every loader shares the guard — a metadata-only read must not slip past it.
        assert!(store.load_meta("demo").is_err());
        assert!(store.feature_spec("demo", "FEAT-001").is_err());

        // A board from the past is read as it stands: that direction is handled by reading both
        // shapes, and refusing it would break every upgrade.
        std::fs::write(
            &config_path,
            std::fs::read_to_string(&config_path).unwrap().replace(
                &format!("schema_version: {}", CURRENT_SCHEMA_VERSION + 7),
                "schema_version: 0",
            ),
        )
        .unwrap();
        let old = store.load("demo").unwrap();
        assert_eq!(old.features.len(), 1, "an older board still reads");
        assert_eq!(old.config.schema_version, 0);
    }

    /// FEAT-071: a board written before the status folders moved under `features/` must keep
    /// working, and must come up to date on its next write without any file existing in neither
    /// place along the way.
    #[test]
    fn a_board_with_status_folders_at_the_root_is_migrated_on_the_next_write() {
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Old", "# Old", "M", None)
            .unwrap();
        let project_dir = d.path.join("projects/demo");

        // Put the board back into the old shape: status folders at the project root.
        let from = project_dir.join("features").join("Planned");
        let to = project_dir.join("Planned");
        std::fs::rename(&from, &to).unwrap();
        std::fs::remove_dir_all(project_dir.join("features")).unwrap();
        assert!(to.join(format!("{}.yaml", f.code)).is_file());

        // Readable as it stands — an old board keeps working untouched.
        let loaded = store.load("demo").unwrap();
        assert_eq!(loaded.features.len(), 1);
        assert_eq!(loaded.feature(&f.code).unwrap().specification, "# Old");

        // The next write brings it up to date, and the item is in exactly one place.
        store
            .set_feature_attrs(
                "demo",
                &f.code,
                Some("chore".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(
            project_dir
                .join("features/Planned")
                .join(format!("{}.yaml", f.code))
                .is_file(),
            "moved under features/"
        );
        assert!(
            !to.exists(),
            "and the root folder is gone, not left as a duplicate"
        );
        // The spec moved with it, and nothing was lost.
        let loaded = store.load("demo").unwrap();
        assert_eq!(loaded.features.len(), 1);
        assert_eq!(loaded.feature(&f.code).unwrap().specification, "# Old");
        assert_eq!(
            loaded.feature(&f.code).unwrap().kind.as_deref(),
            Some("chore")
        );
    }

    /// FEAT-078: the review queue and `doctor` answered "what still needs agreement?" differently —
    /// the doctor named two items on the real board while the queue offered six, four of them
    /// Completed or deliberately Deferred. A signature on merged work changes nothing, and a queue
    /// padded with signatures that mean nothing is how the gate becomes theatre.
    #[test]
    fn the_review_queue_asks_only_where_agreement_still_matters() {
        use crate::models::FeatureDefinition;
        use serde_json::{Value, json};
        let (store, _d) = temp_store();
        new_project(&store, "demo");

        // One item per interesting status, each defined and none approved.
        let mut codes = Vec::new();
        for (title, status) in [
            ("Being built", "Scheduled"),
            ("Not started", "Planned"),
            ("Already done", "Completed"),
            ("Parked", "Deferred"),
            ("Dropped", "Out-of-Scope"),
        ] {
            let f = store.add_feature("demo", title, "spec", "M", None).unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(FeatureDefinition {
                        statement: format!("{title} matters"),
                        ..Default::default()
                    }),
                )
                .unwrap();
            // Route through legal transitions: the default workflow has no Planned -> Completed.
            let route: &[&str] = match status {
                "Planned" => &[],
                "Completed" => &["Scheduled", "Completed"],
                other => &[other],
            };
            for step in route {
                store.move_feature("demo", &f.code, step).unwrap();
            }
            codes.push((f.code, status));
        }

        let out = crate::dispatch::dispatch(&store, "GET", "/projects/demo/review", None).unwrap();
        let queue: Vec<Value> = serde_json::from_str(&out).unwrap();
        let listed: Vec<&str> = queue.iter().map(|v| v["code"].as_str().unwrap()).collect();

        // Completed, Deferred and the no-op disposition are not decisions anyone can still make.
        let expected: Vec<&str> = codes
            .iter()
            .filter(|(_, s)| *s == "Scheduled" || *s == "Planned")
            .map(|(c, _)| c.as_str())
            .collect();
        assert_eq!(listed, expected, "queue: {listed:?}");

        // R-2: work already being built without agreement is asked about first.
        assert_eq!(
            queue[0]["code"].as_str().unwrap(),
            codes[0].0,
            "the item already being worked on must come first"
        );

        // R-3: the queue's membership is the doctor's scope gate, not a second opinion on it.
        let project = store.load("demo").unwrap();
        for (code, status) in &codes {
            let live = crate::graph::is_live_work(&project.config, status);
            assert_eq!(
                live,
                listed.contains(&code.as_str()),
                "{code} ({status}): queue and scope gate disagree"
            );
        }

        // And an approved item drops out regardless of status.
        crate::dispatch::dispatch(
            &store,
            "POST",
            &format!("/projects/demo/features/{}/approve", codes[0].0),
            Some(&json!({ "by": "Ada L" })),
        )
        .unwrap();
        let out = crate::dispatch::dispatch(&store, "GET", "/projects/demo/review", None).unwrap();
        let queue: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert_eq!(queue.len(), expected.len() - 1);
    }

    /// FEAT-077 R-1: a verdict must name who gave it. Dispatch used to default `by` to "unknown",
    /// so any caller that forgot the field produced an approval attributable to nobody — which is
    /// how the monitor came to record twenty-eight of them as "reviewed in the monitor", a place.
    #[test]
    fn a_verdict_with_no_named_approver_is_refused() {
        use crate::models::FeatureDefinition;
        use serde_json::json;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();

        let approve = format!("/projects/demo/features/{}/approve", f.code);
        for body in [json!({}), json!({ "by": "" }), json!({ "by": "   " })] {
            let err = crate::dispatch::dispatch(&store, "POST", &approve, Some(&body))
                .expect_err("an unattributable approval must be refused");
            assert!(
                err.to_string().contains("must name who gave it"),
                "unhelpful refusal: {err}"
            );
        }
        // And it left nothing behind: the item is still waiting for agreement.
        let project = store.load("demo").unwrap();
        let untouched = project.feature(&f.code).unwrap();
        assert!(untouched.definition.as_ref().unwrap().approval.is_none());

        // A named approver is accepted, and the name is what gets recorded.
        crate::dispatch::dispatch(&store, "POST", &approve, Some(&json!({ "by": "Ada L" })))
            .unwrap();
        let project = store.load("demo").unwrap();
        let approved = project.feature(&f.code).unwrap();
        assert_eq!(
            approved
                .definition
                .as_ref()
                .unwrap()
                .approval
                .as_ref()
                .unwrap()
                .by,
            "Ada L"
        );

        // Withdrawing carries the same requirement — the record of who took it back matters more.
        let un = format!("/projects/demo/features/{}/unapprove", f.code);
        assert!(
            crate::dispatch::dispatch(&store, "POST", &un, Some(&json!({ "reason": "changed" })))
                .is_err()
        );
    }

    /// FEAT-109 R-1: work finished under a recorded bypass is still a question for a person. It
    /// used to reach only the doctor, and ratifying it only the CLI; the review queue now asks it,
    /// first, and stops asking once it is ratified. An item finished WITHOUT a bypass stays out.
    #[test]
    fn the_review_queue_lists_work_finished_without_agreement() {
        use crate::models::FeatureDefinition;
        use serde_json::{Value, json};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let define = |code: &str| {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart for 7 days".into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        };
        let queue = || -> Vec<Value> {
            let out =
                crate::dispatch::dispatch(&store, "GET", "/projects/demo/review", None).unwrap();
            serde_json::from_str(&out).unwrap()
        };

        let waiting = store
            .add_feature("demo", "Waiting", "spec", "M", None)
            .unwrap();
        define(&waiting.code);
        let bypassed = store
            .add_feature("demo", "Bypassed", "spec", "M", None)
            .unwrap();
        define(&bypassed.code);
        store
            .move_feature_approved("demo", &bypassed.code, "Scheduled", Some("urgent"))
            .unwrap();
        store
            .move_feature("demo", &bypassed.code, "Completed")
            .unwrap();
        // Finished without ever having been asked: not a bypass, so not a ratification question.
        let done = store
            .add_feature("demo", "Done", "spec", "M", None)
            .unwrap();
        define(&done.code);
        store.move_feature("demo", &done.code, "Scheduled").unwrap();
        store.move_feature("demo", &done.code, "Completed").unwrap();

        let q = queue();
        let codes: Vec<&str> = q.iter().filter_map(|v| v["code"].as_str()).collect();
        assert_eq!(
            codes,
            [bypassed.code.as_str(), waiting.code.as_str()],
            "{q:?}"
        );
        assert_eq!(q[0]["approval"], "unratified");
        assert_eq!(q[0]["started_unapproved"], "urgent");
        assert_eq!(q[1]["approval"], "missing");

        crate::dispatch::dispatch(
            &store,
            "POST",
            &format!("/projects/demo/features/{}/ratify", bypassed.code),
            Some(&json!({ "by": "Ada L" })),
        )
        .unwrap();
        assert!(
            !queue().iter().any(|v| v["code"] == bypassed.code.as_str()),
            "a ratified item leaves the queue"
        );
    }

    /// FEAT-080: agreement given AFTER the work is a different claim from agreement given before
    /// it, and the record has to be able to say which. Twenty-nine items on this board were
    /// reconciled by approving them late, each indistinguishable from prior agreement — which
    /// erases the one distinction the whole gate exists to draw.
    #[test]
    fn agreement_after_the_work_is_recorded_as_a_different_verdict() {
        use crate::models::{ApprovalState, FeatureDefinition};
        use serde_json::{Value, json};
        let (store, _d) = temp_store();
        new_project(&store, "demo");

        let define = |code: &str| {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart for 7 days".into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        };

        // An item that never took the bypass cannot be ratified — that would make ratification a
        // shortcut around review rather than an answer to a question that was actually asked.
        let plain = store
            .add_feature("demo", "Plain", "spec", "M", None)
            .unwrap();
        define(&plain.code);
        let err = store
            .ratify_feature("demo", &plain.code, "Ada L", "")
            .expect_err("ratifying an item that never bypassed must be refused");
        assert!(err.to_string().contains("nothing to ratify"), "{err}");
        assert!(
            err.to_string().contains("kanbanr approve"),
            "it must name the right command: {err}"
        );

        // An item that DID start under a recorded bypass.
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        define(&f.code);
        store
            .move_feature_approved("demo", &f.code, "Scheduled", Some("urgent"))
            .unwrap();
        let project = store.load("demo").unwrap();
        let def = project
            .feature(&f.code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap();
        assert_eq!(def.approval_state(), ApprovalState::Missing);
        assert!(
            !def.started_unapproved.trim().is_empty(),
            "the bypass is on the record"
        );

        // Ratifying answers it, and says so as its own verdict.
        crate::dispatch::dispatch(
            &store,
            "POST",
            &format!("/projects/demo/features/{}/ratify", f.code),
            Some(&json!({ "by": "Ada L", "reason": "reviewed after the fact" })),
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let def = project
            .feature(&f.code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap();
        assert_eq!(
            def.approval_state(),
            ApprovalState::Ratified,
            "ratified is NOT the same state as approved"
        );
        assert!(def.was_ratified());
        assert_eq!(def.approvals.last().unwrap().verdict, "ratified");
        assert_eq!(
            def.approvals.last().unwrap().reason,
            "reviewed after the fact"
        );
        // The bypass reason survives: it is history, and the charter says raw data is not discarded.
        assert!(!def.started_unapproved.trim().is_empty());

        // It satisfies the gate — agreed is agreed, however late.
        store
            .move_feature_approved("demo", &f.code, "Completed", None)
            .unwrap();

        // And it leaves the review queue, because it is no longer a decision waiting to be made.
        let out = crate::dispatch::dispatch(&store, "GET", "/projects/demo/review", None).unwrap();
        let queue: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert!(
            !queue.iter().any(|v| v["code"] == f.code.as_str()),
            "a ratified item is not still awaiting review: {queue:?}"
        );

        // Editing the definition afterwards lapses it, exactly as an ordinary approval lapses —
        // the agreement no longer covers what was built.
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 30 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let project = store.load("demo").unwrap();
        let def = project
            .feature(&f.code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap();
        assert_eq!(def.approval_state(), ApprovalState::Lapsed);
    }

    /// FEAT-069: an approval recorded in error had no remedy. Found the direct way — Claude
    /// approved an item on the user's behalf, which the method forbids, and nothing could undo it.
    #[test]
    fn an_approval_can_be_withdrawn_and_the_record_survives() {
        use crate::models::{ApprovalState, FeatureDefinition};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();

        // Nothing to withdraw before anything was agreed.
        assert!(
            store
                .unapprove_feature("demo", &f.code, "me", "why")
                .is_err()
        );

        let approved = store.approve_feature("demo", &f.code, "the user").unwrap();
        let definition = approved.definition.as_ref().unwrap();
        let rev_when_approved = definition.content_rev();
        assert_eq!(definition.approval_state(), ApprovalState::Current);
        assert_eq!(definition.approvals.len(), 1);
        assert_eq!(definition.approvals[0].verdict, "approved");

        let withdrawn = store
            .unapprove_feature("demo", &f.code, "the user", "approved by the wrong hand")
            .unwrap();
        let definition = withdrawn.definition.as_ref().unwrap();
        // The agreement is gone…
        assert!(definition.approval.is_none());
        assert_eq!(definition.approval_state(), ApprovalState::Missing);
        // …and both verdicts are kept, with the reason the withdrawal needed.
        assert_eq!(definition.approvals.len(), 2);
        assert_eq!(definition.approvals[1].verdict, "withdrawn");
        assert_eq!(definition.approvals[1].reason, "approved by the wrong hand");
        assert_eq!(
            definition.approvals[0].verdict, "approved",
            "nothing was removed"
        );

        // Recording either verdict must never itself change what was agreed to.
        assert_eq!(
            definition.content_rev(),
            rev_when_approved,
            "the record of who agreed is not part of the scope they agreed to"
        );

        // Approving again is a third entry, not an edit of the first.
        let again = store.approve_feature("demo", &f.code, "the user").unwrap();
        let definition = again.definition.as_ref().unwrap();
        assert_eq!(definition.approvals.len(), 3);
        assert_eq!(definition.approval_state(), ApprovalState::Current);
        assert_eq!(definition.content_rev(), rev_when_approved);
    }

    #[test]
    fn auto_completion_records_the_move_like_any_other() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Login", "spec", "M", None)
            .unwrap();
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        let tl = store.add_todo_list("demo", &f.code, "s1", None).unwrap();
        store
            .add_task("demo", &f.code, &tl.code, "do a", None)
            .unwrap();

        let done = store
            .set_task_state("demo", &f.code, &tl.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(done.status, "Completed");
        let steps: Vec<(&str, &str)> = done
            .history
            .iter()
            .map(|t| (t.from.as_str(), t.to.as_str()))
            .collect();
        assert_eq!(
            steps,
            vec![("Planned", "Scheduled"), ("Scheduled", "Completed")],
            "the auto-completion is in the history beside the manual move"
        );
        assert!(!done.history[1].at.is_empty());

        // Renaming a status is not a move: the item stayed where it was, the label changed.
        store.rename_status("demo", "Completed", "Shipped").unwrap();
        let after = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        assert_eq!(after.status, "Shipped");
        assert_eq!(after.history.len(), 2, "no phantom transition for a rename");
    }

    #[test]
    fn milestone_dependency_cycle_is_rejected() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        store
            .add_milestone("demo", "A", "", vec![], Some("A".into()))
            .unwrap();
        store
            .add_milestone("demo", "B", "", vec!["A".into()], Some("B".into()))
            .unwrap();
        // Now make A depend on B -> cycle.
        let err = store
            .edit_milestone("demo", "A", None, None, Some(vec!["B".into()]))
            .unwrap_err();
        assert!(matches!(err, CoreError::DependencyCycle(_)));
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let err = store
            .add_milestone("demo", "A", "", vec!["NOPE".into()], Some("A".into()))
            .unwrap_err();
        assert!(matches!(err, CoreError::UnknownDependency(_)));
    }

    #[test]
    fn status_change_moves_the_files() {
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Login", "# spec", "M", None)
            .unwrap();
        let proj = d.path.join("projects").join("demo");
        // Default state is "Planned", so the files start in Planned/.
        let planned_yaml = proj.join("features").join("Planned").join("FEAT-001.yaml");
        let planned_spec = proj
            .join("features")
            .join("Planned")
            .join("features-spec")
            .join("FEAT-001.md");
        assert!(planned_yaml.is_file(), "yaml should start in Planned/");
        assert!(
            planned_spec.is_file(),
            "spec md should start in Planned/features-spec/"
        );

        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        assert!(!planned_yaml.exists(), "old yaml removed after move");
        assert!(!planned_spec.exists(), "old spec removed after move");
        assert!(
            proj.join("features")
                .join("Scheduled")
                .join("FEAT-001.yaml")
                .is_file()
        );
        assert!(
            proj.join("features")
                .join("Scheduled")
                .join("features-spec")
                .join("FEAT-001.md")
                .is_file()
        );

        // The yaml metadata must NOT contain the specification (it lives in the .md).
        let yaml = std::fs::read_to_string(
            proj.join("features")
                .join("Scheduled")
                .join("FEAT-001.yaml"),
        )
        .unwrap();
        assert!(
            !yaml.contains("specification"),
            "spec must not be in the yaml"
        );
        // Spec survives the move and reloads.
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .specification,
            "# spec"
        );
    }

    #[test]
    fn feature_persists_as_yaml_and_reloads() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Login", "# spec", "M", None)
            .unwrap();
        assert_eq!(f.code, "FEAT-001");
        let reloaded = store.load("demo").unwrap();
        assert_eq!(reloaded.features.len(), 1);
        assert_eq!(reloaded.features[0].title, "Login");
    }

    #[test]
    fn load_meta_omits_specs_load_keeps_them() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        store
            .add_feature("demo", "Login", "# the full spec body", "M", None)
            .unwrap();

        // load_meta returns correct metadata but EMPTY specification bodies.
        let meta = store.load_meta("demo").unwrap();
        assert_eq!(meta.features.len(), 1);
        let f = &meta.features[0];
        assert_eq!(f.code, "FEAT-001");
        assert_eq!(f.title, "Login");
        assert_eq!(f.status, "Planned");
        assert_eq!(f.milestone, "M");
        assert_eq!(f.specification, "", "load_meta must not populate specs");

        // feature_spec loads the one body on demand.
        assert_eq!(
            store.feature_spec("demo", "FEAT-001").unwrap(),
            "# the full spec body"
        );
        // Unknown code is a not-found error.
        assert!(matches!(
            store.feature_spec("demo", "NOPE").unwrap_err(),
            CoreError::FeatureNotFound(_)
        ));

        // load() still returns the full spec (its contract is unchanged).
        let full = store.load("demo").unwrap();
        assert_eq!(
            full.feature("FEAT-001").unwrap().specification,
            "# the full spec body"
        );
    }

    #[test]
    fn index_is_written_on_add_and_matches_meta_then_rebuilds() {
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let a = store
            .add_feature("demo", "A", "# spec a", "M", None)
            .unwrap();
        store
            .set_feature_attrs(
                "demo",
                &a.code,
                Some("chore".into()),
                Some("high".into()),
                None,
                Some("alice".into()),
                Some("core".into()),
                Some(vec!["infra".into()]),
                None,
            )
            .unwrap();
        store
            .add_feature("demo", "B", "# spec b", "M", None)
            .unwrap();

        // index.yaml is written on a feature mutation and lives at the project root.
        let index_file = d.path.join("projects").join("demo").join("index.yaml");
        assert!(index_file.is_file(), "index.yaml should be written on add");
        // It must NOT carry spec bodies.
        let raw = std::fs::read_to_string(&index_file).unwrap();
        assert!(!raw.contains("spec a") && !raw.contains("specification"));

        // load_index matches load_meta (same codes/status/attrs, with done/total counts).
        let index = store.load_index("demo").unwrap();
        let meta = store.load_meta("demo").unwrap();
        assert_eq!(index.len(), meta.features.len());
        for f in &meta.features {
            let e = index.iter().find(|e| e.code == f.code).unwrap();
            assert_eq!(e.status, f.status);
            assert_eq!(e.milestone, f.milestone);
            assert_eq!(e.kind, f.kind);
            assert_eq!(e.priority, f.priority);
            assert_eq!(e.assignee, f.assignee);
            assert_eq!(e.team, f.team);
            assert_eq!(e.labels, f.labels);
            assert_eq!(e.done, f.done_count());
            assert_eq!(e.total, f.task_count());
        }

        // rebuild_index reconstructs the file after deletion.
        std::fs::remove_file(&index_file).unwrap();
        assert!(!index_file.exists());
        store.rebuild_index("demo").unwrap();
        assert!(index_file.is_file(), "rebuild_index recreates index.yaml");
        assert_eq!(store.load_index("demo").unwrap(), index);

        // load_index also self-heals: a missing file is rebuilt on read.
        std::fs::remove_file(&index_file).unwrap();
        let healed = store.load_index("demo").unwrap();
        assert!(index_file.is_file());
        assert_eq!(healed, index);
    }

    #[test]
    fn no_op_states_are_non_displayed_and_inert() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let p = store.load("demo").unwrap();
        // The 3 default no-op states exist and are NOT displayed.
        assert!(p.config.no_op_states.iter().any(|s| s == "Out-of-Scope"));
        for n in &p.config.no_op_states {
            assert!(
                !p.config.displayed_states.contains(n),
                "no-op must be non-displayed"
            );
        }
        // A feature in a no-op state stays inert: finishing its tasks does NOT auto-complete it.
        let f = store.add_feature("demo", "F", "", "M", None).unwrap();
        store.move_feature("demo", &f.code, "Out-of-Scope").unwrap();
        let tl = store.add_todo_list("demo", &f.code, "x", None).unwrap();
        store
            .add_task("demo", &f.code, &tl.code, "t", None)
            .unwrap();
        let updated = store
            .set_task_state("demo", &f.code, &tl.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(updated.status, "Out-of-Scope", "no-op state is inert");
        // A no-op state can never be displayed.
        assert!(matches!(
            store
                .set_displayed_states("demo", vec!["Out-of-Scope".into()])
                .unwrap_err(),
            CoreError::DisplayedNoOp(_)
        ));
    }

    #[test]
    fn referential_integrity_guards_deletes() {
        let (store, _d) = temp_store();
        new_project(&store, "demo"); // seeds milestone "M"
        store.add_feature("demo", "F", "", "M", None).unwrap();

        // Milestone "M" is referenced by the feature -> cannot delete.
        assert!(matches!(
            store.delete_milestone("demo", "M").unwrap_err(),
            CoreError::MilestoneInUse(_, 1)
        ));
        // The feature's current status (Planned) is in use -> cannot be removed by a workflow reset.
        let wf = store.set_workflow(
            "demo",
            vec!["Scheduled".into(), "Completed".into()], // drops Planned (in use)
            Default::default(),
            None,
            None,
            None,
            None,
        );
        assert!(matches!(wf.unwrap_err(), CoreError::StatusInUse(_, 1)));
        // A project with feature items can NEVER be deleted (features are permanent — no delete).
        assert!(matches!(
            store.delete_project("demo").unwrap_err(),
            CoreError::ProjectNotEmpty(_, _, _)
        ));

        // Milestones are removable when unreferenced; an empty project deletes cleanly.
        store
            .init_project("empty", ProjectConfig::default_for("empty"))
            .unwrap();
        store
            .add_milestone("empty", "Marker", "", vec![], Some("MK".into()))
            .unwrap();
        store.delete_milestone("empty", "MK").unwrap(); // no features reference it
        store.delete_project("empty").unwrap(); // now empty
        assert!(!store.project_exists("empty"));
    }

    #[test]
    fn batch_applies_bundle_with_ref_aliases() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let ops: Vec<crate::batch::BatchOp> = serde_json::from_value(serde_json::json!([
            {"op":"milestone.add","ref":"m1","name":"Auth","code":"MS-9"},
            {"op":"feature.add","ref":"f1","title":"Login","milestone":"m1","spec":"# Login"},
            {"op":"feature.move","code":"f1","to":"Scheduled"},
            {"op":"todo.add","ref":"t1","feature":"f1","description":"session 1"},
            {"op":"task.add","feature":"f1","todo":"t1","text":"do it"},
            {"op":"task.state","feature":"f1","todo":"t1","key":"T1","state":"Completed"},
            {"op":"doc.write","path":"design/overview","content":"# Overview"}
        ]))
        .unwrap();
        let results = store.apply_batch("demo", ops).unwrap();
        assert_eq!(results.len(), 7);

        let p = store.load("demo").unwrap();
        let f = p.features.iter().find(|f| f.title == "Login").unwrap();
        assert_eq!(
            f.milestone, "MS-9",
            "ref alias resolved to the new milestone code"
        );
        // Scheduled then all-tasks-done -> auto-completed.
        assert_eq!(f.status, "Completed");
        assert_eq!(f.todo_lists.len(), 1);
        assert!(store.read_doc("demo", "design/overview.md").is_ok());

        // A failing op reports its index.
        let bad: Vec<crate::batch::BatchOp> = serde_json::from_value(serde_json::json!([
            {"op":"feature.add","title":"X","milestone":"NOPE"}
        ]))
        .unwrap();
        assert!(matches!(
            store.apply_batch("demo", bad).unwrap_err(),
            CoreError::BatchOpFailed(0, _)
        ));
    }

    /// FEAT-093: a batch whose sixth op failed left five applied — an item auto-completed, the
    /// index rewritten — on disk and uncommitted, because the failure path flushed what had run and
    /// the caller commits only on success. The contract is all or nothing, and that is asserted on
    /// the FILES, not on a reload that could share the same in-memory mistake.
    #[test]
    fn a_failed_batch_leaves_the_board_untouched() {
        let (store, dir) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        store
            .apply_batch(
                "demo",
                ops(serde_json::json!([
                    {"op":"todo.add","ref":"t","feature":f.code,"description":"work"},
                    {"op":"task.add","feature":f.code,"todo":"t","text":"one"},
                    {"op":"doc.write","path":"notes/keep.md","content":"original"},
                ])),
            )
            .unwrap();

        // Every file under the project, with its bytes, before the failing bundle.
        let snapshot = |root: &std::path::Path| {
            let mut all = std::collections::BTreeMap::new();
            let mut stack = vec![root.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        if p.file_name().is_some_and(|n| n == ".git") {
                            continue;
                        }
                        stack.push(p);
                    } else {
                        all.insert(p.clone(), std::fs::read(&p).unwrap());
                    }
                }
            }
            all
        };
        let before = snapshot(&dir.path);

        // The shape of the real failure: ops that move state and write docs, then one that fails.
        let err = store
            .apply_batch(
                "demo",
                ops(serde_json::json!([
                    {"op":"feature.move","code":f.code,"to":"Scheduled"},
                    {"op":"task.state","feature":f.code,"todo":"TL-001","key":"T1","state":"Completed"},
                    {"op":"doc.write","path":"notes/keep.md","content":"OVERWRITTEN"},
                    {"op":"doc.write","path":"notes/new.md","content":"should never exist"},
                    {"op":"task.state","feature":f.code,"todo":"TL-001","key":"T9","state":"Completed"},
                ])),
            )
            .expect_err("the last op names a task that does not exist");
        assert!(matches!(err, CoreError::BatchOpFailed(4, _)), "{err}");

        let after = snapshot(&dir.path);
        let changed: Vec<_> = after
            .iter()
            .filter(|(p, b)| before.get(*p) != Some(*b))
            .map(|(p, _)| p.display().to_string())
            .chain(
                before
                    .keys()
                    .filter(|p| !after.contains_key(*p))
                    .map(|p| p.display().to_string()),
            )
            .collect();
        assert!(
            changed.is_empty(),
            "a failed batch must change nothing on disk, but these changed: {changed:?}"
        );
        assert_eq!(
            store.load("demo").unwrap().feature(&f.code).unwrap().status,
            "Planned"
        );
        assert_eq!(store.read_doc("demo", "notes/keep.md").unwrap(), "original");
        assert!(store.read_doc("demo", "notes/new.md").is_err());
    }

    fn ops(v: serde_json::Value) -> Vec<crate::batch::BatchOp> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn batch_import_records_provenance_and_preserves_the_original() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let results = store
            .apply_batch(
                "demo",
                ops(serde_json::json!([
                    {"op":"milestone.add","ref":"m","name":"Imported","code":"MS-1"},
                    {"op":"feature.add","ref":"a","title":"Fix  login ","milestone":"m",
                     "spec":"# Fix login",
                     "source":{"system":"file","ref":"TODO.md:14","revision":"a1b2c3d"},
                     "original":"- [ ] Fix login (see ```notes```)"},
                    {"op":"feature.add","title":"Crash on save","milestone":"m",
                     "source":{"system":"github","ref":"acme/app#12","url":"https://github.com/acme/app/issues/12"},
                     "issue":{"system":"github","repo":"acme/app","number":12,"url":"https://github.com/acme/app/issues/12"}}
                ])),
            )
            .unwrap();
        assert_eq!(results.len(), 3);

        let p = store.load("demo").unwrap();
        let a = p
            .features
            .iter()
            .find(|f| f.title == "Fix  login ")
            .unwrap();
        let src = a.source.as_ref().unwrap();
        assert_eq!(src.reference, "TODO.md:14");
        assert_eq!(src.revision.as_deref(), Some("a1b2c3d"));
        assert!(!src.imported_at.is_empty(), "imported_at is stamped");
        assert!(src.key.starts_with("file:"), "file keys hash the title");
        assert!(a.specification.starts_with("# Fix login"));
        assert!(a.specification.contains("## Imported from"));
        assert!(
            a.specification
                .contains("`TODO.md:14` (file) at commit `a1b2c3d`")
        );
        assert!(
            a.specification
                .contains("````text\n- [ ] Fix login (see ```notes```)\n````"),
            "fence outgrows backtick runs: {}",
            a.specification
        );

        let b = p
            .features
            .iter()
            .find(|f| f.title == "Crash on save")
            .unwrap();
        assert_eq!(b.source.as_ref().unwrap().key, "github:acme/app#12");
        assert_eq!(b.issue.as_ref().unwrap().number, 12);
    }

    #[test]
    fn batch_reimport_skips_known_sources_and_the_ops_under_them() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let bundle = serde_json::json!([
            {"op":"milestone.add","ref":"m","name":"Imported"},
            // Title differs only in case/whitespace: same file key.
            {"op":"feature.add","ref":"a","title":"Fix login","milestone":"m",
             "source":{"system":"file","ref":"TODO.md:3"}},
            {"op":"todo.add","ref":"t","feature":"a","description":"imported"},
            {"op":"task.add","feature":"a","todo":"t","text":"write test"},
            {"op":"task.state","feature":"a","todo":"t","key":"T1","state":"InProgress"},
            {"op":"feature.add","ref":"b","title":"New thing","milestone":"M","depends_on":["a"],
             "source":{"system":"file","ref":"ROADMAP.md:9"}}
        ]);
        store.apply_batch("demo", ops(bundle.clone())).unwrap();
        assert_eq!(store.load("demo").unwrap().features.len(), 2);

        // The same items again (one moved to another file, retitled in case) plus one new item.
        let mut again = bundle.as_array().unwrap().clone();
        again[1]["title"] = serde_json::json!("FIX   LOGIN");
        again[1]["source"]["ref"] = serde_json::json!("docs/old-todo.md:40");
        again.push(serde_json::json!(
            {"op":"feature.add","title":"Brand new","milestone":"M","source":{"system":"file","ref":"TODO.md:5"}}
        ));
        let results = store
            .apply_batch("demo", ops(serde_json::Value::Array(again)))
            .unwrap();
        let skipped = results.iter().filter(|r| r["skipped"] == true).count();
        assert_eq!(
            skipped, 6,
            "the milestone, both known items and the ops under them: {results:?}"
        );

        let p = store.load("demo").unwrap();
        assert_eq!(p.features.len(), 3, "only the new item was added");
        assert_eq!(
            p.milestones.iter().filter(|m| m.name == "Imported").count(),
            1,
            "milestone reused, not duplicated"
        );
        let a = p.features.iter().find(|f| f.title == "Fix login").unwrap();
        assert_eq!(a.todo_lists.len(), 1, "no duplicate todo-list");
        assert_eq!(a.todo_lists[0].tasks.len(), 1, "no duplicate task");
    }

    #[test]
    fn batch_dry_run_reports_without_writing() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let before = store.load("demo").unwrap().features.len();
        let results = store
            .apply_batch_with(
                "demo",
                ops(serde_json::json!([
                    {"op":"feature.add","ref":"a","title":"Preview me","milestone":"M"},
                    {"op":"todo.add","feature":"a","description":"s1"},
                    {"op":"doc.write","path":"imports/TODO.md","content":"# old"}
                ])),
                true,
            )
            .unwrap();
        assert_eq!(results[0]["code"], "FEAT-001");
        assert_eq!(results[0]["title"], "Preview me");
        assert_eq!(store.load("demo").unwrap().features.len(), before);
        assert!(store.read_doc("demo", "imports/TODO.md").is_err());

        // A dry run still reports the failing op.
        assert!(matches!(
            store
                .apply_batch_with(
                    "demo",
                    ops(serde_json::json!([{"op":"feature.add","title":"X","milestone":"NOPE"}])),
                    true
                )
                .unwrap_err(),
            CoreError::BatchOpFailed(0, _)
        ));
    }

    #[test]
    fn definition_round_trips_through_meta_and_assigns_requirement_ids() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Mirror", "# Mirror", "M", None)
            .unwrap();

        let def: crate::models::FeatureDefinition = serde_yaml::from_str(
            r#"
statement: Mirror items to GitHub so collaborators see the board
goals: [G-2]
zachman:
  what: A one-way push of items to issues
  how: gh, gated on a content hash
  where: kanbanr-core/mirror.rs
  when: after every successful write
  who: solo dev and repo collaborators
  why: collaborators live in issues
requirements:
  - kind: functional
    text: "WHEN content differs from the last push, THE SYSTEM SHALL update the issue."
    tests:
      - name: mirror::tests::plan_skips_unchanged
        kind: unit
        state: green
  - id: R-9
    kind: nfr
    text: "IF gh is unavailable, THE SYSTEM SHALL complete the write."
    iso25010: [Reliability]
    scenario:
      stimulus: gh is logged out
      environment: a normal write
      response: the write succeeds with a warning
      measure: "0 failed writes; local_mode::mirror_failure_does_not_fail_write"
    tests:
      - name: local_mode::mirror_failure_does_not_fail_write
        kind: integration
        state: red
"#,
        )
        .unwrap();
        let saved = store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        let saved_def = saved.definition.clone().unwrap();

        // Blank ids are filled above the highest in use; a hand-written one is kept.
        assert_eq!(saved_def.requirements[0].id, "R-10");
        assert_eq!(saved_def.requirements[1].id, "R-9");
        assert!(saved_def.zachman.missing().is_empty(), "all six answered");

        // The whole block survives the yaml round-trip THROUGH meta()/from_meta() — the split that
        // silently drops a field if it is not added in all three places.
        let reloaded = store.load("demo").unwrap();
        let reloaded = reloaded.feature(&f.code).unwrap();
        assert_eq!(reloaded.definition.as_ref(), Some(&saved_def));
        assert_eq!(
            reloaded.definition.as_ref().unwrap().requirements[1]
                .scenario
                .as_ref()
                .unwrap()
                .measure,
            "0 failed writes; local_mode::mirror_failure_does_not_fail_write"
        );
        assert_eq!(
            reloaded.definition.as_ref().unwrap().requirements[1].tests[0].state,
            crate::models::TestState::Red
        );

        // It travels with the item when the status folder changes, and clears on demand.
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        let moved = store.load("demo").unwrap();
        assert_eq!(
            moved.feature(&f.code).unwrap().definition.as_ref(),
            Some(&saved_def)
        );
        store.set_feature_definition("demo", &f.code, None).unwrap();
        assert!(
            store
                .load("demo")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .definition
                .is_none()
        );
    }

    #[test]
    fn a_feature_without_a_definition_is_untouched_on_disk() {
        // Existing boards must keep loading AND re-saving byte-identically: `skip_serializing_if`
        // is what stops an unrelated edit from adding `definition: null` to all 45 files.
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let f = store
            .add_feature("demo", "Legacy", "# Legacy", "M", None)
            .unwrap();
        let path = d
            .path
            .join("projects/demo/features/Planned")
            .join(format!("{}.yaml", f.code));
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(!before.contains("definition"), "not written when absent");

        store
            .set_feature_attrs(
                "demo",
                &f.code,
                Some("chore".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            !after.contains("definition"),
            "an unrelated edit must not introduce the key: {after}"
        );

        // And a hand-written legacy yaml (no `definition` key at all) still parses.
        let meta: crate::models::FeatureMeta = serde_yaml::from_str(&before).unwrap();
        assert!(meta.definition.is_none());
    }

    #[test]
    fn batch_feature_add_carries_a_definition() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let results = store
            .apply_batch(
                "demo",
                ops(serde_json::json!([
                    {"op":"feature.add","ref":"a","title":"Defined","milestone":"M",
                     "definition":{
                       "statement":"Do the thing",
                       "goals":["G-1"],
                       "zachman":{"what":"w","how":"h","where":"e","when":"n","who":"o","why":"y"},
                       "requirements":[{"kind":"functional","text":"THE SYSTEM SHALL do the thing.",
                         "tests":[{"name":"core::does_the_thing","state":"planned"}]}]}},
                    {"op":"feature.edit","code":"a","definition":{"statement":"Do it better"}}
                ])),
            )
            .unwrap();
        assert_eq!(results.len(), 2);
        let p = store.load("demo").unwrap();
        let f = p.features.iter().find(|f| f.title == "Defined").unwrap();
        let def = f.definition.as_ref().unwrap();
        assert_eq!(def.statement, "Do it better", "edit replaces the block");
        assert!(
            def.requirements.is_empty(),
            "replace, not merge — the block is authored whole"
        );
    }

    /// Adopt the method: the start gate only applies to projects that have a charter, and only to
    /// items created after it was adopted — so upgrading never breaks an existing board.
    fn adopt_charter(store: &Store, id: &str) {
        crate::charter::save(
            store,
            id,
            &crate::Charter {
                purpose: "Because reasons".into(),
                goals: vec![crate::Goal {
                    statement: "Ship the thing".into(),
                    ..crate::Goal::default()
                }],
                ..crate::Charter::default()
            },
        )
        .unwrap();
    }

    /// A definition that answers everything, so gate tests exercise approval and not gaps.
    fn full_definition(statement: &str) -> crate::models::FeatureDefinition {
        serde_yaml::from_str(&format!(
            r#"
statement: {statement}
goals: [G-1]
zachman:
  what: w
  how: h
  where: e
  when: n
  who: o
  why: y
requirements:
  - kind: functional
    text: "THE SYSTEM SHALL do the thing."
    tests:
      - name: core::does_the_thing
        state: planned
"#
        ))
        .unwrap()
    }

    #[test]
    fn approval_is_pinned_to_content_and_lapses_when_the_definition_changes() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        use crate::models::ApprovalState;

        store
            .set_feature_definition("demo", &f.code, Some(full_definition("Keep carts 7 days")))
            .unwrap();
        let def = |store: &Store| {
            store
                .load("demo")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .definition
                .clone()
                .unwrap()
        };
        assert_eq!(def(&store).approval_state(), ApprovalState::Missing);

        store.approve_feature("demo", &f.code, "Venkat").unwrap();
        assert_eq!(def(&store).approval_state(), ApprovalState::Current);
        assert_eq!(def(&store).approval.unwrap().by, "Venkat");

        // Evidence moving is not a scope change: a test going green must NOT lapse the approval,
        // or people learn to re-approve reflexively and the gate becomes a rubber stamp.
        let mut progressed = def(&store);
        progressed.requirements[0].tests[0].state = crate::models::TestState::Green;
        progressed.requirements[0].tests[0].checked_rev = "abc1234".into();
        store
            .set_feature_definition("demo", &f.code, Some(progressed))
            .unwrap();
        assert_eq!(def(&store).approval_state(), ApprovalState::Current);

        // Changing WHAT verifies it is a scope change, and does lapse.
        let mut retested = def(&store);
        retested.requirements[0].tests[0].name = "core::something_else".into();
        store
            .set_feature_definition("demo", &f.code, Some(retested))
            .unwrap();
        assert_eq!(def(&store).approval_state(), ApprovalState::Lapsed);
        store.approve_feature("demo", &f.code, "Venkat").unwrap();

        // Scope changes after the yes: the approval LAPSES rather than vanishing, so the record of
        // who agreed to what survives and the message can say which it is.
        store
            .set_feature_definition("demo", &f.code, Some(full_definition("Keep carts 30 days")))
            .unwrap();
        assert_eq!(def(&store).approval_state(), ApprovalState::Lapsed);
        assert_eq!(def(&store).approval.unwrap().by, "Venkat");

        store.approve_feature("demo", &f.code, "Venkat").unwrap();
        assert_eq!(def(&store).approval_state(), ApprovalState::Current);
    }

    #[test]
    fn the_start_gate_refuses_unapproved_work_but_never_blocks_closing_it() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let code = f.code.as_str();

        // No definition: starting work is refused, in words that say what to do.
        let err = store
            .move_feature_approved("demo", code, "Scheduled", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no definition"), "{err}");

        // Defined but unapproved: still refused.
        store
            .set_feature_definition("demo", code, Some(full_definition("Keep carts")))
            .unwrap();
        let err = store
            .move_feature_approved("demo", code, "Scheduled", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not approved"), "{err}");

        // Dispositioning is never gated — you can always stop work you never started.
        store
            .move_feature_approved("demo", code, "Out-of-Scope", None)
            .unwrap();
        store.move_feature("demo", code, "Planned").unwrap();

        // Approved: it starts.
        store.approve_feature("demo", code, "V").unwrap();
        let moved = store
            .move_feature_approved("demo", code, "Scheduled", None)
            .unwrap();
        assert_eq!(moved.status, "Scheduled");

        // Terminal moves stay ungated even once the approval lapses.
        store
            .set_feature_definition("demo", code, Some(full_definition("Changed scope")))
            .unwrap();
        store
            .move_feature_approved("demo", code, "Completed", None)
            .unwrap();
    }

    #[test]
    fn an_override_is_recorded_and_the_batch_path_is_gated_too() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let code = f.code.clone();
        store
            .set_feature_definition("demo", &code, Some(full_definition("Keep carts")))
            .unwrap();

        // The batch path is how work is usually started, so exempting it would make the gate
        // decorative.
        let err = store
            .apply_batch(
                "demo",
                ops(serde_json::json!([{"op":"feature.move","code":code,"to":"Scheduled"}])),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("not approved"), "{err}");

        // An override goes through, and leaves a trace on the item.
        store
            .apply_batch(
                "demo",
                ops(serde_json::json!([
                    {"op":"feature.move","code":code,"to":"Scheduled","unapproved":"prod outage"}
                ])),
            )
            .unwrap();
        let def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        assert_eq!(def.started_unapproved, "prod outage");

        // And approving through the batch path works, so one bundle can define, approve and start.
        store
            .apply_batch(
                "demo",
                ops(serde_json::json!([{"op":"feature.approve","code":code,"by":"V"}])),
            )
            .unwrap();
        let def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        assert_eq!(def.approval_state(), crate::models::ApprovalState::Current);
    }

    /// A charter-adopted project with one defined but unapproved item, and gates on Scheduled.
    fn gated_board(gate: crate::config::Gate) -> (Store, tempdir::TempDirLike, String) {
        use crate::models::FeatureDefinition;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        (store, d, f.code)
    }

    /// FEAT-116 R-1: every preset applies cleanly — it passes the same validation as a hand-written
    /// workflow — and brings its gates with it.
    #[test]
    fn presets_apply_with_their_gates() {
        use serde_json::json;
        let (store, _d) = temp_store();
        crate::dispatch::dispatch(&store, "POST", "/projects", Some(&json!({"name": "demo"})))
            .unwrap();
        // A new project gets the default preset, which is this board's own shape.
        let fresh = store.load("demo").unwrap().config;
        assert!(
            fresh.has_status("In Progress") && fresh.has_status("Ongoing"),
            "{fresh:?}"
        );
        assert_eq!(fresh.terminal_states, ["Completed"]);
        for (name, about) in crate::config::presets() {
            assert!(!about.is_empty(), "{name} says what it is");
            crate::dispatch::dispatch(
                &store,
                "PUT",
                "/projects/demo/config/workflow",
                Some(&json!({"preset": name})),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let config = store.load("demo").unwrap().config;
            let preset = crate::config::preset(name).unwrap();
            assert_eq!(config.statuses, preset.statuses, "{name}");
            assert_eq!(config.gates, preset.gates, "{name}");
            assert_eq!(
                config.schema_version,
                config.required_schema_version(),
                "{name}"
            );
        }
        // The phased presets gate their stages; TOGAF makes the branch at Implementation.
        let togaf = crate::config::preset("togaf").unwrap();
        assert!(
            togaf.gates["Implementation"]
                .on_enter
                .contains(&crate::config::Action::Branch)
        );
        assert!(togaf.transitions["Implementation"].contains(&"Operations".to_string()));
        assert_eq!(togaf.gates["Operations"].signoffs, ["release"]);
    }

    /// FEAT-116 R-2: a preset name nobody defined is refused, with the ones there are — at creation
    /// too, where it used to fall back to the default without a word.
    #[test]
    fn an_unknown_preset_is_refused() {
        use serde_json::json;
        let (store, _d) = temp_store();
        let err = crate::dispatch::dispatch(
            &store,
            "POST",
            "/projects",
            Some(&json!({"name": "demo", "workflow": "waterfal"})),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("waterfal") && err.contains("togaf") && err.contains("pdca"),
            "{err}"
        );
        assert!(store.load("demo").is_err(), "nothing was created");
    }

    /// FEAT-116 R-3: a workflow exported and loaded again is the same workflow, gates included — so
    /// an organisation's process file moves between projects intact.
    #[test]
    fn workflow_files_round_trip() {
        let original = crate::config::preset("design-control").unwrap();
        let text = serde_yaml::to_string(&original).unwrap();
        let back: crate::config::WorkflowFile = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back, original);
        let config = back.clone().into_config("x");
        assert_eq!(crate::config::WorkflowFile::from_config(&config), original);
        // The Mermaid export names each gate, and a note never carries a `;` (L-10).
        let diagram = crate::mermaid::to_state_diagram(&config);
        assert!(
            diagram
                .contains("note left of Verification: needs tests named, sign-off design-review"),
            "{diagram}"
        );
        assert!(
            !diagram
                .lines()
                .any(|l| l.contains("note") && l.contains(';'))
        );
        // And importing the diagram ignores the notes, as before.
        let parsed = crate::mermaid::parse_state_diagram(&diagram).unwrap();
        assert_eq!(
            parsed.default_state.as_deref(),
            Some(config.default_state.as_str())
        );
    }

    /// A project with sprints and points switched on, and three estimated items.
    fn sprint_board() -> (Store, tempdir::TempDirLike, Vec<String>) {
        use serde_json::json;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&json!({"sprints": true, "estimate_unit": "points"})),
        )
        .unwrap();
        let mut codes = Vec::new();
        for (title, points) in [("A", 3.0), ("B", 5.0), ("C", 8.0)] {
            let f = store.add_feature("demo", title, "", "M", None).unwrap();
            store.set_feature_points("demo", &f.code, points).unwrap();
            codes.push(f.code);
        }
        (store, d, codes)
    }

    /// FEAT-119 R-3: planning past capacity is said, not refused — the capacity is a guess.
    #[test]
    fn sprint_planning_warns_over_capacity() {
        let (store, _d, codes) = sprint_board();
        let sp = crate::sprints::add(
            &store,
            "demo",
            "2026-10-05",
            Some(14),
            "Ship cart",
            Some(10.0),
            "",
        )
        .unwrap();
        assert_eq!(
            (sp.code.as_str(), sp.end.as_str()),
            ("SP-001", "2026-10-18")
        );
        let under = crate::sprints::plan(&store, "demo", &sp.code, &codes[..2]).unwrap();
        assert_eq!(under.committed, 8.0);
        assert!(under.over_capacity.is_none());
        let over = crate::sprints::plan(&store, "demo", &sp.code, &codes[2..]).unwrap();
        assert_eq!(over.committed, 16.0);
        assert!(
            over.over_capacity
                .unwrap()
                .contains("16 points against a capacity of 10")
        );
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature(&codes[2])
                .unwrap()
                .sprint
                .as_deref(),
            Some("SP-001"),
            "it was planned all the same"
        );
    }

    /// FEAT-119 R-1: closing a sprint moves what it did not finish on, and says so on the sprint.
    #[test]
    fn closing_a_sprint_carries_unfinished_work() {
        let (store, _d, codes) = sprint_board();
        let one = crate::sprints::add(&store, "demo", "2026-10-05", None, "", None, "").unwrap();
        let two = crate::sprints::add(&store, "demo", "2026-10-19", None, "", None, "").unwrap();
        crate::sprints::plan(&store, "demo", &one.code, &codes).unwrap();
        crate::sprints::start(&store, "demo", &one.code).unwrap();
        // Only one sprint runs at a time.
        assert!(crate::sprints::start(&store, "demo", &two.code).is_err());
        store.move_feature("demo", &codes[0], "Scheduled").unwrap();
        store.move_feature("demo", &codes[0], "Completed").unwrap();

        let closed = crate::sprints::close(&store, "demo", &one.code, Some(&two.code)).unwrap();
        assert_eq!(closed.state, crate::sprints::SprintState::Closed);
        let carried: Vec<&str> = closed.carried.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(carried, [codes[1].as_str(), codes[2].as_str()]);
        assert!(closed.carried.iter().all(|c| c.to == two.code));
        let project = store.load("demo").unwrap();
        assert_eq!(
            project.feature(&codes[0]).unwrap().sprint.as_deref(),
            Some("SP-001"),
            "done stays"
        );
        assert_eq!(
            project.feature(&codes[1]).unwrap().sprint.as_deref(),
            Some("SP-002")
        );
        // Planning into a closed sprint is refused; the next one can start now.
        assert!(crate::sprints::plan(&store, "demo", &one.code, &codes[..1]).is_err());
        crate::sprints::start(&store, "demo", &two.code).unwrap();
        // Carrying to the backlog clears the sprint.
        let back = crate::sprints::close(&store, "demo", &two.code, Some("backlog")).unwrap();
        assert!(back.carried.iter().all(|c| c.to == "backlog"));
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature(&codes[1])
                .unwrap()
                .sprint,
            None
        );
    }

    /// FEAT-119 R-2: the burndown is read off the moves the items recorded — nothing is stored.
    #[test]
    fn burndown_is_derived_from_history() {
        let (store, _d, codes) = sprint_board();
        let sp = crate::sprints::add(&store, "demo", "2026-10-05", Some(5), "", None, "").unwrap();
        crate::sprints::plan(&store, "demo", &sp.code, &codes).unwrap();
        crate::sprints::start(&store, "demo", &sp.code).unwrap();
        // Record two finishes on known days, as the move history would.
        for (code, day) in [
            (&codes[0], "2026-10-06T10:00:00Z"),
            (&codes[1], "2026-10-08T16:00:00Z"),
        ] {
            store.move_feature("demo", code, "Scheduled").unwrap();
            store.move_feature("demo", code, "Completed").unwrap();
            let mut f = store.load("demo").unwrap().feature(code).unwrap().clone();
            f.history.last_mut().unwrap().at = day.to_string();
            store.persist_feature_for_test("demo", &f).unwrap();
        }
        let date = |s: &str| crate::gantt::parse_ymd(s).unwrap();
        let r = crate::sprints::report(&store, "demo", &sp.code, date("2026-10-09")).unwrap();
        assert_eq!(r.committed, 16.0);
        let remaining: Vec<f64> = r.burndown.iter().map(|d| d.remaining).collect();
        assert_eq!(
            remaining,
            [16.0, 13.0, 13.0, 8.0, 8.0],
            "day by day, from the recorded moves"
        );
        assert_eq!(r.done, 8.0);
        assert_eq!(r.days_left, 1, "the ninth is the last day");
        assert!(r.unestimated.is_empty());
    }

    /// FEAT-119 R-4: velocity appears in the report only where the project uses sprints.
    #[test]
    fn burn_rate_reports_only_where_sprints_are_on() {
        let (store, _d) = temp_store();
        new_project(&store, "plain");
        let window = crate::report::Window::default();
        let plain = crate::report::run(&store, "plain", &window, None).unwrap();
        assert!(plain.velocity.is_none());
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("velocity"), "{json}");
        // And a sprint of any kind is refused there, with the switch named.
        let err = crate::sprints::add(&store, "plain", "2026-10-05", None, "", None, "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("--sprints on"), "{err}");

        let (store, _d, codes) = sprint_board();
        let sp = crate::sprints::add(&store, "demo", "2026-10-05", None, "", None, "").unwrap();
        crate::sprints::plan(&store, "demo", &sp.code, &codes[..1]).unwrap();
        store.move_feature("demo", &codes[0], "Scheduled").unwrap();
        store.move_feature("demo", &codes[0], "Completed").unwrap();
        crate::sprints::start(&store, "demo", &sp.code).unwrap();
        crate::sprints::close(&store, "demo", &sp.code, None).unwrap();
        let with = crate::report::run(&store, "demo", &window, None).unwrap();
        assert_eq!(with.velocity, Some(vec![("SP-001".to_string(), 3.0)]));
    }

    /// A project with releases on, v0.1.0 and v0.2.0 planned, and three items in v0.1.0: one
    /// already Completed, one Scheduled with everything `finish` asks, one still Planned.
    fn release_board() -> (Store, tempdir::TempDirLike, [String; 3]) {
        use crate::models::{FeatureDefinition, Requirement, TestRef, TestState, Zachman};
        use serde_json::json;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&json!({"releases": true})),
        )
        .unwrap();
        crate::releases::add(&store, "demo", "v0.1.0", "2026-10-31", "First cut").unwrap();
        crate::releases::add(&store, "demo", "v0.2.0", "", "").unwrap();
        let ready = |title: &str| {
            let f = store.add_feature("demo", title, "", "M", None).unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(FeatureDefinition {
                        statement: format!("{title} for shoppers so that they can pay"),
                        goals: vec!["G-1".into()],
                        zachman: Zachman {
                            what: "w".into(),
                            how: "h".into(),
                            where_: "w".into(),
                            when: "w".into(),
                            who: "w".into(),
                            why: "w".into(),
                        },
                        requirements: vec![Requirement {
                            id: "R-1".into(),
                            text: format!("WHEN a shopper pays, THE SYSTEM SHALL confirm {title}."),
                            tests: vec![TestRef {
                                name: "t".into(),
                                state: TestState::Green,
                                ..Default::default()
                            }],
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )
                .unwrap();
            store.approve_feature("demo", &f.code, "Ada L").unwrap();
            f.code
        };
        let done = ready("Receipts");
        store.move_feature("demo", &done, "Scheduled").unwrap();
        store.move_feature("demo", &done, "Completed").unwrap();
        let finishing = ready("Checkout");
        store.move_feature("demo", &finishing, "Scheduled").unwrap();
        let open = store
            .add_feature("demo", "Refunds", "", "M", None)
            .unwrap()
            .code;
        crate::releases::plan(
            &store,
            "demo",
            "v0.1.0",
            &[done.clone(), finishing.clone(), open.clone()],
        )
        .unwrap();
        (store, d, [done, finishing, open])
    }

    /// FEAT-120 R-1: a cut ships the planned items that are finished — moving the ones whose work
    /// is done to their end status — and nothing else.
    #[test]
    fn a_cut_ships_only_what_is_done() {
        let (store, _d, [done, finishing, open]) = release_board();
        let cut = crate::releases::cut(&store, "demo", "v0.1.0").unwrap();
        assert_eq!(cut.release.shipped, [done.clone(), finishing.clone()]);
        assert_eq!(cut.release.state, crate::releases::ReleaseState::Shipped);
        let project = store.load("demo").unwrap();
        assert_eq!(
            project.feature(&finishing).unwrap().status,
            "Completed",
            "it was moved"
        );
        assert_eq!(
            project.feature(&open).unwrap().status,
            "Planned",
            "it was not"
        );
        // A shipped release takes no more planning, and is not cut twice.
        assert!(
            crate::releases::plan(&store, "demo", "v0.1.0", std::slice::from_ref(&open)).is_err()
        );
        assert!(crate::releases::cut(&store, "demo", "v0.1.0").is_err());
    }

    /// FEAT-120 R-2: the notes are written from what the shipped items said, onto the board.
    #[test]
    fn a_cut_writes_release_notes() {
        let (store, _d, _) = release_board();
        let cut = crate::releases::cut(&store, "demo", "v0.1.0").unwrap();
        assert_eq!(cut.release.notes_doc, "releases/v0.1.0.md");
        let doc = store.read_doc("demo", "releases/v0.1.0.md").unwrap();
        assert!(doc.starts_with("# v0.1.0 — First cut"), "{doc}");
        assert!(
            doc.contains("Checkout for shoppers so that they can pay"),
            "{doc}"
        );
        assert!(doc.contains("THE SYSTEM SHALL confirm Checkout"), "{doc}");
        assert!(doc.contains("## Carried over"), "{doc}");
    }

    /// FEAT-120 R-3: what did not make it goes to the next planned release, with the reason; and
    /// with no release left to take it, back to unplanned.
    #[test]
    fn a_cut_carries_unfinished_items() {
        let (store, _d, [_, _, open]) = release_board();
        let cut = crate::releases::cut(&store, "demo", "v0.1.0").unwrap();
        assert_eq!(cut.release.carried.len(), 1);
        let carried = &cut.release.carried[0];
        assert_eq!(
            (carried.code.as_str(), carried.to.as_str()),
            (open.as_str(), "v0.2.0")
        );
        assert!(carried.why.contains("not finished"), "{}", carried.why);
        let project = store.load("demo").unwrap();
        assert_eq!(
            project.feature(&open).unwrap().release.as_deref(),
            Some("v0.2.0")
        );
        let last = crate::releases::cut(&store, "demo", "v0.2.0").unwrap();
        assert_eq!(last.release.carried[0].to, "unplanned");
        assert_eq!(
            store.load("demo").unwrap().feature(&open).unwrap().release,
            None
        );
    }

    /// FEAT-120 R-4: releases are refused where a project has not switched them on.
    #[test]
    fn releases_only_where_switched_on() {
        let (store, _d) = temp_store();
        new_project(&store, "plain");
        let err = crate::releases::add(&store, "plain", "v1.0.0", "", "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("--releases on"), "{err}");
        let err = crate::releases::cut(&store, "plain", "v1.0.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("--releases on"), "{err}");
    }

    /// FEAT-122 R-3: the processes that work in sprints bring sprints, releases and their unit with
    /// them; the others leave a project's cadence as it was.
    #[test]
    fn presets_set_the_cadence_switches() {
        use serde_json::json;
        let (store, _d) = temp_store();
        crate::dispatch::dispatch(
            &store,
            "POST",
            "/projects",
            Some(&json!({"name": "team", "workflow": "scrum"})),
        )
        .unwrap();
        let team = store.load("team").unwrap().config;
        assert!(team.cadence.sprints && team.cadence.releases);
        assert_eq!(team.estimate_unit, crate::config::EstimateUnit::Points);
        assert_eq!(team.cadence.sprint_length_days, Some(14));

        new_project(&store, "flow");
        let apply = |preset: &str| {
            crate::dispatch::dispatch(
                &store,
                "PUT",
                "/projects/flow/config/workflow",
                Some(&json!({"preset": preset})),
            )
            .unwrap();
            store.load("flow").unwrap().config
        };
        let togaf = apply("togaf");
        assert!(togaf.cadence.is_off(), "togaf does not switch them on");
        let agile = apply("agile");
        assert!(agile.cadence.sprints && agile.cadence.releases);
        assert_eq!(
            agile.estimate_unit,
            crate::config::EstimateUnit::Days,
            "agile plans in days"
        );
        let back = apply("pdca");
        assert!(
            back.cadence.sprints,
            "another preset leaves the cadence as it was"
        );
    }

    /// FEAT-122 R-2: under Scrum, work starts only inside the sprint — In Progress asks for it.
    #[test]
    fn scrum_refuses_work_outside_the_sprint() {
        use crate::models::FeatureDefinition;
        use serde_json::json;
        let (store, _d) = temp_store();
        crate::dispatch::dispatch(
            &store,
            "POST",
            "/projects",
            Some(&json!({"name": "team", "workflow": "scrum"})),
        )
        .unwrap();
        adopt_charter(&store, "team");
        store
            .add_milestone("team", "M", "", vec![], Some("M".into()))
            .unwrap();
        let f = store
            .add_feature("team", "Checkout", "", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "team",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Checkout for shoppers so that they can pay".into(),
                    goals: vec!["G-1".into()],
                    requirements: vec![crate::models::Requirement {
                        id: "R-1".into(),
                        text: "WHEN a shopper pays, THE SYSTEM SHALL confirm the order.".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // The Definition of Ready asks for an estimate in points.
        let err = store
            .move_feature_approved("team", &f.code, "Ready", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not estimated — this project estimates in story points"),
            "{err}"
        );
        store.set_feature_points("team", &f.code, 3.0).unwrap();
        store
            .move_feature_approved("team", &f.code, "Ready", None)
            .unwrap();
        store.approve_feature("team", &f.code, "Ada L").unwrap();
        // Agreed and ready, but no sprint is running.
        let err = store
            .move_feature_approved("team", &f.code, "In Progress", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no sprint is active"), "{err}");
        let sp = crate::sprints::add(&store, "team", "2026-10-05", None, "", None, "").unwrap();
        crate::sprints::start(&store, "team", &sp.code).unwrap();
        let err = store
            .move_feature_approved("team", &f.code, "In Progress", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not in the active sprint SP-001"), "{err}");
        crate::sprints::plan(&store, "team", &sp.code, std::slice::from_ref(&f.code)).unwrap();
        store
            .move_feature_approved("team", &f.code, "In Progress", None)
            .expect("in the sprint, it starts");
        // The working agreement is generated from those same gates.
        let agreement =
            crate::config::working_agreement("team", &store.load("team").unwrap().config);
        assert!(
            agreement.contains("## Ready\n\nDefinition of Ready"),
            "{agreement}"
        );
        assert!(agreement.contains("- estimated") && agreement.contains("- in sprint"));
        assert!(agreement.contains("Sprints of 14 days") && agreement.contains("story points"));
    }

    /// FEAT-121 R-1: "estimated" means estimated in the unit the project plans in — story points
    /// for a points project, days otherwise.
    #[test]
    fn the_estimate_unit_drives_capacity_and_checks() {
        use crate::readiness::{Check, Condition};
        use serde_json::json;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_schedule("demo", &f.code, None, Some(2.0))
            .unwrap();
        let estimated = [Condition::Check(Check::Estimated)];
        let item = |store: &Store| {
            store
                .load("demo")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .clone()
        };
        let lacks = |unit| {
            let ctx = crate::readiness::Context {
                unit,
                ..Default::default()
            };
            !crate::readiness::evaluate_conditions(&item(&store), None, &estimated, &ctx).is_empty()
        };
        use crate::config::EstimateUnit::{Days, Points};
        assert!(!lacks(Days), "two days is an estimate in days");
        assert!(lacks(Points), "but not in points");
        store.set_feature_points("demo", &f.code, 5.0).unwrap();
        assert!(!lacks(Points));
        assert_eq!(item(&store).points, Some(5.0));
        // The unit is the project's, set with the cadence, and the API takes points like any attr.
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&json!({"estimate_unit": "points"})),
        )
        .unwrap();
        assert_eq!(store.load("demo").unwrap().config.estimate_unit, Points);
        crate::dispatch::dispatch(
            &store,
            "PATCH",
            &format!("/projects/demo/features/{}", f.code),
            Some(&json!({"points": 0})),
        )
        .unwrap();
        assert_eq!(item(&store).points, None, "0 clears it");
        // An item that never had points does not grow a `points:` line on disk.
        let other = store.add_feature("demo", "Other", "", "M", None).unwrap();
        let raw = std::fs::read_dir(d.path.join("projects/demo/features/Planned"))
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with(&other.code))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .unwrap();
        assert!(!raw.contains("points"), "{raw}");
    }

    /// FEAT-121 R-2: sprints and releases are off until a project switches them on, and a request
    /// for them says how.
    #[test]
    fn cadence_capabilities_are_off_unless_switched_on() {
        use crate::store::Capability;
        use serde_json::json;
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let project = store.load("demo").unwrap();
        assert!(!project.config.cadence.sprints && !project.config.cadence.releases);
        let err = Store::require_cadence(&project, Capability::Sprints)
            .unwrap_err()
            .to_string();
        assert!(err.contains("kanbanr config cadence --sprints on"), "{err}");
        assert!(Store::require_cadence(&project, Capability::Releases).is_err());
        // A project that never switched them on carries no cadence at all on disk.
        let config = std::fs::read_to_string(d.path.join("projects/demo/config.yaml")).unwrap();
        assert!(
            !config.contains("cadence") && !config.contains("estimate_unit"),
            "{config}"
        );

        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&json!({"sprints": true, "sprint_length_days": 14, "release": "per_sprint"})),
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        assert!(Store::require_cadence(&project, Capability::Sprints).is_ok());
        assert!(
            Store::require_cadence(&project, Capability::Releases).is_err(),
            "each on its own"
        );
        assert_eq!(project.config.cadence.sprint_length_days, Some(14));
        let bad = crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&json!({"release": "whenever"})),
        );
        assert!(bad.is_err());
    }

    /// FEAT-117 R-2: where the workflow declares its gates, doctor reports what the item's NEXT stage
    /// needs — not everything a later stage will ask. A definition built stage by stage is not
    /// incomplete for lacking what it is not yet expected to say.
    #[test]
    fn doctor_is_phase_aware() {
        use crate::models::FeatureDefinition;
        use serde_json::json;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/workflow",
            Some(&json!({"preset": "togaf"})),
        )
        .unwrap();
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    goals: vec!["G-1".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        let report = crate::doctor::run_project(&store, "demo").unwrap();
        let said: Vec<&str> = report
            .issues
            .iter()
            .filter(|i| i.code.as_deref() == Some(f.code.as_str()))
            .map(|i| i.message.as_str())
            .collect();
        let all = said.join(" | ");
        // Business Arch asks who, what and why, and requirements…
        for asked in [
            "[MISSING: Who]",
            "[MISSING: What]",
            "[MISSING: Why]",
            "no requirements",
        ] {
            assert!(all.contains(asked), "{asked} not reported: {all}");
        }
        // …and not how or where, which System Design asks later.
        for later in ["[MISSING: How]", "[MISSING: Where]"] {
            assert!(!all.contains(later), "{later} reported too early: {all}");
        }
        // The next-stage answer is on the readiness endpoint too, with what the stage is for.
        let out = crate::dispatch::dispatch(
            &store,
            "GET",
            &format!("/projects/demo/features/{}/readiness", f.code),
            None,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["next"][0]["status"], "Business Arch");
        assert!(
            v["next"][0]["purpose"]
                .as_str()
                .unwrap()
                .contains("Who it is for")
        );
    }

    /// FEAT-115 R-3: finishing the last task does not carry an item past its end status's gate —
    /// it stays, and says why; once the gate is met, the next task write advances it.
    #[test]
    fn auto_advance_respects_the_gate() {
        let (store, _d, code) = gated_board(crate::config::Gate::default());
        // Gate the end: Completed needs a release sign-off. (Scheduled's gate from the helper asks
        // nothing.)
        let mut gates = store.load("demo").unwrap().config.gates.clone();
        gates.insert(
            "Completed".to_string(),
            crate::config::Gate {
                signoffs: vec!["release".into()],
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .unwrap();
        let tl = store.add_todo_list("demo", &code, "work", None).unwrap();
        store
            .add_task("demo", &code, &tl.code, "do it", None)
            .unwrap();

        let (f, held) = store
            .set_task_state_reported("demo", &code, &tl.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(f.status, "Scheduled", "the gate holds it");
        let held = held.expect("and says why");
        assert!(held.contains("no sign-off 'release'"), "{held}");

        store
            .signoff_feature("demo", &code, "release", "Ada L", "", "")
            .unwrap();
        let (f, held) = store
            .set_task_state_reported("demo", &code, &tl.code, "T1", TaskState::Completed)
            .unwrap();
        assert_eq!(f.status, "Completed");
        assert!(held.is_none());
        assert_eq!(
            f.history.last().unwrap().to,
            "Completed",
            "the move is on the record"
        );
    }

    /// FEAT-114 R-1: a sign-off records who gave it, when, in which status, and the definition it
    /// covered — and a gate that asks for it passes only once it is there.
    #[test]
    fn signoffs_are_recorded_with_identity_and_revision() {
        let (store, _d, code) = gated_board(crate::config::Gate {
            signoffs: vec!["design-review".into()],
            ..Default::default()
        });
        let err = store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no sign-off 'design-review'"), "{err}");

        let f = store
            .signoff_feature("demo", &code, "design-review", "Ada L", "held 29 Sep", "")
            .unwrap();
        let def = f.definition.as_ref().unwrap();
        let s = def.signoffs.last().unwrap();
        assert_eq!(
            (s.id.as_str(), s.by.as_str(), s.status.as_str()),
            ("design-review", "Ada L", "Planned")
        );
        assert_eq!(
            s.rev,
            def.content_rev(),
            "pinned to the definition it covered"
        );
        assert!(!s.at.is_empty());
        // Recording it does not change what the approval covers: a sign-off is a verdict, not content.
        store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .expect("the gate passes once the sign-off is there");
    }

    /// FEAT-114 R-2: change the definition and the sign-off no longer covers it.
    #[test]
    fn a_signoff_lapses_when_the_definition_changes() {
        use crate::models::FeatureDefinition;
        let (store, _d, code) = gated_board(crate::config::Gate {
            signoffs: vec!["design-review".into()],
            ..Default::default()
        });
        store
            .signoff_feature("demo", &code, "design-review", "Ada L", "", "")
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 30 days".into(),
                    ..store
                        .load("demo")
                        .unwrap()
                        .feature(&code)
                        .unwrap()
                        .definition
                        .clone()
                        .unwrap()
                }),
            )
            .unwrap();
        let err = store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("sign-off 'design-review' lapsed"), "{err}");
        // The old one is kept — raw data is not discarded — and a new one covers the new text.
        store
            .signoff_feature("demo", &code, "design-review", "Ada L", "re-reviewed", "")
            .unwrap();
        let f = store.load("demo").unwrap();
        assert_eq!(
            f.feature(&code)
                .unwrap()
                .definition
                .as_ref()
                .unwrap()
                .signoffs
                .len(),
            2
        );
        store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .unwrap();
    }

    /// FEAT-114 R-3: a sign-off that cannot say who gave it is not evidence of anything.
    #[test]
    fn a_signoff_needs_an_identity() {
        use serde_json::json;
        let (store, _d, code) = gated_board(crate::config::Gate::default());
        assert!(
            store
                .signoff_feature("demo", &code, "design-review", " ", "", "")
                .is_err()
        );
        let err = crate::dispatch::dispatch(
            &store,
            "POST",
            &format!("/projects/demo/features/{code}/signoff/design-review"),
            Some(&json!({})),
        )
        .unwrap_err();
        assert!(err.to_string().contains("who"), "{err}");
        let ok = crate::dispatch::dispatch(
            &store,
            "POST",
            &format!("/projects/demo/features/{code}/signoff/release%20approval"),
            Some(&json!({"by": "Ada L", "note": "go"})),
        );
        assert!(ok.is_ok(), "{ok:?}");
        let f = store.load("demo").unwrap();
        let s = f
            .feature(&code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .signoffs[0]
            .clone();
        assert_eq!(
            s.id, "release approval",
            "the name is decoded from the path"
        );
        // Approvals now say which status they were given in, too.
        store.approve_feature("demo", &code, "Ada L").unwrap();
        let f = store.load("demo").unwrap();
        let def = f.feature(&code).unwrap().definition.clone().unwrap();
        assert_eq!(def.approvals.last().unwrap().status, "Planned");
    }

    /// FEAT-113 R-1: a blocking gate refuses the move and names every condition that fails.
    #[test]
    fn gates_block_and_list_what_is_missing() {
        use crate::readiness::{Check, Condition};
        let (store, _d, code) = gated_board(crate::config::Gate {
            requires: vec![
                Condition::Check(Check::Requirements),
                Condition::Zachman {
                    zachman: vec!["what".into(), "why".into()],
                },
            ],
            ..Default::default()
        });
        let err = store
            .move_feature_approved("demo", &code, "Scheduled", None)
            .expect_err("the gate must block")
            .to_string();
        for missing in ["no requirements", "[MISSING: What]", "[MISSING: Why]"] {
            assert!(err.contains(missing), "{missing} not named: {err}");
        }
        assert!(
            !err.contains("[MISSING: How]"),
            "only the columns asked for: {err}"
        );
        assert!(err.contains("--override"), "{err}");
        assert_eq!(
            store.load("demo").unwrap().feature(&code).unwrap().status,
            "Planned"
        );
    }

    /// FEAT-113 R-2: a warn gate lets the move through and says what it lacks.
    #[test]
    fn gates_warn_and_move() {
        use crate::readiness::{Check, Condition};
        let (store, _d, code) = gated_board(crate::config::Gate {
            requires: vec![Condition::Check(Check::Requirements)],
            warns: vec![Condition::Check(Check::Goals)],
            enforce: crate::config::Enforce::Warn,
            ..Default::default()
        });
        let (moved, warnings) = store
            .move_feature_gated("demo", &code, "Scheduled", None)
            .unwrap();
        assert_eq!(moved.status, "Scheduled");
        let said: Vec<&str> = warnings.iter().map(|g| g.label.as_str()).collect();
        assert!(
            said.contains(&"no requirements") && said.contains(&"no goal link"),
            "{said:?}"
        );
        // No override was needed, so none is recorded.
        assert_eq!(moved.history.last().unwrap().override_reason, None);
    }

    /// FEAT-113 R-3: an override passes a blocking gate, and the reason stays in the history.
    #[test]
    fn an_override_is_recorded_in_history() {
        use crate::readiness::{Check, Condition};
        let (store, _d, code) = gated_board(crate::config::Gate {
            requires: vec![Condition::Check(Check::Requirements)],
            ..Default::default()
        });
        let (moved, warnings) = store
            .move_feature_gated("demo", &code, "Scheduled", Some("customer demo tomorrow"))
            .unwrap();
        assert_eq!(moved.status, "Scheduled");
        assert_eq!(
            moved.history.last().unwrap().override_reason.as_deref(),
            Some("customer demo tomorrow")
        );
        assert!(
            warnings.iter().any(|g| g.label == "no requirements"),
            "what was passed over is said"
        );
        // It survives a reload: it is history, and raw data is not discarded.
        let reloaded = store.load("demo").unwrap();
        assert_eq!(
            reloaded
                .feature(&code)
                .unwrap()
                .history
                .last()
                .unwrap()
                .override_reason
                .as_deref(),
            Some("customer demo tomorrow")
        );
    }

    /// FEAT-113 R-4: with no gates declared, the synthesised ones make exactly the decisions the old
    /// start gate made, for every status of both built-in workflows.
    #[test]
    fn effective_gates_reproduce_the_legacy_gate() {
        use crate::config::{Action, ProjectConfig};
        // The togaf preset declares its own gates now; the synthesis is for workflows that do not.
        let mut togaf = crate::config::togaf_preset("t");
        togaf.gates.clear();
        for config in [
            ProjectConfig::default_for("d"),
            togaf,
            ProjectConfig::for_new_project("n"),
        ] {
            let gates = config.effective_gates();
            let first_active = config.displayed_states.iter().find(|s| {
                **s != config.default_state
                    && !crate::graph::is_terminal_status(&config, s)
                    && !config.is_no_op(s)
            });
            for status in &config.statuses {
                // The old rule, restated: displayed, not the default, not terminal, not a no-op.
                let old_gated = config.displayed_states.contains(status)
                    && *status != config.default_state
                    && !crate::graph::is_terminal_status(&config, status)
                    && !config.is_no_op(status);
                let gate = gates.get(status);
                assert_eq!(gate.is_some(), old_gated, "{status}");
                if let Some(gate) = gate {
                    assert_eq!(gate.enforce, crate::config::Enforce::Block);
                    assert_eq!(gate.requires.len(), 2, "definition and approval");
                    assert_eq!(
                        gate.on_enter.contains(&Action::Branch),
                        Some(status) == first_active,
                        "the branch is made where work starts: {status}"
                    );
                }
            }
        }
    }

    /// FEAT-113 R-5: a gate that could never match is refused when the workflow is saved.
    #[test]
    fn unknown_gate_names_are_rejected() {
        use serde_json::json;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let put = |gates: serde_json::Value| {
            crate::dispatch::dispatch(
                &store,
                "PUT",
                "/projects/demo/config/workflow",
                Some(&json!({"defaults": true, "gates": gates})),
            )
        };
        assert!(
            put(json!({"Nowhere": {"requires": ["statement"]}})).is_err(),
            "unknown status"
        );
        assert!(
            put(json!({"In Progress": {"requires": ["telepathy"]}})).is_err(),
            "unknown check"
        );
        let err = put(json!({"In Progress": {"requires": [{"zachman": ["what", "whence"]}]}}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("whence"), "{err}");
        // A good one is accepted, and removing a status a gate names is refused.
        assert!(
            put(json!({"In Progress": {"requires": ["statement", {"zachman": ["what"]}]}})).is_ok()
        );
        let err = store
            .set_workflow(
                "demo",
                vec!["Planned".into(), "Completed".into()],
                Default::default(),
                None,
                None,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("In Progress"), "{err}");
    }

    /// FEAT-112 R-1: one item, asked through every surface, gets one answer. The endpoint (what
    /// the monitor reads and `check` prints), doctor and `query --gap` used to carry separate copies
    /// of the rules, and they disagreed — doctor never asked for a green test, the query counted a
    /// ratified item as unapproved.
    #[test]
    fn readiness_is_one_answer_across_surfaces() {
        use crate::models::{FeatureDefinition, Requirement, TestRef, TestState};
        use serde_json::Value;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    goals: vec!["G-9".into()], // not in the charter
                    requirements: vec![
                        Requirement {
                            id: "R-1".into(),
                            text: "carts are kept".into(), // not EARS
                            tests: vec![TestRef {
                                name: "t1".into(),
                                state: TestState::Red,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        Requirement {
                            id: "R-2".into(),
                            text: "WHEN a cart is idle, THE SYSTEM SHALL keep it.".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
            )
            .unwrap();

        let out = crate::dispatch::dispatch(
            &store,
            "GET",
            &format!("/projects/demo/features/{}/readiness", f.code),
            None,
        )
        .unwrap();
        let endpoint: Value = serde_json::from_str(&out).unwrap();
        let said: Vec<String> = endpoint["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["message"].as_str().unwrap().to_string())
            .collect();
        let project = store.load("demo").unwrap();
        let item = project.feature(&f.code).unwrap();
        let engine: Vec<String> = crate::readiness::evaluate(item, None, crate::readiness::CHECK)
            .into_iter()
            .map(|g| g.message)
            .collect();
        assert_eq!(said, engine, "the endpoint is the engine's CHECK list");

        // Every rule doctor shares with check says exactly the same thing in both.
        let doctor = crate::doctor::run_project(&store, "demo").unwrap();
        let doctor_said: Vec<&str> = doctor
            .issues
            .iter()
            .filter(|i| i.code.as_deref() == Some(f.code.as_str()))
            .map(|i| i.message.as_str())
            .collect();
        for shared in ["R-1 is not in EARS form", "R-2 has no test"] {
            let from_check = said.iter().find(|m| m.starts_with(shared)).unwrap();
            assert!(
                doctor_said.contains(&from_check.as_str()),
                "doctor must say {from_check:?} word for word: {doctor_said:?}"
            );
        }
        // Doctor's own extra: the goal the charter does not have, reported as an error.
        assert!(
            doctor.issues.iter().any(
                |i| i.severity == crate::doctor::Severity::Error && i.message.contains("'G-9'")
            )
        );

        // The query's groups agree with the gaps above.
        // why: Zachman is unanswered; test: nothing is green; approval: never approved.
        for (group, expect) in [("why", true), ("test", true), ("approval", true)] {
            assert_eq!(
                !crate::readiness::evaluate(
                    item,
                    None,
                    crate::readiness::query_group(group).unwrap()
                )
                .is_empty(),
                expect
            );
            let out = crate::dispatch::dispatch(
                &store,
                "GET",
                &format!("/projects/demo/query?gap={group}"),
                None,
            )
            .unwrap();
            let hits: Value = serde_json::from_str(&out).unwrap();
            let found = hits.to_string().contains(&f.code);
            assert_eq!(found, expect, "query --gap {group}: {hits}");
        }
    }

    #[test]
    fn gap_reporting_stays_quiet_about_work_that_predates_the_method() {
        // The check that decides whether anyone ever runs `doctor` again: a board full of finished
        // items must produce nothing, or the signal drowns.
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let old = store.add_feature("demo", "Ancient", "", "M", None).unwrap();
        store.move_feature("demo", &old.code, "Scheduled").unwrap();
        let done = store
            .add_feature("demo", "Finished", "", "M", None)
            .unwrap();
        adopt_charter(&store, "demo");
        let live = store.add_feature("demo", "Live", "", "M", None).unwrap();
        store.move_feature("demo", &done.code, "Scheduled").unwrap();
        store.move_feature("demo", &done.code, "Completed").unwrap();

        let report = crate::doctor::run_project(&store, "demo").unwrap();
        let definition_issues: Vec<&crate::doctor::Issue> =
            report.issues.iter().filter(|i| i.code.is_some()).collect();
        assert_eq!(
            definition_issues.len(),
            1,
            "only the item created after adoption is in scope: {definition_issues:?}"
        );
        assert_eq!(
            definition_issues[0].code.as_deref(),
            Some(live.code.as_str())
        );
        assert!(definition_issues[0].message.contains("no definition"));

        // A goal nobody is working on is worth saying once, at project level.
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code.is_none() && i.message.contains("has no work linked to it"))
        );
    }

    #[test]
    fn requirement_checks_name_gaps_and_unsupported_claims() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Live", "", "M", None).unwrap();
        let def: crate::models::FeatureDefinition = serde_yaml::from_str(
            r#"
statement: A statement
goals: [G-1]
zachman: {what: w, how: h, where: e, when: n, who: o, why: y}
requirements:
  - kind: functional
    text: "The cart should be user friendly."
    tests: [{name: cart::friendly, state: planned}]
  - kind: functional
    text: "WHEN a cart is abandoned, THE SYSTEM SHALL retain it."
  - kind: nfr
    text: "THE SYSTEM SHALL respond quickly."
    iso25010: [Speediness]
    scenario: {stimulus: s, environment: e, response: r, measure: "under 200 ms"}
    tests: [{name: bench::latency, state: planned}]
"#,
        )
        .unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        let messages: Vec<String> = crate::doctor::run_project(&store, "demo")
            .unwrap()
            .issues
            .iter()
            .filter(|i| i.code.as_deref() == Some(f.code.as_str()))
            .map(|i| i.message.clone())
            .collect();
        let said = |needle: &str| messages.iter().any(|m| m.contains(needle));

        assert!(said("R-1 is not in EARS form"), "{messages:?}");
        assert!(said("R-2 has no test"), "{messages:?}");
        assert!(said("'Speediness', which is not an ISO"), "{messages:?}");
        // The measure says "under 200 ms" but names nothing that checks it — rigour by appearance.
        assert!(said("R-3: the measure names no test"), "{messages:?}");
        // R-1 has a test and R-3 has a tag+scenario, so those are NOT reported.
        assert!(!said("R-1 has no test"), "{messages:?}");
        assert!(
            !said("R-3 is a quality requirement with no ISO"),
            "{messages:?}"
        );
    }

    #[test]
    fn an_exempt_item_and_a_linked_goal_go_unreported() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        adopt_charter(&store, "demo");
        let f = store.add_feature("demo", "Spike", "", "M", None).unwrap();
        let def: crate::models::FeatureDefinition =
            serde_yaml::from_str("exempt: throwaway spike, deleted on Friday\nstatement: \"\"\n")
                .unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        let issues = crate::doctor::run_project(&store, "demo").unwrap();
        assert!(
            !issues
                .issues
                .iter()
                .any(|i| i.code.as_deref() == Some(f.code.as_str())),
            "a recorded exemption silences the item: {issues:?}"
        );
    }

    #[test]
    fn a_test_moves_planned_to_red_to_green_without_rewriting_the_definition() {
        use crate::models::TestState;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(full_definition("Keep carts")))
            .unwrap();
        let statement_before = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .definition
            .clone()
            .unwrap()
            .statement;

        for (state, rev) in [(TestState::Red, None), (TestState::Green, Some("abc1234"))] {
            store
                .set_test_state("demo", &f.code, "R-1", "core::does_the_thing", state, rev)
                .unwrap();
            let def = store
                .load("demo")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .definition
                .clone()
                .unwrap();
            assert_eq!(def.requirements[0].tests[0].state, state);
            // Flipping evidence must not disturb the prose — that is the whole point of a
            // targeted op rather than re-sending the block.
            assert_eq!(def.statement, statement_before);
        }
        let def = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        assert_eq!(def.requirements[0].tests[0].checked_rev, "abc1234");

        // Naming something that does not exist fails loudly rather than silently doing nothing.
        assert!(
            store
                .set_test_state(
                    "demo",
                    &f.code,
                    "R-9",
                    "core::does_the_thing",
                    TestState::Green,
                    None
                )
                .is_err()
        );
        assert!(
            store
                .set_test_state("demo", &f.code, "R-1", "core::typo", TestState::Green, None)
                .is_err()
        );
    }

    #[test]
    fn the_togaf_preset_is_a_phase_workflow_not_a_second_field() {
        let config = crate::config::togaf_preset("arch");
        assert_eq!(config.default_state, "Vision");
        assert_eq!(config.terminal_states, vec!["Operations".to_string()]);
        assert_eq!(config.displayed_states.len(), 6);
        // Forward one phase and back one phase — rework is normal — but not a leap to the end.
        assert!(config.transition_allowed("Vision", "Business Arch"));
        assert!(config.transition_allowed("System Design", "Business Arch"));
        assert!(!config.transition_allowed("Vision", "Operations"));
        // Any phase can be dispositioned, and a no-op state reopens at the start.
        assert!(config.transition_allowed("Implementation", "Out-of-Scope"));
        assert!(config.transition_allowed("Out-of-Scope", "Vision"));
    }

    #[test]
    fn feature_requires_existing_milestone() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        // Empty milestone is rejected.
        assert!(matches!(
            store.add_feature("demo", "X", "", "", None).unwrap_err(),
            CoreError::MilestoneRequired
        ));
        // Unknown milestone is rejected.
        assert!(matches!(
            store
                .add_feature("demo", "X", "", "NOPE", None)
                .unwrap_err(),
            CoreError::MilestoneNotFound(_)
        ));
    }

    #[test]
    fn feature_attrs_and_cross_feature_deps() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();

        // Set attrs on B, depending on A.
        let b2 = store
            .set_feature_attrs(
                "demo",
                &b.code,
                Some("chore".into()),
                Some("high".into()),
                Some("2026-07-01".into()),
                None,
                None,
                Some(vec!["infra".into(), "ci".into()]),
                Some(vec![a.code.clone()]),
            )
            .unwrap();
        assert_eq!(b2.kind.as_deref(), Some("chore"));
        assert_eq!(b2.priority.as_deref(), Some("high"));
        assert_eq!(b2.labels, vec!["infra".to_string(), "ci".to_string()]);
        assert_eq!(b2.depends_on, vec![a.code.clone()]);
        // Persists/reloads.
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature(&b.code)
                .unwrap()
                .kind
                .as_deref(),
            Some("chore")
        );

        // Empty string clears; unknown dep rejected; cycle rejected; self-dep rejected.
        let cleared = store
            .set_feature_attrs(
                "demo",
                &b.code,
                Some("".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(cleared.kind.is_none());
        assert!(matches!(
            store
                .set_feature_attrs(
                    "demo",
                    &a.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec!["NOPE".into()])
                )
                .unwrap_err(),
            CoreError::UnknownDependency(_)
        ));
        // A depends on B, B depends on A -> cycle.
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
        assert!(matches!(
            store
                .set_feature_attrs(
                    "demo",
                    &a.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec![b.code.clone()])
                )
                .unwrap_err(),
            CoreError::DependencyCycle(_)
        ));
        assert!(matches!(
            store
                .set_feature_attrs(
                    "demo",
                    &a.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec![a.code.clone()])
                )
                .unwrap_err(),
            CoreError::DependencyCycle(_)
        ));
    }

    #[test]
    fn cross_project_dependencies() {
        let (store, _d) = temp_store();
        new_project(&store, "alpha"); // seeds milestone "M"
        new_project(&store, "beta");
        let a = store.add_feature("alpha", "A", "", "M", None).unwrap(); // alpha:FEAT-001
        let b = store.add_feature("beta", "B", "", "M", None).unwrap(); //  beta:FEAT-001

        // A qualified cross-project dependency resolves and is stored verbatim.
        store
            .set_feature_attrs(
                "beta",
                &b.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec!["alpha:FEAT-001".into()]),
            )
            .unwrap();
        assert_eq!(
            store
                .load("beta")
                .unwrap()
                .feature(&b.code)
                .unwrap()
                .depends_on,
            vec!["alpha:FEAT-001".to_string()]
        );

        // Dangling cross-project refs are rejected (missing code, and missing project).
        assert!(matches!(
            store
                .set_feature_attrs(
                    "beta",
                    &b.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec!["alpha:FEAT-999".into()])
                )
                .unwrap_err(),
            CoreError::UnknownDependency(_)
        ));
        assert!(matches!(
            store
                .set_feature_attrs(
                    "beta",
                    &b.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec!["ghost:FEAT-001".into()])
                )
                .unwrap_err(),
            CoreError::UnknownDependency(_)
        ));

        // Cross-project cycle: beta:B already depends on alpha:A, so alpha:A -> beta:B is a cycle.
        assert!(matches!(
            store
                .set_feature_attrs(
                    "alpha",
                    &a.code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec!["beta:FEAT-001".into()])
                )
                .unwrap_err(),
            CoreError::DependencyCycle(_)
        ));
        // The rejected cyclic write did not persist.
        assert!(
            store
                .load("alpha")
                .unwrap()
                .feature(&a.code)
                .unwrap()
                .depends_on
                .is_empty()
        );
    }

    #[test]
    fn rename_status_migrates_config_and_features() {
        let (store, d) = temp_store();
        new_project(&store, "demo"); // default workflow incl. "Scheduled"
        let f = store
            .add_feature("demo", "Login", "# spec", "M", None)
            .unwrap();
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        let proj = d.path.join("projects").join("demo");
        assert!(
            proj.join("features")
                .join("Scheduled")
                .join("FEAT-001.yaml")
                .is_file()
        );

        // Rename the status the feature is in.
        store
            .rename_status("demo", "Scheduled", "In Progress")
            .unwrap();

        // Config renamed everywhere; old status gone, new present and displayed.
        let p = store.load("demo").unwrap();
        assert!(p.config.has_status("In Progress") && !p.config.has_status("Scheduled"));
        assert!(p.config.displayed_states.iter().any(|s| s == "In Progress"));
        // Feature migrated: status field + on-disk folder moved, old folder gone.
        assert_eq!(p.feature(&f.code).unwrap().status, "In Progress");
        assert!(
            proj.join("features")
                .join("In Progress")
                .join("FEAT-001.yaml")
                .is_file()
        );
        assert!(!proj.join("features").join("Scheduled").exists());

        // Renaming an unknown status errors; renaming onto an existing one is rejected.
        assert!(matches!(
            store.rename_status("demo", "Nope", "X").unwrap_err(),
            CoreError::UnknownStatus(_)
        ));
        assert!(
            store
                .rename_status("demo", "In Progress", "Completed")
                .is_err()
        );
    }

    #[test]
    fn readiness_ready_blocked_and_completed_unblocks() {
        use crate::graph::{DependencyView, Readiness};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap(); // FEAT-001
        let b = store.add_feature("demo", "B", "", "M", None).unwrap(); // FEAT-002, deps on A
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

        let view = DependencyView::build(&store, None).unwrap();
        let qa = crate::graph::qualify("demo", &a.code);
        let qb = crate::graph::qualify("demo", &b.code);
        // A has no deps -> ready. B depends on the (non-terminal) A -> blocked.
        assert_eq!(view.readiness(&qa), Some(Readiness::Ready));
        assert_eq!(view.readiness(&qb), Some(Readiness::Blocked));
        assert_eq!(view.ready(Some("demo")), vec![qa.clone()]);
        assert_eq!(view.blocked(Some("demo")), vec![qb.clone()]);

        // Complete A (Planned -> Scheduled -> Completed). Now B is unblocked, A is "done".
        store.move_feature("demo", &a.code, "Scheduled").unwrap();
        store.move_feature("demo", &a.code, "Completed").unwrap();
        let view = DependencyView::build(&store, None).unwrap();
        assert_eq!(view.readiness(&qa), Some(Readiness::Done));
        assert_eq!(view.readiness(&qb), Some(Readiness::Ready));
        assert!(view.blocked(Some("demo")).is_empty());

        // A no-op (inert) dependency also counts as terminal -> dependent is ready.
        let c = store.add_feature("demo", "C", "", "M", None).unwrap();
        let d = store.add_feature("demo", "D", "", "M", None).unwrap();
        store
            .set_feature_attrs(
                "demo",
                &d.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![c.code.clone()]),
            )
            .unwrap();
        store.move_feature("demo", &c.code, "Out-of-Scope").unwrap();
        let view = DependencyView::build(&store, None).unwrap();
        assert_eq!(
            view.readiness(&crate::graph::qualify("demo", &c.code)),
            Some(Readiness::Done)
        );
        assert_eq!(
            view.readiness(&crate::graph::qualify("demo", &d.code)),
            Some(Readiness::Ready)
        );
    }

    #[test]
    fn config_schema_version_defaults_and_stamps() {
        use crate::config::CURRENT_SCHEMA_VERSION;
        let (store, d) = temp_store();
        // A freshly-init project carries the schema its content needs (stamped on save): the base
        // layout, as it declares no gates.
        new_project(&store, "demo");
        assert_eq!(
            store.load("demo").unwrap().config.schema_version,
            crate::config::BASE_SCHEMA_VERSION
        );

        // A legacy config (no schema_version key) loads as 0.
        let cfg_path = d.path.join("projects").join("demo").join("config.yaml");
        let text = std::fs::read_to_string(&cfg_path).unwrap();
        let legacy: String = text
            .lines()
            .filter(|l| !l.starts_with("schema_version"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!legacy.contains("schema_version"));
        std::fs::write(&cfg_path, legacy).unwrap();
        assert_eq!(store.load("demo").unwrap().config.schema_version, 0);

        // Re-saving forward-stamps it back to what its content needs (non-destructive migration).
        store
            .set_project_meta("demo", Some("Demo".into()), None)
            .unwrap();
        assert_eq!(
            store.load("demo").unwrap().config.schema_version,
            crate::config::BASE_SCHEMA_VERSION
        );
        // Declaring gates stamps the version that knows them, so an older reader refuses the board
        // instead of silently ignoring its guardrails (FEAT-113).
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), crate::config::Gate::default());
        store.set_gates("demo", gates).unwrap();
        assert_eq!(
            store.load("demo").unwrap().config.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        store.set_gates("demo", Default::default()).unwrap();
        assert_eq!(
            store.load("demo").unwrap().config.schema_version,
            crate::config::BASE_SCHEMA_VERSION
        );
    }

    #[test]
    fn impact_is_transitive_downstream_closure() {
        use crate::graph::{DependencyView, qualify};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
        let c = store.add_feature("demo", "C", "", "M", None).unwrap();
        // B -> A, C -> B (chain). impact(A) = {B, C}; impact(B) = {C}; impact(C) = {}.
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
        store
            .set_feature_attrs(
                "demo",
                &c.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![b.code.clone()]),
            )
            .unwrap();

        let view = DependencyView::build(&store, None).unwrap();
        assert_eq!(
            view.impact(&qualify("demo", &a.code)),
            vec![qualify("demo", &b.code), qualify("demo", &c.code)]
        );
        assert_eq!(
            view.impact(&qualify("demo", &b.code)),
            vec![qualify("demo", &c.code)]
        );
        assert!(view.impact(&qualify("demo", &c.code)).is_empty());
    }

    #[test]
    fn cross_project_readiness() {
        use crate::graph::{DependencyView, Readiness, qualify};
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");
        let a = store.add_feature("alpha", "A", "", "M", None).unwrap(); // alpha:FEAT-001
        let b = store.add_feature("beta", "B", "", "M", None).unwrap(); //  beta:FEAT-001
        // beta:B depends on alpha:A (cross-project).
        store
            .set_feature_attrs(
                "beta",
                &b.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec!["alpha:FEAT-001".into()]),
            )
            .unwrap();

        let qa = qualify("alpha", &a.code);
        let qb = qualify("beta", &b.code);
        let view = DependencyView::build(&store, None).unwrap();
        // B is blocked by the non-terminal cross-project dep A.
        assert_eq!(view.readiness(&qb), Some(Readiness::Blocked));
        // impact(alpha:A) reaches across projects to beta:B.
        assert_eq!(view.impact(&qa), vec![qb.clone()]);
        // Portfolio-wide blocked includes beta:B; per-project beta blocked includes it too.
        assert!(view.blocked(None).contains(&qb));
        assert_eq!(view.blocked(Some("beta")), vec![qb.clone()]);

        // Complete alpha:A -> beta:B becomes ready (cross-project unblock).
        store.move_feature("alpha", &a.code, "Scheduled").unwrap();
        store.move_feature("alpha", &a.code, "Completed").unwrap();
        let view = DependencyView::build(&store, None).unwrap();
        assert_eq!(view.readiness(&qb), Some(Readiness::Ready));
    }

    #[test]
    fn dispatch_readiness_and_graph_routes() {
        use crate::dispatch::dispatch;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
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

        let ready = dispatch(&store, "GET", "/projects/demo/ready", None).unwrap();
        assert!(ready.contains("demo:FEAT-001") && !ready.contains("FEAT-002"));
        let blocked = dispatch(&store, "GET", "/projects/demo/blocked", None).unwrap();
        assert!(blocked.contains("demo:FEAT-002"));
        let impact = dispatch(
            &store,
            "GET",
            "/projects/demo/features/FEAT-001/impact",
            None,
        )
        .unwrap();
        assert!(impact.contains("demo:FEAT-002"));
        let json = dispatch(&store, "GET", "/projects/demo/graph?format=json", None).unwrap();
        assert!(json.contains("\"nodes\"") && json.contains("\"readiness\""));
        let dot = dispatch(&store, "GET", "/projects/demo/graph?format=dot", None).unwrap();
        assert!(dot.starts_with("digraph") && dot.contains("->"));
        // Portfolio-wide route works too.
        assert!(
            dispatch(&store, "GET", "/ready", None)
                .unwrap()
                .contains("demo:FEAT-001")
        );
        // Unknown project / feature -> error.
        assert!(matches!(
            dispatch(&store, "GET", "/projects/ghost/ready", None).unwrap_err(),
            CoreError::ProjectNotFound(_)
        ));
    }

    #[test]
    fn doctor_detects_integrity_problems() {
        use crate::doctor::{self, Severity};
        let (store, d) = temp_store();
        new_project(&store, "demo"); // seeds milestone "M", current schema
        let f = store.add_feature("demo", "Login", "", "M", None).unwrap();

        // A clean project reports no Errors.
        let report = doctor::run(&store).unwrap();
        assert!(
            !report.has_errors(),
            "clean project should have no errors: {report:?}"
        );

        // Unknown milestone (error): delete the feature's milestone reference by editing the yaml.
        let feat_yaml = d
            .path
            .join("projects")
            .join("demo")
            .join("features")
            .join("Planned")
            .join(format!("{}.yaml", f.code));
        let yaml = std::fs::read_to_string(&feat_yaml).unwrap();
        let yaml = yaml.replace("milestone: M", "milestone: GONE");
        std::fs::write(&feat_yaml, yaml).unwrap();

        // Outdated schema (warning): rewrite the config to schema_version 0.
        let cfg_path = d.path.join("projects").join("demo").join("config.yaml");
        let cfg = std::fs::read_to_string(&cfg_path).unwrap();
        let cfg = cfg.replace(
            &format!("schema_version: {}", crate::config::BASE_SCHEMA_VERSION),
            "schema_version: 0",
        );
        std::fs::write(&cfg_path, cfg).unwrap();

        let report = doctor::run(&store).unwrap();
        assert!(
            report.has_errors(),
            "expected an error for the unknown milestone"
        );
        assert!(
            report.issues.iter().any(|i| i.severity == Severity::Error
                && i.code.as_deref() == Some(f.code.as_str())
                && i.message.contains("unknown milestone")),
            "missing unknown-milestone error: {report:?}"
        );
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.message.contains("schema_version")),
            "missing outdated-schema warning: {report:?}"
        );

        // run_project scopes to one project but finds the same issues.
        let scoped = doctor::run_project(&store, "demo").unwrap();
        assert!(scoped.has_errors());
    }

    #[test]
    fn doctor_detects_dangling_dependency() {
        use crate::doctor::{self, Severity};
        let (store, d) = temp_store();
        new_project(&store, "demo");
        let f = store.add_feature("demo", "Login", "", "M", None).unwrap();

        // Cross-project ref validation rejects CREATING a dangling dep, so write one to disk
        // directly to simulate a ref that went stale (e.g. its target was renamed away).
        let feat_yaml = d
            .path
            .join("projects")
            .join("demo")
            .join("features")
            .join("Planned")
            .join(format!("{}.yaml", f.code));
        let yaml = std::fs::read_to_string(&feat_yaml).unwrap();
        // The empty list serializes as `depends_on: []`; swap in a dangling same-project ref.
        assert!(
            yaml.contains("depends_on: []"),
            "expected empty depends_on list: {yaml}"
        );
        let yaml = yaml.replace("depends_on: []", "depends_on:\n- FEAT-999");
        std::fs::write(&feat_yaml, yaml).unwrap();

        let report = doctor::run(&store).unwrap();
        assert!(
            report.issues.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("dangling dependency")
                && i.message.contains("FEAT-999")),
            "missing dangling-dependency error: {report:?}"
        );
    }

    #[test]
    fn dispatch_routes_data_operations() {
        use crate::dispatch::{commit_message, dispatch, is_mutation};
        let (store, _d) = temp_store();
        // Create project, milestone, feature via the shared dispatcher (as local mode does).
        dispatch(
            &store,
            "POST",
            "/projects",
            Some(&serde_json::json!({"name":"demo"})),
        )
        .unwrap();
        dispatch(
            &store,
            "POST",
            "/projects/demo/milestones",
            Some(&serde_json::json!({"name":"M","code":"MS-1"})),
        )
        .unwrap();
        let out = dispatch(
            &store,
            "POST",
            "/projects/demo/features",
            Some(&serde_json::json!({"title":"Login","milestone":"MS-1","specification":"# s"})),
        )
        .unwrap();
        assert!(out.contains("FEAT-001"));
        // Read it back.
        let proj = dispatch(&store, "GET", "/projects/demo", None).unwrap();
        assert!(proj.contains("Login"));
        // Export (raw markdown, not JSON).
        let md = dispatch(
            &store,
            "GET",
            "/projects/demo/features/FEAT-001/export?format=md",
            None,
        )
        .unwrap();
        assert!(md.contains("# s") || md.contains("Login"));
        // Helpers.
        assert!(is_mutation("POST") && !is_mutation("GET"));
        assert_eq!(
            commit_message("POST", "/projects/demo/features", None),
            "add feature item"
        );
        // Unknown route -> Unsupported.
        assert!(matches!(
            dispatch(&store, "GET", "/nope", None).unwrap_err(),
            CoreError::Unsupported(_)
        ));
    }

    #[test]
    fn feature_ownership_set_clear_and_persist() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();

        // Set assignee + team.
        let set = store
            .set_feature_attrs(
                "demo",
                &a.code,
                None,
                None,
                None,
                Some("alice".into()),
                Some("core".into()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(set.assignee.as_deref(), Some("alice"));
        assert_eq!(set.team.as_deref(), Some("core"));
        // Persists/reloads.
        let reloaded = store.load("demo").unwrap();
        let f = reloaded.feature(&a.code).unwrap();
        assert_eq!(f.assignee.as_deref(), Some("alice"));
        assert_eq!(f.team.as_deref(), Some("core"));

        // Empty string clears each.
        let cleared = store
            .set_feature_attrs(
                "demo",
                &a.code,
                None,
                None,
                None,
                Some("".into()),
                Some("".into()),
                None,
                None,
            )
            .unwrap();
        assert!(cleared.assignee.is_none());
        assert!(cleared.team.is_none());
        let reloaded = store.load("demo").unwrap();
        let f = reloaded.feature(&a.code).unwrap();
        assert!(f.assignee.is_none());
        assert!(f.team.is_none());
    }

    #[test]
    fn feature_filter_by_owner() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
        let c = store.add_feature("demo", "C", "", "M", None).unwrap();
        store
            .set_feature_attrs(
                "demo",
                &a.code,
                None,
                None,
                None,
                Some("alice".into()),
                Some("core".into()),
                None,
                None,
            )
            .unwrap();
        store
            .set_feature_attrs(
                "demo",
                &b.code,
                None,
                None,
                None,
                Some("bob".into()),
                Some("core".into()),
                None,
                None,
            )
            .unwrap();
        store
            .set_feature_attrs(
                "demo",
                &c.code,
                None,
                None,
                None,
                Some("alice".into()),
                Some("infra".into()),
                None,
                None,
            )
            .unwrap();

        let project = store.load("demo").unwrap();
        // Filter by assignee.
        let by_alice: Vec<&str> = project
            .features
            .iter()
            .filter(|f| f.assignee.as_deref() == Some("alice"))
            .map(|f| f.code.as_str())
            .collect();
        assert_eq!(by_alice, vec![a.code.as_str(), c.code.as_str()]);
        // Filter by team.
        let by_core: Vec<&str> = project
            .features
            .iter()
            .filter(|f| f.team.as_deref() == Some("core"))
            .map(|f| f.code.as_str())
            .collect();
        assert_eq!(by_core, vec![a.code.as_str(), b.code.as_str()]);
    }

    // ---- FEAT-039: workflow terminal states + Mermaid I/O --------------------------------------

    #[test]
    fn mermaid_export_contains_start_transitions_and_terminal() {
        use crate::mermaid::to_state_diagram;
        let cfg = ProjectConfig::default_for("demo");
        let out = to_state_diagram(&cfg);
        // Start edge to the default state.
        assert!(out.contains("[*] --> Planned"), "missing start edge: {out}");
        // At least one declared transition.
        assert!(
            out.contains("Planned --> Scheduled"),
            "missing transition: {out}"
        );
        // Terminal state edges to the end pseudo-state (default has terminal "Completed").
        assert!(
            out.contains("Completed --> [*]"),
            "missing terminal edge: {out}"
        );
        // No-op states are annotated.
        assert!(out.contains("note right of"), "missing no-op note: {out}");
    }

    #[test]
    fn mermaid_export_aliases_non_plain_status_names() {
        use crate::mermaid::to_state_diagram;
        let mut cfg = ProjectConfig::default_for("demo");
        cfg.statuses = vec!["To Do".into(), "In Progress".into(), "Out-of-Scope".into()];
        cfg.transitions = Default::default();
        cfg.transitions
            .insert("To Do".into(), vec!["In Progress".into()]);
        cfg.default_state = "To Do".into();
        cfg.terminal_states = vec!["In Progress".into()];
        cfg.no_op_states = vec!["Out-of-Scope".into()];
        cfg.displayed_states = vec!["To Do".into(), "In Progress".into()];
        let out = to_state_diagram(&cfg);
        // Names with spaces/'-' are aliased; the edges reference the sanitized ids.
        assert!(
            out.contains("state \"To Do\" as To_Do"),
            "missing alias: {out}"
        );
        assert!(
            out.contains("state \"In Progress\" as In_Progress"),
            "{out}"
        );
        assert!(out.contains("[*] --> To_Do"), "{out}");
        assert!(out.contains("To_Do --> In_Progress"), "{out}");
        assert!(out.contains("In_Progress --> [*]"), "{out}");
    }

    #[test]
    fn mermaid_parse_known_diagram() {
        use crate::mermaid::{WorkflowDef, parse_state_diagram};
        let text = "stateDiagram-v2\n\
            state \"To Do\" as To_Do\n\
            [*] --> To_Do\n\
            To_Do --> Done : finish\n\
            Done --> [*]\n";
        let def = parse_state_diagram(text).unwrap();
        let mut transitions = std::collections::BTreeMap::new();
        transitions.insert("To Do".to_string(), vec!["Done".to_string()]);
        assert_eq!(
            def,
            WorkflowDef {
                statuses: vec!["To Do".into(), "Done".into()],
                transitions,
                default_state: Some("To Do".into()),
                terminal_states: vec!["Done".into()],
            }
        );
    }

    #[test]
    fn mermaid_round_trip_reproduces_workflow() {
        use crate::mermaid::{parse_state_diagram, to_state_diagram};
        let cfg = ProjectConfig::default_for("demo");
        let def = parse_state_diagram(&to_state_diagram(&cfg)).unwrap();
        assert_eq!(def.default_state.as_deref(), Some("Planned"));
        assert_eq!(def.terminal_states, vec!["Completed".to_string()]);
        // Transitions survive the round-trip.
        assert_eq!(def.transitions, cfg.transitions);
        // Every original status is recovered (order-independent).
        for s in &cfg.statuses {
            assert!(
                def.statuses.contains(s),
                "lost status {s}: {:?}",
                def.statuses
            );
        }
    }

    // ---- portfolio / program hierarchy (FEAT-030) -------------------------------------------

    /// Add a todo-list with `total` tasks to `feature`, mark `done` of them Completed.
    fn seed_tasks(store: &Store, project: &str, feature: &str, total: usize, done: usize) {
        let tl = store.add_todo_list(project, feature, "work", None).unwrap();
        for i in 0..total {
            let t = store
                .add_task(project, feature, &tl.code, &format!("t{i}"), None)
                .unwrap();
            if i < done {
                store
                    .set_task_state(project, feature, &tl.code, &t.key, TaskState::Completed)
                    .unwrap();
            }
        }
    }

    #[test]
    fn mermaid_rejects_composite_state_diagram() {
        use crate::mermaid::parse_state_diagram;
        let text = "stateDiagram-v2\n\
            [*] --> Active\n\
            state Active {\n\
            [*] --> Sub\n\
            }\n";
        assert!(matches!(
            parse_state_diagram(text).unwrap_err(),
            CoreError::InvalidMermaid(_)
        ));
    }

    #[test]
    fn terminal_states_make_feature_done_in_graph() {
        use crate::graph::is_terminal_status;
        let mut cfg = ProjectConfig::default_for("demo");
        // A custom, non-"Completed", non-no-op status flagged terminal is treated as done.
        cfg.statuses.push("Shipped".into());
        cfg.terminal_states = vec!["Shipped".into()];
        assert!(is_terminal_status(&cfg, "Shipped"));
        // Empty terminal_states falls back to the legacy heuristic (Completed / no-op only).
        let mut legacy = ProjectConfig::default_for("demo");
        legacy.terminal_states.clear();
        assert!(is_terminal_status(&legacy, "Completed"));
        assert!(is_terminal_status(&legacy, "Out-of-Scope"));
        assert!(!is_terminal_status(&legacy, "Planned"));
    }

    #[test]
    fn set_workflow_validates_and_persists_terminal_states() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        // Unknown terminal status is rejected.
        assert!(matches!(
            store
                .set_workflow(
                    "demo",
                    vec!["Planned".into(), "Completed".into()],
                    Default::default(),
                    Some("Planned".into()),
                    Some(vec!["Planned".into()]),
                    None,
                    Some(vec!["Nope".into()]),
                )
                .unwrap_err(),
            CoreError::UnknownStatus(_)
        ));
        // A valid terminal status persists.
        let cfg = store
            .set_workflow(
                "demo",
                vec!["Planned".into(), "Completed".into()],
                Default::default(),
                Some("Planned".into()),
                Some(vec!["Planned".into()]),
                None,
                Some(vec!["Completed".into()]),
            )
            .unwrap();
        assert_eq!(cfg.terminal_states, vec!["Completed".to_string()]);
        assert_eq!(
            store.load("demo").unwrap().config.terminal_states,
            vec!["Completed".to_string()]
        );
    }

    #[test]
    fn dispatch_workflow_mermaid_export_route() {
        use crate::dispatch::dispatch;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let out = dispatch(
            &store,
            "GET",
            "/projects/demo/workflow?format=mermaid",
            None,
        )
        .unwrap();
        assert!(out.starts_with("stateDiagram-v2"));
        assert!(out.contains("[*] --> Planned") && out.contains("Completed --> [*]"));
        // Unknown project still 404s.
        assert!(matches!(
            dispatch(
                &store,
                "GET",
                "/projects/ghost/workflow?format=mermaid",
                None
            )
            .unwrap_err(),
            CoreError::ProjectNotFound(_)
        ));
    }

    #[test]
    fn portfolio_absent_workspace_is_default_program() {
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");
        let view = crate::portfolio::view(&store).unwrap();
        assert_eq!(view.programs.len(), 1);
        let prog = &view.programs[0];
        assert!(prog.implicit);
        assert_eq!(prog.id, crate::portfolio::DEFAULT_PROGRAM_ID);
        assert_eq!(prog.projects, vec!["alpha", "beta"]);
    }

    #[test]
    fn portfolio_rollups_compute_percentages() {
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");
        // alpha: one feature, 4 tasks, 2 done => 50%.
        let fa = store.add_feature("alpha", "A", "", "M", None).unwrap();
        seed_tasks(&store, "alpha", &fa.code, 4, 2);
        // beta: one feature, 2 tasks, 2 done => 100%.
        let fb = store.add_feature("beta", "B", "", "M", None).unwrap();
        seed_tasks(&store, "beta", &fb.code, 2, 2);

        // Two programs, one project each.
        crate::portfolio::add_program(&store, "p1", None, None, vec!["alpha".into()]).unwrap();
        crate::portfolio::add_program(&store, "p2", None, None, vec!["beta".into()]).unwrap();

        let report = crate::portfolio::rollups(&store).unwrap();
        assert_eq!(report.programs.len(), 2);
        let p1 = report.programs.iter().find(|p| p.id == "p1").unwrap();
        assert_eq!(p1.counts.percent, 50);
        assert_eq!(p1.projects[0].id, "alpha");
        assert_eq!(p1.projects[0].milestones[0].counts.percent, 50);
        let p2 = report.programs.iter().find(|p| p.id == "p2").unwrap();
        assert_eq!(p2.counts.percent, 100);
        // Percentage is mean per-feature completion: alpha A = 2/4 = 0.5, beta B = 2/2 = 1.0, so
        // the portfolio is (0.5 + 1.0)/2 = 75%. Task totals (4/6) are retained for display.
        assert_eq!(report.counts.tasks_done, 4);
        assert_eq!(report.counts.tasks_total, 6);
        assert_eq!(report.counts.percent, 75);
    }

    #[test]
    fn rollups_count_terminal_features_with_no_tasks_as_done() {
        // Regression: features completed by a STATUS move (no checklist tasks — the common case)
        // must roll up as 100%, not 0%. (A whole milestone of Completed, taskless features was
        // reading 0% because the percentage used to be task-ratio only.)
        let (store, _d) = temp_store();
        new_project(&store, "demo"); // default workflow Planned -> Scheduled -> Completed
        for title in ["A", "B"] {
            let f = store.add_feature("demo", title, "", "M", None).unwrap();
            store.move_feature("demo", &f.code, "Scheduled").unwrap();
            store.move_feature("demo", &f.code, "Completed").unwrap();
        }
        let report = crate::portfolio::rollups(&store).unwrap();
        let ms = &report.programs[0].projects[0].milestones[0];
        assert_eq!(ms.counts.tasks_total, 0, "these features track no tasks");
        assert_eq!(
            ms.counts.percent, 100,
            "terminal-by-status features must count as done"
        );
        assert_eq!(report.counts.percent, 100, "portfolio rolls up to 100%");
    }

    /// FEAT-097: the cross-project board showed `Deferred` FEAT-074 as "Not started", although its
    /// own project board hides Deferred. A view across boards must show what the boards show.
    #[test]
    fn the_cross_project_board_shows_only_what_each_board_shows() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let shown = store
            .add_feature("demo", "On the board", "", "M", None)
            .unwrap();
        let parked = store.add_feature("demo", "Parked", "", "M", None).unwrap();
        let dropped = store.add_feature("demo", "Dropped", "", "M", None).unwrap();

        let mut config = store.load("demo").unwrap().config;
        for s in ["Deferred", "Out-of-Scope"] {
            if !config.statuses.iter().any(|x| x == s) {
                config.statuses.push(s.into());
            }
        }
        config.displayed_states = vec!["Planned".into(), "Scheduled".into(), "Completed".into()];
        config.no_op_states = vec!["Out-of-Scope".into()];
        config
            .transitions
            .entry("Planned".into())
            .or_default()
            .extend(["Deferred".into(), "Out-of-Scope".into()]);
        store.save_config("demo", &config).unwrap();
        store
            .move_feature("demo", &parked.code, "Deferred")
            .unwrap();
        store
            .move_feature("demo", &dropped.code, "Out-of-Scope")
            .unwrap();

        let board = crate::portfolio::cross_project_board(&store).unwrap();
        let codes: Vec<&str> = board
            .lanes
            .iter()
            .flat_map(|l| l.cards.iter().map(|c| c.code.as_str()))
            .collect();
        assert!(
            codes.contains(&shown.code.as_str()),
            "a displayed item stays: {codes:?}"
        );
        assert!(
            !codes.contains(&parked.code.as_str()),
            "Deferred is off its own board, so it is off this one — not 'Not started': {codes:?}"
        );
        assert!(
            !codes.contains(&dropped.code.as_str()),
            "a dismissed item is not 'Done' — it was never delivered: {codes:?}"
        );
    }

    #[test]
    fn cross_project_board_groups_by_disposition() {
        use crate::portfolio::Disposition;
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");

        // alpha: not-started (no tasks) and in-progress (has tasks, not all done).
        let ns = store.add_feature("alpha", "NS", "", "M", None).unwrap();
        let ip = store.add_feature("alpha", "IP", "", "M", None).unwrap();
        seed_tasks(&store, "alpha", &ip.code, 3, 1);
        // beta: a done feature (moved to Completed).
        let dn = store.add_feature("beta", "DN", "", "M", None).unwrap();
        store.move_feature("beta", &dn.code, "Scheduled").unwrap();
        store.move_feature("beta", &dn.code, "Completed").unwrap();

        crate::portfolio::add_program(
            &store,
            "all",
            None,
            None,
            vec!["alpha".into(), "beta".into()],
        )
        .unwrap();

        let board = crate::portfolio::cross_project_board(&store).unwrap();
        let lane = |d: Disposition| {
            board
                .lanes
                .iter()
                .find(|l| l.disposition == d)
                .unwrap()
                .cards
                .clone()
        };
        let not_started = lane(Disposition::NotStarted);
        assert_eq!(not_started.len(), 1);
        assert_eq!(not_started[0].code, ns.code);
        let in_progress = lane(Disposition::InProgress);
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].code, ip.code);
        let done = lane(Disposition::Done);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].code, dn.code);

        let totals = crate::portfolio::lane_totals(&board);
        assert_eq!(totals.get("done"), Some(&1));
        assert_eq!(totals.get("in-progress"), Some(&1));
        assert_eq!(totals.get("not-started"), Some(&1));
    }

    // ---- query: rich filters + full-text + cross-project (FEAT-032) -------------------------

    #[test]
    fn query_filters_by_attributes_and_labels() {
        use crate::query::{Query, run};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "Login", "", "M", None).unwrap(); // FEAT-001
        let b = store.add_feature("demo", "Signup", "", "M", None).unwrap(); // FEAT-002
        let c = store.add_feature("demo", "Reset", "", "M", None).unwrap(); // FEAT-003
        store
            .set_feature_attrs(
                "demo",
                &a.code,
                Some("bug".into()),
                Some("high".into()),
                Some("2026-07-01".into()),
                Some("alice".into()),
                Some("core".into()),
                Some(vec!["auth".into(), "infra".into()]),
                None,
            )
            .unwrap();
        store
            .set_feature_attrs(
                "demo",
                &b.code,
                Some("feature".into()),
                None,
                Some("2026-09-01".into()),
                Some("bob".into()),
                Some("core".into()),
                Some(vec!["auth".into()]),
                None,
            )
            .unwrap();
        store
            .set_feature_attrs(
                "demo",
                &c.code,
                None,
                None,
                None,
                Some("alice".into()),
                None,
                Some(vec!["ui".into()]),
                None,
            )
            .unwrap();

        let ids = |hits: Vec<crate::query::QueryHit>| -> Vec<String> {
            hits.into_iter().map(|h| h.code).collect()
        };

        // assignee filter
        let by_alice = run(
            &store,
            &Query {
                project: Some("demo".into()),
                assignee: Some("alice".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(by_alice), vec!["FEAT-001", "FEAT-003"]);

        // team filter
        let by_core = run(
            &store,
            &Query {
                project: Some("demo".into()),
                team: Some("core".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(by_core), vec!["FEAT-001", "FEAT-002"]);

        // label any-of: "auth" matches A and B
        let by_label = run(
            &store,
            &Query {
                project: Some("demo".into()),
                labels: vec!["auth".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(by_label), vec!["FEAT-001", "FEAT-002"]);

        // kind + priority combine (AND)
        let bug_high = run(
            &store,
            &Query {
                project: Some("demo".into()),
                kind: Some("bug".into()),
                priority: Some("high".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(bug_high), vec!["FEAT-001"]);

        // due range (string compare): on/before 2026-08-01 -> only A (Sep is after; C has no due)
        let due = run(
            &store,
            &Query {
                project: Some("demo".into()),
                due_before: Some("2026-08-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(due), vec!["FEAT-001"]);
    }

    #[test]
    fn query_dep_state_and_full_text() {
        use crate::query::{MatchField, Query, run};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store
            .add_feature("demo", "Auth core", "secret handshake protocol", "M", None)
            .unwrap(); // FEAT-001
        let b = store
            .add_feature("demo", "UI", "buttons", "M", None)
            .unwrap(); // FEAT-002, deps on A
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

        // dep-state: A is ready (no deps), B is blocked (A is not terminal).
        let ready = run(
            &store,
            &Query {
                project: Some("demo".into()),
                ready: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            ready.iter().map(|h| h.code.clone()).collect::<Vec<_>>(),
            vec!["FEAT-001"]
        );
        let blocked = run(
            &store,
            &Query {
                project: Some("demo".into()),
                blocked: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            blocked.iter().map(|h| h.code.clone()).collect::<Vec<_>>(),
            vec!["FEAT-002"]
        );

        // text over title only: "auth" matches A's title.
        let title_hit = run(
            &store,
            &Query {
                project: Some("demo".into()),
                text: Some("auth".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(title_hit.len(), 1);
        assert_eq!(title_hit[0].code, "FEAT-001");
        assert_eq!(title_hit[0].matched, vec![MatchField::Title]);

        // Without --full-text, a spec-only term does NOT match.
        let no_ft = run(
            &store,
            &Query {
                project: Some("demo".into()),
                text: Some("handshake".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(no_ft.is_empty());

        // With full_text, the spec body is searched.
        let ft = run(
            &store,
            &Query {
                project: Some("demo".into()),
                text: Some("handshake".into()),
                full_text: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ft.len(), 1);
        assert_eq!(ft[0].code, "FEAT-001");
        assert_eq!(ft[0].matched, vec![MatchField::Spec]);
    }

    #[test]
    fn query_spans_projects_and_dispatch_route() {
        use crate::dispatch::dispatch;
        use crate::query::{Query, run};
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");
        store.add_feature("alpha", "A", "", "M", None).unwrap(); // alpha:FEAT-001
        let bf = store.add_feature("beta", "B login", "", "M", None).unwrap(); // beta:FEAT-001
        store
            .set_feature_attrs(
                "beta",
                &bf.code,
                None,
                None,
                None,
                Some("alice".into()),
                None,
                None,
                None,
            )
            .unwrap();

        // Cross-project (project: None) spans both, ordered by qualified id.
        let all = run(&store, &Query::default()).unwrap();
        assert_eq!(
            all.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            vec!["alpha:FEAT-001", "beta:FEAT-001"]
        );

        // Cross-project assignee filter reaches into beta only.
        let alice = run(
            &store,
            &Query {
                assignee: Some("alice".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            alice.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            vec!["beta:FEAT-001"]
        );

        // Portfolio-wide dispatch route returns JSON hits.
        let portfolio = dispatch(&store, "GET", "/query", None).unwrap();
        assert!(portfolio.contains("alpha:FEAT-001") && portfolio.contains("beta:FEAT-001"));
        // Per-project route + filter (text on title) via the query string.
        let scoped = dispatch(&store, "GET", "/projects/beta/query?text=login", None).unwrap();
        assert!(scoped.contains("beta:FEAT-001") && !scoped.contains("alpha"));
        // Unknown project 404s.
        assert!(matches!(
            dispatch(&store, "GET", "/projects/ghost/query", None).unwrap_err(),
            CoreError::ProjectNotFound(_)
        ));
    }

    // ---- scheduling: critical path + Gantt (FEAT-035) ------------------------------------------

    #[test]
    fn critical_path_and_schedule_offsets() {
        use crate::graph::{DependencyView, qualify};
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        // Chain A -> B -> C with estimates 2, 3, 1.
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
        let c = store.add_feature("demo", "C", "", "M", None).unwrap();
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
        store
            .set_feature_attrs(
                "demo",
                &c.code,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![b.code.clone()]),
            )
            .unwrap();
        store
            .set_feature_schedule("demo", &a.code, None, Some(2.0))
            .unwrap();
        store
            .set_feature_schedule("demo", &b.code, None, Some(3.0))
            .unwrap();
        store
            .set_feature_schedule("demo", &c.code, None, Some(1.0))
            .unwrap();
        // The estimate persists/reloads.
        assert_eq!(
            store
                .load("demo")
                .unwrap()
                .feature(&a.code)
                .unwrap()
                .estimate_days,
            Some(2.0)
        );

        let view = DependencyView::build(&store, None).unwrap();
        let sched = view.schedule(Some("demo"));
        let qa = qualify("demo", &a.code);
        let qb = qualify("demo", &b.code);
        let qc = qualify("demo", &c.code);
        // Offsets: A [0,2], B [2,5], C [5,6]; makespan 6; critical path A->B->C.
        assert_eq!(sched.tasks[&qa].start, 0.0);
        assert_eq!(sched.tasks[&qa].finish, 2.0);
        assert_eq!(sched.tasks[&qb].start, 2.0);
        assert_eq!(sched.tasks[&qb].finish, 5.0);
        assert_eq!(sched.tasks[&qc].start, 5.0);
        assert_eq!(sched.tasks[&qc].finish, 6.0);
        assert_eq!(sched.makespan, 6.0);
        assert_eq!(
            view.critical_path(Some("demo")),
            vec![qa.clone(), qb.clone(), qc.clone()]
        );
        assert!(sched.is_critical(&qb));
    }

    #[test]
    fn gantt_has_sections_sequencing_and_crit_marker() {
        use crate::gantt::project_gantt;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
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
        // A has an explicit start date; B is sequenced after A.
        store
            .set_feature_schedule("demo", &a.code, Some("2026-07-01".into()), Some(2.0))
            .unwrap();

        let out = project_gantt(&store, "demo").unwrap();
        assert!(out.starts_with("gantt"), "missing gantt header: {out}");
        assert!(
            out.contains("section M"),
            "missing milestone section: {out}"
        );
        // A is dated; B is sequenced with `after`.
        assert!(out.contains("2026-07-01"), "missing A date: {out}");
        assert!(
            out.contains("after"),
            "missing dependency sequencing: {out}"
        );
        // The critical path (A->B) is marked `crit`.
        assert!(out.contains("crit"), "missing crit marker: {out}");
        // Regression: task labels must NOT contain a raw qualified `proj:code` colon — Mermaid gantt
        // splits a task line on the first ':', so a colon in the label corrupts every task. Labels
        // render as `demo/FEAT-001` instead. Each task line's text before " :" must be colon-free.
        for line in out.lines().filter(|l| l.contains(" :")) {
            let title = line.split(" :").next().unwrap_or("");
            assert!(
                !title.contains(':'),
                "task label has a colon (breaks gantt): {line}"
            );
        }
        assert!(
            out.contains("demo/FEAT-001"),
            "label should be slash-qualified: {out}"
        );
    }

    #[test]
    fn gantt_dateless_root_gets_a_concrete_start() {
        use crate::gantt::project_gantt;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        // A root feature with no dependencies and no explicit start must still get a concrete
        // YYYY-MM-DD start — a bare `id, duration` makes Mermaid read the id as the date and throw
        // "Invalid date". With no planned schedule it anchors to the feature's real created_at date.
        let f = store.add_feature("demo", "Root", "", "M", None).unwrap(); // FEAT-001
        let out = project_gantt(&store, "demo").unwrap();
        let line = out
            .lines()
            .find(|l| l.contains("demo/FEAT-001"))
            .expect("root task line");
        let day = &f.created_at[..10]; // YYYY-MM-DD
        assert!(
            line.contains(day),
            "dateless root must anchor to its created_at date {day}: {line}"
        );
    }

    #[test]
    fn portfolio_gantt_spans_projects() {
        use crate::gantt::portfolio_gantt;
        let (store, _d) = temp_store();
        new_project(&store, "alpha");
        new_project(&store, "beta");
        store.add_feature("alpha", "A", "", "M", None).unwrap();
        store.add_feature("beta", "B", "", "M", None).unwrap();

        let out = portfolio_gantt(&store).unwrap();
        assert!(out.starts_with("gantt"));
        assert!(
            out.contains("section alpha") && out.contains("section beta"),
            "{out}"
        );
        assert!(
            out.contains("alpha_FEAT_001") && out.contains("beta_FEAT_001"),
            "{out}"
        );
    }

    #[test]
    fn dispatch_gantt_and_critical_path_routes() {
        use crate::dispatch::dispatch;
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let b = store.add_feature("demo", "B", "", "M", None).unwrap();
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

        let gantt = dispatch(&store, "GET", "/projects/demo/gantt", None).unwrap();
        assert!(gantt.starts_with("gantt") && gantt.contains("section"));
        let cp = dispatch(&store, "GET", "/projects/demo/critical-path", None).unwrap();
        assert!(cp.contains("critical_path") && cp.contains("demo:FEAT-001"));
        // Portfolio-wide routes.
        assert!(
            dispatch(&store, "GET", "/gantt", None)
                .unwrap()
                .starts_with("gantt")
        );
        assert!(
            dispatch(&store, "GET", "/critical-path", None)
                .unwrap()
                .contains("critical_path")
        );
        // Unknown project 404s.
        assert!(matches!(
            dispatch(&store, "GET", "/projects/ghost/gantt", None).unwrap_err(),
            CoreError::ProjectNotFound(_)
        ));
    }

    #[test]
    fn feature_schedule_set_and_clear() {
        let (store, _d) = temp_store();
        new_project(&store, "demo");
        let a = store.add_feature("demo", "A", "", "M", None).unwrap();
        let set = store
            .set_feature_schedule("demo", &a.code, Some("2026-07-01".into()), Some(3.0))
            .unwrap();
        assert_eq!(set.start.as_deref(), Some("2026-07-01"));
        assert_eq!(set.estimate_days, Some(3.0));
        // Empty start clears; estimate <= 0 clears.
        let cleared = store
            .set_feature_schedule("demo", &a.code, Some("".into()), Some(0.0))
            .unwrap();
        assert!(cleared.start.is_none());
        assert!(cleared.estimate_days.is_none());
    }
}
