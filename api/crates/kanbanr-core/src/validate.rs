//! Validation helpers: code generation, milestone DAG cycle detection.

use crate::error::{CoreError, Result};
use crate::models::{FeatureItem, Milestone};
use std::collections::{HashMap, HashSet};

/// Generate the next sequential code for a prefix (e.g. "FEAT" -> "FEAT-001") that does not
/// collide with `existing` codes. Scans existing `PREFIX-<n>` codes and picks max+1.
pub fn next_code(prefix: &str, existing: &[String]) -> String {
    let mut max = 0u32;
    let needle = format!("{prefix}-");
    for code in existing {
        if let Some(rest) = code.strip_prefix(&needle) {
            if let Ok(n) = rest.parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    format!("{prefix}-{:03}", max + 1)
}

/// Generate the next unpadded key for a prefix (`"T"` -> `T1`, `"G-"` -> `G-1`, `"R-"` -> `R-1`),
/// skipping past every numeric key already in use. Unpadded because these ids are quoted inline in
/// prose, acceptance criteria and commit trailers — unlike feature codes, which are filenames and
/// so stay zero-padded via [`next_code`].
pub fn next_key(prefix: &str, existing: &[String]) -> String {
    let mut max = 0u32;
    for key in existing {
        if let Some(rest) = key.strip_prefix(prefix) {
            if let Ok(n) = rest.parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    format!("{prefix}{}", max + 1)
}

/// Generate the next task key for a feature (e.g. "T1") not colliding with existing keys.
pub fn next_task_key(existing: &[String]) -> String {
    next_key("T", existing)
}

/// Validate that the milestone dependency graph is acyclic and every referenced dep exists.
pub fn validate_dependencies(milestones: &[Milestone], code: &str) -> Result<()> {
    let nodes: Vec<(&str, &Vec<String>)> = milestones
        .iter()
        .map(|m| (m.code.as_str(), &m.depends_on))
        .collect();
    validate_dag(&nodes, code)
}

/// Validate that the **same-project** cross-feature dependency graph is acyclic and every referenced
/// dep exists. Only bare (unqualified) deps participate here; qualified `"project:code"` deps point
/// at other projects and are validated against the portfolio-wide graph (see [`crate::graph`]).
pub fn validate_feature_dependencies(features: &[FeatureItem], code: &str) -> Result<()> {
    let filtered: Vec<(String, Vec<String>)> = features
        .iter()
        .map(|f| {
            let local = f
                .depends_on
                .iter()
                .filter(|d| !d.contains(':'))
                .cloned()
                .collect();
            (f.code.clone(), local)
        })
        .collect();
    let nodes: Vec<(&str, &Vec<String>)> = filtered.iter().map(|(c, d)| (c.as_str(), d)).collect();
    validate_dag(&nodes, code)
}

/// Generic DAG validation over `(code, depends_on)` nodes: every referenced dependency must exist
/// and the graph must be acyclic. `code` is the node whose deps just changed (for error messages).
fn validate_dag(nodes: &[(&str, &Vec<String>)], code: &str) -> Result<()> {
    let codes: HashSet<&str> = nodes.iter().map(|(c, _)| *c).collect();
    let graph: HashMap<&str, &Vec<String>> = nodes.iter().map(|(c, d)| (*c, *d)).collect();

    // All referenced dependencies must exist.
    for (_, deps) in nodes {
        for dep in deps.iter() {
            if !codes.contains(dep.as_str()) {
                return Err(CoreError::UnknownDependency(dep.clone()));
            }
        }
    }

    // Detect cycles via DFS with a recursion stack.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    let mut marks: HashMap<&str, Mark> = HashMap::new();

    fn visit<'a>(
        node: &'a str,
        graph: &HashMap<&'a str, &'a Vec<String>>,
        marks: &mut HashMap<&'a str, Mark>,
        start: &str,
    ) -> Result<()> {
        match marks.get(node) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => return Err(CoreError::DependencyCycle(start.to_string())),
            None => {}
        }
        marks.insert(node, Mark::Visiting);
        if let Some(deps) = graph.get(node) {
            for dep in deps.iter() {
                // Look up the canonical &str key so lifetimes line up.
                if let Some((k, _)) = graph.get_key_value(dep.as_str()) {
                    visit(k, graph, marks, start)?;
                }
            }
        }
        marks.insert(node, Mark::Done);
        Ok(())
    }

    // Start the DFS from the changed node if present, else check all.
    if codes.contains(code) {
        if let Some((k, _)) = graph.get_key_value(code) {
            visit(k, &graph, &mut marks, code)?;
        }
    }
    for (c, _) in nodes {
        if let Some((k, _)) = graph.get_key_value(*c) {
            visit(k, &graph, &mut marks, c)?;
        }
    }
    Ok(())
}

/// A project/feature/milestone name segment is restricted so it maps cleanly to a folder/file.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
