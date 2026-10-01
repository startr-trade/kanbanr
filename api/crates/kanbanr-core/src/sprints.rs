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
#[serde(rename_all = "snake_case")]
pub enum SprintState {
    #[default]
    Planned,
    Active,
    Closed,
}

/// An item that left a sprint unfinished, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carry {
    pub code: String,
    /// The sprint it moved to, or `backlog`.
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, sprints: &[Sprint]) -> Result<()> {
    std::fs::write(path(store, id), serde_yaml::to_string(sprints)?)?;
    Ok(())
}

/// The sprint currently running, if one is.
pub fn active(sprints: &[Sprint]) -> Option<&Sprint> {
    sprints.iter().find(|s| s.state == SprintState::Active)
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
    let first = crate::gantt::parse_ymd(start)
        .ok_or_else(|| CoreError::Unsupported(format!("'{start}' is not a date (YYYY-MM-DD)")))?;
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

/// Make a sprint the active one. Only one runs at a time.
pub fn start(store: &Store, id: &str, code: &str) -> Result<Sprint> {
    cadence_on(store, id)?;
    let mut sprints = load(store, id)?;
    if let Some(running) = active(&sprints)
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
pub struct BurndownDay {
    pub date: String,
    pub remaining: f64,
}

/// A sprint as a report: its record, what it holds, and how it burned down.
#[derive(Debug, Clone, Serialize)]
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
}
