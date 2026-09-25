//! What we learned, and how much we still believe it (FEAT-055).
//!
//! A lesson learned mid-execution exists only in a transcript, so the next session pays for it
//! again. This keeps them in the board, next to the work that produced them.
//!
//! The part that makes it more than a notes file is **decay**. A lessons list that only grows
//! becomes a wall of advice, much of it stale, and a wall nobody reads is the same as no list at
//! all. So confidence falls with age unless something reaffirms it, a contradiction costs more
//! than an affirmation gains, and a lesson that drops below the threshold **retires**: kept as a
//! record, no longer surfaced. Being wrong later is normal; pretending a lesson is still true is
//! what does damage.
//!
//! Stored as a per-project side file following the `charter.yaml` pattern: absent means "none
//! yet", never an error, and it is not part of `Project`.

use crate::Store;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const LESSONS_FILE: &str = "lessons.yaml";

/// Confidence halves after this long without reaffirmation. Long enough that a real lesson
/// survives a quiet month; short enough that a year-old assumption has to earn its place again.
const HALF_LIFE_DAYS: f64 = 90.0;
/// Below this, a lesson stops being surfaced.
pub const RETIRE_BELOW: f64 = 0.25;
/// What one affirmation adds, and one contradiction takes. Contradiction is heavier on purpose:
/// evidence that a lesson is wrong is worth more than another day of it not being challenged.
const AFFIRM: f64 = 0.15;
const CONTRADICT: f64 = 0.45;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LessonKind {
    /// Something that worked and is worth repeating.
    Practice,
    /// Something that cost us, and how to avoid it.
    #[default]
    Pitfall,
    /// A choice made once that later work should not relitigate.
    Decision,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LessonStatus {
    /// Recorded, not yet confirmed by anything else.
    #[default]
    Candidate,
    /// Confirmed at least once.
    Active,
    /// Fell below the threshold, or was contradicted. Kept, not surfaced.
    Retired,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    /// `L-1`, `L-2`, … assigned on save.
    #[serde(default)]
    pub id: String,
    /// The lesson itself, in one sentence a person can act on.
    pub lesson: String,
    #[serde(default)]
    pub kind: LessonKind,
    #[serde(default)]
    pub at: String,
    /// The item that taught it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_item: String,
    /// The retrospective that promoted it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_retro: String,
    /// What actually happened — the part that makes it a lesson rather than an opinion.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
    /// Labels it applies to, so it can be offered to the right work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Charter goals it bears on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    /// How much it was believed at `last_affirmed`. What is *reported* decays from here.
    #[serde(default)]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_affirmed: String,
    #[serde(default)]
    pub status: LessonStatus,
    /// Every affirmation and contradiction, so the number can be explained rather than trusted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Judgement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    pub at: String,
    /// `affirmed` | `contradicted`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Lesson {
    /// Confidence as of now: what was recorded, decayed by how long it has gone unchallenged.
    /// Computed rather than stored, so nothing has to run on a schedule to keep it honest.
    pub fn confidence_now(&self) -> f64 {
        let days = self
            .last_affirmed
            .as_str()
            .or_default_time(&self.at)
            .map(days_since)
            .unwrap_or(0.0);
        let decayed = self.confidence * 0.5_f64.powf(days / HALF_LIFE_DAYS);
        decayed.clamp(0.0, 1.0)
    }

    /// Is this still worth showing someone?
    pub fn surfaced(&self) -> bool {
        self.status != LessonStatus::Retired && self.confidence_now() >= RETIRE_BELOW
    }

    /// Does this lesson bear on that item? Matching is deliberately loose — a lesson missed is
    /// worse than a lesson shown once too often — but it is never "everything".
    pub fn applies_to(&self, feature: &crate::FeatureItem) -> bool {
        if self.from_item == feature.code {
            return true;
        }
        if self.tags.iter().any(|t| {
            feature.labels.iter().any(|l| l.eq_ignore_ascii_case(t))
                || feature
                    .kind
                    .as_deref()
                    .is_some_and(|k| k.eq_ignore_ascii_case(t))
        }) {
            return true;
        }
        let goals = feature
            .definition
            .as_ref()
            .map(|d| d.goals.clone())
            .unwrap_or_default();
        self.goals.iter().any(|g| goals.contains(g))
    }
}

/// A key for "the same lesson said again": case and punctuation folded away, so a re-record with
/// different wording of the same sentence does not create a second entry to maintain.
pub fn dedupe_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(LESSONS_FILE)
}

/// Every lesson, newest first. An absent file means none yet, which is not an error.
pub fn load(store: &Store, id: &str) -> Result<Vec<Lesson>> {
    match std::fs::read_to_string(path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, lessons: &[Lesson]) -> Result<()> {
    let file = path(store, id);
    if lessons.is_empty() {
        let _ = std::fs::remove_file(file);
        return Ok(());
    }
    std::fs::write(file, serde_yaml::to_string(lessons)?)?;
    Ok(())
}

/// Record a lesson. Saying the same thing again affirms the existing one rather than adding a
/// duplicate — repetition IS evidence, and two copies of a lesson are two things to keep true.
pub fn add(store: &Store, id: &str, lesson: Lesson) -> Result<Lesson> {
    let mut lessons = load(store, id)?;
    let key = dedupe_key(&lesson.lesson);
    if key.is_empty() {
        return Err(crate::error::CoreError::Unsupported(
            "a lesson needs something to say".to_string(),
        ));
    }
    if let Some(existing) = lessons.iter().position(|l| dedupe_key(&l.lesson) == key) {
        let code = lessons[existing].id.clone();
        drop(lessons);
        return judge(store, id, &code, true, "recorded again");
    }
    let mut lesson = lesson;
    let taken: Vec<String> = lessons.iter().map(|l| l.id.clone()).collect();
    lesson.id = crate::validate::next_key("L-", &taken);
    lesson.at = crate::now_rfc3339();
    lesson.last_affirmed = lesson.at.clone();
    if lesson.confidence <= 0.0 {
        // A newly recorded lesson is believed, but not yet confirmed by anything else.
        lesson.confidence = 0.6;
    }
    lesson.status = LessonStatus::Candidate;
    lessons.insert(0, lesson.clone());
    save(store, id, &lessons)?;
    Ok(lesson)
}

/// Affirm or contradict a lesson, from evidence. Both reset the clock: what matters is when it was
/// last tested against reality, not when it was written down.
pub fn judge(store: &Store, id: &str, lesson_id: &str, affirm: bool, note: &str) -> Result<Lesson> {
    let mut lessons = load(store, id)?;
    let found = lessons
        .iter_mut()
        .find(|l| l.id == lesson_id)
        .ok_or_else(|| {
            crate::error::CoreError::Unsupported(format!("no lesson {lesson_id} in {id}"))
        })?;
    let from = found.confidence_now();
    found.confidence = if affirm {
        (from + AFFIRM).min(1.0)
    } else {
        (from - CONTRADICT).max(0.0)
    };
    found.last_affirmed = crate::now_rfc3339();
    found.history.push(Judgement {
        at: found.last_affirmed.clone(),
        verdict: if affirm { "affirmed" } else { "contradicted" }.to_string(),
        note: note.trim().to_string(),
    });
    found.status = if found.confidence < RETIRE_BELOW {
        LessonStatus::Retired
    } else if affirm {
        LessonStatus::Active
    } else {
        found.status
    };
    let updated = found.clone();
    save(store, id, &lessons)?;
    Ok(updated)
}

/// The lessons worth showing, most believed first. With `for_item`, only those that bear on it.
pub fn surfaced(
    store: &Store,
    id: &str,
    for_item: Option<&crate::FeatureItem>,
) -> Result<Vec<Lesson>> {
    let mut lessons: Vec<Lesson> = load(store, id)?
        .into_iter()
        .filter(|l| l.surfaced())
        .filter(|l| for_item.is_none_or(|f| l.applies_to(f)))
        .collect();
    lessons.sort_by(|a, b| {
        b.confidence_now()
            .partial_cmp(&a.confidence_now())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(lessons)
}

fn days_since(ts: &str) -> f64 {
    let now = crate::now_rfc3339();
    match (
        crate::report::parse_time(ts),
        crate::report::parse_time(&now),
    ) {
        (Some(then), Some(now)) => ((now - then) / 86_400.0).max(0.0),
        _ => 0.0,
    }
}

/// `last_affirmed`, falling back to when it was written.
trait OrDefaultTime {
    fn or_default_time<'a>(&'a self, fallback: &'a str) -> Option<&'a str>;
}

impl OrDefaultTime for str {
    fn or_default_time<'a>(&'a self, fallback: &'a str) -> Option<&'a str> {
        if self.trim().is_empty() {
            (!fallback.trim().is_empty()).then_some(fallback)
        } else {
            Some(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn fixture() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-lessons-{}-{}",
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

    fn lesson(text: &str) -> Lesson {
        Lesson {
            lesson: text.to_string(),
            kind: LessonKind::Pitfall,
            from_item: "FEAT-001".into(),
            evidence: "it happened, twice".into(),
            tags: vec!["mirror".into()],
            ..Default::default()
        }
    }

    #[test]
    fn a_lesson_is_recorded_once_with_its_provenance() {
        let (store, dir) = fixture();
        let first = add(&store, "demo", lesson("Auto-sync re-pushes every issue")).unwrap();
        assert_eq!(first.id, "L-1");
        assert_eq!(first.from_item, "FEAT-001");
        assert_eq!(first.evidence, "it happened, twice");
        assert_eq!(first.status, LessonStatus::Candidate);

        // The same lesson, said differently: not a second entry — and saying it again is evidence,
        // so it counts as an affirmation.
        let again = add(&store, "demo", lesson("auto-sync RE-PUSHES every issue!!")).unwrap();
        assert_eq!(again.id, "L-1");
        assert_eq!(load(&store, "demo").unwrap().len(), 1);
        assert!(again.confidence > first.confidence, "{again:?}");
        assert_eq!(again.status, LessonStatus::Active);

        // A lesson with nothing to say is refused rather than stored as an empty row.
        assert!(add(&store, "demo", lesson("   ")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn confidence_decays_until_something_reaffirms_it() {
        let fresh = Lesson {
            confidence: 0.8,
            last_affirmed: crate::now_rfc3339(),
            ..lesson("x")
        };
        assert!((fresh.confidence_now() - 0.8).abs() < 0.01);

        // Ninety days unchallenged halves it; a year drops it below the threshold.
        let stale = Lesson {
            last_affirmed: days_ago(90),
            ..fresh.clone()
        };
        assert!(
            (stale.confidence_now() - 0.4).abs() < 0.01,
            "{}",
            stale.confidence_now()
        );
        let ancient = Lesson {
            last_affirmed: days_ago(365),
            ..fresh
        };
        assert!(ancient.confidence_now() < RETIRE_BELOW);
        assert!(
            !ancient.surfaced(),
            "an unchallenged year is not confidence"
        );
    }

    #[test]
    fn contradiction_costs_more_than_affirmation() {
        let (store, dir) = fixture();
        let l = add(&store, "demo", lesson("Prefer a batch to five writes")).unwrap();
        let affirmed = judge(&store, "demo", &l.id, true, "held again on FEAT-002").unwrap();
        let gain = affirmed.confidence - l.confidence;
        let contradicted = judge(&store, "demo", &l.id, false, "it made the write slower").unwrap();
        let loss = affirmed.confidence - contradicted.confidence;
        assert!(
            loss > gain * 2.0,
            "evidence against should outweigh another quiet day: gained {gain}, lost {loss}"
        );
        assert_eq!(contradicted.history.len(), 2);
        assert_eq!(contradicted.history[1].verdict, "contradicted");
        assert_eq!(contradicted.history[1].note, "it made the write slower");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lesson_below_the_threshold_retires_but_is_kept() {
        let (store, dir) = fixture();
        let l = add(&store, "demo", lesson("Always do the thing")).unwrap();
        let out = judge(&store, "demo", &l.id, false, "it was wrong").unwrap();
        assert_eq!(out.status, LessonStatus::Retired);
        assert!(!out.surfaced());
        // Kept: the record of having believed it is part of the history.
        assert_eq!(load(&store, "demo").unwrap().len(), 1);
        assert!(surfaced(&store, "demo", None).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lessons_are_matched_to_the_item_about_to_be_worked_on() {
        let (store, dir) = fixture();
        let tagged = add(&store, "demo", lesson("Mirrors re-push on any write")).unwrap();
        add(
            &store,
            "demo",
            Lesson {
                tags: vec!["docs".into()],
                from_item: String::new(),
                goals: vec!["G-2".into()],
                ..lesson("Write the why before the code")
            },
        )
        .unwrap();

        let mut f = store
            .add_feature("demo", "Mirror sync", "", "M", None)
            .unwrap();
        f.labels = vec!["mirror".into()];
        assert_eq!(f.code, "FEAT-001", "the lesson above names this item");
        let matched = surfaced(&store, "demo", Some(&f)).unwrap();
        assert_eq!(matched.len(), 1, "{matched:?}");
        assert_eq!(matched[0].id, tagged.id);

        // A goal link matches too, so a lesson can apply to work that merely shares a purpose —
        // on a different item, which the first lesson has no other claim on.
        let mut other = store
            .add_feature("demo", "Something else", "", "M", None)
            .unwrap();
        other.definition = Some(crate::models::FeatureDefinition {
            goals: vec!["G-2".into()],
            ..Default::default()
        });
        let matched = surfaced(&store, "demo", Some(&other)).unwrap();
        assert_eq!(matched.len(), 1, "{matched:?}");
        assert!(matched[0].lesson.contains("why before the code"));

        // An item with nothing in common gets nothing: matching is loose, not universal.
        let unrelated = store
            .add_feature("demo", "Unrelated", "", "M", None)
            .unwrap();
        assert!(
            surfaced(&store, "demo", Some(&unrelated))
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn days_ago(days: i64) -> String {
        crate::report::days_ago(days)
    }
}
