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
///
/// 4 — gates may only label stage roles, or be addressed to a role (`@build`) (FEAT-188). Stamped
/// only then: an older reader would take the labels for declared gates, or never match `@build`.
pub const CURRENT_SCHEMA_VERSION: u32 = 4;

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
    /// The saved process this workflow was applied from, if it was (FEAT-169): what drift is
    /// measured against. An older kanbanr keeps it through `extra`, so it needs no schema bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessSource>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,
}

/// Where a saved process lives (FEAT-169), in the order a name is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Library {
    /// `<board>/processes/`: shared with everyone who has the board, through its remote.
    Board,
    /// `~/.kanbanr/processes/`: this user's, on any board.
    Personal,
    /// Carried by the binary.
    Builtin,
}

impl Library {
    pub fn as_str(self) -> &'static str {
        match self {
            Library::Board => "board",
            Library::Personal => "personal",
            Library::Builtin => "built-in",
        }
    }
}

/// Which saved process a project's workflow came from, and which version of it (FEAT-169).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSource {
    pub name: String,
    pub library: Library,
    /// The process's version when it was applied; 0 for a built-in one.
    #[serde(default)]
    pub version: u32,
    /// The content rev of what was applied ([`WorkflowFile::content_rev`]).
    pub rev: String,
}

/// A saved process's header (FEAT-169). Optional: a file without one is still a process, and an
/// older kanbanr reading a file with one ignores it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHeader {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Bumped by `kanbanr process save` when the content changes.
    #[serde(default)]
    pub version: u32,
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
    pub fn is_days(&self) -> bool {
        *self == EstimateUnit::Days
    }
}

/// A project's rhythm (FEAT-121): whether it has sprints and releases, and their defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// The environments a release moves through, in order (FEAT-183). The last one is the cut.
    /// Empty means [`DEFAULT_RELEASE_STAGES`]; read it through [`Cadence::stages`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_stages: Vec<String>,
    /// The branch a release branch is made from. Unset: the code repository's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_branch: Option<String>,
    /// The branch production is fast-forwarded onto. Unset: the code repository's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_branch: Option<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,
}

/// The stages a release moves through when a project names none (FEAT-183).
pub const DEFAULT_RELEASE_STAGES: [&str; 4] = ["candidate", "test", "staging", "production"];

impl Cadence {
    pub fn is_off(&self) -> bool {
        *self == Cadence::default()
    }

    /// The release stages in force: the project's own list, else the default. The first makes the
    /// release branch, the last is the cut (FEAT-183).
    pub fn stages(&self) -> Vec<String> {
        if self.release_stages.is_empty() {
            DEFAULT_RELEASE_STAGES
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.release_stages.clone()
        }
    }

    /// A stage list must name each stage once, and none blank. An empty list is not checked here:
    /// it means the default, and refusing an explicitly empty one is the caller's (FEAT-183).
    pub fn validate_stages(stages: &[String]) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for stage in stages {
            let stage = stage.trim();
            if stage.is_empty() {
                return Err("a release stage cannot be blank".into());
            }
            if !seen.insert(stage) {
                return Err(format!("release stage '{stage}' is listed twice"));
            }
        }
        Ok(())
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

impl Gate {
    /// Whether judging this gate reads spec bodies (FEAT-179), so a metadata-only load would
    /// report every spec section as missing.
    pub fn reads_spec(&self) -> bool {
        self.requires
            .iter()
            .chain(&self.warns)
            .any(crate::readiness::Condition::reads_spec)
    }
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
    /// The stage roles this status holds (FEAT-188): `design`, `build`, `integrate`, `release`. The
    /// method and git automation find their stage by role, so they behave the same in every
    /// process. Kept as names, so an unknown one is a reported problem, not a parse failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Reaching this stage finishes an item for the cadence views — the sprint burndown, its done
    /// total, what a closing sprint carries over (FEAT-137). Scrum's Definition of Done is the Done
    /// column, not the release that ships it, and a burndown that waited for the release dropped in
    /// one step on release day.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,
}

impl Gate {
    /// A gate that only names roles and asks nothing (FEAT-188). It labels a stage; it does not
    /// declare a guardrail, so a workflow whose gates are all labels keeps the built-in rule.
    pub fn is_role_label(&self) -> bool {
        !self.roles.is_empty()
            && self.purpose.is_empty()
            && self.requires.is_empty()
            && self.warns.is_empty()
            && self.signoffs.is_empty()
            && self.enforce.is_block()
            && self.kinds.is_empty()
            && self.on_enter.is_empty()
            && !self.done
    }

    /// Does this gate ask for anything (beyond naming roles)?
    pub fn declares_rules(&self) -> bool {
        !self.requires.is_empty() || !self.warns.is_empty() || !self.signoffs.is_empty()
    }

    /// Does this gate apply to an item of `kind`?
    pub fn applies_to(&self, kind: Option<&str>) -> bool {
        let kind = kind
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or("feature");
        self.kinds.is_empty() || self.kinds.iter().any(|k| k.eq_ignore_ascii_case(kind))
    }
}

/// What a stage is for, in any process (FEAT-188). Each role belongs to at most one status; a
/// status may hold several (a small process can integrate and release in one step).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Design,
    Build,
    Integrate,
    Release,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Design, Role::Build, Role::Integrate, Role::Release];

    pub fn name(self) -> &'static str {
        match self {
            Role::Design => "design",
            Role::Build => "build",
            Role::Integrate => "integrate",
            Role::Release => "release",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        let s = s.trim();
        Role::ALL
            .into_iter()
            .find(|r| r.name().eq_ignore_ascii_case(s))
    }

    /// What the role means, in a sentence.
    pub fn means(self) -> &'static str {
        match self {
            Role::Design => "the item's design is written and agreed: how and where, before build",
            Role::Build => "code is written on the item's branch",
            Role::Integrate => "the item's work joins its line of integration and the docs fold in",
            Role::Release => "the work ships, and the item is finished",
        }
    }
}

impl serde::Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// A gate key that addresses a role rather than a status: `"@build"` (FEAT-188).
pub const ROLE_KEY_PREFIX: &str = "@";

/// The role a gate key addresses, for `"@<role>"` keys.
pub fn role_key(key: &str) -> Option<Role> {
    key.strip_prefix(ROLE_KEY_PREFIX).and_then(Role::parse)
}

/// Which status holds each role, and whether the workflow declared them or they were inferred
/// from what it already does (FEAT-188).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StageRoles {
    pub by_role: BTreeMap<Role, String>,
    pub declared: bool,
}

impl StageRoles {
    pub fn status(&self, role: Role) -> Option<&str> {
        self.by_role.get(&role).map(String::as_str)
    }

    /// The roles `status` holds, in role order.
    pub fn of(&self, status: &str) -> Vec<Role> {
        Role::ALL
            .into_iter()
            .filter(|r| self.status(*r) == Some(status))
            .collect()
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
    /// Name, description and version, on a saved process (FEAT-169).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessHeader>,
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
    /// A process that works in points says so (FEAT-122).
    #[serde(default, skip_serializing_if = "EstimateUnit::is_days")]
    pub estimate_unit: EstimateUnit,
    /// A process that works in sprints and releases switches them on (FEAT-122).
    #[serde(default, skip_serializing_if = "Cadence::is_off")]
    pub cadence: Cadence,
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
    ("scrum", include_str!("../presets/scrum.yaml")),
    ("agile", include_str!("../presets/agile.yaml")),
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

/// Whether `name` is a built-in process's.
pub fn is_builtin(name: &str) -> bool {
    let key = name.trim().to_ascii_lowercase();
    PRESETS.iter().any(|(n, _)| *n == key)
}

/// Check a name a process can be saved under (FEAT-169): lower case letters, digits and hyphens —
/// it is a file name on every platform and a word in a URL — and not a built-in process's, which
/// would make the name mean two things.
pub fn check_process_name(name: &str) -> crate::Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if !ok {
        return Err(crate::CoreError::Unsupported(format!(
            "'{name}' cannot name a process: use lower-case letters, digits and hyphens, such as \
             our-process"
        )));
    }
    if is_builtin(name) {
        return Err(crate::CoreError::Unsupported(format!(
            "'{name}' is a built-in process; save yours under another name"
        )));
    }
    Ok(())
}

/// The file to save as process `name` (FEAT-169), given what is saved under that name now. The
/// process must check clean. The version goes up by one when the content changed and stays when
/// it did not; the description is the one given, else the one already there. Returns the file and
/// whether anything changed.
pub fn versioned(
    name: &str,
    mut new: WorkflowFile,
    existing: Option<&WorkflowFile>,
    description: Option<String>,
) -> crate::Result<(WorkflowFile, bool)> {
    check_process_name(name)?;
    if let Some(problem) = new.problems().into_iter().next() {
        return Err(problem);
    }
    let old_header = existing.and_then(|e| e.process.clone());
    let same = existing.is_some_and(|e| e.content_rev() == new.content_rev());
    let version = match (&old_header, same) {
        (Some(h), true) => h.version.max(1),
        (Some(h), false) => h.version + 1,
        (None, true) => 1,
        (None, false) => existing.map_or(1, |_| 2),
    };
    let description = description
        .or_else(|| new.process.as_ref().map(|h| h.description.clone()))
        .filter(|d| !d.trim().is_empty())
        .or_else(|| old_header.as_ref().map(|h| h.description.clone()))
        .unwrap_or_default();
    new.process = Some(ProcessHeader {
        name: name.to_string(),
        description,
        version,
    });
    let changed = existing != Some(&new);
    Ok((new, changed))
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
            extra: Default::default(),
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
            estimate_unit: self.estimate_unit,
            cadence: self.cadence,
            process: None,
        };
        config.schema_version = config.required_schema_version();
        config
    }

    /// The workflow part of a project's config, as a file.
    pub fn from_config(config: &ProjectConfig) -> WorkflowFile {
        WorkflowFile {
            process: None,
            statuses: config.statuses.clone(),
            default_state: config.default_state.clone(),
            displayed_states: config.displayed_states.clone(),
            no_op_states: config.no_op_states.clone(),
            terminal_states: config.terminal_states.clone(),
            transitions: config.transitions.clone(),
            gates: config.gates.clone(),
            estimate_unit: config.estimate_unit,
            cadence: config.cadence.clone(),
        }
    }
}

/// Fill in what a workflow leaves out, and check it (FEAT-168). This is the one validator: the store
/// runs it before every workflow write, and `kanbanr process check` runs it alone, so a file that
/// checks clean is one the store accepts. `None` means "derive it": the default state is the first
/// status, and the displayed states are every status that is not a no-op. Returns the workflow as it
/// would be saved, and every problem with it, in the order the store reports them.
#[allow(clippy::too_many_arguments)]
pub fn check_workflow(
    statuses: Vec<String>,
    transitions: BTreeMap<String, Vec<String>>,
    default_state: Option<String>,
    displayed_states: Option<Vec<String>>,
    no_op_states: Option<Vec<String>>,
    terminal_states: Option<Vec<String>>,
    gates: BTreeMap<String, Gate>,
) -> (WorkflowFile, Vec<crate::CoreError>) {
    use crate::CoreError;
    let mut problems = Vec::new();
    if statuses.is_empty() {
        problems.push(CoreError::NoStatuses);
    }
    let known = |s: &str| statuses.iter().any(|x| x == s);
    let unknown = |s: &str, problems: &mut Vec<CoreError>| {
        if !known(s) {
            problems.push(CoreError::UnknownStatus(s.to_string()));
        }
    };
    for (from, tos) in &transitions {
        unknown(from, &mut problems);
        for to in tos {
            unknown(to, &mut problems);
        }
    }
    let default_state =
        default_state.unwrap_or_else(|| statuses.first().cloned().unwrap_or_default());
    if !statuses.is_empty() {
        unknown(&default_state, &mut problems);
    }
    let no_ops = no_op_states.unwrap_or_default();
    for s in &no_ops {
        unknown(s, &mut problems);
    }
    // No-op states are always non-displayed; displayed defaults to the active states.
    let displayed = displayed_states.unwrap_or_else(|| {
        statuses
            .iter()
            .filter(|s| !no_ops.contains(s))
            .cloned()
            .collect()
    });
    for s in &displayed {
        unknown(s, &mut problems);
        if no_ops.contains(s) {
            problems.push(CoreError::DisplayedNoOp(s.clone()));
        }
    }
    let terminals = terminal_states.unwrap_or_default();
    for s in &terminals {
        unknown(s, &mut problems);
    }
    problems.extend(gate_problems(&statuses, &no_ops, &terminals, &gates));
    let file = WorkflowFile {
        statuses,
        default_state,
        displayed_states: displayed,
        no_op_states: no_ops,
        terminal_states: terminals,
        transitions,
        gates,
        ..Default::default()
    };
    (file, problems)
}

/// What is wrong with a workflow's gates (FEAT-113): each must name a status of the workflow, name
/// its sign-offs, and ask only for real Zachman columns. A gate that can never match is a guardrail
/// that silently isn't there.
pub fn gate_problems(
    statuses: &[String],
    no_ops: &[String],
    terminals: &[String],
    gates: &BTreeMap<String, Gate>,
) -> Vec<crate::CoreError> {
    use crate::CoreError;
    use crate::readiness::{Check, Condition};
    let mut problems = Vec::new();
    for (status, gate) in gates {
        // A key addressed to a role (`@build`) is checked by `role_problems` (FEAT-188).
        if !status.starts_with(ROLE_KEY_PREFIX) && !statuses.iter().any(|s| s == status) {
            problems.push(CoreError::UnknownStatus(status.clone()));
        }
        if gate.signoffs.iter().any(|s| s.trim().is_empty()) {
            problems.push(CoreError::Unsupported(format!(
                "the gate on '{status}' has a sign-off with no name"
            )));
        }
        for condition in gate.requires.iter().chain(&gate.warns) {
            if *condition == Condition::Check(Check::Signoff) {
                problems.push(CoreError::Unsupported(format!(
                    "the gate on '{status}' lists `signoff` as a check; name it instead: \
                     `signoffs: [design-review]`"
                )));
            }
            if let Some(column) = condition.invalid_column() {
                problems.push(CoreError::Unsupported(format!(
                    "the gate on '{status}' names '{column}', which is not a Zachman column \
                     (what, how, where, when, who, why)"
                )));
            }
        }
    }
    problems.extend(role_problems(statuses, no_ops, terminals, gates));
    problems
}

/// Merge a role-addressed gate into the gate of the status that holds the role (FEAT-188): lists
/// are unioned, the status's own entries first; the status's purpose wins unless it has none.
/// Conflicting `enforce` or `kinds` are refused when the workflow is saved, so they never get here.
fn merge_gate(target: &mut Gate, from: &Gate) {
    fn union<T: Clone + PartialEq>(into: &mut Vec<T>, more: &[T]) {
        for x in more {
            if !into.contains(x) {
                into.push(x.clone());
            }
        }
    }
    union(&mut target.requires, &from.requires);
    union(&mut target.warns, &from.warns);
    union(&mut target.signoffs, &from.signoffs);
    union(&mut target.on_enter, &from.on_enter);
    if target.purpose.is_empty() {
        target.purpose = from.purpose.clone();
    }
    target.done |= from.done;
    if target.kinds.is_empty() {
        target.kinds = from.kinds.clone();
    }
    if target.enforce.is_block() {
        target.enforce = from.enforce;
    }
}

/// What is wrong with a workflow's stage roles (FEAT-188), in the order they are reported.
pub fn role_problems(
    statuses: &[String],
    no_ops: &[String],
    terminals: &[String],
    gates: &BTreeMap<String, Gate>,
) -> Vec<crate::CoreError> {
    use crate::CoreError;
    let known = "design, build, integrate, release";
    let mut problems = Vec::new();
    let mut holders: BTreeMap<Role, Vec<String>> = BTreeMap::new();
    for status in statuses {
        let Some(g) = gates.get(status) else { continue };
        for name in &g.roles {
            match Role::parse(name) {
                None => problems.push(CoreError::Unsupported(format!(
                    "'{name}' on '{status}' is not a stage role ({known})"
                ))),
                Some(r) => holders.entry(r).or_default().push(status.clone()),
            }
        }
    }
    for key in gates.keys().filter(|k| k.starts_with(ROLE_KEY_PREFIX)) {
        if role_key(key).is_none() {
            problems.push(CoreError::Unsupported(format!(
                "the gate '{key}' does not name a stage role ({known})"
            )));
        }
    }
    for (role, at) in &holders {
        if at.len() > 1 {
            problems.push(CoreError::Unsupported(format!(
                "the role '{}' is held by both '{}' and '{}'; give it to one",
                role.name(),
                at[0],
                at[1]
            )));
        }
    }
    for (role, at) in &holders {
        let status = &at[0];
        if no_ops.contains(status) {
            problems.push(CoreError::Unsupported(format!(
                "'{status}' is a no-op status, so it cannot hold the role '{}'",
                role.name()
            )));
        }
        if *role == Role::Release && !terminals.is_empty() && !terminals.contains(status) {
            problems.push(CoreError::Unsupported(format!(
                "the release role belongs on an end status, and '{status}' is not one"
            )));
        }
    }
    let declared = !holders.is_empty();
    if declared {
        for key in gates.keys() {
            if let Some(role) = role_key(key)
                && !holders.contains_key(&role)
            {
                problems.push(CoreError::Unsupported(format!(
                    "the gate '{key}' names a role no status holds"
                )));
            }
        }
        let build = holders.get(&Role::Build).map(|v| v[0].clone());
        for (key, g) in gates {
            if g.on_enter.contains(&Action::Branch)
                && !key.starts_with(ROLE_KEY_PREFIX)
                && build.as_deref() != Some(key.as_str())
            {
                problems.push(CoreError::Unsupported(format!(
                    "the branch is made where the build role is; '{key}' does not hold it"
                )));
            }
        }
    }
    for (key, g) in gates {
        let Some(role) = role_key(key) else { continue };
        let Some(status) = holders.get(&role).map(|v| &v[0]) else {
            continue;
        };
        if let Some(own) = gates.get(status) {
            if g.enforce != own.enforce && g.declares_rules() && own.declares_rules() {
                problems.push(CoreError::Unsupported(format!(
                    "the gate '{key}' and the gate on '{status}' disagree on enforce"
                )));
            }
            if !g.kinds.is_empty() && !own.kinds.is_empty() && g.kinds != own.kinds {
                problems.push(CoreError::Unsupported(format!(
                    "the gate '{key}' and the gate on '{status}' apply to different kinds"
                )));
            }
        }
    }
    problems
}

impl WorkflowFile {
    /// A hash of the process itself — everything but its header (FEAT-169). Two files that would
    /// govern work the same way have the same rev, whatever they are called.
    pub fn content_rev(&self) -> String {
        let mut bare = self.clone();
        bare.process = None;
        crate::hash::stable_hash(&serde_yaml::to_string(&bare).unwrap_or_default())
    }

    /// The version this file carries, 0 when it has no header.
    pub fn version(&self) -> u32 {
        self.process.as_ref().map_or(0, |h| h.version)
    }

    /// Where this file came from, for a project that applies it.
    pub fn source(&self, name: &str, library: Library) -> ProcessSource {
        ProcessSource {
            name: name.to_string(),
            library,
            version: self.version(),
            rev: self.content_rev(),
        }
    }

    /// Every problem the store would refuse this file for, as `--from-file` would send it (FEAT-168).
    /// Whether a status that would disappear still holds items depends on the project, so that is
    /// checked against one, by the caller.
    pub fn problems(&self) -> Vec<crate::CoreError> {
        check_workflow(
            self.statuses.clone(),
            self.transitions.clone(),
            Some(self.default_state.clone()),
            Some(self.displayed_states.clone()),
            Some(self.no_op_states.clone()),
            Some(self.terminal_states.clone()),
            self.gates.clone(),
        )
        .1
    }
}

/// The team's working agreement, rendered from the workflow's gates (FEAT-122): what each stage
/// is for and what entering it asks. Generated rather than written, so it can never say one thing
/// while the gates enforce another. Under Scrum, the Ready and Done entries are the Definition of
/// Ready and the Definition of Done.
pub fn working_agreement(project: &str, config: &ProjectConfig) -> String {
    use crate::readiness::Condition;
    let mut out = format!(
        "# Working agreement — {project}\n\n\
         _Generated from the workflow's gates by `kanbanr config workflow --write-agreement`. Change \
         the gates, not this page, and regenerate it._\n\n"
    );
    let text = |c: &Condition| match c {
        Condition::Check(check) => serde_json::to_value(check)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
            .unwrap_or_default(),
        Condition::Zachman { zachman } => format!("zachman: {}", zachman.join(", ")),
        Condition::SpecSections { spec_sections } => {
            format!("spec sections: {}", spec_sections.join(", "))
        }
    };
    let gates = config.effective_gates();
    let roles = config.roles();
    for status in &config.statuses {
        if config.is_no_op(status) {
            continue;
        }
        out.push_str(&format!("## {status}\n\n"));
        // What the stage is for in any process (FEAT-188), after its own purpose.
        let role_lines = |out: &mut String| {
            for role in roles.of(status) {
                out.push_str(&format!(
                    "_Role: {} — {}.{}_\n\n",
                    role.name(),
                    role.means(),
                    if roles.declared { "" } else { " (inferred)" }
                ));
            }
        };
        let Some(gate) = gates.get(status) else {
            role_lines(&mut out);
            out.push_str("Nothing is asked to enter.\n\n");
            continue;
        };
        if !gate.purpose.trim().is_empty() {
            out.push_str(&format!("{}\n\n", gate.purpose.trim()));
        }
        role_lines(&mut out);
        let mut asks: Vec<String> = gate.requires.iter().map(text).collect();
        asks.extend(gate.signoffs.iter().map(|s| format!("sign-off: {s}")));
        if asks.is_empty() {
            out.push_str("Nothing is asked to enter.\n");
        } else {
            let how = match gate.enforce {
                Enforce::Block => "To enter",
                Enforce::Warn => "Expected on entry (warned, not enforced)",
            };
            out.push_str(&format!("{how}:\n\n"));
            for ask in asks {
                out.push_str(&format!("- {ask}\n"));
            }
        }
        if !gate.warns.is_empty() {
            let warns: Vec<String> = gate.warns.iter().map(text).collect();
            out.push_str(&format!(
                "\nAlso checked, not enforced: {}\n",
                warns.join(", ")
            ));
        }
        if gate.done {
            out.push_str("\nReaching this stage finishes an item for the sprint burndown.\n");
        }
        if gate.on_enter.contains(&Action::Branch) {
            out.push_str("\nThe item's branch is made here.\n");
        }
        out.push('\n');
    }
    let c = &config.cadence;
    if c.sprints || c.releases {
        out.push_str("## Cadence\n\n");
        if c.sprints {
            out.push_str(&format!(
                "- Sprints of {} days.\n",
                c.sprint_length_days.unwrap_or(14)
            ));
        }
        if c.releases {
            out.push_str(&format!(
                "- A release {}.\n",
                match c.release.as_deref() {
                    Some("per_sprint") => "every sprint",
                    Some("every_n") => "every few sprints",
                    _ => "when it is ready",
                }
            ));
        }
        out.push_str(&format!(
            "- Estimates in {}.\n",
            if config.estimate_unit == EstimateUnit::Points {
                "story points"
            } else {
                "days"
            }
        ));
    }
    out
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
        let mut config = preset("scheduled")
            .expect("the built-in scheduled preset parses")
            .into_config(name);
        // The fixture the test suites are written against predates stage roles (FEAT-188): keep it
        // as it was, with no gates at all, so their meaning does not drift.
        config.gates.clear();
        config.schema_version = config.required_schema_version();
        config
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

    /// Does reaching `status` finish an item for the cadence views (FEAT-137)? An end status that is
    /// not a no-op does, as it always has; so does any stage whose gate says `done: true`.
    pub fn counts_as_done(&self, status: &str) -> bool {
        (crate::graph::is_terminal_status(self, status) && !self.is_no_op(status))
            || self.effective_gates().get(status).is_some_and(|g| g.done)
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
        } else if !self.declares_gates()
            || self.gates.keys().any(|k| k.starts_with(ROLE_KEY_PREFIX))
        {
            // Role labels alone, or a gate addressed to a role (FEAT-188): an older reader would
            // take the labels for declared gates (losing the built-in rule) or never match the
            // `@role` key — a guardrail silently gone either way, so it must refuse the board.
            4
        } else {
            3
        }
    }

    /// Does the workflow declare gates of its own (FEAT-188)? Gates that only label roles do not:
    /// such a workflow keeps the built-in rule, as one with no gates at all does.
    pub fn declares_gates(&self) -> bool {
        self.gates.values().any(|g| !g.is_role_label())
    }

    /// The roles declared on status-keyed gates, if any are.
    fn declared_roles(&self) -> Option<StageRoles> {
        let mut by_role = BTreeMap::new();
        for status in &self.statuses {
            if let Some(g) = self.gates.get(status) {
                for r in g.roles.iter().filter_map(|n| Role::parse(n)) {
                    by_role.entry(r).or_insert_with(|| status.clone());
                }
            }
        }
        (!by_role.is_empty()).then_some(StageRoles {
            by_role,
            declared: true,
        })
    }

    /// Which status holds each stage role (FEAT-188). Declared on the gates, or — when nothing
    /// declares them — inferred from what the workflow already does: build where the branch is
    /// made, release at the first end status. Design and integrate are never inferred, so nothing
    /// that keys on them acts on a board that did not opt in.
    pub fn roles(&self) -> StageRoles {
        if let Some(declared) = self.declared_roles() {
            return declared;
        }
        let gates = self.base_gates(None);
        let mut by_role = BTreeMap::new();
        if let Some(build) = self.statuses.iter().find(|s| {
            gates
                .get(*s)
                .is_some_and(|g| g.on_enter.contains(&Action::Branch))
        }) {
            by_role.insert(Role::Build, build.clone());
        }
        if let Some(release) = self
            .statuses
            .iter()
            .find(|s| crate::graph::is_terminal_status(self, s) && !self.is_no_op(s))
        {
            by_role.insert(Role::Release, release.clone());
        }
        StageRoles {
            by_role,
            declared: false,
        }
    }

    /// The status that holds `role`.
    pub fn role_status(&self, role: Role) -> Option<String> {
        self.roles().status(role).map(String::from)
    }

    /// Where `start` makes the item's branch: the build role's status.
    pub fn branch_status(&self) -> Option<String> {
        self.role_status(Role::Build)
    }

    /// The gates this workflow enforces: the declared ones, or — when none are declared — today's
    /// rules, synthesised so that no existing board changes behaviour (FEAT-113).
    ///
    /// The synthesis is exactly the old start gate: entering a status the board displays as work in
    /// flight (displayed, not the default backlog, not terminal, not a no-op) needs a definition
    /// and a current agreement. Parking, closing and dispositioning are never gated — you can
    /// always stop work (L-19). The first such status is where `start` creates the branch.
    pub fn effective_gates(&self) -> BTreeMap<String, Gate> {
        let declared = self.declared_roles();
        let mut gates = self.base_gates(declared.as_ref());
        // Role labels travel onto the gates (FEAT-188). A workflow that declares its own gates gives
        // every role stage one; a label-only workflow keeps exactly its built-in gates, so its
        // behaviour is unchanged, and its roles are read from `roles()`.
        let declares = self.declares_gates();
        for (key, g) in &self.gates {
            if g.roles.is_empty() || key.starts_with(ROLE_KEY_PREFIX) {
                continue;
            }
            if declares {
                gates.entry(key.clone()).or_default().roles = g.roles.clone();
            } else if let Some(gate) = gates.get_mut(key) {
                gate.roles = g.roles.clone();
            }
        }
        if let Some(build) = declared.as_ref().and_then(|r| r.status(Role::Build)) {
            let gate = gates.entry(build.to_string()).or_default();
            if !gate.on_enter.contains(&Action::Branch) {
                gate.on_enter.push(Action::Branch);
            }
        }
        // A gate addressed to a role joins the gate of the status that holds it.
        let holders = declared.unwrap_or_else(|| self.roles());
        for (key, g) in &self.gates {
            let Some(role) = role_key(key) else { continue };
            let Some(status) = holders.status(role) else {
                continue;
            };
            let target = gates.entry(status.to_string()).or_default();
            merge_gate(target, g);
        }
        gates
    }

    /// Step one of [`ProjectConfig::effective_gates`]: the declared status-keyed gates, or —
    /// when the workflow declares none of its own — today's rules, synthesised so that no existing
    /// board changes behaviour (FEAT-113).
    ///
    /// The synthesis is exactly the old start gate: entering a status the board displays as work in
    /// flight (displayed, not the default backlog, not terminal, not a no-op) needs a definition
    /// and a current agreement. Parking, closing and dispositioning are never gated — you can
    /// always stop work (L-19). The branch is made at the declared build role, else at the first
    /// such status.
    fn base_gates(&self, declared: Option<&StageRoles>) -> BTreeMap<String, Gate> {
        if self.declares_gates() {
            return self
                .gates
                .iter()
                .filter(|(k, _)| !k.starts_with(ROLE_KEY_PREFIX))
                .map(|(k, g)| (k.clone(), g.clone()))
                .collect();
        }
        use crate::readiness::{Check, Condition};
        let build = declared.and_then(|r| r.status(Role::Build));
        let mut gates = BTreeMap::new();
        let mut first = true;
        for status in self.displayed_states.iter().filter(|s| {
            **s != self.default_state
                && !crate::graph::is_terminal_status(self, s)
                && !self.is_no_op(s)
        }) {
            let branch = match build {
                Some(b) => b == status,
                None => first,
            };
            gates.insert(
                status.clone(),
                Gate {
                    requires: vec![
                        Condition::Check(Check::Definition),
                        Condition::Check(Check::Approved),
                    ],
                    on_enter: if branch { vec![Action::Branch] } else { vec![] },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(name: &str) -> ProjectConfig {
        preset(name).unwrap().into_config("p")
    }

    /// FEAT-188 R-1: each role resolves to one status; a status may hold several.
    #[test]
    fn roles_assigned_once() {
        let roles = config_of("default").roles();
        assert!(roles.declared);
        assert_eq!(roles.status(Role::Build), Some("In Progress"));
        assert_eq!(roles.of("Completed"), vec![Role::Integrate, Role::Release]);
        let togaf = config_of("togaf").roles();
        for role in Role::ALL {
            let at = togaf.status(role).unwrap();
            assert_eq!(togaf.of(at), vec![role], "{at} holds only {}", role.name());
        }
    }

    /// FEAT-188 R-2: every built-in process declares all four roles, the branch is made at build,
    /// release is an end status, and each preset still checks clean.
    #[test]
    fn presets_declare_roles() {
        for (name, _) in presets() {
            let file = preset(name).unwrap();
            assert!(file.problems().is_empty(), "{name}: {:?}", file.problems());
            let config = file.into_config("p");
            let roles = config.roles();
            assert!(roles.declared, "{name}");
            for role in Role::ALL {
                assert!(
                    roles.status(role).is_some(),
                    "{name} has no {}",
                    role.name()
                );
            }
            let gates = config.effective_gates();
            let branch: Vec<&String> = config
                .statuses
                .iter()
                .filter(|s| {
                    gates
                        .get(*s)
                        .is_some_and(|g| g.on_enter.contains(&Action::Branch))
                })
                .collect();
            assert_eq!(
                branch,
                vec![roles.status(Role::Build).unwrap()],
                "{name}: the branch is made at build, and only there"
            );
            assert!(
                config.is_terminal(roles.status(Role::Release).unwrap()),
                "{name}"
            );
        }
    }

    /// FEAT-188 R-4: one role on two statuses is refused, naming both.
    #[test]
    fn duplicate_role_refused() {
        let mut file = preset("pdca").unwrap();
        file.gates.get_mut("Check").unwrap().roles = vec!["build".into()];
        let problems: Vec<String> = file.problems().iter().map(|e| e.to_string()).collect();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("'build' is held by both 'Do' and 'Check'")),
            "{problems:?}"
        );
    }

    /// FEAT-188: a workflow that declares no roles gets build where the branch is made and release
    /// at the first end status, and its gates are exactly what they were.
    #[test]
    fn legacy_roles_follow_the_branch() {
        let config = ProjectConfig::default_for("p");
        let roles = config.roles();
        assert!(!roles.declared);
        assert_eq!(roles.status(Role::Build), Some("Scheduled"));
        assert_eq!(roles.status(Role::Release), Some("Completed"));
        assert_eq!(roles.status(Role::Design), None, "never inferred");
        assert_eq!(roles.status(Role::Integrate), None, "never inferred");
        assert_eq!(config.branch_status().as_deref(), Some("Scheduled"));
    }

    /// FEAT-188: role labels alone keep the built-in rule — In Progress still asks for a definition
    /// and an approval, and the branch is made there.
    #[test]
    fn role_labels_keep_the_builtin_rule() {
        let config = config_of("default");
        assert!(!config.declares_gates());
        let gates = config.effective_gates();
        let wip = gates.get("In Progress").unwrap();
        assert_eq!(wip.requires.len(), 2);
        assert!(wip.on_enter.contains(&Action::Branch));
        assert_eq!(wip.roles, vec!["build".to_string()]);
        assert!(
            !gates.contains_key("Planned"),
            "no gate where there was none"
        );
    }

    /// FEAT-188: every role problem is reported, in order.
    #[test]
    fn role_problems_reported() {
        let mut file = preset("togaf").unwrap();
        file.gates.get_mut("Vision").unwrap().roles = vec!["review".into()];
        file.gates.insert(
            "No Action".into(),
            Gate {
                roles: vec!["integrate".into()],
                ..Default::default()
            },
        );
        file.gates.get_mut("Migration").unwrap().roles = vec![];
        file.gates.get_mut("Operations").unwrap().roles = vec!["release".into()];
        file.gates.get_mut("Business Arch").unwrap().on_enter = vec![Action::Branch];
        file.gates.insert("@nothing".into(), Gate::default());
        file.gates.insert(
            "@build".into(),
            Gate {
                requires: vec![crate::readiness::Condition::Check(
                    crate::readiness::Check::TestsGreen,
                )],
                enforce: Enforce::Warn,
                ..Default::default()
            },
        );
        let problems: Vec<String> = file.problems().iter().map(|e| e.to_string()).collect();
        let has = |needle: &str| problems.iter().any(|p| p.contains(needle));
        assert!(
            has("'review' on 'Vision' is not a stage role"),
            "{problems:?}"
        );
        assert!(
            has("no-op status, so it cannot hold the role 'integrate'"),
            "{problems:?}"
        );
        assert!(has("'Business Arch' does not hold it"), "{problems:?}");
        assert!(has("'@nothing' does not name a stage role"), "{problems:?}");
        assert!(has("disagree on enforce"), "{problems:?}");
        // A release role off an end status.
        let mut f2 = preset("pdca").unwrap();
        f2.gates.get_mut("Act").unwrap().roles = vec![];
        f2.gates.get_mut("Check").unwrap().roles = vec!["integrate".into(), "release".into()];
        let p2: Vec<String> = f2.problems().iter().map(|e| e.to_string()).collect();
        assert!(
            p2.iter().any(|p| p.contains("'Check' is not one")),
            "{p2:?}"
        );
    }

    /// FEAT-188: labels only, or a role-addressed gate, need a reader that knows roles.
    #[test]
    fn schema_four_for_role_labels() {
        assert_eq!(config_of("default").required_schema_version(), 4);
        assert_eq!(
            config_of("togaf").required_schema_version(),
            3,
            "roles beside real gates"
        );
        let mut config = config_of("togaf");
        config.gates.insert(
            "@build".into(),
            Gate {
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        assert_eq!(config.required_schema_version(), 4);
        assert_eq!(
            ProjectConfig::default_for("p").required_schema_version(),
            BASE_SCHEMA_VERSION
        );
    }

    /// FEAT-188: the working agreement says what each stage is for.
    #[test]
    fn working_agreement_names_roles() {
        let text = working_agreement("p", &config_of("togaf"));
        let section = text.split("## Implementation").nth(1).unwrap();
        let section = section.split("\n## ").next().unwrap();
        assert!(
            section.contains("_Role: build — code is written on the item's branch._"),
            "{text}"
        );
        let legacy = working_agreement("p", &ProjectConfig::default_for("p"));
        assert!(
            legacy.contains("_Role: build — code is written on the item's branch. (inferred)_"),
            "{legacy}"
        );
    }

    /// FEAT-137: Scrum's Definition of Done is the Done column, so that is what finishes an item for
    /// a sprint; Released still does too, and a no-op disposition never does.
    #[test]
    fn the_scrum_preset_counts_done_for_the_burndown() {
        let scrum = preset("scrum").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = scrum.statuses.clone();
        config.terminal_states = scrum.terminal_states.clone();
        config.no_op_states = scrum.no_op_states.clone();
        config.gates = scrum.gates.clone();
        assert!(config.counts_as_done("Done"));
        assert!(config.counts_as_done("Released"));
        assert!(!config.counts_as_done("Testing"));
        assert!(!config.counts_as_done("Out-of-Scope"));
        assert!(
            working_agreement("shop", &config).contains("finishes an item for the sprint burndown")
        );
    }
}
