//! Flow and quality numbers derived from what the board already records (FEAT-053).
//!
//! Nothing here is self-reported. Cycle time comes from the transition history, coverage from the
//! test states the capture hook writes, and the escape rate from defects marked as found after the
//! work was called done. Anything that cannot be derived is simply absent rather than estimated —
//! a made-up number is worse than a missing one, because it gets quoted.

use crate::error::Result;
use crate::models::TestState;
use crate::store::Project;
use crate::{FeatureItem, Store};
use serde::Serialize;

/// The window a report covers. Items are counted by when they *finished*, not when they started,
/// so "what shipped in the last fortnight" means what it says.
#[derive(Debug, Clone, Default)]
pub struct Window {
    /// RFC3339 lower bound; items finishing before it are out of scope.
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub project: String,
    pub since: Option<String>,
    /// Items that reached a terminal status inside the window.
    pub completed: usize,
    /// Items in a displayed, non-terminal status right now.
    pub in_progress: usize,
    /// Days from first active move to terminal, for items completed in the window.
    pub cycle_time_days: Option<Percentiles>,
    /// Items that left a terminal status again — work called done that was not.
    pub rework: usize,
    /// Defects created in the window, and how many were found after the work was called done.
    pub defects: usize,
    pub escaped_defects: usize,
    /// Share of requirements with at least one green test, across in-scope items.
    pub requirement_coverage: Coverage,
    /// Items whose green test evidence predates the current project revision.
    pub stale_evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Percentiles {
    pub p50: f64,
    pub p90: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Coverage {
    pub proven: usize,
    pub total: usize,
}

impl Coverage {
    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.proven as f64 / self.total as f64) * 100.0
    }
}

/// Build the report for one project.
pub fn run(store: &Store, id: &str, window: &Window, head_rev: Option<&str>) -> Result<Report> {
    let project = store.load_meta(id)?;
    let since = window.since.as_deref();

    let mut cycle_times: Vec<f64> = Vec::new();
    let mut completed = 0;
    let mut in_progress = 0;
    let mut rework = 0;
    let mut defects = 0;
    let mut escaped = 0;
    let mut coverage = Coverage::default();
    let mut stale_evidence = Vec::new();

    for f in &project.features {
        let terminal = crate::graph::is_terminal_status(&project.config, &f.status);
        let finished_at = finished_at(&project, f);

        if terminal && within(finished_at.as_deref(), since) {
            completed += 1;
            if let Some(days) = cycle_time_days(&project, f) {
                cycle_times.push(days);
            }
        }
        if !terminal
            && project
                .config
                .displayed_states
                .iter()
                .any(|s| s == &f.status)
        {
            in_progress += 1;
        }
        // Work that left a terminal status again: the honest signal that "done" was premature.
        rework += f
            .history
            .iter()
            .filter(|t| {
                crate::graph::is_terminal_status(&project.config, &t.from)
                    && !crate::graph::is_terminal_status(&project.config, &t.to)
                    && within(Some(t.at.as_str()), since)
            })
            .count();

        if let Some(defect) = &f.defect
            && within(Some(f.created_at.as_str()), since)
        {
            defects += 1;
            if defect.escaped {
                escaped += 1;
            }
        }

        if let Some(def) = &f.definition {
            for r in &def.requirements {
                coverage.total += 1;
                if r.tests.iter().any(|t| t.state == TestState::Green) {
                    coverage.proven += 1;
                }
                // A green recorded against an older revision is a claim about code that has since
                // changed; report it rather than counting it as proof.
                if let Some(head) = head_rev {
                    let stale = r.tests.iter().any(|t| {
                        t.state == TestState::Green
                            && !t.checked_rev.is_empty()
                            && t.checked_rev != head
                    });
                    if stale {
                        stale_evidence.push(format!("{}/{}", f.code, r.id));
                    }
                }
            }
        }
    }

    Ok(Report {
        project: id.to_string(),
        since: window.since.clone(),
        completed,
        in_progress,
        cycle_time_days: percentiles(&mut cycle_times),
        rework,
        defects,
        escaped_defects: escaped,
        requirement_coverage: coverage,
        stale_evidence,
    })
}

/// When the item last reached a terminal status.
fn finished_at(project: &Project, f: &FeatureItem) -> Option<String> {
    f.history
        .iter()
        .rev()
        .find(|t| crate::graph::is_terminal_status(&project.config, &t.to))
        .map(|t| t.at.clone())
}

/// Days from the first move out of the backlog to the last move into a terminal status. Items with
/// no recorded history (created before histories existed) contribute nothing rather than a guess.
fn cycle_time_days(project: &Project, f: &FeatureItem) -> Option<f64> {
    let started = f
        .history
        .iter()
        .find(|t| {
            t.to != project.config.default_state
                && !crate::graph::is_terminal_status(&project.config, &t.to)
                && !project.config.is_no_op(&t.to)
        })
        .map(|t| t.at.as_str())?;
    let finished = finished_at(project, f)?;
    let (start, end) = (parse_time(started)?, parse_time(&finished)?);
    ((end - start) / 86_400.0).into()
}

/// Seconds since the epoch for an RFC3339 timestamp, without pulling in a parser: the board writes
/// a fixed `YYYY-MM-DDTHH:MM:SS(.fraction)Z` shape.
fn parse_time(ts: &str) -> Option<f64> {
    let date = ts.get(..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    // Days since a fixed civil epoch (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let time = ts.get(11..19).unwrap_or("00:00:00");
    let mut hms = time.split(':');
    let hh: f64 = hms.next()?.parse().ok()?;
    let mm: f64 = hms.next()?.parse().ok()?;
    let ss: f64 = hms.next().unwrap_or("0").parse().ok()?;
    Some(days as f64 * 86_400.0 + hh * 3600.0 + mm * 60.0 + ss)
}

fn within(at: Option<&str>, since: Option<&str>) -> bool {
    match (at, since) {
        (_, None) => true,
        (Some(at), Some(since)) => at >= since,
        (None, Some(_)) => false,
    }
}

/// p50/p90 by nearest-rank. With a handful of items a percentile is a rough guide, not a
/// statistic — which is why the raw count sits beside it in the output.
fn percentiles(values: &mut [f64]) -> Option<Percentiles> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = |q: f64| {
        let rank = ((values.len() as f64) * q).ceil() as usize;
        values[rank.clamp(1, values.len()) - 1]
    };
    Some(Percentiles {
        p50: at(0.5),
        p90: at(0.9),
        max: *values.last().unwrap_or(&0.0),
    })
}

/// An RFC3339 timestamp `days` before now, for `--since 14d`.
pub fn days_ago(days: i64) -> String {
    let now = time::OffsetDateTime::now_utc() - time::Duration::days(days);
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{Defect, FeatureDefinition, Requirement, TestRef};

    /// A store in a throwaway directory, with one project and one milestone.
    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-report-{}-{}",
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
        (store, dir)
    }

    fn definition(tests: Vec<TestRef>) -> FeatureDefinition {
        FeatureDefinition {
            requirements: vec![Requirement {
                id: "R-1".into(),
                text: "THE SYSTEM SHALL retain the cart.".into(),
                tests,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn moves_are_recorded_and_counted() {
        let (store, dir) = fixture();
        let shipped = store.add_feature("demo", "Shipped", "", "M", None).unwrap();
        let open = store.add_feature("demo", "Open", "", "M", None).unwrap();
        store
            .move_feature("demo", &shipped.code, "Scheduled")
            .unwrap();
        let shipped = store
            .move_feature("demo", &shipped.code, "Completed")
            .unwrap();
        store.move_feature("demo", &open.code, "Scheduled").unwrap();

        // Every move is appended, in order, with where it came from.
        let steps: Vec<(&str, &str)> = shipped
            .history
            .iter()
            .map(|t| (t.from.as_str(), t.to.as_str()))
            .collect();
        assert_eq!(
            steps,
            vec![("Planned", "Scheduled"), ("Scheduled", "Completed")]
        );

        let r = run(&store, "demo", &Window::default(), None).unwrap();
        assert_eq!((r.completed, r.in_progress, r.rework), (1, 1, 0));

        // Reopening completed work is rework — the honest signal that "done" was premature.
        store
            .move_feature("demo", &shipped.code, "Scheduled")
            .unwrap();
        let r = run(&store, "demo", &Window::default(), None).unwrap();
        assert_eq!(
            (r.completed, r.rework),
            (0, 1),
            "reopened work is no longer complete"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_defect_escapes_when_the_work_it_came_from_was_already_done() {
        let (store, dir) = fixture();
        let feature = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let caught = store
            .add_feature("demo", "Cart drops items", "", "M", None)
            .unwrap();
        // Found while the work was still open: not an escape.
        let caught = store
            .set_defect(
                "demo",
                &caught.code,
                Some(Defect {
                    introduced_by: feature.code.clone(),
                    found_in: "review".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        assert!(!caught.defect.unwrap().escaped);

        store
            .move_feature("demo", &feature.code, "Scheduled")
            .unwrap();
        store
            .move_feature("demo", &feature.code, "Completed")
            .unwrap();
        let escaped = store
            .add_feature("demo", "Cart empties", "", "M", None)
            .unwrap();
        let escaped = store
            .set_defect(
                "demo",
                &escaped.code,
                Some(Defect {
                    introduced_by: feature.code.clone(),
                    found_in: "production".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        assert!(
            escaped.defect.unwrap().escaped,
            "the work was already called done, so this one got out"
        );

        let r = run(&store, "demo", &Window::default(), None).unwrap();
        assert_eq!((r.defects, r.escaped_defects), (2, 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn coverage_counts_green_tests_and_flags_evidence_from_an_older_revision() {
        let (store, dir) = fixture();
        let proven = store.add_feature("demo", "Proven", "", "M", None).unwrap();
        let claimed = store.add_feature("demo", "Claimed", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &proven.code,
                Some(definition(vec![TestRef {
                    name: "cart::retains".into(),
                    ..Default::default()
                }])),
            )
            .unwrap();
        store
            .set_feature_definition("demo", &claimed.code, Some(definition(vec![])))
            .unwrap();
        // Planned, not green: a requirement nobody has shown to hold.
        let r = run(&store, "demo", &Window::default(), None).unwrap();
        assert_eq!(
            (r.requirement_coverage.proven, r.requirement_coverage.total),
            (0, 2)
        );

        store
            .set_test_state(
                "demo",
                &proven.code,
                "R-1",
                "cart::retains",
                TestState::Green,
                Some("abc123"),
            )
            .unwrap();
        let r = run(&store, "demo", &Window::default(), None).unwrap();
        assert_eq!(
            (r.requirement_coverage.proven, r.requirement_coverage.total),
            (1, 2)
        );
        assert_eq!(r.requirement_coverage.percent(), 50.0);
        assert!(
            r.stale_evidence.is_empty(),
            "green at the revision we asked about"
        );

        // The same green, read against a newer revision, is a claim about code that has changed.
        let r = run(&store, "demo", &Window::default(), Some("def456")).unwrap();
        assert_eq!(r.stale_evidence, vec![format!("{}/R-1", proven.code)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cycle_time_spans_the_first_active_move_to_the_last_terminal_one() {
        let (store, dir) = fixture();
        let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        store.move_feature("demo", &f.code, "Completed").unwrap();

        // Rewrite the two timestamps in memory: real moves are seconds apart, and the arithmetic
        // is what is under test here, not the clock.
        let mut project = store.load_meta("demo").unwrap();
        let feature = project
            .features
            .iter_mut()
            .find(|x| x.code == f.code)
            .unwrap();
        feature.history[0].at = "2026-09-20T09:00:00Z".into();
        feature.history[1].at = "2026-09-23T21:00:00Z".into();
        let feature = feature.clone();
        assert_eq!(cycle_time_days(&project, &feature), Some(3.5));
        assert_eq!(
            finished_at(&project, &feature).as_deref(),
            Some("2026-09-23T21:00:00Z")
        );
        // Windows are inclusive of the boundary and exclude what finished before it.
        assert!(within(
            Some("2026-09-23T21:00:00Z"),
            Some("2026-09-23T21:00:00Z")
        ));
        assert!(!within(
            Some("2026-09-19T00:00:00Z"),
            Some("2026-09-20T00:00:00Z")
        ));
        // An item with no history contributes nothing rather than a guess.
        let bare = store.add_feature("demo", "Bare", "", "M", None).unwrap();
        assert_eq!(cycle_time_days(&project, &bare), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn civil_dates_and_percentiles() {
        // Two timestamps a day apart really are 86400 seconds apart.
        let a = parse_time("2026-09-24T00:00:00Z").unwrap();
        let b = parse_time("2026-09-25T00:00:00Z").unwrap();
        assert_eq!(b - a, 86_400.0);
        // Across a month and year boundary, too.
        let c = parse_time("2026-12-31T23:00:00Z").unwrap();
        let d = parse_time("2027-01-01T00:00:00Z").unwrap();
        assert_eq!(d - c, 3600.0);

        let mut values = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        let p = percentiles(&mut values).unwrap();
        assert_eq!((p.p50, p.p90, p.max), (3.0, 5.0, 5.0));
        assert!(percentiles(&mut []).is_none(), "no data, no number");
    }
}
