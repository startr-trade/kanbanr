//! Portfolio-wide feature dependency graph (FEAT-026).
//!
//! A feature's `depends_on` entry may be **qualified** as `"<project>:<code>"` to point at a
//! feature in another project; a **bare** `"<code>"` still means "same project". The on-disk model
//! is unchanged (`depends_on` is just `Vec<String>`) — only the *interpretation* and *validation*
//! span projects here. Nodes in the global graph are keyed by their qualified id `"<project>:<code>"`.
//!
//! This module is the resolver the rest of MS-005 builds on (readiness/blocked, impact, etc.).

use crate::error::{CoreError, Result};
use crate::{FeatureItem, Project, Store};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Default scheduling duration (in days) for a feature with no `estimate_days`.
pub const DEFAULT_DURATION_DAYS: f64 = 1.0;

/// A fully-qualified node id, `"<project>:<code>"`.
pub fn qualify(project: &str, code: &str) -> String {
    format!("{project}:{code}")
}

/// Interpret a `depends_on` entry. `"proj:CODE"` → `("proj", "CODE")`; a bare `"CODE"` →
/// `(default_project, "CODE")`. Project names and feature codes never contain `':'`, so the split
/// is unambiguous.
pub fn parse_ref(s: &str, default_project: &str) -> (String, String) {
    match s.split_once(':') {
        Some((p, c)) if !p.is_empty() && !c.is_empty() => (p.to_string(), c.to_string()),
        _ => (default_project.to_string(), s.to_string()),
    }
}

/// The combined cross-project feature graph: each node `"project:code"` maps to its dependency
/// node ids (also qualified). Built from feature *metadata* only (no spec bodies needed).
pub struct FeatureGraph {
    pub edges: BTreeMap<String, Vec<String>>,
}

impl FeatureGraph {
    /// Build the graph across every project in the store. `overlay`, when given, substitutes the
    /// in-memory state of one project (`(id, project)`) for its on-disk state — used to validate a
    /// mutation that hasn't been flushed yet.
    pub fn build(store: &Store, overlay: Option<(&str, &Project)>) -> Result<FeatureGraph> {
        let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut add = |pid: &str, features: &[FeatureItem]| {
            for f in features {
                let deps = f
                    .depends_on
                    .iter()
                    .map(|d| {
                        let (dp, dc) = parse_ref(d, pid);
                        qualify(&dp, &dc)
                    })
                    .collect();
                edges.insert(qualify(pid, &f.code), deps);
            }
        };
        for pid in store.list_projects()? {
            match overlay {
                Some((oid, op)) if oid == pid => add(&pid, &op.features),
                _ => {
                    let project = store.load(&pid)?;
                    add(&pid, &project.features);
                }
            }
        }
        Ok(FeatureGraph { edges })
    }

    /// Cycle check reachable from `start` (the just-changed node). Reports `DependencyCycle(start)`.
    fn assert_acyclic(&self, start: &str) -> Result<()> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }
        fn visit<'a>(
            node: &'a str,
            edges: &'a BTreeMap<String, Vec<String>>,
            marks: &mut BTreeMap<&'a str, Mark>,
            start: &str,
        ) -> Result<()> {
            match marks.get(node) {
                Some(Mark::Done) => return Ok(()),
                Some(Mark::Visiting) => return Err(CoreError::DependencyCycle(start.to_string())),
                None => {}
            }
            marks.insert(node, Mark::Visiting);
            if let Some(deps) = edges.get(node) {
                for dep in deps {
                    if let Some((k, _)) = edges.get_key_value(dep.as_str()) {
                        visit(k, edges, marks, start)?;
                    }
                }
            }
            marks.insert(node, Mark::Done);
            Ok(())
        }
        let mut marks = BTreeMap::new();
        if let Some((k, _)) = self.edges.get_key_value(start) {
            visit(k, &self.edges, &mut marks, start)?;
        }
        Ok(())
    }
}

/// Validate that feature `code` in `project_id` — taken from the in-memory `overlay` project (its
/// post-mutation state) — has cross-project dependencies that all resolve to an existing feature
/// and keep the global graph acyclic. Same-project (bare) deps are also covered, redundantly with
/// the project-local check in `validate::validate_feature_dependencies`.
pub fn validate_feature_deps(
    store: &Store,
    overlay: &Project,
    project_id: &str,
    code: &str,
) -> Result<()> {
    let graph = FeatureGraph::build(store, Some((project_id, overlay)))?;
    let node = qualify(project_id, code);
    if let Some(deps) = graph.edges.get(&node) {
        for dep in deps {
            if dep == &node {
                return Err(CoreError::DependencyCycle(node.clone()));
            }
            if !graph.edges.contains_key(dep) {
                // Qualified ref to a project/code that doesn't exist (the resolved id is shown).
                return Err(CoreError::UnknownDependency(dep.clone()));
            }
        }
    }
    graph.assert_acyclic(&node)
}

// ---- derived dependency state (FEAT-027) ---------------------------------------------------

/// The derived readiness disposition of a feature.
///
/// A feature is **done** when it is itself terminal (status case-insensitively "Completed", or a
/// no-op state in *its own* project's config). A non-terminal feature is **blocked** when any of
/// its DIRECT dependencies is not terminal, and **ready** otherwise (all deps terminal, or no deps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Readiness {
    /// The feature itself is terminal (Completed / no-op); neither ready nor blocked.
    Done,
    /// Every direct dependency is terminal (or there are no dependencies).
    Ready,
    /// At least one direct dependency is not terminal.
    Blocked,
}

/// A node in the richer derived graph: its qualified id, status, terminal-ness, and dependencies.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    /// Qualified id `"project:code"`.
    pub id: String,
    pub project: String,
    pub code: String,
    pub status: String,
    /// True when the status is terminal per the owning project's config (Completed or no-op).
    pub terminal: bool,
    /// Optional planned start date (ISO string), used by Gantt rendering when present.
    pub start: Option<String>,
    /// Scheduling duration in days for this task (from `estimate_days`, defaulting to 1.0).
    pub duration: f64,
    /// Real timestamps (RFC3339), used by Gantt as an *actuals* fallback when a feature has no
    /// planned start/estimate/dependency to schedule from.
    pub created_at: String,
    pub updated_at: String,
    /// Qualified ids of this node's direct dependencies (may include unresolved cross-project ids).
    pub depends_on: Vec<String>,
}

/// A richer, status-aware view over the portfolio graph. Knows each node's status and whether it is
/// terminal (computed per the *owning* project's config, since projects may differ), so it can
/// derive readiness, blocked sets, downstream impact, and graph exports.
pub struct DependencyView {
    /// Nodes keyed by qualified id `"project:code"`.
    pub nodes: BTreeMap<String, GraphNode>,
}

/// Is a status terminal (done) per a project's config? Terminal == an explicitly declared
/// `terminal_states` entry (FEAT-039), OR — as a fallback when none are declared — the built-in
/// heuristic: case-insensitive "Completed" OR a no-op state (an inert disposition). Keeping the
/// fallback means an empty `terminal_states` reproduces the pre-FEAT-039 behavior exactly.
pub fn is_terminal_status(config: &crate::ProjectConfig, status: &str) -> bool {
    config.is_terminal(status)
        || status.eq_ignore_ascii_case("Completed")
        || config.is_no_op(status)
}

impl DependencyView {
    /// Build the status-aware view across every project in the store. `overlay`, when given,
    /// substitutes the in-memory state of one project for its on-disk state (matches
    /// `FeatureGraph::build`).
    pub fn build(store: &Store, overlay: Option<(&str, &Project)>) -> Result<DependencyView> {
        let graph = FeatureGraph::build(store, overlay)?;
        let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();

        let mut add = |pid: &str, project: &Project| {
            for f in &project.features {
                let id = qualify(pid, &f.code);
                let deps = graph.edges.get(&id).cloned().unwrap_or_default();
                nodes.insert(
                    id.clone(),
                    GraphNode {
                        id,
                        project: pid.to_string(),
                        code: f.code.clone(),
                        status: f.status.clone(),
                        terminal: is_terminal_status(&project.config, &f.status),
                        start: f.start.clone(),
                        duration: f
                            .estimate_days
                            .filter(|d| *d > 0.0)
                            .unwrap_or(DEFAULT_DURATION_DAYS),
                        created_at: f.created_at.clone(),
                        updated_at: f.updated_at.clone(),
                        depends_on: deps,
                    },
                );
            }
        };

        for pid in store.list_projects()? {
            match overlay {
                Some((oid, op)) if oid == pid => add(&pid, op),
                _ => {
                    let project = store.load(&pid)?;
                    add(&pid, &project);
                }
            }
        }
        Ok(DependencyView { nodes })
    }

    /// The readiness disposition of a node. A dependency that doesn't resolve to a known node is
    /// treated as non-terminal (conservatively blocking), so dangling refs surface as blocked.
    pub fn readiness(&self, id: &str) -> Option<Readiness> {
        let node = self.nodes.get(id)?;
        if node.terminal {
            return Some(Readiness::Done);
        }
        let blocked = node
            .depends_on
            .iter()
            .any(|dep| self.nodes.get(dep).map(|d| !d.terminal).unwrap_or(true));
        Some(if blocked {
            Readiness::Blocked
        } else {
            Readiness::Ready
        })
    }

    /// Qualified ids whose readiness is `Ready`, optionally restricted to one project.
    pub fn ready(&self, project: Option<&str>) -> Vec<String> {
        self.filter_by_readiness(Readiness::Ready, project)
    }

    /// Qualified ids whose readiness is `Blocked`, optionally restricted to one project.
    pub fn blocked(&self, project: Option<&str>) -> Vec<String> {
        self.filter_by_readiness(Readiness::Blocked, project)
    }

    fn filter_by_readiness(&self, want: Readiness, project: Option<&str>) -> Vec<String> {
        self.nodes
            .values()
            .filter(|n| project.is_none_or(|p| n.project == p))
            .filter(|n| self.readiness(&n.id) == Some(want))
            .map(|n| n.id.clone())
            .collect()
    }

    /// The downstream transitive closure of `id`: every node that depends on `id` directly or
    /// transitively (i.e. everything `id` could unblock). Excludes `id` itself; returned sorted.
    pub fn impact(&self, id: &str) -> Vec<String> {
        // Reverse adjacency: dep -> dependents.
        let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for node in self.nodes.values() {
            for dep in &node.depends_on {
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(node.id.as_str());
            }
        }
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<&str> = dependents.get(id).cloned().unwrap_or_default();
        while let Some(cur) = stack.pop() {
            if out.insert(cur.to_string())
                && let Some(next) = dependents.get(cur)
            {
                stack.extend(next.iter().copied());
            }
        }
        out.into_iter().collect()
    }

    /// JSON-serializable form of the graph (optionally scoped to one project): nodes carry status +
    /// terminal-ness + readiness; edges are directed `from -> to` (depender -> dependency) pairs.
    pub fn to_json_value(&self, project: Option<&str>) -> serde_json::Value {
        let in_scope = |id: &str| {
            project.is_none_or(|p| self.nodes.get(id).map(|n| n.project == p).unwrap_or(false))
        };

        let nodes: Vec<_> = self
            .nodes
            .values()
            .filter(|n| project.is_none_or(|p| n.project == p))
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "project": n.project,
                    "code": n.code,
                    "status": n.status,
                    "terminal": n.terminal,
                    "readiness": self.readiness(&n.id),
                    "depends_on": n.depends_on,
                })
            })
            .collect();

        let mut edges = Vec::new();
        for n in self.nodes.values() {
            if !in_scope(&n.id) {
                continue;
            }
            for dep in &n.depends_on {
                // When scoped to a project, keep only intra-scope edges.
                if project.is_some() && !in_scope(dep) {
                    continue;
                }
                edges.push(serde_json::json!({ "from": n.id, "to": dep }));
            }
        }
        serde_json::json!({ "nodes": nodes, "edges": edges })
    }

    /// Graphviz DOT form of the graph (optionally scoped to one project). Nodes are colored by
    /// readiness (done / ready / blocked); edges point depender -> dependency.
    pub fn to_dot(&self, project: Option<&str>) -> String {
        let in_scope = |id: &str| {
            project.is_none_or(|p| self.nodes.get(id).map(|n| n.project == p).unwrap_or(false))
        };
        let esc = |s: &str| s.replace('"', "\\\"");

        let mut out =
            String::from("digraph kanbanr {\n  rankdir=LR;\n  node [shape=box, style=rounded];\n");
        for n in self.nodes.values() {
            if !in_scope(&n.id) {
                continue;
            }
            let color = match self.readiness(&n.id) {
                Some(Readiness::Done) => "gray",
                Some(Readiness::Ready) => "green",
                Some(Readiness::Blocked) => "red",
                None => "black",
            };
            out.push_str(&format!(
                "  \"{}\" [label=\"{}\\n{}\", color={}];\n",
                esc(&n.id),
                esc(&n.id),
                esc(&n.status),
                color
            ));
        }
        for n in self.nodes.values() {
            if !in_scope(&n.id) {
                continue;
            }
            for dep in &n.depends_on {
                if project.is_some() && !in_scope(dep) {
                    continue;
                }
                out.push_str(&format!("  \"{}\" -> \"{}\";\n", esc(&n.id), esc(dep)));
            }
        }
        out.push_str("}\n");
        out
    }

    // ---- critical path & scheduling (FEAT-035) --------------------------------------------

    /// Compute a longest-path schedule over the dependency DAG, optionally scoped to one project.
    ///
    /// Each node's `start` offset is the maximum `finish` over its in-scope dependencies (0 when it
    /// has none), and `finish = start + duration`. Durations come from `estimate_days` (default
    /// `DEFAULT_DURATION_DAYS`). The longest finish identifies the project's makespan, and the chain
    /// of dependencies realising it is the **critical path**. Cross-project dependencies are honored
    /// when unscoped (`project = None`); when scoped, only intra-scope edges constrain the schedule.
    pub fn schedule(&self, project: Option<&str>) -> Schedule {
        let in_scope = |id: &str| {
            project.is_none_or(|p| self.nodes.get(id).map(|n| n.project == p).unwrap_or(false))
        };

        // Nodes in scope, in a deterministic (sorted) order.
        let ids: Vec<&str> = self
            .nodes
            .values()
            .filter(|n| in_scope(&n.id))
            .map(|n| n.id.as_str())
            .collect();

        let mut start: BTreeMap<String, f64> = BTreeMap::new();
        let mut finish: BTreeMap<String, f64> = BTreeMap::new();
        // The in-scope dependency that determined a node's start (its critical predecessor), if any.
        let mut pred: BTreeMap<String, Option<String>> = BTreeMap::new();

        // Resolve each node via memoized DFS over in-scope dependencies (DAG: validated acyclic).
        fn resolve(
            id: &str,
            view: &DependencyView,
            scoped: &dyn Fn(&str) -> bool,
            start: &mut BTreeMap<String, f64>,
            finish: &mut BTreeMap<String, f64>,
            pred: &mut BTreeMap<String, Option<String>>,
        ) -> f64 {
            if let Some(f) = finish.get(id) {
                return *f;
            }
            let node = match view.nodes.get(id) {
                Some(n) => n,
                None => return 0.0,
            };
            // Provisional insert guards against pathological cycles (should not occur post-validation).
            finish.insert(id.to_string(), 0.0);
            let mut best_start = 0.0_f64;
            let mut best_pred: Option<String> = None;
            for dep in &node.depends_on {
                if !scoped(dep) || !view.nodes.contains_key(dep.as_str()) {
                    continue;
                }
                let dep_finish = resolve(dep, view, scoped, start, finish, pred);
                if dep_finish > best_start {
                    best_start = dep_finish;
                    best_pred = Some(dep.clone());
                }
            }
            start.insert(id.to_string(), best_start);
            pred.insert(id.to_string(), best_pred);
            let f = best_start + node.duration;
            finish.insert(id.to_string(), f);
            f
        }

        for id in &ids {
            resolve(id, self, &in_scope, &mut start, &mut finish, &mut pred);
        }

        // The makespan endpoint is the in-scope node with the greatest finish (ties: first sorted).
        let endpoint = ids
            .iter()
            .copied()
            .max_by(|a, b| {
                let fa = finish.get(*a).copied().unwrap_or(0.0);
                let fb = finish.get(*b).copied().unwrap_or(0.0);
                fa.partial_cmp(&fb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.cmp(a)) // earlier-sorted id wins ties
            })
            .map(String::from);

        // Walk predecessors back from the endpoint to recover the critical path (start -> end).
        let mut critical = Vec::new();
        let mut cur = endpoint.clone();
        while let Some(id) = cur {
            critical.push(id.clone());
            cur = pred.get(&id).cloned().flatten();
        }
        critical.reverse();

        let makespan = endpoint
            .as_ref()
            .and_then(|e| finish.get(e).copied())
            .unwrap_or(0.0);

        let tasks = ids
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    ScheduledTask {
                        id: id.to_string(),
                        start: start.get(*id).copied().unwrap_or(0.0),
                        finish: finish.get(*id).copied().unwrap_or(0.0),
                        duration: self.nodes.get(*id).map(|n| n.duration).unwrap_or(0.0),
                    },
                )
            })
            .collect();

        Schedule {
            tasks,
            critical_path: critical.clone(),
            critical: critical.into_iter().collect(),
            makespan,
        }
    }

    /// The critical path (longest dependency chain) over the DAG, optionally scoped to one project.
    /// Returned start -> end as qualified ids.
    pub fn critical_path(&self, project: Option<&str>) -> Vec<String> {
        self.schedule(project).critical_path
    }
}

/// One scheduled task: its longest-path `start`/`finish` offsets (in days, from time 0) and duration.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTask {
    pub id: String,
    pub start: f64,
    pub finish: f64,
    pub duration: f64,
}

/// A computed longest-path schedule over the dependency DAG (FEAT-035).
#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    /// Per-node offsets, keyed by qualified id.
    pub tasks: BTreeMap<String, ScheduledTask>,
    /// The critical path as an ordered list of qualified ids (start -> end).
    pub critical_path: Vec<String>,
    /// The set of critical-path ids (for O(1) membership tests, e.g. when marking Gantt tasks).
    #[serde(skip)]
    pub critical: BTreeSet<String>,
    /// Total project length in days (the greatest finish offset).
    pub makespan: f64,
}

impl Schedule {
    /// Is `id` on the critical path?
    pub fn is_critical(&self, id: &str) -> bool {
        self.critical.contains(id)
    }
    /// JSON-serializable view: ordered critical path, makespan, and per-node offsets.
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "critical_path": self.critical_path,
            "makespan": self.makespan,
            "tasks": self.tasks,
        })
    }
}
