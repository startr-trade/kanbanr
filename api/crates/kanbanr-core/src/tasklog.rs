//! The task log (FTR-061): every change of a task's state, kept as a day log under
//! `projects/<id>/tasklog/` (FEAT-066), with the tokens, cost, model and build time an agent
//! reports when it makes the change.
//!
//! A task's record keeps only its current `state` and `changed_at`; this log keeps every change,
//! so the time a task spent in progress, how often it was reopened, and what it cost are derived
//! from raw entries rather than stored as totals (ADR-0002). Estimates are compared with these
//! actuals per kind of task, the kind read from the files the task writes.

use crate::models::TaskState;
use crate::taskgraph::TaskGraph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The day log's folder under `projects/<id>/`.
pub const FOLDER: &str = "tasklog";

/// The builds a task ran while it was in progress, summed (from the machine-local build log,
/// FTR-062).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BuildTotals {
    pub runs: u32,
    pub red: u32,
    pub run_ms: u64,
    pub wait_ms: u64,
}

/// One change of one task's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TaskEvent {
    /// RFC3339, the `changed_at` the task write recorded.
    pub at: String,
    /// The item's code.
    pub item: String,
    /// The todo-list's code, `TL-001`.
    pub todo: String,
    /// The task's key, `T3`.
    pub task: String,
    pub from: TaskState,
    pub to: TaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildTotals>,
}

impl TaskEvent {
    /// `CODE/TL-001/T3`, the key [`actuals`] files the event under.
    pub fn task_id(&self) -> String {
        format!("{}/{}/{}", self.item, self.todo, self.task)
    }
}

impl crate::daylog::Dated for TaskEvent {
    fn at(&self) -> &str {
        &self.at
    }
}

/// Append one event to its day's file. Best-effort: a failure never fails the task write.
pub fn append(data_dir: &Path, project: &str, e: TaskEvent) {
    crate::daylog::append(data_dir, project, FOLDER, e);
}

/// Every event, oldest first. A malformed day file is skipped.
pub fn read_all(data_dir: &Path, project: &str) -> Vec<TaskEvent> {
    crate::daylog::read_all(data_dir, project, FOLDER)
}

/// What a task took, derived from its events.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TaskActual {
    /// `CODE/TL-001/T3`.
    pub id: String,
    /// Time in progress, every interval summed; an interval still open counts up to `now`.
    pub agent_ms: u64,
    /// Time in builds, from the reported build totals.
    pub build_ms: u64,
    /// Time waiting for a build slot, from the reported build totals.
    pub wait_ms: u64,
    /// Times the task was reopened after it was Completed.
    pub rework: u32,
    /// Builds that failed, from the reported build totals.
    pub red_builds: u32,
    pub tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    /// When it last went in progress, while it still is.
    pub running_since: Option<String>,
}

/// Actuals per task, keyed by `CODE/TL-001/T3`. Each task's events are read in time order:
/// `agent_ms` sums its InProgress intervals, one still open counting up to `now`; `rework` counts
/// Completed → NotStarted | InProgress; build totals, tokens and cost are summed where reported.
/// An event whose time cannot be read closes or opens no interval.
pub fn actuals(events: &[TaskEvent], now: &str) -> BTreeMap<String, TaskActual> {
    let mut ordered: Vec<&TaskEvent> = events.iter().collect();
    ordered.sort_by(|a, b| a.at.cmp(&b.at));
    let now = parse(now);
    let mut out: BTreeMap<String, TaskActual> = BTreeMap::new();
    // The open InProgress interval per task: its start as written and parsed.
    let mut open: BTreeMap<String, (String, Option<OffsetDateTime>)> = BTreeMap::new();
    for e in ordered {
        let id = e.task_id();
        let a = out.entry(id.clone()).or_insert_with(|| TaskActual {
            id: id.clone(),
            ..TaskActual::default()
        });
        let at = parse(&e.at);
        if e.to != TaskState::InProgress
            && let Some((_, since)) = open.remove(&id)
        {
            a.agent_ms += span_ms(since, at);
        }
        if e.to == TaskState::InProgress && !open.contains_key(&id) {
            open.insert(id.clone(), (e.at.clone(), at));
        }
        if e.from == TaskState::Completed && e.to != TaskState::Completed {
            a.rework += 1;
        }
        if let Some(b) = e.build {
            a.build_ms += b.run_ms;
            a.wait_ms += b.wait_ms;
            a.red_builds += b.red;
        }
        if let Some(t) = e.tokens {
            a.tokens = Some(a.tokens.unwrap_or(0) + t);
        }
        if let Some(c) = e.cost_usd {
            a.cost_usd = Some(a.cost_usd.unwrap_or(0.0) + c);
        }
    }
    for (id, (since_at, since)) in open {
        if let Some(a) = out.get_mut(&id) {
            a.agent_ms += span_ms(since, now);
            a.running_since = Some(since_at);
        }
    }
    out
}

/// Milliseconds from `from` to `to`, 0 when either is unreadable or `to` is earlier.
fn span_ms(from: Option<OffsetDateTime>, to: Option<OffsetDateTime>) -> u64 {
    match (from, to) {
        (Some(f), Some(t)) if t > f => (t - f).whole_milliseconds() as u64,
        _ => 0,
    }
}

/// The kinds, in the order a tie is broken.
const KINDS: [&str; 7] = ["core", "cli", "server", "web", "docs", "ci", "other"];

/// The area one written path lies in.
fn area_of(path: &str) -> &'static str {
    let path = path.trim().trim_start_matches("./");
    let name = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("api/crates/kanbanr-core/") {
        "core"
    } else if path.starts_with("api/crates/kanbanr-cli/") {
        "cli"
    } else if path.starts_with("api/crates/kanbanr-server/") {
        "server"
    } else if path.starts_with("web/") {
        "web"
    } else if path.starts_with("docs/") || path.starts_with("skill/") || name.ends_with(".md") {
        "docs"
    } else if path.starts_with("scripts/") || path.starts_with(".github/") || name == "Makefile" {
        "ci"
    } else {
        "other"
    }
}

/// The kind of a task from the files it writes: core, cli, server, web, docs, ci or other, the
/// most frequent, ties in that order; "unknown" when it records none.
pub fn kind_of(writes: &[String]) -> &'static str {
    let mut counts = [0usize; KINDS.len()];
    for w in writes {
        let area = area_of(w);
        if let Some(i) = KINDS.iter().position(|k| *k == area) {
            counts[i] += 1;
        }
    }
    let best = counts.iter().copied().max().unwrap_or(0);
    if best == 0 {
        return "unknown";
    }
    // `position` takes the first of equal counts: the tie goes to the earlier kind.
    KINDS[counts.iter().position(|c| *c == best).unwrap_or(0)]
}

/// Estimates against actuals for one kind of task.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KindRatio {
    pub kind: String,
    pub tasks: usize,
    pub estimate_h: f64,
    pub actual_h: f64,
    /// `actual_h / estimate_h`.
    pub ratio: f64,
}

/// Estimate-to-actual per kind, over `project`'s completed tasks with an estimate and a recorded
/// actual (a task completed before the log existed has none, and counting it as 0 h would invent
/// one). `actuals` come from `project`'s task log, keyed by local id, so a task of another project
/// the graph holds is never paired with them, whatever its id. Kinds in the order core, cli,
/// server, web, docs, ci, other, unknown.
pub fn ratios(
    graph: &TaskGraph,
    project: &str,
    actuals: &BTreeMap<String, TaskActual>,
) -> Vec<KindRatio> {
    let mut by_kind: BTreeMap<&'static str, (usize, f64, f64)> = BTreeMap::new();
    for n in graph.nodes.values() {
        if n.id.project != project || n.state != TaskState::Completed {
            continue;
        }
        let Some(estimate) = n.estimate_h.filter(|h| *h > 0.0) else {
            continue;
        };
        let Some(a) = actuals.get(&n.id.local()).filter(|a| a.agent_ms > 0) else {
            continue;
        };
        let e = by_kind.entry(kind_of(&n.writes)).or_insert((0, 0.0, 0.0));
        e.0 += 1;
        e.1 += estimate;
        e.2 += a.agent_ms as f64 / 3_600_000.0;
    }
    let rank = |k: &str| KINDS.iter().position(|x| *x == k).unwrap_or(KINDS.len());
    let mut out: Vec<KindRatio> = by_kind
        .into_iter()
        .map(|(kind, (tasks, estimate_h, actual_h))| KindRatio {
            kind: kind.to_string(),
            tasks,
            estimate_h,
            actual_h,
            ratio: actual_h / estimate_h,
        })
        .collect();
    out.sort_by_key(|r| rank(&r.kind));
    out
}

/// An RFC3339 time, or none when it cannot be read.
fn parse(at: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(at.trim(), &Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskgraph::{TaskId, TaskNode};

    fn ev(at: &str, task: &str, from: TaskState, to: TaskState) -> TaskEvent {
        TaskEvent {
            at: at.into(),
            item: "FTR-1".into(),
            todo: "TL-001".into(),
            task: task.into(),
            from,
            to,
            tokens: None,
            cost_usd: None,
            model: None,
            build: None,
        }
    }

    /// R-4: in-progress intervals are summed, an interval open at `now` counts up to `now`,
    /// reopening a completed task is rework, and reported builds, tokens and cost are summed.
    #[test]
    fn actuals_are_derived_from_recorded_task_events() {
        use TaskState::*;
        let mut done = ev("2026-11-17T12:30:00Z", "T1", InProgress, Completed);
        done.build = Some(BuildTotals {
            runs: 3,
            red: 1,
            run_ms: 600_000,
            wait_ms: 60_000,
        });
        done.tokens = Some(1000);
        done.cost_usd = Some(0.5);
        done.model = Some("sonnet".into());
        let mut redone = ev("2026-11-17T13:15:00Z", "T1", InProgress, Completed);
        redone.build = Some(BuildTotals {
            runs: 1,
            red: 0,
            run_ms: 120_000,
            wait_ms: 0,
        });
        redone.tokens = Some(500);
        // Out of order on purpose: actuals sort by time.
        let events = vec![
            redone,
            ev("2026-11-17T10:00:00Z", "T1", NotStarted, InProgress),
            ev("2026-11-17T13:00:00Z", "T1", Completed, InProgress),
            ev("2026-11-17T11:00:00Z", "T1", InProgress, NotStarted),
            done,
            ev("2026-11-17T12:00:00Z", "T1", NotStarted, InProgress),
            ev("2026-11-17T14:00:00Z", "T2", NotStarted, InProgress),
        ];
        let got = actuals(&events, "2026-11-17T15:30:00Z");
        assert_eq!(got.len(), 2);

        let t1 = &got["FTR-1/TL-001/T1"];
        assert_eq!(t1.id, "FTR-1/TL-001/T1");
        // 10:00–11:00, 12:00–12:30, 13:00–13:15.
        assert_eq!(t1.agent_ms, 6_300_000);
        assert_eq!(t1.build_ms, 720_000);
        assert_eq!(t1.wait_ms, 60_000);
        assert_eq!(t1.red_builds, 1);
        assert_eq!(t1.rework, 1);
        assert_eq!(t1.tokens, Some(1500));
        assert_eq!(t1.cost_usd, Some(0.5));
        assert_eq!(t1.running_since, None);

        let t2 = &got["FTR-1/TL-001/T2"];
        assert_eq!(t2.agent_ms, 5_400_000);
        assert_eq!(t2.running_since.as_deref(), Some("2026-11-17T14:00:00Z"));
        assert_eq!(t2.rework, 0);
        assert_eq!(t2.tokens, None);
        assert_eq!(t2.cost_usd, None);
    }

    fn node(key: &str, state: TaskState, writes: &[&str], estimate_h: Option<f64>) -> TaskNode {
        TaskNode {
            id: TaskId {
                project: "p".into(),
                code: "FTR-1".into(),
                todo: "TL-001".into(),
                key: key.into(),
            },
            text: key.into(),
            state,
            item_status: "In Progress".into(),
            at_build: true,
            item_terminal: false,
            depends_on: Vec::new(),
            implicit: Vec::new(),
            writes: writes.iter().map(|s| s.to_string()).collect(),
            reads: Vec::new(),
            estimate_h,
            scope: None,
        }
    }

    fn actual(key: &str, hours: f64) -> (String, TaskActual) {
        let id = format!("FTR-1/TL-001/{key}");
        (
            id.clone(),
            TaskActual {
                id,
                agent_ms: (hours * 3_600_000.0) as u64,
                ..TaskActual::default()
            },
        )
    }

    /// R-4: the kind of a task comes from its written files, and estimates are compared with
    /// actuals per kind over completed, estimated tasks only.
    #[test]
    fn estimates_are_compared_with_actuals_per_kind_of_task() {
        let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            kind_of(&w(&["api/crates/kanbanr-core/src/tasklog.rs"])),
            "core"
        );
        assert_eq!(kind_of(&w(&["api/crates/kanbanr-cli/src/main.rs"])), "cli");
        assert_eq!(
            kind_of(&w(&["api/crates/kanbanr-server/src/lib.rs"])),
            "server"
        );
        assert_eq!(
            kind_of(&w(&[
                "web/src/a.ts",
                "web/src/b.ts",
                "api/crates/kanbanr-core/src/x.rs"
            ])),
            "web"
        );
        // A tie goes to the earlier kind in the order core, cli, server, web, docs, ci, other.
        assert_eq!(
            kind_of(&w(&[
                "api/crates/kanbanr-cli/src/main.rs",
                "api/crates/kanbanr-core/src/lib.rs"
            ])),
            "core"
        );
        assert_eq!(kind_of(&w(&["docs/guide/reference/cli.md"])), "docs");
        assert_eq!(kind_of(&w(&["skill/kanbanr/SKILL.md"])), "docs");
        assert_eq!(kind_of(&w(&["CHANGELOG.md"])), "docs");
        assert_eq!(kind_of(&w(&[".github/workflows/ci.yml"])), "ci");
        assert_eq!(kind_of(&w(&["scripts/ci_local.py"])), "ci");
        assert_eq!(kind_of(&w(&["Makefile"])), "ci");
        assert_eq!(kind_of(&w(&["api/Cargo.lock"])), "other");
        assert_eq!(kind_of(&[]), "unknown");

        use TaskState::*;
        let core = ["api/crates/kanbanr-core/src/x.rs"];
        let web = ["web/src/x.ts"];
        let mut graph = TaskGraph::default();
        for n in [
            node("T1", Completed, &core, Some(2.0)),
            node("T2", Completed, &core, Some(1.0)),
            node("T3", Completed, &web, Some(2.0)),
            // No estimate: nothing to compare with.
            node("T4", Completed, &core, None),
            // Not completed: its actual is not final.
            node("T5", InProgress, &core, Some(1.0)),
        ] {
            graph.nodes.insert(n.id.to_string(), n);
        }
        let acts: BTreeMap<String, TaskActual> = [
            actual("T1", 3.0),
            actual("T2", 1.0),
            actual("T3", 1.0),
            actual("T4", 5.0),
            actual("T5", 1.0),
        ]
        .into_iter()
        .collect();
        let got = ratios(&graph, "p", &acts);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].kind, "core");
        assert_eq!(got[0].tasks, 2);
        assert!((got[0].estimate_h - 3.0).abs() < 1e-9);
        assert!((got[0].actual_h - 4.0).abs() < 1e-9);
        assert!((got[0].ratio - 4.0 / 3.0).abs() < 1e-9);
        assert_eq!(got[1].kind, "web");
        assert_eq!(got[1].tasks, 1);
        assert!((got[1].estimate_h - 2.0).abs() < 1e-9);
        assert!((got[1].actual_h - 1.0).abs() < 1e-9);
        assert!((got[1].ratio - 0.5).abs() < 1e-9);
    }

    /// R-4: the task log is one project's, keyed by local id, so a task of another project the
    /// graph holds (through a cross-project ref) with the same local id is never paired with it.
    #[test]
    fn a_task_of_another_project_with_the_same_local_id_is_never_paired() {
        use TaskState::*;
        let mut graph = TaskGraph::default();
        let mine = node("T1", Completed, &["web/src/x.ts"], Some(2.0));
        let mut theirs = node(
            "T1",
            Completed,
            &["api/crates/kanbanr-core/src/x.rs"],
            Some(4.0),
        );
        theirs.id.project = "other".into();
        for n in [mine, theirs] {
            graph.nodes.insert(n.id.to_string(), n);
        }
        let acts: BTreeMap<String, TaskActual> = [actual("T1", 1.0)].into_iter().collect();
        let got = ratios(&graph, "p", &acts);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].kind, "web");
        assert_eq!(got[0].tasks, 1);
        assert!((got[0].estimate_h - 2.0).abs() < 1e-9);
    }
}
