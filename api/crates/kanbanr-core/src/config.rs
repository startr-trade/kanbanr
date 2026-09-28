//! Per-project configuration: the status workflow (statuses + allowed transitions) and
//! which states are displayed on the dashboard.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The schema version stamped onto freshly-written project configs. A legacy config that predates
/// the field deserializes to `schema_version = 0` (via `#[serde(default)]`), letting `doctor` flag
/// it as outdated. Bump this when the on-disk config shape changes in a way worth surfacing.
/// The on-disk schema this binary writes and understands.
///
/// **Bump this whenever the storage layout changes** — not when a field is added. A new optional
/// field is readable by an older binary, which simply ignores it; a moved file is not, and an older
/// reader that cannot find it reports an empty board rather than an error. That happened: FEAT-071
/// moved the status folders under `features/` and FEAT-066 split the logs into day files, and a
/// daemon started before those changes served a board with zero items, which reads as data loss.
///
/// History: 1 — status folders at the project root, single-file logs. 2 — status folders under
/// `features/`, one log file per day.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// On-disk schema version. Absent in legacy configs (defaults to 0); forward-stamped to
    /// `CURRENT_SCHEMA_VERSION` whenever the config is written.
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub statuses: Vec<String>,
    /// The status assigned to a newly-created feature item. Must be one of `statuses`.
    #[serde(default)]
    pub default_state: String,
    /// Map of from-status -> list of allowed to-statuses.
    #[serde(default)]
    pub transitions: BTreeMap<String, Vec<String>>,
    /// Ordered subset of `statuses` shown on the dashboard.
    #[serde(default)]
    pub displayed_states: Vec<String>,
    /// Subset of `statuses` that are functionally inert dispositions (e.g. "No Action",
    /// "Not Applicable", "Out-of-Scope"). No-op states are always non-displayed, and a feature
    /// in a no-op state does not auto-advance to "Completed" when its tasks finish.
    #[serde(default)]
    pub no_op_states: Vec<String>,
    /// Subset of `statuses` that are explicit terminal (end) states of the workflow — a feature
    /// here is considered "done". Absent in legacy configs (defaults to empty), in which case
    /// `graph::is_terminal_status` falls back to its built-in heuristic (Completed / no-op).
    #[serde(default)]
    pub terminal_states: Vec<String>,
    /// How `kanbanr start` names a branch (FEAT-056): `{code}` and `{slug}`. Absent means the
    /// default, and absent stays absent on disk so an existing config is not rewritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_pattern: Option<String>,
}

/// The default no-op (inert disposition) states.
pub fn default_no_op_states() -> Vec<String> {
    vec![
        "No Action".to_string(),
        "Not Applicable".to_string(),
        "Out-of-Scope".to_string(),
    ]
}

/// An optional TOGAF-phase workflow (FEAT-051), for projects that want architecture phases as
/// board columns: Vision → Business Arch → System Design → Implementation → Migration →
/// Operations. Opt-in, because a phase model is a real commitment; the default workflow stays a
/// plain backlog → scheduled → done. The phase IS the status — there is no second field to keep
/// in step.
pub fn togaf_preset(name: &str) -> ProjectConfig {
    let phases = [
        "Vision",
        "Business Arch",
        "System Design",
        "Implementation",
        "Migration",
        "Operations",
    ];
    let no_ops = default_no_op_states();
    let mut statuses: Vec<String> = phases.iter().map(|s| s.to_string()).collect();
    statuses.extend(no_ops.iter().cloned());

    // Forward one phase, back one phase (rework is normal), and out to any no-op disposition.
    let mut transitions = BTreeMap::new();
    for (i, phase) in phases.iter().enumerate() {
        let mut next: Vec<String> = Vec::new();
        if let Some(forward) = phases.get(i + 1) {
            next.push(forward.to_string());
        }
        if i > 0 {
            next.push(phases[i - 1].to_string());
        }
        next.extend(no_ops.iter().cloned());
        transitions.insert(phase.to_string(), next);
    }
    for n in &no_ops {
        transitions.insert(n.clone(), vec!["Vision".to_string()]);
    }

    ProjectConfig {
        branch_pattern: None,
        schema_version: CURRENT_SCHEMA_VERSION,
        name: name.to_string(),
        description: String::new(),
        displayed_states: phases.iter().map(|s| s.to_string()).collect(),
        default_state: "Vision".to_string(),
        terminal_states: vec!["Operations".to_string()],
        no_op_states: no_ops,
        statuses,
        transitions,
    }
}

impl ProjectConfig {
    /// Sensible defaults for a new project.
    pub fn default_for(name: &str) -> ProjectConfig {
        let no_ops = default_no_op_states();
        let mut statuses = vec![
            "Deferred".to_string(),
            "Planned".to_string(),
            "Scheduled".to_string(),
            "Completed".to_string(),
        ];
        statuses.extend(no_ops.iter().cloned());

        // Active states can be dispositioned to any no-op state; no-op states reopen to Planned.
        let with_no_ops = |mut v: Vec<String>| {
            v.extend(no_ops.iter().cloned());
            v
        };
        let mut transitions = BTreeMap::new();
        transitions.insert(
            "Deferred".to_string(),
            with_no_ops(vec!["Planned".to_string()]),
        );
        transitions.insert(
            "Planned".to_string(),
            with_no_ops(vec!["Scheduled".to_string(), "Deferred".to_string()]),
        );
        transitions.insert(
            "Scheduled".to_string(),
            with_no_ops(vec!["Completed".to_string(), "Planned".to_string()]),
        );
        transitions.insert("Completed".to_string(), vec!["Scheduled".to_string()]);
        for n in &no_ops {
            transitions.insert(n.clone(), vec!["Planned".to_string()]);
        }

        ProjectConfig {
            branch_pattern: None,
            schema_version: CURRENT_SCHEMA_VERSION,
            name: name.to_string(),
            description: String::new(),
            // No-op states (and Deferred) are intentionally NOT displayed.
            displayed_states: vec![
                "Planned".to_string(),
                "Scheduled".to_string(),
                "Completed".to_string(),
            ],
            default_state: "Planned".to_string(),
            terminal_states: vec!["Completed".to_string()],
            no_op_states: no_ops,
            statuses,
            transitions,
        }
    }

    /// Is this status an explicit terminal (end) state of the workflow?
    pub fn is_terminal(&self, status: &str) -> bool {
        self.terminal_states.iter().any(|s| s == status)
    }

    /// The branch pattern this project uses.
    pub fn branch_pattern(&self) -> &str {
        self.branch_pattern
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or(crate::scm::DEFAULT_BRANCH_PATTERN)
    }

    pub fn has_status(&self, status: &str) -> bool {
        self.statuses.iter().any(|s| s == status)
    }

    /// Is this status a functionally inert (no-op) disposition?
    pub fn is_no_op(&self, status: &str) -> bool {
        self.no_op_states.iter().any(|s| s == status)
    }

    /// Is moving from `from` to `to` allowed by the workflow? (Same-status is always allowed.)
    pub fn transition_allowed(&self, from: &str, to: &str) -> bool {
        if from == to {
            return true;
        }
        self.transitions
            .get(from)
            .map(|tos| tos.iter().any(|t| t == to))
            .unwrap_or(false)
    }

    /// The declared default status for new feature items (falls back to the first status).
    pub fn default_status(&self) -> String {
        if !self.default_state.is_empty() && self.has_status(&self.default_state) {
            self.default_state.clone()
        } else {
            self.statuses.first().cloned().unwrap_or_default()
        }
    }
}
