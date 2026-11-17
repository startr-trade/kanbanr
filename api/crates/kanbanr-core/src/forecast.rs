//! The flow plan (FTR-061): a scope's work in agent-hours over its task graph — what is left, the
//! critical path through it, how many tasks can run at once, what runs, what is ready and what
//! each blocked task waits on — and a forecast of how long it takes with 1, 4 or 8 lanes, bounded
//! by the builds the machine can run at once.
//!
//! A forecast is hours from now, never a date: turning hours into a date needs an assumption
//! about when agents run, which is calendar quoting. The only dates in a [`FlowPlan`] are those a
//! person set (a running sprint's last day, a release's target), under `dates`. Derived on every
//! read, never stored (ADR-0002); the actuals come from the task log ([`crate::tasklog`]).

use crate::error::{CoreError, Result};
use crate::graph::is_terminal_status;
use crate::models::TaskState;
use crate::sprints::Sprint;
use crate::taskgraph::{self, DEFAULT_ESTIMATE_H, TaskGraph, TaskId, TaskNode};
use crate::tasklog::{self, TaskActual};
use crate::{ProjectConfig, Store};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The least a running task is taken to have left, in hours, however far past its estimate.
const MIN_REMAINING_H: f64 = 0.25;

/// The build share assumed until enough tasks have measured one.
const ASSUMED_BUILD_SHARE: f64 = 0.25;

/// Completed tasks with build actuals needed before the share is measured, not assumed.
const MEASURED_AFTER: usize = 5;

/// The lane counts every plan forecasts.
const DEFAULT_LANES: [usize; 3] = [1, 4, 8];

/// Build slots when neither the sprint nor the machine says.
const DEFAULT_SLOTS: usize = 2;

/// What a plan covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanScope {
    Sprint(String),
    Release(String),
    Milestone(String),
    /// Every item not at an end status.
    Open,
}

impl PlanScope {
    /// The running sprint on `today`, else every open item.
    pub fn default_for(sprints: &[Sprint], today: time::Date) -> PlanScope {
        match crate::sprints::active_on(sprints, today) {
            Some(s) => PlanScope::Sprint(s.code.clone()),
            None => PlanScope::Open,
        }
    }

    /// `sprint:SP-004`, `release:v0.8.0`, `milestone:MS-004` or `open`: the form the route's
    /// `scope` takes.
    pub fn label(&self) -> String {
        match self {
            PlanScope::Sprint(c) => format!("sprint:{c}"),
            PlanScope::Release(v) => format!("release:{v}"),
            PlanScope::Milestone(m) => format!("milestone:{m}"),
            PlanScope::Open => "open".to_string(),
        }
    }
}

/// What to forecast: each lane count, the build slots, and the build share (measured when none).
#[derive(Debug, Clone, PartialEq)]
pub struct ForecastOptions {
    pub lanes: Vec<usize>,
    pub slots: usize,
    pub build_share: Option<f64>,
}

/// How long the scope's remaining work takes with `lanes` agents and `slots` builds at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Forecast {
    pub lanes: usize,
    pub slots: usize,
    /// The lanes that help: `min(lanes, max(1, floor(slots / build share)))`.
    pub effective_lanes: usize,
    /// Hours from now, to 0.1 h. Never less than the critical path.
    pub finish_h: f64,
}

/// From `at_h` hours on, `running` tasks run at once (with as many lanes as help).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Step {
    pub at_h: f64,
    pub running: usize,
}

/// A task of the scope that cannot start now, and every reason why.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Blocked {
    pub id: String,
    pub waits: Vec<taskgraph::Wait>,
}

/// A task of the scope in progress.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Running {
    pub id: String,
    pub text: String,
    /// The live agent scope working it.
    pub scope: Option<String>,
    /// Hours since it last went in progress (0 when the task log holds no such change).
    pub elapsed_h: f64,
    pub estimate_h: Option<f64>,
    /// It has run longer than its estimate.
    pub over: bool,
}

/// A date a person set: a running sprint's last day, or a release's target.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PersonDate {
    /// `SP-004 ends`, `v0.8.0 target`.
    pub what: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

/// A scope's work in agent-hours over its task graph, and how long it takes with more agents.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FlowPlan {
    /// `sprint:SP-004`, `release:v0.8.0`, `milestone:MS-004` or `open`.
    pub scope: String,
    /// Every task's estimate (an unestimated task counts 1.0 h).
    pub total_h: f64,
    /// The estimates of completed tasks.
    pub done_h: f64,
    /// What the open tasks weigh: an estimate, or what a running one has left.
    pub remaining_h: f64,
    /// The longest chain through the remaining work (tasks outside the scope it waits on
    /// included), start first.
    pub critical_path: Vec<String>,
    /// Its length: the least time the work can take however many agents run.
    pub critical_h: f64,
    /// The most tasks that ever run at once with unlimited lanes.
    pub widest: usize,
    /// How many tasks run at once over time, with unlimited lanes.
    pub profile: Vec<Step>,
    pub running: Vec<Running>,
    pub ready: Vec<taskgraph::ReadyTask>,
    pub blocked: Vec<Blocked>,
    pub forecasts: Vec<Forecast>,
    /// The part of a task's time spent in builds.
    pub build_share: f64,
    /// `build_share` was measured over completed tasks, not assumed or given.
    pub build_share_measured: bool,
    /// Tasks with no estimate, counted as 1.0 h.
    pub unestimated: Vec<String>,
    /// Items in the scope with no tasks: not planned here.
    pub untasked_items: Vec<String>,
    /// Open tasks outside the scope that its tasks wait on: simulated, not counted in totals.
    pub outside: Vec<String>,
    pub ratios: Vec<tasklog::KindRatio>,
    /// The only dates in the plan, each set by a person.
    pub dates: Vec<PersonDate>,
}

/// One task as the scheduler sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SimTask {
    pub id: String,
    /// Hours it still takes.
    pub weight_h: f64,
    /// Positions in the same slice of the tasks it waits on.
    pub deps: Vec<usize>,
    /// The files it writes; none is an unknown footprint.
    pub writes: Vec<String>,
    /// Its item, `project:CODE`.
    pub item: String,
    /// In progress now: it takes a lane before any task starts.
    pub running: bool,
    /// The longest chain starting at it, its own weight included: the start order.
    pub chain_h: f64,
}

/// The lanes and slots to forecast with: lanes 1, 4 and 8 plus the sprint's own, sorted and
/// deduplicated; slots the sprint's, else what the project's build settings give this machine
/// (FTR-062), else 2. The build share is left to be measured.
pub fn default_options(config: &ProjectConfig, sprint: Option<&Sprint>) -> ForecastOptions {
    let mut lanes: Vec<usize> = DEFAULT_LANES.to_vec();
    if let Some(n) = sprint.and_then(|s| s.lanes).filter(|n| *n > 0) {
        lanes.push(n as usize);
    }
    lanes.sort_unstable();
    lanes.dedup();
    let slots = match sprint.and_then(|s| s.build_slots).filter(|n| *n > 0) {
        Some(n) => n as usize,
        None => match std::thread::available_parallelism() {
            Ok(cpus) => {
                let cpus = u32::try_from(cpus.get()).unwrap_or(u32::MAX);
                config.build.effective_slots(cpus) as usize
            }
            Err(_) => DEFAULT_SLOTS,
        },
    };
    ForecastOptions {
        lanes,
        slots: slots.max(1),
        build_share: None,
    }
}

/// The deterministic list scheduler. From `t = 0`, running tasks take lanes first; then, at each
/// event, ready tasks start in order (`chain_h` descending, id ascending) while lanes are free. A
/// task is ready when every task it waits on has finished, it writes none of the files a running
/// task writes ([`taskgraph::overlaps`]), and no running task of its item has an unknown footprint
/// (nor, when its own is unknown, any running task of its item) — the rules
/// [`TaskGraph::readiness`] applies. Time then moves to the earliest finish. Returns the last
/// finish in hours and how many tasks run from each time on (a step at each change, the last
/// one 0). `lanes` of 0 is taken as 1.
pub fn simulate(nodes: &[SimTask], lanes: usize) -> (f64, Vec<Step>) {
    simulate_with(nodes, lanes, &[])
}

/// [`simulate`], with the project's append-only files, which two tasks may both write.
pub(crate) fn simulate_with(
    nodes: &[SimTask],
    lanes: usize,
    append_only: &[String],
) -> (f64, Vec<Step>) {
    const EPS: f64 = 1e-9;
    let lanes = lanes.max(1);
    let n = nodes.len();
    // Start order: running first, then chain descending, then id.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        let (x, y) = (&nodes[*a], &nodes[*b]);
        y.running
            .cmp(&x.running)
            .then(
                y.chain_h
                    .partial_cmp(&x.chain_h)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| x.id.cmp(&y.id))
    });
    let mut rank = vec![0usize; n];
    for (r, i) in order.iter().enumerate() {
        rank[*i] = r;
    }
    let mut waiting_on: Vec<usize> = vec![0; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, t) in nodes.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for d in &t.deps {
            if *d < n && *d != i && seen.insert(*d) {
                waiting_on[i] += 1;
                dependents[*d].push(i);
            }
        }
    }
    // Ready by rank; running tasks need no ready check.
    let mut ready: BTreeSet<usize> = BTreeSet::new();
    let mut pending_running: Vec<usize> = Vec::new();
    for i in 0..n {
        if nodes[i].running {
            pending_running.push(i);
        } else if waiting_on[i] == 0 {
            ready.insert(rank[i]);
        }
    }
    pending_running.sort_by_key(|i| rank[*i]);

    let mut writes = RunningWrites::default();
    // Per item: running tasks, and those of them with an unknown footprint.
    let mut per_item: HashMap<&str, (usize, usize)> = HashMap::new();
    // Running: (finish time, position).
    let mut running: Vec<(f64, usize)> = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut t = 0.0_f64;
    let mut last = 0.0_f64;

    /// Put task `i` in a lane at `t`.
    fn start<'a>(
        nodes: &'a [SimTask],
        i: usize,
        t: f64,
        running: &mut Vec<(f64, usize)>,
        writes: &mut RunningWrites,
        per_item: &mut HashMap<&'a str, (usize, usize)>,
    ) {
        let node = &nodes[i];
        running.push((t + node.weight_h.max(0.0), i));
        writes.add(&node.writes);
        let e = per_item.entry(node.item.as_str()).or_insert((0, 0));
        e.0 += 1;
        if node.writes.is_empty() {
            e.1 += 1;
        }
    }

    for i in std::mem::take(&mut pending_running) {
        if running.len() >= lanes {
            // More in progress than lanes: the rest start as lanes free, before anything else.
            pending_running.push(i);
            continue;
        }
        start(nodes, i, t, &mut running, &mut writes, &mut per_item);
    }

    loop {
        // Waiting running tasks take freed lanes first.
        while running.len() < lanes && !pending_running.is_empty() {
            let i = pending_running.remove(0);
            start(nodes, i, t, &mut running, &mut writes, &mut per_item);
        }
        if running.len() < lanes {
            let mut started: Vec<usize> = Vec::new();
            for &r in &ready {
                if running.len() >= lanes {
                    break;
                }
                let i = order[r];
                let node = &nodes[i];
                let (siblings, unknown) =
                    per_item.get(node.item.as_str()).copied().unwrap_or((0, 0));
                if unknown > 0 || (node.writes.is_empty() && siblings > 0) {
                    continue;
                }
                if writes.clashes(&node.writes, append_only) {
                    continue;
                }
                start(nodes, i, t, &mut running, &mut writes, &mut per_item);
                started.push(r);
            }
            for r in started {
                ready.remove(&r);
            }
        }
        let count = running.len();
        match steps.last_mut() {
            Some(s) if (s.at_h - t).abs() < EPS => s.running = count,
            Some(s) if s.running == count => {}
            _ => steps.push(Step {
                at_h: t,
                running: count,
            }),
        }
        if running.is_empty() {
            break;
        }
        // Advance to the earliest finish and finish every task ending then.
        let next = running
            .iter()
            .map(|(f, _)| *f)
            .fold(f64::INFINITY, f64::min);
        t = next;
        last = last.max(t);
        let mut still = Vec::with_capacity(running.len());
        for (f, i) in running.drain(..) {
            if f <= t + EPS {
                let node = &nodes[i];
                writes.remove(&node.writes);
                if let Some(e) = per_item.get_mut(node.item.as_str()) {
                    e.0 -= 1;
                    if node.writes.is_empty() {
                        e.1 -= 1;
                    }
                }
                for &d in &dependents[i] {
                    waiting_on[d] -= 1;
                    if waiting_on[d] == 0 && !nodes[d].running {
                        ready.insert(rank[d]);
                    }
                }
            } else {
                still.push((f, i));
            }
        }
        running = still;
    }
    // A task that could never start (one it waits on never finishes) is left out of the finish.
    (last, steps)
}

/// The files running tasks write, counted, so a candidate's clashes are found without comparing
/// it with every running task.
#[derive(Default)]
struct RunningWrites {
    by_path: BTreeMap<String, usize>,
}

impl RunningWrites {
    fn add(&mut self, writes: &[String]) {
        for w in writes {
            *self.by_path.entry(w.clone()).or_insert(0) += 1;
        }
    }

    fn remove(&mut self, writes: &[String]) {
        for w in writes {
            if let Some(c) = self.by_path.get_mut(w) {
                *c -= 1;
                if *c == 0 {
                    self.by_path.remove(w);
                }
            }
        }
    }

    /// One of `writes` overlaps a running task's written path as [`taskgraph::overlaps`] decides:
    /// equal, or one a folder (`/`-ended) holding the other; the more specific of the two is the
    /// overlap, and an append-only one is none.
    fn clashes(&self, writes: &[String], append_only: &[String]) -> bool {
        let covers = |f: &str, p: &str| f == p || (f.ends_with('/') && p.starts_with(f));
        let real = |specific: &str| !append_only.iter().any(|a| covers(a, specific));
        for w in writes {
            if self.by_path.contains_key(w.as_str()) && real(w) {
                return true;
            }
            for (i, c) in w.char_indices() {
                if c == '/' && i + 1 < w.len() && self.by_path.contains_key(&w[..=i]) && real(w) {
                    return true;
                }
            }
            if w.ends_with('/') {
                for (k, _) in self.by_path.range(w.clone()..) {
                    if !k.starts_with(w.as_str()) {
                        break;
                    }
                    if real(k) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// A task as the project names it: `CODE/TL-001/T3` there, `project:CODE/TL-001/T3` elsewhere.
fn label(id: &TaskId, project: &str) -> String {
    if id.project == project {
        id.local()
    } else {
        id.to_string()
    }
}

fn done(n: &TaskNode) -> bool {
    n.state == TaskState::Completed || n.item_terminal
}

fn estimate(n: &TaskNode) -> f64 {
    n.estimate_h
        .filter(|h| *h > 0.0)
        .unwrap_or(DEFAULT_ESTIMATE_H)
}

fn round_to(x: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (x * f).round() / f
}

fn parse_at(at: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(at.trim(), &Rfc3339).ok()
}

/// The median of `build_ms / agent_ms` over `project`'s completed tasks that reported builds,
/// once [`MEASURED_AFTER`] or more have.
fn measured_share(
    graph: &TaskGraph,
    project: &str,
    actuals: &BTreeMap<String, TaskActual>,
) -> Option<f64> {
    let mut shares: Vec<f64> = graph
        .nodes
        .values()
        .filter(|n| n.id.project == project && n.state == TaskState::Completed)
        .filter_map(|n| actuals.get(&n.id.local()))
        .filter(|a| a.agent_ms > 0 && a.build_ms > 0)
        .map(|a| (a.build_ms as f64 / a.agent_ms as f64).min(1.0))
        .collect();
    if shares.len() < MEASURED_AFTER {
        return None;
    }
    shares.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = shares.len() / 2;
    Some(if shares.len().is_multiple_of(2) {
        (shares[m - 1] + shares[m]) / 2.0
    } else {
        shares[m]
    })
}

/// The lanes that help with `slots` builds at once when `share` of a task's time is a build.
fn effective_lanes(lanes: usize, slots: usize, share: f64) -> usize {
    let lanes = lanes.max(1);
    if share <= 0.0 || !share.is_finite() {
        return lanes;
    }
    let bound = (slots as f64 / share).floor();
    let bound = if bound >= lanes as f64 {
        lanes
    } else {
        (bound as usize).max(1)
    };
    lanes.min(bound)
}

/// Each task's longest chain through those waiting on it, its own weight included.
fn chains(nodes: &[SimTask]) -> Vec<f64> {
    let n = nodes.len();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for (i, t) in nodes.iter().enumerate() {
        for d in &t.deps {
            dependents[*d].push(i);
            indegree[i] += 1;
        }
    }
    // Topological order (deps first), then valued in reverse.
    let mut topo: Vec<usize> = (0..n).filter(|i| indegree[*i] == 0).collect();
    let mut k = 0;
    while k < topo.len() {
        let v = topo[k];
        k += 1;
        for &d in &dependents[v] {
            indegree[d] -= 1;
            if indegree[d] == 0 {
                topo.push(d);
            }
        }
    }
    let mut chain = vec![0.0_f64; n];
    for &v in topo.iter().rev() {
        let best = dependents[v]
            .iter()
            .map(|d| chain[*d])
            .fold(0.0_f64, f64::max);
        chain[v] = nodes[v].weight_h + best;
    }
    chain
}

/// The critical path over the simulated tasks, start first, and its length: the largest chain,
/// ties to the smaller id, then each step the dependent with the largest chain.
fn critical(nodes: &[SimTask]) -> (Vec<usize>, f64) {
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, t) in nodes.iter().enumerate() {
        for d in &t.deps {
            dependents[*d].push(i);
        }
    }
    let better = |i: usize, p: Option<usize>| {
        p.is_none_or(|p| {
            nodes[i].chain_h > nodes[p].chain_h + 1e-12
                || ((nodes[i].chain_h - nodes[p].chain_h).abs() <= 1e-12
                    && nodes[i].id < nodes[p].id)
        })
    };
    let mut start: Option<usize> = None;
    for i in 0..nodes.len() {
        if better(i, start) {
            start = Some(i);
        }
    }
    let Some(start) = start else {
        return (Vec::new(), 0.0);
    };
    let mut path = vec![start];
    let mut cur = start;
    loop {
        let mut next: Option<usize> = None;
        for &d in &dependents[cur] {
            if !path.contains(&d) && better(d, next) {
                next = Some(d);
            }
        }
        match next {
            Some(d) => {
                path.push(d);
                cur = d;
            }
            None => break,
        }
    }
    (path, nodes[start].chain_h)
}

/// The flow plan of `scope` in project `p` at `now` (RFC3339). Errors: `ProjectNotFound`; a
/// sprint or release the project does not have is `Unsupported`, a milestone
/// `MilestoneNotFound`; a task graph that cannot be read as [`TaskGraph::build`] says. A scope
/// with no tasks is zeros and its untasked items, never an error.
pub fn plan(
    store: &Store,
    p: &str,
    scope: &PlanScope,
    opts: &ForecastOptions,
    now: &str,
) -> Result<FlowPlan> {
    let project = store.load_meta(p)?;
    plan_for(store, &project, scope, opts, now)
}

/// [`plan`] over a project already read: its items stand in for theirs on disk.
pub(crate) fn plan_for(
    store: &Store,
    project: &crate::Project,
    scope: &PlanScope,
    opts: &ForecastOptions,
    now: &str,
) -> Result<FlowPlan> {
    let p = project.id.as_str();
    let sprints = crate::sprints::load(store, p).unwrap_or_default();
    let releases = crate::releases::load(store, p).unwrap_or_default();
    let today = parse_at(now)
        .map(|t| t.date())
        .unwrap_or_else(crate::sprints::today);

    // The scope's items.
    let in_scope: BTreeSet<String> = match scope {
        PlanScope::Sprint(code) => {
            if !sprints.iter().any(|s| s.code == *code) {
                return Err(CoreError::Unsupported(format!("no sprint {code}")));
            }
            project
                .features
                .iter()
                .filter(|f| f.sprint.as_deref() == Some(code.as_str()))
                .map(|f| f.code.clone())
                .collect()
        }
        PlanScope::Release(version) => crate::releases::report_all(store, p)?
            .into_iter()
            .find(|r| r.release.version == *version)
            .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?
            .items
            .into_iter()
            .collect(),
        PlanScope::Milestone(ms) => {
            if !project.milestones.iter().any(|m| m.code == *ms) {
                return Err(CoreError::MilestoneNotFound(ms.clone()));
            }
            project
                .features
                .iter()
                .filter(|f| f.milestone == *ms)
                .map(|f| f.code.clone())
                .collect()
        }
        PlanScope::Open => project
            .features
            .iter()
            .filter(|f| !is_terminal_status(&project.config, &f.status))
            .map(|f| f.code.clone())
            .collect(),
    };

    let graph = TaskGraph::build(store, p, Some(project))?;
    let events = tasklog::read_all(store.data_dir(), p);
    let actuals = tasklog::actuals(&events, now);
    let now_t = parse_at(now);
    let append_only = &project.config.tasks.append_only;

    let mine = |n: &TaskNode| n.id.project == p && in_scope.contains(&n.id.code);
    let scope_keys: Vec<&String> = graph
        .nodes
        .iter()
        .filter(|(_, n)| mine(n))
        .map(|(k, _)| k)
        .collect();

    let tasked: BTreeSet<&str> = scope_keys
        .iter()
        .map(|k| graph.nodes[*k].id.code.as_str())
        .collect();
    let untasked_items: Vec<String> = in_scope
        .iter()
        .filter(|c| !tasked.contains(c.as_str()))
        .cloned()
        .collect();

    // Totals over the scope's tasks.
    let mut total_h = 0.0;
    let mut done_h = 0.0;
    let mut unestimated = Vec::new();
    for k in &scope_keys {
        let n = &graph.nodes[*k];
        let e = estimate(n);
        total_h += e;
        if done(n) {
            done_h += e;
        }
        if n.estimate_h.filter(|h| *h > 0.0).is_none() {
            unestimated.push(label(&n.id, p));
        }
    }

    // The simulated tasks: the scope's open tasks and, transitively, the open tasks they wait on.
    // Walked by task id, so no edge needs its key built.
    let by_id: HashMap<&TaskId, &TaskNode> = graph.nodes.values().map(|n| (&n.id, n)).collect();
    let mut seen: BTreeSet<&TaskId> = BTreeSet::new();
    let mut stack: Vec<&TaskId> = scope_keys
        .iter()
        .map(|k| &graph.nodes[*k])
        .filter(|n| !done(n))
        .map(|n| &n.id)
        .collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let n = by_id[id];
        for d in n.depends_on.iter().chain(n.implicit.iter()) {
            if by_id.get(d).is_some_and(|dn| !done(dn)) && !seen.contains(d) {
                stack.push(d);
            }
        }
    }
    // In key order (`seen` is ordered as the graph's keys are).
    let sim_nodes: Vec<&TaskNode> = seen.iter().map(|id| by_id[*id]).collect();
    let pos: HashMap<&TaskId, usize> = sim_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.id, i))
        .collect();

    let elapsed_h = |n: &TaskNode| -> f64 {
        let since = actuals
            .get(&n.id.local())
            .filter(|_| n.id.project == p)
            .and_then(|a| a.running_since.as_deref())
            .and_then(parse_at);
        match (since, now_t) {
            (Some(s), Some(t)) if t > s => (t - s).as_seconds_f64() / 3600.0,
            _ => 0.0,
        }
    };

    let mut remaining_h = 0.0;
    let mut outside = Vec::new();
    let mut sim: Vec<SimTask> = Vec::with_capacity(sim_nodes.len());
    for n in sim_nodes.iter().copied() {
        let running = n.state == TaskState::InProgress;
        let weight_h = if running {
            (estimate(n) - elapsed_h(n)).max(MIN_REMAINING_H)
        } else {
            estimate(n)
        };
        if mine(n) {
            remaining_h += weight_h;
        } else {
            outside.push(label(&n.id, p));
        }
        // A running task already runs: what it waits on no longer holds it, nor lengthens a chain.
        let mut deps: Vec<usize> = if running {
            Vec::new()
        } else {
            n.depends_on
                .iter()
                .chain(n.implicit.iter())
                .filter_map(|d| pos.get(d).copied())
                .collect()
        };
        deps.sort_unstable();
        deps.dedup();
        sim.push(SimTask {
            id: label(&n.id, p),
            weight_h,
            deps,
            writes: n.writes.clone(),
            item: format!("{}:{}", n.id.project, n.id.code),
            running,
            chain_h: 0.0,
        });
    }
    let chain = chains(&sim);
    for (t, c) in sim.iter_mut().zip(chain) {
        t.chain_h = c;
    }
    let (path, critical_h) = critical(&sim);
    let critical_ids: BTreeSet<&str> = path.iter().map(|i| sim[*i].id.as_str()).collect();
    let critical_path: Vec<String> = path.iter().map(|i| sim[*i].id.clone()).collect();

    let (_, profile) = simulate_with(&sim, usize::MAX, append_only);
    let widest = profile.iter().map(|s| s.running).max().unwrap_or(0);

    let (build_share, build_share_measured) = match opts.build_share {
        Some(s) => (s, false),
        None => match measured_share(&graph, p, &actuals) {
            Some(s) => (s, true),
            None => (ASSUMED_BUILD_SHARE, false),
        },
    };
    let critical_h = round_to(critical_h, 1);
    let forecasts: Vec<Forecast> = opts
        .lanes
        .iter()
        .map(|&lanes| {
            let effective = effective_lanes(lanes, opts.slots, build_share);
            let (finish, _) = simulate_with(&sim, effective, append_only);
            Forecast {
                lanes,
                slots: opts.slots,
                effective_lanes: effective,
                finish_h: round_to(finish, 1),
            }
        })
        .collect();

    // Running, ready and blocked: the graph's readiness, kept to the scope.
    let readiness = graph.readiness(p, append_only, None);
    let scope_ids: BTreeSet<String> = scope_keys
        .iter()
        .map(|k| label(&graph.nodes[*k].id, p))
        .collect();
    let by_label: HashMap<String, &TaskNode> = scope_keys
        .iter()
        .map(|k| (label(&graph.nodes[*k].id, p), &graph.nodes[*k]))
        .collect();
    let running: Vec<Running> = readiness
        .running
        .iter()
        .filter_map(|r| by_label.get(&r.id).map(|n| (r, *n)))
        .map(|(r, n)| {
            let elapsed = elapsed_h(n);
            Running {
                id: r.id.clone(),
                text: n.text.clone(),
                scope: n.scope.clone(),
                elapsed_h: round_to(elapsed, 2),
                estimate_h: n.estimate_h,
                over: n.estimate_h.is_some_and(|e| elapsed > e),
            }
        })
        .collect();
    let sim_chain: HashMap<&str, f64> = sim.iter().map(|t| (t.id.as_str(), t.chain_h)).collect();
    let mut ready: Vec<taskgraph::ReadyTask> = readiness
        .ready
        .into_iter()
        .filter(|r| scope_ids.contains(&r.id))
        .map(|mut r| {
            r.critical = critical_ids.contains(r.id.as_str());
            r.chain_h = round_to(
                sim_chain.get(r.id.as_str()).copied().unwrap_or(r.chain_h),
                2,
            );
            r
        })
        .collect();
    ready.sort_by(|a, b| {
        b.critical
            .cmp(&a.critical)
            .then(
                b.chain_h
                    .partial_cmp(&a.chain_h)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| a.id.cmp(&b.id))
    });
    let blocked: Vec<Blocked> = readiness
        .waiting
        .into_iter()
        .filter(|w| scope_ids.contains(&w.id))
        .map(|w| Blocked {
            id: w.id,
            waits: w.waits,
        })
        .collect();

    // The dates a person set: the running sprint's last day, and the targets of the releases in
    // scope.
    let mut dates = Vec::new();
    if let Some(s) = crate::sprints::active_on(&sprints, today)
        && !s.end.trim().is_empty()
    {
        dates.push(PersonDate {
            what: format!("{} ends", s.code),
            date: s.end.clone(),
        });
    }
    let scope_releases: BTreeSet<&str> = match scope {
        PlanScope::Release(v) => BTreeSet::from([v.as_str()]),
        _ => project
            .features
            .iter()
            .filter(|f| in_scope.contains(&f.code))
            .filter_map(|f| f.release.as_deref())
            .collect(),
    };
    for r in &releases {
        if scope_releases.contains(r.version.as_str()) && !r.target.trim().is_empty() {
            dates.push(PersonDate {
                what: format!("{} target", r.version),
                date: r.target.clone(),
            });
        }
    }

    Ok(FlowPlan {
        scope: scope.label(),
        total_h: round_to(total_h, 2),
        done_h: round_to(done_h, 2),
        remaining_h: round_to(remaining_h, 2),
        critical_path,
        critical_h,
        widest,
        profile: profile
            .into_iter()
            .map(|s| Step {
                at_h: round_to(s.at_h, 2),
                running: s.running,
            })
            .collect(),
        running,
        ready,
        blocked,
        forecasts,
        build_share,
        build_share_measured,
        unestimated,
        untasked_items,
        outside,
        ratios: tasklog::ratios(&graph, p, &actuals),
        dates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;
    use crate::models::{FeatureItem, Task, TodoList};
    use crate::tasklog::{BuildTotals, TaskEvent};
    use TaskState::{Completed as Done, InProgress as Running, NotStarted as Open};
    use std::path::PathBuf;

    const NOW: &str = "2026-11-17T12:00:00Z";

    struct Board {
        store: Store,
        dir: PathBuf,
        build: String,
    }

    impl Drop for Board {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Board {
        fn new(name: &str) -> Board {
            let dir = std::env::temp_dir()
                .join(format!("kanbanr-forecast-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // L-41: the board is this test's own, under the system temp dir.
            assert!(dir.starts_with(std::env::temp_dir()));
            let store = Store::new(dir.clone());
            let mut config = ProjectConfig::for_new_project("demo");
            config.cadence.sprints = true;
            config.cadence.releases = true;
            let build = config.role_status(Role::Build).expect("a build stage");
            store.init_project("demo", config).unwrap();
            store
                .add_milestone("demo", "M", "", vec![], Some("M".into()))
                .unwrap();
            Board { store, dir, build }
        }

        /// An item at the build stage with list TL-001 holding `tasks` (none: no list).
        fn item(&self, code: &str, tasks: Vec<Task>) -> FeatureItem {
            let mut f = self
                .store
                .add_feature("demo", code, "", "M", Some(code.into()))
                .unwrap();
            f.status = self.build.clone();
            if !tasks.is_empty() {
                f.todo_lists = vec![TodoList {
                    code: "TL-001".into(),
                    description: String::new(),
                    tasks,
                    created_at: crate::now_rfc3339(),
                    extra: Default::default(),
                }];
            }
            self.rewrite(&f);
            f
        }

        /// Write `f` where its status puts it, its files at any other status removed.
        fn rewrite(&self, f: &FeatureItem) {
            remove_item_files(&self.store.features_dir("demo"), &f.code);
            self.store.persist_feature_for_test("demo", f).unwrap();
        }

        fn plan(&self, scope: &PlanScope, opts: &ForecastOptions) -> FlowPlan {
            plan(&self.store, "demo", scope, opts, NOW).unwrap()
        }
    }

    fn remove_item_files(dir: &std::path::Path, code: &str) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                remove_item_files(&p, code);
            } else if p.file_stem().is_some_and(|s| s.to_string_lossy() == code) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    fn task(key: &str, state: TaskState, deps: &[&str], writes: &[&str], est: Option<f64>) -> Task {
        Task {
            key: key.into(),
            text: format!("task {key}"),
            state,
            changed_at: None,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            writes: writes.iter().map(|d| d.to_string()).collect(),
            reads: vec![],
            estimate_h: est,
            extra: Default::default(),
        }
    }

    fn opts(lanes: &[usize], slots: usize, share: f64) -> ForecastOptions {
        ForecastOptions {
            lanes: lanes.to_vec(),
            slots,
            build_share: Some(share),
        }
    }

    /// Nine tasks over three items, 1–4 h each. FTR-1 is a 9 h chain; FTR-2's first task writes
    /// the `web/` folder FTR-1's last writes into; FTR-3's first two write the same CLI folder and
    /// so never run together.
    fn fixture(b: &Board) {
        b.item(
            "FTR-1",
            vec![
                task(
                    "T1",
                    Open,
                    &[],
                    &["api/crates/kanbanr-core/src/a.rs"],
                    Some(2.0),
                ),
                task(
                    "T2",
                    Open,
                    &["T1"],
                    &["api/crates/kanbanr-core/src/b.rs"],
                    Some(3.0),
                ),
                task("T3", Open, &["T2"], &["web/src/x.ts"], Some(4.0)),
            ],
        );
        b.item(
            "FTR-2",
            vec![
                task("T1", Open, &[], &["web/"], Some(1.0)),
                task("T2", Open, &["T1"], &["docs/b.md"], Some(2.0)),
                task("T3", Open, &["T2"], &["docs/c.md"], Some(1.0)),
            ],
        );
        b.item(
            "FTR-3",
            vec![
                task("T1", Open, &[], &["api/crates/kanbanr-cli/"], Some(1.0)),
                task(
                    "T2",
                    Open,
                    &[],
                    &["api/crates/kanbanr-cli/src/main.rs"],
                    Some(2.0),
                ),
                task(
                    "T3",
                    Open,
                    &["T1", "T2"],
                    &["api/crates/kanbanr-server/src/s.rs"],
                    Some(3.0),
                ),
            ],
        );
    }

    fn steps(v: &[(f64, usize)]) -> Vec<Step> {
        v.iter()
            .map(|(at_h, running)| Step {
                at_h: *at_h,
                running: *running,
            })
            .collect()
    }

    /// R-1: the critical path through the fixture is FTR-1's chain, 9 h; at most three tasks run
    /// at once; the profile says how many run over time.
    #[test]
    fn the_critical_path_and_widest_parallelism_of_a_fixture_graph() {
        let b = Board::new("critical");
        fixture(&b);
        let got = b.plan(&PlanScope::Open, &opts(&[1, 4, 8], 8, 0.25));
        assert_eq!(got.scope, "open");
        assert_eq!(
            got.critical_path,
            vec!["FTR-1/TL-001/T1", "FTR-1/TL-001/T2", "FTR-1/TL-001/T3"]
        );
        assert_eq!(got.critical_h, 9.0);
        assert_eq!(got.widest, 3);
        assert_eq!(
            got.profile,
            steps(&[(0.0, 3), (4.0, 2), (6.0, 1), (9.0, 0)])
        );
        assert_eq!(got.total_h, 19.0);
        assert_eq!(got.done_h, 0.0);
        assert_eq!(got.remaining_h, 19.0);
        assert!(got.unestimated.is_empty() && got.untasked_items.is_empty());
        assert!(got.outside.is_empty());
        // Ready now: the first tasks, FTR-3's two waiting on each other's folder only once one
        // runs; the critical one first.
        assert_eq!(got.ready[0].id, "FTR-1/TL-001/T1");
        assert!(got.ready[0].critical);
        assert_eq!(got.ready[0].chain_h, 9.0);
        assert!(got.blocked.iter().any(|w| w.id == "FTR-1/TL-001/T2"));
    }

    /// R-1: a task without an estimate weighs 1.0 h and is named; an item with no tasks is named
    /// and counts nothing.
    #[test]
    fn unestimated_tasks_and_untasked_items_are_named_not_invented() {
        let b = Board::new("unestimated");
        b.item(
            "FTR-1",
            vec![
                task("T1", Open, &[], &["a.rs"], None),
                task("T2", Open, &["T1"], &["b.rs"], None),
                task("T3", Done, &[], &["c.rs"], Some(2.0)),
            ],
        );
        b.item("FTR-2", vec![]);
        let got = b.plan(&PlanScope::Open, &opts(&[1], 2, 0.25));
        assert_eq!(got.unestimated, vec!["FTR-1/TL-001/T1", "FTR-1/TL-001/T2"]);
        assert_eq!(got.untasked_items, vec!["FTR-2"]);
        assert_eq!(got.total_h, 4.0);
        assert_eq!(got.done_h, 2.0);
        assert_eq!(got.remaining_h, 2.0);
        assert_eq!(got.critical_h, 2.0);
        assert_eq!(got.forecasts[0].finish_h, 2.0);

        // A board with no tasks at all: zeros, never an error.
        let empty = Board::new("no-tasks");
        empty.item("FTR-9", vec![]);
        let got = empty.plan(&PlanScope::Open, &opts(&[1, 4], 2, 0.25));
        assert_eq!(got.total_h, 0.0);
        assert_eq!(got.critical_h, 0.0);
        assert_eq!(got.widest, 0);
        assert!(got.critical_path.is_empty());
        assert_eq!(got.untasked_items, vec!["FTR-9"]);
        assert!(got.forecasts.iter().all(|f| f.finish_h == 0.0));
    }

    /// R-2: one lane does the whole 19 h in turn; four and eight lanes reach the 9 h critical path,
    /// the floor no lane count goes under; tasks writing the same files never start together.
    #[test]
    fn the_forecast_for_one_four_and_eight_lanes() {
        let b = Board::new("lanes");
        fixture(&b);
        let got = b.plan(&PlanScope::Open, &opts(&[1, 4, 8], 8, 0.25));
        let finish: Vec<(usize, usize, f64)> = got
            .forecasts
            .iter()
            .map(|f| (f.lanes, f.effective_lanes, f.finish_h))
            .collect();
        assert_eq!(finish, vec![(1, 1, 19.0), (4, 4, 9.0), (8, 8, 9.0)]);
        for f in &got.forecasts {
            assert!(
                f.finish_h >= got.critical_h,
                "{f:?} under the critical path"
            );
            assert_eq!(f.slots, 8);
        }

        // Two tasks writing one folder, nothing between them: one after the other, however many
        // lanes; an append-only file never holds two tasks apart.
        let t = |id: &str, item: &str, writes: &[&str]| SimTask {
            id: id.into(),
            weight_h: 1.0,
            deps: vec![],
            writes: writes.iter().map(|s| s.to_string()).collect(),
            item: item.into(),
            running: false,
            chain_h: 1.0,
        };
        let clash = [t("a", "p:A", &["src/"]), t("b", "p:B", &["src/main.rs"])];
        let (f, profile) = simulate(&clash, 8);
        assert_eq!(f, 2.0);
        assert!(profile.iter().all(|s| s.running <= 1), "{profile:?}");
        let log = [
            t("a", "p:A", &["CHANGELOG.md"]),
            t("b", "p:B", &["CHANGELOG.md"]),
        ];
        assert_eq!(simulate(&log, 8).0, 2.0);
        assert_eq!(simulate_with(&log, 8, &["CHANGELOG.md".into()]).0, 1.0);
        // Two tasks of one item with no recorded files never run together either.
        let unknown = [t("a", "p:A", &[]), t("b", "p:A", &[])];
        assert_eq!(simulate(&unknown, 8).0, 2.0);
    }

    fn complete_with_build(b: &Board, code: &str, key: &str, hour: u32, run_min: u64) {
        let at = |h: u32| format!("2026-11-16T{h:02}:00:00Z");
        let ev = |at: String, from, to, build| TaskEvent {
            at,
            item: code.into(),
            todo: "TL-001".into(),
            task: key.into(),
            from,
            to,
            tokens: None,
            cost_usd: None,
            model: None,
            build,
        };
        let data = b.store.data_dir();
        tasklog::append(data, "demo", ev(at(hour), Open, Running, None));
        tasklog::append(
            data,
            "demo",
            ev(
                at(hour + 1),
                Running,
                Done,
                Some(BuildTotals {
                    runs: 1,
                    red: 0,
                    run_ms: run_min * 60_000,
                    wait_ms: 0,
                }),
            ),
        );
    }

    /// R-2: with one build slot and half of each task spent building, two lanes are all that
    /// help; the share is measured once five completed tasks have build actuals, assumed before.
    #[test]
    fn build_slots_bound_the_lanes_that_help() {
        let b = Board::new("slots");
        fixture(&b);
        let got = b.plan(&PlanScope::Open, &opts(&[2, 8], 1, 0.5));
        assert_eq!(got.forecasts[1].lanes, 8);
        assert_eq!(got.forecasts[1].effective_lanes, 2);
        assert_eq!(got.forecasts[1].finish_h, got.forecasts[0].finish_h);
        assert!(got.forecasts[1].finish_h > got.critical_h);
        assert_eq!(effective_lanes(8, 2, 0.25), 8);
        assert_eq!(effective_lanes(8, 0, 0.25), 1);
        assert_eq!(effective_lanes(3, 1, 0.0), 3);

        // Measured: four completed tasks with builds are not enough; the fifth is.
        let done: Vec<Task> = (1..=5)
            .map(|k| task(&format!("T{k}"), Done, &[], &["x.rs"], Some(1.0)))
            .collect();
        b.item("FTR-4", done);
        let unmeasured = ForecastOptions {
            lanes: vec![8],
            slots: 1,
            build_share: None,
        };
        for k in 1..=4 {
            complete_with_build(&b, "FTR-4", &format!("T{k}"), 2 * k, 30);
        }
        let got = b.plan(&PlanScope::Open, &unmeasured);
        assert_eq!(got.build_share, 0.25);
        assert!(!got.build_share_measured);
        assert_eq!(got.forecasts[0].effective_lanes, 4);
        complete_with_build(&b, "FTR-4", "T5", 12, 30);
        let got = b.plan(&PlanScope::Open, &unmeasured);
        assert!((got.build_share - 0.5).abs() < 1e-9, "{}", got.build_share);
        assert!(got.build_share_measured);
        assert_eq!(got.forecasts[0].effective_lanes, 2);

        // The defaults: lanes 1, 4 and 8 plus the sprint's own; its slots when it states them.
        let config = ProjectConfig::for_new_project("demo");
        let sprint = Sprint {
            lanes: Some(3),
            build_slots: Some(5),
            ..Sprint::default()
        };
        let d = default_options(&config, Some(&sprint));
        assert_eq!(d.lanes, vec![1, 3, 4, 8]);
        assert_eq!(d.slots, 5);
        assert_eq!(d.build_share, None);
        let d = default_options(&config, None);
        assert_eq!(d.lanes, vec![1, 4, 8]);
        assert!(d.slots >= 1);
    }

    /// Every string in `v` that reads as a calendar date (`YYYY-MM-DD` anywhere in it), with the
    /// path to it.
    fn dates_in(v: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
        match v {
            serde_json::Value::String(s) => {
                let b = s.as_bytes();
                let is_date = b.windows(10).any(|w| {
                    w.iter().enumerate().all(|(i, c)| match i {
                        4 | 7 => *c == b'-',
                        _ => c.is_ascii_digit(),
                    })
                });
                if is_date {
                    out.push((path.to_string(), s.clone()));
                }
            }
            serde_json::Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    dates_in(x, &format!("{path}[{i}]"), out);
                }
            }
            serde_json::Value::Object(o) => {
                for (k, x) in o {
                    dates_in(x, &format!("{path}.{k}"), out);
                }
            }
            _ => {}
        }
    }

    /// R-7: the plan is in hours; the only dates it holds are under `dates`, each one a person
    /// set — the running sprint's last day and the target of a release in scope.
    #[test]
    fn a_forecast_is_in_hours_and_names_only_dates_a_person_set() {
        let b = Board::new("dates");
        fixture(&b);
        let sp = crate::sprints::add(
            &b.store,
            "demo",
            "2026-11-16",
            Some(14),
            "",
            None,
            "",
            Some(3),
            Some(2),
        )
        .unwrap();
        crate::sprints::start_on(
            &b.store,
            "demo",
            &sp.code,
            time::Date::from_calendar_date(2026, time::Month::November, 17).unwrap(),
        )
        .unwrap();
        crate::releases::add(&b.store, "demo", "v0.8.0", "2026-12-20", "").unwrap();
        // A release no item in scope is in: its target is not this plan's.
        crate::releases::add(&b.store, "demo", "v0.9.0", "2027-01-31", "").unwrap();
        let project = b.store.load_meta("demo").unwrap();
        for code in ["FTR-1", "FTR-2", "FTR-3"] {
            let mut f = project.feature(code).unwrap().clone();
            f.sprint = Some(sp.code.clone());
            f.release = Some("v0.8.0".into());
            if code == "FTR-1" {
                f.todo_lists[0].tasks[0].state = Running;
                f.todo_lists[0].tasks[0].changed_at = Some("2026-11-17T11:00:00Z".into());
            }
            b.rewrite(&f);
        }
        // FTR-1's first task went in progress an hour before now.
        tasklog::append(
            b.store.data_dir(),
            "demo",
            TaskEvent {
                at: "2026-11-17T11:00:00Z".into(),
                item: "FTR-1".into(),
                todo: "TL-001".into(),
                task: "T1".into(),
                from: Open,
                to: Running,
                tokens: None,
                cost_usd: None,
                model: None,
                build: None,
            },
        );

        let sprints = crate::sprints::load(&b.store, "demo").unwrap();
        let today = time::Date::from_calendar_date(2026, time::Month::November, 17).unwrap();
        let scope = PlanScope::default_for(&sprints, today);
        assert_eq!(scope, PlanScope::Sprint(sp.code.clone()));
        let got = b.plan(&scope, &opts(&[1, 3, 4, 8], 2, 0.25));
        assert_eq!(got.scope, format!("sprint:{}", sp.code));
        assert_eq!(
            got.dates,
            vec![
                PersonDate {
                    what: format!("{} ends", sp.code),
                    date: "2026-11-29".into(),
                },
                PersonDate {
                    what: "v0.8.0 target".into(),
                    date: "2026-12-20".into(),
                },
            ]
        );
        // The running task: an hour in of two, so one left, and that shortens the chain.
        assert_eq!(got.running.len(), 1);
        assert_eq!(got.running[0].id, "FTR-1/TL-001/T1");
        assert_eq!(got.running[0].elapsed_h, 1.0);
        assert!(!got.running[0].over);
        assert_eq!(got.critical_h, 8.0);

        let json = serde_json::to_value(&got).unwrap();
        let mut found = Vec::new();
        dates_in(&json, "", &mut found);
        assert!(!found.is_empty());
        for (path, s) in &found {
            assert!(
                path.starts_with(".dates["),
                "a date outside `dates`: {path} = {s}"
            );
        }
        assert_eq!(PlanScope::default_for(&[], today), PlanScope::Open);
    }

    /// R-9: the plan with four forecasts over 3,000 tasks within 300 ms (median of 5) in a debug
    /// build, the project's items once read; 100 ms in a release build. The times are printed.
    #[test]
    fn the_plan_stays_within_budget_on_3000_tasks() {
        let b = Board::new("budget");
        let template = b
            .store
            .add_feature("demo", "seed", "", "M", Some("SEED-1".into()))
            .unwrap();
        for i in 0..300 {
            let mut f = template.clone();
            f.code = format!("FTR-{i}");
            f.status = b.build.clone();
            if i > 0 && i % 3 != 0 {
                f.depends_on = vec![format!("FTR-{}", i - 1)];
            }
            f.todo_lists = vec![TodoList {
                code: "TL-001".into(),
                description: String::new(),
                tasks: (1..=10)
                    .map(|k| {
                        let state = match (i + k) % 7 {
                            0 => Done,
                            1 => Running,
                            _ => Open,
                        };
                        let dep = format!("T{}", k - 1);
                        let deps: Vec<&str> = if k > 1 { vec![dep.as_str()] } else { vec![] };
                        task(
                            &format!("T{k}"),
                            state,
                            &deps,
                            &[&format!("src/m{}/f{k}.rs", i % 40)],
                            Some(1.5),
                        )
                    })
                    .collect(),
                created_at: crate::now_rfc3339(),
                extra: Default::default(),
            }];
            b.store.persist_feature_for_test("demo", &f).unwrap();
        }
        let o = opts(&[1, 4, 8, 16], 2, 0.25);
        // Reading 300 item files is the store's cost, not the plan's (as FTR-060's budget): it
        // is timed once and printed, the plan over the read project is held to the budget.
        let at = std::time::Instant::now();
        let project = b.store.load_meta("demo").unwrap();
        println!("reading the 300 items: {:?}", at.elapsed());
        let at = std::time::Instant::now();
        b.plan(&PlanScope::Open, &o);
        println!("the plan, the read included: {:?}", at.elapsed());
        let mut times = Vec::new();
        for _ in 0..5 {
            let at = std::time::Instant::now();
            let got = plan_for(&b.store, &project, &PlanScope::Open, &o, NOW).unwrap();
            times.push(at.elapsed());
            assert_eq!(got.forecasts.len(), 4);
            assert!(got.total_h > 4000.0, "{}", got.total_h);
            for f in &got.forecasts {
                assert!(
                    f.finish_h >= got.critical_h,
                    "{f:?} under {}",
                    got.critical_h
                );
            }
        }
        times.sort();
        let median = times[2];
        let budget_ms = if cfg!(debug_assertions) { 300 } else { 100 };
        println!("the plan over 3,000 tasks: median {median:?} (budget {budget_ms} ms)");
        assert!(
            median < std::time::Duration::from_millis(budget_ms),
            "median {median:?} over 3,000 tasks, budget {budget_ms} ms"
        );
    }
}
