//! The Plan view (FTR-043): sprints, releases and milestones as rows on a timeline, in three
//! tabs — Current, Planned and Past — with today in focus. It replaces the Gantt and Schedule
//! pages; `gantt.rs` stays for the CLI and for export.
//!
//! Derived, never stored (ADR-0002), and reading never rewrites the board (ADR-0005). The summary
//! reads the board files only (`sprints.yaml`, `releases.yaml`, milestones and item metadata
//! through `load_meta`), never a spec. Every function takes `today`: no clock is read here, so the
//! tests can name the day.
//!
//! No date is ever made up: a row or an item with nothing to place it by is listed undated.

use crate::error::{CoreError, Result};
use crate::gantt::{fmt_date, parse_ymd};
use crate::models::FeatureItem;
use crate::releases::{self, Release, ReleaseReport, ReleaseState};
use crate::sprints::{self, Sprint, SprintReport, SprintState};
use crate::{Project, ProjectConfig, Store};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use time::{Date, Duration};

/// The tab a row falls in. The same three words the Releases page uses (FTR-041).
pub use crate::releases::Scope;

/// Days either side of today in the Current window (four weeks centred on today).
const HALF_WINDOW: i64 = 14;
/// The longest the Current window gets, widened to the running sprint (six weeks).
const WINDOW_CAP: i64 = 42;
/// The Planned tab's timeline: a quarter from today, widened to the latest dated row...
const QUARTER: i64 = 91;
/// ...but never past a year.
const PLANNED_CAP: i64 = 365;
/// The Past tab draws the newest 13 weeks of rows; older ones are listed without bands.
const PAST_SPAN: i64 = 91;

/// The counts per tab and the tab the page opens on (FTR-043 R-1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanSummary {
    pub current: usize,
    pub planned: usize,
    pub past: usize,
    /// `current` when anything runs today, else `planned`.
    pub open: String,
    /// The day the counts are for, `YYYY-MM-DD` (UTC).
    pub today: String,
}

/// A span drawn on a lane: a sprint's dates, or a milestone's reach.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanBand {
    pub from: String,
    pub to: String,
    pub label: String,
    /// `ok`, `info`, `warn`, `error` or `neutral`: the monitor's tones.
    pub tone: String,
    /// The finished part, 0..1: a sprint's done of committed, a release's finished of its items, a
    /// milestone's done of its items. `None` when there is nothing to count.
    pub progress: Option<f64>,
}

/// A point on a lane: a release's target or ship date (`diamond`), or one of its stage changes
/// (`dot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanMark {
    pub at: String,
    pub label: String,
    /// `diamond` or `dot`.
    pub kind: String,
}

/// One row of the Plan page: a sprint, a release or a milestone.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanLane {
    /// `sprint:SP-004`, `release:v0.8.0` or `milestone:MS-004`: the row's id in the URL.
    pub key: String,
    /// `sprint`, `release` or `milestone`.
    pub kind: String,
    pub label: String,
    /// A sprint's `planned`/`active`/`closed`; a release's stage, else `planned`, `shipped` or
    /// `superseded`; a milestone's `open`.
    pub state: String,
    /// The lane's first and last day; empty on an undated lane.
    pub from: String,
    pub to: String,
    pub bands: Vec<PlanBand>,
    pub marks: Vec<PlanMark>,
    /// Delivered and planned: a sprint's done and committed estimate (rounded) — on the Past tab,
    /// its delivered items of its committed items ("9 of 11 delivered") — a release's finished
    /// (shipped, once shipped) and items, a milestone's done and items.
    pub done: usize,
    pub total: usize,
    /// A sprint's done and committed estimate in the project's unit, beside the counts; `None` on
    /// every other lane and where the sprint has no report.
    pub points_done: Option<f64>,
    pub points_total: Option<f64>,
    /// How many items it carried out.
    pub carried: usize,
    /// The running sprint's burndown; `None` on every other lane.
    pub burndown: Option<SprintReport>,
}

/// One tab of the Plan page: its timeline's span and its rows.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanSlice {
    pub scope: Scope,
    pub today: String,
    pub from: String,
    pub to: String,
    pub lanes: Vec<PlanLane>,
    /// Rows with no date to place them by, by key.
    pub undated: Vec<PlanLane>,
}

fn add_days(d: Date, n: i64) -> Date {
    d.checked_add(Duration::days(n)).unwrap_or(d)
}

fn running(sprints: &[Sprint], today: Date) -> impl Iterator<Item = &Sprint> {
    sprints
        .iter()
        .filter(move |s| s.state_on(today) == SprintState::Active)
}

/// The Current tab's window (FTR-043 R-2): `today − 14 .. today + 14`, widened to the running
/// sprint's `start` and `end` when they fall outside, capped at 42 days in all. Over the cap the
/// side nearer today is kept and the farther one trimmed; both are trimmed to 21 days when each
/// reaches that far. A sprint date that does not parse widens nothing.
pub fn window(today: Date, sprints: &[Sprint]) -> (Date, Date) {
    let (mut from, mut to) = (add_days(today, -HALF_WINDOW), add_days(today, HALF_WINDOW));
    for s in running(sprints, today) {
        if let Some(start) = parse_ymd(&s.start) {
            from = from.min(start);
        }
        if let Some(end) = parse_ymd(&s.end) {
            to = to.max(end);
        }
    }
    let (before, after) = ((today - from).whole_days(), (to - today).whole_days());
    if before + after > WINDOW_CAP {
        let half = WINDOW_CAP / 2;
        let (b, a) = if before.min(after) >= half {
            (half, half)
        } else if before < after {
            (before, WINDOW_CAP - before)
        } else {
            (WINDOW_CAP - after, after)
        };
        from = add_days(today, -b);
        to = add_days(today, a);
    }
    (from, to)
}

/// What every read of the view needs: item metadata (never a spec), sprints and releases.
struct Board {
    project: Project,
    sprints: Vec<Sprint>,
    releases: Vec<Release>,
}

fn board(store: &Store, p: &str) -> Result<Board> {
    if !store.project_exists(p) {
        return Err(CoreError::ProjectNotFound(p.to_string()));
    }
    Ok(Board {
        project: store.load_meta(p)?,
        sprints: sprints::load(store, p)?,
        releases: releases::load(store, p)?,
    })
}

/// A release with no stage whose target falls in the Current window: it is due now, so it is
/// counted and drawn in Current, not Planned.
fn due_in(r: &Release, from: Date, to: Date) -> bool {
    releases::scope_of(r) == Scope::Planned
        && parse_ymd(&r.target).is_some_and(|t| t >= from && t <= to)
}

fn finished(config: &ProjectConfig, f: &FeatureItem) -> bool {
    config.counts_as_done(&f.status)
}

/// A milestone is open while it holds an item that is neither finished nor set aside (a no-op
/// status). One with no items has nothing to plan and is not a row.
fn milestone_open(project: &Project, code: &str) -> bool {
    project.features.iter().any(|f| {
        f.milestone == code && !finished(&project.config, f) && !project.config.is_no_op(&f.status)
    })
}

/// The counts per tab (FTR-043 R-1). Reads `sprints.yaml`, `releases.yaml`, the milestones and the
/// item metadata; never a spec.
pub fn summary(store: &Store, p: &str, today: Date) -> Result<PlanSummary> {
    let b = board(store, p)?;
    let (from, to) = window(today, &b.sprints);
    let sprints_in = |state: SprintState| {
        b.sprints
            .iter()
            .filter(|s| s.state_on(today) == state)
            .count()
    };
    let releases_where =
        |keep: &dyn Fn(&Release) -> bool| b.releases.iter().filter(|r| keep(r)).count();
    let current = sprints_in(SprintState::Active)
        + releases_where(&|r| releases::scope_of(r) == Scope::Current || due_in(r, from, to));
    let planned = sprints_in(SprintState::Planned)
        + releases_where(&|r| releases::scope_of(r) == Scope::Planned && !due_in(r, from, to))
        + b.project
            .milestones
            .iter()
            .filter(|m| milestone_open(&b.project, &m.code))
            .count();
    let past =
        sprints_in(SprintState::Closed) + releases_where(&|r| releases::scope_of(r) == Scope::Past);
    Ok(PlanSummary {
        current,
        planned,
        past,
        open: if current > 0 { "current" } else { "planned" }.to_string(),
        today: fmt_date(today),
    })
}

fn titled(code: &str, name: &str) -> String {
    if name.trim().is_empty() {
        code.to_string()
    } else {
        format!("{code} · {name}")
    }
}

fn ratio(done: f64, total: f64) -> Option<f64> {
    (total > 0.0).then(|| (done / total).clamp(0.0, 1.0))
}

fn sprint_state_word(state: SprintState) -> &'static str {
    match state {
        SprintState::Planned => "planned",
        SprintState::Active => "active",
        SprintState::Closed => "closed",
    }
}

fn cadence_on(b: &Board) -> bool {
    b.project.config.cadence.sprints
}

/// What a sprint holds, counted the way `sprints::report` counts it: the items planned into it and
/// those it carried out; done is what reached a finished status, less what it carried out.
#[derive(Clone, Copy)]
struct Tally {
    committed: f64,
    done: f64,
    items: usize,
    delivered: usize,
}

fn tally(b: &Board, s: &Sprint) -> Tally {
    let carried: BTreeSet<&str> = s.carried.iter().map(|c| c.code.as_str()).collect();
    let size = |f: &FeatureItem| match b.project.config.estimate_unit {
        crate::config::EstimateUnit::Points => f.points.unwrap_or(0.0),
        crate::config::EstimateUnit::Days => f.estimate_days.unwrap_or(0.0),
    };
    let mut t = Tally {
        committed: 0.0,
        done: 0.0,
        items: 0,
        delivered: 0,
    };
    for f in &b.project.features {
        let out = carried.contains(f.code.as_str());
        if f.sprint.as_deref() != Some(s.code.as_str()) && !out {
            continue;
        }
        t.items += 1;
        t.committed += size(f);
        if !out && finished(&b.project.config, f) {
            t.delivered += 1;
            t.done += size(f);
        }
    }
    t
}

/// How a sprint's lane counts `done` and `total`: by its estimate, or by its items (the Past tab).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Count {
    Estimate,
    Items,
}

/// A sprint's lane: one band over its dates, filled by done of committed. With `burndown`, the
/// lane carries the sprint's report. A sprint whose dates do not parse gets an undated lane.
fn sprint_lane(
    b: &Board,
    store: &Store,
    p: &str,
    s: &Sprint,
    today: Date,
    burndown: bool,
    count: Count,
) -> PlanLane {
    // The burndown needs the sprint's report, which reads the board again; the other lanes count
    // from the metadata already loaded, so a tab of many sprints loads it once (R-9's budget). With
    // the sprints cadence off a board's sprints show with nothing counted, never an error.
    let report = if burndown {
        sprints::report(store, p, &s.code, today).ok()
    } else {
        None
    };
    let tally = cadence_on(b).then(|| tally(b, s));
    let (done, committed) = tally.map(|t| (t.done, t.committed)).unwrap_or((0.0, 0.0));
    let (done_n, total_n) = match (count, tally) {
        (Count::Items, Some(t)) => (t.delivered, t.items),
        (Count::Items, None) => (0, 0),
        (Count::Estimate, _) => (
            done.round().max(0.0) as usize,
            committed.round().max(0.0) as usize,
        ),
    };
    let state = s.state_on(today);
    let label = titled(&s.code, &s.name);
    let dated = parse_ymd(&s.start).is_some() && parse_ymd(&s.end).is_some();
    let tone = match state {
        SprintState::Active => "info",
        SprintState::Planned => "neutral",
        SprintState::Closed => "ok",
    };
    PlanLane {
        key: format!("sprint:{}", s.code),
        kind: "sprint".into(),
        state: sprint_state_word(state).into(),
        from: if dated {
            s.start.clone()
        } else {
            String::new()
        },
        to: if dated { s.end.clone() } else { String::new() },
        bands: if dated {
            vec![PlanBand {
                from: s.start.clone(),
                to: s.end.clone(),
                label: label.clone(),
                tone: tone.into(),
                progress: ratio(done, committed),
            }]
        } else {
            Vec::new()
        },
        marks: Vec::new(),
        done: done_n,
        total: total_n,
        points_done: tally.map(|t| t.done),
        points_total: tally.map(|t| t.committed),
        carried: s.carried.len(),
        burndown: if burndown { report } else { None },
        label,
    }
}

fn release_state_word(r: &Release) -> String {
    if r.state == ReleaseState::Shipped {
        "shipped".into()
    } else if !r.superseded_by.is_empty() {
        "superseded".into()
    } else if !r.stage.is_empty() {
        r.stage.clone()
    } else {
        "planned".into()
    }
}

/// A release's lane with the given marks and no band; its span runs from its first mark to its
/// last, and it is undated when it has none.
fn release_lane(rep: &ReleaseReport, marks: Vec<PlanMark>, done: usize) -> PlanLane {
    let r = &rep.release;
    let from = marks.iter().map(|m| m.at.clone()).min().unwrap_or_default();
    let to = marks.iter().map(|m| m.at.clone()).max().unwrap_or_default();
    PlanLane {
        key: format!("release:{}", r.version),
        kind: "release".into(),
        label: titled(&r.version, &r.name),
        state: release_state_word(r),
        from,
        to,
        bands: Vec::new(),
        marks,
        done,
        total: rep.items.len(),
        points_done: None,
        points_total: None,
        carried: r.carried.len(),
        burndown: None,
    }
}

fn diamond(at: Date, label: &str) -> PlanMark {
    PlanMark {
        at: fmt_date(at),
        label: label.to_string(),
        kind: "diamond".into(),
    }
}

/// A release in flight: a dot for each stage change, and a diamond at its target, or at its
/// newest stage change when it has no target.
fn in_flight_marks(r: &Release) -> Vec<PlanMark> {
    let mut marks: Vec<PlanMark> = r
        .stages
        .iter()
        .filter_map(|c| {
            parse_ymd(&c.at).map(|at| PlanMark {
                at: fmt_date(at),
                label: c.stage.clone(),
                kind: "dot".into(),
            })
        })
        .collect();
    let target =
        parse_ymd(&r.target).or_else(|| r.stages.iter().rev().find_map(|c| parse_ymd(&c.at)));
    if let Some(t) = target {
        marks.push(diamond(t, &r.version));
    }
    marks
}

/// Put each lane with no date in `undated`, by key.
fn split_undated(lanes: Vec<PlanLane>) -> (Vec<PlanLane>, Vec<PlanLane>) {
    let (dated, mut undated): (Vec<_>, Vec<_>) =
        lanes.into_iter().partition(|l| !l.from.is_empty());
    undated.sort_by(|a, b| a.key.cmp(&b.key));
    (dated, undated)
}

/// One tab's rows (FTR-043 R-2, R-4, R-5).
///
/// - **Current**: the running sprints with their burndown, the releases in flight and the releases
///   due in the window, soonest end first, over [`window`].
/// - **Planned**: planned sprints, planned releases (a diamond at the target) and open milestones (a
///   band from today to the latest target of the releases their items are planned into), soonest
///   start first, over a quarter from today widened to the latest row and capped at a year.
/// - **Past**: closed sprints and shipped or superseded releases, newest first, over the newest 13
///   weeks of them; older rows are listed without bands or marks.
///
/// A row with nothing to date it by is in `undated`, by key, never dropped.
pub fn slice(store: &Store, p: &str, scope: Scope, today: Date) -> Result<PlanSlice> {
    let b = board(store, p)?;
    let reports = releases::report_all(store, p)?;
    let (win_from, win_to) = window(today, &b.sprints);
    let in_scope = |r: &Release, s: Scope| releases::scope_of(r) == s;
    let (from, to, lanes, undated) = match scope {
        Scope::Current => {
            let mut lanes: Vec<PlanLane> = running(&b.sprints, today)
                .map(|s| sprint_lane(&b, store, p, s, today, true, Count::Estimate))
                .collect();
            for rep in &reports {
                let r = &rep.release;
                if in_scope(r, Scope::Current) {
                    lanes.push(release_lane(rep, in_flight_marks(r), rep.finished));
                } else if due_in(r, win_from, win_to) {
                    let marks = parse_ymd(&r.target)
                        .map(|t| vec![diamond(t, &r.version)])
                        .unwrap_or_default();
                    lanes.push(release_lane(rep, marks, rep.finished));
                }
            }
            let (mut dated, undated) = split_undated(lanes);
            dated.sort_by(|a, b| a.to.cmp(&b.to).then_with(|| a.key.cmp(&b.key)));
            (win_from, win_to, dated, undated)
        }
        Scope::Planned => {
            let mut lanes: Vec<PlanLane> = b
                .sprints
                .iter()
                .filter(|s| s.state_on(today) == SprintState::Planned)
                .map(|s| sprint_lane(&b, store, p, s, today, false, Count::Estimate))
                .collect();
            for rep in &reports {
                let r = &rep.release;
                if in_scope(r, Scope::Planned) && !due_in(r, win_from, win_to) {
                    let marks = parse_ymd(&r.target)
                        .map(|t| vec![diamond(t, &r.version)])
                        .unwrap_or_default();
                    lanes.push(release_lane(rep, marks, rep.finished));
                }
            }
            for m in &b.project.milestones {
                if milestone_open(&b.project, &m.code) {
                    lanes.push(milestone_lane(&b, m, today));
                }
            }
            let (mut dated, undated) = split_undated(lanes);
            dated.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.key.cmp(&b.key)));
            let latest = dated.iter().filter_map(|l| parse_ymd(&l.to)).max();
            let to = latest
                .map_or(add_days(today, QUARTER), |l| {
                    l.max(add_days(today, QUARTER))
                })
                .min(add_days(today, PLANNED_CAP));
            (today, to, dated, undated)
        }
        Scope::Past => {
            let mut lanes: Vec<PlanLane> = b
                .sprints
                .iter()
                .filter(|s| s.state_on(today) == SprintState::Closed)
                .map(|s| sprint_lane(&b, store, p, s, today, false, Count::Items))
                .collect();
            for rep in &reports {
                let r = &rep.release;
                if in_scope(r, Scope::Past) {
                    let marks = parse_ymd(&r.shipped_at)
                        .map(|t| vec![diamond(t, &r.version)])
                        .unwrap_or_default();
                    lanes.push(release_lane(rep, marks, r.shipped.len()));
                }
            }
            let (mut dated, undated) = split_undated(lanes);
            dated.sort_by(past_order);
            let newest = dated.iter().filter_map(|l| parse_ymd(&l.to)).max();
            let to = newest.unwrap_or(today);
            let from = add_days(to, -PAST_SPAN);
            for lane in &mut dated {
                if parse_ymd(&lane.to).is_some_and(|d| d < from) {
                    lane.bands.clear();
                    lane.marks.clear();
                }
            }
            (from, to, dated, undated)
        }
    };
    Ok(PlanSlice {
        scope,
        today: fmt_date(today),
        from: fmt_date(from),
        to: fmt_date(to),
        lanes,
        undated,
    })
}

/// Newest first by the lane's last day (a sprint's end, a release's ship date); two releases on one
/// day newest version first; then by key.
fn past_order(a: &PlanLane, b: &PlanLane) -> Ordering {
    b.to.cmp(&a.to)
        .then_with(|| match (a.kind.as_str(), b.kind.as_str()) {
            ("release", "release") => {
                let version = |l: &PlanLane| l.key.trim_start_matches("release:").to_string();
                releases::version_key(&version(b)).cmp(&releases::version_key(&version(a)))
            }
            _ => Ordering::Equal,
        })
        .then_with(|| a.key.cmp(&b.key))
}

/// An open milestone's lane: a band from today to the latest target of the releases its items are
/// planned into, filled by its done items; undated when none of those releases has a target.
fn milestone_lane(b: &Board, m: &crate::models::Milestone, today: Date) -> PlanLane {
    let items: Vec<&FeatureItem> = b
        .project
        .features
        .iter()
        .filter(|f| f.milestone == m.code)
        .collect();
    let planned_into: BTreeSet<&str> = items.iter().filter_map(|f| f.release.as_deref()).collect();
    let target = b
        .releases
        .iter()
        .filter(|r| planned_into.contains(r.version.as_str()))
        .filter_map(|r| parse_ymd(&r.target))
        .max();
    let done = items
        .iter()
        .filter(|f| finished(&b.project.config, f))
        .count();
    let label = titled(&m.code, &m.name);
    let (from, to, bands) = match target {
        Some(t) => {
            let end = t.max(today);
            (
                fmt_date(today),
                fmt_date(end),
                vec![PlanBand {
                    from: fmt_date(today),
                    to: fmt_date(end),
                    label: label.clone(),
                    tone: "neutral".into(),
                    progress: ratio(done as f64, items.len() as f64),
                }],
            )
        }
        None => (String::new(), String::new(), Vec::new()),
    };
    PlanLane {
        key: format!("milestone:{}", m.code),
        kind: "milestone".into(),
        label,
        state: "open".into(),
        from,
        to,
        bands,
        marks: Vec::new(),
        done,
        total: items.len(),
        points_done: None,
        points_total: None,
        carried: 0,
        burndown: None,
    }
}

/// Where an item sits on the timeline (FTR-043 R-3): `(from, to, placed)` by the first rule that
/// fits.
///
/// 1. Finished: from its first move out of a status that is not a no-op (else `created_at`) to its
///    last move into the status it is in (else `updated_at`), as `moves`. A finished item with no
///    moves recorded is `created_at..updated_at`.
/// 2. In a known sprint with dates: the sprint's `start..end`, as `sprint`.
/// 3. A `start` that parses: `start .. start + ceil(estimate_days, or 1) − 1`, as `start`.
/// 4. Otherwise `("", "", "undated")`. No date is ever made up.
pub fn place(
    f: &FeatureItem,
    config: &ProjectConfig,
    sprints: &[Sprint],
) -> (String, String, &'static str) {
    if config.counts_as_done(&f.status) {
        let from = f
            .history
            .iter()
            .find(|t| !config.is_no_op(&t.from))
            .map_or(f.created_at.as_str(), |t| t.at.as_str());
        let to = f
            .history
            .iter()
            .rev()
            .find(|t| t.to == f.status)
            .map_or(f.updated_at.as_str(), |t| t.at.as_str());
        if let (Some(a), Some(b)) = (parse_ymd(from), parse_ymd(to)) {
            return (fmt_date(a.min(b)), fmt_date(a.max(b)), "moves");
        }
    }
    if let Some(s) = f
        .sprint
        .as_deref()
        .and_then(|code| sprints.iter().find(|s| s.code == code))
        && let (Some(a), Some(b)) = (parse_ymd(&s.start), parse_ymd(&s.end))
    {
        return (fmt_date(a), fmt_date(b), "sprint");
    }
    if let Some(start) = f.start.as_deref().and_then(parse_ymd) {
        let days = f.estimate_days.unwrap_or(1.0).ceil().max(1.0) as i64;
        return (
            fmt_date(start),
            fmt_date(add_days(start, days - 1)),
            "start",
        );
    }
    (String::new(), String::new(), "undated")
}

/// One item in an opened row, placed on the row's timeline by [`place`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlacedItem {
    pub code: String,
    pub title: String,
    pub status: String,
    /// The item's first and last day; empty when undated.
    pub from: String,
    pub to: String,
    /// `sprint`, `moves`, `start` or `undated`: the rule that placed it.
    pub placed: String,
    /// On the project's dependency critical path.
    pub critical: bool,
    /// The ids of the pending planning decisions that name the item, oldest first (FTR-049).
    pub pending: Vec<String>,
}

/// An opened row's body: its items placed, a milestone's items by status, and what it carried out.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RowBody {
    pub key: String,
    pub items: Vec<PlacedItem>,
    /// A milestone's item codes per displayed status, in the workflow's order and dependencies
    /// first; a status none of them is in is left out. Empty for a sprint or a release.
    pub by_status: Vec<(String, Vec<String>)>,
    /// What a sprint or a release carried out, and where to (a sprint's carries have no `why`).
    pub carried: Vec<releases::Carry>,
}

/// One opened row (FTR-043 R-3): the items of `sprint:<code>`, `release:<version>` or
/// `milestone:<code>`, each placed and marked when it is on the dependency critical path. A key of
/// another kind, or naming a row the board does not hold, is a 404.
pub fn row(store: &Store, p: &str, key: &str, _today: Date) -> Result<RowBody> {
    let b = board(store, p)?;
    let missing = || CoreError::NoSuchRoute(format!("no plan row {key}"));
    let (kind, code) = key.split_once(':').ok_or_else(missing)?;
    let in_items = |codes: &[String]| -> Vec<&FeatureItem> {
        codes
            .iter()
            .filter_map(|c| b.project.features.iter().find(|f| &f.code == c))
            .collect()
    };
    let (items, carried, by_status): (Vec<&FeatureItem>, Vec<releases::Carry>, _) = match kind {
        "sprint" => {
            let s = b
                .sprints
                .iter()
                .find(|s| s.code == code)
                .ok_or_else(missing)?;
            let items = b
                .project
                .features
                .iter()
                .filter(|f| f.sprint.as_deref() == Some(code))
                .collect();
            let carried = s
                .carried
                .iter()
                .map(|c| releases::Carry {
                    code: c.code.clone(),
                    to: c.to.clone(),
                    why: String::new(),
                })
                .collect();
            (items, carried, Vec::new())
        }
        "release" => {
            let rep = releases::report_all(store, p)?
                .into_iter()
                .find(|r| r.release.version == code)
                .ok_or_else(missing)?;
            (in_items(&rep.items), rep.release.carried, Vec::new())
        }
        "milestone" => {
            if !b.project.milestones.iter().any(|m| m.code == code) {
                return Err(CoreError::MilestoneNotFound(code.to_string()));
            }
            let items: Vec<&FeatureItem> = b
                .project
                .features
                .iter()
                .filter(|f| f.milestone == code)
                .collect();
            let deps_of = |c: &str| -> Vec<String> {
                items
                    .iter()
                    .find(|f| f.code == c)
                    .map(|f| {
                        f.depends_on
                            .iter()
                            .filter_map(|d| {
                                let (dp, dc) = crate::graph::parse_ref(d, p);
                                (dp == p).then_some(dc)
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let by_status = b
                .project
                .config
                .displayed_states
                .iter()
                .filter_map(|state| {
                    let codes: Vec<String> = items
                        .iter()
                        .filter(|f| &f.status == state)
                        .map(|f| f.code.clone())
                        .collect();
                    (!codes.is_empty())
                        .then(|| (state.clone(), crate::graph::order_by_deps(&codes, deps_of)))
                })
                .collect();
            (items, Vec::new(), by_status)
        }
        _ => return Err(missing()),
    };
    let critical = if items.is_empty() {
        BTreeSet::new()
    } else {
        crate::graph::DependencyView::build(store, None)?
            .schedule(Some(p))
            .critical
    };
    // The planning decisions waiting on each item, oldest first (FTR-049).
    let mut pending: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for v in crate::planning::list(store, p, Some("pending"))? {
        for code in v.decision.items.iter().collect::<BTreeSet<_>>() {
            pending
                .entry(code.clone())
                .or_default()
                .push(v.decision.id.clone());
        }
    }
    let items = items
        .into_iter()
        .map(|f| {
            let (from, to, placed) = place(f, &b.project.config, &b.sprints);
            PlacedItem {
                code: f.code.clone(),
                title: f.title.clone(),
                status: f.status.clone(),
                from,
                to,
                placed: placed.to_string(),
                critical: critical.contains(&crate::graph::qualify(p, &f.code)),
                pending: pending.get(&f.code).cloned().unwrap_or_default(),
            }
        })
        .collect();
    Ok(RowBody {
        key: key.to_string(),
        items,
        by_status,
        carried,
    })
}

/// One project's row on the portfolio's timeline (FTR-043 R-7).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PortfolioLane {
    pub project: String,
    /// The project's name, else its id.
    pub label: String,
    /// Its running sprint's band.
    pub bands: Vec<PlanBand>,
    /// Its releases in flight, as diamonds at their targets.
    pub marks: Vec<PlanMark>,
}

/// The portfolio's timeline: its span and one lane per project, so the web never works out the
/// window (the coordinator, 17 Nov 2026).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PortfolioPlan {
    /// The Current tab's window over the running sprints the lanes draw, `YYYY-MM-DD` (UTC).
    pub from: String,
    pub to: String,
    pub today: String,
    pub lanes: Vec<PortfolioLane>,
}

/// The portfolio's timeline (FTR-043 R-7): one lane per project with sprints or releases on,
/// holding its running sprint's band and its releases-in-flight diamonds, over the Current tab's
/// window (widened to the running sprints of the projects with sprints on). A project with neither
/// cadence on has no lane.
pub fn portfolio(store: &Store, today: Date) -> Result<PortfolioPlan> {
    let mut lanes = Vec::new();
    let mut drawn: Vec<Sprint> = Vec::new();
    for p in store.list_projects()? {
        let b = board(store, &p)?;
        let cadence = &b.project.config.cadence;
        if !cadence.sprints && !cadence.releases {
            continue;
        }
        let bands = if cadence.sprints {
            drawn.extend(running(&b.sprints, today).cloned());
            running(&b.sprints, today)
                .flat_map(|s| sprint_lane(&b, store, &p, s, today, false, Count::Estimate).bands)
                .collect()
        } else {
            Vec::new()
        };
        let marks = if cadence.releases {
            b.releases
                .iter()
                .filter(|r| releases::scope_of(r) == Scope::Current)
                .flat_map(in_flight_marks)
                .filter(|m| m.kind == "diamond")
                .collect()
        } else {
            Vec::new()
        };
        let name = b.project.config.name.trim();
        lanes.push(PortfolioLane {
            label: if name.is_empty() {
                p.clone()
            } else {
                name.to_string()
            },
            project: p,
            bands,
            marks,
        });
    }
    let (from, to) = window(today, &drawn);
    Ok(PortfolioPlan {
        from: fmt_date(from),
        to: fmt_date(to),
        today: fmt_date(today),
        lanes,
    })
}

#[cfg(test)]
mod tests {
    use super::{Scope, place, portfolio, row, slice, summary, window};
    use crate::releases::{Carry as ReleaseCarry, Release, ReleaseState, StageChange};
    use crate::sprints::{Carry as SprintCarry, Sprint, SprintState};
    use crate::{FeatureItem, ProjectConfig, Store};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use time::macros::date;

    const TODAY: time::Date = date!(2026 - 11 - 17);

    fn sprint(code: &str, start: &str, end: &str, state: SprintState) -> Sprint {
        Sprint {
            code: code.into(),
            start: start.into(),
            end: end.into(),
            state,
            ..Default::default()
        }
    }

    fn release(version: &str, target: &str) -> Release {
        Release {
            version: version.into(),
            target: target.into(),
            ..Default::default()
        }
    }

    /// A board under the temp dir with sprints and releases on, points as the unit, and:
    ///
    /// - sprints: SP-000 closed in June, SP-001 closed 1–14 Nov (carried FEAT-007 to SP-002),
    ///   SP-002 running 15–28 Nov, SP-003 planned 29 Nov–12 Dec, SP-009 planned with no dates;
    /// - releases: v0.0.9 and v0.1.0 shipped on 10 Nov, v0.2.0 a candidate due 20 Nov, v0.3.0
    ///   due 25 Nov (in the window), v0.4.0 due 15 Jan, v0.5.0 with no target;
    /// - milestones: M1 open (items in v0.3.0 and v0.4.0), M2 all finished, M3 open (in no
    ///   release), M4 empty;
    /// - items, by title: A (M2, SP-002, 2 pts, Completed), B (M1, SP-002, 2 pts), C (M1,
    ///   v0.4.0), D (M1, v0.3.0), E (M3), P1 (M2, SP-001, 3 pts, Completed, shipped in v0.1.0),
    ///   P2 (M3, SP-002, 2 pts, carried out of SP-001).
    fn board(tag: &str) -> (Store, BTreeMap<&'static str, String>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-planview-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("shop", ProjectConfig::default_for("shop"))
            .unwrap();
        // The board is the test's own, under the temp dir (L-41).
        assert!(store.project_dir("shop").starts_with(std::env::temp_dir()));
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(
                &serde_json::json!({"sprints": true, "releases": true, "estimate_unit": "points"}),
            ),
        )
        .unwrap();
        for m in ["M1", "M2", "M3", "M4"] {
            store
                .add_milestone("shop", m, "", vec![], Some(m.into()))
                .unwrap();
        }
        let mut codes = BTreeMap::new();
        // (title, milestone, sprint, release, points, finished)
        type Spec = (
            &'static str,
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
        );
        let specs: [(Spec, f64, bool); 7] = [
            (("A", "M2", Some("SP-002"), None), 2.0, true),
            (("B", "M1", Some("SP-002"), None), 2.0, false),
            (("C", "M1", None, Some("v0.4.0")), 0.0, false),
            (("D", "M1", None, Some("v0.3.0")), 0.0, false),
            (("E", "M3", None, None), 0.0, false),
            (("P1", "M2", Some("SP-001"), Some("v0.1.0")), 3.0, true),
            (("P2", "M3", Some("SP-002"), None), 2.0, false),
        ];
        for (i, ((title, ms, sp, rel), points, done)) in specs.into_iter().enumerate() {
            let code = format!("FEAT-{:03}", i + 1);
            store
                .add_feature("shop", title, "", ms, Some(code.clone()))
                .unwrap();
            if done {
                store.move_feature("shop", &code, "Scheduled").unwrap();
                store.move_feature("shop", &code, "Completed").unwrap();
            }
            let mut f = store
                .load_meta("shop")
                .unwrap()
                .feature(&code)
                .unwrap()
                .clone();
            f.sprint = sp.map(str::to_string);
            f.release = rel.map(str::to_string);
            f.points = (points > 0.0).then_some(points);
            store.persist_feature_for_test("shop", &f).unwrap();
            codes.insert(title, code);
        }
        let sprints = vec![
            sprint("SP-000", "2026-06-01", "2026-06-14", SprintState::Closed),
            Sprint {
                carried: vec![SprintCarry {
                    code: codes["P2"].clone(),
                    to: "SP-002".into(),
                }],
                ..sprint("SP-001", "2026-11-01", "2026-11-14", SprintState::Closed)
            },
            sprint("SP-002", "2026-11-15", "2026-11-28", SprintState::Active),
            sprint("SP-003", "2026-11-29", "2026-12-12", SprintState::Planned),
            sprint("SP-009", "soon", "", SprintState::Planned),
        ];
        let releases = vec![
            Release {
                state: ReleaseState::Shipped,
                shipped_at: "2026-11-10T09:00:00Z".into(),
                ..release("v0.0.9", "")
            },
            Release {
                state: ReleaseState::Shipped,
                shipped_at: "2026-11-10T15:00:00Z".into(),
                shipped: vec![codes["P1"].clone()],
                carried: vec![ReleaseCarry {
                    code: codes["C"].clone(),
                    to: "v0.4.0".into(),
                    why: String::new(),
                }],
                ..release("v0.1.0", "2026-11-09")
            },
            Release {
                stage: "candidate".into(),
                stages: vec![StageChange {
                    stage: "candidate".into(),
                    at: "2026-11-16T10:00:00Z".into(),
                    ..Default::default()
                }],
                ..release("v0.2.0", "2026-11-20")
            },
            release("v0.3.0", "2026-11-25"),
            release("v0.4.0", "2027-01-15"),
            release("v0.5.0", ""),
        ];
        let pdir = store.project_dir("shop");
        crate::atomic::write(
            pdir.join("sprints.yaml"),
            crate::yaml::to_string(&sprints).unwrap(),
        )
        .unwrap();
        crate::atomic::write(
            pdir.join("releases.yaml"),
            crate::yaml::to_string(&releases).unwrap(),
        )
        .unwrap();
        (store, codes, dir)
    }

    fn keys(lanes: &[super::PlanLane]) -> Vec<&str> {
        lanes.iter().map(|l| l.key.as_str()).collect()
    }

    /// FTR-043 R-1: sprints, releases and milestones each fall in one tab; the page opens on
    /// Current when something runs, else Planned; each tab's slice holds what its count says.
    #[test]
    fn rows_fall_into_current_planned_and_past() {
        let (store, _codes, dir) = board("r1");
        let s = summary(&store, "shop", TODAY).unwrap();
        // Current: SP-002, v0.2.0 (at a stage), v0.3.0 (due in the window).
        assert_eq!(s.current, 3, "{s:?}");
        // Planned: SP-003, SP-009, v0.4.0, v0.5.0, M1, M3 (M2 is finished, M4 empty).
        assert_eq!(s.planned, 6, "{s:?}");
        // Past: SP-000, SP-001, v0.0.9, v0.1.0.
        assert_eq!(s.past, 4, "{s:?}");
        assert_eq!(s.open, "current");
        assert_eq!(s.today, "2026-11-17");
        for (scope, count) in [
            (Scope::Current, s.current),
            (Scope::Planned, s.planned),
            (Scope::Past, s.past),
        ] {
            let tab = slice(&store, "shop", scope, TODAY).unwrap();
            assert_eq!(tab.lanes.len() + tab.undated.len(), count, "{scope:?}");
        }

        // A board where nothing runs opens on Planned.
        store
            .init_project("quiet", ProjectConfig::default_for("quiet"))
            .unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/quiet/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        crate::atomic::write(
            store.project_dir("quiet").join("releases.yaml"),
            crate::yaml::to_string(&vec![release("v1.0.0", "")]).unwrap(),
        )
        .unwrap();
        let q = summary(&store, "quiet", TODAY).unwrap();
        assert_eq!((q.current, q.planned, q.past), (0, 1, 0));
        assert_eq!(q.open, "planned");
        assert!(summary(&store, "nowhere", TODAY).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-043 R-2: four weeks centred on today, widened to the running sprint, capped at six
    /// weeks; the Current tab holds the running sprint with its burndown and the releases in
    /// flight and due, soonest end first.
    #[test]
    fn the_current_window_centres_on_today_and_covers_the_running_sprint() {
        let d = |n: i64| super::add_days(TODAY, n);
        let ymd = |n: i64| crate::gantt::fmt_date(d(n));
        let active = |from: i64, to: i64| sprint("SP-1", &ymd(from), &ymd(to), SprintState::Active);
        // No sprint: ±14 days.
        assert_eq!(window(TODAY, &[]), (d(-14), d(14)));
        // A sprint inside the window changes nothing.
        assert_eq!(window(TODAY, &[active(-3, 5)]), (d(-14), d(14)));
        // A sprint from today − 20: the window starts with it.
        assert_eq!(window(TODAY, &[active(-20, 5)]), (d(-20), d(14)));
        // A planned or closed sprint never widens it.
        let closed = sprint("SP-0", &ymd(-40), &ymd(-20), SprintState::Closed);
        assert_eq!(window(TODAY, &[closed]), (d(-14), d(14)));
        // The 42-day cap: the nearer side is kept, the farther trimmed...
        assert_eq!(window(TODAY, &[active(-35, 3)]), (d(-28), d(14)));
        // ...and both are trimmed to 21 days when each reaches past that.
        assert_eq!(window(TODAY, &[active(-30, 30)]), (d(-21), d(21)));
        // A sprint whose dates do not parse widens nothing.
        assert_eq!(
            window(
                TODAY,
                &[sprint("SP-2", "soon", "later", SprintState::Active)]
            ),
            (d(-14), d(14))
        );

        let (store, _codes, dir) = board("r2");
        let tab = slice(&store, "shop", Scope::Current, TODAY).unwrap();
        assert_eq!(
            (tab.from.as_str(), tab.to.as_str()),
            ("2026-11-03", "2026-12-01")
        );
        // Soonest end first: v0.2.0 (20 Nov), v0.3.0 (25 Nov), SP-002 (28 Nov).
        assert_eq!(
            keys(&tab.lanes),
            ["release:v0.2.0", "release:v0.3.0", "sprint:SP-002"]
        );
        assert!(tab.undated.is_empty());
        let sp = &tab.lanes[2];
        assert_eq!(sp.state, "active");
        assert_eq!(sp.bands.len(), 1);
        assert_eq!(
            (sp.bands[0].from.as_str(), sp.bands[0].to.as_str()),
            ("2026-11-15", "2026-11-28")
        );
        // Committed A + B + P2 = 6 points, A done.
        assert_eq!((sp.done, sp.total), (2, 6));
        let progress = sp.bands[0].progress.unwrap();
        assert!((progress - 2.0 / 6.0).abs() < 1e-9, "{progress}");
        let burndown = sp.burndown.as_ref().expect("the running sprint's burndown");
        assert_eq!(burndown.sprint.code, "SP-002");
        assert_eq!(burndown.burndown.first().unwrap().date, "2026-11-15");
        // In flight: a dot for the stage change and a diamond at the target; no band.
        let flight = &tab.lanes[0];
        assert!(flight.bands.is_empty());
        assert_eq!(flight.state, "candidate");
        let marks: Vec<(&str, &str)> = flight
            .marks
            .iter()
            .map(|m| (m.at.as_str(), m.kind.as_str()))
            .collect();
        assert_eq!(marks, [("2026-11-16", "dot"), ("2026-11-20", "diamond")]);
        // Due in the window: a diamond at the target.
        let due = &tab.lanes[1];
        assert_eq!(due.state, "planned");
        assert_eq!(due.marks.len(), 1);
        assert_eq!(
            (due.marks[0].at.as_str(), due.marks[0].kind.as_str()),
            ("2026-11-25", "diamond")
        );
        // Only the running sprint carries a burndown.
        assert!(tab.lanes[..2].iter().all(|l| l.burndown.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn item(code: &str, status: &str) -> FeatureItem {
        let mut f: FeatureItem = crate::yaml::from_str(&format!(
            "code: {code}\ntitle: t\nstatus: {status}\nmilestone: M\ncreated_at: \"2026-09-01T08:00:00Z\"\nupdated_at: \"2026-09-09T08:00:00Z\"\n"
        ))
        .unwrap();
        f.history.clear();
        f
    }

    fn moved(at: &str, from: &str, to: &str) -> crate::models::Transition {
        crate::models::Transition {
            at: at.into(),
            from: from.into(),
            to: to.into(),
            ..Default::default()
        }
    }

    /// FTR-043 R-3: an item is placed by its own moves once finished, else by its sprint, else by
    /// its start, else listed undated — never at a made-up date.
    #[test]
    fn an_item_is_placed_by_its_sprint_its_moves_or_its_start_and_never_at_a_made_up_date() {
        let config = ProjectConfig::default_for("shop");
        let sprints = [
            sprint("SP-002", "2026-11-15", "2026-11-28", SprintState::Active),
            sprint("SP-009", "soon", "", SprintState::Planned),
        ];
        let at = |f: &FeatureItem| {
            let (a, b, how) = place(f, &config, &sprints);
            (a, b, how.to_string())
        };
        let s = |a: &str, b: &str, how: &str| (a.to_string(), b.to_string(), how.to_string());

        // 1. Finished: from the first move out of a non-no-op status to the move into its end
        //    status; the sprint it was in does not matter.
        let mut done = item("FEAT-001", "Completed");
        done.sprint = Some("SP-002".into());
        done.history = vec![
            moved("2026-10-01T08:00:00Z", "No Action", "Planned"),
            moved("2026-10-03T08:00:00Z", "Planned", "Scheduled"),
            moved("2026-10-06T08:00:00Z", "Scheduled", "Completed"),
        ];
        assert_eq!(at(&done), s("2026-10-03", "2026-10-06", "moves"));
        // Reopened and finished again: the last move into the end status.
        done.history
            .push(moved("2026-10-08T08:00:00Z", "Completed", "Scheduled"));
        done.history
            .push(moved("2026-10-10T08:00:00Z", "Scheduled", "Completed"));
        assert_eq!(at(&done), s("2026-10-03", "2026-10-10", "moves"));
        // A finished item with no moves recorded: created_at..updated_at, as moves (B-19).
        let old = item("FEAT-002", "Completed");
        assert_eq!(at(&old), s("2026-09-01", "2026-09-09", "moves"));

        // 2. In a known sprint: the sprint's dates.
        let mut in_sprint = item("FEAT-003", "Scheduled");
        in_sprint.sprint = Some("SP-002".into());
        in_sprint.start = Some("2026-12-24".into());
        assert_eq!(at(&in_sprint), s("2026-11-15", "2026-11-28", "sprint"));

        // 3. A start: start .. start + ceil(estimate) − 1, one day with no estimate. An unknown
        //    sprint, or one whose dates do not parse, falls through to the start.
        let mut started = item("FEAT-004", "Planned");
        started.sprint = Some("SP-404".into());
        started.start = Some("2026-12-01".into());
        started.estimate_days = Some(2.5);
        assert_eq!(at(&started), s("2026-12-01", "2026-12-03", "start"));
        started.estimate_days = None;
        started.sprint = Some("SP-009".into());
        assert_eq!(at(&started), s("2026-12-01", "2026-12-01", "start"));

        // 4. Nothing to place it by: undated. 25 of them share no date.
        let undated: Vec<_> = (0..25)
            .map(|i| {
                let mut f = item(&format!("FEAT-{:03}", 100 + i), "Planned");
                if i % 2 == 0 {
                    f.sprint = Some("SP-009".into());
                }
                at(&f)
            })
            .collect();
        assert_eq!(undated.len(), 25);
        assert!(
            undated.iter().all(|p| *p == s("", "", "undated")),
            "{undated:?}"
        );
    }

    /// FTR-043 R-4: the Planned tab is soonest first with undated rows last by code; a milestone
    /// is placed by the latest target of the releases its items are in, or listed undated; and a
    /// milestone's items order by their dependencies (`graph::order_by_deps`, which `row` uses for
    /// `by_status`).
    #[test]
    fn planned_rows_are_soonest_first_and_milestones_carry_their_items_by_status() {
        let (store, codes, dir) = board("r4");
        let tab = slice(&store, "shop", Scope::Planned, TODAY).unwrap();
        assert_eq!(tab.from, "2026-11-17");
        // A quarter from today; the latest row (15 Jan) is inside it.
        assert_eq!(tab.to, "2027-02-16");
        // M1 from today, SP-003 from 29 Nov, v0.4.0 on 15 Jan.
        assert_eq!(
            keys(&tab.lanes),
            ["milestone:M1", "sprint:SP-003", "release:v0.4.0"]
        );
        // Undated last, by code: M3 is in no release with a target, SP-009 has no dates, v0.5.0
        // no target.
        assert_eq!(
            keys(&tab.undated),
            ["milestone:M3", "release:v0.5.0", "sprint:SP-009"]
        );
        // M1 reaches the latest target of v0.3.0 and v0.4.0.
        let m1 = &tab.lanes[0];
        assert_eq!(m1.bands.len(), 1);
        assert_eq!(
            (m1.bands[0].from.as_str(), m1.bands[0].to.as_str()),
            ("2026-11-17", "2027-01-15")
        );
        assert_eq!((m1.done, m1.total), (0, 3));
        assert_eq!(m1.bands[0].progress, Some(0.0));
        let v4 = &tab.lanes[2];
        assert!(v4.bands.is_empty());
        assert_eq!(v4.marks.len(), 1);
        assert_eq!(
            (v4.marks[0].at.as_str(), v4.marks[0].kind.as_str()),
            ("2027-01-15", "diamond")
        );
        // A later row widens the timeline, never past a year.
        let mut releases = crate::releases::load(&store, "shop").unwrap();
        releases.push(release("v0.6.0", "2027-06-01"));
        releases.push(release("v0.7.0", "2028-06-01"));
        crate::atomic::write(
            store.project_dir("shop").join("releases.yaml"),
            crate::yaml::to_string(&releases).unwrap(),
        )
        .unwrap();
        let wide = slice(&store, "shop", Scope::Planned, TODAY).unwrap();
        assert_eq!(wide.to, "2027-11-17");

        // The milestone's items, dependencies first: B needs D, D needs C, E stands alone.
        let deps: BTreeMap<String, Vec<String>> = [
            (codes["B"].clone(), vec![codes["D"].clone()]),
            (
                codes["D"].clone(),
                vec![codes["C"].clone(), "OTHER-1".into()],
            ),
        ]
        .into_iter()
        .collect();
        let input = vec![
            codes["B"].clone(),
            codes["E"].clone(),
            codes["D"].clone(),
            codes["C"].clone(),
        ];
        let ordered =
            crate::graph::order_by_deps(&input, |c| deps.get(c).cloned().unwrap_or_default());
        assert_eq!(
            ordered,
            [
                codes["C"].clone(),
                codes["D"].clone(),
                codes["B"].clone(),
                codes["E"].clone()
            ]
        );
        // A cycle is broken where it is met; every code appears once.
        let cycle: BTreeMap<&str, Vec<String>> =
            [("X", vec!["Y".to_string()]), ("Y", vec!["X".to_string()])]
                .into_iter()
                .collect();
        let ordered = crate::graph::order_by_deps(&["X".to_string(), "Y".to_string()], |c| {
            cycle.get(c).cloned().unwrap_or_default()
        });
        assert_eq!(ordered, ["Y", "X"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-043 R-5: closed sprints and shipped releases, newest first, each with planned,
    /// delivered and carried; rows older than the newest 13 weeks are listed without bands.
    #[test]
    fn past_rows_are_newest_first_with_planned_delivered_and_carried() {
        let (store, _codes, dir) = board("r5");
        let tab = slice(&store, "shop", Scope::Past, TODAY).unwrap();
        // SP-001 ended 14 Nov; v0.1.0 and v0.0.9 shipped 10 Nov, the newer version first; SP-000
        // ended in June.
        assert_eq!(
            keys(&tab.lanes),
            [
                "sprint:SP-001",
                "release:v0.1.0",
                "release:v0.0.9",
                "sprint:SP-000"
            ]
        );
        assert!(tab.undated.is_empty());
        assert_eq!(
            (tab.from.as_str(), tab.to.as_str()),
            ("2026-08-15", "2026-11-14")
        );
        // SP-001 counts items: P1 delivered of P1 and P2 committed; P2 carried to SP-002. The
        // estimate stays beside them: 3 of 5 points.
        let sp1 = &tab.lanes[0];
        assert_eq!((sp1.done, sp1.total, sp1.carried), (1, 2, 1));
        assert_eq!((sp1.points_done, sp1.points_total), (Some(3.0), Some(5.0)));
        assert_eq!(sp1.state, "closed");
        assert_eq!(sp1.bands.len(), 1);
        // v0.1.0: one shipped of one item; C carried to v0.4.0.
        let v1 = &tab.lanes[1];
        assert_eq!((v1.done, v1.total, v1.carried), (1, 1, 1));
        assert_eq!(v1.state, "shipped");
        assert_eq!(
            (v1.marks[0].at.as_str(), v1.marks[0].kind.as_str()),
            ("2026-11-10", "diamond")
        );
        // SP-000 is older than the newest 13 weeks: listed compactly, with no band.
        let sp0 = &tab.lanes[3];
        assert!(sp0.bands.is_empty() && sp0.marks.is_empty());
        assert_eq!(sp0.to, "2026-06-14");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Add an item to `shop` in milestone `ms`, planned into `sprint`; returns its code.
    fn add_item(store: &Store, title: &str, ms: &str, sprint: Option<&str>) -> String {
        let f = store.add_feature("shop", title, "", ms, None).unwrap();
        let mut f = store
            .load_meta("shop")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        f.sprint = sprint.map(str::to_string);
        store.persist_feature_for_test("shop", &f).unwrap();
        f.code
    }

    /// FTR-043 R-3: an opened row lists its items, each placed by `place` and marked when it is on
    /// the dependency critical path; a milestone's items come by status, dependencies first; a
    /// sprint's and a release's carries come with them; an unknown row is a 404.
    #[test]
    fn a_rows_items_mark_the_critical_path() {
        let (store, codes, dir) = board("row");
        // In SP-003, in M4: W needs X, X needs Y (a chain of three), and Z stands alone. W is
        // added first, so the board's order is not the dependency order.
        let w = add_item(&store, "W", "M4", Some("SP-003"));
        let x = add_item(&store, "X", "M4", Some("SP-003"));
        let y = add_item(&store, "Y", "M4", Some("SP-003"));
        let z = add_item(&store, "Z", "M4", Some("SP-003"));
        let needs = |code: &str, dep: &str| {
            store
                .set_feature_attrs(
                    "shop",
                    code,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec![dep.to_string()]),
                )
                .unwrap();
        };
        needs(&w, &x);
        needs(&x, &y);
        store.move_feature("shop", &z, "Scheduled").unwrap();

        let body = row(&store, "shop", "sprint:SP-003", TODAY).unwrap();
        assert_eq!(body.key, "sprint:SP-003");
        let got: Vec<(&str, &str, &str, &str, bool)> = body
            .items
            .iter()
            .map(|i| {
                (
                    i.code.as_str(),
                    i.from.as_str(),
                    i.to.as_str(),
                    i.placed.as_str(),
                    i.critical,
                )
            })
            .collect();
        fn sp3(c: &str, critical: bool) -> (&str, &str, &str, &str, bool) {
            (c, "2026-11-29", "2026-12-12", "sprint", critical)
        }
        assert_eq!(
            got,
            [
                sp3(w.as_str(), true),
                sp3(x.as_str(), true),
                sp3(y.as_str(), true),
                sp3(z.as_str(), false)
            ]
        );
        assert!(body.items.iter().all(|i| i.pending.is_empty()));
        assert_eq!(body.items[0].title, "W");
        assert!(body.by_status.is_empty() && body.carried.is_empty());

        // The milestone's items by displayed status, in the workflow's order, dependencies first.
        let config = store.load_meta("shop").unwrap().config;
        let first = store
            .load_meta("shop")
            .unwrap()
            .feature(&w)
            .unwrap()
            .status
            .clone();
        let pos = |s: &str| config.displayed_states.iter().position(|d| d == s).unwrap();
        assert!(
            pos(&first) < pos("Scheduled"),
            "{:?}",
            config.displayed_states
        );
        let m4 = row(&store, "shop", "milestone:M4", TODAY).unwrap();
        assert_eq!(
            m4.by_status,
            [
                (first, vec![y.clone(), x.clone(), w.clone()]),
                ("Scheduled".to_string(), vec![z.clone()])
            ]
        );
        assert_eq!(m4.items.len(), 4);

        // A sprint's carries, with no why; a release's, with its own.
        let sp1 = row(&store, "shop", "sprint:SP-001", TODAY).unwrap();
        assert_eq!(
            sp1.carried
                .iter()
                .map(|c| (c.code.as_str(), c.to.as_str(), c.why.as_str()))
                .collect::<Vec<_>>(),
            [(codes["P2"].as_str(), "SP-002", "")]
        );
        assert_eq!(
            sp1.items
                .iter()
                .map(|i| i.code.as_str())
                .collect::<Vec<_>>(),
            [codes["P1"].as_str()]
        );
        let v1 = row(&store, "shop", "release:v0.1.0", TODAY).unwrap();
        assert_eq!(
            v1.items
                .iter()
                .map(|i| (i.code.as_str(), i.placed.as_str()))
                .collect::<Vec<_>>(),
            [(codes["P1"].as_str(), "moves")]
        );
        assert_eq!(v1.carried[0].code, codes["C"]);
        assert_eq!(v1.carried[0].to, "v0.4.0");

        // An unknown row, or a key of no kind, is a 404.
        for key in [
            "sprint:SP-404",
            "release:v9",
            "milestone:M9",
            "SP-003",
            "goal:G-1",
        ] {
            let e = row(&store, "shop", key, TODAY).unwrap_err();
            assert_eq!(e.status(), 404, "{key}: {e}");
        }
        assert_eq!(
            row(&store, "nowhere", "sprint:SP-001", TODAY)
                .unwrap_err()
                .status(),
            404
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-043 R-7: the portfolio's timeline has one row per project with sprints or releases on:
    /// its running sprint's band and its releases-in-flight diamonds. One with sprints off has no
    /// band; one with neither cadence has no row.
    #[test]
    fn the_portfolio_timeline_has_one_row_per_project() {
        let (store, _codes, dir) = board("portfolio");
        store
            .init_project("ops", ProjectConfig::default_for("ops"))
            .unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/ops/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        crate::atomic::write(
            store.project_dir("ops").join("releases.yaml"),
            crate::yaml::to_string(&vec![
                Release {
                    stage: "candidate".into(),
                    ..release("r1", "2026-11-21")
                },
                release("r2", "2026-11-22"),
            ])
            .unwrap(),
        )
        .unwrap();
        store
            .init_project("bare", ProjectConfig::default_for("bare"))
            .unwrap();

        let plan = portfolio(&store, TODAY).unwrap();
        // The window is the Current tab's, over the running sprints the lanes draw, so the web
        // never works it out (the coordinator, 17 Nov 2026).
        assert_eq!(
            (plan.today.as_str(), plan.from.as_str(), plan.to.as_str()),
            ("2026-11-17", "2026-11-03", "2026-12-01"),
            "{plan:?}"
        );
        let lanes = plan.lanes;
        let mut projects: Vec<&str> = lanes.iter().map(|l| l.project.as_str()).collect();
        projects.sort();
        assert_eq!(projects, ["ops", "shop"], "{lanes:?}");
        let lane = |p: &str| lanes.iter().find(|l| l.project == p).unwrap();
        let shop = lane("shop");
        assert!(!shop.label.is_empty());
        assert_eq!(
            shop.bands
                .iter()
                .map(|b| (b.from.as_str(), b.to.as_str()))
                .collect::<Vec<_>>(),
            [("2026-11-15", "2026-11-28")]
        );
        assert_eq!(
            shop.marks
                .iter()
                .map(|m| (m.at.as_str(), m.kind.as_str(), m.label.as_str()))
                .collect::<Vec<_>>(),
            [("2026-11-20", "diamond", "v0.2.0")]
        );
        // Sprints off: no band; only the release at a stage is in flight.
        let ops = lane("ops");
        assert!(ops.bands.is_empty(), "{ops:?}");
        assert_eq!(
            ops.marks
                .iter()
                .map(|m| (m.at.as_str(), m.kind.as_str()))
                .collect::<Vec<_>>(),
            [("2026-11-21", "diamond")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
