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
use crate::report::{cycle_time_days, finished_at, parse_time, percentiles};
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

    /// `since` bounds the wave by when an item was last touched, so a window means "the work of
    /// the last fortnight" rather than "items created in it" — an item that started earlier and
    /// finished inside the window is part of what that window delivered.
    fn covers(&self, feature: &FeatureItem) -> bool {
        self.milestone
            .as_ref()
            .is_none_or(|m| &feature.milestone == m)
            && self
                .label
                .as_ref()
                .is_none_or(|l| feature.labels.iter().any(|x| x == l))
            && crate::report::within(Some(feature.updated_at.as_str()), self.since.as_deref())
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
    /// Every item is finished. Separate from `finished`, because a wave can be over without the
    /// board knowing when — saying "still running" in that case is simply false.
    pub all_done: bool,
    pub scope_growth: ScopeGrowth,
    pub defects: Defects,
    pub cycle_time_days: Option<crate::report::Percentiles>,
    /// Items that left a terminal status again, with the date it happened.
    pub rework: Vec<String>,
    /// Items with no recorded moves at all — their flow numbers are absent, not zero.
    pub no_history: Vec<String>,
    /// The oldest day the activity log reaches, when the wave began before it. Silence about an
    /// earlier period means "not recorded here", which is a different claim from "nothing happened"
    /// (FEAT-066).
    pub log_starts: Option<String>,
    /// Items whose only timestamps come from the changelog. The activity log records when the
    /// BOARD was written, not how long work took, so these are reported separately and never
    /// mixed into the cycle time — a five-minute median for a month of work is worse than a gap.
    pub approximate: Vec<String>,
    pub evidence: Evidence,
    pub estimates: Vec<Estimate>,
    /// What this wave taught, as it stood when the retro was produced (FEAT-064). Derived from the
    /// lessons rows rather than copied into the document, so the scored record stays canonical —
    /// but a reader looking for "what did we learn" finds it where they look for it.
    pub lessons: Vec<crate::lessons::Lesson>,
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
pub fn run(store: &Store, id: &str, wave: &Wave) -> Result<Retro> {
    let project = store.load_meta(id)?;
    // The whole log, not a window: this is the fallback for items that finished before transitions
    // were recorded, and asking for a bounded slice is what made it silently useless (FEAT-066).
    let moves = crate::activity::read_all(store.data_dir(), id);
    let items: Vec<&FeatureItem> = project.features.iter().filter(|f| wave.covers(f)).collect();

    // The wave begins when work on it begins — the first recorded transition across its items,
    // NOT the earliest creation. Items planned together are created seconds apart, so measuring
    // growth from a creation time makes every one of them after the first look like it crept in.
    let started = items.iter().filter_map(|f| first_move(f, &moves)).min();
    let all_done = !items.is_empty() && items.iter().all(|f| is_done(&project, f));
    let finished = items
        .iter()
        .filter_map(|f| finished_at(&project, f))
        .max()
        .filter(|_| all_done);

    let mut retro = Retro {
        project: id.to_string(),
        wave: wave.describe(),
        items: items.len(),
        completed: 0,
        still_open: 0,
        started: started.clone(),
        finished,
        all_done,
        scope_growth: ScopeGrowth::default(),
        defects: Defects::default(),
        cycle_time_days: None,
        rework: Vec::new(),
        no_history: Vec::new(),
        log_starts: None,
        approximate: Vec::new(),
        evidence: Evidence::default(),
        estimates: Vec::new(),
        lessons: Vec::new(),
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

        // Flow, from recorded transitions only. An item that finished before histories existed
        // gets an approximate span from the changelog, reported apart from the real numbers.
        match cycle_time_days(&project, f) {
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
            _ if !f.history.is_empty() => {}
            _ if cycle_from_activity(&project, f, &moves).is_some() => {
                retro.approximate.push(f.code.clone())
            }
            _ => retro.no_history.push(f.code.clone()),
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
                // A retrospective is a historical account: what matters is whether the wave
                // produced evidence, not whether that evidence is current. Staleness against the
                // project's head is `kanbanr report`'s question, and applying it here erased every
                // requirement proven before the most recent commit.
                let green = r.tests.iter().any(|t| t.state == TestState::Green);
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
    // Only worth saying when the wave actually predates the log: otherwise it is noise about a
    // boundary nobody is near.
    retro.log_starts = crate::activity::oldest_day(store.data_dir(), id).filter(|oldest| {
        retro
            .started
            .as_deref()
            .or(items.first().map(|f| f.created_at.as_str()))
            .is_some_and(|began| &began[..10.min(began.len())] < oldest.as_str())
    });
    // A lesson belongs to the wave whose work taught it, or to the retrospective that promoted it.
    // Retired ones are included: a retrospective is a historical account, and the wave did learn
    // it — what changed afterwards is a fact about the lesson, not about the wave.
    let document = document_path(wave, wave.milestone.as_deref());
    retro.lessons = crate::lessons::load(store, id)?
        .into_iter()
        .filter(|l| {
            wave_codes.contains(&l.from_item.as_str())
                || (!l.from_retro.trim().is_empty()
                    && (l.from_retro == document
                        || wave
                            .milestone
                            .as_deref()
                            .is_some_and(|m| l.from_retro.contains(m))))
        })
        .collect();
    retro.lessons.sort_by(|a, b| {
        b.confidence_now()
            .partial_cmp(&a.confidence_now())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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
            // A wave the board never watched cannot be retrospected honestly: with no recorded
            // transition on any item, every flow number is absent and the narrative would be
            // invention. Asking for one anyway is how a prompt becomes noise to dismiss.
            !items.is_empty()
                && items.iter().all(|f| is_done(&project, f))
                && items.iter().any(|f| !f.history.is_empty())
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
        let r = run(&store, "demo", &wave).unwrap();
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

    /// The bug this test exists for (FEAT-060): twelve items planned in one batch were reported
    /// as "1 to begin with, 11 added", because the wave's start was read from a creation time and
    /// items created milliseconds apart therefore looked like creep.
    #[test]
    fn planned_together_is_not_scope_growth() {
        let (store, dir) = fixture();
        let planned: Vec<String> = (0..3)
            .map(|i| {
                store
                    .add_feature("demo", &format!("Item {i}"), "", "MS-1", None)
                    .unwrap()
                    .code
            })
            .collect();
        let wave = Wave {
            milestone: Some("MS-1".into()),
            ..Default::default()
        };

        // Nothing has moved: the wave has not started, so nothing can have crept in.
        let r = run(&store, "demo", &wave).unwrap();
        assert_eq!(r.scope_growth.original, 3);
        assert_eq!(r.scope_growth.added(), 0, "{:?}", r.scope_growth);
        assert!(r.started.is_none());

        // Work begins, and only what arrives afterwards is growth.
        store
            .move_feature("demo", &planned[0], "Scheduled")
            .unwrap();
        let late = store.add_feature("demo", "Late", "", "MS-1", None).unwrap();
        let r = run(&store, "demo", &wave).unwrap();
        assert_eq!(r.scope_growth.original, 3);
        assert_eq!(r.scope_growth.unclassified, vec![late.code]);
        assert!(r.started.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Also FEAT-060: a retro is a historical account. Evidence recorded during the wave counts,
    /// whatever the project's head is now — staleness is the report's question, not this one's.
    #[test]
    fn evidence_recorded_during_the_wave_still_counts() {
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
        // Green, recorded at a revision the project has long since moved past.
        store
            .set_test_state(
                "demo",
                &f.code,
                "R-1",
                "cart::holds",
                TestState::Green,
                Some("a-commit-from-during-the-wave"),
            )
            .unwrap();
        complete(&store, &f.code);

        let r = run(&store, "demo", &Wave::default()).unwrap();
        assert_eq!((r.evidence.proven, r.evidence.requirements), (1, 1));
        assert!(
            r.evidence.finished_unproven.is_empty(),
            "the wave did prove it: {:?}",
            r.evidence.finished_unproven
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
        let r = run(&store, "demo", &wave).unwrap();
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
        let r = run(&store, "demo", &wave).unwrap();
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

    /// FEAT-063: the Stop hook asked for retrospectives on three waves that finished months
    /// before the method existed. A wave the board never watched has nothing to retrospect, and a
    /// prompt with nothing behind it is a prompt people learn to dismiss.
    /// FEAT-066: a wave that began before the log reaches is told so. Silence about an earlier
    /// period means "not recorded here", which is a different claim from "nothing happened" — and
    /// reporting the second when you mean the first is what made every pre-method wave look empty.
    #[test]
    fn a_period_older_than_the_log_is_named_as_such() {
        let (store, dir) = fixture();
        let f = store
            .add_feature("demo", "Old work", "", "MS-1", None)
            .unwrap();
        complete(&store, &f.code);

        // No log at all: nothing to claim about a boundary.
        let r = run(&store, "demo", &Wave::default()).unwrap();
        assert!(r.log_starts.is_none());

        // A log that starts well after this wave began.
        let folder = store.data_dir().join("projects/demo/activity");
        std::fs::create_dir_all(&folder).unwrap();
        let later: Vec<crate::activity::Activity> = vec![crate::activity::Activity {
            time: "2099-01-01T00:00:00Z".into(),
            actor: "t".into(),
            message: "much later".into(),
            item: None,
        }];
        std::fs::write(
            folder.join("2099-01-01.yaml"),
            serde_yaml::to_string(&later).unwrap(),
        )
        .unwrap();
        let r = run(&store, "demo", &Wave::default()).unwrap();
        assert_eq!(
            r.log_starts.as_deref(),
            Some("2099-01-01"),
            "the wave predates the log, and the report says where the log begins"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_wave_the_board_never_watched_is_not_due() {
        let (store, dir) = fixture();
        let f = store
            .add_feature("demo", "Old work", "", "MS-1", None)
            .unwrap();
        complete(&store, &f.code);
        assert_eq!(due(&store, "demo").unwrap(), vec!["MS-1".to_string()]);

        // Strip the history, as an item finished before transitions were recorded would have.
        let mut project = store.load("demo").unwrap();
        project.features.iter_mut().for_each(|x| x.history.clear());
        for feature in &project.features {
            store.persist_feature_for_test("demo", feature).unwrap();
        }
        assert!(
            due(&store, "demo").unwrap().is_empty(),
            "nothing was recorded, so there is nothing to write up"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn changelog_timestamps_are_reported_as_approximate() {
        let (store, dir) = fixture();
        let f = store
            .add_feature("demo", "Old work", "", "MS-1", None)
            .unwrap();
        complete(&store, &f.code);
        let mut project = store.load("demo").unwrap();
        project.features.iter_mut().for_each(|x| x.history.clear());
        for feature in &project.features {
            store.persist_feature_for_test("demo", feature).unwrap();
        }
        // Two changelog entries a day apart: a span, but of board writes, not of work.
        let moves: Vec<crate::activity::Activity> =
            ["2026-09-20T09:00:00Z", "2026-09-21T09:00:00Z"]
                .iter()
                .map(|at| crate::activity::Activity {
                    time: (*at).to_string(),
                    actor: "t".into(),
                    message: format!("move feature {} -> Completed", f.code),
                    item: Some(f.code.clone()),
                })
                .collect();
        crate::activity::write_legacy_for_test(store.data_dir(), "demo", &moves);

        let r = run(&store, "demo", &Wave::default()).unwrap();
        assert!(
            r.cycle_time_days.is_none(),
            "a changelog gap is not a cycle time: {:?}",
            r.cycle_time_days
        );
        assert_eq!(r.approximate, vec![f.code.clone()]);
        assert!(r.no_history.is_empty());
        // And a wave that is over but undated says so rather than claiming to be running.
        assert!(r.all_done);
        assert!(r.finished.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_finished_wave_without_a_timestamp_says_so() {
        let (store, dir) = fixture();
        let f = store.add_feature("demo", "Work", "", "MS-1", None).unwrap();
        complete(&store, &f.code);
        let done = run(&store, "demo", &Wave::default()).unwrap();
        assert!(
            done.all_done && done.finished.is_some(),
            "recorded, so dated"
        );

        let open = store.add_feature("demo", "More", "", "MS-1", None).unwrap();
        let r = run(&store, "demo", &Wave::default()).unwrap();
        assert!(!r.all_done, "{} is still open", open.code);
        assert!(
            r.finished.is_none(),
            "a wave is not over while an item is open"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FEAT-064: lessons are scored rows so they can be matched and decay — but a reader asking
    /// "what did this wave teach us?" looks in the retrospective, and nothing was there.
    #[test]
    fn a_waves_lessons_are_part_of_its_retrospective() {
        let (store, dir) = fixture();
        let item = store
            .add_feature("demo", "The work", "", "MS-1", None)
            .unwrap();
        let wave = Wave {
            milestone: Some("MS-1".into()),
            ..Default::default()
        };
        assert!(run(&store, "demo", &wave).unwrap().lessons.is_empty());

        // A lesson recorded against an item of this wave belongs to this wave.
        crate::lessons::add(
            &store,
            "demo",
            crate::lessons::Lesson {
                lesson: "Run it against the real board first".into(),
                from_item: item.code.clone(),
                evidence: "it was wrong twice".into(),
                ..Default::default()
            },
        )
        .unwrap();
        // One promoted by the retrospective itself belongs to it too.
        crate::lessons::add(
            &store,
            "demo",
            crate::lessons::Lesson {
                lesson: "Waves grow by defects more than by discovery".into(),
                from_retro: "retros/MS-1-2026-09-26.md".into(),
                ..Default::default()
            },
        )
        .unwrap();
        // A lesson from elsewhere does not.
        crate::lessons::add(
            &store,
            "demo",
            crate::lessons::Lesson {
                lesson: "Something learned on another wave entirely".into(),
                from_item: "FEAT-999".into(),
                ..Default::default()
            },
        )
        .unwrap();

        let r = run(&store, "demo", &wave).unwrap();
        let texts: Vec<&str> = r.lessons.iter().map(|l| l.lesson.as_str()).collect();
        assert_eq!(r.lessons.len(), 2, "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("real board first")));
        assert!(texts.iter().any(|t| t.contains("Waves grow by defects")));
        assert!(!texts.iter().any(|t| t.contains("another wave entirely")));

        // Contradicted into retirement, it is still part of what this wave learned: a retro is a
        // historical account, and what changed later is a fact about the lesson, not the wave.
        let retired =
            crate::lessons::judge(&store, "demo", "L-1", false, "it did not hold").unwrap();
        assert_eq!(retired.status, crate::lessons::LessonStatus::Retired);
        let r = run(&store, "demo", &wave).unwrap();
        assert_eq!(r.lessons.len(), 2, "still two — one of them now retired");
        assert!(
            r.lessons
                .iter()
                .any(|l| l.status == crate::lessons::LessonStatus::Retired)
        );
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
