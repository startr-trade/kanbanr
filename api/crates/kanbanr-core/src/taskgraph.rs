//! The task graph (FTR-060): every task of a project's items, with the tasks it waits on, the
//! files it writes and reads, and its estimate in agent-hours.
//!
//! Derived on every read, never stored (ADR-0002): the edges come from each task's `depends_on`
//! and, where an item depends on another and none of its tasks names one of the other's, from the
//! item dependency (every task waits on every task of the other item). The scope working a task
//! comes from `scopes.yaml`. From the graph follow the tasks free to start now, why the others
//! wait, the critical path by estimate, and a set of tasks that can start together without
//! writing the same files.

use crate::config::Role;
use crate::doctor::{Issue, Severity};
use crate::error::{CoreError, Result};
use crate::graph::{is_live_work, is_terminal_status, parse_ref};
use crate::models::TaskState;
use crate::{Project, Store};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;

/// The weight of a task with no estimate, so the critical path stays defined.
pub const DEFAULT_ESTIMATE_H: f64 = 1.0;

/// A task, named across the portfolio.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId {
    pub project: String,
    pub code: String,
    pub todo: String,
    pub key: String,
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}/{}/{}",
            self.project, self.code, self.todo, self.key
        )
    }
}

impl TaskId {
    /// `CODE/TL-001/T3`: the task named within its project.
    pub fn local(&self) -> String {
        format!("{}/{}/{}", self.code, self.todo, self.key)
    }

    /// The task as `project` names it: [`TaskId::local`] there, qualified anywhere else.
    fn label(&self, project: &str) -> String {
        if self.project == project {
            self.local()
        } else {
            self.to_string()
        }
    }
}

fn is_todo_code(s: &str) -> bool {
    s.starts_with("TL-") && s.len() > 3
}

fn is_key(s: &str) -> bool {
    !s.is_empty() && !s.contains([':', '/', ' ']) && !is_todo_code(s)
}

/// Read a task reference written on a task of item `code`, list `todo`, in `project`:
/// `(project, code, todo, key)`. `T3` names this item and list (no list when `todo` is empty, or
/// for the `CODE/?/T3` form a Design's task list writes); `TL-002/T3` this item; `CODE/TL-001/T3`
/// and `project:CODE/TL-001/T3` as named. `CODE/T3` is refused: in the `Refs:` grammar it names
/// a requirement.
pub fn parse_task_ref(
    r: &str,
    project: &str,
    code: &str,
    todo: &str,
) -> Option<(String, String, Option<String>, String)> {
    let r = r.trim();
    let parts: Vec<&str> = r.split('/').collect();
    let own_todo = || (!todo.is_empty()).then(|| todo.to_string());
    match parts.as_slice() {
        [key] if is_key(key) => Some((project.into(), code.into(), own_todo(), key.to_string())),
        [list, key] if is_todo_code(list) && !list.contains(':') && is_key(key) => Some((
            project.into(),
            code.into(),
            Some(list.to_string()),
            key.to_string(),
        )),
        [item, list, key] if (is_todo_code(list) || *list == "?") && is_key(key) => {
            let (p, c) = match item.split_once(':') {
                Some((p, c)) if !p.is_empty() && !c.is_empty() => (p.to_string(), c.to_string()),
                Some(_) => return None,
                None if !item.is_empty() => (project.to_string(), item.to_string()),
                None => return None,
            };
            if c.contains(['/', ' ']) {
                return None;
            }
            let list = (*list != "?").then(|| list.to_string());
            Some((p, c, list, key.to_string()))
        }
        _ => None,
    }
}

/// One task in the graph.
#[derive(Debug, Clone)]
pub struct TaskNode {
    pub id: TaskId,
    pub text: String,
    pub state: TaskState,
    /// The item's status.
    pub item_status: String,
    /// The item is at its build stage (or is live work, where the workflow has no build stage).
    pub at_build: bool,
    /// The item is terminal: every one of its tasks counts as Completed.
    pub item_terminal: bool,
    /// The tasks it names in `depends_on`, resolved; one that no longer exists stays here, and
    /// keeps the task waiting.
    pub depends_on: Vec<TaskId>,
    /// The tasks it waits on through an item dependency no task edge refines.
    pub implicit: Vec<TaskId>,
    pub writes: Vec<String>,
    pub reads: Vec<String>,
    pub estimate_h: Option<f64>,
    /// The live scope working it, from `scopes.yaml`.
    pub scope: Option<String>,
}

impl TaskNode {
    /// Completed, or its item is terminal.
    fn done(&self) -> bool {
        self.state == TaskState::Completed || self.item_terminal
    }

    fn weight(&self) -> f64 {
        if self.done() {
            0.0
        } else {
            self.estimate_h
                .filter(|h| *h > 0.0)
                .unwrap_or(DEFAULT_ESTIMATE_H)
        }
    }

    fn edges(&self) -> impl Iterator<Item = &TaskId> {
        self.depends_on.iter().chain(self.implicit.iter())
    }
}

/// The task graph, nodes keyed by [`TaskId`]'s `Display` (`project:CODE/TL-001/T3`).
#[derive(Debug, Clone, Default)]
pub struct TaskGraph {
    pub nodes: BTreeMap<String, TaskNode>,
}

/// Two path lists' overlapping paths, sorted: two paths overlap when equal, or when one ends in
/// `/` and the other starts with it; the more specific of the two is named. A path that is, or
/// lies in, one of `append_only` never conflicts.
pub fn overlaps(a: &[String], b: &[String], append_only: &[String]) -> Vec<String> {
    fn covers(folder_or_file: &str, path: &str) -> bool {
        folder_or_file == path
            || (folder_or_file.ends_with('/') && path.starts_with(folder_or_file))
    }
    let mut out = BTreeSet::new();
    for x in a {
        for y in b {
            let specific = if covers(x, y) {
                y
            } else if covers(y, x) {
                x
            } else {
                continue;
            };
            if append_only.iter().any(|f| covers(f, specific)) {
                continue;
            }
            out.insert(specific.clone());
        }
    }
    out.into_iter().collect()
}

/// Why an open task cannot start now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Wait {
    /// A task it waits on is not Completed.
    Dependency(String),
    /// A task in progress writes some of the same files (none named: that task's files are not
    /// recorded, and it is in the same item).
    Conflict { task: String, paths: Vec<String> },
    /// Its item is not at the build stage: the item's status.
    ItemNotAtBuild(String),
    /// It records no files, and this task of its item is in progress.
    UnknownFootprint(String),
}

/// A task that is running or free to start.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReadyTask {
    pub id: String,
    pub text: String,
    pub estimate_h: Option<f64>,
    /// On the project's critical path.
    pub critical: bool,
    /// The longest remaining chain starting at this task, in agent-hours.
    pub chain_h: f64,
}

/// A task that cannot start yet, and every reason why.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WaitingTask {
    pub id: String,
    pub waits: Vec<Wait>,
}

/// What can run now in a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TaskReadiness {
    pub running: Vec<ReadyTask>,
    pub ready: Vec<ReadyTask>,
    pub waiting: Vec<WaitingTask>,
    /// Ready tasks that can start together, critical path first (empty unless lanes were asked).
    pub lanes: Vec<ReadyTask>,
}

/// The status of an item and the facts the graph needs about it.
struct ItemFacts {
    status: String,
    at_build: bool,
    terminal: bool,
}

fn item_facts(config: &crate::ProjectConfig, status: &str) -> ItemFacts {
    let at_build = match config.role_status(Role::Build) {
        Some(build) => build == status,
        None => is_live_work(config, status),
    };
    ItemFacts {
        status: status.to_string(),
        at_build,
        terminal: is_terminal_status(config, status),
    }
}

impl TaskGraph {
    /// Every task of every item of `project` — `overlay` standing in for its state on disk, for a
    /// write not yet flushed — and of every project its refs name. Refs are resolved; a ref to a
    /// task that no longer exists stays an edge (it keeps its task waiting; [`issues`] names it).
    /// Errors: `UnknownDependency` for a ref that cannot be read, `DependencyCycle` with the cycle
    /// as `A -> B -> A`.
    pub fn build(store: &Store, project: &str, overlay: Option<&Project>) -> Result<TaskGraph> {
        let mut loaded: BTreeMap<String, Cow<'_, Project>> = BTreeMap::new();
        let mut queue: VecDeque<String> = VecDeque::from([project.to_string()]);
        while let Some(pid) = queue.pop_front() {
            if loaded.contains_key(&pid) {
                continue;
            }
            let p = match overlay {
                Some(o) if pid == project => Cow::Borrowed(o),
                _ if pid == project => Cow::Owned(store.load_meta(&pid)?),
                _ => match store.load_meta(&pid) {
                    Ok(p) => Cow::Owned(p),
                    // A project named by a ref that is gone: its tasks are missing, which the
                    // refs into it show.
                    Err(_) => continue,
                },
            };
            for f in &p.features {
                for d in &f.depends_on {
                    let (dp, _) = parse_ref(d, &pid);
                    if !loaded.contains_key(&dp) {
                        queue.push_back(dp);
                    }
                }
                for l in &f.todo_lists {
                    for t in &l.tasks {
                        for r in &t.depends_on {
                            if let Some((dp, ..)) = parse_task_ref(r, &pid, &f.code, &l.code)
                                && !loaded.contains_key(&dp)
                            {
                                queue.push_back(dp);
                            }
                        }
                    }
                }
            }
            loaded.insert(pid, p);
        }

        let scopes = live_task_scopes(store, loaded.keys());
        let mut nodes: BTreeMap<String, TaskNode> = BTreeMap::new();
        // Item -> its tasks, and item -> the items it depends on, both qualified.
        let mut tasks_of: BTreeMap<String, Vec<(String, TaskId)>> = BTreeMap::new();
        // Item -> the items its tasks name explicitly.
        let mut named: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut item_deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (pid, p) in &loaded {
            for f in &p.features {
                let facts = item_facts(&p.config, &f.status);
                let item = crate::graph::qualify(pid, &f.code);
                item_deps.insert(
                    item.clone(),
                    f.depends_on
                        .iter()
                        .map(|d| {
                            let (dp, dc) = parse_ref(d, pid);
                            crate::graph::qualify(&dp, &dc)
                        })
                        .collect(),
                );
                let into = named.entry(item.clone()).or_default();
                let ids = tasks_of.entry(item).or_default();
                for l in &f.todo_lists {
                    for t in &l.tasks {
                        let id = TaskId {
                            project: pid.clone(),
                            code: f.code.clone(),
                            todo: l.code.clone(),
                            key: t.key.clone(),
                        };
                        let mut deps: Vec<TaskId> = Vec::new();
                        for r in &t.depends_on {
                            let Some((dp, dc, dl, dk)) = parse_task_ref(r, pid, &f.code, &l.code)
                            else {
                                return Err(CoreError::UnknownDependency(format!(
                                    "{}: {r}",
                                    id.label(project)
                                )));
                            };
                            let dep = TaskId {
                                project: dp,
                                code: dc,
                                todo: dl.unwrap_or_default(),
                                key: dk,
                            };
                            if !deps.contains(&dep) {
                                if dep.code != f.code || dep.project != *pid {
                                    into.insert(crate::graph::qualify(&dep.project, &dep.code));
                                }
                                deps.push(dep);
                            }
                        }
                        let key = id.to_string();
                        ids.push((key.clone(), id.clone()));
                        nodes.insert(
                            key.clone(),
                            TaskNode {
                                scope: scopes.get(&key).cloned(),
                                id,
                                text: t.text.clone(),
                                state: t.state,
                                item_status: facts.status.clone(),
                                at_build: facts.at_build,
                                item_terminal: facts.terminal,
                                depends_on: deps,
                                implicit: Vec::new(),
                                writes: t.writes.clone(),
                                reads: t.reads.clone(),
                                estimate_h: t.estimate_h,
                            },
                        );
                    }
                }
            }
        }

        // The item dependency, where no task edge refines it.
        for (item, deps) in &item_deps {
            let Some(mine) = tasks_of.get(item).filter(|m| !m.is_empty()) else {
                continue;
            };
            let refined = named.get(item);
            let mut extra: Vec<TaskId> = Vec::new();
            for dep in deps {
                if refined.is_some_and(|r| r.contains(dep)) {
                    continue;
                }
                for (_, t) in tasks_of.get(dep).into_iter().flatten() {
                    if !extra.contains(t) {
                        extra.push(t.clone());
                    }
                }
            }
            if !extra.is_empty() {
                for (k, _) in mine {
                    if let Some(n) = nodes.get_mut(k) {
                        n.implicit = extra.clone();
                    }
                }
            }
        }

        let graph = TaskGraph { nodes };
        graph.assert_acyclic(project)?;
        Ok(graph)
    }

    /// One task's refs all resolve, and no cycle runs through it: the check a write makes.
    pub fn validate_task(&self, id: &TaskId) -> Result<()> {
        let project = id.project.as_str();
        let node = self.nodes.get(&id.to_string()).ok_or_else(|| {
            CoreError::TaskNotFound(id.key.clone(), format!("{}/{}", id.code, id.todo))
        })?;
        for d in &node.depends_on {
            if !self.nodes.contains_key(&d.to_string()) {
                return Err(CoreError::UnknownDependency(d.label(project)));
            }
        }
        if let Some(cycle) = self.cycle_from(&id.to_string()) {
            return Err(CoreError::DependencyCycle(self.cycle_text(&cycle, project)));
        }
        Ok(())
    }

    fn cycle_text(&self, cycle: &[String], project: &str) -> String {
        cycle
            .iter()
            .map(|k| {
                self.nodes
                    .get(k)
                    .map(|n| n.id.label(project))
                    .unwrap_or_else(|| k.clone())
            })
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    fn assert_acyclic(&self, project: &str) -> Result<()> {
        let ix = Ix::new(self);
        // 0 unvisited, 1 on the path, 2 done.
        let mut mark = vec![0u8; ix.nodes.len()];
        for start in 0..ix.nodes.len() {
            if mark[start] != 0 {
                continue;
            }
            // Iterative depth-first walk: (node, next edge to try).
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            mark[start] = 1;
            while let Some((n, e)) = stack.last().copied() {
                if let Some(&d) = ix.out[n].get(e) {
                    stack.last_mut().expect("non-empty").1 += 1;
                    match mark[d] {
                        0 => {
                            mark[d] = 1;
                            stack.push((d, 0));
                        }
                        1 => {
                            let at = stack.iter().position(|(s, _)| *s == d).unwrap_or(0);
                            let mut cycle: Vec<String> = stack[at..]
                                .iter()
                                .map(|(s, _)| ix.keys[*s].clone())
                                .collect();
                            cycle.push(ix.keys[d].clone());
                            return Err(CoreError::DependencyCycle(
                                self.cycle_text(&cycle, project),
                            ));
                        }
                        _ => {}
                    }
                } else {
                    mark[n] = 2;
                    stack.pop();
                }
            }
        }
        Ok(())
    }

    /// A cycle through `start` (a node key), as the path `start -> … -> start`.
    fn cycle_from(&self, start: &str) -> Option<Vec<String>> {
        let ix = Ix::new(self);
        let s = ix.keys.iter().position(|k| *k == start)?;
        let mut seen = vec![false; ix.nodes.len()];
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        while let Some((n, e)) = stack.last().copied() {
            if let Some(&d) = ix.out[n].get(e) {
                stack.last_mut().expect("non-empty").1 += 1;
                if d == s {
                    let mut path: Vec<String> =
                        stack.iter().map(|(x, _)| ix.keys[*x].clone()).collect();
                    path.push(ix.keys[s].clone());
                    return Some(path);
                }
                if !seen[d] {
                    seen[d] = true;
                    stack.push((d, 0));
                }
            } else {
                stack.pop();
            }
        }
        None
    }

    /// The longest remaining chain starting at `id` (a node key, `project:CODE/TL-001/T3`), the
    /// task's own weight included: its estimate, 1.0 h without one, 0 once done.
    pub fn chain_h(&self, id: &str) -> f64 {
        let ix = Ix::new(self);
        let chains = ix.chains();
        ix.keys
            .iter()
            .position(|k| *k == id)
            .map(|i| chains[i])
            .unwrap_or(0.0)
    }

    /// The longest chain of `project`'s open tasks by estimate, start first, as the project names
    /// them, and its length in hours. Ties go to the smaller id.
    pub fn critical_path(&self, project: &str) -> (Vec<String>, f64) {
        let ix = Ix::new(self);
        let chains = ix.chains();
        let (path, len) = ix.critical(project, &chains);
        (
            path.iter()
                .map(|i| ix.nodes[*i].id.label(project))
                .collect(),
            len,
        )
    }

    /// What can run now in `project`: the tasks in progress, the tasks free to start, and why the
    /// rest wait; with `lanes`, up to that many ready tasks that can start together. Tasks of a
    /// terminal item count as done and are not listed.
    pub fn readiness(
        &self,
        project: &str,
        append_only: &[String],
        lanes: Option<usize>,
    ) -> TaskReadiness {
        let ix = Ix::new(self);
        let chains = ix.chains();
        let critical: BTreeSet<usize> = ix.critical(project, &chains).0.into_iter().collect();
        let labels: Vec<String> = ix.nodes.iter().map(|n| n.id.label(project)).collect();
        let entry = |i: usize| {
            let n = ix.nodes[i];
            ReadyTask {
                id: labels[i].clone(),
                text: n.text.clone(),
                estimate_h: n.estimate_h,
                critical: critical.contains(&i),
                chain_h: chains[i],
            }
        };
        let mine: Vec<usize> = (0..ix.nodes.len())
            .filter(|i| ix.nodes[*i].id.project == project && !ix.nodes[*i].item_terminal)
            .collect();
        let running: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|i| ix.nodes[*i].state == TaskState::InProgress)
            .collect();
        let index = WriteIndex::new(running.iter().map(|i| ix.nodes[*i]));
        let mut running_by_item: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (r, i) in running.iter().enumerate() {
            running_by_item
                .entry(ix.nodes[*i].id.code.as_str())
                .or_default()
                .push(r);
        }

        let mut out = TaskReadiness::default();
        let mut ready: Vec<usize> = Vec::new();
        for &i in &mine {
            let n = ix.nodes[i];
            if n.state != TaskState::NotStarted {
                continue;
            }
            let mut waits = Vec::new();
            if !n.at_build {
                waits.push(Wait::ItemNotAtBuild(n.item_status.clone()));
            }
            for d in n.edges() {
                let found = ix.pos.get(d).copied();
                if !found.is_some_and(|j| ix.nodes[j].done()) {
                    let w = Wait::Dependency(match found {
                        Some(j) => labels[j].clone(),
                        None => d.label(project),
                    });
                    if !waits.contains(&w) {
                        waits.push(w);
                    }
                }
            }
            let siblings = running_by_item.get(n.id.code.as_str());
            let mut clashes: BTreeSet<usize> = index.hits(&n.writes);
            for r in siblings.into_iter().flatten() {
                if !ix.nodes[running[*r]].has_footprint() {
                    clashes.insert(*r);
                }
            }
            for r in clashes {
                let r = ix.nodes[running[r]];
                if !r.has_footprint() {
                    waits.push(Wait::Conflict {
                        task: r.id.label(project),
                        paths: Vec::new(),
                    });
                    continue;
                }
                let paths = overlaps(&r.writes, &n.writes, append_only);
                if !paths.is_empty() {
                    waits.push(Wait::Conflict {
                        task: r.id.label(project),
                        paths,
                    });
                }
            }
            if !n.has_footprint() {
                for r in siblings.into_iter().flatten() {
                    waits.push(Wait::UnknownFootprint(
                        ix.nodes[running[*r]].id.label(project),
                    ));
                }
            }
            if waits.is_empty() {
                ready.push(i);
            } else {
                out.waiting.push(WaitingTask {
                    id: n.id.label(project),
                    waits,
                });
            }
        }
        out.running = running.iter().map(|i| entry(*i)).collect();
        let order = |a: &ReadyTask, b: &ReadyTask| {
            b.critical
                .cmp(&a.critical)
                .then(
                    b.chain_h
                        .partial_cmp(&a.chain_h)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then_with(|| a.id.cmp(&b.id))
        };
        let mut ranked: Vec<(ReadyTask, &TaskNode)> =
            ready.iter().map(|i| (entry(*i), ix.nodes[*i])).collect();
        ranked.sort_by(|a, b| order(&a.0, &b.0));

        if let Some(n) = lanes {
            let mut taken: Vec<&TaskNode> = Vec::new();
            let mut unknown_items: BTreeSet<String> = BTreeSet::new();
            for (t, node) in &ranked {
                if out.lanes.len() >= n {
                    break;
                }
                if !node.has_footprint() {
                    let item = format!("{}:{}", node.id.project, node.id.code);
                    if unknown_items.contains(&item) {
                        continue;
                    }
                    unknown_items.insert(item);
                } else {
                    let clash = index
                        .hits(&node.writes)
                        .into_iter()
                        .map(|r| ix.nodes[running[r]])
                        .chain(taken.iter().copied())
                        .any(|o| !overlaps(&o.writes, &node.writes, append_only).is_empty());
                    if clash {
                        continue;
                    }
                }
                taken.push(node);
                out.lanes.push(t.clone());
            }
        }
        out.ready = ranked.into_iter().map(|(t, _)| t).collect();
        out
    }
}

/// The graph by position, for the walks: node `i` is the `i`th of `nodes` in key order, so a
/// smaller position is a smaller id.
struct Ix<'a> {
    keys: Vec<&'a String>,
    nodes: Vec<&'a TaskNode>,
    pos: HashMap<&'a TaskId, usize>,
    /// The tasks each waits on that exist.
    out: Vec<Vec<usize>>,
    /// The tasks waiting on each.
    rev: Vec<Vec<usize>>,
}

impl<'a> Ix<'a> {
    fn new(g: &'a TaskGraph) -> Ix<'a> {
        let keys: Vec<&String> = g.nodes.keys().collect();
        let nodes: Vec<&TaskNode> = g.nodes.values().collect();
        let pos: HashMap<&TaskId, usize> =
            nodes.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
        let mut out = vec![Vec::new(); nodes.len()];
        let mut rev = vec![Vec::new(); nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for d in n.edges() {
                if let Some(&j) = pos.get(d)
                    && !out[i].contains(&j)
                {
                    out[i].push(j);
                    rev[j].push(i);
                }
            }
            out[i].sort_unstable();
        }
        for r in &mut rev {
            r.sort_unstable();
        }
        Ix {
            keys,
            nodes,
            pos,
            out,
            rev,
        }
    }

    /// Each task's longest remaining chain: its weight plus the longest of those waiting on it.
    fn chains(&self) -> Vec<f64> {
        let n = self.nodes.len();
        let mut memo: Vec<Option<f64>> = vec![None; n];
        // On the walk's path: a guard against a cycle, which build refuses.
        let mut on_path = vec![false; n];
        for start in 0..n {
            if memo[start].is_some() {
                continue;
            }
            // Post-order walk over the dependents; a node is valued once they all are.
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            on_path[start] = true;
            while let Some((v, e)) = stack.last().copied() {
                if let Some(&d) = self.rev[v].get(e) {
                    stack.last_mut().expect("non-empty").1 += 1;
                    if memo[d].is_none() && !on_path[d] {
                        on_path[d] = true;
                        stack.push((d, 0));
                    }
                } else {
                    on_path[v] = false;
                    let best = self.rev[v]
                        .iter()
                        .filter_map(|d| memo[*d])
                        .fold(0.0_f64, f64::max);
                    memo[v] = Some(self.nodes[v].weight() + best);
                    stack.pop();
                }
            }
        }
        memo.into_iter().map(|v| v.unwrap_or(0.0)).collect()
    }

    fn is_open_in(&self, i: usize, project: &str) -> bool {
        let n = self.nodes[i];
        n.id.project == project && !n.done()
    }

    /// The critical path's positions, start first, and its length.
    fn critical(&self, project: &str, chains: &[f64]) -> (Vec<usize>, f64) {
        // The largest chain; ties to the smaller position.
        let best = |cands: &mut dyn Iterator<Item = usize>| -> Option<usize> {
            let mut pick: Option<usize> = None;
            for i in cands {
                if pick.is_none_or(|p| chains[i] > chains[p]) {
                    pick = Some(i);
                }
            }
            pick
        };
        let Some(start) = best(&mut (0..self.nodes.len()).filter(|i| self.is_open_in(*i, project)))
        else {
            return (Vec::new(), 0.0);
        };
        let mut path = vec![start];
        let mut cur = start;
        while let Some(next) = best(
            &mut self.rev[cur]
                .iter()
                .copied()
                .filter(|i| self.is_open_in(*i, project) && !path.contains(i)),
        ) {
            path.push(next);
            cur = next;
        }
        (path, chains[start])
    }
}

impl TaskNode {
    fn has_footprint(&self) -> bool {
        !self.writes.is_empty()
    }
}

/// The tasks' written paths, indexed so a task's possible overlaps are found without comparing
/// it with every other task.
struct WriteIndex<'a> {
    by_path: BTreeMap<&'a str, Vec<usize>>,
}

impl<'a> WriteIndex<'a> {
    fn new(tasks: impl Iterator<Item = &'a TaskNode>) -> WriteIndex<'a> {
        let mut by_path: BTreeMap<&'a str, Vec<usize>> = BTreeMap::new();
        for (i, t) in tasks.enumerate() {
            for w in &t.writes {
                by_path.entry(w.as_str()).or_default().push(i);
            }
        }
        WriteIndex { by_path }
    }

    /// The tasks with a written path equal to one of `writes`, a folder holding one, or inside
    /// one of `writes`' folders: the candidates [`overlaps`] then decides on.
    fn hits(&self, writes: &[String]) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for p in writes {
            if let Some(v) = self.by_path.get(p.as_str()) {
                out.extend(v.iter().copied());
            }
            for (i, c) in p.char_indices() {
                if c == '/'
                    && i + 1 < p.len()
                    && let Some(v) = self.by_path.get(&p[..=i])
                {
                    out.extend(v.iter().copied());
                }
            }
            if p.ends_with('/') {
                for (k, v) in self.by_path.range(p.as_str()..) {
                    if !k.starts_with(p.as_str()) {
                        break;
                    }
                    out.extend(v.iter().copied());
                }
            }
        }
        out
    }
}

/// The live scopes working a task: task node key -> scope id. A scope names its task as
/// `TL-001/T3`.
fn live_task_scopes<'a>(
    store: &Store,
    projects: impl Iterator<Item = &'a String>,
) -> BTreeMap<String, String> {
    let now = crate::now_rfc3339();
    let mut out = BTreeMap::new();
    for pid in projects {
        for s in crate::agentscope::load(store, pid).unwrap_or_default() {
            if !s.state(&now).is_live() {
                continue;
            }
            // `Scope.task` arrives with FTR-060 T5; until then a scope carries it in `extra`.
            let Some(task) = s.extra.get("task").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some((todo, key)) = task.split_once('/') else {
                continue;
            };
            let id = TaskId {
                project: pid.clone(),
                code: s.item.clone(),
                todo: todo.to_string(),
                key: key.to_string(),
            };
            out.insert(id.to_string(), s.id.clone());
        }
    }
    out
}

/// A warning for each task ref in `project` that no longer resolves (an item or task deleted
/// after the edge was written), or one for a graph that cannot be read.
pub fn issues(store: &Store, project: &str) -> Vec<Issue> {
    let graph = match TaskGraph::build(store, project, None) {
        Ok(g) => g,
        Err(e) => {
            return vec![Issue {
                severity: Severity::Warning,
                project: project.to_string(),
                code: None,
                message: format!("the task graph cannot be read: {e}"),
            }];
        }
    };
    let mut out = Vec::new();
    for n in graph.nodes.values().filter(|n| n.id.project == project) {
        for d in &n.depends_on {
            if !graph.nodes.contains_key(&d.to_string()) {
                out.push(Issue {
                    severity: Severity::Warning,
                    project: project.to_string(),
                    code: Some(n.id.code.clone()),
                    message: format!(
                        "task {} waits on {}, which no longer exists",
                        n.id.local(),
                        d.label(project)
                    ),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectConfig;
    use crate::models::{FeatureItem, Task, TodoList};
    use std::path::PathBuf;

    struct Board {
        store: Store,
        dir: PathBuf,
        config: ProjectConfig,
    }

    impl Drop for Board {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Board {
        fn new(name: &str) -> Board {
            let dir = std::env::temp_dir()
                .join(format!("kanbanr-taskgraph-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // L-41: the board is this test's own, under the system temp dir.
            assert!(dir.starts_with(std::env::temp_dir()));
            let store = Store::new(dir.clone());
            let config = ProjectConfig::for_new_project("demo");
            store.init_project("demo", config.clone()).unwrap();
            store
                .add_milestone("demo", "M", "", vec![], Some("M".into()))
                .unwrap();
            Board { store, dir, config }
        }

        fn build_status(&self) -> String {
            self.config.role_status(Role::Build).expect("a build stage")
        }

        fn before_build(&self) -> String {
            let build = self.build_status();
            self.config
                .statuses
                .iter()
                .find(|s| **s != build && !is_terminal_status(&self.config, s))
                .cloned()
                .expect("a stage before build")
        }

        fn terminal(&self) -> String {
            self.config
                .statuses
                .iter()
                .find(|s| is_terminal_status(&self.config, s) && !self.config.is_no_op(s))
                .cloned()
                .expect("a terminal stage")
        }

        /// An item at `status`, depending on `deps`, with list TL-001 holding `tasks`.
        fn item(&self, code: &str, status: &str, deps: &[&str], tasks: Vec<Task>) {
            let existing = self.store.load_meta("demo").unwrap();
            let mut f: FeatureItem = match existing.features.iter().find(|f| f.code == code) {
                Some(f) => f.clone(),
                None => self
                    .store
                    .add_feature("demo", code, "", "M", Some(code.into()))
                    .unwrap(),
            };
            f.status = status.to_string();
            f.depends_on = deps.iter().map(|d| d.to_string()).collect();
            f.todo_lists = vec![TodoList {
                code: "TL-001".into(),
                description: String::new(),
                tasks,
                created_at: crate::now_rfc3339(),
                extra: Default::default(),
            }];
            // Remove the item's files at its old status, then write it where its status puts it.
            remove_item_files(&self.store.features_dir("demo"), code);
            self.store.persist_feature_for_test("demo", &f).unwrap();
        }

        fn graph(&self) -> TaskGraph {
            TaskGraph::build(&self.store, "demo", None).unwrap()
        }
    }

    /// Every file under the board's features folder, with its contents.
    fn snapshot(dir: &std::path::Path) -> BTreeMap<PathBuf, String> {
        let mut out = BTreeMap::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(snapshot(&p));
            } else {
                out.insert(p.clone(), std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
        out
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

    use TaskState::{Completed as Done, InProgress as Running, NotStarted as Open};

    fn ids(v: &[ReadyTask]) -> Vec<String> {
        v.iter().map(|t| t.id.clone()).collect()
    }

    fn waits<'a>(r: &'a TaskReadiness, id: &str) -> &'a [Wait] {
        &r.waiting
            .iter()
            .find(|w| w.id == id)
            .unwrap_or_else(|| panic!("{id} is not waiting: {r:?}"))
            .waits
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn task_refs_are_read_in_every_form_and_code_slash_key_is_refused() {
        let p = |r| parse_task_ref(r, "demo", "FTR-1", "TL-001");
        assert_eq!(
            p("T3"),
            Some((
                "demo".into(),
                "FTR-1".into(),
                Some("TL-001".into()),
                "T3".into()
            ))
        );
        assert_eq!(
            p("TL-002/T3"),
            Some((
                "demo".into(),
                "FTR-1".into(),
                Some("TL-002".into()),
                "T3".into()
            ))
        );
        assert_eq!(
            p("FTR-2/TL-001/T1"),
            Some((
                "demo".into(),
                "FTR-2".into(),
                Some("TL-001".into()),
                "T1".into()
            ))
        );
        assert_eq!(
            p("other:FTR-2/TL-001/T1"),
            Some((
                "other".into(),
                "FTR-2".into(),
                Some("TL-001".into()),
                "T1".into()
            ))
        );
        assert_eq!(
            p("FTR-2/?/T1"),
            Some(("demo".into(), "FTR-2".into(), None, "T1".into()))
        );
        assert_eq!(p("FTR-2/T3"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("a/b/c/d"), None);
        let id = TaskId {
            project: "demo".into(),
            code: "FTR-1".into(),
            todo: "TL-001".into(),
            key: "T3".into(),
        };
        assert_eq!(id.to_string(), "demo:FTR-1/TL-001/T3");
        assert_eq!(id.local(), "FTR-1/TL-001/T3");
    }

    /// FTR-060 R-2: an unknown key, list or item, the `CODE/T3` form, and a cycle across two
    /// items are refused; the board on disk is unchanged.
    #[test]
    fn an_unknown_or_cyclic_task_dependency_is_refused() {
        let b = Board::new("refused");
        let build = b.build_status();
        b.item("FTR-1", &build, &[], vec![task("T1", Open, &[], &[], None)]);
        b.item("FTR-2", &build, &[], vec![task("T1", Open, &[], &[], None)]);
        let before = snapshot(&b.store.features_dir("demo"));
        assert!(!before.is_empty());

        let id = TaskId {
            project: "demo".into(),
            code: "FTR-1".into(),
            todo: "TL-001".into(),
            key: "T1".into(),
        };
        let with = |deps: &[&str]| -> Result<()> {
            let mut p = b.store.load_meta("demo").unwrap();
            let f = p.features.iter_mut().find(|f| f.code == "FTR-1").unwrap();
            f.todo_lists[0].tasks[0].depends_on = s(deps);
            let g = TaskGraph::build(&b.store, "demo", Some(&p))?;
            g.validate_task(&id)
        };
        assert!(with(&["FTR-2/TL-001/T1"]).is_ok());
        for bad in ["T9", "TL-009/T1", "FTR-9/TL-001/T1"] {
            match with(&[bad]) {
                Err(CoreError::UnknownDependency(r)) => assert!(!r.is_empty(), "{bad}"),
                other => panic!("{bad}: {other:?}"),
            }
        }
        assert!(matches!(
            with(&["FTR-2/T1"]),
            Err(CoreError::UnknownDependency(_))
        ));

        // FTR-2/T1 already waits on FTR-1/T1: FTR-1/T1 waiting on FTR-2/T1 closes the cycle.
        let mut p = b.store.load_meta("demo").unwrap();
        for f in &mut p.features {
            let other = if f.code == "FTR-1" { "FTR-2" } else { "FTR-1" };
            f.todo_lists[0].tasks[0].depends_on = vec![format!("{other}/TL-001/T1")];
        }
        match TaskGraph::build(&b.store, "demo", Some(&p)) {
            Err(CoreError::DependencyCycle(path)) => {
                assert_eq!(
                    path, "FTR-1/TL-001/T1 -> FTR-2/TL-001/T1 -> FTR-1/TL-001/T1",
                    "the cycle is named"
                );
            }
            other => panic!("a cycle is refused: {:?}", other.map(|_| ())),
        }
        let after = snapshot(&b.store.features_dir("demo"));
        assert_eq!(before, after, "the board files are unchanged");
        assert!(b.graph().nodes.len() == 2);
    }

    /// FTR-060 R-3: each reason a task waits, on its own fixture, and the ready case.
    #[test]
    fn a_task_is_ready_when_its_dependencies_are_done_its_item_is_at_build_and_no_running_task_writes_its_files()
     {
        let b = Board::new("ready");
        let build = b.build_status();
        let early = b.before_build();
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                task("T1", Done, &[], &["a/x.rs"], None),
                task("T2", Open, &["T1"], &["a/y.rs"], None),
                task("T3", Open, &["T4"], &["a/z.rs"], None),
                task("T4", Open, &[], &["a/w.rs"], None),
            ],
        );
        b.item(
            "FTR-2",
            &build,
            &[],
            vec![task("T1", Running, &[], &["src/"], None)],
        );
        b.item(
            "FTR-3",
            &build,
            &[],
            vec![task("T1", Open, &[], &["src/lib.rs", "CHANGELOG.md"], None)],
        );
        b.item(
            "FTR-4",
            &early,
            &[],
            vec![task("T1", Open, &[], &["b/q.rs"], None)],
        );
        b.item(
            "FTR-5",
            &build,
            &[],
            vec![
                task("T1", Running, &[], &["c/one.rs"], None),
                task("T2", Open, &[], &[], None),
            ],
        );
        let r = b.graph().readiness("demo", &s(&["CHANGELOG.md"]), None);
        let mut ready = ids(&r.ready);
        ready.sort();
        assert_eq!(ready, ["FTR-1/TL-001/T2", "FTR-1/TL-001/T4"], "{r:?}");
        let mut running = ids(&r.running);
        running.sort();
        assert_eq!(running, ["FTR-2/TL-001/T1", "FTR-5/TL-001/T1"]);
        assert_eq!(
            waits(&r, "FTR-1/TL-001/T3"),
            [Wait::Dependency("FTR-1/TL-001/T4".into())]
        );
        assert_eq!(
            waits(&r, "FTR-3/TL-001/T1"),
            [Wait::Conflict {
                task: "FTR-2/TL-001/T1".into(),
                paths: s(&["src/lib.rs"])
            }]
        );
        assert_eq!(waits(&r, "FTR-4/TL-001/T1"), [Wait::ItemNotAtBuild(early)]);
        assert_eq!(
            waits(&r, "FTR-5/TL-001/T2"),
            [Wait::UnknownFootprint("FTR-5/TL-001/T1".into())]
        );
        assert!(r.lanes.is_empty(), "no lanes unless asked");
    }

    /// FTR-060 R-3: B depends on A. With no task edge, B's tasks wait on all of A's; with
    /// B/T2 -> A/T1 only B/T2 waits; a terminal A releases all.
    #[test]
    fn cross_item_task_dependencies_refine_the_item_dependency() {
        let b = Board::new("refine");
        let build = b.build_status();
        let a_tasks = || {
            vec![
                task("T1", Open, &[], &["a/1.rs"], None),
                task("T2", Open, &[], &["a/2.rs"], None),
            ]
        };
        b.item("FTR-A", &build, &[], a_tasks());
        b.item(
            "FTR-B",
            &build,
            &["FTR-A"],
            vec![
                task("T1", Open, &[], &["b/1.rs"], None),
                task("T2", Open, &[], &["b/2.rs"], None),
            ],
        );
        let r = b.graph().readiness("demo", &[], None);
        for t in ["FTR-B/TL-001/T1", "FTR-B/TL-001/T2"] {
            assert_eq!(
                waits(&r, t),
                [
                    Wait::Dependency("FTR-A/TL-001/T1".into()),
                    Wait::Dependency("FTR-A/TL-001/T2".into())
                ],
                "{t} waits on all of A"
            );
        }

        b.item(
            "FTR-B",
            &build,
            &["FTR-A"],
            vec![
                task("T1", Open, &[], &["b/1.rs"], None),
                task("T2", Open, &["FTR-A/TL-001/T1"], &["b/2.rs"], None),
            ],
        );
        let r = b.graph().readiness("demo", &[], None);
        assert!(
            ids(&r.ready).contains(&"FTR-B/TL-001/T1".to_string()),
            "{r:?}"
        );
        assert_eq!(
            waits(&r, "FTR-B/TL-001/T2"),
            [Wait::Dependency("FTR-A/TL-001/T1".into())]
        );

        b.item("FTR-A", &b.terminal(), &[], a_tasks());
        let r = b.graph().readiness("demo", &[], None);
        let mut ready = ids(&r.ready);
        ready.sort();
        assert_eq!(ready, ["FTR-B/TL-001/T1", "FTR-B/TL-001/T2"], "{r:?}");
        assert!(
            r.waiting.is_empty(),
            "A's open tasks are done with A: {r:?}"
        );
    }

    /// FTR-060 R-3: a task with no files waits on a running task of its own item, not on one of
    /// another item.
    #[test]
    fn a_task_without_a_footprint_runs_alone_within_its_item() {
        let b = Board::new("footprint");
        let build = b.build_status();
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                task("T1", Running, &[], &["x/a.rs"], None),
                task("T2", Open, &[], &[], None),
            ],
        );
        b.item("FTR-2", &build, &[], vec![task("T1", Open, &[], &[], None)]);
        let r = b.graph().readiness("demo", &[], None);
        assert_eq!(ids(&r.ready), ["FTR-2/TL-001/T1"]);
        assert_eq!(
            waits(&r, "FTR-1/TL-001/T2"),
            [Wait::UnknownFootprint("FTR-1/TL-001/T1".into())]
        );

        // A running task with no files holds its whole item, and only its item.
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                task("T1", Running, &[], &[], None),
                task("T2", Open, &[], &["y/b.rs"], None),
            ],
        );
        let r = b.graph().readiness("demo", &[], None);
        assert_eq!(ids(&r.ready), ["FTR-2/TL-001/T1"]);
        assert_eq!(
            waits(&r, "FTR-1/TL-001/T2"),
            [Wait::Conflict {
                task: "FTR-1/TL-001/T1".into(),
                paths: vec![]
            }]
        );
    }

    /// FTR-060 R-4: five ready tasks, two overlapping on a folder, one pair overlapping only on
    /// the append-only file: three lanes take the critical one, skip the overlap, keep the pair.
    #[test]
    fn lanes_proposes_conflict_free_tasks_critical_path_first() {
        let b = Board::new("lanes");
        let build = b.build_status();
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                // The critical chain: T1 (3 h) then T6 (4 h).
                task("T1", Open, &[], &["api/core/"], Some(3.0)),
                task("T2", Open, &[], &["api/core/store.rs"], Some(5.0)),
                task("T3", Open, &[], &["web/a.ts", "CHANGELOG.md"], Some(2.0)),
                task("T4", Open, &[], &["docs/b.md", "CHANGELOG.md"], Some(1.0)),
                task("T5", Open, &[], &["tools/c.sh"], Some(0.5)),
                task("T6", Open, &["T1"], &["api/cli/"], Some(4.0)),
            ],
        );
        let g = b.graph();
        let r = g.readiness("demo", &s(&["CHANGELOG.md"]), Some(3));
        assert_eq!(ids(&r.ready).len(), 5, "{r:?}");
        assert_eq!(r.ready[0].id, "FTR-1/TL-001/T1", "critical first");
        assert!(r.ready[0].critical);
        assert_eq!(r.ready[0].chain_h, 7.0);
        assert_eq!(
            ids(&r.lanes),
            ["FTR-1/TL-001/T1", "FTR-1/TL-001/T3", "FTR-1/TL-001/T4"],
            "T2 overlaps T1's folder; T3 and T4 share only the append-only file"
        );
        let r = g.readiness("demo", &[], Some(3));
        assert_eq!(
            ids(&r.lanes),
            ["FTR-1/TL-001/T1", "FTR-1/TL-001/T3", "FTR-1/TL-001/T5"],
            "without the append-only rule T4 overlaps T3"
        );
        assert_eq!(
            overlaps(&s(&["api/core/"]), &s(&["api/core/store.rs"]), &[]),
            ["api/core/store.rs"]
        );
        assert!(overlaps(&s(&["api/core"]), &s(&["api/core/store.rs"]), &[]).is_empty());
        assert!(
            overlaps(
                &s(&["CHANGELOG.md"]),
                &s(&["CHANGELOG.md"]),
                &s(&["CHANGELOG.md"])
            )
            .is_empty()
        );
    }

    /// FTR-060 R-4: the critical path runs across items by estimate; Completed tasks weigh 0; ties
    /// go to the smaller id.
    #[test]
    fn the_task_critical_path_follows_estimates_across_items() {
        let b = Board::new("critical");
        let build = b.build_status();
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                task("T1", Done, &[], &["a"], Some(9.0)),
                task("T2", Open, &["T1"], &["b"], Some(2.0)),
                task("T3", Open, &[], &["c"], Some(1.0)),
            ],
        );
        b.item(
            "FTR-2",
            &build,
            &[],
            vec![
                task("T1", Open, &["FTR-1/TL-001/T2"], &["d"], Some(3.0)),
                task("T2", Open, &["FTR-1/TL-001/T3"], &["e"], None),
            ],
        );
        let g = b.graph();
        let (path, len) = g.critical_path("demo");
        assert_eq!(path, ["FTR-1/TL-001/T2", "FTR-2/TL-001/T1"]);
        assert_eq!(len, 5.0);
        assert_eq!(g.chain_h("demo:FTR-1/TL-001/T1"), 5.0, "done weighs 0");
        assert_eq!(
            g.chain_h("demo:FTR-1/TL-001/T3"),
            2.0,
            "no estimate weighs 1 h"
        );

        // Two equal chains: the smaller id wins.
        let b = Board::new("critical-tie");
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![
                task("T1", Open, &[], &["a"], Some(2.0)),
                task("T2", Open, &[], &["b"], Some(2.0)),
            ],
        );
        assert_eq!(
            b.graph().critical_path("demo"),
            (s(&["FTR-1/TL-001/T1"]), 2.0)
        );
    }

    /// FTR-060 R-8: graph, ready and four lanes over 3,000 tasks within 100 ms (median of 5).
    #[test]
    fn ready_and_lanes_stay_within_budget_on_3000_tasks() {
        let b = Board::new("budget");
        let build = b.build_status();
        let mut p = b.store.load_meta("demo").unwrap();
        let template = b
            .store
            .add_feature("demo", "seed", "", "M", Some("SEED-1".into()))
            .unwrap();
        for i in 0..300 {
            let mut f = template.clone();
            f.code = format!("FTR-{i}");
            f.status = build.clone();
            if i > 0 && i % 3 != 0 {
                f.depends_on = vec![format!("FTR-{}", i - 1)];
            }
            let tasks = (1..=10)
                .map(|k| {
                    let state = match (i + k) % 7 {
                        0 => Done,
                        1 => Running,
                        _ => Open,
                    };
                    let deps = if k > 1 {
                        vec![format!("T{}", k - 1)]
                    } else {
                        vec![]
                    };
                    let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
                    task(
                        &format!("T{k}"),
                        state,
                        &deps,
                        &[&format!("src/m{}/f{k}.rs", i % 40)],
                        Some(1.5),
                    )
                })
                .collect();
            f.todo_lists = vec![TodoList {
                code: "TL-001".into(),
                description: String::new(),
                tasks,
                created_at: crate::now_rfc3339(),
                extra: Default::default(),
            }];
            p.features.push(f);
        }
        let append = s(&["CHANGELOG.md"]);
        let mut times = Vec::new();
        for _ in 0..5 {
            let at = std::time::Instant::now();
            let g = TaskGraph::build(&b.store, "demo", Some(&p)).unwrap();
            let r = g.readiness("demo", &append, Some(4));
            assert_eq!(g.nodes.len(), 3000);
            assert!(r.lanes.len() <= 4);
            times.push(at.elapsed());
        }
        times.sort();
        let median = times[2];
        assert!(
            median < std::time::Duration::from_millis(100),
            "median {median:?} over 3,000 tasks"
        );
    }

    #[test]
    fn a_dangling_task_ref_is_a_doctor_warning() {
        let b = Board::new("issues");
        let build = b.build_status();
        b.item(
            "FTR-1",
            &build,
            &[],
            vec![task("T1", Open, &["FTR-9/TL-001/T1"], &[], None)],
        );
        let found = issues(&b.store, "demo");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(found[0].message.contains("FTR-9/TL-001/T1"));
        let r = b.graph().readiness("demo", &[], None);
        assert_eq!(
            waits(&r, "FTR-1/TL-001/T1"),
            [Wait::Dependency("FTR-9/TL-001/T1".into())]
        );
    }
}
