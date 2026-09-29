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
///
/// 3 — the config may declare `gates` (FEAT-113). A board is stamped 3 only when it
/// declares gates, so an older binary refuses it instead of silently ignoring its guardrails; a
/// board without gates stays at [`BASE_SCHEMA_VERSION`] and remains readable by older binaries.
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// What a board with no gates is stamped with: the storage layout of FEAT-071/FEAT-066.
pub const BASE_SCHEMA_VERSION: u32 = 2;

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
    /// Entry criteria per status (FEAT-113): what an item must show before it may move into that
    /// status, and what happens when it does. Absent means today's rules, synthesised by
    /// [`ProjectConfig::effective_gates`]; absent stays absent on disk.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gates: BTreeMap<String, Gate>,
    /// What this project estimates in (FEAT-121): days, or story points.
    #[serde(default, skip_serializing_if = "EstimateUnit::is_days")]
    pub estimate_unit: EstimateUnit,
    /// Whether this project works in sprints and releases at all (FEAT-121). Off unless switched
    /// on: many projects follow a different workflow, and data nobody asked for is clutter.
    #[serde(default, skip_serializing_if = "Cadence::is_off")]
    pub cadence: Cadence,
}

/// The unit a project estimates in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateUnit {
    #[default]
    Days,
    Points,
}

impl EstimateUnit {
    fn is_days(&self) -> bool {
        *self == EstimateUnit::Days
    }
}

/// A project's rhythm (FEAT-121): whether it has sprints and releases, and their defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cadence {
    #[serde(default)]
    pub sprints: bool,
    #[serde(default)]
    pub releases: bool,
    /// The length `sprint add` uses when none is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_length_days: Option<u32>,
    /// How often a release is cut: `per_sprint`, `every_n` sprints, or `on_demand`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
}

impl Cadence {
    fn is_off(&self) -> bool {
        *self == Cadence::default()
    }
}

/// Whether a failed gate stops the move or only reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforce {
    #[default]
    Block,
    Warn,
}

impl Enforce {
    fn is_block(&self) -> bool {
        *self == Enforce::Block
    }
}

/// Something that happens when an item enters a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// `kanbanr start` creates the item's branch here (where the project is a git repository).
    Branch,
}

/// The entry criteria of one status. Every field is optional; an empty gate asks nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    /// What this stage is for, in a sentence — shown to Claude and in the monitor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    /// Conditions that must hold to enter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<crate::readiness::Condition>,
    /// Conditions reported, never enforced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warns: Vec<crate::readiness::Condition>,
    /// Named sign-offs that must be recorded against the current definition (FEAT-114).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signoffs: Vec<String>,
    /// `block` (the default) refuses the move; `warn` allows it and reports what `requires` lacks.
    #[serde(default, skip_serializing_if = "Enforce::is_block")]
    pub enforce: Enforce,
    /// When set, the gate applies only to items of these kinds (an item with no kind is a
    /// `feature`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_enter: Vec<Action>,
}

impl Gate {
    /// Does this gate apply to an item of `kind`?
    pub fn applies_to(&self, kind: Option<&str>) -> bool {
        let kind = kind
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or("feature");
        self.kinds.is_empty() || self.kinds.iter().any(|k| k.eq_ignore_ascii_case(kind))
    }
}

/// The default no-op (inert disposition) states.
pub fn default_no_op_states() -> Vec<String> {
    vec![
        "No Action".to_string(),
        "Not Applicable".to_string(),
        "Out-of-Scope".to_string(),
    ]
}

/// A workflow as a file: what `--from-file` reads, `--export` writes, and every preset is
/// (FEAT-116). The project-specific parts of a config (name, description, branch pattern) are not in
/// it, so one process file serves any number of projects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowFile {
    pub statuses: Vec<String>,
    #[serde(default)]
    pub default_state: String,
    #[serde(default)]
    pub displayed_states: Vec<String>,
    #[serde(default)]
    pub no_op_states: Vec<String>,
    #[serde(default)]
    pub terminal_states: Vec<String>,
    #[serde(default)]
    pub transitions: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gates: BTreeMap<String, Gate>,
}

/// The built-in processes, as data (FEAT-116). Each file's leading comment is its description.
const PRESETS: &[(&str, &str)] = &[
    ("default", include_str!("../presets/default.yaml")),
    ("scheduled", include_str!("../presets/scheduled.yaml")),
    ("togaf", include_str!("../presets/togaf.yaml")),
    ("pdca", include_str!("../presets/pdca.yaml")),
    (
        "design-control",
        include_str!("../presets/design-control.yaml"),
    ),
];

/// Every preset: `(name, one-line description)`.
pub fn presets() -> Vec<(&'static str, String)> {
    PRESETS
        .iter()
        .map(|(name, text)| {
            let about: Vec<&str> = text
                .lines()
                .take_while(|l| l.starts_with('#'))
                .map(|l| l.trim_start_matches('#').trim())
                .collect();
            (*name, about.join(" "))
        })
        .collect()
}

/// A preset by name, or an error that lists the ones there are.
pub fn preset(name: &str) -> crate::Result<WorkflowFile> {
    let key = name.trim().to_ascii_lowercase();
    let text = PRESETS
        .iter()
        .find(|(n, _)| *n == key)
        .map(|(_, t)| *t)
        .ok_or_else(|| {
            let known: Vec<&str> = PRESETS.iter().map(|(n, _)| *n).collect();
            crate::CoreError::Unsupported(format!(
                "no workflow preset named '{name}' — the presets are: {}",
                known.join(", ")
            ))
        })?;
    serde_yaml::from_str(text)
        .map_err(|e| crate::CoreError::Unsupported(format!("the {key} preset does not parse: {e}")))
}

impl WorkflowFile {
    /// This workflow as a project's config.
    pub fn into_config(self, name: &str) -> ProjectConfig {
        let mut config = ProjectConfig {
            schema_version: BASE_SCHEMA_VERSION,
            name: name.to_string(),
            description: String::new(),
            statuses: self.statuses,
            default_state: self.default_state,
            transitions: self.transitions,
            displayed_states: self.displayed_states,
            no_op_states: self.no_op_states,
            terminal_states: self.terminal_states,
            branch_pattern: None,
            gates: self.gates,
            estimate_unit: EstimateUnit::default(),
            cadence: Cadence::default(),
        };
        config.schema_version = config.required_schema_version();
        config
    }

    /// The workflow part of a project's config, as a file.
    pub fn from_config(config: &ProjectConfig) -> WorkflowFile {
        WorkflowFile {
            statuses: config.statuses.clone(),
            default_state: config.default_state.clone(),
            displayed_states: config.displayed_states.clone(),
            no_op_states: config.no_op_states.clone(),
            terminal_states: config.terminal_states.clone(),
            transitions: config.transitions.clone(),
            gates: config.gates.clone(),
        }
    }
}

/// The TOGAF phases as board columns, with the definition growing phase by phase (the `togaf`
/// preset).
pub fn togaf_preset(name: &str) -> ProjectConfig {
    preset("togaf")
        .expect("the built-in togaf preset parses")
        .into_config(name)
}

impl ProjectConfig {
    /// The `scheduled` preset — kanbanr's original default (Planned → Scheduled → Completed). What
    /// the tests build on; a new project gets [`ProjectConfig::for_new_project`] instead.
    pub fn default_for(name: &str) -> ProjectConfig {
        preset("scheduled")
            .expect("the built-in scheduled preset parses")
            .into_config(name)
    }

    /// The workflow a new project gets when it names none: the `default` preset (FEAT-116).
    pub fn for_new_project(name: &str) -> ProjectConfig {
        preset("default")
            .expect("the built-in default preset parses")
            .into_config(name)
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

    /// The schema a config is stamped with: gates need a reader that enforces them.
    pub fn required_schema_version(&self) -> u32 {
        if self.gates.is_empty() {
            BASE_SCHEMA_VERSION
        } else {
            CURRENT_SCHEMA_VERSION
        }
    }

    /// The gates this workflow enforces: the declared ones, or — when none are declared — today's
    /// rules, synthesised so that no existing board changes behaviour (FEAT-113).
    ///
    /// The synthesis is exactly the old start gate: entering a status the board displays as work in
    /// flight (displayed, not the default backlog, not terminal, not a no-op) needs a definition
    /// and a current agreement. Parking, closing and dispositioning are never gated — you can
    /// always stop work (L-19). The first such status is where `start` creates the branch.
    pub fn effective_gates(&self) -> BTreeMap<String, Gate> {
        if !self.gates.is_empty() {
            return self.gates.clone();
        }
        use crate::readiness::{Check, Condition};
        let mut gates = BTreeMap::new();
        let mut first = true;
        for status in self.displayed_states.iter().filter(|s| {
            **s != self.default_state
                && !crate::graph::is_terminal_status(self, s)
                && !self.is_no_op(s)
        }) {
            gates.insert(
                status.clone(),
                Gate {
                    requires: vec![
                        Condition::Check(Check::Definition),
                        Condition::Check(Check::Approved),
                    ],
                    on_enter: if first { vec![Action::Branch] } else { vec![] },
                    ..Gate::default()
                },
            );
            first = false;
        }
        gates
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
