//! Moving an item through its workflow the way `start` and `finish` do (FEAT-176).
//!
//! Where work starts, which end an item finishes at, and whether the workflow lets it get there
//! are the workflow's to say, not a front end's: the CLI collects what it can see of the code
//! repository, asks for a plan here, and then acts on it — branches, moves, prints. A plan is
//! pure: nothing here touches the board or the repository.

use crate::config::ProjectConfig;
use crate::error::{CoreError, Result};
use crate::scm::RepoFacts;
use crate::store::Project;

/// What `start` is to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartPlan {
    /// The status the item moves to.
    pub status: String,
    /// The item is already there: nothing to move.
    pub already: bool,
    /// The branch its work goes on, where there is a repository to branch in.
    pub branch: Option<String>,
}

/// Plan `start` for `code` (FEAT-115): to `to`, or to the status whose gate makes the branch, or to
/// the first active status. `repo` is the code repository's state, `None` where there is none to
/// branch in — a project need not be code, and there the item simply moves.
pub fn plan_start(
    project: &Project,
    code: &str,
    to: Option<&str>,
    repo: Option<&RepoFacts>,
) -> Result<StartPlan> {
    let feature = project.feature(code)?;
    let config = &project.config;
    // Where work starts is the workflow's to say: the status whose gate makes the branch
    // (FEAT-115). Under TOGAF that is Implementation, not the first phase after Vision.
    let status = match to {
        Some(s) => s.to_string(),
        None => config
            .branch_status()
            .or_else(|| config.first_active_status())
            .ok_or_else(|| {
                CoreError::Refused(
                    "this workflow has no active status to start in; pass --to <STATUS>".into(),
                )
            })?,
    };
    require_reachable(config, code, &feature.status, &status)?;
    let branch = match repo {
        None => None,
        // An item branch is made from the default branch, and a repository with no commits has
        // nothing to make it from (FEAT-105): branching anyway leaves an orphan that never joins.
        Some(repo) if !repo.has_commits => {
            return Err(CoreError::Refused(format!(
                "this repository has no commits yet, so there is nothing to branch {code} from. \
                 Make the initial commit on {} first, e.g.\n\n    \
                 git commit -m \"[no-ref] initial commit\"\n",
                repo.default_branch
            )));
        }
        Some(_) => {
            // A defect goes on the pattern's `fix/` twin when the pattern is the default
            // (FEAT-189, Q3).
            let pattern = config.branch_pattern();
            let pattern = match crate::scm::fix_twin(pattern) {
                Some(twin)
                    if matches!(feature.kind.as_deref(), Some("defect" | "bug"))
                        && pattern == crate::scm::DEFAULT_BRANCH_PATTERN =>
                {
                    twin
                }
                _ => pattern.to_string(),
            };
            Some(crate::scm::branch_for(&pattern, code, &feature.title))
        }
    };
    Ok(StartPlan {
        already: feature.status == status,
        status,
        branch,
    })
}

/// What `finish` is to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishPlan {
    /// The item being finished.
    pub code: String,
    /// The end status it moves to.
    pub terminal: String,
}

/// Plan `finish` (FEAT-115): the item `code` names, or else the one the current `branch` names;
/// the end status it can reach; and everything that makes "done" mean something, checked before
/// it is claimed. A workflow that declares its gates says that itself, on the end status, and the
/// move enforces it; one that does not keeps the rules `check` reports. Open tasks are never done.
pub fn plan_finish(
    project: &Project,
    code: Option<&str>,
    branch: Option<&str>,
) -> Result<FinishPlan> {
    let config = &project.config;
    let code = match code {
        Some(c) => c.to_string(),
        None => {
            let branch = branch.ok_or_else(|| {
                CoreError::Refused("no branch to infer the item from; name it".into())
            })?;
            if branch.starts_with(crate::scm::SPIKE_PREFIX) {
                return Err(CoreError::Refused(format!(
                    "'{branch}' is a spike. A spike's output is a change to an item's definition, \
                     not merged code — write that up, then start the item it belongs to."
                )));
            }
            crate::scm::code_from_branch(config.branch_pattern(), branch).ok_or_else(|| {
                CoreError::Refused(format!(
                    "the branch '{branch}' names no item; pass the code"
                ))
            })?
        }
    };
    let feature = project.feature(&code)?;
    let terminal = config.finish_target(&feature.status);
    require_reachable(config, &code, &feature.status, &terminal)?;

    let mut lines: Vec<String> = if config.declares_gates() {
        Vec::new()
    } else {
        // The engine every surface shares (FEAT-112); `check` and `finish` ask its CHECK list.
        crate::readiness::evaluate(feature, None, crate::readiness::CHECK)
            .into_iter()
            .map(|g| g.message)
            .collect()
    };
    lines.extend(feature.todo_lists.iter().flat_map(|l| {
        l.tasks
            .iter()
            .filter(|t| t.state != crate::models::TaskState::Completed)
            .map(move |t| format!("task not finished: {}/{} {}", l.code, t.key, t.text))
    }));
    if !lines.is_empty() {
        return Err(CoreError::Refused(format!(
            "{code} is not finished:\n  {}\n\nFix these, or move it by hand if you disagree.",
            lines.join("\n  ")
        )));
    }
    Ok(FinishPlan { code, terminal })
}

/// Refuse a jump the workflow does not allow, naming the stages in between — each has its own
/// gate, so they are walked, not skipped (FEAT-115).
pub fn require_reachable(config: &ProjectConfig, code: &str, from: &str, to: &str) -> Result<()> {
    if from == to || config.transition_allowed(from, to) {
        return Ok(());
    }
    Err(CoreError::Refused(match config.path_between(from, to) {
        Some(path) if path.len() > 2 => format!(
            "{code} is at {from}, and {to} is reached through {}. Move it through those first — \
             each stage has its own gate — with `kanbanr move {code} <STATUS>`.",
            path[1..path.len() - 1].join(" → ")
        ),
        _ => format!("the workflow has no way from {from} to {to}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `scheduled` workflow: Planned → Scheduled → Completed, one item at `status`.
    fn board(status: &str, kind: Option<&str>) -> Project {
        let item: crate::FeatureItem = serde_json::from_value(serde_json::json!({
            "code": "FEAT-001", "title": "Cart keeps items", "status": status,
            "milestone": "MS-001", "kind": kind,
            "created_at": "2026-10-21T00:00:00Z", "updated_at": "2026-10-21T00:00:00Z"
        }))
        .unwrap();
        Project {
            id: "p".into(),
            config: ProjectConfig::default_for("p"),
            features: vec![item],
            milestones: Vec::new(),
        }
    }

    fn repo(has_commits: bool) -> RepoFacts {
        RepoFacts {
            branch: Some("main".into()),
            default_branch: "main".into(),
            has_commits,
            ..Default::default()
        }
    }

    fn refused<T: std::fmt::Debug>(r: Result<T>) -> String {
        match r {
            Err(CoreError::Refused(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_workflow_says_where_a_path_goes() {
        let config = ProjectConfig::default_for("p");
        assert_eq!(config.first_active_status().as_deref(), Some("Scheduled"));
        assert_eq!(
            config.path_between("Planned", "Completed"),
            Some(vec![
                "Planned".to_string(),
                "Scheduled".into(),
                "Completed".into()
            ])
        );
        assert_eq!(config.path_between("Completed", "Nowhere"), None);
        assert_eq!(
            refused(require_reachable(
                &config,
                "FEAT-001",
                "Completed",
                "Nowhere"
            )),
            "the workflow has no way from Completed to Nowhere"
        );
        // The end reachable from here, else the first end, which is still walked to.
        assert_eq!(config.finish_target("Scheduled"), "Completed");
        assert_eq!(config.finish_target("Planned"), "Completed");
        assert!(
            refused(require_reachable(
                &config,
                "FEAT-001",
                "Planned",
                "Completed"
            ))
            .contains("reached through Scheduled")
        );
        assert!(require_reachable(&config, "FEAT-001", "Planned", "Scheduled").is_ok());
    }

    /// FEAT-217 R-8: a defect coded by its kind starts on its `fix/` branch, named by that code.
    #[test]
    fn a_kind_coded_defect_starts_on_its_fix_branch() {
        let mut p = board("Planned", Some("defect"));
        p.features[0].code = "DEF-001".into();
        p.config.codes.kinds.insert("defect".into(), "DEF".into());
        let plan = plan_start(&p, "DEF-001", None, Some(&repo(true))).unwrap();
        assert_eq!(plan.branch.as_deref(), Some("fix/DEF-001-cart-keeps-items"));
        assert_eq!(
            crate::scm::branch_item(&p, plan.branch.as_deref()).as_deref(),
            Some("DEF-001")
        );
    }

    #[test]
    fn start_plans_the_status_and_the_branch() {
        let plan =
            plan_start(&board("Planned", None), "FEAT-001", None, Some(&repo(true))).unwrap();
        assert_eq!(plan.status, "Scheduled");
        assert!(!plan.already);
        assert_eq!(
            plan.branch.as_deref(),
            Some("feat/FEAT-001-cart-keeps-items")
        );
        // A defect goes on the fix/ twin (FEAT-189); no repository, no branch.
        let defect = board("Planned", Some("defect"));
        let plan = plan_start(&defect, "FEAT-001", None, Some(&repo(true))).unwrap();
        assert_eq!(
            plan.branch.as_deref(),
            Some("fix/FEAT-001-cart-keeps-items")
        );
        assert_eq!(
            plan_start(&defect, "FEAT-001", None, None).unwrap().branch,
            None
        );
        // FEAT-105: nothing to branch from in a repository with no commits.
        assert!(
            refused(plan_start(&defect, "FEAT-001", None, Some(&repo(false))))
                .contains("no commits yet")
        );
        assert!(
            plan_start(&board("Scheduled", None), "FEAT-001", None, None)
                .unwrap()
                .already
        );
        assert!(matches!(
            plan_start(&defect, "FEAT-404", None, None),
            Err(CoreError::FeatureNotFound(_))
        ));
    }

    #[test]
    fn finish_names_the_item_and_refuses_what_is_not_done() {
        let at_planned = board("Planned", None);
        assert!(refused(plan_finish(&at_planned, None, None)).contains("no branch"));
        assert!(refused(plan_finish(&at_planned, None, Some("spike/try"))).contains("is a spike"));
        assert!(refused(plan_finish(&at_planned, None, Some("main"))).contains("names no item"));
        assert!(
            refused(plan_finish(&at_planned, None, Some("feat/FEAT-001-x")))
                .contains("reached through Scheduled")
        );
        // From the stage before the end, what `check` reports stands in the way.
        assert!(
            refused(plan_finish(
                &board("Scheduled", None),
                Some("FEAT-001"),
                None
            ))
            .contains("FEAT-001 is not finished")
        );
    }
}
