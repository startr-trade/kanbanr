//! Mermaid `stateDiagram-v2` I/O for the project workflow (FEAT-039).
//!
//! The per-project `config.yaml` is the SINGLE SOURCE OF TRUTH for the workflow; Mermaid is only an
//! I/O *format* (import + export), never the stored representation. This module renders a
//! [`ProjectConfig`](crate::ProjectConfig) to a flat state diagram and parses the same flat subset
//! back into a [`WorkflowDef`] that the CLI threads into the existing `config workflow` write path.
//!
//! The supported (flat) grammar is deliberately small:
//! - `[*] --> X`        — the start edge; `X` is the default state.
//! - `X --> [*]`        — a terminal (end) state.
//! - `A --> B`          — an allowed transition (an optional `: label` is ignored).
//! - `state "Label" as id` — declares a status whose human name is `Label`, referenced by `id`.
//!
//! Composite / parallel / history features (`state X {`, `--`, `[H]`) are rejected.

use crate::error::{CoreError, Result};
use crate::ProjectConfig;
use std::collections::BTreeMap;

/// A workflow parsed out of a Mermaid state diagram. The CLI maps this onto `set_workflow`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowDef {
    /// Every status named in the diagram (in first-seen order).
    pub statuses: Vec<String>,
    /// Allowed transitions, from-status -> to-statuses.
    pub transitions: BTreeMap<String, Vec<String>>,
    /// The default (start) state, from `[*] --> X`, if present.
    pub default_state: Option<String>,
    /// Explicit terminal states, from `X --> [*]`.
    pub terminal_states: Vec<String>,
}

/// True when `s` is usable verbatim as a Mermaid state id (no aliasing needed): non-empty and made
/// only of ASCII alphanumerics or `_`.
fn is_plain_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A stable Mermaid id for a status name. Plain names are used as-is; names with spaces/`-`/etc. are
/// sanitized to `[A-Za-z0-9_]` so the diagram stays valid, and aliased via `state "Name" as id`.
fn id_for(name: &str) -> String {
    if is_plain_id(name) {
        return name.to_string();
    }
    let mut id: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // A Mermaid id can't start with a digit; prefix if needed (and never be empty).
    if id.is_empty() || id.starts_with(|c: char| c.is_ascii_digit()) {
        id.insert(0, 's');
    }
    id
}

/// Render a project config as a Mermaid `stateDiagram-v2`.
///
/// Emits, in order: alias declarations for any status whose name isn't a plain id; the start edge
/// `[*] --> <default>`; one `A --> B` per allowed transition; a `X --> [*]` for each terminal state;
/// and a `note` flagging each no-op (inert) state. Every distinct status name maps to a single
/// Mermaid id, so the round-trip recovers the original names.
pub fn to_state_diagram(config: &ProjectConfig) -> String {
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for s in &config.statuses {
        ids.entry(s.clone()).or_insert_with(|| id_for(s));
    }
    // Resolve a status name to its id, minting one on the fly for names referenced by transitions
    // but somehow absent from `statuses` (defensive — keeps the diagram self-consistent).
    let id_of = |name: &str, ids: &BTreeMap<String, String>| -> String {
        ids.get(name).cloned().unwrap_or_else(|| id_for(name))
    };

    let mut out = String::from("stateDiagram-v2\n");

    // Alias declarations for non-plain names (sorted for stable output).
    for (name, id) in &ids {
        if id != name {
            out.push_str(&format!("    state \"{name}\" as {id}\n"));
        }
    }

    // Start edge.
    if !config.default_state.is_empty() {
        out.push_str(&format!(
            "    [*] --> {}\n",
            id_of(&config.default_state, &ids)
        ));
    }

    // Transitions (BTreeMap keys are already sorted).
    for (from, tos) in &config.transitions {
        for to in tos {
            out.push_str(&format!(
                "    {} --> {}\n",
                id_of(from, &ids),
                id_of(to, &ids)
            ));
        }
    }

    // Terminal edges.
    for t in &config.terminal_states {
        out.push_str(&format!("    {} --> [*]\n", id_of(t, &ids)));
    }

    // No-op annotations.
    for n in &config.no_op_states {
        out.push_str(&format!(
            "    note right of {}: no-op (inert disposition)\n",
            id_of(n, &ids)
        ));
    }

    out
}

/// Reject Mermaid features outside the flat subset we support, with a clear message.
fn reject_unsupported(line: &str) -> Result<()> {
    let t = line.trim();
    // Composite state: `state Foo {` (an opening brace introduces a nested machine).
    if t.starts_with("state") && t.ends_with('{') {
        return Err(CoreError::InvalidMermaid(format!(
            "composite states are not supported: '{t}'"
        )));
    }
    if t == "}" {
        return Err(CoreError::InvalidMermaid(
            "composite states are not supported: stray '}'".into(),
        ));
    }
    // Parallel (concurrency) divider.
    if t == "--" {
        return Err(CoreError::InvalidMermaid(
            "parallel/concurrent states ('--') are not supported".into(),
        ));
    }
    // History pseudo-states.
    if t.contains("[H]") || t.contains("[H*]") {
        return Err(CoreError::InvalidMermaid(
            "history states ('[H]') are not supported".into(),
        ));
    }
    Ok(())
}

/// Parse the flat Mermaid subset into a [`WorkflowDef`], resolving aliased ids back to their
/// human-readable status names. Returns [`CoreError::InvalidMermaid`] on composite/parallel/history
/// constructs or malformed edges.
pub fn parse_state_diagram(text: &str) -> Result<WorkflowDef> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut statuses: Vec<String> = Vec::new();
    let mut transitions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut default_state: Option<String> = None;
    let mut terminal_states: Vec<String> = Vec::new();

    // First pass: collect `state "Label" as id` aliases so edges can resolve ids -> labels.
    for raw in text.lines() {
        let line = strip_comment(raw);
        let t = line.trim();
        if let Some((label, id)) = parse_alias(t) {
            aliases.insert(id, label);
        }
    }

    let resolve =
        |tok: &str| -> String { aliases.get(tok).cloned().unwrap_or_else(|| tok.to_string()) };
    let see = |s: &str, statuses: &mut Vec<String>| {
        if !statuses.iter().any(|x| x == s) {
            statuses.push(s.to_string());
        }
    };

    for raw in text.lines() {
        let line = strip_comment(raw);
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        reject_unsupported(t)?;
        // Header / declarations are skipped.
        if t == "stateDiagram-v2" || t == "stateDiagram" {
            continue;
        }
        if parse_alias(t).is_some() {
            // Make sure aliased statuses appear even if they have no edges.
            if let Some((label, _)) = parse_alias(t) {
                see(&label, &mut statuses);
            }
            continue;
        }
        if t.starts_with("note ") || t.starts_with("note:") {
            continue;
        }
        if t.starts_with("direction ") || t.starts_with("classDef ") || t.starts_with("class ") {
            continue;
        }

        // An edge: `LHS --> RHS [: label]`.
        if let Some((lhs, rhs_full)) = t.split_once("-->") {
            let lhs = lhs.trim();
            // Drop an optional `: label` on the edge.
            let rhs = rhs_full.split(':').next().unwrap_or(rhs_full).trim();
            if lhs.is_empty() || rhs.is_empty() {
                return Err(CoreError::InvalidMermaid(format!("malformed edge: '{t}'")));
            }
            match (lhs == "[*]", rhs == "[*]") {
                (true, true) => {
                    return Err(CoreError::InvalidMermaid(
                        "edge from start to end ('[*] --> [*]') is not supported".into(),
                    ));
                }
                (true, false) => {
                    let to = resolve(rhs);
                    see(&to, &mut statuses);
                    default_state = Some(to);
                }
                (false, true) => {
                    let from = resolve(lhs);
                    see(&from, &mut statuses);
                    if !terminal_states.contains(&from) {
                        terminal_states.push(from);
                    }
                }
                (false, false) => {
                    let from = resolve(lhs);
                    let to = resolve(rhs);
                    see(&from, &mut statuses);
                    see(&to, &mut statuses);
                    let entry = transitions.entry(from).or_default();
                    if !entry.contains(&to) {
                        entry.push(to);
                    }
                }
            }
            continue;
        }

        return Err(CoreError::InvalidMermaid(format!(
            "unrecognized line: '{t}'"
        )));
    }

    Ok(WorkflowDef {
        statuses,
        transitions,
        default_state,
        terminal_states,
    })
}

/// Strip a trailing `%% ...` Mermaid comment from a line.
fn strip_comment(line: &str) -> &str {
    match line.find("%%") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Parse a `state "Label" as id` declaration into `(label, id)`.
fn parse_alias(t: &str) -> Option<(String, String)> {
    let rest = t.strip_prefix("state ")?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let (label, after) = rest.split_once('"')?;
    let after = after.trim();
    let id = after.strip_prefix("as ")?.trim();
    if id.is_empty() {
        return None;
    }
    Some((label.to_string(), id.to_string()))
}
