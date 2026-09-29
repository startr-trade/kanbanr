//! Gantt rendering (FEAT-035): turn the dependency schedule into a Mermaid `gantt` diagram.
//!
//! This is intentionally separate from `mermaid.rs` (which renders the *workflow* state diagram).
//! Two entry points:
//! - [`project_gantt`] — one project: sections by **milestone**, scoped to that project's nodes.
//! - [`portfolio_gantt`] — cross-project: sections by **project**, spanning the whole portfolio.
//!
//! Tasks are placed two ways, matching what data is available:
//! - When a feature has a `start` (ISO) date, the task is **dated** (`<id>, <start>, <dur>d`).
//! - Otherwise it is **sequenced** off the dependency schedule: a task with in-scope dependencies
//!   is emitted as `after <dep…>, <dur>d` (Mermaid sequences it after those tasks); a root task
//!   gets an explicit `0d`-anchored offset via a synthetic start so output stays deterministic.
//!
//! Critical-path tasks (longest chain) are marked `crit` so they stand out in the rendered chart.

use crate::Store;
use crate::error::Result;
use crate::graph::{DependencyView, Schedule};
use std::collections::BTreeMap;

/// Round a day count for display: whole numbers print without a fraction (`2d`, not `2.0d`).
fn days(d: f64) -> String {
    if (d.round() - d).abs() < f64::EPSILON {
        format!("{}d", d.round() as i64)
    } else {
        format!("{d}d")
    }
}

/// A Mermaid gantt task id must be a single token; sanitize qualified ids (`proj:CODE`) to `proj_CODE`.
fn task_id(qualified: &str) -> String {
    qualified.replace([':', ' ', '-'], "_")
}

/// Escape a task label for the `<label> :` field. Mermaid gantt splits a task line on the first
/// `:` (label vs. metadata) and on `,` (metadata fields), so the label must contain NEITHER —
/// otherwise a qualified id like `proj:CODE` corrupts the whole line. Render it as `proj/CODE`.
fn label(s: &str) -> String {
    s.replace(':', "/").replace(',', " ")
}

/// Format a `time::Date` as `YYYY-MM-DD`.
pub(crate) fn fmt_date(d: time::Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Parse the `YYYY-MM-DD` prefix of an RFC3339 timestamp into a `time::Date` (no `parsing` feature
/// needed — we only need the calendar date, which we build from the first 10 chars).
pub(crate) fn parse_ymd(s: &str) -> Option<time::Date> {
    let mut it = s.get(..10)?.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u8 = it.next()?.parse().ok()?;
    let d: u8 = it.next()?.parse().ok()?;
    time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, d).ok()
}

/// Map a schedule offset (days from project start) to a concrete `YYYY-MM-DD` date. The base is a
/// fixed, arbitrary epoch so the timeline is deterministic — used only when a task has neither a
/// planned date, a dependency, nor parseable real timestamps.
fn day_date(offset_days: f64) -> String {
    let base = time::macros::date!(2025 - 01 - 01);
    let n = offset_days.round().max(0.0) as i64;
    fmt_date(base.checked_add(time::Duration::days(n)).unwrap_or(base))
}

/// Derive a concrete `(start_date, duration)` from a feature's REAL timestamps (`created_at` →
/// `updated_at`) — the actuals fallback so the chart reflects history when there's no planned
/// schedule. Duration is at least 1 day so a same-day item is still visible.
fn actual_span(created_at: &str, updated_at: &str) -> Option<(String, String)> {
    let c = parse_ymd(created_at)?;
    let u = parse_ymd(updated_at).unwrap_or(c);
    let dur = (u - c).whole_days().max(1);
    Some((fmt_date(c), format!("{dur}d")))
}

/// Common gantt header.
fn header(title: &str) -> String {
    format!("gantt\n    title {title}\n    dateFormat YYYY-MM-DD\n    axisFormat %m-%d\n")
}

/// Emit one task line into `out`, using a dated placement when `start` is set, else sequencing it
/// after its in-scope dependencies (or anchoring a root at offset 0). `crit` marks the critical path.
fn emit_task(
    out: &mut String,
    view: &DependencyView,
    sched: &Schedule,
    id: &str,
    in_scope: &dyn Fn(&str) -> bool,
) {
    let node = match view.nodes.get(id) {
        Some(n) => n,
        None => return,
    };
    let tid = task_id(id);
    let dur = sched
        .tasks
        .get(id)
        .map(|t| t.duration)
        .unwrap_or(node.duration);

    // Tag list: `crit` (on the critical path) and `done` (already terminal) where applicable.
    let mut tags: Vec<&str> = Vec::new();
    if sched.is_critical(id) {
        tags.push("crit");
    }
    if node.terminal {
        tags.push("done");
    }
    let tag_prefix = if tags.is_empty() {
        String::new()
    } else {
        format!("{}, ", tags.join(", "))
    };

    // The label shows the human code; the trailing token is the gantt task id.
    let lab = label(&node.id);

    if let Some(start) = node.start.as_deref().filter(|s| !s.is_empty()) {
        // Dated task: explicit start + duration.
        out.push_str(&format!(
            "    {lab} :{tag_prefix}{tid}, {start}, {dur}\n",
            dur = days(dur)
        ));
        return;
    }

    // Sequenced task: depend on in-scope, known predecessors.
    let deps: Vec<String> = node
        .depends_on
        .iter()
        .filter(|d| in_scope(d) && view.nodes.contains_key(d.as_str()))
        .map(|d| task_id(d))
        .collect();
    if deps.is_empty() {
        // No planned date and no in-scope predecessor. Prefer the feature's REAL timeline
        // (created_at -> updated_at) so the chart reflects history instead of collapsing every
        // unplanned task onto one synthetic day; fall back to the schedule offset only if the
        // timestamps don't parse. A concrete start is required either way — a bare `id, duration`
        // makes Mermaid read the id token as the start date ("Invalid date").
        let (start, dur_s) = actual_span(&node.created_at, &node.updated_at).unwrap_or_else(|| {
            let offset = sched.tasks.get(id).map(|t| t.start).unwrap_or(0.0);
            (day_date(offset), days(dur))
        });
        out.push_str(&format!("    {lab} :{tag_prefix}{tid}, {start}, {dur_s}\n"));
    } else {
        out.push_str(&format!(
            "    {lab} :{tag_prefix}{tid}, after {after}, {dur}\n",
            after = deps.join(" "),
            dur = days(dur)
        ));
    }
}

/// Render a single project's schedule as a Mermaid gantt diagram, sectioned by milestone.
pub fn project_gantt(store: &Store, project_id: &str) -> Result<String> {
    let project = store.load(project_id)?;
    let view = DependencyView::build(store, None)?;
    let sched = view.schedule(Some(project_id));
    let in_scope = |id: &str| {
        view.nodes
            .get(id)
            .map(|n| n.project == project_id)
            .unwrap_or(false)
    };

    let title = if project.config.name.is_empty() {
        project_id.to_string()
    } else {
        project.config.name.clone()
    };
    let mut out = header(&title);

    // Group this project's features by milestone, preserving feature order within each milestone.
    let mut by_milestone: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &project.features {
        let id = crate::graph::qualify(project_id, &f.code);
        by_milestone
            .entry(f.milestone.clone())
            .or_default()
            .push(id);
    }

    for (ms, ids) in &by_milestone {
        let section = if ms.is_empty() { "(no milestone)" } else { ms };
        out.push_str(&format!(
            "    section {section}\n",
            section = label(section)
        ));
        for id in ids {
            emit_task(&mut out, &view, &sched, id, &in_scope);
        }
    }
    Ok(out)
}

/// Render the whole portfolio's schedule as a Mermaid gantt diagram, sectioned by project.
pub fn portfolio_gantt(store: &Store) -> Result<String> {
    let view = DependencyView::build(store, None)?;
    let sched = view.schedule(None);
    let in_scope = |_id: &str| true; // portfolio-wide: every node is in scope

    let mut out = header("Portfolio");

    // Group nodes by project (BTreeMap over node ids already gives a stable order within a project).
    let mut by_project: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in view.nodes.values() {
        by_project
            .entry(n.project.clone())
            .or_default()
            .push(n.id.clone());
    }

    for (proj, ids) in &by_project {
        out.push_str(&format!("    section {proj}\n", proj = label(proj)));
        for id in ids {
            emit_task(&mut out, &view, &sched, id, &in_scope);
        }
    }
    Ok(out)
}
