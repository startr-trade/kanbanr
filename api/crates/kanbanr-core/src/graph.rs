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
use std::collections::BTreeMap;

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
