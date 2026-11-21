//! Planning decisions (FTR-049): the person's decision, made on a monitor page, to plan items into
//! a sprint or a release, or to take them out of their sprint. The page records the decision; it
//! changes no plan. Claude checks it ([`assess`]) and either carries it out with the plan command
//! naming it (`--decision PD-n`, [`carry`]) or declines it with a reason and the nearest valid move
//! ([`decline`]). The CLI stays the single writer of plan state.
//!
//! Each decision is a file of its own, `planning/PD-<n>.yaml` in the project folder, and each
//! outcome another, `planning/PD-<n>.outcome.yaml`; both are written once with a temporary file and
//! a rename and never rewritten, as prepared records are. A decision's state is derived when
//! listed: `pending` with no outcome, else the outcome's `carried` or `declined` (ADR-0005).

use crate::error::{CoreError, Result};
use crate::models::{Extra, FeatureItem};
use crate::releases::{self, Scope};
use crate::sprints::{self, SprintState};
use crate::store::{Project, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The project folder that holds planning decisions and their outcomes.
pub const FOLDER: &str = "planning";

/// The longest note a decision keeps.
pub const NOTE_MAX: usize = 4000;

/// The most items one decision names.
pub const ITEMS_MAX: usize = 50;

/// Plan the items into the sprint `target`.
pub const INTO_SPRINT: &str = "into-sprint";
/// Plan the items into the release `target`.
pub const INTO_RELEASE: &str = "into-release";
/// Take the items out of their sprint; `target` is empty.
pub const OUT_OF_SPRINT: &str = "out-of-sprint";

/// The actions a decision may name.
pub const ACTIONS: [&str; 3] = [INTO_SPRINT, INTO_RELEASE, OUT_OF_SPRINT];

/// The states a decision is listed in.
pub const STATES: [&str; 3] = ["pending", "carried", "declined"];

const OUTCOME: &str = ".outcome.yaml";

/// The person's planning decision, as the page recorded it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanningDecision {
    /// `PD-<n>`.
    pub id: String,
    /// `into-sprint`, `into-release` or `out-of-sprint`.
    pub action: String,
    /// The sprint code or the release version; empty for `out-of-sprint`.
    #[serde(default)]
    pub target: String,
    pub items: Vec<String>,
    pub by: String,
    /// `monitor` or `terminal`, stamped by the front end, never taken from the body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// RFC 3339, UTC.
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Keys this version does not know are kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A move a declined decision could have been instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Nearest {
    pub action: String,
    pub target: String,
}

/// What became of a decision: carried out, or declined with its reason.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanningOutcome {
    /// The decision's id, `PD-<n>`.
    pub decision: String,
    /// `carried` or `declined`.
    pub outcome: String,
    /// The agent, or the person.
    pub by: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// Why it was declined; for a carried decision, the command that carried it out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest: Option<Nearest>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A decision with its derived state and its outcome, when it has one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanningView {
    pub decision: PlanningDecision,
    /// `pending`, `carried` or `declined`, derived.
    pub state: String,
    pub outcome: Option<PlanningOutcome>,
}

/// What `kanbanr planning check` reports: reasons to decline, things for the agent to judge, and the
/// nearest valid move when the target cannot take the items.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Assessment {
    pub decision: String,
    /// Each a reason to decline.
    pub problems: Vec<String>,
    /// Each for the agent to judge.
    pub warnings: Vec<String>,
    pub nearest: Option<Nearest>,
}

fn unsupported(msg: impl Into<String>) -> CoreError {
    CoreError::Unsupported(msg.into())
}

fn dir(store: &Store, p: &str) -> PathBuf {
    store.project_dir(p).join(FOLDER)
}

/// The number in `PD-<n>`, or `None` when `id` is not a decision's id. An id from a route is
/// checked here, so no path comes from it.
fn number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix("PD-")?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn decision_path(store: &Store, p: &str, id: &str) -> PathBuf {
    dir(store, p).join(format!("{id}.yaml"))
}

fn outcome_path(store: &Store, p: &str, id: &str) -> PathBuf {
    dir(store, p).join(format!("{id}{OUTCOME}"))
}

/// The id the next decision takes: one more than the highest number under `planning/`, decision
/// or outcome. The caller holds the board's write lock, so two presses make two numbers.
fn next_id(store: &Store, p: &str) -> String {
    let highest = std::fs::read_dir(dir(store, p))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| {
            let name = f.file_name().to_string_lossy().to_string();
            let stem = name
                .strip_suffix(OUTCOME)
                .or_else(|| name.strip_suffix(".yaml"))?
                .to_string();
            number(&stem)
        })
        .max()
        .unwrap_or(0);
    format!("PD-{}", highest + 1)
}

fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(_) => Err(unsupported(format!(
            "a planning decision's '{key}' is text"
        ))),
    }
}

fn is_finished(project: &Project, f: &FeatureItem) -> bool {
    project.config.counts_as_done(&f.status)
}

/// Record the person's planning decision, as `POST /projects/{p}/planning` takes it:
/// `{action, target, items, by, note?}`, with `origin` stamped by the front end. The action must be
/// known; `items` must name 1 to 50 items that exist; the target must be a sprint for
/// `into-sprint`, a release for `into-release`, and empty for `out-of-sprint`; `by` must name
/// someone. It writes `planning/PD-<n>.yaml` and changes no plan. It does not judge the decision:
/// [`assess`] does that.
pub fn record(store: &Store, p: &str, body: &Value, origin: &str) -> Result<PlanningDecision> {
    let project = store.load_meta(p)?;
    let action = text(body, "action")?.trim().to_string();
    if !ACTIONS.contains(&action.as_str()) {
        return Err(unsupported(format!(
            "'{action}' is not a planning action: use {}",
            ACTIONS.join(", ")
        )));
    }
    let target = text(body, "target")?.trim().to_string();
    let items: Vec<String> = match body.get("items") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| unsupported("a planning decision's items are item codes"))
            })
            .collect::<Result<_>>()?,
        _ => return Err(unsupported("a planning decision names its items")),
    };
    if items.is_empty() {
        return Err(unsupported("a planning decision names at least one item"));
    }
    if items.len() > ITEMS_MAX {
        return Err(unsupported(format!(
            "a planning decision names at most {ITEMS_MAX} items"
        )));
    }
    for code in &items {
        if project.feature(code).is_err() {
            return Err(unsupported(format!("there is no item {code}")));
        }
    }
    match action.as_str() {
        INTO_SPRINT => {
            if !sprints::load(store, p)?.iter().any(|s| s.code == target) {
                return Err(unsupported(format!("there is no sprint '{target}'")));
            }
        }
        INTO_RELEASE => {
            if !releases::load(store, p)?
                .iter()
                .any(|r| r.version == target)
            {
                return Err(unsupported(format!("there is no release '{target}'")));
            }
        }
        _ => {
            if !target.is_empty() {
                return Err(unsupported(
                    "taking items out of their sprint names no target",
                ));
            }
        }
    }
    let by = text(body, "by")?.trim().to_string();
    if by.is_empty() {
        return Err(unsupported(
            "a planning decision must name the person who made it",
        ));
    }
    let note = text(body, "note")?.trim().to_string();
    if note.chars().count() > NOTE_MAX {
        return Err(unsupported(format!(
            "the note is longer than {NOTE_MAX} characters"
        )));
    }
    std::fs::create_dir_all(dir(store, p))?;
    let decision = PlanningDecision {
        id: next_id(store, p),
        action,
        target,
        items,
        by,
        origin: origin.to_string(),
        at: crate::now_rfc3339(),
        note,
        extra: Extra::default(),
    };
    let path = decision_path(store, p, &decision.id);
    if path.exists() {
        return Err(unsupported(format!("{} already exists", decision.id)));
    }
    crate::atomic::write(&path, crate::yaml::to_string(&decision)?)?;
    Ok(decision)
}

/// Every decision with its state, oldest first, optionally only those in `state` (`pending`,
/// `carried` or `declined`). A file that cannot be read is skipped, as `manual::results` does.
pub fn list(store: &Store, p: &str, state: Option<&str>) -> Result<Vec<PlanningView>> {
    if let Some(s) = state
        && !STATES.contains(&s)
    {
        return Err(unsupported(format!(
            "'{s}' is not a planning state: use {}",
            STATES.join(", ")
        )));
    }
    if !store.project_exists(p) {
        return Err(CoreError::ProjectNotFound(p.to_string()));
    }
    let Ok(files) = std::fs::read_dir(dir(store, p)) else {
        return Ok(Vec::new());
    };
    let mut decisions: Vec<(u64, PlanningDecision)> = Vec::new();
    let mut outcomes: BTreeMap<String, PlanningOutcome> = BTreeMap::new();
    for f in files.flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        let Ok(body) = std::fs::read_to_string(f.path()) else {
            continue;
        };
        if let Some(id) = name.strip_suffix(OUTCOME) {
            if number(id).is_none() {
                continue;
            }
            if let Ok(o) = crate::yaml::from_str::<PlanningOutcome>(&body) {
                outcomes.insert(id.to_string(), o);
            }
            continue;
        }
        let Some(id) = name.strip_suffix(".yaml") else {
            continue;
        };
        let Some(n) = number(id) else { continue };
        if let Ok(mut d) = crate::yaml::from_str::<PlanningDecision>(&body) {
            // The file's name is the id: a decision is found by it.
            d.id = id.to_string();
            decisions.push((n, d));
        }
    }
    decisions.sort_by_key(|(n, _)| *n);
    Ok(decisions
        .into_iter()
        .map(|(_, decision)| {
            let outcome = outcomes.remove(&decision.id);
            let state = match &outcome {
                None => "pending".to_string(),
                Some(o) => o.outcome.clone(),
            };
            PlanningView {
                decision,
                state,
                outcome,
            }
        })
        .filter(|v| state.is_none_or(|s| v.state == s))
        .collect())
}

/// One decision with its state. Refused when there is no such decision.
pub fn get(store: &Store, p: &str, id: &str) -> Result<PlanningView> {
    if number(id).is_none() {
        return Err(unsupported(format!(
            "'{id}' is not a planning decision (PD-n)"
        )));
    }
    list(store, p, None)?
        .into_iter()
        .find(|v| v.decision.id == id)
        .ok_or_else(|| unsupported(format!("there is no planning decision {id}")))
}

fn pending(store: &Store, p: &str, id: &str) -> Result<PlanningDecision> {
    let v = get(store, p, id)?;
    match v.state.as_str() {
        "pending" => Ok(v.decision),
        "carried" => Err(unsupported(format!("{id} was already carried out"))),
        other => Err(unsupported(format!("{id} was already {other}"))),
    }
}

/// The plan command that carries out `action`.
fn command_of(action: &str) -> &'static str {
    match action {
        INTO_SPRINT => "sprint plan",
        OUT_OF_SPRINT => "sprint unplan",
        _ => "release plan",
    }
}

/// The check [`carry`] makes before anything is written: the decision is pending, and the
/// command's action, target and set of items are the decision's. The plan routes call it before
/// they change any plan, so a refusal writes nothing.
pub fn check_carry(
    store: &Store,
    p: &str,
    id: &str,
    action: &str,
    target: &str,
    items: &[String],
) -> Result<PlanningDecision> {
    let d = pending(store, p, id)?;
    let theirs: BTreeSet<&str> = d.items.iter().map(String::as_str).collect();
    let ours: BTreeSet<&str> = items.iter().map(String::as_str).collect();
    let target_of = |a: &str, t: &str| match a {
        OUT_OF_SPRINT => "out of their sprint".to_string(),
        _ => format!("into {t}"),
    };
    if d.action != action || d.target != target {
        return Err(unsupported(format!(
            "{id} plans {} {}, not {}: run `kanbanr {}` with its items and target, or decline it",
            d.items.join(", "),
            target_of(&d.action, &d.target),
            target_of(action, target),
            command_of(&d.action),
        )));
    }
    if theirs != ours {
        return Err(unsupported(format!(
            "{id} names {}, not {}: carry it out with exactly its items, or decline it",
            d.items.join(", "),
            items.join(", ")
        )));
    }
    Ok(d)
}

fn write_outcome(store: &Store, p: &str, o: &PlanningOutcome) -> Result<()> {
    let path = outcome_path(store, p, &o.decision);
    if path.exists() {
        return Err(unsupported(format!(
            "{} already has an outcome",
            o.decision
        )));
    }
    std::fs::create_dir_all(dir(store, p))?;
    crate::atomic::write(&path, crate::yaml::to_string(o)?)?;
    Ok(())
}

/// Record decision `id` as carried out by `by` from `origin`, once the plan change it names has
/// been applied in the same dispatch. Refused unless the decision is pending and the command's
/// `action`, `target` and set of `items` equal its own ([`check_carry`]).
#[allow(clippy::too_many_arguments)]
pub fn carry(
    store: &Store,
    p: &str,
    id: &str,
    action: &str,
    target: &str,
    items: &[String],
    by: &str,
    origin: &str,
) -> Result<()> {
    let d = check_carry(store, p, id, action, target, items)?;
    write_outcome(
        store,
        p,
        &PlanningOutcome {
            decision: d.id,
            outcome: "carried".into(),
            by: by.to_string(),
            origin: origin.to_string(),
            at: crate::now_rfc3339(),
            reason: format!("planned by `kanbanr {}`", command_of(&d.action)),
            nearest: None,
            extra: Extra::default(),
        },
    )
}

/// Who carries out a decision named by a plan route's `decision`: the body's `by`, else the board's
/// git identity (R-9: the person carrying out their own decision in a terminal is recorded as the
/// board's identity). Refused when neither names anyone.
pub(crate) fn carrier(store: &Store, body_by: Option<String>) -> Result<String> {
    body_by
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .or_else(|| crate::git::identity(store.data_dir()).map(|(name, _)| name))
        .ok_or_else(|| unsupported("a carried-out planning decision names who carried it out"))
}

/// What each of `items` was planned into before a plan change made for a decision, read by
/// `field` (its sprint or its release), so a failed outcome write can put the plan back.
pub(crate) fn snapshot(
    store: &Store,
    p: &str,
    items: &[String],
    field: fn(&FeatureItem) -> Option<String>,
) -> Result<Vec<(String, Option<String>)>> {
    let project = store.load_meta(p)?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for code in items {
        if seen.insert(code.as_str()) {
            out.push((code.clone(), field(project.feature(code)?)));
        }
    }
    Ok(out)
}

/// Put the plan back as [`snapshot`] read it, after the change for a decision failed part way or
/// its outcome could not be written, and turn `cause` into the error to report. Only an item whose
/// `field` now differs is written, so a change refused before it began leaves every file as it
/// was. Nothing is committed on an error; an item that cannot be put back is named, so the board
/// is known to be changed on disk.
pub(crate) fn restore(
    store: &Store,
    p: &str,
    before: Vec<(String, Option<String>)>,
    field: fn(&FeatureItem) -> Option<String>,
    set: fn(&Store, &str, &str, Option<String>) -> Result<FeatureItem>,
    cause: CoreError,
) -> CoreError {
    let now = store.load_meta(p).ok();
    let changed = |code: &str, was: &Option<String>| match &now {
        Some(project) => project
            .feature(code)
            .map(|f| &field(f) != was)
            .unwrap_or(false),
        None => true,
    };
    let stuck: Vec<String> = before
        .into_iter()
        .filter(|(code, was)| changed(code, was))
        .filter_map(|(code, was)| set(store, p, &code, was).err().map(|_| code))
        .collect();
    if stuck.is_empty() {
        cause
    } else {
        unsupported(format!(
            "{cause}; and the plan of {} could not be put back, so it is changed on disk but NOT \
             committed",
            stuck.join(", ")
        ))
    }
}

/// Decline decision `id`, keeping the reason and the nearest valid move. Refused when it is not
/// pending, the reason is empty, or the nearest move names an unknown action.
pub fn decline(
    store: &Store,
    p: &str,
    id: &str,
    by: &str,
    origin: &str,
    reason: &str,
    nearest: Option<Nearest>,
) -> Result<PlanningOutcome> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(unsupported(
            "a declined planning decision says why: give a reason",
        ));
    }
    if by.trim().is_empty() {
        return Err(unsupported("a decline names who declined it"));
    }
    if let Some(n) = &nearest
        && !ACTIONS.contains(&n.action.as_str())
    {
        return Err(unsupported(format!(
            "'{}' is not a planning action: use {}",
            n.action,
            ACTIONS.join(", ")
        )));
    }
    let d = pending(store, p, id)?;
    let o = PlanningOutcome {
        decision: d.id,
        outcome: "declined".into(),
        by: by.trim().to_string(),
        origin: origin.to_string(),
        at: crate::now_rfc3339(),
        reason: reason.to_string(),
        nearest,
        extra: Extra::default(),
    };
    write_outcome(store, p, &o)?;
    Ok(o)
}

/// An item's size in the project's unit, as `sprints::committed` counts it; an unestimated item
/// counts as nothing.
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

/// Check decision `id` on `today`, changing nothing (R-6). Problems are reasons to decline: the
/// target is frozen (a release shipped, superseded or at a stage; a closed sprint) or gone, an item
/// is gone, finished or in a no-op status, or, taking items out of their sprint, an item is in
/// none or in a closed one (which can't change, as `sprints::unplan` refuses). Warnings are for
/// the agent to judge: a dependency planned later or not at all, a sprint over its capacity, an item
/// due before the sprint's end or the release's target (it would land too late). The nearest move is
/// the next planned release for a frozen release, and the active, else the next planned, sprint
/// for a closed one.
pub fn assess(store: &Store, p: &str, id: &str, today: time::Date) -> Result<Assessment> {
    let view = get(store, p, id)?;
    let d = view.decision;
    let project = store.load_meta(p)?;
    let all_sprints = sprints::load(store, p)?;
    let all_releases = releases::load(store, p)?;
    let mut out = Assessment {
        decision: d.id.clone(),
        ..Default::default()
    };
    if view.state != "pending" {
        out.problems
            .push(format!("{} was already {}", d.id, view.state));
    }

    let mut items: Vec<&FeatureItem> = Vec::new();
    for code in &d.items {
        match project.features.iter().find(|f| &f.code == code) {
            None => out.problems.push(format!("{code} no longer exists")),
            Some(f) => {
                if project.config.is_no_op(&f.status) {
                    out.problems
                        .push(format!("{code} is at {}, a no-op status", f.status));
                } else if is_finished(&project, f) {
                    out.problems
                        .push(format!("{code} is finished (it is at {})", f.status));
                }
                items.push(f);
            }
        }
    }
    let named: BTreeSet<&str> = d.items.iter().map(String::as_str).collect();
    // A dependency that is not one of the decision's items, still exists and is not finished.
    let open_deps = |f: &FeatureItem| -> Vec<FeatureItem> {
        f.depends_on
            .iter()
            .filter(|dep| !named.contains(dep.as_str()))
            .filter_map(|dep| project.features.iter().find(|g| &g.code == dep))
            .filter(|g| !is_finished(&project, g) && !project.config.is_no_op(&g.status))
            .cloned()
            .collect()
    };
    let due_of = |f: &FeatureItem| {
        f.due
            .as_deref()
            .and_then(crate::gantt::parse_ymd)
            .map(|d| (d, f.due.clone().unwrap_or_default()))
    };

    match d.action.as_str() {
        INTO_SPRINT => {
            let Some(sprint) = all_sprints.iter().find(|s| s.code == d.target) else {
                out.problems.push(format!("{} no longer exists", d.target));
                return Ok(out);
            };
            if sprint.state_on(today) == SprintState::Closed {
                out.problems.push(format!(
                    "{} is closed; a closed sprint keeps its items as its record",
                    sprint.code
                ));
                let next = sprints::active_on(&all_sprints, today)
                    .or_else(|| {
                        all_sprints
                            .iter()
                            .filter(|s| s.state_on(today) == SprintState::Planned)
                            .min_by(|a, b| a.start.cmp(&b.start))
                    })
                    .map(|s| s.code.clone());
                out.nearest = next.map(|target| Nearest {
                    action: INTO_SPRINT.into(),
                    target,
                });
            }
            let start_of = |code: &str| {
                all_sprints
                    .iter()
                    .find(|s| s.code == code)
                    .map(|s| s.start.clone())
            };
            for f in &items {
                for g in open_deps(f) {
                    match g.sprint.as_deref() {
                        None => out.warnings.push(format!(
                            "{} depends on {}, which is in no sprint",
                            f.code, g.code
                        )),
                        Some(s) if start_of(s).is_some_and(|st| st > sprint.start) => {
                            out.warnings.push(format!(
                                "{} depends on {}, planned into {s}, after {}",
                                f.code, g.code, sprint.code
                            ))
                        }
                        _ => {}
                    }
                }
            }
            if let Some(cap) = sprint.capacity {
                let adding: f64 = items
                    .iter()
                    .filter(|f| f.sprint.as_deref() != Some(sprint.code.as_str()))
                    .map(|f| size(&project, f))
                    .fold(0.0, |a, b| a + b);
                let would = sprints::committed(&project, &sprint.code) + adding;
                if would > cap {
                    out.warnings.push(format!(
                        "{} would hold {would} {} against a capacity of {cap}",
                        sprint.code,
                        unit_name(&project)
                    ));
                }
            }
            if let Some(end) = crate::gantt::parse_ymd(&sprint.end) {
                for f in &items {
                    if let Some((due, text)) = due_of(f)
                        && due < end
                    {
                        out.warnings.push(format!(
                            "{} is due {text}, before {} ends on {}",
                            f.code, sprint.code, sprint.end
                        ));
                    }
                }
            }
        }
        INTO_RELEASE => {
            let Some(release) = all_releases.iter().find(|r| r.version == d.target) else {
                out.problems.push(format!("{} no longer exists", d.target));
                return Ok(out);
            };
            let v = &release.version;
            match releases::scope_of(release) {
                Scope::Planned => {}
                _ => {
                    let why = if release.state == releases::ReleaseState::Shipped {
                        format!("{v} has shipped; its scope is frozen")
                    } else if !release.superseded_by.is_empty() {
                        format!(
                            "{v} was superseded by {}; its scope is frozen",
                            release.superseded_by
                        )
                    } else {
                        format!("{v} is at {}; its scope is frozen", release.stage)
                    };
                    out.problems.push(why);
                    let key = releases::version_key(v);
                    out.nearest = all_releases
                        .iter()
                        .filter(|r| releases::scope_of(r) == Scope::Planned)
                        .filter(|r| releases::version_key(&r.version) > key)
                        .min_by_key(|r| releases::version_key(&r.version))
                        .map(|r| Nearest {
                            action: INTO_RELEASE.into(),
                            target: r.version.clone(),
                        });
                }
            }
            let key = releases::version_key(v);
            for f in &items {
                for g in open_deps(f) {
                    match g.release.as_deref() {
                        None => out.warnings.push(format!(
                            "{} depends on {}, which is in no release",
                            f.code, g.code
                        )),
                        Some(r) if releases::version_key(r) > key => out.warnings.push(format!(
                            "{} depends on {}, planned into {r}, after {v}",
                            f.code, g.code
                        )),
                        _ => {}
                    }
                }
            }
            if let Some(target) = crate::gantt::parse_ymd(&release.target) {
                for f in &items {
                    if let Some((due, text)) = due_of(f)
                        && due < target
                    {
                        out.warnings.push(format!(
                            "{} is due {text}, before {v}'s target of {}",
                            f.code, release.target
                        ));
                    }
                }
            }
        }
        _ => {
            for f in &items {
                match f.sprint.as_deref() {
                    None => out.problems.push(format!("{} is not in a sprint", f.code)),
                    Some(s)
                        if all_sprints
                            .iter()
                            .any(|x| x.code == s && x.state_on(today) == SprintState::Closed) =>
                    {
                        out.problems.push(format!(
                            "{} sits in closed sprint {s}, which can't change",
                            f.code
                        ))
                    }
                    Some(_) => {}
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectConfig;
    use serde_json::json;

    /// A scrum board in the temp folder with sprints, releases and points on, three items, two
    /// sprints and two releases.
    fn board(tag: &str) -> (Store, Vec<String>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-planning-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The board is the test's own, never one a walk up from the repository would find (L-41).
        assert!(dir.starts_with(std::env::temp_dir()));
        let store = Store::new(dir.clone());
        let scrum = crate::config::preset("scrum").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = scrum.statuses.clone();
        config.default_state = scrum.default_state.clone();
        config.displayed_states = scrum.displayed_states.clone();
        config.terminal_states = scrum.terminal_states.clone();
        config.no_op_states = scrum.no_op_states.clone();
        config.transitions = scrum.transitions.clone();
        config.gates = Default::default();
        store.init_project("shop", config).unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&json!({"sprints": true, "releases": true, "estimate_unit": "points"})),
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
        sprints::add(
            &store,
            "shop",
            "2026-11-02",
            Some(14),
            "",
            Some(10.0),
            "",
            None,
            None,
        )
        .unwrap();
        sprints::add(
            &store,
            "shop",
            "2026-11-16",
            Some(14),
            "",
            None,
            "",
            None,
            None,
        )
        .unwrap();
        releases::add(&store, "shop", "v0.8.0", "2026-11-30", "").unwrap();
        releases::add(&store, "shop", "v0.9.0", "", "").unwrap();
        (store, codes, dir)
    }

    fn decide(
        store: &Store,
        action: &str,
        target: &str,
        items: &[impl AsRef<str>],
    ) -> Result<PlanningDecision> {
        let items: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
        record(
            store,
            "shop",
            &json!({"action": action, "target": target, "items": items, "by": "Ada L"}),
            "monitor",
        )
    }

    fn day(s: &str) -> time::Date {
        crate::gantt::parse_ymd(s).unwrap()
    }

    /// FTR-049 R-1: a decision is a file of its own, numbered in turn; recording it changes no
    /// plan; an unknown item, target or action is refused.
    #[test]
    fn a_planning_decision_is_kept_as_a_file_and_changes_no_plan() {
        let (store, codes, dir) = board("r1");
        let project_dir = store.project_dir("shop");
        let sprints_before = std::fs::read(project_dir.join("sprints.yaml")).unwrap();
        let releases_before = std::fs::read(project_dir.join("releases.yaml")).unwrap();
        let items_before = store.load_meta("shop").unwrap().features;

        let a = &codes[0];
        let first = decide(&store, INTO_SPRINT, "SP-001", &[a, &codes[1]]).unwrap();
        assert_eq!(first.id, "PD-1");
        assert_eq!(first.by, "Ada L");
        assert_eq!(first.origin, "monitor");
        assert!(project_dir.join("planning").join("PD-1.yaml").is_file());
        let second = decide(&store, INTO_RELEASE, "v0.9.0", &[a]).unwrap();
        assert_eq!(second.id, "PD-2");
        let third = decide(&store, OUT_OF_SPRINT, "", &[a]).unwrap();
        assert_eq!(third.id, "PD-3");

        // The kept file reads back as written.
        let text = std::fs::read_to_string(project_dir.join("planning").join("PD-1.yaml")).unwrap();
        let kept: PlanningDecision = crate::yaml::from_str(&text).unwrap();
        assert_eq!(kept, first);

        // Nothing planned.
        assert_eq!(
            std::fs::read(project_dir.join("sprints.yaml")).unwrap(),
            sprints_before
        );
        assert_eq!(
            std::fs::read(project_dir.join("releases.yaml")).unwrap(),
            releases_before
        );
        let items_after = store.load_meta("shop").unwrap().features;
        for (b, a) in items_before.iter().zip(&items_after) {
            assert_eq!(
                (&b.sprint, &b.release),
                (&a.sprint, &a.release),
                "{}",
                a.code
            );
        }

        // Refused: an unknown item, an unknown sprint or release, a target out of a sprint, an
        // unknown action, no items, no person.
        for (action, target, items) in [
            (INTO_SPRINT, "SP-001", vec!["FEAT-999"]),
            (INTO_SPRINT, "SP-009", vec![a.as_str()]),
            (INTO_RELEASE, "v9.9.9", vec![a.as_str()]),
            (OUT_OF_SPRINT, "SP-001", vec![a.as_str()]),
            ("into-milestone", "M", vec![a.as_str()]),
            (INTO_SPRINT, "SP-001", vec![]),
        ] {
            assert!(
                decide(&store, action, target, items.as_slice()).is_err(),
                "{action} {target} {items:?}"
            );
        }
        let nobody = record(
            &store,
            "shop",
            &json!({"action": INTO_SPRINT, "target": "SP-001", "items": [a], "by": " "}),
            "monitor",
        );
        assert!(nobody.is_err());
        let too_many: Vec<String> = (0..=ITEMS_MAX).map(|_| a.clone()).collect();
        assert!(
            record(
                &store,
                "shop",
                &json!({"action": INTO_SPRINT, "target": "SP-001", "items": too_many, "by": "Ada L"}),
                "monitor",
            )
            .is_err()
        );
        // A refusal writes no file.
        assert_eq!(list(&store, "shop", None).unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-3: a decision with no outcome is pending; one with an outcome is carried or
    /// declined; the state filter lists each.
    #[test]
    fn pending_lists_decisions_with_no_outcome() {
        let (store, codes, dir) = board("r3");
        let a = codes[0].clone();
        decide(&store, INTO_SPRINT, "SP-001", &[&a]).unwrap();
        decide(&store, INTO_RELEASE, "v0.8.0", &[&a]).unwrap();
        decide(&store, INTO_RELEASE, "v0.9.0", &[&codes[1]]).unwrap();
        assert!(
            list(&store, "shop", None)
                .unwrap()
                .iter()
                .all(|v| v.state == "pending")
        );

        carry(
            &store,
            "shop",
            "PD-1",
            INTO_SPRINT,
            "SP-001",
            std::slice::from_ref(&a),
            "Claude",
            "conversation",
        )
        .unwrap();
        decline(
            &store,
            "shop",
            "PD-2",
            "Claude",
            "conversation",
            "not now",
            None,
        )
        .unwrap();
        // An unreadable file is skipped.
        std::fs::write(
            store.project_dir("shop").join("planning").join("PD-9.yaml"),
            ":::",
        )
        .unwrap();

        let all = list(&store, "shop", None).unwrap();
        let states: Vec<(&str, &str)> = all
            .iter()
            .map(|v| (v.decision.id.as_str(), v.state.as_str()))
            .collect();
        assert_eq!(
            states,
            vec![
                ("PD-1", "carried"),
                ("PD-2", "declined"),
                ("PD-3", "pending")
            ]
        );
        assert!(all[2].outcome.is_none());
        let ids = |state| -> Vec<String> {
            list(&store, "shop", Some(state))
                .unwrap()
                .into_iter()
                .map(|v| v.decision.id)
                .collect()
        };
        assert_eq!(ids("pending"), vec!["PD-3"]);
        assert_eq!(ids("carried"), vec!["PD-1"]);
        assert_eq!(ids("declined"), vec!["PD-2"]);
        assert!(list(&store, "shop", Some("lost")).is_err());
        // The next decision is numbered past every file, the unreadable one included.
        assert_eq!(
            decide(&store, INTO_SPRINT, "SP-002", &[&a]).unwrap().id,
            "PD-10"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-5: a decline needs a reason, keeps it and the nearest move, and is refused once
    /// the decision has an outcome.
    #[test]
    fn a_decline_keeps_its_reason_and_nearest_move() {
        let (store, codes, dir) = board("r5");
        decide(&store, INTO_RELEASE, "v0.8.0", &[&codes[0]]).unwrap();
        assert!(decline(&store, "shop", "PD-1", "Claude", "conversation", "  ", None).is_err());
        assert_eq!(list(&store, "shop", Some("pending")).unwrap().len(), 1);
        let nearest = Nearest {
            action: INTO_RELEASE.into(),
            target: "v0.9.0".into(),
        };
        let o = decline(
            &store,
            "shop",
            "PD-1",
            "Claude",
            "conversation",
            "v0.8.0 is at candidate; its scope is frozen",
            Some(nearest.clone()),
        )
        .unwrap();
        assert_eq!(o.outcome, "declined");
        let v = get(&store, "shop", "PD-1").unwrap();
        assert_eq!(v.state, "declined");
        let kept = v.outcome.unwrap();
        assert_eq!(kept.reason, "v0.8.0 is at candidate; its scope is frozen");
        assert_eq!(kept.nearest, Some(nearest));
        assert_eq!(
            (kept.by.as_str(), kept.origin.as_str()),
            ("Claude", "conversation")
        );
        // Once declined, neither a second decline nor carrying it out is accepted.
        assert!(
            decline(
                &store,
                "shop",
                "PD-1",
                "Claude",
                "conversation",
                "again",
                None
            )
            .is_err()
        );
        assert!(
            check_carry(
                &store,
                "shop",
                "PD-1",
                INTO_RELEASE,
                "v0.8.0",
                std::slice::from_ref(&codes[0])
            )
            .is_err()
        );
        // An unknown decision, or an id that is not one, is refused.
        assert!(decline(&store, "shop", "PD-7", "Claude", "conversation", "x", None).is_err());
        assert!(decline(&store, "shop", "../x", "Claude", "conversation", "x", None).is_err());
        // A nearest move names a planning action.
        decide(&store, INTO_RELEASE, "v0.8.0", &[&codes[0]]).unwrap();
        let bad = Nearest {
            action: "into-milestone".into(),
            target: "M".into(),
        };
        assert!(
            decline(
                &store,
                "shop",
                "PD-2",
                "Claude",
                "conversation",
                "x",
                Some(bad)
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn save_releases(store: &Store, all: &[releases::Release]) {
        crate::atomic::write(
            store.project_dir("shop").join("releases.yaml"),
            crate::yaml::to_string(all).unwrap(),
        )
        .unwrap();
    }

    fn save_sprints(store: &Store, all: &[sprints::Sprint]) {
        crate::atomic::write(
            store.project_dir("shop").join("sprints.yaml"),
            crate::yaml::to_string(all).unwrap(),
        )
        .unwrap();
    }

    /// FTR-049 R-6: the check names a frozen release and a closed sprint with the nearest move, a
    /// finished item, a dependency planned later, an over-capacity sprint and a due date after the
    /// end; it changes nothing.
    #[test]
    fn assess_names_frozen_scope_dependencies_and_dates() {
        let (store, codes, dir) = board("r6");
        let (a, b, c) = (codes[0].clone(), codes[1].clone(), codes[2].clone());
        let today = day("2026-11-17");

        // A release at candidate: frozen, nearest the next planned release.
        let mut all = releases::load(&store, "shop").unwrap();
        all[0].stage = "candidate".into();
        save_releases(&store, &all);
        decide(&store, INTO_RELEASE, "v0.8.0", &[&a]).unwrap();
        let r = assess(&store, "shop", "PD-1", today).unwrap();
        assert_eq!(r.decision, "PD-1");
        assert_eq!(
            r.problems,
            vec!["v0.8.0 is at candidate; its scope is frozen"]
        );
        assert_eq!(
            r.nearest,
            Some(Nearest {
                action: INTO_RELEASE.into(),
                target: "v0.9.0".into()
            })
        );
        // A shipped one is frozen too.
        all[0].stage.clear();
        all[0].state = releases::ReleaseState::Shipped;
        save_releases(&store, &all);
        let r = assess(&store, "shop", "PD-1", today).unwrap();
        assert_eq!(r.problems, vec!["v0.8.0 has shipped; its scope is frozen"]);

        // A closed sprint: nearest the active one (SP-002 runs on 17 Nov).
        let mut sps = sprints::load(&store, "shop").unwrap();
        sps[0].state = SprintState::Closed;
        sps[1].state = SprintState::Active;
        save_sprints(&store, &sps);
        decide(&store, INTO_SPRINT, "SP-001", &[&a]).unwrap();
        let r = assess(&store, "shop", "PD-2", today).unwrap();
        assert_eq!(
            r.problems,
            vec!["SP-001 is closed; a closed sprint keeps its items as its record"]
        );
        assert_eq!(
            r.nearest,
            Some(Nearest {
                action: INTO_SPRINT.into(),
                target: "SP-002".into()
            })
        );
        // No sprint active: the next planned one.
        sps[1].state = SprintState::Planned;
        save_sprints(&store, &sps);
        let r = assess(&store, "shop", "PD-2", today).unwrap();
        assert_eq!(r.nearest.map(|n| n.target).as_deref(), Some("SP-002"));
        sps[0].state = SprintState::Active;
        save_sprints(&store, &sps);

        // A finished item, and one in a no-op status.
        for to in [
            "Ready",
            "In Progress",
            "Review",
            "Testing",
            "Done",
            "Released",
        ] {
            store.move_feature("shop", &c, to).unwrap();
        }
        decide(&store, INTO_SPRINT, "SP-001", &[&c]).unwrap();
        let r = assess(&store, "shop", "PD-3", today).unwrap();
        assert_eq!(
            r.problems,
            vec![format!("{c} is finished (it is at Released)")]
        );
        assert_eq!(r.nearest, None);

        // A dependency in a later sprint, the sprint over capacity (10 points), a due date before
        // the sprint ends: warnings, not problems.
        store
            .set_feature_attrs(
                "shop",
                &a,
                None,
                None,
                Some("2026-11-10".into()),
                None,
                None,
                None,
                Some(vec![b.clone()]),
            )
            .unwrap();
        store
            .set_feature_sprint("shop", &b, Some("SP-002".into()))
            .unwrap();
        store.set_feature_points("shop", &a, 9.0).unwrap();
        store.set_feature_sprint("shop", &c, None).unwrap();
        decide(&store, INTO_SPRINT, "SP-001", &[&a]).unwrap();
        let r = assess(&store, "shop", "PD-4", today).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.warnings,
            vec![
                format!("{a} depends on {b}, planned into SP-002, after SP-001"),
                format!("{a} is due 2026-11-10, before SP-001 ends on 2026-11-15"),
            ],
            "nine of ten points is within capacity"
        );
        // Plan more into SP-001 so the item would take it over.
        store
            .set_feature_sprint("shop", &c, Some("SP-001".into()))
            .unwrap();
        store.move_feature("shop", &c, "In Progress").unwrap();
        let r = assess(&store, "shop", "PD-4", today).unwrap();
        assert!(
            r.warnings
                .contains(&"SP-001 would hold 17 points against a capacity of 10".to_string()),
            "{:?}",
            r.warnings
        );
        // A dependency in no release, planned into v0.9.0 with no target date.
        decide(&store, INTO_RELEASE, "v0.9.0", &[&a]).unwrap();
        let r = assess(&store, "shop", "PD-5", today).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.warnings,
            vec![format!("{a} depends on {b}, which is in no release")]
        );

        // Out of a sprint: an item in none is a problem; a deleted item is named.
        decide(&store, OUT_OF_SPRINT, "", &[&b, &a]).unwrap();
        let r = assess(&store, "shop", "PD-6", today).unwrap();
        assert_eq!(r.problems, vec![format!("{a} is not in a sprint")]);
        let mut gone = get(&store, "shop", "PD-6").unwrap().decision;
        gone.items = vec!["FEAT-404".into()];
        std::fs::write(
            store.project_dir("shop").join("planning").join("PD-6.yaml"),
            crate::yaml::to_string(&gone).unwrap(),
        )
        .unwrap();
        let r = assess(&store, "shop", "PD-6", today).unwrap();
        assert_eq!(r.problems, vec!["FEAT-404 no longer exists"]);

        // A due date before the release's target is a warning; one on or after it holds.
        all[0].state = releases::ReleaseState::Planned;
        save_releases(&store, &all);
        decide(&store, INTO_RELEASE, "v0.8.0", &[&a]).unwrap();
        let r = assess(&store, "shop", "PD-7", today).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.warnings,
            vec![
                format!("{a} depends on {b}, which is in no release"),
                format!("{a} is due 2026-11-10, before v0.8.0's target of 2026-11-30"),
            ]
        );
        let due = |when: &str| {
            store
                .set_feature_attrs(
                    "shop",
                    &a,
                    None,
                    None,
                    Some(when.into()),
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
        };
        due("2026-11-30");
        let r = assess(&store, "shop", "PD-7", today).unwrap();
        assert!(
            !r.warnings.iter().any(|w| w.contains(" is due ")),
            "due on the target holds: {:?}",
            r.warnings
        );
        due("2026-11-15");
        let r = assess(&store, "shop", "PD-4", today).unwrap();
        assert!(
            !r.warnings.iter().any(|w| w.contains(" is due ")),
            "due on the sprint's end holds: {:?}",
            r.warnings
        );

        // Out of a closed sprint: the item sits in it, and a closed sprint can't change.
        sps[1].state = SprintState::Closed;
        save_sprints(&store, &sps);
        decide(&store, OUT_OF_SPRINT, "", &[&b]).unwrap();
        let r = assess(&store, "shop", "PD-8", today).unwrap();
        assert_eq!(
            r.problems,
            vec![format!(
                "{b} sits in closed sprint SP-002, which can't change"
            )]
        );

        // The check changed nothing: every decision is still pending.
        assert_eq!(list(&store, "shop", Some("pending")).unwrap().len(), 8);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-4: a plan route naming a decision carries it out: the plan change and the outcome
    /// are written by one dispatch; a command that does not match its decision (another target,
    /// another item), or one that names a decision already carried out or declined, is refused
    /// with nothing written; an outcome that cannot be written puts the plan back.
    #[test]
    fn carrying_out_records_the_outcome_with_the_change() {
        let (store, codes, dir) = board("r4");
        let (a, b, c) = (&codes[0], &codes[1], &codes[2]);
        let post = |path: &str, body: Value| {
            crate::dispatch::dispatch(&store, "POST", path, Some(&body)).map(|_| ())
        };
        let item = |code: &str| {
            store
                .load_meta("shop")
                .unwrap()
                .feature(code)
                .unwrap()
                .clone()
        };
        let outcome_file = |id: &str| {
            store
                .project_dir("shop")
                .join(FOLDER)
                .join(format!("{id}{OUTCOME}"))
        };
        let state_of = |id: &str| get(&store, "shop", id).unwrap();

        // Into a sprint: the items are planned and the outcome is written, by one dispatch.
        let pd1 = decide(&store, INTO_SPRINT, "SP-001", &[a, b]).unwrap();
        post(
            "/projects/shop/sprints/SP-001/plan",
            json!({"items": [b, a], "decision": pd1.id, "by": "Claude", "origin": "conversation"}),
        )
        .unwrap();
        assert_eq!(item(a).sprint.as_deref(), Some("SP-001"));
        assert_eq!(item(b).sprint.as_deref(), Some("SP-001"));
        let v = state_of("PD-1");
        assert_eq!(v.state, "carried");
        let o = v.outcome.unwrap();
        assert_eq!(
            (o.by.as_str(), o.origin.as_str()),
            ("Claude", "conversation")
        );
        assert_eq!(o.reason, "planned by `kanbanr sprint plan`");

        // Carried out once only.
        let again = post(
            "/projects/shop/sprints/SP-001/plan",
            json!({"items": [a, b], "decision": "PD-1", "by": "Claude"}),
        )
        .unwrap_err();
        assert!(again.to_string().contains("already carried out"), "{again}");

        // A mismatch is refused before anything is written: another target, then another item.
        let pd2 = decide(&store, INTO_RELEASE, "v0.9.0", &[c]).unwrap();
        let other_target = post(
            "/projects/shop/releases/v0.8.0/plan",
            json!({"items": [c], "decision": pd2.id, "by": "Claude"}),
        )
        .unwrap_err();
        assert!(
            other_target
                .to_string()
                .contains("into v0.9.0, not into v0.8.0"),
            "{other_target}"
        );
        let other_route = post(
            "/projects/shop/sprints/SP-002/plan",
            json!({"items": [c], "decision": pd2.id, "by": "Claude"}),
        )
        .unwrap_err();
        assert!(
            other_route.to_string().contains("PD-2 plans"),
            "{other_route}"
        );
        let other_item = post(
            "/projects/shop/releases/v0.9.0/plan",
            json!({"items": [a], "decision": pd2.id, "by": "Claude"}),
        )
        .unwrap_err();
        assert!(other_item.to_string().contains("not "), "{other_item}");
        assert_eq!(item(c).release, None);
        assert_eq!(item(c).sprint, None);
        assert_eq!(item(a).release, None);
        assert!(!outcome_file("PD-2").exists());
        assert_eq!(state_of("PD-2").state, "pending");

        // Into a release, matching: carried.
        post(
            "/projects/shop/releases/v0.9.0/plan",
            json!({"items": [c], "decision": pd2.id, "by": "Claude"}),
        )
        .unwrap();
        assert_eq!(item(c).release.as_deref(), Some("v0.9.0"));
        let o = state_of("PD-2").outcome.unwrap();
        assert_eq!(o.reason, "planned by `kanbanr release plan`");

        // A declined decision is refused, and the plan stays as it was.
        let pd3 = decide(&store, OUT_OF_SPRINT, "", &[a]).unwrap();
        decline(
            &store,
            "shop",
            &pd3.id,
            "Claude",
            "conversation",
            "a stays in",
            None,
        )
        .unwrap();
        let declined = post(
            "/projects/shop/sprints/unplan",
            json!({"items": [a], "decision": pd3.id, "by": "Claude"}),
        )
        .unwrap_err();
        assert!(
            declined.to_string().contains("already declined"),
            "{declined}"
        );
        assert_eq!(item(a).sprint.as_deref(), Some("SP-001"));

        // Out of a sprint, matching: carried.
        let pd4 = decide(&store, OUT_OF_SPRINT, "", &[b]).unwrap();
        post(
            "/projects/shop/sprints/unplan",
            json!({"items": [b], "decision": pd4.id, "by": "Claude"}),
        )
        .unwrap();
        assert_eq!(item(b).sprint, None);
        assert_eq!(
            state_of("PD-4").outcome.unwrap().reason,
            "planned by `kanbanr sprint unplan`"
        );

        // An outcome that cannot be written puts the plan back: an unreadable outcome file leaves
        // the decision pending but blocks the write.
        let pd5 = decide(&store, INTO_SPRINT, "SP-002", &[c]).unwrap();
        std::fs::write(outcome_file(&pd5.id), ": not yaml [").unwrap();
        assert_eq!(state_of("PD-5").state, "pending");
        let blocked = post(
            "/projects/shop/sprints/SP-002/plan",
            json!({"items": [c], "decision": pd5.id, "by": "Claude"}),
        )
        .unwrap_err();
        assert!(
            blocked.to_string().contains("already has an outcome"),
            "{blocked}"
        );
        assert_eq!(item(c).sprint, None);

        // Without a decision the routes plan as they always have, and write no outcome.
        let before = std::fs::read_dir(store.project_dir("shop").join(FOLDER))
            .unwrap()
            .count();
        post("/projects/shop/sprints/SP-002/plan", json!({"items": [b]})).unwrap();
        assert_eq!(item(b).sprint.as_deref(), Some("SP-002"));
        assert_eq!(
            std::fs::read_dir(store.project_dir("shop").join(FOLDER))
                .unwrap()
                .count(),
            before
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
