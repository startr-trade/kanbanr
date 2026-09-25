//! What actually happened in a wave (FEAT-054).
//!
//! A wave grows while it runs — defects found inside it are added to it, work gets sliced, new work
//! is discovered — so "we finished twelve items" can hide that four of them were damage the wave
//! caused. This module answers the question the count cannot: **how did it go, and why**.
//!
//! Every number here is derived from what the board already recorded: transition history, defect
//! blocks, test states, estimates. Nothing is self-reported, and nothing is inferred from a shape
//! that merely looks like a cause — an item that joined mid-wave and says nothing about why is
//! reported as unclassified rather than assigned to a bucket that reads well.
//!
//! The narrative belongs to whoever writes it, and lives in a separate section of the document, so
//! a reader can always tell the measured part from the argued part.

use crate::error::Result;
use crate::models::TestState;
use crate::report::{cycle_time_days, finished_at, parse_time, percentiles, within};
use crate::store::Project;
use crate::{FeatureItem, Store};
use serde::{Deserialize, Serialize};

/// Which items the retro covers. All three filters intersect; none means the whole board.
#[derive(Debug, Clone, Default)]
pub struct Wave {
    pub milestone: Option<String>,
    /// RFC3339 lower bound: items that finished, or joined, on or after this.
    pub since: Option<String>,
    pub label: Option<String>,
}

impl Wave {
    /// How the wave will be described in the report and the document title.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(m) = &self.milestone {
            parts.push(m.clone());
        }
        if let Some(l) = &self.label {
            parts.push(format!("label {l}"));
        }
        if let Some(s) = &self.since {
            parts.push(format!("since {}", s.get(..10).unwrap_or(s)));
        }
        if parts.is_empty() {
            "the whole board".to_string()
        } else {
            parts.join(", ")
        }
    }

    fn covers(&self, feature: &FeatureItem) -> bool {
        self.milestone
            .as_ref()
            .is_none_or(|m| &feature.milestone == m)
            && self
                .label
                .as_ref()
                .is_none_or(|l| feature.labels.iter().any(|x| x == l))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retro {
    pub project: String,
    pub wave: String,
    pub items: usize,
    pub completed: usize,
    pub still_open: usize,
    /// When the wave's first item started, and when its last one finished.
    pub started: Option<String>,
    pub finished: Option<String>,
    pub scope_growth: ScopeGrowth,
    pub defects: Defects,
    pub cycle_time_days: Option<crate::report::Percentiles>,
    /// Items that left a terminal status again, with the date it happened.
    pub rework: Vec<String>,
    /// Items with no recorded moves at all — their flow numbers are absent, not zero.
    pub no_history: Vec<String>,
    pub evidence: Evidence,
    pub estimates: Vec<Estimate>,
}

/// Why the wave is bigger than it started. Each bucket is something an item *says* about itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScopeGrowth {
    /// Items that were there when the wave's first item started.
    pub original: usize,
    /// Added later because something was broken.
    pub defects: Vec<String>,
    /// Added later by slicing an item that was already in the wave.
    pub split: Vec<String>,
    /// Added later, saying nothing about where it came from. Not a bucket to be proud of: it is
    /// the honest place for everything the board cannot attribute.
    pub unclassified: Vec<String>,
}

impl ScopeGrowth {
    pub fn added(&self) -> usize {
        self.defects.len() + self.split.len() + self.unclassified.len()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Defects {
    pub total: usize,
    /// Found after the work was called done.
    pub escaped: usize,
    /// Caused by work inside this same wave — the wave damaging itself.
    pub self_inflicted: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    pub requirements: usize,
    pub proven: usize,
    /// Finished items whose requirements are not all proven: done by status, not by evidence.
    pub finished_unproven: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estimate {
    pub code: String,
    pub estimate_days: f64,
    pub actual_days: f64,
}

/// Build the retro for one wave.
pub fn run(store: &Store, id: &str, wave: &Wave, head_rev: Option<&str>) -> Result<Retro> {
    let project = store.load_meta(id)?;
    let moves = crate::activity::read(store.data_dir(), id, 1000);
    let items: Vec<&FeatureItem> = project.features.iter().filter(|f| wave.covers(f)).collect();

    // The wave begins when its earliest item was created: everything after that is growth.
    let started = items.iter().filter_map(|f| first_move(f, &moves)).min();
    let finished = items
        .iter()
        .filter_map(|f| finished_at(&project, f))
        .max()
        .filter(|_| items.iter().all(|f| is_done(&project, f)));

    let mut retro = Retro {
        project: id.to_string(),
        wave: wave.describe(),
        items: items.len(),
        completed: 0,
        still_open: 0,
        started: started.clone(),
        finished,
        scope_growth: ScopeGrowth::default(),
        defects: Defects::default(),
        cycle_time_days: None,
        rework: Vec::new(),
        no_history: Vec::new(),
        evidence: Evidence::default(),
        estimates: Vec::new(),
    };
    let wave_codes: Vec<&str> = items.iter().map(|f| f.code.as_str()).collect();
    let mut cycle_times: Vec<f64> = Vec::new();

    for f in &items {
        let done = is_done(&project, f);
        if done {
            retro.completed += 1;
        } else {
            retro.still_open += 1;
        }
        if !within(Some(f.created_at.as_str()), wave.since.as_deref()) && wave.since.is_some() {
            // Outside the window: counted as part of the wave, but not as growth within it.
        }

        // Scope growth: an item created after the wave's first item began is an addition.
        let joined_late = started
            .as_deref()
            .is_some_and(|start| f.created_at.as_str() > start);
        if !joined_late {
            retro.scope_growth.original += 1;
        } else if f.defect.is_some() || is_defect_kind(f) {
            retro.scope_growth.defects.push(f.code.clone());
        } else if let Some(parent) = f.split_from.as_deref().filter(|p| !p.trim().is_empty()) {
            retro
                .scope_growth
                .split
                .push(format!("{} ← {parent}", f.code));
        } else {
            retro.scope_growth.unclassified.push(f.code.clone());
        }

        if let Some(defect) = &f.defect {
            retro.defects.total += 1;
            if defect.escaped {
                retro.defects.escaped += 1;
            }
            let from = defect.introduced_by.trim();
            if !from.is_empty() && wave_codes.contains(&from) {
                retro
                    .defects
                    .self_inflicted
                    .push(format!("{} ← {from}", f.code));
            }
        }

        // Flow. History is the source; the activity log is the fallback for items that predate it.
        match cycle_time_days(&project, f).or_else(|| cycle_from_activity(&project, f, &moves)) {
            Some(days) if done => {
                cycle_times.push(days);
                if let Some(estimate) = f.estimate_days.filter(|d| *d > 0.0) {
                    retro.estimates.push(Estimate {
                        code: f.code.clone(),
                        estimate_days: estimate,
                        actual_days: (days * 10.0).round() / 10.0,
                    });
                }
            }
            _ if f.history.is_empty() && first_move(f, &moves).is_none() => {
                retro.no_history.push(f.code.clone())
            }
            _ => {}
        }
        for t in &f.history {
            if crate::graph::is_terminal_status(&project.config, &t.from)
                && !crate::graph::is_terminal_status(&project.config, &t.to)
            {
                retro
                    .rework
                    .push(format!("{} ({})", f.code, t.at.get(..10).unwrap_or(&t.at)));
            }
        }

        // Evidence at completion: a finished item whose requirements nothing proves.
        if let Some(def) = &f.definition {
            let mut unproven = 0;
            for r in &def.requirements {
                retro.evidence.requirements += 1;
                let green = r.tests.iter().any(|t| {
                    t.state == TestState::Green
                        && head_rev
                            .is_none_or(|head| t.checked_rev.is_empty() || t.checked_rev == head)
                });
                if green {
                    retro.evidence.proven += 1;
                } else {
                    unproven += 1;
                }
            }
            if done && unproven > 0 {
                retro
                    .evidence
                    .finished_unproven
                    .push(format!("{} ({unproven})", f.code));
            }
        }
    }

    retro.cycle_time_days = percentiles(&mut cycle_times);
    Ok(retro)
}

fn is_done(project: &Project, f: &FeatureItem) -> bool {
    crate::graph::is_terminal_status(&project.config, &f.status)
}

fn is_defect_kind(f: &FeatureItem) -> bool {
    f.kind
        .as_deref()
        .is_some_and(|k| matches!(k.to_lowercase().as_str(), "defect" | "bug"))
}

/// When this item first moved — from its history, else from the activity log.
fn first_move(f: &FeatureItem, moves: &[crate::activity::Activity]) -> Option<String> {
    f.history
        .first()
        .map(|t| t.at.clone())
        .or_else(|| activity_times(f, moves).into_iter().min())
        .or_else(|| Some(f.created_at.clone()))
}

/// The activity log records every write with the item it touched, so an item created before
/// histories existed still has dated moves — just less precise ones. Best effort by design: it is
/// better to say "about this long, from the changelog" than to report nothing for the whole era
/// before the field existed.
fn activity_times(f: &FeatureItem, moves: &[crate::activity::Activity]) -> Vec<String> {
    moves
        .iter()
        .filter(|a| a.item.as_deref() == Some(f.code.as_str()))
        .filter(|a| a.message.contains("move") || a.message.contains("->"))
        .map(|a| a.time.clone())
        .collect()
}

fn cycle_from_activity(
    project: &Project,
    f: &FeatureItem,
    moves: &[crate::activity::Activity],
) -> Option<f64> {
    if !f.history.is_empty() || !is_done(project, f) {
        return None;
    }
    let times = activity_times(f, moves);
    let (first, last) = (times.iter().min()?, times.iter().max()?);
    let (start, end) = (parse_time(first)?, parse_time(last)?);
    (end > start).then(|| (end - start) / 86_400.0)
}

/// Milestones that are finished and have no retrospective document yet (FEAT-054). This is what
/// the Stop hook surfaces: the moment a wave ends is the only moment its lessons are still fresh.
pub fn due(store: &Store, id: &str) -> Result<Vec<String>> {
    let project = store.load_meta(id)?;
    let written = store.list_docs(id).unwrap_or_default().join("\n");
    Ok(project
        .milestones
        .iter()
        .filter(|m| {
            let items: Vec<&FeatureItem> = project
                .features
                .iter()
                .filter(|f| f.milestone == m.code)
                .collect();
            !items.is_empty() && items.iter().all(|f| is_done(&project, f))
        })
        .filter(|m| !written.contains(&format!("retros/{}", m.code)))
        .map(|m| m.code.clone())
        .collect())
}

/// The document a retro is written to: facts first, and the narrative kept visibly separate so a
/// reader always knows which part the board vouches for.
pub fn document_path(wave: &Wave, code: Option<&str>) -> String {
    let name = code
        .map(str::to_string)
        .unwrap_or_else(|| wave.describe().replace([' ', ',', '/'], "-"));
    format!(
        "retros/{}-{}.md",
        name,
        crate::now_rfc3339().get(..10).unwrap_or("undated")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{Defect, FeatureDefinition, Requirement, TestRef};

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-retro-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "Wave", "", vec![], Some("MS-1".into()))
            .unwrap();
        (store, dir)
    }

    fn complete(store: &Store, code: &str) {
        store.move_feature("demo", code, "Scheduled").unwrap();
        store.move_feature("demo", code, "Completed").unwrap();
    }

    #[test]
    fn scope_growth_counts_only_what_items_record() {
        let (store, dir) = fixture();
        let first = store
            .add_feature("demo", "The work", "", "MS-1", None)
            .unwrap();
        // Everything below joins after the wave's first item started.
        store
            .move_feature("demo", &first.code, "Scheduled")
            .unwrap();
        let defect = store
            .add_feature("demo", "It breaks", "", "MS-1", None)
            .unwrap();
        store
            .set_defect(
                "demo",
                &defect.code,
                Some(Defect {
                    introduced_by: first.code.clone(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let split = store
            .add_feature("demo", "Second half", "", "MS-1", None)
            .unwrap();
        store
            .set_split_from("demo", &split.code, Some(first.code.clone()))
            .unwrap();
        let mystery = store
            .add_feature("demo", "Something else", "", "MS-1", None)
            .unwrap();

        let wave = Wave {
            milestone: Some("MS-1".into()),
            ..Default::default()
        };
        let r = run(&store, "demo", &wave, None).unwrap();
        assert_eq!(r.items, 4);
        assert_eq!(
            r.scope_growth.original, 1,
            "only the item the wave began with"
        );
        assert_eq!(r.scope_growth.defects, vec![defect.code.clone()]);
        assert_eq!(
            r.scope_growth.split,
            vec![format!("{} ← {}", split.code, first.code)]
        );
        assert_eq!(
            r.scope_growth.unclassified,
            vec![mystery.code],
            "an item that says nothing about where it came from is not quietly filed as discovered"
        );
        assert_eq!(r.scope_growth.added(), 3);
        // A defect blamed on an item in this same wave is the wave damaging itself.
        assert_eq!(r.defects.total, 1);
        assert_eq!(
            r.defects.self_inflicted,
            vec![format!("{} ← {}", defect.code, first.code)]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_numbers_come_from_the_board_and_nowhere_else() {
        let (store, dir) = fixture();
        let f = store.add_feature("demo", "Cart", "", "MS-1", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "THE SYSTEM SHALL hold the cart.".into(),
                        tests: vec![TestRef {
                            name: "cart::holds".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        complete(&store, &f.code);

        let wave = Wave::default();
        let r = run(&store, "demo", &wave, None).unwrap();
        assert_eq!((r.completed, r.still_open), (1, 0));
        assert_eq!(r.evidence.requirements, 1);
        assert_eq!(r.evidence.proven, 0);
        assert_eq!(
            r.evidence.finished_unproven,
            vec![format!("{} (1)", f.code)],
            "finished by status, not by evidence — the gap a wave count hides"
        );
        assert!(r.rework.is_empty());

        // Reopening it is rework, and the retro says when.
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        store
            .set_test_state(
                "demo",
                &f.code,
                "R-1",
                "cart::holds",
                TestState::Green,
                None,
            )
            .unwrap();
        let r = run(&store, "demo", &wave, None).unwrap();
        assert_eq!(r.rework.len(), 1, "{:?}", r.rework);
        assert_eq!(r.evidence.proven, 1);
        assert!(
            r.evidence.finished_unproven.is_empty(),
            "no longer finished"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn history_falls_back_to_the_activity_log() {
        let (store, dir) = fixture();
        let f = store
            .add_feature("demo", "Old work", "", "MS-1", None)
            .unwrap();
        complete(&store, &f.code);
        // Strip the history, as an item created before histories existed would have.
        let mut project = store.load("demo").unwrap();
        let feature = project
            .features
            .iter_mut()
            .find(|x| x.code == f.code)
            .unwrap();
        feature.history.clear();
        let bare = feature.clone();
        assert_eq!(
            cycle_time_days(&project, &bare),
            None,
            "nothing to derive from"
        );

        let moves = vec![
            crate::activity::Activity {
                time: "2026-09-20T09:00:00Z".into(),
                actor: "t".into(),
                message: "move feature FEAT-001 -> Scheduled".into(),
                item: Some(f.code.clone()),
            },
            crate::activity::Activity {
                time: "2026-09-23T09:00:00Z".into(),
                actor: "t".into(),
                message: "move feature FEAT-001 -> Completed".into(),
                item: Some(f.code.clone()),
            },
        ];
        assert_eq!(cycle_from_activity(&project, &bare, &moves), Some(3.0));
        // With neither source, the item is reported as having no history rather than as instant.
        assert_eq!(cycle_from_activity(&project, &bare, &[]), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_finished_wave_with_no_document_is_due() {
        let (store, dir) = fixture();
        let f = store
            .add_feature("demo", "Only item", "", "MS-1", None)
            .unwrap();
        assert!(due(&store, "demo").unwrap().is_empty(), "still open");
        complete(&store, &f.code);
        assert_eq!(due(&store, "demo").unwrap(), vec!["MS-1".to_string()]);

        store
            .write_doc("demo", "retros/MS-1-2026-09-25.md", "# Retro")
            .unwrap();
        assert!(
            due(&store, "demo").unwrap().is_empty(),
            "written up, so no longer due"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
