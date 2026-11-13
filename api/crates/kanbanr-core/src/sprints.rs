//! Sprints: timeboxes with a goal, a capacity, and a burndown (FEAT-119).
//!
//! Only for projects that switch them on (`cadence.sprints`, FEAT-121) — most projects follow a
//! different rhythm, and a sprint file nobody asked for is clutter. A sprint is its own record,
//! not a milestone with dates: a milestone is a theme with dependencies, a sprint is a fortnight,
//! and an item belongs to one of each.
//!
//! Stored beside the charter in `projects/<id>/sprints.yaml`. Burndown and velocity are never
//! stored: they are derived from the moves every item already records, so they cannot disagree
//! with what happened.

use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::store::{Capability, Project, Store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SPRINTS_FILE: &str = "sprints.yaml";

/// Where a sprint is in its life.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SprintState {
    #[default]
    Planned,
    Active,
    Closed,
}

/// An item that left a sprint unfinished, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(rename = "SprintCarry")
)]
pub struct Carry {
    pub code: String,
    /// The sprint it moved to, or `backlog`.
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Sprint {
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What this sprint is for, in a sentence.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub goal: String,
    /// First day, `YYYY-MM-DD`.
    pub start: String,
    /// Last day, `YYYY-MM-DD`, inclusive.
    pub end: String,
    /// What the sprint can take, in the project's estimate unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<f64>,
    #[serde(default)]
    pub state: SprintState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub closed_at: String,
    /// What left unfinished, and where it went — the record a retro reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<Carry>,
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(SPRINTS_FILE)
}

/// A project's sprints, oldest first. None is a normal state, never an error.
pub fn load(store: &Store, id: &str) -> Result<Vec<Sprint>> {
    match std::fs::read_to_string(path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(crate::yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, sprints: &[Sprint]) -> Result<()> {
    crate::atomic::write(path(store, id), crate::yaml::to_string(sprints)?)?;
    Ok(())
}

impl Sprint {
    /// The sprint's state on `today`: one stored `Active` whose first day is still ahead is
    /// `Planned` (DEF-041). A start that does not parse keeps the stored state.
    pub fn state_on(&self, today: time::Date) -> SprintState {
        if self.state == SprintState::Active
            && crate::gantt::parse_ymd(&self.start).is_some_and(|first| first > today)
        {
            SprintState::Planned
        } else {
            self.state
        }
    }
}

/// The sprint running on `today`, if one is.
pub fn active_on(sprints: &[Sprint], today: time::Date) -> Option<&Sprint> {
    sprints
        .iter()
        .find(|s| s.state_on(today) == SprintState::Active)
}

/// The sprint currently running, if one is.
pub fn active(sprints: &[Sprint]) -> Option<&Sprint> {
    active_on(sprints, today())
}

/// The sprints as they read on `today`, for showing only. Never saved: reading a board never
/// rewrites it (ADR-0005).
pub fn as_of(sprints: &[Sprint], today: time::Date) -> Vec<Sprint> {
    sprints
        .iter()
        .map(|s| Sprint {
            state: s.state_on(today),
            ..s.clone()
        })
        .collect()
}

/// A first day: a date, or the word `today`.
fn first_day(x: &str) -> Option<time::Date> {
    if x.trim().eq_ignore_ascii_case("today") {
        Some(today())
    } else {
        crate::gantt::parse_ymd(x)
    }
}

fn find<'a>(sprints: &'a mut [Sprint], code: &str) -> Result<&'a mut Sprint> {
    sprints
        .iter_mut()
        .find(|s| s.code == code)
        .ok_or_else(|| CoreError::Unsupported(format!("no sprint {code}")))
}

fn cadence_on(store: &Store, id: &str) -> Result<Project> {
    let project = store.load(id)?;
    Store::require_cadence(&project, Capability::Sprints)?;
    Ok(project)
}

/// Add a sprint starting on `start`, `length_days` long (the project's default when not given,
/// else fourteen).
pub fn add(
    store: &Store,
    id: &str,
    start: &str,
    length_days: Option<u32>,
    goal: &str,
    capacity: Option<f64>,
    name: &str,
) -> Result<Sprint> {
    let project = cadence_on(store, id)?;
    let first = first_day(start).ok_or_else(|| {
        CoreError::Unsupported(format!("'{start}' is not a date (YYYY-MM-DD, or today)"))
    })?;
    let length = length_days
        .or(project.config.cadence.sprint_length_days)
        .unwrap_or(14)
        .max(1);
    let last = first
        .checked_add(time::Duration::days(i64::from(length) - 1))
        .unwrap_or(first);
    let mut sprints = load(store, id)?;
    // Numbered like milestones (MS-001), so a sprint code sorts and reads like the board's other
    // codes.
    let next = sprints
        .iter()
        .filter_map(|s| s.code.strip_prefix("SP-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let code = format!("SP-{next:03}");
    let sprint = Sprint {
        name: if name.trim().is_empty() {
            format!("Sprint {}", sprints.len() + 1)
        } else {
            name.trim().to_string()
        },
        code,
        goal: goal.trim().to_string(),
        start: crate::gantt::fmt_date(first),
        end: crate::gantt::fmt_date(last),
        capacity: capacity.filter(|c| *c > 0.0),
        ..Sprint::default()
    };
    sprints.push(sprint.clone());
    save(store, id, &sprints)?;
    Ok(sprint)
}

/// What to change on a planned sprint. Fields left `None` are not touched.
#[derive(Debug, Clone, Default)]
pub struct SprintEdit {
    pub name: Option<String>,
    pub goal: Option<String>,
    pub start: Option<String>,
    pub length_days: Option<u32>,
    pub end: Option<String>,
    /// `Some(None)` clears the capacity.
    pub capacity: Option<Option<f64>>,
}

/// Change a sprint that has not started: its plan is the planner's to reshape. Once started its
/// dates and goal are the record of what happened (burndown and retro read them), so it is refused.
/// Nothing is written until every field checks.
pub fn edit(store: &Store, id: &str, code: &str, edit: &SprintEdit) -> Result<Sprint> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    let sprint = find(&mut sprints, code)?;
    match sprint.state {
        SprintState::Planned => {}
        SprintState::Active => {
            return Err(CoreError::Unsupported(format!(
                "{code} has started — its dates and goal are the record of what happened; add a sprint instead"
            )));
        }
        SprintState::Closed => {
            return Err(CoreError::Unsupported(format!(
                "{code} is closed — its dates and goal are the record of what happened; add a sprint instead"
            )));
        }
    }
    let date = |x: &str| {
        crate::gantt::parse_ymd(x)
            .ok_or_else(|| CoreError::Unsupported(format!("'{x}' is not a date (YYYY-MM-DD)")))
    };
    if edit.length_days.is_some() && edit.end.is_some() {
        return Err(CoreError::Unsupported(
            "give a length or an end, not both".into(),
        ));
    }
    let old_start = date(&sprint.start)?;
    let old_end = date(&sprint.end)?;
    let new_start = match &edit.start {
        Some(x) => first_day(x).ok_or_else(|| {
            CoreError::Unsupported(format!("'{x}' is not a date (YYYY-MM-DD, or today)"))
        })?,
        None => old_start,
    };
    let new_end = if let Some(x) = &edit.end {
        date(x)?
    } else if let Some(length) = edit.length_days {
        new_start
            .checked_add(time::Duration::days(i64::from(length.max(1)) - 1))
            .unwrap_or(new_start)
    } else {
        // Only the start moved (or nothing did): keep the length.
        new_start
            .checked_add(old_end - old_start)
            .unwrap_or(new_start)
    };
    if new_end < new_start {
        return Err(CoreError::Unsupported(format!(
            "{code} would end ({}) before it starts ({})",
            crate::gantt::fmt_date(new_end),
            crate::gantt::fmt_date(new_start)
        )));
    }
    let name = match &edit.name {
        Some(n) if n.trim().is_empty() => {
            return Err(CoreError::Unsupported(
                "a sprint's name cannot be empty".into(),
            ));
        }
        Some(n) => Some(n.trim().to_string()),
        None => None,
    };
    if let Some(Some(c)) = edit.capacity
        && c <= 0.0
    {
        return Err(CoreError::Unsupported(
            "capacity must be more than zero (to clear it, give none)".into(),
        ));
    }
    if let Some(n) = name {
        sprint.name = n;
    }
    if let Some(g) = &edit.goal {
        sprint.goal = g.trim().to_string();
    }
    sprint.start = crate::gantt::fmt_date(new_start);
    sprint.end = crate::gantt::fmt_date(new_end);
    if let Some(c) = edit.capacity {
        sprint.capacity = c;
    }
    let edited = sprint.clone();
    save(store, id, &sprints)?;
    Ok(edited)
}

/// Remove a sprint that holds nothing: planned, with no item in it. Anything else would lose a
/// reference (an item's sprint, a burndown, a retro's carried list), so it is refused with why.
pub fn remove(store: &Store, id: &str, code: &str) -> Result<Sprint> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    let at = sprints
        .iter()
        .position(|s| s.code == code)
        .ok_or_else(|| CoreError::Unsupported(format!("no sprint {code}")))?;
    match sprints[at].state {
        SprintState::Planned => {}
        SprintState::Active => {
            return Err(CoreError::Unsupported(format!(
                "{code} has started — close it (`kanbanr sprint close {code}`) rather than remove it"
            )));
        }
        SprintState::Closed => {
            return Err(CoreError::Unsupported(format!(
                "{code} is closed — it is the record of a sprint that ran, so it stays"
            )));
        }
    }
    let project = store.load_meta(id)?;
    let held: Vec<&str> = project
        .features
        .iter()
        .filter(|f| f.sprint.as_deref() == Some(code))
        .map(|f| f.code.as_str())
        .collect();
    if !held.is_empty() {
        let shown: Vec<&str> = held.iter().copied().take(10).collect();
        let more = held.len().saturating_sub(shown.len());
        return Err(CoreError::Unsupported(format!(
            "{code} holds {}{} — only an empty sprint is removed; plan the items into another sprint first",
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        )));
    }
    let removed = sprints.remove(at);
    save(store, id, &sprints)?;
    Ok(removed)
}

/// What planning some items into a sprint produced: the sprint, and a warning when the committed
/// estimate goes over its capacity.
#[derive(Debug, Clone, Serialize)]
pub struct Planned {
    pub sprint: Sprint,
    pub committed: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub over_capacity: Option<String>,
}

/// Plan items into a sprint. Over capacity is a warning, not a refusal: the capacity is a guess,
/// and the person planning knows things it does not.
pub fn plan(store: &Store, id: &str, code: &str, items: &[String]) -> Result<Planned> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    let sprint = find(&mut sprints, code)?.clone();
    if sprint.state == SprintState::Closed {
        return Err(CoreError::Unsupported(format!(
            "{code} is closed — plan into the current or a coming sprint"
        )));
    }
    for item in items {
        store.set_feature_sprint(id, item, Some(code.to_string()))?;
    }
    let project = store.load_meta(id)?;
    let committed = committed(&project, code);
    let over_capacity = sprint.capacity.filter(|cap| committed > *cap).map(|cap| {
        format!(
            "{code} now holds {committed} {} against a capacity of {cap}",
            unit_name(&project)
        )
    });
    Ok(Planned {
        sprint,
        committed,
        over_capacity,
    })
}

/// One item taken out of a sprint.
#[derive(Debug, Clone, Serialize)]
pub struct Left {
    pub code: String,
    pub from: String,
}

/// What taking items out of their sprints did.
#[derive(Debug, Clone, Serialize)]
pub struct Unplanned {
    /// Items that were in a sprint and are now in none.
    pub removed: Vec<Left>,
    /// Items named that were in no sprint; nothing changed for them.
    pub not_in_a_sprint: Vec<String>,
}

/// Take items out of their sprint (DEF-018). A closed sprint keeps its items as its record, and an
/// unknown item is an error; both are refused before anything is written.
pub fn unplan(store: &Store, id: &str, items: &[String]) -> Result<Unplanned> {
    cadence_on(store, id)?;
    let sprints = load(store, id)?;
    let project = store.load(id)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for code in items {
        if !seen.insert(code.clone()) {
            continue;
        }
        let sprint = project.feature(code)?.sprint.clone();
        if let Some(s) = &sprint
            && sprints
                .iter()
                .any(|x| &x.code == s && x.state == SprintState::Closed)
        {
            return Err(CoreError::Unsupported(format!(
                "{code} is in {s}, which is closed; a closed sprint keeps its items as its record"
            )));
        }
        found.push((code.clone(), sprint));
    }
    let mut out = Unplanned {
        removed: Vec::new(),
        not_in_a_sprint: Vec::new(),
    };
    for (code, sprint) in found {
        match sprint {
            Some(from) => {
                store.set_feature_sprint(id, &code, None)?;
                out.removed.push(Left { code, from });
            }
            None => out.not_in_a_sprint.push(code),
        }
    }
    Ok(out)
}

/// Make a sprint the active one. Only one runs at a time.
pub fn start(store: &Store, id: &str, code: &str) -> Result<Sprint> {
    start_on(store, id, code, today())
}

/// [`start`] on a given day. A sprint dated to begin after `today` starts today instead, keeping
/// its length, so the record is true (DEF-041).
pub(crate) fn start_on(store: &Store, id: &str, code: &str, today: time::Date) -> Result<Sprint> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    if let Some(running) = active_on(&sprints, today)
        && running.code != code
    {
        return Err(CoreError::Unsupported(format!(
            "{} is still active — close it first (`kanbanr sprint close {}`)",
            running.code, running.code
        )));
    }
    let sprint = find(&mut sprints, code)?;
    if sprint.state == SprintState::Closed {
        return Err(CoreError::Unsupported(format!("{code} is already closed")));
    }
    if let Some(first) = crate::gantt::parse_ymd(&sprint.start)
        && first > today
    {
        let last = crate::gantt::parse_ymd(&sprint.end).unwrap_or(first);
        let end = today.checked_add(last - first).ok_or_else(|| {
            CoreError::Unsupported(format!("{code}'s dates cannot be moved to today"))
        })?;
        sprint.start = crate::gantt::fmt_date(today);
        sprint.end = crate::gantt::fmt_date(end);
    }
    sprint.state = SprintState::Active;
    let started = sprint.clone();
    save(store, id, &sprints)?;
    Ok(started)
}

/// Close a sprint. Its unfinished items move to `carry_to` (another sprint's code) or back to the
/// backlog, and each move is recorded on the sprint, so a retro can say what did not fit.
pub fn close(store: &Store, id: &str, code: &str, carry_to: Option<&str>) -> Result<Sprint> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    let target = match carry_to.map(str::trim) {
        None | Some("") | Some("backlog") => None,
        Some(next) => {
            let next_sprint = sprints
                .iter()
                .find(|s| s.code == next)
                .ok_or_else(|| CoreError::Unsupported(format!("no sprint {next}")))?;
            if next_sprint.state == SprintState::Closed {
                return Err(CoreError::Unsupported(format!(
                    "{next} is closed — carry to a sprint that is still to run"
                )));
            }
            Some(next.to_string())
        }
    };
    let project = store.load_meta(id)?;
    let unfinished: Vec<String> = project
        .features
        .iter()
        .filter(|f| f.sprint.as_deref() == Some(code) && !done(&project, f))
        .map(|f| f.code.clone())
        .collect();
    for item in &unfinished {
        store.set_feature_sprint(id, item, target.clone())?;
    }
    let sprint = find(&mut sprints, code)?;
    sprint.state = SprintState::Closed;
    sprint.closed_at = crate::now_rfc3339();
    sprint.carried.extend(unfinished.into_iter().map(|c| Carry {
        code: c,
        to: target.clone().unwrap_or_else(|| "backlog".to_string()),
    }));
    let closed = sprint.clone();
    save(store, id, &sprints)?;
    Ok(closed)
}

/// Finished for the sprint: at a stage that counts as done — an end status, or one the workflow
/// marks `done` (FEAT-137). An item Done but not yet released is not carried over; one sent back
/// from Done for rework is.
fn done(project: &Project, f: &FeatureItem) -> bool {
    project.config.counts_as_done(&f.status)
}

/// An item's size in the project's unit; an unestimated item counts as nothing.
fn size(project: &Project, f: &FeatureItem) -> f64 {
    match project.config.estimate_unit {
        crate::config::EstimateUnit::Points => f.points.unwrap_or(0.0),
        crate::config::EstimateUnit::Days => f.estimate_days.unwrap_or(0.0),
    }
}

fn unit_name(project: &Project) -> &'static str {
    match project.config.estimate_unit {
        crate::config::EstimateUnit::Points => "points",
        crate::config::EstimateUnit::Days => "days",
    }
}

/// The estimate committed to a sprint: the items planned into it now.
pub fn committed(project: &Project, code: &str) -> f64 {
    total(
        project
            .features
            .iter()
            .filter(|f| f.sprint.as_deref() == Some(code))
            .map(|f| size(project, f)),
    )
}

/// A sum that reads as a person would write it: floating-point addition of nothing is `-0.0`.
fn total(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |a, b| a + b)
}

/// When an item first reached an end status, from its recorded moves.
fn finished_on(project: &Project, f: &FeatureItem) -> Option<time::Date> {
    f.history
        .iter()
        .find(|t| project.config.counts_as_done(&t.to))
        .and_then(|t| crate::gantt::parse_ymd(&t.at))
}

/// One day of a burndown: what was still to do at the end of it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BurndownDay {
    pub date: String,
    pub remaining: f64,
}

/// A sprint as a report: its record, what it holds, and how it burned down.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SprintReport {
    #[serde(flatten)]
    pub sprint: Sprint,
    pub unit: String,
    pub items: Vec<String>,
    pub committed: f64,
    pub done: f64,
    /// Items with no estimate in the project's unit: they count as nothing, which is said, not hidden.
    pub unestimated: Vec<String>,
    /// Days left, counting today; 0 once the sprint is over.
    pub days_left: i64,
    pub burndown: Vec<BurndownDay>,
}

/// Report a sprint. The burndown is derived from the moves the items recorded — the scope is what
/// the sprint holds (plus what it carried out, which it was committed to), and each day's
/// remaining is that scope less what had reached an end status by then.
pub fn report(store: &Store, id: &str, code: &str, today: time::Date) -> Result<SprintReport> {
    let project = store.load_meta(id)?;
    Store::require_cadence(&project, Capability::Sprints)?;
    let sprints = load(store, id)?;
    let sprint = sprints
        .iter()
        .find(|s| s.code == code)
        .cloned()
        .ok_or_else(|| CoreError::Unsupported(format!("no sprint {code}")))?;
    let carried_out: Vec<&str> = sprint.carried.iter().map(|c| c.code.as_str()).collect();
    let scope: Vec<&FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.sprint.as_deref() == Some(code) || carried_out.contains(&f.code.as_str()))
        .collect();
    let committed = total(scope.iter().map(|f| size(&project, f)));
    let first = crate::gantt::parse_ymd(&sprint.start).unwrap_or(today);
    let last = crate::gantt::parse_ymd(&sprint.end).unwrap_or(today);
    let until = if today < last { today } else { last };
    let mut burndown = Vec::new();
    let mut day = first;
    while day <= until {
        let burned = total(
            scope
                .iter()
                .filter(|f| {
                    !carried_out.contains(&f.code.as_str())
                        && finished_on(&project, f).is_some_and(|d| d <= day)
                })
                .map(|f| size(&project, f)),
        );
        burndown.push(BurndownDay {
            date: crate::gantt::fmt_date(day),
            remaining: committed - burned,
        });
        match day.next_day() {
            Some(next) => day = next,
            None => break,
        }
    }
    let done = total(
        scope
            .iter()
            .filter(|f| !carried_out.contains(&f.code.as_str()) && done(&project, f))
            .map(|f| size(&project, f)),
    );
    let days_left = if today > last {
        0
    } else {
        (last - today).whole_days() + 1
    };
    Ok(SprintReport {
        unit: unit_name(&project).to_string(),
        items: scope.iter().map(|f| f.code.clone()).collect(),
        committed,
        done,
        unestimated: scope
            .iter()
            .filter(|f| size(&project, f) == 0.0)
            .map(|f| f.code.clone())
            .collect(),
        days_left,
        burndown,
        sprint,
    })
}

/// Velocity: what each closed sprint finished, in the project's unit. `None` where the project does
/// not use sprints — a burn rate for a project with no sprints would be a number about nothing.
pub fn velocity(store: &Store, id: &str) -> Result<Option<Vec<(String, f64)>>> {
    let project = store.load_meta(id)?;
    if !project.config.cadence.sprints {
        return Ok(None);
    }
    let sprints = load(store, id)?;
    Ok(Some(
        sprints
            .iter()
            .filter(|s| s.state == SprintState::Closed)
            .map(|s| {
                let carried: Vec<&str> = s.carried.iter().map(|c| c.code.as_str()).collect();
                let finished = total(
                    project
                        .features
                        .iter()
                        .filter(|f| f.sprint.as_deref() == Some(s.code.as_str()))
                        .filter(|f| !carried.contains(&f.code.as_str()) && done(&project, f))
                        .map(|f| size(&project, f)),
                );
                (s.code.clone(), finished)
            })
            .collect(),
    ))
}

/// A live item planned into a release that has not shipped and into no sprint, while a sprint runs
/// (DEF-029).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutsideSprint {
    pub code: String,
    pub release: String,
    /// The sprint that is running.
    pub sprint: String,
}

/// Live items planned into a release that has not shipped, in no sprint, while a sprint runs.
/// Empty unless the project runs sprints and one is active. With `only`, just those items (the ones
/// a plan just put into a release). Sorted by code. Reads only; planning into the sprint is the
/// person's or Claude's own command.
pub fn outside_running_sprint(
    project: &Project,
    sprints: &[Sprint],
    releases: &[crate::releases::Release],
    only: Option<&[String]>,
) -> Vec<OutsideSprint> {
    if !project.config.cadence.sprints {
        return Vec::new();
    }
    let Some(running) = active(sprints) else {
        return Vec::new();
    };
    let mut found: Vec<OutsideSprint> = project
        .features
        .iter()
        .filter(|f| f.sprint.is_none())
        .filter(|f| only.is_none_or(|o| o.contains(&f.code)))
        .filter(|f| !crate::graph::is_terminal_status(&project.config, &f.status))
        .filter_map(|f| {
            let version = f.release.as_deref()?;
            releases
                .iter()
                .any(|r| r.version == version && r.state != crate::releases::ReleaseState::Shipped)
                .then(|| OutsideSprint {
                    code: f.code.clone(),
                    release: version.to_string(),
                    sprint: running.code.clone(),
                })
        })
        .collect();
    found.sort_by(|a, b| a.code.cmp(&b.code));
    found
}

/// Today's date (UTC, as every timestamp on the board is).
pub fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

#[cfg(test)]
mod tests {
    use crate::{ProjectConfig, Store};

    /// A store whose project has sprints and points on, three estimated items planned into SP-001
    /// (5–9 Oct), and the given gates.
    fn board(
        tag: &str,
        gates: std::collections::BTreeMap<String, crate::config::Gate>,
    ) -> (Store, Vec<String>, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-sprints-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let scrum = crate::config::preset("scrum").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = scrum.statuses.clone();
        config.default_state = scrum.default_state.clone();
        config.displayed_states = scrum.displayed_states.clone();
        config.terminal_states = scrum.terminal_states.clone();
        config.no_op_states = scrum.no_op_states.clone();
        config.transitions = scrum.transitions.clone();
        config.gates = gates;
        store.init_project("shop", config).unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": true, "estimate_unit": "points"})),
        )
        .unwrap();
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let mut codes = Vec::new();
        for (title, points) in [("A", 3.0), ("B", 5.0), ("C", 8.0)] {
            let f = store.add_feature("shop", title, "", "M", None).unwrap();
            store.set_feature_points("shop", &f.code, points).unwrap();
            codes.push(f.code);
        }
        let sp = super::add(&store, "shop", "2026-10-05", Some(5), "", None, "").unwrap();
        super::plan(&store, "shop", &sp.code, &codes).unwrap();
        (store, codes, dir)
    }

    /// DEF-018 R-1: one item leaves its sprint; the others and the sprint's record stay.
    #[test]
    fn unplan_takes_an_item_out_of_its_sprint_and_leaves_the_rest() {
        let (store, codes, dir) = board("def018-r1", Default::default());
        let before = std::fs::read(super::path(&store, "shop")).unwrap();
        let out = super::unplan(&store, "shop", &codes[..1]).unwrap();
        assert_eq!(out.removed.len(), 1);
        assert_eq!(out.removed[0].code, codes[0]);
        assert_eq!(out.removed[0].from, "SP-001");
        assert!(out.not_in_a_sprint.is_empty());
        let project = store.load("shop").unwrap();
        assert_eq!(project.feature(&codes[0]).unwrap().sprint, None);
        for c in &codes[1..] {
            assert_eq!(
                project.feature(c).unwrap().sprint.as_deref(),
                Some("SP-001")
            );
        }
        assert_eq!(before, std::fs::read(super::path(&store, "shop")).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-018 R-2: an item in no sprint is named, not reported removed.
    #[test]
    fn unplan_names_an_item_that_was_in_no_sprint() {
        let (store, codes, dir) = board("def018-r2", Default::default());
        super::unplan(&store, "shop", &codes[..1]).unwrap();
        let out = super::unplan(&store, "shop", &codes[..1]).unwrap();
        assert!(out.removed.is_empty());
        assert_eq!(out.not_in_a_sprint, vec![codes[0].clone()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-018 R-3: a closed sprint and an unknown item refuse the whole command.
    #[test]
    fn unplan_refuses_a_closed_sprint_and_an_unknown_item_before_changing_anything() {
        let gates = crate::config::preset("scrum").unwrap().gates;
        let (store, codes, dir) = board("def018-r3", gates);
        // Finish A so closing SP-001 leaves it in the closed sprint; B and C are carried to SP-002.
        let sp2 = super::add(&store, "shop", "2026-10-12", Some(5), "", None, "").unwrap();
        arrive(&store, &codes[0], "Done", "2026-10-06T10:00:00Z");
        super::close(&store, "shop", "SP-001", Some(&sp2.code)).unwrap();
        let err = super::unplan(&store, "shop", &codes[..1])
            .unwrap_err()
            .to_string();
        assert!(err.contains(&codes[0]) && err.contains("SP-001"), "{err}");
        let both = vec![codes[1].clone(), "FEAT-999".to_string()];
        let err = super::unplan(&store, "shop", &both)
            .unwrap_err()
            .to_string();
        assert!(err.contains("FEAT-999"), "{err}");
        let project = store.load("shop").unwrap();
        assert_eq!(
            project.feature(&codes[1]).unwrap().sprint.as_deref(),
            Some(sp2.code.as_str())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-018 R-4: the command needs sprints switched on.
    #[test]
    fn unplan_needs_sprints_switched_on() {
        let (store, codes, dir) = board("def018-r4", Default::default());
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": false})),
        )
        .unwrap();
        assert!(super::unplan(&store, "shop", &codes[..1]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-029 R-5: items planned into a release while a sprint runs are named when they are in no
    /// sprint; nothing plans them in.
    #[test]
    fn planning_into_a_release_names_items_outside_the_running_sprint() {
        let (store, codes, dir) = board("def029-outside", Default::default());
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": true, "releases": true})),
        )
        .unwrap();
        let outside = |store: &Store, only: Option<&[String]>| {
            let project = store.load("shop").unwrap();
            super::outside_running_sprint(
                &project,
                &super::load(store, "shop").unwrap(),
                &crate::releases::load(store, "shop").unwrap(),
                only,
            )
        };
        crate::releases::add(&store, "shop", "v1", "", "").unwrap();
        let left_out = store.add_feature("shop", "D", "", "M", None).unwrap().code;
        let both: Vec<String> = vec![codes[0].clone(), left_out.clone()];
        crate::releases::plan(&store, "shop", "v1", &both).unwrap();
        // No sprint runs yet: nothing to say.
        assert!(outside(&store, Some(&both)).is_empty());
        super::start(&store, "shop", "SP-001").unwrap();
        let found = outside(&store, Some(&both));
        assert_eq!(
            found,
            vec![super::OutsideSprint {
                code: left_out.clone(),
                release: "v1".into(),
                sprint: "SP-001".into(),
            }]
        );
        // Only what was asked about, and nothing was planned into the sprint.
        assert!(outside(&store, Some(&codes[..1])).is_empty());
        assert_eq!(outside(&store, None).len(), 1);
        let project = store.load("shop").unwrap();
        assert!(project.feature(&left_out).unwrap().sprint.is_none());
        // Planned into the sprint, it is no longer named.
        super::plan(&store, "shop", "SP-001", std::slice::from_ref(&left_out)).unwrap();
        assert!(outside(&store, None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Walk an item along the scrum stages to `status`, then date its arrival there `at`, as the
    /// move history would have recorded it.
    fn arrive(store: &Store, code: &str, status: &str, at: &str) {
        const STAGES: [&str; 7] = [
            "Backlog",
            "Ready",
            "In Progress",
            "Review",
            "Testing",
            "Done",
            "Released",
        ];
        let from = store
            .load("shop")
            .unwrap()
            .feature(code)
            .unwrap()
            .status
            .clone();
        let start = STAGES.iter().position(|s| *s == from).unwrap();
        let end = STAGES.iter().position(|s| *s == status).unwrap();
        for stage in &STAGES[start + 1..=end] {
            store.move_feature("shop", code, stage).unwrap();
        }
        let mut f = store.load("shop").unwrap().feature(code).unwrap().clone();
        f.history.last_mut().unwrap().at = at.into();
        store.persist_feature_for_test("shop", &f).unwrap();
    }

    fn remaining(store: &Store) -> Vec<f64> {
        let today = crate::gantt::parse_ymd("2026-10-09").unwrap();
        super::report(store, "shop", "SP-001", today)
            .unwrap()
            .burndown
            .iter()
            .map(|d| d.remaining)
            .collect()
    }

    /// FEAT-137 R-1: with Done marked, an item burns down the day it reaches Done, not the day a
    /// release ships it.
    #[test]
    fn a_stage_marked_done_burns_down_the_day_it_is_reached() {
        let gates = crate::config::preset("scrum").unwrap().gates;
        assert!(gates["Done"].done, "the preset marks Done");
        let (store, codes, dir) = board("marked", gates);
        arrive(&store, &codes[0], "Done", "2026-10-06T10:00:00Z");
        arrive(&store, &codes[1], "Done", "2026-10-08T16:00:00Z");
        arrive(&store, &codes[1], "Released", "2026-10-09T09:00:00Z");
        assert_eq!(remaining(&store), [16.0, 13.0, 13.0, 8.0, 8.0]);
        // And closing the sprint carries only what is not Done.
        let closed = super::close(&store, "shop", "SP-001", Some("backlog")).unwrap();
        let carried: Vec<&str> = closed.carried.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(carried, [codes[2].as_str()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-137 R-2: with no stage marked, the end status alone counts — as before.
    #[test]
    fn without_a_marked_stage_the_end_statuses_count() {
        let mut gates = crate::config::preset("scrum").unwrap().gates;
        for gate in gates.values_mut() {
            gate.done = false;
        }
        let (store, codes, dir) = board("unmarked", gates);
        arrive(&store, &codes[0], "Done", "2026-10-06T10:00:00Z");
        arrive(&store, &codes[1], "Done", "2026-10-07T10:00:00Z");
        arrive(&store, &codes[1], "Released", "2026-10-08T16:00:00Z");
        assert_eq!(remaining(&store), [16.0, 16.0, 16.0, 11.0, 11.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-025 R-1: under TOGAF an item at Migration is finished for the sprint, so closing the
    /// sprint carries only the item still at Vision.
    #[test]
    fn a_togaf_item_at_migration_is_not_carried_over() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-sprints-togaf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let togaf = crate::config::preset("togaf").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = togaf.statuses.clone();
        config.default_state = togaf.default_state.clone();
        config.displayed_states = togaf.displayed_states.clone();
        config.terminal_states = togaf.terminal_states.clone();
        config.no_op_states = togaf.no_op_states.clone();
        config.transitions = togaf.transitions.clone();
        config.gates = togaf.gates.clone();
        store.init_project("shop", config).unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": true})),
        )
        .unwrap();
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let early = store.add_feature("shop", "A", "", "M", None).unwrap();
        let late = store.add_feature("shop", "B", "", "M", None).unwrap();
        let mut f = store
            .load("shop")
            .unwrap()
            .feature(&late.code)
            .unwrap()
            .clone();
        // Gates are not the subject: write the item at Migration and drop its Vision file.
        let old = store.feature_path("shop", &f.status, &f.code);
        f.status = "Migration".into();
        store.persist_feature_for_test("shop", &f).unwrap();
        std::fs::remove_file(old).unwrap();
        let sp = super::add(&store, "shop", "2026-10-05", Some(5), "", None, "").unwrap();
        let codes = vec![early.code.clone(), late.code.clone()];
        super::plan(&store, "shop", &sp.code, &codes).unwrap();
        let closed = super::close(&store, "shop", "SP-001", Some("backlog")).unwrap();
        let carried: Vec<&str> = closed.carried.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(carried, [early.code.as_str()]);
    }

    fn sprint_bytes(store: &Store) -> Vec<u8> {
        std::fs::read(store.project_dir("shop").join("sprints.yaml")).unwrap()
    }

    /// FTR-022 R-7: a planned sprint's fields change, and only the ones given.
    #[test]
    fn a_planned_sprint_can_be_edited() {
        let (store, _codes, dir) = board("edit", Default::default());
        let sp = super::add(&store, "shop", "2026-10-05", Some(5), "old", Some(10.0), "").unwrap();
        let e = super::SprintEdit {
            name: Some(" Alpha ".into()),
            goal: Some("ship it".into()),
            start: Some("2026-10-12".into()),
            ..Default::default()
        };
        let got = super::edit(&store, "shop", &sp.code, &e).unwrap();
        assert_eq!(got.name, "Alpha");
        assert_eq!(got.goal, "ship it");
        assert_eq!(
            (got.start.as_str(), got.end.as_str()),
            ("2026-10-12", "2026-10-16")
        );
        assert_eq!(got.capacity, Some(10.0));
        let e = super::SprintEdit {
            length_days: Some(14),
            ..Default::default()
        };
        let got = super::edit(&store, "shop", &sp.code, &e).unwrap();
        assert_eq!(
            (got.start.as_str(), got.end.as_str()),
            ("2026-10-12", "2026-10-25")
        );
        let e = super::SprintEdit {
            end: Some("2026-10-30".into()),
            capacity: Some(None),
            ..Default::default()
        };
        let got = super::edit(&store, "shop", &sp.code, &e).unwrap();
        assert_eq!(got.end, "2026-10-30");
        assert_eq!(got.capacity, None);
        assert_eq!((got.name.as_str(), got.goal.as_str()), ("Alpha", "ship it"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-022 R-8: a started or closed sprint is a record, so an edit is refused and nothing moves.
    #[test]
    fn a_started_sprint_refuses_edits() {
        let (store, _codes, dir) = board("started", Default::default());
        let e = super::SprintEdit {
            name: Some("x".into()),
            ..Default::default()
        };
        super::start(&store, "shop", "SP-001").unwrap();
        let before = sprint_bytes(&store);
        let err = super::edit(&store, "shop", "SP-001", &e)
            .unwrap_err()
            .to_string();
        assert!(err.contains("has started"), "{err}");
        super::close(&store, "shop", "SP-001", None).unwrap();
        let before_closed = sprint_bytes(&store);
        let err = super::edit(&store, "shop", "SP-001", &e)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is closed"), "{err}");
        assert_eq!(sprint_bytes(&store), before_closed);
        assert_ne!(before, before_closed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-022 R-8: a bad date, or an end before the start, refuses the whole edit.
    #[test]
    fn an_edit_with_bad_dates_changes_nothing() {
        let (store, _codes, dir) = board("baddates", Default::default());
        super::add(&store, "shop", "2026-10-12", Some(5), "", None, "").unwrap();
        let before = sprint_bytes(&store);
        for e in [
            super::SprintEdit {
                name: Some("renamed".into()),
                start: Some("10/05".into()),
                ..Default::default()
            },
            super::SprintEdit {
                name: Some("renamed".into()),
                end: Some("2026-10-01".into()),
                ..Default::default()
            },
        ] {
            assert!(super::edit(&store, "shop", "SP-002", &e).is_err());
            assert_eq!(sprint_bytes(&store), before);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-022 R-12: editing one sprint leaves the others as they were.
    #[test]
    fn an_edit_keeps_unknown_fields_and_other_sprints() {
        let (store, _codes, dir) = board("keeps", Default::default());
        super::add(&store, "shop", "2026-10-12", Some(5), "g", Some(4.0), "").unwrap();
        let before = super::load(&store, "shop").unwrap();
        let e = super::SprintEdit {
            goal: Some("new".into()),
            ..Default::default()
        };
        super::edit(&store, "shop", "SP-002", &e).unwrap();
        let after = super::load(&store, "shop").unwrap();
        assert_eq!(after[0], before[0]);
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].goal, "new");
        assert_eq!(after[1].capacity, before[1].capacity);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-022 R-9: an empty planned sprint can go.
    #[test]
    fn an_empty_planned_sprint_can_be_removed() {
        let (store, _codes, dir) = board("rm", Default::default());
        super::add(&store, "shop", "2026-10-12", Some(5), "", None, "").unwrap();
        let removed = super::remove(&store, "shop", "SP-002").unwrap();
        assert_eq!(removed.code, "SP-002");
        let left = super::load(&store, "shop").unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].code, "SP-001");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-022 R-9: a sprint that holds an item, or has started, stays, and the refusal says why.
    #[test]
    fn a_sprint_with_items_or_a_start_is_not_removed() {
        let (store, codes, dir) = board("rmno", Default::default());
        let err = super::remove(&store, "shop", "SP-001")
            .unwrap_err()
            .to_string();
        assert!(err.contains(&codes[0]), "{err}");
        for c in &codes {
            store.set_feature_sprint("shop", c, None).unwrap();
        }
        super::start(&store, "shop", "SP-001").unwrap();
        let err = super::remove(&store, "shop", "SP-001")
            .unwrap_err()
            .to_string();
        assert!(err.contains("has started"), "{err}");
        assert_eq!(super::load(&store, "shop").unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-041 R-2: a sprint stored Active whose first day is still ahead reads as planned.
    #[test]
    fn a_sprint_before_its_start_date_is_planned() {
        use super::{Sprint, SprintState};
        let day = |m: u8, d: u8| {
            time::Date::from_calendar_date(2026, time::Month::try_from(m).unwrap(), d).unwrap()
        };
        let stored = Sprint {
            code: "SP-001".into(),
            start: "2026-11-16".into(),
            end: "2026-11-29".into(),
            state: SprintState::Active,
            ..Sprint::default()
        };
        assert_eq!(stored.state_on(day(11, 12)), SprintState::Planned);
        assert_eq!(stored.state_on(day(11, 16)), SprintState::Active);
        let sprints = vec![stored.clone()];
        assert!(super::active_on(&sprints, day(11, 12)).is_none());
        assert!(super::active_on(&sprints, day(11, 16)).is_some());
        let seen = super::as_of(&sprints, day(11, 12));
        assert_eq!(seen[0].state, SprintState::Planned);
        assert_eq!(sprints[0].state, SprintState::Active);
        let undated = Sprint {
            start: String::new(),
            ..stored
        };
        assert_eq!(undated.state_on(day(11, 12)), SprintState::Active);
    }

    /// DEF-041 R-2: starting a sprint before its first day starts it today and keeps its length.
    #[test]
    fn starting_a_sprint_before_its_first_day_starts_it_today() {
        let (store, codes, dir) = board("def041-early", Default::default());
        let day = |m: u8, d: u8| {
            time::Date::from_calendar_date(2026, time::Month::try_from(m).unwrap(), d).unwrap()
        };
        // SP-001 is 5-9 Oct (five days); today is 1 Oct.
        let early = super::start_on(&store, "shop", "SP-001", day(10, 1)).unwrap();
        assert_eq!(early.state, super::SprintState::Active);
        assert_eq!(
            (early.start.as_str(), early.end.as_str()),
            ("2026-10-01", "2026-10-05")
        );
        // A sprint dated in the past keeps its dates.
        for c in &codes {
            store.set_feature_sprint("shop", c, None).unwrap();
        }
        super::close(&store, "shop", "SP-001", None).unwrap();
        let sp = super::add(&store, "shop", "2026-10-10", Some(5), "", None, "").unwrap();
        let late = super::start_on(&store, "shop", &sp.code, day(10, 12)).unwrap();
        assert_eq!(
            (late.start.as_str(), late.end.as_str()),
            ("2026-10-10", "2026-10-14")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-041 R-3: `today` is a first day, for add and for edit.
    #[test]
    fn a_sprint_can_start_today() {
        let (store, _codes, dir) = board("def041-today", Default::default());
        let today = crate::gantt::fmt_date(super::today());
        let added = super::add(&store, "shop", " Today ", Some(5), "", None, "").unwrap();
        assert_eq!(added.start, today);
        let edit = super::SprintEdit {
            start: Some("Today".into()),
            ..Default::default()
        };
        let edited = super::edit(&store, "shop", "SP-001", &edit).unwrap();
        assert_eq!(edited.start, today);
        let _ = std::fs::remove_dir_all(dir);
    }
}
