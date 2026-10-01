//! Portfolio-wide integrity scan (FEAT-037).
//!
//! `doctor` walks every project (or one project) and reports structural problems that the
//! mutating ops can't always prevent after the fact — e.g. a feature whose milestone was renamed
//! out from under it, a `depends_on` ref that no longer resolves, or a config written by an older
//! schema version. It is a **read**: it never mutates the store.
//!
//! Severities: an `Error` is a broken reference that should be fixed; a `Warning` is advisory
//! (currently only an outdated `schema_version`).

use crate::config::CURRENT_SCHEMA_VERSION;
use crate::error::Result;
use crate::graph::{parse_ref, qualify};
use crate::{Project, Store};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub project: String,
    /// The feature/entity code the issue concerns, when applicable.
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}

/// Scan every project in the store.
pub fn run(store: &Store) -> Result<Report> {
    // The set of fully-qualified feature ids that exist anywhere, used to resolve cross-project
    // `depends_on` refs without re-loading per dependency.
    let mut report = Report::default();
    let ids = store.list_projects()?;

    // Build the universe of existing qualified feature ids first (one load each).
    let projects: Vec<Project> = ids.iter().filter_map(|id| store.load(id).ok()).collect();
    let existing: BTreeSet<String> = projects
        .iter()
        .flat_map(|p| p.features.iter().map(move |f| qualify(&p.id, &f.code)))
        .collect();

    for project in &projects {
        scan_project(project, &existing, &mut report);
        let charter = crate::charter::load(store, &project.id)?;
        scan_charter(&charter, &project.id, &mut report);
        let ctx = crate::readiness::Context::load(store, &project.id, &project.config);
        scan_definitions(project, &charter, &ctx, &mut report);
        scan_decisions(store, project, &mut report);
        scan_stray_folders(store, project, &mut report);
    }
    scan_identity(store, "board", &mut report);
    scan_remote_lag(store, "board", &mut report);
    Ok(report)
}

/// Scan a single project (cross-project dependency resolution still consults the whole store, so
/// a dangling ref into another project is still detected).
pub fn run_project(store: &Store, id: &str) -> Result<Report> {
    let mut report = Report::default();
    let project = store.load(id)?;
    let mut existing: BTreeSet<String> = BTreeSet::new();
    for pid in store.list_projects()? {
        if let Ok(p) = store.load(&pid) {
            for f in &p.features {
                existing.insert(qualify(&p.id, &f.code));
            }
        }
    }
    scan_project(&project, &existing, &mut report);
    let charter = crate::charter::load(store, id)?;
    scan_charter(&charter, id, &mut report);
    let ctx = crate::readiness::Context::load(store, id, &project.config);
    scan_definitions(&project, &charter, &ctx, &mut report);
    scan_decisions(store, &project, &mut report);
    scan_stray_folders(store, &project, &mut report);
    scan_identity(store, id, &mut report);
    scan_remote_lag(store, id, &mut report);
    Ok(report)
}

/// Report a board further behind its remote than its push policy allows (FEAT-142). kanbanr's own
/// public board drifted 58 commits behind GitHub with nothing saying so: the CLI's batched push
/// never fired, and when it was asked to, it could not reach GitHub at all.
fn scan_remote_lag(store: &Store, project: &str, report: &mut Report) {
    use crate::git::PushPolicy;
    let dir = store.data_dir();
    let Some(ahead) = crate::git::ahead_of_remotes(dir) else {
        return;
    };
    let (policy, _) = PushPolicy::for_board(dir);
    let allowed = match policy {
        PushPolicy::Auto => 0,
        PushPolicy::Debounce { every } => every as usize,
        // Pushing only by hand is a choice, not a fault; still say so once it is a real backlog.
        PushPolicy::Off => 10,
    };
    if ahead > allowed {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: format!(
                "this board is {ahead} commit(s) ahead of its remote (push policy: {policy}) — \
                 `kanbanr sync` pushes them; `kanbanr remote push-policy` shows why they have not \
                 gone"
            ),
        });
    }
}

/// Report a board that commits as nobody (FEAT-128): the placeholder identity older versions wrote
/// into every new data repository, or no identity at all — in which case kanbanr now refuses to
/// commit, and this says why before a write does.
fn scan_identity(store: &Store, project: &str, report: &mut Report) {
    let dir = store.data_dir();
    if !dir.join(".git").exists() {
        return;
    }
    let message = if crate::git::has_placeholder_identity(dir) {
        "this board's git config names the placeholder `kanbanr <kanbanr@local>` as its commit \
         identity, which authors commits as nobody. Set yours with `kanbanr identity --name \
         \"Your Name\" --email you@example.com`"
    } else if crate::git::identity(dir).is_none() {
        "this board has no commit identity, so writes are refused. Set one with `kanbanr identity \
         --name \"Your Name\" --email you@example.com`"
    } else {
        return;
    };
    report.issues.push(Issue {
        severity: Severity::Warning,
        project: project.to_string(),
        code: None,
        message: message.to_string(),
    });
}

/// Report what a work item has not said yet (FEAT-049).
///
/// Scope is everything here. A naive "warn when undefined" fires on every item a board has ever
/// held — 45 on kanbanr's own board, 40 of them finished — and a report that long is wallpaper
/// nobody reads. An item is in scope only when it is live work under the method: not terminal,
/// displayed on the board, and created after the charter was adopted.
fn scan_definitions(
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    report: &mut Report,
) {
    if charter.adopted_at.trim().is_empty() {
        return; // the method has not been adopted here; nothing to hold anyone to
    }
    let goal_ids = charter.goal_ids();
    let mut goals_served: BTreeSet<String> = BTreeSet::new();

    for feature in &project.features {
        // Goal coverage counts EVERY item, finished ones included: a goal delivered last month is
        // served, even though that item is out of scope for gap reporting.
        if let Some(def) = &feature.definition {
            goals_served.extend(def.goals.iter().cloned());
        }
        // An unreconciled bypass outlives the terminal gate (FEAT-080). Every other gap check
        // rightly stops caring once work is finished — you cannot fix a missing requirement on a
        // shipped item. But "built without agreement, and still never agreed to" is a question
        // that finishing does not answer; letting it age out silently meant a clean report read as
        // "nothing needs attention" when it meant "the things that did, completed first".
        if !in_scope(project, charter, feature) {
            if let Some(def) = &feature.definition
                && unreconciled_bypass(def)
                && feature.created_at.as_str() >= charter.adopted_at.as_str()
                && def.exempt.trim().is_empty()
            {
                push(
                    report,
                    project,
                    feature,
                    Severity::Warning,
                    format!(
                        "finished without agreement: {} — `kanbanr ratify {}` to agree to it \
                         after the fact, or record why it does not need one",
                        def.started_unapproved.trim(),
                        feature.code
                    ),
                );
            }
            continue;
        }

        // The same rules every surface uses (FEAT-112). Where the workflow declares its gates,
        // doctor asks only what the item's NEXT stage needs, plus that stage's warnings — a
        // definition built stage by stage is not incomplete for lacking what a later stage asks
        // (FEAT-117). Agreement and sign-offs are the review queue's questions, not doctor's.
        let gaps = if project.config.gates.is_empty() {
            crate::readiness::evaluate(feature, Some(&goal_ids), crate::readiness::DOCTOR)
        } else {
            use crate::readiness::Check;
            let mut gaps = crate::readiness::evaluate(
                feature,
                Some(&goal_ids),
                &[Check::Definition, Check::GoalsKnown, Check::Bypass],
            );
            for next in crate::readiness::next_gates(project, Some(charter), feature, ctx) {
                for gap in next.gaps.into_iter().chain(next.warnings) {
                    if !matches!(
                        gap.check,
                        Check::Approved | Check::Signoff | Check::Definition
                    ) && !gaps.contains(&gap)
                    {
                        gaps.push(gap);
                    }
                }
            }
            gaps
        };
        // One combined message for what the definition has not said: a separate issue per column
        // would bury the report.
        use crate::readiness::Check;
        let (unsaid, rest): (Vec<_>, Vec<_>) = gaps
            .into_iter()
            .partition(|g| matches!(g.check, Check::Statement | Check::Zachman | Check::Goals));
        if !unsaid.is_empty() {
            let labels: Vec<&str> = unsaid.iter().map(|g| g.label.as_str()).collect();
            push(
                report,
                project,
                feature,
                Severity::Warning,
                format!("definition gaps: {}", labels.join(" ")),
            );
        }
        for gap in rest {
            let severity = match gap.level {
                crate::readiness::Level::Error => Severity::Error,
                crate::readiness::Level::Warning => Severity::Warning,
            };
            push(report, project, feature, severity, gap.message);
        }
    }

    for goal in &charter.goals {
        if !goals_served.contains(&goal.id) {
            report.issues.push(Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: None,
                message: format!(
                    "charter goal {} has no work linked to it: \"{}\"",
                    goal.id,
                    goal.statement.trim()
                ),
            });
        }
    }
}

/// Architecture decisions in the graph (FEAT-057).
///
/// The severities follow the existing rule: a **broken reference** is an error, because someone
/// can fix it by editing a name; everything else is a warning, because it is a judgement about
/// whether the reasoning is complete, and a judgement that blocks a write gets worked around.
fn scan_decisions(store: &Store, project: &Project, report: &mut Report) {
    let adrs = crate::adr::list(store, &project.id).unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();
    let mut warn = |message: String| warnings.push(message);

    for problem in crate::adr::dangling(&adrs, project) {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: project.id.clone(),
            code: None,
            message: problem,
        });
    }

    for adr in &adrs {
        let missing = adr.missing_sections();
        // An ADR without its Decision or Consequences is a note about a meeting. Consequences is
        // the section a later reader needs most, because it says what the decision cost.
        for section in ["Decision", "Consequences"] {
            if missing.contains(&section) {
                warn(format!(
                    "{} has no {} section — the part a later reader needs before overturning it",
                    adr.id,
                    section.to_lowercase()
                ));
            }
        }
        if adr.is_accepted() && adr.affects.is_empty() && !adr.is_superseded() {
            warn(format!(
                "{} is accepted but claims to affect nothing, so nothing rests on it",
                adr.id
            ));
        }
    }

    // A quality a requirement measures, that no decision serves: the number is an aspiration.
    let mut measured: BTreeSet<String> = BTreeSet::new();
    for feature in &project.features {
        for requirement in feature
            .definition
            .iter()
            .flat_map(|d| d.requirements.iter())
            .filter(|r| matches!(r.kind, crate::models::RequirementKind::Nfr))
        {
            measured.extend(requirement.iso.iter().cloned());
        }
    }
    let served: BTreeSet<String> = adrs
        .iter()
        .filter(|a| !a.is_superseded())
        .flat_map(|a| a.quality.iter().cloned())
        .collect();
    for quality in measured.difference(&served) {
        warn(format!(
            "{quality} is measured by a requirement but no decision claims to serve it"
        ));
    }

    report
        .issues
        .extend(warnings.into_iter().map(|message| Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message,
        }));
}

fn push(
    report: &mut Report,
    project: &Project,
    feature: &crate::FeatureItem,
    severity: Severity,
    message: String,
) {
    report.issues.push(Issue {
        severity,
        project: project.id.clone(),
        code: Some(feature.code.clone()),
        message,
    });
}

/// Did this item start under a recorded bypass that nobody has answered yet?
///
/// Answered means agreed to — before the work (`approved`) or after it (`ratified`). A lapsed
/// approval is not an answer: the definition changed, so the agreement no longer covers what was
/// built, and the question is open again.
pub(crate) fn unreconciled_bypass(def: &crate::models::FeatureDefinition) -> bool {
    use crate::models::ApprovalState::{Current, Ratified};
    !def.started_unapproved.trim().is_empty() && !matches!(def.approval_state(), Current | Ratified)
}

/// Live work under the method: not finished, on the board, and created since adoption. The first two
/// are `graph::is_live_work`, shared with the review queue so the two cannot drift (FEAT-078); the
/// adoption cutoff is the doctor's own, keeping pre-method items out of the report.
fn in_scope(project: &Project, charter: &crate::Charter, feature: &crate::FeatureItem) -> bool {
    crate::graph::is_live_work(&project.config, &feature.status)
        && feature.created_at.as_str() >= charter.adopted_at.as_str()
}

/// Check a project's charter — the root of its reasoning (FEAT-046). Without a purpose there is
/// nothing to judge work against; without goals, items have nothing to link to. Both are warnings:
/// a project with no charter is incomplete, not broken.
fn scan_charter(charter: &crate::Charter, project: &str, report: &mut Report) {
    if charter.purpose.trim().is_empty() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: "no charter purpose — nothing states why this project exists \
                      (write one with `kanbanr charter set --file charter.yaml`)"
                .to_string(),
        });
    } else if charter.goals.is_empty() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message:
                "charter states a purpose but declares no goals — work items have no goal ids \
                      to link to"
                    .to_string(),
        });
    }
}

/// Run the three checks against one loaded project, given the universe of existing qualified
/// feature ids (`existing`) for cross-project dependency resolution.
/// A folder that looks like a status but is not one any more (FEAT-071). Renaming a status used to
/// leave its directory behind, and an empty orphan can sit in a board for months — this one did.
/// Reported, never removed: deleting a directory the tool does not understand is not doctor's job.
fn scan_stray_folders(store: &Store, project: &Project, report: &mut Report) {
    let known: BTreeSet<&str> = project.config.statuses.iter().map(String::as_str).collect();
    const EXPECTED: [&str; 5] = ["features", "milestones", "docs", "activity", "events"];
    let Ok(entries) = std::fs::read_dir(store.project_dir(&project.id)) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if EXPECTED.contains(&name.as_str()) || name.starts_with('.') {
            continue;
        }
        let items = std::fs::read_dir(entry.path())
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        let note = if known.contains(name.as_str()) {
            "a status folder at the project root — it belongs under features/ and will move on the              next write"
        } else if items == 0 {
            "an empty folder matching no status and no part of the layout — most likely left by a              status rename"
        } else {
            "a folder matching no status and no part of the layout, and it is not empty"
        };
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message: format!("{name}/ is {note}"),
        });
    }
}

fn scan_project(project: &Project, existing: &BTreeSet<String>, report: &mut Report) {
    let pid = &project.id;

    // A board newer than this binary cannot be read correctly at all — but `run` only gets here
    // when the load succeeded, so this is the belt to the load guard's braces: it catches a project
    // whose config was stamped forward while the process was running (FEAT-072).
    if project.config.schema_version > CURRENT_SCHEMA_VERSION {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: pid.clone(),
            code: None,
            message: format!(
                "config schema_version {} is NEWER than this build understands ({}) — upgrade the \
                 binary and restart anything long-running, such as `kanbanr serve`",
                project.config.schema_version, CURRENT_SCHEMA_VERSION
            ),
        });
    }

    // Outdated schema_version (warning).
    if project.config.schema_version < project.config.required_schema_version() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: pid.clone(),
            code: None,
            message: format!(
                "config schema_version {} is older than current {} (re-save to migrate)",
                project.config.schema_version,
                project.config.required_schema_version()
            ),
        });
    }

    let milestones: BTreeSet<&str> = project.milestones.iter().map(|m| m.code.as_str()).collect();

    for f in &project.features {
        // Unknown milestone (error).
        if !milestones.contains(f.milestone.as_str()) {
            report.issues.push(Issue {
                severity: Severity::Error,
                project: pid.clone(),
                code: Some(f.code.clone()),
                message: format!("references unknown milestone '{}'", f.milestone),
            });
        }

        // Dangling dependency (error): each `depends_on` ref must resolve to an existing feature.
        for dep in &f.depends_on {
            let (dp, dc) = parse_ref(dep, pid);
            if !existing.contains(&qualify(&dp, &dc)) {
                report.issues.push(Issue {
                    severity: Severity::Error,
                    project: pid.clone(),
                    code: Some(f.code.clone()),
                    message: format!(
                        "dangling dependency '{dep}' (resolves to {dp}:{dc}, which does not exist)"
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef, Zachman};

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-doctor-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Nothing is built on reasoning nobody agreed to".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        (store, dir)
    }

    /// FEAT-080: every other gap check rightly stops caring once work is finished — you cannot fix
    /// a missing requirement on a shipped item. "Built without agreement, and still never agreed
    /// to" is different: finishing does not answer it. Six items on this project's own board had
    /// aged out of every report, and producing the list took a script.
    #[test]
    fn an_unreconciled_bypass_outlives_the_terminal_gate() {
        let (store, _dir) = fixture();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    goals: vec!["G-1".into()],
                    started_unapproved: "shipped under a deadline".into(),
                    ..Default::default()
                }),
            )
            .unwrap();

        let bypass = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .map(|i| i.message)
                .filter(|m| m.contains("without approval") || m.contains("without agreement"))
                .collect()
        };

        store.move_feature("demo", &item.code, "Scheduled").unwrap();
        assert_eq!(
            bypass(&store).len(),
            1,
            "in flight and unanswered: it warns"
        );

        // Finishing it used to be how the question stopped being asked.
        store.move_feature("demo", &item.code, "Completed").unwrap();
        let after = bypass(&store);
        assert_eq!(
            after.len(),
            1,
            "a finished item with an unanswered bypass must STILL be reported: {after:?}"
        );
        assert!(
            after[0].contains("finished without agreement"),
            "and it should read as the question it is: {}",
            after[0]
        );
        assert!(
            after[0].contains("kanbanr ratify"),
            "a warning that names no remedy is wallpaper: {}",
            after[0]
        );

        // Ratifying answers it, and the report goes quiet — for the right reason this time.
        store
            .ratify_feature("demo", &item.code, "the user", "reviewed as a set")
            .unwrap();
        assert!(
            bypass(&store).is_empty(),
            "answered after the fact is still answered"
        );
    }

    /// FEAT-068: the warning asked the reader to "review and approve what was actually built". The
    /// user did, for twelve items, and it kept asking — which is how a warning becomes wallpaper.
    #[test]
    fn an_answered_escape_stops_warning() {
        let (store, dir) = fixture();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let definition = FeatureDefinition {
            statement: "Keep a cart for 7 days".into(),
            goals: vec!["G-1".into()],
            zachman: Zachman {
                what: "cart persistence".into(),
                how: "server-side".into(),
                where_: "checkout".into(),
                when: "on mutation".into(),
                who: "returning shoppers".into(),
                why: "carts vanish".into(),
            },
            requirements: vec![Requirement {
                id: "R-1".into(),
                text: "THE SYSTEM SHALL retain the cart.".into(),
                tests: vec![TestRef {
                    name: "cart::retains".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            started_unapproved: "shipped under a deadline".into(),
            ..Default::default()
        };
        store
            .set_feature_definition("demo", &item.code, Some(definition.clone()))
            .unwrap();
        store.move_feature("demo", &item.code, "Scheduled").unwrap();

        let unapproved = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("started without approval"))
                .map(|i| i.message)
                .collect()
        };
        assert_eq!(unapproved(&store).len(), 1, "unanswered, so it warns");

        // Approving is the answer the warning asked for.
        store
            .approve_feature("demo", &item.code, "the user")
            .unwrap();
        assert!(
            unapproved(&store).is_empty(),
            "answered, so it stops: a warning that cannot be resolved is wallpaper"
        );

        // The record is kept — work did start before agreement, and that is history.
        let after = store.load("demo").unwrap();
        let after = after.feature(&item.code).unwrap();
        assert_eq!(
            after.definition.as_ref().unwrap().started_unapproved,
            "shipped under a deadline"
        );

        // Changing the definition lapses the approval, and the warning returns with it: the
        // agreement no longer covers what was built.
        let widened = FeatureDefinition {
            statement: "Keep a cart for 30 days".into(),
            ..definition
        };
        store
            .set_feature_definition("demo", &item.code, Some(widened))
            .unwrap();
        assert_eq!(
            unapproved(&store).len(),
            1,
            "scope changed after the yes, so the escape is unanswered again"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FEAT-128: a board whose git config still carries the placeholder identity commits as
    /// nobody; doctor says so, with the command that fixes it, and stops once it is fixed.
    #[test]
    fn a_placeholder_identity_is_reported() {
        let (store, dir) = fixture();
        crate::git::ensure_repo_as(&dir, Some(("kanbanr", crate::git::PLACEHOLDER_EMAIL)));
        let identity_issues = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("commit identity"))
                .map(|i| i.message)
                .collect()
        };
        let found = identity_issues(&store);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("placeholder") && found[0].contains("kanbanr identity"));

        crate::git::set_identity(&dir, "Ada", "ada@example.com").unwrap();
        assert!(identity_issues(&store).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-142 R-6: a board further ahead of its remote than its policy allows is reported, with
    /// the command that pushes it; one within it is not.
    #[test]
    fn a_board_far_ahead_of_its_remote_is_reported() {
        let (store, dir) = fixture();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let remote =
            std::env::temp_dir().join(format!("kanbanr-doctor-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remote);
        git2::Repository::init_bare(&remote).unwrap();
        crate::git::add_remote(&dir, "origin", remote.to_str().unwrap()).unwrap();
        crate::git::set_push_setting(&dir, crate::git::PushPolicy::Debounce { every: 2 }).unwrap();
        let lag = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("ahead of its remote"))
                .map(|i| i.message)
                .collect()
        };
        for n in 0..3 {
            std::fs::write(dir.join(format!("note-{n}.txt")), "x").unwrap();
            assert!(crate::git::commit_local(&dir, "a change"));
        }
        // Never pushed: every commit is ahead — more than two.
        let found = lag(&store);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("kanbanr sync"), "{}", found[0]);
        let pushed = crate::git::push_pending(&dir);
        assert!(pushed.failures.is_empty(), "{pushed:?}");
        assert!(lag(&store).is_empty(), "within the policy once pushed");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&remote);
    }
}
