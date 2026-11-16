//! Items, filtered (FTR-048): the list every number on the Overview links down to.
//!
//! Derived from the board on each read and never stored (ADR-0002, ADR-0005). The evidence filters
//! use [`crate::report::item_evidence`], the rule the flow report counts coverage by, so a tile's
//! number and the list it opens cannot disagree.

use crate::dispatch::{only_params, query_param};
use crate::error::{CoreError, Result};
use crate::report::item_evidence;
use crate::{FeatureItem, Store};
use serde::Serialize;

/// The query parameters `GET /projects/{p}/items` takes; any other is refused.
pub const PARAMS: &[&str] = &[
    "kind",
    "status",
    "milestone",
    "goal",
    "sprint",
    "release",
    "evidence",
];

/// Which items to list. Every filter given must hold (AND); an absent one matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFilter {
    /// The item's kind, lower-cased and trimmed as the Overview counts it; `none` for untyped.
    pub kind: Option<String>,
    pub status: Option<String>,
    pub milestone: Option<String>,
    /// A charter goal id the item's definition serves.
    pub goal: Option<String>,
    pub sprint: Option<String>,
    pub release: Option<String>,
    pub evidence: Option<Evidence>,
}

/// What an item's evidence shows, by [`crate::report::item_evidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Evidence {
    /// At least one red test.
    Red,
    /// At least one test named and not yet run.
    Planned,
    /// A green recorded against an older revision.
    Stale,
    /// Fewer requirements proven than it has.
    Unproven,
    /// Every requirement proven, and at least one.
    Proven,
    /// At least one requirement with no test named.
    Untested,
}

impl Evidence {
    /// The query values, in the order the OpenAPI description lists them.
    pub const VALUES: &'static [&'static str] =
        &["red", "planned", "stale", "unproven", "proven", "untested"];

    fn parse(s: &str) -> Option<Evidence> {
        Some(match s {
            "red" => Evidence::Red,
            "planned" => Evidence::Planned,
            "stale" => Evidence::Stale,
            "unproven" => Evidence::Unproven,
            "proven" => Evidence::Proven,
            "untested" => Evidence::Untested,
            _ => return None,
        })
    }
}

/// One item in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemRow {
    pub code: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub status: String,
    pub milestone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    pub req_total: usize,
    pub req_proven: usize,
    pub tests_red: usize,
    pub tests_planned: usize,
    pub stale: bool,
}

/// The kind as counted and filtered: lower-cased and trimmed, `None` when absent or empty. No
/// kind is mapped onto another (`bug` and `defect` stay apart).
pub fn normal_kind(kind: Option<&str>) -> Option<String> {
    kind.map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
}

/// The filter a query string asks for. An unknown key, or an `evidence` value outside
/// [`Evidence::VALUES`], is refused (400); an empty value is no filter.
pub fn parse_filter(query: Option<&str>) -> Result<ItemFilter> {
    only_params(query, PARAMS)?;
    let get = |key: &str| query_param(query, key).filter(|v| !v.trim().is_empty());
    let evidence = match get("evidence") {
        Some(v) => Some(Evidence::parse(v.trim()).ok_or_else(|| {
            CoreError::Unsupported(format!(
                "unknown evidence '{v}' (allowed: {})",
                Evidence::VALUES.join(", ")
            ))
        })?),
        None => None,
    };
    Ok(ItemFilter {
        kind: get("kind"),
        status: get("status"),
        milestone: get("milestone"),
        goal: get("goal"),
        sprint: get("sprint"),
        release: get("release"),
        evidence,
    })
}

/// The project's items that `filter` matches, in index order.
pub fn list(store: &Store, p: &str, filter: &ItemFilter) -> Result<Vec<ItemRow>> {
    let project = store.load_meta(p)?;
    Ok(project
        .features
        .iter()
        .filter_map(|f| row_if_matching(f, filter))
        .collect())
}

fn row_if_matching(f: &FeatureItem, filter: &ItemFilter) -> Option<ItemRow> {
    let kind = normal_kind(f.kind.as_deref());
    if let Some(want) = &filter.kind {
        let want = want.trim().to_lowercase();
        let matches = if want == "none" {
            kind.is_none()
        } else {
            kind.as_deref() == Some(want.as_str())
        };
        if !matches {
            return None;
        }
    }
    let same =
        |want: &Option<String>, have: Option<&str>| want.as_deref().is_none_or(|w| have == Some(w));
    if !same(&filter.status, Some(&f.status))
        || !same(&filter.milestone, Some(&f.milestone))
        || !same(&filter.sprint, f.sprint.as_deref())
        || !same(&filter.release, f.release.as_deref())
    {
        return None;
    }
    if let Some(goal) = &filter.goal {
        let serves = f
            .definition
            .as_ref()
            .is_some_and(|d| d.goals.iter().any(|g| g == goal));
        if !serves {
            return None;
        }
    }
    // The list reads no code repository, so it has no head to judge staleness against: `stale`
    // here is what `item_evidence` says with no head.
    let e = item_evidence(f, None);
    if let Some(want) = filter.evidence {
        let holds = match want {
            Evidence::Red => e.tests_red > 0,
            Evidence::Planned => e.tests_planned > 0,
            Evidence::Stale => e.stale,
            Evidence::Unproven => e.req_proven < e.req_total,
            Evidence::Proven => e.req_total > 0 && e.req_proven == e.req_total,
            Evidence::Untested => e.untested_reqs > 0,
        };
        if !holds {
            return None;
        }
    }
    Some(ItemRow {
        code: f.code.clone(),
        title: f.title.clone(),
        kind: f.kind.clone().filter(|k| !k.trim().is_empty()),
        status: f.status.clone(),
        milestone: f.milestone.clone(),
        sprint: f.sprint.clone(),
        release: f.release.clone(),
        req_total: e.req_total,
        req_proven: e.req_proven,
        tests_red: e.tests_red,
        tests_planned: e.tests_planned,
        stale: e.stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef, TestState};

    /// A store in a throwaway directory under the system temp dir, never under the repository
    /// (L-41), with one project and one milestone.
    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-items-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !dir.join(".kanbanr").exists(),
            "the board is the test's own"
        );
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        store
            .add_milestone("demo", "N", "", vec![], Some("N".into()))
            .unwrap();
        (store, dir)
    }

    fn requirement(id: &str, tests: &[&str]) -> Requirement {
        Requirement {
            id: id.into(),
            text: "THE SYSTEM SHALL hold.".into(),
            tests: tests
                .iter()
                .map(|n| TestRef {
                    name: (*n).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn define(store: &Store, code: &str, goals: &[&str], reqs: Vec<Requirement>) {
        store
            .set_feature_definition(
                "demo",
                code,
                Some(FeatureDefinition {
                    goals: goals.iter().map(|g| (*g).into()).collect(),
                    requirements: reqs,
                    ..Default::default()
                }),
            )
            .unwrap();
    }

    fn kind(store: &Store, code: &str, k: &str) {
        store
            .set_feature_attrs(
                "demo",
                code,
                Some(k.into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
    }

    fn codes(rows: &[ItemRow]) -> Vec<&str> {
        rows.iter().map(|r| r.code.as_str()).collect()
    }

    /// The codes a query lists, sorted: which items match, apart from their order.
    fn listed(store: &Store, query: &str) -> Vec<String> {
        let filter = parse_filter(Some(query)).unwrap();
        let mut codes: Vec<String> = list(store, "demo", &filter)
            .unwrap()
            .into_iter()
            .map(|r| r.code)
            .collect();
        codes.sort();
        codes
    }

    /// R-9: each filter alone, two together, and the refusals.
    #[test]
    fn filters_by_kind_status_goal_and_evidence() {
        let (store, dir) = fixture();
        let a = store.add_feature("demo", "A", "", "M", None).unwrap().code;
        let b = store.add_feature("demo", "B", "", "M", None).unwrap().code;
        let c = store.add_feature("demo", "C", "", "N", None).unwrap().code;
        let d = store.add_feature("demo", "D", "", "N", None).unwrap().code;
        kind(&store, &a, "Feature ");
        kind(&store, &b, "bug");
        kind(&store, &c, "defect");
        // d is untyped.
        store.move_feature("demo", &b, "Scheduled").unwrap();
        store
            .set_feature_sprint("demo", &c, Some("SP-1".into()))
            .unwrap();
        store
            .set_feature_release("demo", &d, Some("v1".into()))
            .unwrap();

        // a: one requirement proven. b: one red. c: one planned and one with no test. d: none.
        define(&store, &a, &["G-1"], vec![requirement("R-1", &["a::ok"])]);
        store
            .set_test_state("demo", &a, "R-1", "a::ok", TestState::Green, Some("abc"))
            .unwrap();
        define(
            &store,
            &b,
            &["G-1", "G-2"],
            vec![requirement("R-1", &["b::fails"])],
        );
        store
            .set_test_state("demo", &b, "R-1", "b::fails", TestState::Red, None)
            .unwrap();
        define(
            &store,
            &c,
            &["G-2"],
            vec![requirement("R-1", &["c::later"]), requirement("R-2", &[])],
        );

        // No filter: every item, in index order (the order `load_meta` reads them in).
        let all = list(&store, "demo", &ItemFilter::default()).unwrap();
        let index: Vec<String> = store
            .load_meta("demo")
            .unwrap()
            .features
            .into_iter()
            .map(|f| f.code)
            .collect();
        assert_eq!(codes(&all), index);
        assert_eq!(all.len(), 4);

        // Kind, as counted: trimmed and lower-cased, and no kind mapped onto another.
        assert_eq!(listed(&store, "kind=feature"), [a.as_str()]);
        assert_eq!(listed(&store, "kind=bug"), [b.as_str()]);
        assert_eq!(listed(&store, "kind=defect"), [c.as_str()]);
        assert_eq!(listed(&store, "kind=none"), [d.as_str()]);
        // Status, milestone, sprint and release.
        assert_eq!(listed(&store, "status=Scheduled"), [b.as_str()]);
        assert_eq!(listed(&store, "milestone=N"), [c.as_str(), d.as_str()]);
        assert_eq!(listed(&store, "sprint=SP-1"), [c.as_str()]);
        assert_eq!(listed(&store, "release=v1"), [d.as_str()]);
        // Goal.
        assert_eq!(listed(&store, "goal=G-1"), [a.as_str(), b.as_str()]);
        assert_eq!(listed(&store, "goal=G-2"), [b.as_str(), c.as_str()]);
        // Evidence.
        assert_eq!(listed(&store, "evidence=red"), [b.as_str()]);
        assert_eq!(listed(&store, "evidence=planned"), [c.as_str()]);
        assert_eq!(listed(&store, "evidence=proven"), [a.as_str()]);
        assert_eq!(
            listed(&store, "evidence=unproven"),
            [b.as_str(), c.as_str()]
        );
        assert_eq!(listed(&store, "evidence=untested"), [c.as_str()]);
        assert!(listed(&store, "evidence=stale").is_empty());
        // Two together: AND.
        assert_eq!(listed(&store, "goal=G-2&evidence=red"), [b.as_str()]);
        assert!(listed(&store, "kind=bug&milestone=N").is_empty());
        // An empty value is no filter.
        assert_eq!(listed(&store, "kind=&evidence=").len(), 4);

        // The row carries what the page shows.
        let row = &list(
            &store,
            "demo",
            &parse_filter(Some("evidence=planned")).unwrap(),
        )
        .unwrap()[0];
        assert_eq!(
            (
                row.req_total,
                row.req_proven,
                row.tests_planned,
                row.tests_red
            ),
            (2, 0, 1, 0)
        );
        assert_eq!(row.sprint.as_deref(), Some("SP-1"));
        assert_eq!(row.kind.as_deref(), Some("defect"));

        // Refusals: an unknown evidence value, and an unknown key.
        let bogus = parse_filter(Some("evidence=bogus")).unwrap_err();
        assert!(matches!(bogus, CoreError::Unsupported(_)), "{bogus}");
        let unknown = parse_filter(Some("colour=red")).unwrap_err();
        assert!(matches!(unknown, CoreError::Unsupported(_)), "{unknown}");
        assert_eq!(parse_filter(None).unwrap(), ItemFilter::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-9: `item_evidence` agrees with the report's coverage and stale lists, item by item.
    #[test]
    fn evidence_uses_the_reports_rule() {
        let (store, dir) = fixture();
        let fresh = store
            .add_feature("demo", "Fresh", "", "M", None)
            .unwrap()
            .code;
        let old = store
            .add_feature("demo", "Old", "", "M", None)
            .unwrap()
            .code;
        let open = store
            .add_feature("demo", "Open", "", "M", None)
            .unwrap()
            .code;
        store.add_feature("demo", "Bare", "", "M", None).unwrap();
        define(&store, &fresh, &[], vec![requirement("R-1", &["f::ok"])]);
        define(
            &store,
            &old,
            &[],
            vec![
                requirement("R-1", &["o::ok"]),
                requirement("R-2", &["o::ok2"]),
            ],
        );
        define(&store, &open, &[], vec![requirement("R-1", &["p::red"])]);
        store
            .set_test_state(
                "demo",
                &fresh,
                "R-1",
                "f::ok",
                TestState::Green,
                Some("head"),
            )
            .unwrap();
        store
            .set_test_state(
                "demo",
                &old,
                "R-1",
                "o::ok",
                TestState::Green,
                Some("older"),
            )
            .unwrap();
        store
            .set_test_state("demo", &open, "R-1", "p::red", TestState::Red, Some("head"))
            .unwrap();

        for head in [None, Some("head")] {
            let report =
                crate::report::run(&store, "demo", &crate::report::Window::default(), head)
                    .unwrap();
            let project = store.load_meta("demo").unwrap();
            let (mut proven, mut total) = (0, 0);
            for f in &project.features {
                let e = item_evidence(f, head);
                proven += e.req_proven;
                total += e.req_total;
                let listed_stale = report
                    .stale_evidence
                    .iter()
                    .any(|s| s.starts_with(&format!("{}/", f.code)));
                assert_eq!(e.stale, listed_stale, "{} at {head:?}", f.code);
            }
            assert_eq!(
                (proven, total),
                (
                    report.requirement_coverage.proven,
                    report.requirement_coverage.total
                )
            );
        }
        // The report itself is unchanged: two of four requirements proven, one stale at `head`.
        let report = crate::report::run(
            &store,
            "demo",
            &crate::report::Window::default(),
            Some("head"),
        )
        .unwrap();
        assert_eq!(
            (
                report.requirement_coverage.proven,
                report.requirement_coverage.total
            ),
            (2, 4)
        );
        assert_eq!(report.stale_evidence, vec![format!("{old}/R-1")]);
        let project = store.load_meta("demo").unwrap();
        let e = item_evidence(
            project.features.iter().find(|f| f.code == old).unwrap(),
            Some("head"),
        );
        assert_eq!(
            e,
            crate::report::ItemEvidence {
                req_total: 2,
                req_proven: 1,
                tests_red: 0,
                tests_planned: 1,
                untested_reqs: 0,
                stale: true,
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
