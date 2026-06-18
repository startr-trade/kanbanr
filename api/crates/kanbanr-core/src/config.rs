//! Per-project configuration: the status workflow (statuses + allowed transitions) and
//! which states are displayed on the dashboard.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The schema version stamped onto freshly-written project configs. A legacy config that predates
/// the field deserializes to `schema_version = 0` (via `#[serde(default)]`), letting `doctor` flag
/// it as outdated. Bump this when the on-disk config shape changes in a way worth surfacing.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
}

/// The default no-op (inert disposition) states.
pub fn default_no_op_states() -> Vec<String> {
    vec![
        "No Action".to_string(),
        "Not Applicable".to_string(),
        "Out-of-Scope".to_string(),
    ]
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
            no_op_states: no_ops,
            statuses,
            transitions,
        }
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
