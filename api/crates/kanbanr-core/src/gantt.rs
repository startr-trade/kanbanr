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

use crate::error::Result;
use crate::graph::{DependencyView, Schedule};
use crate::Store;
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
        // A root with no scheduling anchor: place it at its computed start offset (in days from 0)
        // using Mermaid's relative form is not available without a date, so anchor explicitly.
        out.push_str(&format!(
            "    {lab} :{tag_prefix}{tid}, {dur}\n",
            dur = days(dur)
        ));
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
