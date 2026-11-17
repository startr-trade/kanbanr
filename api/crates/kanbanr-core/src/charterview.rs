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
    let board_rev = crate::git::head_commit(store.data_dir()).unwrap_or_default();
    Ok(charter_progress_on(
        &project,
        &charter,
        &releases,
        &lessons,
        &board_rev,
        &crate::now_rfc3339(),
    ))
}

/// The pure part of [`charter_progress`]: every unit test calls it.
pub(crate) fn charter_progress_on(
    project: &Project,
    charter: &Charter,
    releases: &[Release],
    _lessons: &[Lesson],
    board_rev: &str,
    now: &str,
) -> CharterProgress {
    let releases_on = project.config.cadence.releases;
    let goals: Vec<GoalProgress> = charter
        .goals
        .iter()
        .map(|g| GoalProgress {
            facts: crate::overview::goal_facts(project, &g.id, &g.statement),
            lessons: Vec::new(),
        })
        .collect();
    let route = if releases_on {
        route(project, releases)
    } else {
        Vec::new()
    };
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
        lessons: LessonGrowth::default(),
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
#[allow(dead_code)] // T2 uses it
pub(crate) fn cites(_text: &str, _id: &str) -> bool {
    todo!()
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
}
