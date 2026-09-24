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
        scan_charter(
            &crate::charter::load(store, &project.id)?,
            &project.id,
            &mut report,
        );
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
    scan_charter(&crate::charter::load(store, id)?, id, &mut report);
    Ok(report)
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
