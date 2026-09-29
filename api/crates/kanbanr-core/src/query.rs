//! Rich feature query: filter + full-text search across one project or the whole portfolio (FEAT-032).
//!
//! This is a **read-only** view. A [`Query`] declares a set of filters (status / milestone / kind /
//! priority / labels / assignee / team / due range / dependency-state) plus an optional full-text
//! term; [`run`] evaluates it against one project (`Query::project = Some(id)`) or every project
//! (cross-project / portfolio-wide) and returns the matching [`QueryHit`]s.
//!
//! Cost discipline: filtering and title matching use `load_meta` (metadata only — no spec `.md`
//! reads), and the dependency-state filter reuses the already-built [`DependencyView`]. Specification
//! bodies are only read (via `Store::feature_spec`) when `full_text` is set AND a `text` term is
//! given — so the common case stays cheap.

use crate::Store;
use crate::error::Result;
use crate::graph::{self, DependencyView, Readiness};
use serde::Serialize;

/// A feature query. All filters are AND-combined; `labels` is any-of (a feature matches if it
/// carries *any* of the requested labels). An empty/`None` field is "don't filter on this".
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Restrict to a single project; `None` searches every project (cross-project / portfolio-wide).
    pub project: Option<String>,
    pub status: Option<String>,
    pub milestone: Option<String>,
    pub kind: Option<String>,
    pub priority: Option<String>,
    /// Any-of: a hit must carry at least one of these labels.
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub team: Option<String>,
    /// Inclusive lower bound on `due` (string compare; ISO dates sort lexically).
    pub due_after: Option<String>,
    /// Inclusive upper bound on `due` (string compare).
    pub due_before: Option<String>,
    /// Only features whose derived dependency-state is `Ready` (all deps terminal / no deps).
    pub ready: bool,
    /// Only features whose derived dependency-state is `Blocked` (a dep is not yet terminal).
    pub blocked: bool,
    /// Free-text term matched (case-insensitively) against the title (and the spec body when
    /// `full_text` is set).
    pub text: Option<String>,
    /// When set, the `text` term is also matched against each feature's specification body. This
    /// reads spec files, so it is opt-in to keep the default path cheap.
    pub full_text: bool,
    /// Only items serving this charter goal id (FEAT-050) — the "what is this for?" filter.
    pub goal: Option<String>,
    /// Only items with this kind of gap: `why` (no definition or missing dimensions), `test`
    /// (a requirement with no test, or none green), `approval` (unapproved, lapsed, or started
    /// without one). This is the troubleshooting half of search.
    pub gap: Option<String>,
}

/// Which field a full-text term matched in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchField {
    /// The statement, a dimension answer, or a requirement's text.
    Definition,
    /// A test name recorded against a requirement.
    Test,
    Title,
    Spec,
}

/// One feature matching a [`Query`].
#[derive(Debug, Clone, Serialize)]
pub struct QueryHit {
    /// Qualified id `"project:code"`.
    pub id: String,
    pub project: String,
    pub code: String,
    pub title: String,
    pub status: String,
    pub milestone: String,
    /// Which fields the `text` term matched in (empty when no text term was given).
    pub matched: Vec<MatchField>,
}

impl Query {
    /// True when no dependency-state restriction is requested (both flags unset, or — defensively —
    /// both set, which would be unsatisfiable, so we treat it as "no filter" only when neither is).
    fn wants_dep_state(&self) -> bool {
        self.ready || self.blocked
    }
}

fn eq_opt(want: &Option<String>, have: &Option<String>) -> bool {
    match want {
        None => true,
        Some(w) => have.as_deref() == Some(w.as_str()),
    }
}

/// Evaluate `query` against the store, returning the matching hits ordered by qualified id.
///
/// One project (`query.project = Some(id)`) or all projects (`None`) are scanned with `load_meta`
/// (metadata + titles, no spec bodies). The dependency-state filter, when requested, consults a
/// single [`DependencyView`] built once. Spec bodies are read only for full-text matching.
pub fn run(store: &Store, query: &Query) -> Result<Vec<QueryHit>> {
    let projects: Vec<String> = match &query.project {
        Some(p) => {
            // Surface a not-found error for an unknown project (mirrors other scoped routes).
            store.load_meta(p)?;
            vec![p.clone()]
        }
        None => store.list_projects()?,
    };

    // Build the dependency view once if a dep-state filter is requested (it spans all projects).
    let view = if query.wants_dep_state() {
        Some(DependencyView::build(store, None)?)
    } else {
        None
    };

    let needle = query
        .text
        .as_ref()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty());

    let mut hits = Vec::new();
    for pid in &projects {
        let project = store.load_meta(pid)?;
        for f in &project.features {
            // ---- cheap attribute filters (metadata only) ----
            if !eq_opt(&query.status, &Some(f.status.clone())) {
                continue;
            }
            if !eq_opt(&query.milestone, &Some(f.milestone.clone())) {
                continue;
            }
            if !eq_opt(&query.kind, &f.kind) {
                continue;
            }
            if !eq_opt(&query.priority, &f.priority) {
                continue;
            }
            if !eq_opt(&query.assignee, &f.assignee) {
                continue;
            }
            if !eq_opt(&query.team, &f.team) {
                continue;
            }
            if !query.labels.is_empty() && !query.labels.iter().any(|l| f.labels.contains(l)) {
                continue;
            }
            // Due-range (string compare). A feature with no due date can't satisfy a bound.
            if let Some(after) = &query.due_after {
                match &f.due {
                    Some(d) if d.as_str() >= after.as_str() => {}
                    _ => continue,
                }
            }
            if let Some(before) = &query.due_before {
                match &f.due {
                    Some(d) if d.as_str() <= before.as_str() => {}
                    _ => continue,
                }
            }

            // ---- dependency-state filter (reuse DependencyView) ----
            if let Some(view) = &view {
                let readiness = view.readiness(&graph::qualify(pid, &f.code));
                if query.ready && readiness != Some(Readiness::Ready) {
                    continue;
                }
                if query.blocked && readiness != Some(Readiness::Blocked) {
                    continue;
                }
            }

            // ---- goal and gap filters (FEAT-050): the troubleshooting half of search ----
            if let Some(goal) = &query.goal {
                let serves = f
                    .definition
                    .as_ref()
                    .is_some_and(|d| d.goals.iter().any(|g| g == goal));
                if !serves {
                    continue;
                }
            }
            if let Some(gap) = &query.gap
                && !has_gap(f, gap)
            {
                continue;
            }

            // ---- full-text (title always; spec only when --full-text and a term is set) ----
            let mut matched = Vec::new();
            if let Some(needle) = &needle {
                if f.title.to_lowercase().contains(needle) {
                    matched.push(MatchField::Title);
                }
                // The definition is already in memory, so searching it costs nothing — unlike the
                // spec body, which is read from disk only when asked for.
                if let Some(def) = &f.definition {
                    let in_definition = [
                        def.statement.as_str(),
                        def.zachman.what.as_str(),
                        def.zachman.how.as_str(),
                        def.zachman.where_.as_str(),
                        def.zachman.when.as_str(),
                        def.zachman.who.as_str(),
                        def.zachman.why.as_str(),
                    ]
                    .iter()
                    .any(|v| v.to_lowercase().contains(needle))
                        || def
                            .requirements
                            .iter()
                            .any(|r| r.text.to_lowercase().contains(needle));
                    if in_definition {
                        matched.push(MatchField::Definition);
                    }
                    if def
                        .requirements
                        .iter()
                        .flat_map(|r| r.tests.iter())
                        .any(|t| t.name.to_lowercase().contains(needle))
                    {
                        matched.push(MatchField::Test);
                    }
                }
                if query.full_text {
                    // Read the one spec body on demand (kept out of the cheap path).
                    let spec = store.feature_spec(pid, &f.code).unwrap_or_default();
                    if spec.to_lowercase().contains(needle) {
                        matched.push(MatchField::Spec);
                    }
                }
                if matched.is_empty() {
                    continue; // a text term was given but nothing matched
                }
            }

            hits.push(QueryHit {
                id: graph::qualify(pid, &f.code),
                project: pid.clone(),
                code: f.code.clone(),
                title: f.title.clone(),
                status: f.status.clone(),
                milestone: f.milestone.clone(),
                matched,
            });
        }
    }

    hits.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(hits)
}

/// Does this item have the named kind of gap? The vocabulary is deliberately small — these are the
/// three questions asked when something has gone wrong: why does this exist, is it verified, and
/// did anyone agree to it.
fn has_gap(f: &crate::FeatureItem, gap: &str) -> bool {
    // The same rules every other surface uses (FEAT-112). An unknown gap name matches nothing,
    // rather than silently matching everything.
    crate::readiness::query_group(gap)
        .is_some_and(|checks| !crate::readiness::evaluate(f, None, checks).is_empty())
}
