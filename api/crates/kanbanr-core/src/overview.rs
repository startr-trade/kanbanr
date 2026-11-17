//! The Project Overview (FTR-048): what needs the person, what is happening now, how much the
//! board holds, the goals, recent decisions and activity, in one read.
//!
//! Derived from the board on each read and never stored (ADR-0002, ADR-0005). It takes no
//! verdicts and only links to where they are taken (ADR-0016, ADR-0019). Every number comes from
//! the rule that already counts it: coverage and flow from [`crate::report::run`] itself, a goal's
//! proof from [`crate::report::item_evidence`], what needs the person from
//! [`crate::attention::attention`].
//!
//! A board read error fails the whole read (an unknown project is 404, a newer board 409). Every
//! other source fails soft: its part is left empty and a Health line in `attention` says what
//! could not be read, so nothing fails silently.

use crate::attention::{Attention, AttentionLine, Section};
use crate::error::Result;
use crate::items::normal_kind;
use crate::releases::{Release, ReleaseState};
use crate::report::item_evidence;
use crate::store::Project;
use crate::{FeatureItem, Store};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// How many recent decisions and activity lines the Overview shows; sized so the page fits a
/// 1440×900 screen above Goals.
pub const RECENT: usize = 8;
pub const ACTIVITY: usize = 8;
/// A title in `recent` is cut at this many characters, with "…".
const TITLE_CUT: usize = 60;

/// The whole Overview of one project.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Overview {
    pub project: String,
    /// The board's HEAD commit the numbers were read at; empty when the board has no commit.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub board_rev: String,
    /// When it was read, RFC 3339.
    pub generated_at: String,
    pub attention: Attention,
    pub now: Now,
    pub holds: Holds,
    /// In charter order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<GoalFacts>,
    /// Newest first, at most [`RECENT`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<Recent>,
    /// Newest first, at most [`ACTIVITY`], in plain words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ActivityLine>,
}

/// What is happening now: the running sprint, the releases in flight, the milestones under way.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Now {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint: Option<SprintNow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<ReleaseNow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<MilestoneNow>,
}

/// The running sprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SprintNow {
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub goal: String,
    pub start: String,
    pub end: String,
    /// Days to its last day, that day included; never below 0.
    pub days_left: i64,
    /// Items planned into it.
    pub committed: usize,
    /// Of those, the ones finished by the cadence rule (an end status, or a stage marked done).
    pub done: usize,
}

/// A release in flight, as a stage stepper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseNow {
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    pub waits_on_tag: bool,
    pub steps: Vec<StepFact>,
    /// Items planned into it or shipped in it.
    pub items: usize,
    /// Of those, the ones finished.
    pub finished: usize,
}

/// One stage of a release: done (with when and by whom, where recorded), current, or next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StepFact {
    pub key: String,
    pub label: String,
    /// `done`, `current` or `next`.
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
}

/// A milestone under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MilestoneNow {
    pub code: String,
    pub name: String,
    pub done: usize,
    pub total: usize,
}

/// How much the board holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Holds {
    pub items: ItemCounts,
    pub requirements: RequirementCounts,
    pub tests: TestCounts,
    pub coverage: CoverageCounts,
    pub flow: FlowCounts,
    pub architecture: ArchCounts,
    pub records: RecordCounts,
}

/// Items by kind, as recorded: lower-cased and trimmed, no kind mapped onto another.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemCounts {
    pub total: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_kind: BTreeMap<String, usize>,
    /// Items with no kind, or an empty one.
    pub untyped: usize,
}

/// Requirements by kind, EARS pattern and ISO 25010 tagging.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RequirementCounts {
    pub total: usize,
    pub functional: usize,
    pub nfr: usize,
    /// By the EARS pattern the requirement's text classifies as (`ubiquitous`, `event`, `state`,
    /// `optional`, `unwanted`); a requirement that conforms to none is not counted here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_pattern: BTreeMap<String, usize>,
    /// Requirements naming at least one ISO/IEC 25010 characteristic.
    pub iso_tagged: usize,
}

/// Tests by state (`planned`, `red`, `green`) and kind (`none` when no kind is named).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TestCounts {
    pub total: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_state: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_kind: BTreeMap<String, usize>,
}

/// Requirement coverage, as the flow report counts it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CoverageCounts {
    pub proven: usize,
    pub total: usize,
    /// The distinct items among the report's stale evidence (its entries are `CODE/R-n`, one per
    /// requirement whose green predates the code repository's HEAD), as `?evidence=stale` lists
    /// them; 0 without a code repository.
    pub stale_items: usize,
}

/// Flow, as the flow report counts it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FlowCounts {
    pub completed: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_p50_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_p90_days: Option<f64>,
    pub rework: usize,
    pub defects: usize,
    pub escaped: usize,
}

/// Architecture records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ArchCounts {
    /// ADRs by status.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adrs: BTreeMap<String, usize>,
    /// Board docs under `design/`, outside `design/components/`.
    pub design_docs: usize,
    /// Board docs under `design/components/`.
    pub component_docs: usize,
}

/// Other records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RecordCounts {
    /// Board docs under `retros/`.
    pub retros: usize,
    /// Board docs under `releases/`.
    pub release_notes: usize,
    pub lessons: usize,
    pub releases: usize,
}

/// One charter goal, with the work serving it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GoalFacts {
    pub id: String,
    /// The goal's statement.
    pub title: String,
    pub items_done: usize,
    pub items_open: usize,
    pub req_proven: usize,
    pub req_total: usize,
}

/// One recent decision, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Recent {
    pub at: String,
    /// `adr`, `approval`, `signoff`, `ratify`, `release-stage` or `shipped`.
    pub kind: String,
    /// The item or ADR it is about; absent for a release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub words: String,
    /// The monitor page that shows it.
    pub to: String,
}

/// One activity entry, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActivityLine {
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub words: String,
}

/// The Overview of project `p`. `repo` is the code repository, when the server has one: the
/// review groups and git doctor in `attention` need it, and staleness is judged against its HEAD.
pub fn overview(store: &Store, p: &str, repo: Option<&Path>) -> Result<Overview> {
    overview_on(store, p, repo, crate::sprints::today())
}

/// [`overview`] as it reads on `today`.
pub(crate) fn overview_on(
    store: &Store,
    p: &str,
    repo: Option<&Path>,
    today: time::Date,
) -> Result<Overview> {
    // The one read that fails the whole answer: an unknown project, a newer board.
    let project = store.load_meta(p)?;
    let mut attention = crate::attention::attention(store, p, repo)?;
    let mut failed = Vec::new();
    let head = repo.and_then(|r| crate::coderepo::rev_parse(r, "HEAD").ok());

    let releases = soft(&mut failed, "releases", crate::releases::load(store, p));
    let sprints = soft(&mut failed, "sprints", crate::sprints::load(store, p));
    let docs = soft(&mut failed, "docs", store.list_docs(p));
    let adrs = soft(&mut failed, "decisions", crate::adr::list(store, p));
    let lessons = soft(&mut failed, "lessons", crate::lessons::load(store, p));
    let charter = soft(&mut failed, "charter", crate::charter::load(store, p));
    let report =
        match crate::report::run(store, p, &crate::report::Window::default(), head.as_deref()) {
            Ok(r) => Some(r),
            Err(e) => {
                failed.push(could_not_read("the flow report", &e));
                None
            }
        };

    let now = Now {
        sprint: sprint_now(&project, &sprints, today),
        releases: releases
            .iter()
            .filter(|r| in_flight(r))
            .map(|r| release_now(&project, r))
            .collect(),
        milestones: milestones_now(&project),
    };
    let holds = holds(&project, &docs, &adrs, &lessons, &releases, report.as_ref());
    let goals = charter
        .goals
        .iter()
        .map(|g| goal_facts(&project, &g.id, &g.statement))
        .collect();
    let recent = recent(&project, &adrs, &releases, p);
    let activity = activity(store, &project, p);

    attention.lines.extend(failed);
    // The read-error lines are Health lines: keep the lines in section order.
    attention.lines.sort_by_key(|l| l.section);

    Ok(Overview {
        project: p.to_string(),
        board_rev: crate::git::head_commit(store.data_dir()).unwrap_or_default(),
        generated_at: crate::now_rfc3339(),
        attention,
        now,
        holds,
        goals,
        recent,
        activity,
    })
}

/// A source's value, or its default with a Health line saying it could not be read.
fn soft<T: Default>(failed: &mut Vec<AttentionLine>, source: &str, r: Result<T>) -> T {
    r.unwrap_or_else(|e| {
        failed.push(could_not_read(source, &e));
        T::default()
    })
}

/// The Health line that stands in for a source that failed (FEAT-084: nothing fails silently),
/// worded as [`crate::attention`]'s.
fn could_not_read(source: &str, e: &crate::CoreError) -> AttentionLine {
    AttentionLine {
        kind: "read-error".into(),
        section: Section::Health,
        label: format!("Could not read {source}: {e}"),
        count: 0,
        to: None,
        ask: None,
        tone: "warn".into(),
        yours: false,
    }
}

/// A release the Overview shows as in flight: the ones attention lists under Releases.
fn in_flight(r: &Release) -> bool {
    r.waits_on_tag()
        || (!r.stage.is_empty() && r.state == ReleaseState::Planned && r.superseded_by.is_empty())
}

fn done(project: &Project, f: &FeatureItem) -> bool {
    project.config.counts_as_done(&f.status)
}

fn sprint_now(
    project: &Project,
    sprints: &[crate::sprints::Sprint],
    today: time::Date,
) -> Option<SprintNow> {
    let s = crate::sprints::active_on(sprints, today)?;
    let days_left = crate::gantt::parse_ymd(&s.end)
        .map(|end| ((end - today).whole_days() + 1).max(0))
        .unwrap_or(0);
    let committed: Vec<&FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.sprint.as_deref() == Some(s.code.as_str()))
        .collect();
    Some(SprintNow {
        code: s.code.clone(),
        name: s.name.clone(),
        goal: s.goal.clone(),
        start: s.start.clone(),
        end: s.end.clone(),
        days_left,
        committed: committed.len(),
        done: committed.iter().filter(|f| done(project, f)).count(),
    })
}

fn release_now(project: &Project, r: &Release) -> ReleaseNow {
    // What it holds, by releases::report_all's rule.
    let items: Vec<&FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.release.as_deref() == Some(r.version.as_str()) || r.shipped.contains(&f.code))
        .collect();
    ReleaseNow {
        version: r.version.clone(),
        name: r.name.clone(),
        stage: r.stage.clone(),
        waits_on_tag: r.waits_on_tag(),
        steps: steps(&project.config.cadence.stages(), r),
        items: items.len(),
        finished: items.iter().filter(|f| done(project, f)).count(),
    }
}

/// The stepper's steps, by the rule of the monitor's `stepsFromStages` (FTR-046): stages before
/// the current one are done, with the `at` and `by` of the latest move into each; the current one
/// is current; the rest are next. A stage that names none of `stages` leaves every stage next.
fn steps(stages: &[String], r: &Release) -> Vec<StepFact> {
    let at = stages.iter().position(|s| *s == r.stage);
    stages
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let (state, reached) = match at {
                Some(at) if i < at => (
                    "done",
                    r.stages
                        .iter()
                        .filter(|c| c.stage == *key)
                        .max_by(|a, b| a.at.cmp(&b.at)),
                ),
                Some(at) if i == at => ("current", None),
                _ => ("next", None),
            };
            StepFact {
                key: key.clone(),
                label: capitalised(key),
                state: state.into(),
                at: reached.map(|c| c.at.clone()).unwrap_or_default(),
                by: reached.map(|c| c.by.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The milestones under way: at least one item not done and at least one in a displayed status,
/// in the board's milestone order.
fn milestones_now(project: &Project) -> Vec<MilestoneNow> {
    project
        .milestones
        .iter()
        .filter_map(|m| {
            let items: Vec<&FeatureItem> = project
                .features
                .iter()
                .filter(|f| f.milestone == m.code)
                .collect();
            let finished = items.iter().filter(|f| done(project, f)).count();
            let shown = items
                .iter()
                .any(|f| project.config.displayed_states.contains(&f.status));
            (finished < items.len() && shown).then(|| MilestoneNow {
                code: m.code.clone(),
                name: m.name.clone(),
                done: finished,
                total: items.len(),
            })
        })
        .collect()
}

fn holds(
    project: &Project,
    docs: &[String],
    adrs: &[crate::adr::Adr],
    lessons: &[crate::lessons::Lesson],
    releases: &[Release],
    report: Option<&crate::report::Report>,
) -> Holds {
    let mut items = ItemCounts {
        total: project.features.len(),
        ..Default::default()
    };
    let mut requirements = RequirementCounts::default();
    let mut tests = TestCounts::default();
    for f in &project.features {
        match normal_kind(f.kind.as_deref()) {
            Some(k) => *items.by_kind.entry(k).or_default() += 1,
            None => items.untyped += 1,
        }
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            requirements.total += 1;
            match r.kind {
                crate::models::RequirementKind::Functional => requirements.functional += 1,
                crate::models::RequirementKind::Nfr => requirements.nfr += 1,
            }
            if let Some(pattern) = crate::ears::classify(&r.text) {
                *requirements
                    .by_pattern
                    .entry(pattern.as_str().to_string())
                    .or_default() += 1;
            }
            if r.iso.iter().any(|i| !i.trim().is_empty()) {
                requirements.iso_tagged += 1;
            }
            for t in &r.tests {
                tests.total += 1;
                let state = match t.state {
                    crate::models::TestState::Planned => "planned",
                    crate::models::TestState::Red => "red",
                    crate::models::TestState::Green => "green",
                };
                *tests.by_state.entry(state.into()).or_default() += 1;
                let kind = t.kind.trim().to_lowercase();
                let kind = if kind.is_empty() { "none".into() } else { kind };
                *tests.by_kind.entry(kind).or_default() += 1;
            }
        }
    }

    // Coverage and flow are the report's own numbers, never recomputed.
    let (coverage, flow) = match report {
        Some(r) => (
            CoverageCounts {
                proven: r.requirement_coverage.proven,
                total: r.requirement_coverage.total,
                stale_items: r
                    .stale_evidence
                    .iter()
                    .map(|e| e.split_once('/').map_or(e.as_str(), |(code, _)| code))
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
            },
            FlowCounts {
                completed: r.completed,
                cycle_p50_days: r.cycle_time_days.as_ref().map(|c| c.p50),
                cycle_p90_days: r.cycle_time_days.as_ref().map(|c| c.p90),
                rework: r.rework,
                defects: r.defects,
                escaped: r.escaped_defects,
            },
        ),
        None => Default::default(),
    };

    let mut architecture = ArchCounts::default();
    for a in adrs {
        *architecture.adrs.entry(a.status.clone()).or_default() += 1;
    }
    let under = |folder: &str| docs.iter().filter(|d| d.starts_with(folder)).count();
    architecture.component_docs = under("design/components/");
    architecture.design_docs = under("design/") - architecture.component_docs;

    Holds {
        items,
        requirements,
        tests,
        coverage,
        flow,
        architecture,
        records: RecordCounts {
            retros: under("retros/"),
            release_notes: under("releases/"),
            lessons: lessons.len(),
            releases: releases.len(),
        },
    }
}

pub(crate) fn goal_facts(project: &Project, id: &str, statement: &str) -> GoalFacts {
    let mut g = GoalFacts {
        id: id.to_string(),
        title: statement.to_string(),
        items_done: 0,
        items_open: 0,
        req_proven: 0,
        req_total: 0,
    };
    for f in project.features.iter().filter(|f| {
        f.definition
            .as_ref()
            .is_some_and(|d| d.goals.iter().any(|x| x == id))
    }) {
        if done(project, f) {
            g.items_done += 1;
        } else {
            g.items_open += 1;
        }
        let e = item_evidence(f, None);
        g.req_proven += e.req_proven;
        g.req_total += e.req_total;
    }
    g
}

/// A title cut at [`TITLE_CUT`] characters, with "…".
fn cut(title: &str) -> String {
    if title.chars().count() <= TITLE_CUT {
        title.to_string()
    } else {
        let mut s: String = title.chars().take(TITLE_CUT).collect();
        s.push('…');
        s
    }
}

fn on_the_monitor(origin: &str) -> &'static str {
    if origin == "monitor" {
        " on the monitor"
    } else {
        ""
    }
}

/// ADRs, verdicts on items and release moves, newest first, at most [`RECENT`].
fn recent(
    project: &Project,
    adrs: &[crate::adr::Adr],
    releases: &[Release],
    p: &str,
) -> Vec<Recent> {
    let mut out = Vec::new();
    for a in adrs.iter().filter(|a| !a.date.is_empty()) {
        out.push(Recent {
            at: a.date.clone(),
            kind: "adr".into(),
            code: Some(a.id.clone()),
            words: format!("{} {}: {}", a.id, a.status, cut(&a.title)),
            to: format!("/p/{p}/docs/file/{}", a.path),
        });
    }
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        let item = |at: &str, kind: &str, what: String| Recent {
            at: at.to_string(),
            kind: kind.into(),
            code: Some(f.code.clone()),
            words: format!("{} {} {what}", f.code, cut(&f.title)),
            to: format!("/p/{p}/feature/{}", f.code),
        };
        if def.approvals.is_empty() {
            // A record from before the approval history: the current approval alone.
            if let Some(a) = &def.approval {
                out.push(item(
                    &a.at,
                    "approval",
                    format!("approved{}", on_the_monitor(&a.origin)),
                ));
            }
        }
        for e in &def.approvals {
            let (kind, word) = match e.verdict.as_str() {
                "approved" => ("approval", "approved"),
                "ratified" => ("ratify", "ratified"),
                _ => continue,
            };
            out.push(item(
                &e.at,
                kind,
                format!("{word}{}", on_the_monitor(&e.origin)),
            ));
        }
        for s in def
            .signoffs
            .iter()
            .filter(|s| s.verdict == crate::models::SignoffVerdict::Given)
        {
            out.push(item(
                &s.at,
                "signoff",
                format!("signed off {}{}", s.id, on_the_monitor(&s.origin)),
            ));
        }
    }
    for r in releases {
        let to = format!("/p/{p}/releases?tab=current#{}", r.version);
        for c in &r.stages {
            let words = if c.undoes.is_empty() {
                format!("{} promoted to {}", r.version, c.stage)
            } else {
                format!("{} moved back to {}", r.version, c.stage)
            };
            out.push(Recent {
                at: c.at.clone(),
                kind: "release-stage".into(),
                code: None,
                words: format!("{words}{}", on_the_monitor(&c.origin)),
                to: to.clone(),
            });
        }
        if !r.shipped_at.is_empty() {
            out.push(Recent {
                at: r.shipped_at.clone(),
                kind: "shipped".into(),
                code: None,
                words: format!("{} shipped{}", r.version, on_the_monitor(&r.shipped_origin)),
                to: to.clone(),
            });
        }
    }
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out.truncate(RECENT);
    out
}

/// The last [`ACTIVITY`] activity entries, in plain words.
fn activity(store: &Store, project: &Project, p: &str) -> Vec<ActivityLine> {
    let title_of = |code: &str| project.feature(code).ok().map(|f| f.title.clone());
    crate::activity::read_filtered(
        store,
        p,
        &crate::activity::Filter {
            limit: Some(ACTIVITY),
            ..Default::default()
        },
    )
    .into_iter()
    .map(|e| {
        let (item, words) = crate::activity::humanise(&e.message, title_of);
        ActivityLine {
            at: e.time,
            item: item.or(e.item),
            words,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{ApprovalEvent, FeatureDefinition, Requirement, TestRef, TestState};
    use crate::releases::StageChange;

    /// A board in its own folder under the temp dir (L-41), checked before the first write, with
    /// one milestone `M`.
    fn board(sprints: bool, releases: bool) -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-overview-{}-{}",
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
        config.cadence.sprints = sprints;
        config.cadence.releases = releases;
        config.cadence.release_stages = vec!["candidate".into(), "main".into(), "tag".into()];
        store.init_project("demo", config).unwrap();
        assert_eq!(std::fs::canonicalize(store.data_dir()).unwrap(), root);
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    fn day(s: &str) -> time::Date {
        crate::gantt::parse_ymd(s).unwrap()
    }

    /// An item in milestone `ms`, changed by `edit` and written back.
    fn item(store: &Store, ms: &str, title: &str, edit: impl FnOnce(&mut FeatureItem)) -> String {
        let f = store.add_feature("demo", title, "", ms, None).unwrap();
        let mut f = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        edit(&mut f);
        store.persist_feature_for_test("demo", &f).unwrap();
        f.code
    }

    fn finish(store: &Store, code: &str) {
        store.move_feature("demo", code, "Scheduled").unwrap();
        store.move_feature("demo", code, "Completed").unwrap();
    }

    fn write_releases(store: &Store, all: &[Release]) {
        std::fs::write(
            store.project_dir("demo").join("releases.yaml"),
            crate::yaml::to_string(&all).unwrap(),
        )
        .unwrap();
    }

    fn requirement(id: &str, tests: Vec<TestRef>) -> Requirement {
        Requirement {
            id: id.into(),
            text: "THE SYSTEM SHALL hold.".into(),
            tests,
            ..Default::default()
        }
    }

    fn test_in(state: TestState) -> TestRef {
        TestRef {
            name: format!("t_{state:?}").to_lowercase(),
            state,
            ..Default::default()
        }
    }

    /// FTR-048 R-5: the running sprint with its days left, committed and done; a release in
    /// flight with its steps done (when and by whom), current and next; a milestone half done is
    /// listed and a finished one is not.
    #[test]
    fn now_shows_the_running_sprint_the_release_in_flight_and_open_milestones() {
        let (store, dir) = board(true, true);
        std::fs::write(
            store.project_dir("demo").join("sprints.yaml"),
            crate::yaml::to_string(&vec![crate::sprints::Sprint {
                code: "SP-001".into(),
                name: "First".into(),
                goal: "Ship it".into(),
                start: "2026-11-10".into(),
                end: "2026-11-20".into(),
                state: crate::sprints::SprintState::Active,
                ..Default::default()
            }])
            .unwrap(),
        )
        .unwrap();
        let a = item(&store, "M", "In the sprint, done", |f| {
            f.sprint = Some("SP-001".into());
            f.release = Some("v0.8.0".into());
        });
        finish(&store, &a);
        item(&store, "M", "In the sprint, open", |f| {
            f.sprint = Some("SP-001".into());
            f.release = Some("v0.8.0".into());
        });
        item(&store, "M", "Not in the sprint", |_| {});
        store
            .add_milestone("demo", "Done wave", "", vec![], Some("MS-2".into()))
            .unwrap();
        let d = item(&store, "MS-2", "Finished", |_| {});
        finish(&store, &d);
        write_releases(
            &store,
            &[
                Release {
                    version: "v0.8.0".into(),
                    name: "Eight".into(),
                    stage: "main".into(),
                    stages: vec![
                        StageChange {
                            stage: "candidate".into(),
                            by: "Alice".into(),
                            at: "2026-11-14T10:00:00Z".into(),
                            ..Default::default()
                        },
                        StageChange {
                            stage: "main".into(),
                            by: "Bob".into(),
                            at: "2026-11-16T10:00:00Z".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                // Not promoted yet: not in flight.
                Release {
                    version: "v0.9.0".into(),
                    ..Default::default()
                },
            ],
        );

        let o = overview_on(&store, "demo", None, day("2026-11-17")).unwrap();
        let s = o.now.sprint.as_ref().expect("a running sprint");
        assert_eq!(s.code, "SP-001");
        assert_eq!(s.name, "First");
        assert_eq!(s.goal, "Ship it");
        assert_eq!(s.days_left, 4, "17 to 20 November, both included");
        assert_eq!((s.committed, s.done), (2, 1));

        assert_eq!(o.now.releases.len(), 1, "{:#?}", o.now.releases);
        let r = &o.now.releases[0];
        assert_eq!(r.version, "v0.8.0");
        assert_eq!(r.stage, "main");
        assert!(!r.waits_on_tag);
        assert_eq!((r.items, r.finished), (2, 1));
        let steps: Vec<(&str, &str, &str, &str)> = r
            .steps
            .iter()
            .map(|s| {
                (
                    s.key.as_str(),
                    s.state.as_str(),
                    s.at.as_str(),
                    s.by.as_str(),
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                ("candidate", "done", "2026-11-14T10:00:00Z", "Alice"),
                ("main", "current", "", ""),
                ("tag", "next", "", ""),
            ]
        );
        assert_eq!(r.steps[0].label, "Candidate");

        assert_eq!(
            o.now.milestones,
            [MilestoneNow {
                code: "M".into(),
                name: "M".into(),
                done: 1,
                total: 3
            }]
        );

        // The day after the sprint's last day: nothing is running.
        let later = overview_on(&store, "demo", None, day("2026-11-21")).unwrap();
        assert!(later.now.sprint.is_none() || later.now.sprint.as_ref().unwrap().days_left == 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-5: with sprints and releases off, Now is empty, not an error.
    #[test]
    fn with_no_sprint_or_release_now_is_empty_not_an_error() {
        let (store, dir) = board(false, false);
        item(&store, "M", "Something", |_| {});
        let o = overview(&store, "demo", None).unwrap();
        assert_eq!(o.now.sprint, None);
        assert_eq!(o.now.releases, Vec::new());
        assert_eq!(o.project, "demo");
        assert!(
            o.attention.lines.iter().all(|l| l.kind != "read-error"),
            "{:#?}",
            o.attention
        );
        assert!(
            overview(&store, "nope", None).is_err(),
            "an unknown project"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-6: coverage and flow are the flow report's own numbers on the same board.
    #[test]
    fn the_board_holds_counts_match_the_report() {
        let (store, dir) = board(false, false);
        let proven = item(&store, "M", "Proven", |f| {
            f.definition = Some(FeatureDefinition {
                requirements: vec![
                    requirement("R-1", vec![test_in(TestState::Green)]),
                    requirement("R-2", vec![test_in(TestState::Red)]),
                ],
                ..Default::default()
            });
        });
        finish(&store, &proven);
        item(&store, "M", "A defect", |f| {
            f.kind = Some("defect".into());
            f.defect = Some(crate::models::Defect {
                escaped: true,
                ..Default::default()
            });
        });
        item(&store, "M", "Untested", |f| {
            f.definition = Some(FeatureDefinition {
                requirements: vec![requirement("R-1", vec![])],
                ..Default::default()
            });
        });

        let o = overview(&store, "demo", None).unwrap();
        let r =
            crate::report::run(&store, "demo", &crate::report::Window::default(), None).unwrap();
        let c = &o.holds.coverage;
        assert_eq!(
            (c.proven, c.total, c.stale_items),
            (
                r.requirement_coverage.proven,
                r.requirement_coverage.total,
                0
            )
        );
        assert_eq!((c.proven, c.total), (1, 3));
        let f = &o.holds.flow;
        assert_eq!(f.completed, r.completed);
        assert_eq!(f.cycle_p50_days, r.cycle_time_days.as_ref().map(|c| c.p50));
        assert_eq!(f.cycle_p90_days, r.cycle_time_days.as_ref().map(|c| c.p90));
        assert_eq!(f.rework, r.rework);
        assert_eq!((f.defects, f.escaped), (r.defects, r.escaped_defects));
        assert_eq!((f.completed, f.defects, f.escaped), (1, 1, 1));

        // Requirements and tests, counted from the definitions.
        assert_eq!(o.holds.requirements.total, 3);
        assert_eq!(o.holds.requirements.functional, 3);
        assert_eq!(o.holds.requirements.by_pattern.get("ubiquitous"), Some(&3));
        assert_eq!(o.holds.tests.total, 2);
        assert_eq!(o.holds.tests.by_state.get("green"), Some(&1));
        assert_eq!(o.holds.tests.by_state.get("red"), Some(&1));

        // Stale evidence is counted by item, as its name and the ?evidence=stale tile read: one
        // item whose two greens both predate the code repository's HEAD is one stale item.
        let old_green = || TestRef {
            checked_rev: "0000000".into(),
            ..test_in(TestState::Green)
        };
        item(&store, "M", "Stale twice", |f| {
            f.definition = Some(FeatureDefinition {
                requirements: vec![
                    requirement("R-1", vec![old_green()]),
                    requirement("R-2", vec![old_green()]),
                ],
                ..Default::default()
            });
        });
        let repo = crate::coderepo::tests::scratch_repo("overview-stale");
        let head = crate::coderepo::rev_parse(&repo, "HEAD").unwrap();
        let o = overview(&store, "demo", Some(&repo)).unwrap();
        let r = crate::report::run(
            &store,
            "demo",
            &crate::report::Window::default(),
            Some(&head),
        )
        .unwrap();
        assert_eq!(r.stale_evidence.len(), 2, "{:?}", r.stale_evidence);
        let items: std::collections::BTreeSet<&str> = r
            .stale_evidence
            .iter()
            .map(|e| e.split('/').next().unwrap())
            .collect();
        assert_eq!(o.holds.coverage.stale_items, items.len());
        assert_eq!(o.holds.coverage.stale_items, 1);
        let _ = std::fs::remove_dir_all(repo);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-6: kinds are counted as recorded, lower-cased and trimmed; `bug` and `defect`
    /// stay apart and an item with no kind is untyped.
    #[test]
    fn items_by_kind_count_untyped_apart() {
        let (store, dir) = board(false, false);
        for kind in [
            Some("feature"),
            Some("Feature "),
            Some("bug"),
            Some("defect"),
            None,
        ] {
            item(&store, "M", "An item", |f| f.kind = kind.map(String::from));
        }
        let o = overview(&store, "demo", None).unwrap();
        let k = &o.holds.items;
        assert_eq!(k.total, 5);
        assert_eq!(k.by_kind.get("feature"), Some(&2));
        assert_eq!(k.by_kind.get("bug"), Some(&1));
        assert_eq!(k.by_kind.get("defect"), Some(&1));
        assert_eq!(k.by_kind.len(), 3, "{:?}", k.by_kind);
        assert_eq!(k.untyped, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-7: each goal counts the items serving it, done and open, and their requirements
    /// proven of total.
    #[test]
    fn goals_count_items_done_open_and_requirements_proven() {
        let (store, dir) = board(false, false);
        let mut charter = crate::charter::load(&store, "demo").unwrap();
        charter.goals = vec![
            crate::charter::Goal {
                id: "G-1".into(),
                statement: "First goal".into(),
                ..Default::default()
            },
            crate::charter::Goal {
                id: "G-2".into(),
                statement: "Second goal".into(),
                ..Default::default()
            },
        ];
        crate::charter::save(&store, "demo", &charter).unwrap();
        let serving = |goals: &[&str], tests: Vec<Vec<TestRef>>| FeatureDefinition {
            goals: goals.iter().map(|g| g.to_string()).collect(),
            requirements: tests
                .into_iter()
                .enumerate()
                .map(|(i, t)| requirement(&format!("R-{}", i + 1), t))
                .collect(),
            ..Default::default()
        };
        let done = item(&store, "M", "Both, done", |f| {
            f.definition = Some(serving(
                &["G-1", "G-2"],
                vec![vec![test_in(TestState::Green)], vec![]],
            ));
        });
        finish(&store, &done);
        item(&store, "M", "First, open", |f| {
            f.definition = Some(serving(&["G-1"], vec![vec![test_in(TestState::Red)]]));
        });
        item(&store, "M", "Second, open", |f| {
            f.definition = Some(serving(&["G-2"], vec![vec![], vec![], vec![]]));
        });

        let o = overview(&store, "demo", None).unwrap();
        let facts: Vec<(&str, &str, usize, usize, usize, usize)> = o
            .goals
            .iter()
            .map(|g| {
                (
                    g.id.as_str(),
                    g.title.as_str(),
                    g.items_done,
                    g.items_open,
                    g.req_proven,
                    g.req_total,
                )
            })
            .collect();
        assert_eq!(
            facts,
            [
                ("G-1", "First goal", 1, 1, 1, 3),
                ("G-2", "Second goal", 1, 1, 1, 5),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-8: an ADR, an approval taken on the monitor and a release stage read as three
    /// lines in words, newest first, each linked to where it is shown.
    #[test]
    fn recent_lists_adrs_verdicts_and_releases_newest_first_in_words() {
        let (store, dir) = board(false, true);
        let adr = crate::adr::create(
            &store,
            "demo",
            "Keep it local",
            crate::adr::Adr {
                status: "accepted".into(),
                date: "2026-11-10".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let title = "A title long enough that the recent line has to cut it short somewhere";
        let code = item(&store, "M", title, |f| {
            f.definition = Some(FeatureDefinition {
                approvals: vec![
                    ApprovalEvent {
                        at: "2026-11-12T09:00:00Z".into(),
                        by: "Alice".into(),
                        verdict: "approved".into(),
                        origin: "monitor".into(),
                        ..Default::default()
                    },
                    // A withdrawal is not a decision line.
                    ApprovalEvent {
                        at: "2026-11-11T09:00:00Z".into(),
                        by: "Alice".into(),
                        verdict: "withdrawn".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            });
        });
        write_releases(
            &store,
            &[Release {
                version: "v0.8.0".into(),
                stage: "candidate".into(),
                stages: vec![StageChange {
                    stage: "candidate".into(),
                    by: "Bob".into(),
                    at: "2026-11-14T10:00:00Z".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );

        let o = overview(&store, "demo", None).unwrap();
        let cut: String = title.chars().take(60).collect();
        let lines: Vec<(&str, &str, &str, &str)> = o
            .recent
            .iter()
            .map(|r| {
                (
                    r.at.as_str(),
                    r.kind.as_str(),
                    r.words.as_str(),
                    r.to.as_str(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (
                    "2026-11-14T10:00:00Z",
                    "release-stage",
                    "v0.8.0 promoted to candidate",
                    "/p/demo/releases?tab=current#v0.8.0"
                ),
                (
                    "2026-11-12T09:00:00Z",
                    "approval",
                    format!("{code} {cut}… approved on the monitor").as_str(),
                    format!("/p/demo/feature/{code}").as_str()
                ),
                (
                    "2026-11-10",
                    "adr",
                    format!("{} accepted: Keep it local", adr.id).as_str(),
                    format!("/p/demo/docs/file/{}", adr.path).as_str()
                ),
            ]
        );
        assert_eq!(o.recent[1].code.as_deref(), Some(code.as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
