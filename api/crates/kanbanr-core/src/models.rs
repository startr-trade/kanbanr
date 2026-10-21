//! Core domain models. All entities serialize to/from YAML.

use serde::{Deserialize, Serialize};

/// The fields of a board file this version of kanbanr does not know (FEAT-151). See
/// `docs/src/project/stability.md`: reading a newer board is safe, and so is rewriting it.
pub type Extra = std::collections::BTreeMap<String, serde_json::Value>;

/// How [`Extra`] is described in the published API (FEAT-177): keys a newer kanbanr wrote, of any
/// shape. A YAML value always has a JSON form, so any JSON value is what a client can receive.
#[cfg(feature = "schema")]
pub type ExtraSchema = std::collections::BTreeMap<String, serde_json::Value>;

/// A status is a configurable label (e.g. "Planned"). Kept as a free string so the
/// workflow can be reconfigured per project without code changes.
pub type Status = String;

/// Tri-state for a feature's todo items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TaskState {
    NotStarted,
    InProgress,
    Completed,
}

impl TaskState {
    pub fn parse(s: &str) -> Option<TaskState> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "notstarted" | "todo" | "new" => Some(TaskState::NotStarted),
            "inprogress" | "wip" | "doing" => Some(TaskState::InProgress),
            "completed" | "done" | "complete" => Some(TaskState::Completed),
            _ => None,
        }
    }
}

/// A todo item belonging to a todo-list. `key` is short and unique within its todo-list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Task {
    pub key: String,
    pub text: String,
    pub state: TaskState,
    /// When the task's state last changed (RFC3339; FEAT-214): what agent drift is measured from.
    /// Absent on tasks no kanbanr since FEAT-214 has changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// A persistent todo-list attached to a feature item. A feature (acting as an epic) can hold
/// many of these — e.g. one added per working session — so the work survives across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TodoList {
    pub code: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<Task>,
    pub created_at: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl TodoList {
    /// A list is fully completed when it has at least one task and all are Completed.
    pub fn fully_completed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.state == TaskState::Completed)
    }
    pub fn done_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.state == TaskState::Completed)
            .count()
    }
}

/// A feature item: the unit Claude develops. Holds the spec and its todo list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FeatureItem {
    pub code: String,
    pub title: String,
    #[serde(default)]
    pub specification: String,
    pub status: Status,
    /// The milestone this feature belongs to (required).
    pub milestone: String,
    /// Work kind (feature / chore / bug / refactor / docs / recurring …); free text.
    #[serde(default)]
    pub kind: Option<String>,
    /// Priority (low / medium / high …); free text.
    #[serde(default)]
    pub priority: Option<String>,
    /// Optional planned start date (an ISO date, e.g. "2026-07-01"). Used by scheduling/Gantt.
    #[serde(default)]
    pub start: Option<String>,
    /// Optional due date (e.g. an ISO date).
    #[serde(default)]
    pub due: Option<String>,
    /// Optional estimated effort, in days (used as the task duration for scheduling/Gantt;
    /// defaults to 1 day when absent). Kept as a float so half-days etc. are expressible.
    #[serde(default)]
    pub estimate_days: Option<f64>,
    /// Optional estimate in story points (FEAT-121). Which of the two a project plans in is its
    /// `estimate_unit`; absent stays absent on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<f64>,
    /// The sprint this item is planned into (FEAT-119), for projects that use sprints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint: Option<String>,
    /// The release this item is planned into (FEAT-120), for projects that use releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// Optional assignee (the person/agent owning this feature); free text.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Optional owning team; free text.
    #[serde(default)]
    pub team: Option<String>,
    /// Free-form labels/tags.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Other feature codes this one is blocked by (a cross-feature DAG; cycles are rejected).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Persistent todo-lists (newest-first ordering is applied by callers when displaying).
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    /// Where this feature was imported from, if it was (FEAT-042).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The external issue this feature is mirrored to, if any (FEAT-043).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueLink>,
    /// Why this item exists, what must be true, and how it is verified (FEAT-047).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<FeatureDefinition>,
    /// For a defect: what it cost and where it came from (FEAT-053).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defect: Option<Defect>,
    /// The item this was sliced out of (FEAT-054). A wave grows for three reasons — a defect, a
    /// split, or newly discovered work — and only this one leaves no other trace, so a retro that
    /// tried to infer it would be guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_from: Option<String>,
    /// Every status change, appended (FEAT-053). Cycle time, time-in-status, WIP aging and rework
    /// are all derived from this; without it the board records where work IS but never how it got
    /// there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Transition>,
    pub created_at: String,
    pub updated_at: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// Where an imported feature came from (FEAT-042). This is a record of history, not a live
/// pointer: the original text is preserved in the feature's spec, so nothing depends on the
/// source still existing (a deleted file, rewritten git history, a removed issue).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Source {
    /// Tracker kind: `file`, `github`, `gitlab`, `jira`, `linear`, …
    pub system: String,
    /// Location in that tracker, for humans: `TODO.md:14`, `owner/repo#123`, `PROJ-45`.
    #[serde(default, rename = "ref")]
    pub reference: String,
    /// Project repo commit at import time (file sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// When it was imported (RFC 3339); stamped by kanbanr.
    #[serde(default)]
    pub imported_at: String,
    /// Stable identity used to skip re-imports; derived by kanbanr when not given.
    #[serde(default)]
    pub key: String,
    /// When the source was found to be gone (set by `kanbanr sources --write`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_since: Option<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl Source {
    /// The re-import identity. Trackers with stable ids (anything but `file`) use
    /// `<system>:<ref>`; files use a hash of the normalized title, so a moved, renumbered or
    /// deleted file doesn't cause a re-import.
    pub fn derive_key(&self, title: &str) -> String {
        let system = self.system.trim().to_lowercase();
        let reference = self.reference.trim();
        if system != "file" && !reference.is_empty() {
            return format!("{system}:{reference}");
        }
        let normalized = title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        format!("{system}:{}", crate::hash::stable_hash(&normalized))
    }
}

/// Why a work item exists, what must be true, and how that is verified (FEAT-047).
///
/// The prose case *for* the work stays in the item's specification markdown — this block holds the
/// short, checkable parts: one-line answers, ids that link to the charter, and requirements each
/// carrying the tests that verify them. Keeping it one-line-per-field is deliberate: a structured
/// copy of the spec would rot into a second, disagreeing version of the same argument.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FeatureDefinition {
    /// One sentence: what, for whom, and why.
    #[serde(default)]
    pub statement: String,
    /// Charter goal ids this item serves — the link that makes "why" checkable rather than prose.
    #[serde(default)]
    pub goals: Vec<String>,
    /// The six completeness dimensions, one line each.
    #[serde(default)]
    pub zachman: Zachman,
    /// Optional pointer into the board's doc tree (a design note), rather than an inline diagram.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub design_doc: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<Requirement>,
    /// Who agreed to this definition, when, and what exactly they agreed to (FEAT-048).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    /// Every agreement and withdrawal, oldest first (FEAT-069). `approval` is the current one, or
    /// absent when the last verdict was a withdrawal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<ApprovalEvent>,
    /// A recorded reason work started without approval. An escape hatch that leaves a trace beats
    /// one that is silent — an unrecorded bypass just teaches itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_unapproved: String,
    /// A recorded reason this item is exempt from gap reporting. An escape hatch that is visible
    /// beats one that is habitual (`--no-verify` teaches itself).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub exempt: String,
    /// Named agreements a stage can require (FEAT-114): "design review held", "release approved".
    /// Appended, never overwritten — each pinned to the definition it covered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signoffs: Vec<Signoff>,
    /// Board docs this item changes (FEAT-180), e.g. "architecture/data.md". Declared at the design
    /// stage; `docs_updated` checks each was changed before the item moves on. Declared last, so a
    /// board where it is the only key an older binary does not know reads back byte for byte.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects_docs: Vec<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// A named agreement, recorded by a person against the definition as it stood (FEAT-114).
///
/// Like an approval it carries the definition's `rev`, so changing the definition afterwards lapses
/// it: a design review of a different design is not a review of this one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Signoff {
    /// The name a gate asks for (`design-review`).
    pub id: String,
    pub by: String,
    pub at: String,
    /// The definition content it covered.
    pub rev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// A board doc holding the record — minutes, a checklist.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// The status the item was in when it was given.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// `given`, or `withdrawn` (FEAT-182). A withdrawal is its own entry, appended like any other,
    /// with an empty `rev`: an older kanbanr, which does not know this field, then reads the
    /// latest entry as lapsed and stops counting the sign-off too.
    #[serde(default, skip_serializing_if = "SignoffVerdict::is_given")]
    pub verdict: SignoffVerdict,
    /// Why it was withdrawn. Giving a sign-off needs no reason; taking one back does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// Whether a sign-off entry gives the sign-off or takes it back (FEAT-182).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SignoffVerdict {
    #[default]
    Given,
    Withdrawn,
}

impl SignoffVerdict {
    pub fn is_given(&self) -> bool {
        *self == Self::Given
    }
}

/// Agreement to a definition, pinned to its content (FEAT-048).
///
/// `rev` is a hash of the definition as approved. If the definition changes afterwards the hash no
/// longer matches and the approval has **lapsed** — so scope cannot drift silently past a "yes",
/// which is exactly how work gets built and then rejected.
/// One agreement, or one withdrawal of it (FEAT-069). Append-only: an approval that was given and
/// later taken back is more informative than no record at all, and the charter constraint says raw
/// data is never discarded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ApprovalEvent {
    pub at: String,
    pub by: String,
    /// `approved` | `ratified` | `withdrawn`.
    ///
    /// `ratified` is agreement given **after** the work was built, under a recorded
    /// `--unapproved` start (FEAT-080). It is a separate verdict from `approved` because the
    /// whole point of the gate is to distinguish "we agreed, then built" from "we built, then
    /// agreed" — and with one verdict the record could not tell them apart. Twenty-nine items on
    /// this board were reconciled that way before the distinction existed.
    pub verdict: String,
    /// Why it was withdrawn. An agreement needs no reason; taking one back does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// The definition content this verdict was about.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rev: String,
    /// The status the item was in when the verdict was given (FEAT-114), so a definition that grows
    /// stage by stage leaves a readable trail: approved at Vision, again at System Design.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Approval {
    pub by: String,
    pub at: String,
    pub rev: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// Where an item stands against its approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// No definition, or no approval recorded.
    Missing,
    /// Approved, and the definition has not changed since.
    Current,
    /// Approved once, but the definition changed afterwards.
    Lapsed,
    /// Agreed to **after** the work was built. The gate was bypassed with a recorded reason and
    /// the question was answered later — which is a real resolution, and not the same thing as
    /// prior agreement, so it reads differently wherever it is shown.
    Ratified,
}

impl FeatureDefinition {
    /// A stable hash of what was agreed to, so an approval can be pinned to it.
    ///
    /// Excluded: the approval itself (recording it would otherwise invalidate it), and each test's
    /// **state** and `checked_rev`. Those are evidence, not scope — a test going from planned to
    /// green means the work is progressing as agreed, and lapsing the approval for it would train
    /// people to re-approve reflexively, which is how a gate becomes a rubber stamp. Adding,
    /// removing or renaming a test DOES change the hash: what is verified is part of the deal.
    ///
    /// The hash is of the definition's canonical JSON (FEAT-187), not of its YAML: a revision is
    /// about what was agreed, and must not lapse because a file was written differently.
    pub fn content_rev(&self) -> String {
        let mut bare = self.clone();
        bare.approval = None;
        bare.approvals = Vec::new();
        bare.started_unapproved = String::new();
        // Sign-offs are verdicts about the content, not part of it (FEAT-114).
        bare.signoffs = Vec::new();
        // Which docs the work touches is design, written after the definition was agreed: naming
        // them does not reopen the agreement (FEAT-180, Q1).
        bare.affects_docs = Vec::new();
        for requirement in &mut bare.requirements {
            for test in &mut requirement.tests {
                test.state = TestState::default();
                test.checked_rev = String::new();
            }
        }
        crate::hash::stable_hash(&crate::hash::canonical_json(&bare))
    }

    /// Whether this definition is approved as it currently stands.
    pub fn approval_state(&self) -> ApprovalState {
        match &self.approval {
            None => ApprovalState::Missing,
            Some(a) if a.rev != self.content_rev() => ApprovalState::Lapsed,
            // Current, but say WHICH kind of current: the last verdict decides.
            Some(_) if self.was_ratified() => ApprovalState::Ratified,
            Some(_) => ApprovalState::Current,
        }
    }

    /// The latest entry for sign-off `id`, given or withdrawn (FEAT-182).
    pub fn signoff_latest(&self, id: &str) -> Option<&Signoff> {
        self.signoffs.iter().rev().find(|s| s.id == id)
    }

    /// Is sign-off `id` recorded against the definition as it now stands? The latest entry counts,
    /// and only if it gives the sign-off: a withdrawal after it takes it back (FEAT-182).
    pub fn signoff_current(&self, id: &str) -> Option<&Signoff> {
        let rev = self.content_rev();
        self.signoff_latest(id)
            .filter(|s| s.verdict.is_given() && s.rev == rev)
    }

    /// Was the standing agreement given after the fact? Read from the log's last verdict rather
    /// than stored on the approval, so the two can never disagree.
    pub fn was_ratified(&self) -> bool {
        self.approvals
            .iter()
            .rev()
            .find(|e| e.verdict != "withdrawn")
            .is_some_and(|e| e.verdict == "ratified")
    }
}

/// The six completeness dimensions. Answers are one line; anything longer belongs in the spec.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Zachman {
    #[serde(default)]
    pub what: String,
    #[serde(default)]
    pub how: String,
    #[serde(default, rename = "where")]
    pub where_: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub who: String,
    #[serde(default)]
    pub why: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl Zachman {
    /// The six columns in order — the only place `where_` is spelled out.
    pub fn columns(&self) -> [(&'static str, &str); 6] {
        [
            ("What", self.what.as_str()),
            ("How", self.how.as_str()),
            ("Where", self.where_.as_str()),
            ("When", self.when.as_str()),
            ("Who", self.who.as_str()),
            ("Why", self.why.as_str()),
        ]
    }

    /// One column by name, case-insensitively. Blank when the name is not a column.
    pub fn column(&self, name: &str) -> &str {
        self.columns()
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
            .unwrap_or("")
    }

    /// The columns left blank. Gaps are DERIVED here and rendered as `[MISSING: …]`; storing the
    /// marker would let the data disagree with the check.
    pub fn missing(&self) -> Vec<&'static str> {
        self.columns()
            .iter()
            .filter(|(_, v)| v.trim().is_empty())
            .map(|(k, _)| *k)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.missing().len() == 6
    }
}

/// Functional behaviour, or a quality the system must exhibit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum RequirementKind {
    #[default]
    Functional,
    Nfr,
}

/// One requirement: a single behaviour, stated so it can be tested.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Requirement {
    /// `R-1`, `R-2`, … assigned when left blank.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: RequirementKind,
    /// The requirement itself, ideally in EARS form. Stored verbatim: the EARS pattern is derived
    /// when rendering or checking, never persisted, so stored text cannot disagree with the
    /// classifier.
    pub text: String,
    /// ISO/IEC 25010 characteristics, for NFRs.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "iso25010")]
    pub iso: Vec<String>,
    /// The quality attribute scenario behind an NFR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<QualityScenario>,
    /// How this requirement is verified. A requirement with no test is not ready.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestRef>,
    /// For a defect: the requirement it violates (`FEAT-046/R-2`), or blank when the defect
    /// reveals that no requirement covered the case — which is itself the finding.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub violates: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// Stimulus / environment / response / measure — what turns "it should be fast" into something
/// checkable. The `measure` names the test or benchmark that checks it; a number with nothing
/// behind it is reported as an unsupported claim.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QualityScenario {
    #[serde(default)]
    pub stimulus: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub measure: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// A test that verifies a requirement, and where it currently stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TestRef {
    /// The test's name as it appears in the codebase, so it can be checked for existence.
    pub name: String,
    /// unit | integration | e2e | manual | property — free text, like `kind` elsewhere.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default)]
    pub state: TestState,
    /// The project revision at which this was last observed green. A green older than HEAD is
    /// stale rather than trustworthy.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked_rev: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// The TDD lifecycle of a test: written but not yet run, failing, passing.
///
/// Serialized lowercase (`state: green`) because that is how it is written in definition files and
/// on the command line; the aliases keep hand-written YAML forgiving.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum TestState {
    #[default]
    #[serde(alias = "todo", alias = "notwritten")]
    Planned,
    #[serde(alias = "failing", alias = "fail")]
    Red,
    #[serde(alias = "passing", alias = "pass", alias = "passed")]
    Green,
}

impl TestState {
    /// Lenient parsing, like [`TaskState::parse`], so both TDD and plain wording work.
    pub fn parse(s: &str) -> Option<TestState> {
        match s.trim().to_ascii_lowercase().as_str() {
            "planned" | "todo" | "notwritten" | "not_started" => Some(TestState::Planned),
            "red" | "failing" | "fail" => Some(TestState::Red),
            "green" | "passing" | "pass" | "passed" => Some(TestState::Green),
            _ => None,
        }
    }
}

/// One status change. Append-only: the board's memory of how work actually moved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Transition {
    pub at: String,
    pub from: String,
    pub to: String,
    /// Why a gate was passed over, when it was (FEAT-113). Absent on an ordinary move.
    #[serde(default, rename = "override", skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// What a defect cost and where it came from (FEAT-053).
///
/// `escaped` is the field that earns its keep: a defect found after the work was called done is a
/// different animal from one caught during it, and the ratio between them is the one quality
/// number a board can honestly produce.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Defect {
    /// Free text, like `kind` and `priority`: low | medium | high | critical.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
    /// The item or commit that introduced it. A defect caused by a fix points at that fix, which
    /// is how fix-induced chains become visible.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub introduced_by: String,
    /// Where it was found: a status, an environment, or "production".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub found_in: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root_cause: String,
    /// Found after the work was called done.
    #[serde(default)]
    pub escaped: bool,
    /// The commit or test that proves it is fixed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fixed_by: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// Changes to a defect record, a field at a time (FEAT-053): a field given replaces what is
/// recorded (an empty one clears it); one not given is kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefectChange {
    pub severity: Option<String>,
    pub introduced_by: Option<String>,
    pub found_in: Option<String>,
    pub root_cause: Option<String>,
    pub fixed_by: Option<String>,
    /// Marks it escaped; an escape once recorded is never unrecorded here.
    pub escaped: bool,
}

impl Defect {
    /// This record with `change` applied (FEAT-176), so one flag at a time is enough.
    pub fn merge(mut self, change: &DefectChange) -> Defect {
        for (field, value) in [
            (&mut self.severity, &change.severity),
            (&mut self.introduced_by, &change.introduced_by),
            (&mut self.found_in, &change.found_in),
            (&mut self.root_cause, &change.root_cause),
            (&mut self.fixed_by, &change.fixed_by),
        ] {
            if let Some(v) = value {
                *field = v.clone();
            }
        }
        self.escaped = self.escaped || change.escaped;
        self
    }
}

/// A link from a feature to the external issue it is mirrored to (FEAT-043).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IssueLink {
    /// Tracker kind; currently `github`.
    pub system: String,
    /// `owner/repo`.
    pub repo: String,
    pub number: u64,
    #[serde(default)]
    pub url: String,
    /// Hash of the issue content kanbanr last pushed; `None` until the first push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl FeatureItem {
    /// When the item entered its current status: the last move into it, else its creation
    /// (FEAT-180). A board edited by hand can carry a status no move led to; creation is then the
    /// honest answer, since nothing says when it got there.
    pub fn entered_status_at(&self) -> &str {
        self.history
            .iter()
            .rev()
            .find(|t| t.to == self.status)
            .map(|t| t.at.as_str())
            .unwrap_or(self.created_at.as_str())
    }

    /// Iterate over every task across all todo-lists.
    pub fn all_tasks(&self) -> impl Iterator<Item = &Task> {
        self.todo_lists.iter().flat_map(|l| l.tasks.iter())
    }

    /// A feature is complete when it has at least one task and all tasks (across all
    /// todo-lists) are Completed.
    pub fn all_tasks_completed(&self) -> bool {
        let mut any = false;
        for t in self.all_tasks() {
            any = true;
            if t.state != TaskState::Completed {
                return false;
            }
        }
        any
    }

    pub fn done_count(&self) -> usize {
        self.all_tasks()
            .filter(|t| t.state == TaskState::Completed)
            .count()
    }
    pub fn task_count(&self) -> usize {
        self.all_tasks().count()
    }

    pub fn todo_list(&self, code: &str) -> Option<&TodoList> {
        self.todo_lists.iter().find(|l| l.code == code)
    }

    /// Split into the on-disk metadata (yaml) and the specification (separate .md file).
    pub fn meta(&self) -> FeatureMeta {
        FeatureMeta {
            code: self.code.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
            milestone: self.milestone.clone(),
            kind: self.kind.clone(),
            priority: self.priority.clone(),
            start: self.start.clone(),
            due: self.due.clone(),
            estimate_days: self.estimate_days,
            points: self.points,
            sprint: self.sprint.clone(),
            release: self.release.clone(),
            assignee: self.assignee.clone(),
            team: self.team.clone(),
            labels: self.labels.clone(),
            depends_on: self.depends_on.clone(),
            todo_lists: self.todo_lists.clone(),
            source: self.source.clone(),
            issue: self.issue.clone(),
            definition: self.definition.clone(),
            defect: self.defect.clone(),
            split_from: self.split_from.clone(),
            history: self.history.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            extra: self.extra.clone(),
        }
    }

    /// Reassemble from on-disk metadata and the spec markdown loaded separately.
    pub fn from_meta(meta: FeatureMeta, specification: String) -> FeatureItem {
        FeatureItem {
            code: meta.code,
            title: meta.title,
            specification,
            status: meta.status,
            milestone: meta.milestone,
            kind: meta.kind,
            priority: meta.priority,
            start: meta.start,
            due: meta.due,
            estimate_days: meta.estimate_days,
            points: meta.points,
            sprint: meta.sprint,
            release: meta.release,
            assignee: meta.assignee,
            team: meta.team,
            labels: meta.labels,
            depends_on: meta.depends_on,
            todo_lists: meta.todo_lists,
            source: meta.source,
            issue: meta.issue,
            definition: meta.definition,
            defect: meta.defect,
            split_from: meta.split_from,
            history: meta.history,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
            extra: meta.extra,
        }
    }
}

/// On-disk metadata for a feature (the `.yaml` file). The specification lives in a separate
/// `feature-specs/<code>.md` file and is not duplicated here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureMeta {
    pub code: String,
    pub title: String,
    pub status: Status,
    /// The milestone this feature belongs to (required).
    pub milestone: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub estimate_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<FeatureDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defect: Option<Defect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_from: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Transition>,
    pub created_at: String,
    pub updated_at: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,
}

/// A compact, spec-free per-feature row stored in the per-project `index.yaml` cache (FEAT-033).
/// It carries everything board/list/graph/rollup callers need (status, milestone, attributes, and
/// the todo-list done/total counts) WITHOUT any specification body, so a project's whole feature
/// set can be read from one small file. It is a derivable cache: the status-folder yaml/md files
/// remain the source of truth, and the index is rebuilt from them by `Store::rebuild_index`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub code: String,
    pub status: Status,
    pub milestone: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub estimate_days: Option<f64>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Completed task count across all of the feature's todo-lists.
    #[serde(default)]
    pub done: usize,
    /// Total task count across all of the feature's todo-lists.
    #[serde(default)]
    pub total: usize,
}

impl IndexEntry {
    /// Build an index row from a (fully- or metadata-only-) loaded feature. The spec body is
    /// intentionally ignored — only the cheap, listable attributes are captured.
    pub fn from_feature(f: &FeatureItem) -> IndexEntry {
        IndexEntry {
            code: f.code.clone(),
            status: f.status.clone(),
            milestone: f.milestone.clone(),
            kind: f.kind.clone(),
            priority: f.priority.clone(),
            start: f.start.clone(),
            due: f.due.clone(),
            estimate_days: f.estimate_days,
            assignee: f.assignee.clone(),
            team: f.team.clone(),
            labels: f.labels.clone(),
            depends_on: f.depends_on.clone(),
            done: f.done_count(),
            total: f.task_count(),
        }
    }
}

/// A milestone groups feature items and may depend on other milestones (the DAG).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Milestone {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

// NOTE: There is no `Schedule` entity. A "schedule" is a *derived view*: for a given feature
// status, the milestones that contain feature items in that status (ordered by their dependency
// DAG). The UI computes this from features + milestones; nothing is stored on disk.

/// A starting skeleton for a definition, shaped to the kind of work (FEAT-047). Every kind states
/// why it exists and how it is verified; what differs is the FORM a requirement takes — a feature
/// asserts new behaviour, a defect names the requirement it violates, a chore asserts an invariant.
/// Printing only the fields that apply keeps an author from staring at a form full of blanks.
pub fn definition_template(kind: &str) -> String {
    let head = "\
# Why this item exists, what must be true, and how that is verified.
# Fill what you know; leave the rest BLANK so `kanbanr doctor` can flag it — never invent.
statement: \"<capability> for <whom> so that <why>\"
goals: [G-1]          # charter goal ids this serves (kanbanr charter show)
zachman:
  what:  \"\"            # data, functions, rules
  how:   \"\"            # approach
  where: \"\"            # component or service
  when:  \"\"            # trigger, frequency, timing
  who:   \"\"            # stakeholder role
  why:   \"\"            # the problem or goal served
affects_docs: []      # board docs the design changes, e.g. [architecture/data.md]: named at the
                      # design stage; a `docs_updated` gate checks each changed before moving on
";
    let requirements = match kind.trim().to_ascii_lowercase().as_str() {
        "defect" | "bug" => {
            "\
requirements:
  # A defect violates a requirement. If none covers the case, THAT is the finding: add the
  # missing requirement here instead of leaving `violates` blank.
  - kind: functional
    violates: FEAT-000/R-0
    text: \"WHEN <the trigger>, THE SYSTEM SHALL <the behaviour that was wrong>\"
    tests:
      - name: \"<test that reproduces it>\"
        kind: unit
        state: red      # red first, then green once fixed
"
        }
        "chore" | "refactor" => {
            "\
requirements:
  # No new behaviour: the requirement is what must NOT change.
  - kind: functional
    text: \"THE SYSTEM SHALL continue to <invariant this work must preserve>\"
    tests:
      - name: \"<existing suite that proves it>\"
        kind: integration
        state: green
"
        }
        "docs" => {
            "\
requirements:
  # The requirement is the contract the documentation must match.
  - kind: functional
    text: \"THE SYSTEM SHALL document <the contract> as it actually behaves\"
    tests:
      - name: \"<doc check or doctor rule>\"
        kind: manual
        state: planned
"
        }
        "recurring" => {
            "\
requirements:
  # Standing work: one statement, verified per occurrence by its checklist.
  - kind: functional
    text: \"THE SYSTEM SHALL <the standing obligation>\"
    tests:
      - name: \"<per-occurrence check>\"
        kind: manual
        state: planned
"
        }
        _ => {
            "\
requirements:
  - kind: functional
    text: \"WHEN <trigger>, THE SYSTEM SHALL <response>\"
    tests:
      - name: \"<test that verifies it>\"
        kind: unit
        state: planned
  # A quality requirement needs a tag AND a measure that names what checks it — a number with
  # nothing behind it is an unsupported claim, so leave it out unless it is real.
  - kind: nfr
    text: \"WHERE <condition>, THE SYSTEM SHALL <quality behaviour>\"
    iso25010: [Reliability]
    scenario:
      stimulus: \"\"
      environment: \"\"
      response: \"\"
      measure: \"<number, and the test or benchmark that checks it>\"
    tests:
      - name: \"<test that verifies it>\"
        kind: integration
        state: planned
"
        }
    };
    format!("{head}{requirements}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-187 R-3: one definition, written two ways — quotes, flow or block style, key order,
    /// indentation, a test's state — has one revision.
    #[test]
    fn content_rev_is_writer_independent() {
        let block: FeatureDefinition = crate::yaml::from_str(concat!(
            "statement: 'Do it: well'\n",
            "goals:\n- G-1\n",
            "zachman:\n  what: x\n  why: y\n",
            "requirements:\n",
            "- id: R-1\n",
            "  kind: functional\n",
            "  text: |-\n    THE SYSTEM SHALL\n    do it.\n",
            "  tests:\n  - name: does_it\n    kind: unit\n",
            "  x_newer: {b: 2, a: [1, two]}\n",
        ))
        .unwrap();
        let flow: FeatureDefinition = crate::yaml::from_str(concat!(
            r#"{"requirements": [{"x_newer": {"a": [1, "two"], "b": 2},"#,
            r#" "tests": [{"kind": "unit", "name": "does_it", "state": "green"}],"#,
            r#" "text": "THE SYSTEM SHALL\ndo it.", "kind": "functional", "id": "R-1"}],"#,
            r#" "zachman": {"why": "y", "what": "x"}, "goals": ["G-1"],"#,
            r#" "statement": "Do it: well"}"#,
        ))
        .unwrap();
        assert_eq!(block.content_rev(), flow.content_rev());
        // Written back by this build and read again: the same revision.
        let again: FeatureDefinition =
            crate::yaml::from_str(&crate::yaml::to_string(&flow).unwrap()).unwrap();
        assert_eq!(again.content_rev(), block.content_rev());
        // And the content still decides it.
        let mut changed = block.clone();
        changed.statement.push('!');
        assert_ne!(changed.content_rev(), block.content_rev());
    }
}
