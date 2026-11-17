//! The Charter page's derived read (FTR-076): each goal's progress, the releases travelled toward
//! the goals, the lessons by theme and release cycle, and the most recent wins, in one answer.
//!
//! Derived from the board on each read and never stored (ADR-0002, ADR-0005). Every number comes
//! from the rule that already counts it: a goal's items and proof from [`crate::overview`]'s own
//! `goal_facts`, so the Charter and the Overview cannot disagree; a release cell from the filter
//! `GET /items?goal=&release=` applies; lessons from [`crate::lessons`]. No target and no date is
//! invented: a release's `target` is copied as the person set it.

use crate::charter::Charter;
use crate::error::Result;
use crate::lessons::{Lesson, LessonKind, LessonStatus};
use crate::overview::GoalFacts;
use crate::releases::{Release, ReleaseState};
use crate::report::item_evidence;
use crate::store::Project;
use crate::{FeatureItem, Store};
use serde::Serialize;

/// How many recent wins the page lists.
pub const WINS: usize = 5;
/// How many tag themes are kept before the rest fold into "other tags".
pub const THEMES_KEPT: usize = 8;

/// The Charter page's derived read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CharterProgress {
    pub project: String,
    /// The board commit this answer was read from; empty when the board is not a git repository.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub board_rev: String,
    /// When this answer was read, RFC 3339.
    pub generated_at: String,
    /// The project works in releases (`cadence.releases`); when false `route`, `marks` and
    /// `series` are empty.
    pub releases_on: bool,
    /// One per charter goal, in charter order.
    pub goals: Vec<GoalProgress>,
    /// Shipped releases in ship order, then the next planned one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<RouteStop>,
    /// Items per goal and release, only the non-zero cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<GoalMark>,
    /// Per goal, one point per shipped release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<GoalSeries>,
    pub lessons: LessonGrowth,
    /// The most recently finished items that serve a goal, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wins: Vec<Win>,
    /// Every finished item that serves a goal, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finished: Vec<Finished>,
}

/// One charter goal with the work and the lessons serving it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GoalProgress {
    /// The Overview's own counts for the goal.
    #[serde(flatten)]
    pub facts: GoalFacts,
    /// The ids of the lessons that name this goal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<String>,
}

/// Whether a stop on the way travelled has shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum StopState {
    Shipped,
    Next,
}

/// One release on the way travelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RouteStop {
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub state: StopState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shipped_at: String,
    /// The target date as the person set it; empty when none was set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// The release this patches, for a patch release.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub patch_of: String,
    /// Items whose release is this version.
    pub items: usize,
}

/// Items serving `goal` in `release`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GoalMark {
    pub goal: String,
    pub release: String,
    pub items: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GoalSeries {
    pub goal: String,
    pub points: Vec<SeriesPoint>,
}

/// A goal's proof as of one shipped release, read from the evidence recorded today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SeriesPoint {
    pub release: String,
    pub items: usize,
    pub req_proven: usize,
    pub req_total: usize,
}

/// The lessons, counted and grouped.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LessonGrowth {
    /// Lessons in force, by the rule `kanbanr lessons` uses.
    pub in_force: usize,
    /// All lessons, by the rule `kanbanr lessons --all` uses.
    pub total: usize,
    pub retired: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<Theme>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycles: Vec<Cycle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<LessonRow>,
}

/// A group of lessons sharing a tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Theme {
    pub key: String,
    pub label: String,
    /// Lesson ids.
    pub lessons: Vec<String>,
}

/// Lessons recorded in one release cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Cycle {
    pub key: String,
    pub label: String,
    pub recorded: usize,
    pub cumulative: usize,
}

/// One lesson with where it was learned and cited.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LessonRow {
    pub id: String,
    pub lesson: String,
    pub kind: LessonKind,
    pub status: LessonStatus,
    pub at: String,
    /// Confidence as of now, 0 to 1.
    pub confidence_now: f64,
    pub in_force: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_retro: String,
    /// `sprint`, `milestone` or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub retro_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    pub affirmed: usize,
    pub contradicted: usize,
    /// Items, other than the one it came from, whose specification cites the lesson.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cited_by: Vec<String>,
}

/// A recently finished item that serves a goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Win {
    pub code: String,
    pub title: String,
    pub at: String,
    pub goals: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release: String,
}

/// A finished item that serves a goal, for "since you last looked".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Finished {
    pub code: String,
    pub at: String,
    pub goals: Vec<String>,
}

/// The Charter page's one read. Fails only for an unknown project, a newer board, or an
/// unreadable `releases.yaml` or `lessons.yaml` (a missing one is empty).
pub fn charter_progress(store: &Store, p: &str) -> Result<CharterProgress> {
    let project = store.load(p)?;
    let charter = crate::charter::load(store, p)?;
    let releases = crate::releases::load(store, p)?;
    let lessons = crate::lessons::load(store, p)?;
    let sprints = crate::sprints::load(store, p).unwrap_or_default();
    let board_rev = crate::git::head_commit(store.data_dir()).unwrap_or_default();
    Ok(charter_progress_on(
        &project,
        &charter,
        &releases,
        &lessons,
        &sprints,
        &board_rev,
        &crate::now_rfc3339(),
    ))
}

/// The pure part of [`charter_progress`]: every unit test calls it.
pub(crate) fn charter_progress_on(
    project: &Project,
    charter: &Charter,
    releases: &[Release],
    lessons: &[Lesson],
    sprints: &[crate::sprints::Sprint],
    board_rev: &str,
    now: &str,
) -> CharterProgress {
    let releases_on = project.config.cadence.releases;
    let route = if releases_on {
        route(project, releases)
    } else {
        Vec::new()
    };
    let lesson_growth = lesson_growth(project, lessons, sprints, &route);
    let goals: Vec<GoalProgress> = charter
        .goals
        .iter()
        .map(|g| GoalProgress {
            facts: crate::overview::goal_facts(project, &g.id, &g.statement),
            lessons: lesson_growth
                .rows
                .iter()
                .filter(|r| r.goals.contains(&g.id))
                .map(|r| r.id.clone())
                .collect(),
        })
        .collect();
    let marks = marks(project, charter, &route);
    let series = series(project, charter, &route);
    let finished = finished(project);
    let wins = finished
        .iter()
        .take(WINS)
        .filter_map(|f| {
            let item = project.features.iter().find(|i| i.code == f.code)?;
            Some(Win {
                code: f.code.clone(),
                title: item.title.clone(),
                at: f.at.clone(),
                goals: f.goals.clone(),
                release: item.release.clone().unwrap_or_default(),
            })
        })
        .collect();
    CharterProgress {
        project: project.id.clone(),
        board_rev: board_rev.to_string(),
        generated_at: now.to_string(),
        releases_on,
        goals,
        route,
        marks,
        series,
        lessons: lesson_growth,
        wins,
        finished,
    }
}

/// Whether an item's definition names `goal`.
fn serves(f: &FeatureItem, goal: &str) -> bool {
    f.definition
        .as_ref()
        .is_some_and(|d| d.goals.iter().any(|g| g == goal))
}

/// Shipped releases in ship order, then at most one next planned release.
fn route(project: &Project, releases: &[Release]) -> Vec<RouteStop> {
    let count = |version: &str| {
        project
            .features
            .iter()
            .filter(|f| f.release.as_deref() == Some(version))
            .count()
    };
    let stop = |r: &Release, state: StopState| RouteStop {
        version: r.version.clone(),
        name: r.name.clone(),
        state,
        shipped_at: if state == StopState::Shipped {
            r.shipped_at.clone()
        } else {
            String::new()
        },
        target: r.target.clone(),
        patch_of: r.parent.clone(),
        items: count(&r.version),
    };
    let mut shipped: Vec<&Release> = releases
        .iter()
        .filter(|r| r.state == ReleaseState::Shipped && r.superseded_by.is_empty())
        .collect();
    // A release with no ship date sorts after the dated ones, by version.
    shipped.sort_by(|a, b| {
        (a.shipped_at.is_empty(), &a.shipped_at, &a.version).cmp(&(
            b.shipped_at.is_empty(),
            &b.shipped_at,
            &b.version,
        ))
    });
    let mut out: Vec<RouteStop> = shipped
        .into_iter()
        .map(|r| stop(r, StopState::Shipped))
        .collect();
    let next = releases
        .iter()
        .filter(|r| r.state == ReleaseState::Planned && r.superseded_by.is_empty())
        .min_by_key(|r| {
            let in_flight = !r.stage.is_empty() || r.waits_on_tag();
            let parsed = crate::writer::parse_version(r.version.trim_start_matches('v'));
            (
                !in_flight,
                r.target.is_empty(),
                r.target.clone(),
                parsed.is_none(),
                parsed.unwrap_or_default(),
                r.version.clone(),
            )
        });
    if let Some(r) = next {
        out.push(stop(r, StopState::Next));
    }
    out
}

/// Per goal and stop: the items whose release is the stop and whose definition names the goal.
fn marks(project: &Project, charter: &Charter, route: &[RouteStop]) -> Vec<GoalMark> {
    let mut out = Vec::new();
    for g in &charter.goals {
        for s in route {
            let items = project
                .features
                .iter()
                .filter(|f| f.release.as_deref() == Some(s.version.as_str()) && serves(f, &g.id))
                .count();
            if items > 0 {
                out.push(GoalMark {
                    goal: g.id.clone(),
                    release: s.version.clone(),
                    items,
                });
            }
        }
    }
    out
}

/// Per goal, one cumulative point per shipped stop, from the evidence recorded today.
fn series(project: &Project, charter: &Charter, route: &[RouteStop]) -> Vec<GoalSeries> {
    let shipped: Vec<&str> = route
        .iter()
        .filter(|s| s.state == StopState::Shipped)
        .map(|s| s.version.as_str())
        .collect();
    if shipped.is_empty() {
        return Vec::new();
    }
    charter
        .goals
        .iter()
        .map(|g| GoalSeries {
            goal: g.id.clone(),
            points: (0..shipped.len())
                .map(|upto| {
                    let mut p = SeriesPoint {
                        release: shipped[upto].to_string(),
                        items: 0,
                        req_proven: 0,
                        req_total: 0,
                    };
                    for f in project.features.iter().filter(|f| {
                        serves(f, &g.id)
                            && f.release
                                .as_deref()
                                .is_some_and(|r| shipped[..=upto].contains(&r))
                    }) {
                        let e = item_evidence(f, None);
                        p.items += 1;
                        p.req_proven += e.req_proven;
                        p.req_total += e.req_total;
                    }
                    p
                })
                .collect(),
        })
        .collect()
}

/// Every finished item that serves a goal and has a finish time, newest first.
fn finished(project: &Project) -> Vec<Finished> {
    let mut out: Vec<Finished> = project
        .features
        .iter()
        .filter(|f| project.config.counts_as_done(&f.status))
        .filter_map(|f| {
            let goals: Vec<String> = f
                .definition
                .as_ref()
                .map(|d| d.goals.clone())
                .unwrap_or_default();
            if goals.is_empty() {
                return None;
            }
            let at = crate::report::finished_at(project, f)?;
            Some(Finished {
                code: f.code.clone(),
                at,
                goals,
            })
        })
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.code.cmp(&b.code)));
    out
}

/// True when `id` (`L-23`) occurs in `text` as a whole token: the byte before is not ASCII
/// alphanumeric, `-` or `_`, and the byte after is not an ASCII digit.
pub(crate) fn cites(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    text.match_indices(id).any(|(at, _)| {
        let before_ok = at == 0 || {
            let b = bytes[at - 1];
            !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        let after_ok = bytes.get(at + id.len()).is_none_or(|b| !b.is_ascii_digit());
        before_ok && after_ok
    })
}

/// The lessons, counted, grouped by tag and by release cycle, each with where it was learned and
/// cited.
fn lesson_growth(
    project: &Project,
    lessons: &[Lesson],
    sprints: &[crate::sprints::Sprint],
    route: &[RouteStop],
) -> LessonGrowth {
    let mut rows: Vec<LessonRow> = lessons
        .iter()
        .map(|l| {
            let retro_kind = if l.from_retro.is_empty() {
                ""
            } else if sprints.iter().any(|s| s.code == l.from_retro) {
                "sprint"
            } else if project.milestones.iter().any(|m| m.code == l.from_retro) {
                "milestone"
            } else {
                ""
            };
            let verdicts = |v: &str| l.history.iter().filter(|j| j.verdict == v).count();
            let mut cited_by: Vec<String> = project
                .features
                .iter()
                .filter(|f| f.code != l.from_item && item_cites(f, &l.id))
                .map(|f| f.code.clone())
                .collect();
            cited_by.sort();
            LessonRow {
                id: l.id.clone(),
                lesson: l.lesson.clone(),
                kind: l.kind,
                status: l.status,
                at: l.at.clone(),
                confidence_now: l.confidence_now(),
                in_force: l.surfaced(),
                from_item: l.from_item.clone(),
                from_retro: l.from_retro.clone(),
                retro_kind: retro_kind.into(),
                evidence: l.evidence.clone(),
                tags: l.tags.clone(),
                goals: l.goals.clone(),
                affirmed: verdicts("affirmed"),
                contradicted: verdicts("contradicted"),
                cited_by,
            }
        })
        .collect();
    // The `kanbanr lessons` order: in force first, most believed first; ties by id.
    rows.sort_by(|a, b| {
        b.in_force
            .cmp(&a.in_force)
            .then(
                b.confidence_now
                    .partial_cmp(&a.confidence_now)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| a.id.cmp(&b.id))
    });
    let in_force = rows.iter().filter(|r| r.in_force).count();
    LessonGrowth {
        in_force,
        total: rows.len(),
        retired: rows.len() - in_force,
        themes: themes(&rows),
        cycles: cycles(&rows, route),
        rows,
    }
}

/// Whether an item's specification or definition (statement and requirement texts) cites `id`.
fn item_cites(f: &FeatureItem, id: &str) -> bool {
    cites(&f.specification, id)
        || f.definition.as_ref().is_some_and(|d| {
            cites(&d.statement, id) || d.requirements.iter().any(|r| cites(&r.text, id))
        })
}

/// One theme per tag (case folded, labelled as first written), largest first; the eight largest
/// are kept, the rest join "other tags", and lessons without a tag form "untagged".
fn themes(rows: &[LessonRow]) -> Vec<Theme> {
    let mut by_tag: Vec<Theme> = Vec::new();
    let mut untagged = Vec::new();
    for r in rows {
        if r.tags.is_empty() {
            untagged.push(r.id.clone());
        }
        for tag in &r.tags {
            let key = tag.to_lowercase();
            match by_tag.iter_mut().find(|t| t.key == key) {
                Some(t) => {
                    if !t.lessons.contains(&r.id) {
                        t.lessons.push(r.id.clone());
                    }
                }
                None => by_tag.push(Theme {
                    key,
                    label: tag.clone(),
                    lessons: vec![r.id.clone()],
                }),
            }
        }
    }
    by_tag.sort_by(|a, b| {
        b.lessons
            .len()
            .cmp(&a.lessons.len())
            .then_with(|| a.key.cmp(&b.key))
    });
    let mut out: Vec<Theme> = by_tag.drain(..by_tag.len().min(THEMES_KEPT)).collect();
    if !by_tag.is_empty() {
        let mut lessons: Vec<String> = Vec::new();
        for t in by_tag {
            for id in t.lessons {
                if !lessons.contains(&id) {
                    lessons.push(id);
                }
            }
        }
        // Keep the lessons in the table's order.
        lessons.sort_by_key(|id| rows.iter().position(|r| &r.id == id));
        out.push(Theme {
            key: "other".into(),
            label: "other tags".into(),
            lessons,
        });
    }
    if !untagged.is_empty() {
        out.push(Theme {
            key: "untagged".into(),
            label: "untagged".into(),
            lessons: untagged,
        });
    }
    out
}

/// Lessons recorded per release cycle, or per calendar month without releases, cumulative.
/// Retired lessons count: the learning happened.
fn cycles(rows: &[LessonRow], route: &[RouteStop]) -> Vec<Cycle> {
    let time = |s: &str| crate::report::parse_time(s);
    let shipped: Vec<&RouteStop> = route
        .iter()
        .filter(|s| s.state == StopState::Shipped)
        .collect();
    let mut cycles: Vec<(String, String, usize)> = Vec::new();
    if let Some(first) = shipped.first() {
        cycles.push(("before".into(), format!("before {}", first.version), 0));
        for s in shipped.iter().skip(1) {
            cycles.push((s.version.clone(), s.version.clone(), 0));
        }
        let last = shipped.last().map_or("", |s| s.version.as_str());
        cycles.push(("since".into(), format!("since {last}"), 0));
        let ends: Vec<Option<f64>> = shipped.iter().map(|s| time(&s.shipped_at)).collect();
        for r in rows {
            // The first shipped stop on or after the lesson; none (or an unparsable date) is the
            // last cycle.
            let slot = time(&r.at)
                .and_then(|at| ends.iter().position(|e| e.is_some_and(|e| e >= at)))
                .unwrap_or(cycles.len() - 1);
            cycles[slot].2 += 1;
        }
        // The trailing cycle is shown only when it holds something.
        if cycles.last().is_some_and(|c| c.2 == 0) {
            cycles.pop();
        }
    } else {
        let months: Vec<Option<i32>> = rows.iter().map(|r| month_index(&r.at)).collect();
        let (Some(lo), Some(hi)) = (
            months.iter().flatten().min().copied(),
            months.iter().flatten().max().copied(),
        ) else {
            if rows.is_empty() {
                return Vec::new();
            }
            return vec![Cycle {
                key: "undated".into(),
                label: "undated".into(),
                recorded: rows.len(),
                cumulative: rows.len(),
            }];
        };
        for m in lo..=hi {
            let key = format!("{:04}-{:02}", m.div_euclid(12), m.rem_euclid(12) + 1);
            cycles.push((key.clone(), key, 0));
        }
        for m in &months {
            // An unparsable date falls in the last cycle.
            let slot = m.map_or(cycles.len() - 1, |m| (m - lo) as usize);
            cycles[slot].2 += 1;
        }
    }
    let mut running = 0;
    cycles
        .into_iter()
        .map(|(key, label, recorded)| {
            running += recorded;
            Cycle {
                key,
                label,
                recorded,
                cumulative: running,
            }
        })
        .collect()
}

/// `year * 12 + month - 1` for the `YYYY-MM` that starts an RFC 3339 time.
fn month_index(at: &str) -> Option<i32> {
    let year: i32 = at.get(0..4)?.parse().ok()?;
    let month: i32 = at.get(5..7)?.parse().ok()?;
    ((1..=12).contains(&month) && at.get(4..5) == Some("-")).then_some(year * 12 + month - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef, TestState};

    /// A board in its own folder under the temp dir (L-41), checked before the first write.
    fn board(releases: bool) -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-charterview-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        assert!(
            root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
            "a test board lives in the temp dir: {}",
            root.display()
        );
        assert!(
            !dir.join(".kanbanr").exists(),
            "the board is the test's own"
        );
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.cadence.releases = releases;
        store.init_project("demo", config).unwrap();
        assert_eq!(std::fs::canonicalize(store.data_dir()).unwrap(), root);
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    fn goals(ids: &[&str]) -> Charter {
        Charter {
            goals: ids
                .iter()
                .map(|id| crate::charter::Goal {
                    id: (*id).into(),
                    statement: format!("Goal {id}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn requirement(id: &str, state: Option<TestState>) -> Requirement {
        Requirement {
            id: id.into(),
            text: "THE SYSTEM SHALL hold.".into(),
            tests: state
                .map(|s| {
                    vec![TestRef {
                        name: format!("t_{s:?}").to_lowercase(),
                        state: s,
                        ..Default::default()
                    }]
                })
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    /// An item serving `goals` with one requirement per entry of `reqs`, in `release`.
    fn item(
        store: &Store,
        title: &str,
        goals: &[&str],
        reqs: &[Option<TestState>],
        release: Option<&str>,
    ) -> String {
        let f = store.add_feature("demo", title, "", "M", None).unwrap();
        let mut f = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        f.release = release.map(Into::into);
        f.definition = Some(FeatureDefinition {
            goals: goals.iter().map(|g| (*g).into()).collect(),
            requirements: reqs
                .iter()
                .enumerate()
                .map(|(i, s)| requirement(&format!("R-{}", i + 1), *s))
                .collect(),
            ..Default::default()
        });
        store.persist_feature_for_test("demo", &f).unwrap();
        f.code
    }

    /// Finish `code`, then say it finished at `at`.
    fn finish_at(store: &Store, code: &str, at: &str) {
        store.move_feature("demo", code, "Scheduled").unwrap();
        store.move_feature("demo", code, "Completed").unwrap();
        let mut f = store.load("demo").unwrap().feature(code).unwrap().clone();
        f.history.last_mut().unwrap().at = at.into();
        store.persist_feature_for_test("demo", &f).unwrap();
    }

    fn shipped(version: &str, at: &str) -> Release {
        Release {
            version: version.into(),
            state: ReleaseState::Shipped,
            shipped_at: at.into(),
            ..Default::default()
        }
    }

    fn write_releases(store: &Store, all: &[Release]) {
        std::fs::write(
            store.project_dir("demo").join("releases.yaml"),
            crate::yaml::to_string(&all).unwrap(),
        )
        .unwrap();
    }

    fn read(store: &Store, charter: &Charter) -> CharterProgress {
        let project = store.load("demo").unwrap();
        let releases = crate::releases::load(store, "demo").unwrap();
        charter_progress_on(
            &project,
            charter,
            &releases,
            &[],
            &[],
            "abc1234",
            "2026-11-17T10:00:00Z",
        )
    }

    fn lesson(id: &str, at: &str, edit: impl FnOnce(&mut Lesson)) -> Lesson {
        let mut l = Lesson {
            id: id.into(),
            lesson: format!("Lesson {id}"),
            at: at.into(),
            confidence: 0.9,
            last_affirmed: crate::now_rfc3339(),
            ..Default::default()
        };
        edit(&mut l);
        l
    }

    fn read_with(store: &Store, charter: &Charter, lessons: &[Lesson]) -> CharterProgress {
        let project = store.load("demo").unwrap();
        let releases = crate::releases::load(store, "demo").unwrap();
        let sprints = crate::sprints::load(store, "demo").unwrap();
        charter_progress_on(
            &project,
            charter,
            &releases,
            lessons,
            &sprints,
            "abc1234",
            "2026-11-17T10:00:00Z",
        )
    }

    /// FTR-076 R-2: the goal rows are the Overview's own counts.
    #[test]
    fn goal_progress_matches_the_overview_goal_counts() {
        let (store, dir) = board(false);
        let charter = goals(&["G-1", "G-2"]);
        crate::charter::save(&store, "demo", &charter).unwrap();
        let a = item(
            &store,
            "Both",
            &["G-1", "G-2"],
            &[Some(TestState::Green), None],
            None,
        );
        finish_at(&store, &a, "2026-11-10T10:00:00Z");
        item(&store, "First", &["G-1"], &[Some(TestState::Red)], None);
        item(&store, "Second", &["G-2"], &[None, None], None);

        let progress = charter_progress(&store, "demo").unwrap();
        let overview = crate::overview::overview(&store, "demo", None).unwrap();
        let ours: Vec<&GoalFacts> = progress.goals.iter().map(|g| &g.facts).collect();
        let theirs: Vec<&GoalFacts> = overview.goals.iter().collect();
        assert_eq!(ours, theirs);
        assert_eq!(
            (ours[0].items_done, ours[0].items_open, ours[0].req_proven),
            (1, 1, 1)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-2: a goal no item serves has zero counts, and says so by having no items.
    #[test]
    fn a_goal_no_item_serves_reads_as_unserved() {
        let (store, dir) = board(false);
        let charter = goals(&["G-1", "G-2"]);
        item(
            &store,
            "Only first",
            &["G-1"],
            &[Some(TestState::Green)],
            None,
        );
        let p = read(&store, &charter);
        let g2 = p.goals.iter().find(|g| g.facts.id == "G-2").unwrap();
        assert_eq!(
            (g2.facts.items_done, g2.facts.items_open, g2.facts.req_total),
            (0, 0, 0)
        );
        assert_eq!(g2.facts.title, "Goal G-2");
        assert!(g2.lessons.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-4: ship order, not version order; a superseded release is absent; a patch
    /// carries its parent; of two planned releases the one at a stage is next.
    #[test]
    fn the_way_travelled_lists_shipped_releases_in_ship_order_and_the_next_one() {
        let (store, dir) = board(true);
        item(&store, "In v0.3.0", &["G-1"], &[None], Some("v0.3.0"));
        item(&store, "In v0.2.1", &["G-1"], &[None], Some("v0.2.1"));
        item(&store, "In v0.2.1 too", &["G-1"], &[None], Some("v0.2.1"));
        write_releases(
            &store,
            &[
                shipped("v0.3.0", "2026-10-01T09:00:00Z"),
                Release {
                    parent: "v0.2.0".into(),
                    ..shipped("v0.2.1", "2026-10-05T09:00:00Z")
                },
                Release {
                    superseded_by: "v0.2.1".into(),
                    ..shipped("v0.2.0", "2026-09-30T09:00:00Z")
                },
                Release {
                    version: "v0.4.0".into(),
                    target: "2026-12-01".into(),
                    ..Default::default()
                },
                Release {
                    version: "v0.5.0".into(),
                    stage: "candidate".into(),
                    ..Default::default()
                },
            ],
        );
        let p = read(&store, &goals(&["G-1"]));
        assert!(p.releases_on);
        let order: Vec<(&str, StopState)> = p
            .route
            .iter()
            .map(|s| (s.version.as_str(), s.state))
            .collect();
        assert_eq!(
            order,
            [
                ("v0.3.0", StopState::Shipped),
                ("v0.2.1", StopState::Shipped),
                ("v0.5.0", StopState::Next)
            ]
        );
        assert_eq!(p.route[1].patch_of, "v0.2.0");
        assert_eq!((p.route[0].items, p.route[1].items), (1, 2));
        assert_eq!(p.route[0].shipped_at, "2026-10-01T09:00:00Z");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-4: the next release carries only a date the person set.
    #[test]
    fn an_undated_next_release_carries_no_date() {
        let (store, dir) = board(true);
        write_releases(
            &store,
            &[
                shipped("v0.1.0", "2026-10-01T09:00:00Z"),
                Release {
                    version: "v0.2.0".into(),
                    ..Default::default()
                },
            ],
        );
        let p = read(&store, &goals(&[]));
        let next = p.route.last().unwrap();
        assert_eq!(
            (next.version.as_str(), next.state),
            ("v0.2.0", StopState::Next)
        );
        assert_eq!(next.target, "");
        let json = serde_json::to_value(next).unwrap();
        assert!(json.get("target").is_none(), "{json}");

        write_releases(
            &store,
            &[Release {
                version: "v0.2.0".into(),
                target: "2026-12-18".into(),
                ..Default::default()
            }],
        );
        let p = read(&store, &goals(&[]));
        assert_eq!(p.route[0].target, "2026-12-18");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-4: a cell counts what `/items?goal=&release=` lists.
    #[test]
    fn goal_marks_per_release_count_what_the_items_list_shows() {
        let (store, dir) = board(true);
        item(&store, "a", &["G-1"], &[None], Some("v0.1.0"));
        item(&store, "b", &["G-1", "G-2"], &[None], Some("v0.1.0"));
        item(&store, "c", &["G-2"], &[None], Some("v0.2.0"));
        item(&store, "d", &["G-1"], &[None], None);
        write_releases(
            &store,
            &[
                shipped("v0.1.0", "2026-10-01T09:00:00Z"),
                shipped("v0.2.0", "2026-10-09T09:00:00Z"),
            ],
        );
        let p = read(&store, &goals(&["G-1", "G-2"]));
        for goal in ["G-1", "G-2"] {
            for release in ["v0.1.0", "v0.2.0"] {
                let listed = crate::items::list(
                    &store,
                    "demo",
                    &crate::items::ItemFilter {
                        goal: Some(goal.into()),
                        release: Some(release.into()),
                        ..Default::default()
                    },
                    None,
                )
                .unwrap()
                .len();
                let marked = p
                    .marks
                    .iter()
                    .find(|m| m.goal == goal && m.release == release)
                    .map_or(0, |m| m.items);
                assert_eq!(marked, listed, "{goal} in {release}");
            }
        }
        assert!(
            p.marks.iter().all(|m| m.items > 0),
            "only non-zero cells: {:?}",
            p.marks
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-5: three releases shipping 1, 0 and 2 items give cumulative points, and a red
    /// test lowers today's proven.
    #[test]
    fn goal_series_rises_with_each_release_from_todays_evidence() {
        let (store, dir) = board(true);
        item(
            &store,
            "a",
            &["G-1"],
            &[Some(TestState::Green)],
            Some("v0.1.0"),
        );
        item(
            &store,
            "b",
            &["G-1"],
            &[Some(TestState::Green), Some(TestState::Green)],
            Some("v0.3.0"),
        );
        let red = item(
            &store,
            "c",
            &["G-1"],
            &[Some(TestState::Red)],
            Some("v0.3.0"),
        );
        write_releases(
            &store,
            &[
                shipped("v0.1.0", "2026-10-01T09:00:00Z"),
                shipped("v0.2.0", "2026-10-09T09:00:00Z"),
                shipped("v0.3.0", "2026-10-20T09:00:00Z"),
            ],
        );
        let p = read(&store, &goals(&["G-1"]));
        let points: Vec<(&str, usize, usize, usize)> = p.series[0]
            .points
            .iter()
            .map(|x| (x.release.as_str(), x.items, x.req_proven, x.req_total))
            .collect();
        assert_eq!(
            points,
            [
                ("v0.1.0", 1, 1, 1),
                ("v0.2.0", 1, 1, 1),
                ("v0.3.0", 3, 3, 4)
            ]
        );
        // A test turning green later moves the point it belongs to.
        let mut f = store.load("demo").unwrap().feature(&red).unwrap().clone();
        f.definition.as_mut().unwrap().requirements[0].tests[0].state = TestState::Green;
        store.persist_feature_for_test("demo", &f).unwrap();
        let p = read(&store, &goals(&["G-1"]));
        assert_eq!(p.series[0].points[2].req_proven, 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-8: seven finished items, one serving no goal: five wins, newest first.
    #[test]
    fn recent_wins_are_the_latest_finished_items_that_serve_a_goal() {
        let (store, dir) = board(false);
        let mut codes = Vec::new();
        for day in 1..=7 {
            let goals: &[&str] = if day == 7 { &[] } else { &["G-1"] };
            let c = item(&store, &format!("Item {day}"), goals, &[None], None);
            finish_at(&store, &c, &format!("2026-11-{day:02}T10:00:00Z"));
            codes.push(c);
        }
        item(&store, "Still open", &["G-1"], &[None], None);
        let p = read(&store, &goals(&["G-1"]));
        let wins: Vec<&str> = p.wins.iter().map(|w| w.code.as_str()).collect();
        let want: Vec<&str> = [6, 5, 4, 3, 2]
            .iter()
            .map(|d| codes[d - 1].as_str())
            .collect();
        assert_eq!(wins, want);
        assert_eq!(p.wins[0].goals, ["G-1"]);
        assert_eq!(p.finished.len(), 6, "every finished item serving a goal");
        assert_eq!(p.finished[0].at, "2026-11-06T10:00:00Z");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-9: a project with nothing in it answers with empty sections, not an error.
    #[test]
    fn a_fresh_project_has_empty_sections_not_errors() {
        let (store, dir) = board(true);
        let p = charter_progress(&store, "demo").unwrap();
        assert_eq!(p.project, "demo");
        assert!(p.goals.is_empty());
        assert!(p.route.is_empty() && p.marks.is_empty() && p.series.is_empty());
        assert!(p.wins.is_empty() && p.finished.is_empty());
        assert_eq!(p.lessons, LessonGrowth::default());

        let (store, dir2) = board(false);
        let p = charter_progress(&store, "demo").unwrap();
        assert!(!p.releases_on);
        assert!(charter_progress(&store, "nope").is_err());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    /// FTR-076 R-6: the counts are the lessons list's: `kanbanr lessons` for in force and
    /// `--all` for the total, with one retired and one decayed lesson.
    #[test]
    fn lesson_counts_match_the_lessons_list() {
        let (store, dir) = board(false);
        let all = vec![
            lesson("L-1", "2026-10-01T00:00:00Z", |_| {}),
            lesson("L-2", "2026-10-02T00:00:00Z", |l| {
                l.status = LessonStatus::Retired;
            }),
            lesson("L-3", "2025-01-01T00:00:00Z", |l| {
                l.last_affirmed = "2025-01-01T00:00:00Z".into();
            }),
            lesson("L-4", "2026-10-04T00:00:00Z", |l| l.confidence = 0.5),
        ];
        std::fs::write(
            store.project_dir("demo").join("lessons.yaml"),
            crate::yaml::to_string(&all).unwrap(),
        )
        .unwrap();
        let p = charter_progress(&store, "demo").unwrap();
        let surfaced = crate::lessons::surfaced(&store, "demo", None).unwrap();
        let every = crate::lessons::load(&store, "demo").unwrap();
        assert_eq!(p.lessons.in_force, surfaced.len());
        assert_eq!(p.lessons.total, every.len());
        assert_eq!(
            (p.lessons.in_force, p.lessons.total, p.lessons.retired),
            (2, 4, 2)
        );
        // In force first, most believed first; the retired and the decayed after.
        let ids: Vec<&str> = p.lessons.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids[..2], ["L-1", "L-4"]);
        assert!(p.lessons.rows[..2].iter().all(|r| r.in_force));
        assert!(p.lessons.rows[2..].iter().all(|r| !r.in_force));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-6: an id counts only as a whole token.
    #[test]
    fn a_citation_is_a_whole_token_never_a_substring() {
        for no in ["L-23", "FL-2", "XL-2x", "a_L-2", "x-L-2"] {
            assert!(!cites(no, "L-2"), "{no}");
        }
        for yes in [
            "(L-2)",
            "L-2.",
            "L-2,",
            "L-2",
            "see L-2 here",
            "lesson L-2: x",
            "[L-2]",
        ] {
            assert!(cites(yes, "L-2"), "{yes}");
        }
        assert!(
            cites("L-23 and L-2", "L-2"),
            "a later whole token still counts"
        );

        let (store, dir) = board(false);
        let source = item(&store, "Source", &["G-1"], &[None], None);
        let citing = item(&store, "Cites", &["G-1"], &[None], None);
        let near = item(&store, "Near miss", &["G-1"], &[None], None);
        for (code, spec) in [
            (&source, "applied L-2 itself"),
            (&citing, "We follow L-2."),
            (&near, "We follow L-23 only."),
        ] {
            let mut f = store.load("demo").unwrap().feature(code).unwrap().clone();
            f.specification = spec.into();
            store.persist_feature_for_test("demo", &f).unwrap();
        }
        let l = lesson("L-2", "2026-10-01T00:00:00Z", |l| {
            l.from_item = source.clone()
        });
        let p = read_with(&store, &goals(&["G-1"]), &[l]);
        assert_eq!(p.lessons.rows[0].cited_by, [citing]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-6: a lesson lands in the cycle of the first release shipped on or after it, and
    /// the counts are cumulative.
    #[test]
    fn lessons_fall_into_the_release_cycle_they_were_learned_in() {
        let (store, dir) = board(true);
        write_releases(
            &store,
            &[
                shipped("v0.1.0", "2026-10-01T09:00:00Z"),
                shipped("v0.2.0", "2026-10-10T09:00:00Z"),
            ],
        );
        let lessons = [
            lesson("L-1", "2026-09-20T00:00:00Z", |_| {}),
            lesson("L-2", "2026-10-05T00:00:00Z", |_| {}),
            lesson("L-3", "2026-10-06T00:00:00Z", |_| {}),
            lesson("L-4", "2026-10-20T00:00:00Z", |_| {}),
            lesson("L-5", "not a time", |l| l.status = LessonStatus::Retired),
        ];
        let p = read_with(&store, &goals(&[]), &lessons);
        let cycles: Vec<(&str, &str, usize, usize)> = p
            .lessons
            .cycles
            .iter()
            .map(|c| (c.key.as_str(), c.label.as_str(), c.recorded, c.cumulative))
            .collect();
        assert_eq!(
            cycles,
            [
                ("before", "before v0.1.0", 1, 1),
                ("v0.2.0", "v0.2.0", 2, 3),
                ("since", "since v0.2.0", 2, 5),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);

        // Without releases: one cycle per month that holds a lesson, and the gaps between.
        let (store, dir) = board(false);
        let lessons = [
            lesson("L-1", "2026-08-03T00:00:00Z", |_| {}),
            lesson("L-2", "2026-10-09T00:00:00Z", |_| {}),
            lesson("L-3", "2026-10-30T00:00:00Z", |_| {}),
        ];
        let p = read_with(&store, &goals(&[]), &lessons);
        let months: Vec<(&str, usize, usize)> = p
            .lessons
            .cycles
            .iter()
            .map(|c| (c.key.as_str(), c.recorded, c.cumulative))
            .collect();
        assert_eq!(
            months,
            [("2026-08", 1, 1), ("2026-09", 0, 1), ("2026-10", 2, 3)]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-076 R-7: a goal carries the lessons that name it, and each row its goals, retro kind and
    /// themes; the largest tags are kept, the rest fold, the untagged have their own theme.
    #[test]
    fn each_goal_carries_the_lessons_that_name_it() {
        let (store, dir) = board(false);
        store
            .add_milestone("demo", "Wave", "", vec![], Some("MS-9".into()))
            .unwrap();
        std::fs::write(
            store.project_dir("demo").join("sprints.yaml"),
            crate::yaml::to_string(&vec![crate::sprints::Sprint {
                code: "SP-1".into(),
                start: "2026-10-01".into(),
                end: "2026-10-10".into(),
                ..Default::default()
            }])
            .unwrap(),
        )
        .unwrap();
        let mut lessons = vec![
            lesson("L-1", "2026-10-01T00:00:00Z", |l| {
                l.goals = vec!["G-1".into(), "G-2".into()];
                l.tags = vec!["Testing".into(), "board".into()];
                l.from_retro = "SP-1".into();
            }),
            lesson("L-2", "2026-10-02T00:00:00Z", |l| {
                l.goals = vec!["G-1".into()];
                l.tags = vec!["testing".into()];
                l.from_retro = "MS-9".into();
            }),
            lesson("L-3", "2026-10-03T00:00:00Z", |_| {}),
        ];
        // Ten more distinct single-tag lessons push the smallest tags past the eight kept.
        for n in 0..10 {
            lessons.push(lesson(
                &format!("L-{}", 10 + n),
                "2026-10-04T00:00:00Z",
                |l| {
                    l.tags = vec![format!("tag{n:02}")];
                },
            ));
        }
        let p = read_with(&store, &goals(&["G-1", "G-2", "G-3"]), &lessons);
        let of = |id: &str| p.goals.iter().find(|g| g.facts.id == id).unwrap();
        assert_eq!(of("G-1").lessons, ["L-1", "L-2"]);
        assert_eq!(of("G-2").lessons, ["L-1"]);
        assert!(of("G-3").lessons.is_empty());
        let row = |id: &str| p.lessons.rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row("L-1").goals, ["G-1", "G-2"]);
        assert_eq!(row("L-1").retro_kind, "sprint");
        assert_eq!(row("L-2").retro_kind, "milestone");
        assert_eq!(row("L-3").retro_kind, "");

        let keys: Vec<&str> = p.lessons.themes.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys.len(), THEMES_KEPT + 2, "{keys:?}");
        assert_eq!(keys[0], "testing");
        assert_eq!(
            p.lessons.themes[0].label, "Testing",
            "label as first written"
        );
        assert_eq!(p.lessons.themes[0].lessons, ["L-1", "L-2"]);
        assert_eq!(keys[keys.len() - 2], "other");
        assert_eq!(keys[keys.len() - 1], "untagged");
        assert_eq!(p.lessons.themes[keys.len() - 1].lessons, ["L-3"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
