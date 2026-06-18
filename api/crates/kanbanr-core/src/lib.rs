//! kanbanr-core: domain models, YAML store, validation, and export.
//! Shared by the CLI (the only writer) and the read-only server.

pub mod activity;
pub mod batch;
pub mod config;
pub mod dispatch;
pub mod docs;
pub mod error;
pub mod export;
pub mod git;
pub mod graph;
pub mod models;
pub mod project;
pub mod store;
pub mod validate;

pub use config::ProjectConfig;
pub use error::{CoreError, Result};
pub use models::{FeatureItem, Milestone, Status, Task, TaskState, TodoList};
pub use store::{Project, Store};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

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
        let planned_yaml = proj.join("Planned").join("FEAT-001.yaml");
        let planned_spec = proj
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
        assert!(proj.join("Scheduled").join("FEAT-001.yaml").is_file());
        assert!(proj
            .join("Scheduled")
            .join("features-spec")
            .join("FEAT-001.md")
            .is_file());

        // The yaml metadata must NOT contain the specification (it lives in the .md).
        let yaml = std::fs::read_to_string(proj.join("Scheduled").join("FEAT-001.yaml")).unwrap();
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
            .set_feature_attrs("demo", &b.code, Some("".into()), None, None, None, None)
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
                    Some(vec!["beta:FEAT-001".into()])
                )
                .unwrap_err(),
            CoreError::DependencyCycle(_)
        ));
        // The rejected cyclic write did not persist.
        assert!(store
            .load("alpha")
            .unwrap()
            .feature(&a.code)
            .unwrap()
            .depends_on
            .is_empty());
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
        assert!(proj.join("Scheduled").join("FEAT-001.yaml").is_file());

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
        assert!(proj.join("In Progress").join("FEAT-001.yaml").is_file());
        assert!(!proj.join("Scheduled").exists());

        // Renaming an unknown status errors; renaming onto an existing one is rejected.
        assert!(matches!(
            store.rename_status("demo", "Nope", "X").unwrap_err(),
            CoreError::UnknownStatus(_)
        ));
        assert!(store
            .rename_status("demo", "In Progress", "Completed")
            .is_err());
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
    fn impact_is_transitive_downstream_closure() {
        use crate::graph::{qualify, DependencyView};
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
        use crate::graph::{qualify, DependencyView, Readiness};
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
        assert!(dispatch(&store, "GET", "/ready", None)
            .unwrap()
            .contains("demo:FEAT-001"));
        // Unknown project / feature -> error.
        assert!(matches!(
            dispatch(&store, "GET", "/projects/ghost/ready", None).unwrap_err(),
            CoreError::ProjectNotFound(_)
        ));
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
}
