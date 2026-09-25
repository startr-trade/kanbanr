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
        scan_definitions(project, &charter, &mut report);
        scan_decisions(store, project, &mut report);
    }
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
    scan_definitions(&project, &charter, &mut report);
    scan_decisions(store, &project, &mut report);
    Ok(report)
}

/// Report what a work item has not said yet (FEAT-049).
///
/// Scope is everything here. A naive "warn when undefined" fires on every item a board has ever
/// held — 45 on kanbanr's own board, 40 of them finished — and a report that long is wallpaper
/// nobody reads. An item is in scope only when it is live work under the method: not terminal,
/// displayed on the board, and created after the charter was adopted.
fn scan_definitions(project: &Project, charter: &crate::Charter, report: &mut Report) {
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
        if !in_scope(project, charter, feature) {
            continue;
        }

        let Some(def) = &feature.definition else {
            push(report, project, feature, Severity::Warning,
                "has no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`)".to_string());
            continue;
        };
        if !def.exempt.trim().is_empty() {
            continue; // exempt for a recorded reason; the escape hatch stays visible
        }

        // One combined message per item: a separate issue per column would bury the report.
        let mut gaps: Vec<String> = def
            .zachman
            .missing()
            .iter()
            .map(|c| format!("[MISSING: {c}]"))
            .collect();
        if def.statement.trim().is_empty() {
            gaps.insert(0, "[MISSING: statement]".to_string());
        }
        if def.goals.is_empty() {
            gaps.push("no goal link".to_string());
        }
        if !gaps.is_empty() {
            push(
                report,
                project,
                feature,
                Severity::Warning,
                format!("definition gaps: {}", gaps.join(" ")),
            );
        }
        for goal in &def.goals {
            if !goal_ids.contains(goal.as_str()) {
                push(
                    report,
                    project,
                    feature,
                    Severity::Error,
                    format!("links goal '{goal}', which is not in the project charter"),
                );
            }
        }
        if !def.started_unapproved.trim().is_empty() {
            push(
                report,
                project,
                feature,
                Severity::Warning,
                format!(
                    "started without approval: {} — review and approve what was actually built",
                    def.started_unapproved.trim()
                ),
            );
        }
        if def.requirements.is_empty() {
            push(
                report,
                project,
                feature,
                Severity::Warning,
                "has no requirements — nothing states what must be true for this to be done"
                    .to_string(),
            );
        }
        for r in &def.requirements {
            scan_requirement(project, feature, r, report);
        }
        // INVEST, reduced to what is mechanically checkable: Small, and Testable (above).
        if let Some(days) = feature.estimate_days.filter(|d| *d > 3.0) {
            push(
                report,
                project,
                feature,
                Severity::Warning,
                format!("estimated at {days} days — slice it smaller (INVEST: Small)"),
            );
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

/// Live work under the method: not finished, on the board, and created since adoption.
fn in_scope(project: &Project, charter: &crate::Charter, feature: &crate::FeatureItem) -> bool {
    !crate::graph::is_terminal_status(&project.config, &feature.status)
        && project
            .config
            .displayed_states
            .iter()
            .any(|s| s == &feature.status)
        && feature.created_at.as_str() >= charter.adopted_at.as_str()
}

/// Per-requirement checks. The unsupported-claim ones matter most: a quality tag or a measured
/// number that nothing verifies reads as rigour while being decoration.
fn scan_requirement(
    project: &Project,
    feature: &crate::FeatureItem,
    r: &crate::models::Requirement,
    report: &mut Report,
) {
    let id = &r.id;
    let mut warn = |message: String| push(report, project, feature, Severity::Warning, message);
    if crate::ears::classify(&r.text).is_none() {
        warn(format!(
            "{id} is not in EARS form (THE SYSTEM SHALL … / WHEN … / WHILE … / WHERE … / IF …)"
        ));
    }
    if r.tests.is_empty() {
        warn(format!("{id} has no test — it cannot be shown to be met"));
    }
    for tag in &r.iso {
        if crate::ears::normalize_iso(tag).is_none() {
            warn(format!(
                "{id} is tagged '{tag}', which is not an ISO/IEC 25010 characteristic"
            ));
        }
    }
    if matches!(r.kind, crate::models::RequirementKind::Nfr) {
        if r.iso.is_empty() {
            warn(format!(
                "{id} is a quality requirement with no ISO 25010 tag"
            ));
        }
        match r.scenario.as_ref() {
            None => warn(format!(
                "{id} is a quality requirement with no scenario (stimulus, environment, response, measure)"
            )),
            Some(s) if s.measure.trim().is_empty() => warn(format!(
                "{id} has a quality scenario with no measure — an unmeasured quality is an opinion"
            )),
            Some(s) => {
                // The measure must name what checks it; a number nothing verifies is a claim.
                let measure = s.measure.to_lowercase();
                let named = r
                    .tests
                    .iter()
                    .any(|t| !t.name.trim().is_empty() && measure.contains(&t.name.to_lowercase()));
                if !named {
                    warn(format!(
                        "{id}: the measure names no test or benchmark that checks it (unsupported claim)"
                    ));
                }
            }
        }
    } else if !r.iso.is_empty() {
        warn(format!(
            "{id} carries a quality tag but is not a quality requirement (kind: nfr)"
        ));
    }
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
fn scan_project(project: &Project, existing: &BTreeSet<String>, report: &mut Report) {
    let pid = &project.id;

    // Outdated schema_version (warning).
    if project.config.schema_version < CURRENT_SCHEMA_VERSION {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: pid.clone(),
            code: None,
            message: format!(
                "config schema_version {} is older than current {} (re-save to migrate)",
                project.config.schema_version, CURRENT_SCHEMA_VERSION
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
