//! The Review page's decision groups (FTR-038): every decision waiting for the person, grouped by
//! the decision it needs, so one confirmation applies one decision to the rows the person chose.
//!
//! [`summary`] counts each group, [`slice`] lists one group's rows and [`evidence`] reads one row
//! in full. They only read the board (ADR-0005). [`ops`] turns one group decision into the single
//! routes the page posts today, one verdict per row, which the write path records in one commit.

use crate::Store;
use crate::adr::Adr;
use crate::decisions::{DelegatedDecision, DelegationState, DelegationView, Mode};
use crate::error::{CoreError, Result};
use crate::manual::ManualWaiting;
use crate::prepared::PreparedView;
use crate::review::{ApprovalKind, ReviewEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Approve a definition never approved, lapsed, or added to.
const APPROVE: &str = "approve";

/// The pill an Approve row carries when its approval lapsed through an edit to its tests alone
/// (DEF-111). The web page matches the same text (`TESTS_ONLY_PILL` in web/src/reviewGroups.ts).
pub const TESTS_ONLY_PILL: &str = "tests only";
/// Ratify work done under a bypass or an override Claude made.
const RATIFY: &str = "ratify";
/// Which release a ready item ships in (FEAT-185).
const WHICH_RELEASE: &str = "which-release";
/// Manual checks with a result Claude prepared.
const MANUAL: &str = "manual";
/// Delegations Claude proposed (FTR-028).
const DELEGATION: &str = "delegation";
/// Decisions Claude took under a delegation, to keep or undo.
const DELEGATED: &str = "delegated";
/// Proposed ADRs.
const ADR: &str = "adr";
/// The system baseline's acceptance (FTR-006): one project-level row, first in the order.
const SYSTEM_BASELINE: &str = "system-baseline";
/// The bulk recode of codes to their kinds' prefixes, prepared for the person to confirm
/// (FTR-056): one row, last in the order.
const RECODE: &str = "recode";
/// The group key of the sign-off named `release`.
const RELEASE_SIGNOFF: &str = "release-signoff";
/// The prefix of a sign-off's group key other than the named ones.
const SIGNOFF_PREFIX: &str = "signoff-";

/// What a conversation-mode project's item rows say (ADR-0019).
const IN_CONVERSATION: &str = "Decisions are taken in the conversation in this project";
/// What a manual check with nothing prepared says: the person's own observation is never taken in
/// bulk.
const DO_THE_CHECK: &str = "Do the check, then press Passed or Failed in the row.";

/// One decision group, counted: a tab on the Review page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GroupCount {
    pub key: String,
    pub label: String,
    /// The rows in the group.
    pub count: usize,
    /// The group button's verb: "Approve", "Ratify", "Sign off", "Plan into", "Confirm", "Grant",
    /// "Keep" or "Accept".
    pub action: String,
    /// The rows that can be decided now.
    pub ready: usize,
    /// The rows with an open prepared record.
    pub prepared: usize,
    /// The rows Claude holds under a delegation in force: listed, not in `count` (DEF-104).
    pub held: usize,
}

/// One row of a group: one decision waiting on one thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReviewRow {
    /// Unique within the group: the item's code, `<CODE>/<R-n>/<test>` for a manual check, the
    /// delegation's id, the delegated decision's id, or the ADR's id.
    pub key: String,
    pub code: String,
    pub title: String,
    /// What is waiting, in a few words.
    pub what: String,
    /// What a yes records.
    pub records: String,
    pub release: String,
    pub sprint: String,
    pub changed_at: String,
    /// Empty when the row can be decided; else one sentence naming who provides what is missing.
    pub blocked: String,
    pub pills: Vec<String>,
    /// The open prepared record's id.
    pub prepared: String,
    /// That record's batch.
    pub batch: String,
    /// What the group decision sends back.
    pub rev: String,
    pub spec_rev: String,
    pub signoff: String,
    pub requirement: String,
    pub test: String,
    /// The delegation under which Claude holds this row's decision (DEF-104); empty when the
    /// decision is the person's. A held row is shown, last, and not counted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub held_by: String,
    /// When that delegation ends (RFC 3339).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub held_until: String,
}

/// One group's rows, in the page's default order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GroupSlice {
    pub key: String,
    pub label: String,
    pub action: String,
    pub rows: Vec<ReviewRow>,
    /// The planned releases, in file order: for `which-release` only.
    pub choices: Vec<String>,
    /// The releases whose items the page lists first, in order (FTR-057,
    /// [`crate::manual::holding`]): for `manual` only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub holding: Vec<String>,
}

/// One row's full evidence: what the row's body shows when it is opened.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RowEvidence {
    pub entry: Option<ReviewEntry>,
    pub manual: Option<ManualWaiting>,
    pub adr: Option<Adr>,
    pub delegated: Option<DelegatedDecision>,
    pub delegation: Option<DelegationView>,
    pub prepared: Option<PreparedView>,
}

// ---- grouping --------------------------------------------------------------------------------

/// The group key of the sign-off `name`.
fn signoff_key(name: &str) -> String {
    match name {
        crate::method::DESIGN_REVIEW | crate::method::DOCS_FOLDED => name.to_string(),
        "release" => RELEASE_SIGNOFF.to_string(),
        _ => format!("{SIGNOFF_PREFIX}{name}"),
    }
}

/// The sign-off a group key names, if it is a sign-off group.
fn signoff_of_key(key: &str) -> Option<String> {
    match key {
        crate::method::DESIGN_REVIEW | crate::method::DOCS_FOLDED => Some(key.to_string()),
        RELEASE_SIGNOFF => Some("release".to_string()),
        _ => key
            .strip_prefix(SIGNOFF_PREFIX)
            .filter(|n| !n.is_empty())
            .map(String::from),
    }
}

/// The groups a review entry's decisions sit in. An item with an approval and a which-release
/// question is two decisions, so it is in two groups.
fn group_keys(entry: &ReviewEntry) -> Vec<String> {
    let mut keys = match entry.approval {
        ApprovalKind::Missing | ApprovalKind::Lapsed | ApprovalKind::Added => {
            vec![APPROVE.to_string()]
        }
        ApprovalKind::Unratified => vec![RATIFY.to_string()],
        ApprovalKind::Signoff | ApprovalKind::SpecChanged => entry
            .signoffs_needed
            .iter()
            .map(|n| signoff_key(n))
            .collect(),
        ApprovalKind::Release => Vec::new(),
    };
    if entry.release_choices.is_some() {
        keys.push(WHICH_RELEASE.to_string());
    }
    keys
}

/// The label and the action of group `key`.
fn label_and_action(key: &str) -> (String, &'static str) {
    match key {
        APPROVE => ("Approve".into(), "Approve"),
        RATIFY => ("Ratify".into(), "Ratify"),
        WHICH_RELEASE => ("Which release".into(), "Plan into"),
        MANUAL => ("Manual checks".into(), "Confirm"),
        DELEGATION => ("Delegations".into(), "Grant"),
        DELEGATED => ("Delegated decisions".into(), "Keep"),
        ADR => ("Proposed ADRs".into(), "Accept"),
        SYSTEM_BASELINE => ("System baseline".into(), "Accept"),
        RECODE => ("Recode".into(), "Confirm"),
        _ => match signoff_of_key(key) {
            Some(name) => (format!("Sign off {name}"), "Sign off"),
            None => (key.to_string(), ""),
        },
    }
}

/// Is `key` a group this version knows?
fn is_known(key: &str) -> bool {
    matches!(
        key,
        SYSTEM_BASELINE
            | APPROVE
            | RATIFY
            | WHICH_RELEASE
            | MANUAL
            | DELEGATION
            | DELEGATED
            | ADR
            | RECODE
    ) || signoff_of_key(key).is_some()
}

/// `n` and the noun, plural when `n` is not one.
fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// An RFC 3339 instant as `4 Dec`, in UTC; the text itself when it does not parse.
pub(crate) fn day_month(at: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    match time::OffsetDateTime::parse(at.trim(), &Rfc3339) {
        Ok(t) => {
            let t = t.to_offset(time::UtcOffset::UTC);
            let month = t.month().to_string();
            format!("{} {}", t.day(), month.get(..3).unwrap_or(&month))
        }
        Err(_) => at.to_string(),
    }
}

/// The verdict a group key's item rows ask: `approve`, `ratify` or `signoff:<name>`.
fn verdict_of_key(key: &str) -> Option<String> {
    match key {
        APPROVE => Some("approve".to_string()),
        RATIFY => Some("ratify".to_string()),
        _ => signoff_of_key(key).map(|n| format!("signoff:{n}")),
    }
}

/// The day of an RFC 3339 instant.
fn day(at: &str) -> &str {
    at.get(..10).unwrap_or(at)
}

/// Why an item row cannot be decided here: one sentence, or empty.
fn blocked_sentence(key: &str, entry: &ReviewEntry, mode: Mode) -> String {
    if mode == Mode::Conversation {
        return IN_CONVERSATION.to_string();
    }
    if key == crate::method::DOCS_FOLDED {
        let left = crate::review::docs_folded_left(entry);
        if left > 0 {
            return format!(
                "Claude settles {} first: ask Claude to check them.",
                count_of(left, "text", "texts")
            );
        }
    }
    String::new()
}

/// Every group key in the fixed order: system-baseline, approve, ratify, the sign-offs in the
/// order the process's gates name them, which-release, manual, delegation, delegated, adr,
/// recode. `entries` are the queue's: a sign-off asked again after its gate was changed still has
/// a group, after the rest.
fn keys_of(config: &crate::config::ProjectConfig, entries: &[ReviewEntry]) -> Vec<String> {
    let gates = config.effective_gates();
    let mut signoffs: Vec<String> = Vec::new();
    let mut push = |k: String| {
        if !signoffs.contains(&k) {
            signoffs.push(k);
        }
    };
    let mut staged: Vec<(&String, &crate::config::Gate)> = gates.iter().collect();
    staged.sort_by_key(|(status, _)| {
        config
            .statuses
            .iter()
            .position(|s| s == *status)
            .unwrap_or(usize::MAX)
    });
    for (_, gate) in staged {
        for name in &gate.signoffs {
            push(signoff_key(name));
        }
    }
    // A sign-off asked again after its gate was changed still has a group, after the rest.
    let mut others: Vec<String> = entries
        .iter()
        .flat_map(group_keys)
        .filter(|k| signoff_of_key(k).is_some())
        .collect();
    others.sort();
    for k in others {
        push(k);
    }
    let mut keys = vec![
        SYSTEM_BASELINE.to_string(),
        APPROVE.to_string(),
        RATIFY.to_string(),
    ];
    keys.extend(signoffs);
    keys.extend(
        [WHICH_RELEASE, MANUAL, DELEGATION, DELEGATED, ADR, RECODE]
            .into_iter()
            .map(String::from),
    );
    keys
}

/// A row built for a group, and whether it counts (a manual check not yet due does not).
struct Built {
    row: ReviewRow,
    counted: bool,
}

/// Everything the groups are built from, read once per request.
struct Board {
    entries: Vec<ReviewEntry>,
    manual: Vec<ManualWaiting>,
    delegations: Vec<DelegationView>,
    delegated: Vec<DelegatedDecision>,
    adrs: Vec<Adr>,
    prepared: Vec<PreparedView>,
    /// The system baseline's state, read only when the project has a baseline file (FTR-006).
    baseline: Option<crate::systembaseline::State>,
    mode: Mode,
    meta: crate::Project,
    releases: Vec<crate::releases::Release>,
    /// Every delegation, whatever its state, and the instant the board was read (DEF-104).
    all: Vec<crate::decisions::Delegation>,
    now: String,
}

impl Board {
    #[cfg(test)]
    fn load(store: &Store, p: &str, repo: Option<&Path>) -> Result<Board> {
        Self::load_at(store, p, repo, &crate::now_rfc3339())
    }

    fn load_at(store: &Store, p: &str, repo: Option<&Path>, now: &str) -> Result<Board> {
        // One read of the board's items for everything below (DEF-106).
        let project = store.load(p)?;
        let decisions = crate::decisions::view_parts_in(store, p, repo, now, &project)?;
        let all: Vec<crate::decisions::Delegation> = decisions
            .delegations
            .iter()
            .map(|d| d.delegation.clone())
            .collect();
        let baseline = match crate::systembaseline::load(store, p)? {
            Some(_) => Some(crate::systembaseline::state(store, &project, repo)?),
            None => None,
        };
        Ok(Board {
            baseline,
            entries: crate::review::pending_in(store, p, repo, &project)?,
            manual: crate::manual::waiting_in(store, p, &project)?,
            delegations: decisions
                .delegations
                .into_iter()
                .filter(|d| d.state == DelegationState::Proposed)
                .collect(),
            delegated: decisions.to_review,
            adrs: crate::adr::list(store, p)?
                .into_iter()
                .filter(|a| a.status.eq_ignore_ascii_case("proposed") && !a.is_superseded())
                .collect(),
            prepared: crate::prepared::list_in(
                store,
                p,
                &crate::prepared::ListQuery::default(),
                repo,
                &project,
            )?,
            mode: decisions.mode,
            meta: project,
            releases: crate::releases::load(store, p)?,
            all,
            now: now.to_string(),
        })
    }

    /// Every group key in the fixed order: system-baseline, approve, ratify, the sign-offs in the
    /// order the process's gates name them, which-release, manual, delegation, delegated, adr,
    /// recode.
    fn keys(&self) -> Vec<String> {
        keys_of(&self.meta.config, &self.entries)
    }

    /// The open prepared record of `kind` whose subject matches `item`, `signoff`, `requirement`
    /// and `test` (each empty to match any).
    fn open_record(
        &self,
        kind: &str,
        item: &str,
        signoff: &str,
        requirement: &str,
        test: &str,
    ) -> Option<&PreparedView> {
        self.prepared.iter().find(|v| {
            let s = &v.record.subject;
            v.state == crate::prepared::State::Open
                && v.record.kind == kind
                && s.item == item
                && (signoff.is_empty() || s.signoff == signoff)
                && (requirement.is_empty() || s.requirement == requirement)
                && (test.is_empty() || s.test == test)
        })
    }

    /// The release, sprint and last change of item `code`.
    fn placed(&self, code: &str) -> (String, String, String) {
        self.meta
            .features
            .iter()
            .find(|f| f.code == code)
            .map(|f| {
                (
                    f.release.clone().unwrap_or_default(),
                    f.sprint.clone().unwrap_or_default(),
                    f.updated_at.clone(),
                )
            })
            .unwrap_or_default()
    }

    /// The prepared record a row of group `key` for `entry` would confirm.
    fn entry_record(&self, key: &str, entry: &ReviewEntry) -> Option<&PreparedView> {
        if key == RATIFY {
            return self.open_record("ratification", &entry.code, "", "", "");
        }
        let name = signoff_of_key(key)?;
        // A docs-folded sign-off is not offered while texts are left, so nothing prepared for it
        // shows either (DEF-039).
        if name == crate::method::DOCS_FOLDED && crate::review::docs_folded_left(entry) > 0 {
            return None;
        }
        self.open_record("signoff", &entry.code, &name, "", "")
    }

    fn entry_row(&self, key: &str, e: &ReviewEntry) -> Built {
        let short: String = e.rev.chars().take(7).collect();
        let signoff = signoff_of_key(key).unwrap_or_default();
        let (what, records) = match key {
            APPROVE => (
                match e.approval {
                    ApprovalKind::Lapsed => "approval lapsed",
                    ApprovalKind::Added => "added since approval",
                    _ => "never approved",
                }
                .to_string(),
                format!("Records your approval of the definition as shown (rev `{short}`)"),
            ),
            RATIFY => {
                let n = e.overrides.len();
                (
                    format!(
                        "{} without agreement: {}",
                        e.status,
                        count_of(n, "override", "overrides")
                    ),
                    format!(
                        "Records a ratification; it closes the {} Claude made",
                        count_of(n, "override", "overrides")
                    ),
                )
            }
            WHICH_RELEASE => (
                "ready for release: which one?".to_string(),
                "Plans it into the release you choose (`kanbanr release plan`)".to_string(),
            ),
            _ => (
                if e.approval == ApprovalKind::SpecChanged {
                    "spec changed since sign-off".to_string()
                } else {
                    format!("waiting on {signoff}")
                },
                format!(
                    "Records the {signoff} sign-off, pinned to the definition and spec as shown"
                ),
            ),
        };
        let record = if key == WHICH_RELEASE || key == APPROVE {
            None
        } else {
            self.entry_record(key, e)
        };
        let mut pills: Vec<String> = match e.approval {
            ApprovalKind::Lapsed => vec!["approval lapsed".into()],
            ApprovalKind::SpecChanged => vec!["spec changed since sign-off".into()],
            ApprovalKind::Added => vec!["added since approval".into()],
            _ => Vec::new(),
        };
        // An approval that lapsed or grew through an edit to the tests alone is picked out (DEF-111).
        if key == APPROVE && e.tests_only {
            pills.push(TESTS_ONLY_PILL.into());
        }
        if e.finished {
            pills.push("finished".into());
        }
        push_record_pills(&mut pills, record);
        // A decision Claude holds under a delegation is shown, last, and not counted (DEF-104).
        let held = verdict_of_key(key)
            .and_then(|v| e.held.iter().find(|h| h.verdict == v))
            .cloned();
        if let Some(h) = &held {
            pills.push(format!(
                "Claude holds \u{b7} {} until {}",
                h.delegation,
                day_month(&h.until)
            ));
        }
        let spec_rev = match &e.spec {
            Some(spec) if !signoff.is_empty() && spec.signoff == signoff => e.spec_rev.clone(),
            _ => String::new(),
        };
        Built {
            row: ReviewRow {
                key: e.code.clone(),
                code: e.code.clone(),
                title: e.title.clone(),
                what,
                records,
                release: e.release.clone(),
                sprint: e.sprint.clone(),
                changed_at: e.changed_at.clone(),
                blocked: blocked_sentence(key, e, self.mode),
                pills,
                prepared: record.map(|r| r.record.id.clone()).unwrap_or_default(),
                batch: record
                    .and_then(|r| r.record.batch.clone())
                    .unwrap_or_default(),
                rev: e.rev.clone(),
                spec_rev,
                signoff,
                requirement: String::new(),
                test: String::new(),
                held_by: held
                    .as_ref()
                    .map(|h| h.delegation.clone())
                    .unwrap_or_default(),
                held_until: held.as_ref().map(|h| h.until.clone()).unwrap_or_default(),
            },
            counted: held.is_none(),
        }
    }

    fn manual_row(&self, m: &ManualWaiting) -> Built {
        let record = self.open_record("manual-test", &m.code, "", &m.requirement, &m.test);
        let blocked = if !m.opens_at.is_empty() {
            // DEF-076's sentence: the check opens at the build stage.
            format!(
                "{} is at {}: its manual check opens at {}, once the code it checks exists. \
                 Record it then.",
                m.code, m.status, m.opens_at
            )
        } else if record.is_none() {
            DO_THE_CHECK.to_string()
        } else {
            String::new()
        };
        let mut pills = Vec::new();
        push_record_pills(&mut pills, record);
        let (release, sprint, changed_at) = self.placed(&m.code);
        Built {
            row: ReviewRow {
                key: manual_key(&m.code, &m.requirement, &m.test),
                code: m.code.clone(),
                title: m.title.clone(),
                what: format!("{}: {}", m.requirement, m.test),
                records: "Records the prepared result as you".into(),
                release,
                sprint,
                changed_at,
                blocked,
                pills,
                prepared: record.map(|r| r.record.id.clone()).unwrap_or_default(),
                batch: record
                    .and_then(|r| r.record.batch.clone())
                    .unwrap_or_default(),
                rev: m.checked_rev.clone(),
                spec_rev: String::new(),
                signoff: String::new(),
                requirement: m.requirement.clone(),
                test: m.test.clone(),
                held_by: String::new(),
                held_until: String::new(),
            },
            counted: m.opens_at.is_empty(),
        }
    }

    fn delegation_row(&self, d: &DelegationView) -> Built {
        let g = &d.delegation;
        let mut scope: Vec<String> = Vec::new();
        if g.all {
            scope.push("every item".into());
        }
        scope.extend(g.items.iter().cloned());
        scope.extend(g.kinds.iter().map(|k| format!("every {k}")));
        if !g.milestone.is_empty() {
            scope.push(format!("milestone {}", g.milestone));
        }
        Built {
            row: ReviewRow {
                key: g.id.clone(),
                code: g.id.clone(),
                title: g.note.clone(),
                what: format!(
                    "{} for {} until {}",
                    g.verdicts.join(", "),
                    scope.join(", "),
                    day(&g.until)
                ),
                records: format!("Grants {}: Claude may take these verdicts", g.id),
                release: String::new(),
                sprint: String::new(),
                changed_at: g.proposed.at.clone(),
                blocked: String::new(),
                pills: Vec::new(),
                prepared: String::new(),
                batch: String::new(),
                rev: d.rev.clone(),
                spec_rev: String::new(),
                signoff: String::new(),
                requirement: String::new(),
                test: String::new(),
                held_by: String::new(),
                held_until: String::new(),
            },
            counted: true,
        }
    }

    fn delegated_row(&self, d: &DelegatedDecision) -> Built {
        let verdict = if d.signoff.is_empty() {
            d.verdict.clone()
        } else {
            format!("{} {}", d.verdict, d.signoff)
        };
        let (release, sprint, _) = self.placed(&d.code);
        Built {
            row: ReviewRow {
                key: d.decision.clone(),
                code: d.code.clone(),
                title: d.title.clone(),
                what: format!("{verdict} under {} on {}", d.delegation, day(&d.at)),
                records: "Keep: it stands. Undo: it is withdrawn".into(),
                release,
                sprint,
                changed_at: d.at.clone(),
                blocked: String::new(),
                pills: Vec::new(),
                prepared: String::new(),
                batch: String::new(),
                rev: String::new(),
                spec_rev: String::new(),
                signoff: d.signoff.clone(),
                requirement: String::new(),
                test: String::new(),
                held_by: String::new(),
                held_until: String::new(),
            },
            counted: true,
        }
    }

    /// The one row of the system-baseline group: shown while the baseline awaits its first
    /// acceptance or an acceptance is prepared. Not blocked in conversation mode: the acceptance
    /// is not an item verdict (FTR-006).
    fn baseline_rows(&self) -> Vec<Built> {
        let Some(state) = &self.baseline else {
            return Vec::new();
        };
        let record = self.prepared.iter().find(|v| {
            v.state == crate::prepared::State::Open && v.record.kind == "system-baseline"
        });
        if state.kind != crate::systembaseline::StateKind::Awaiting && record.is_none() {
            return Vec::new();
        }
        let pinned = state
            .rows
            .len()
            .saturating_sub(state.unpinned.len() + state.missing.len());
        let what = match record {
            Some(r) => {
                let note = r
                    .record
                    .fields
                    .get("note")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                if note.is_empty() {
                    "prepared".to_string()
                } else {
                    format!("prepared: {note}")
                }
            }
            None => format!(
                "{}, {pinned} pinned: awaiting acceptance",
                count_of(state.rows.len(), "row", "rows")
            ),
        };
        let short: String = state.pins_rev.chars().take(7).collect();
        let blocked = if state.can_accept {
            String::new()
        } else if state.missing.is_empty() {
            state.cannot_accept_why.clone()
        } else {
            format!(
                "Write the system docs first: {} have no source yet",
                state.missing.join(", ")
            )
        };
        let mut pills = Vec::new();
        push_record_pills(&mut pills, record);
        vec![Built {
            row: ReviewRow {
                key: SYSTEM_BASELINE.to_string(),
                code: SYSTEM_BASELINE.to_string(),
                title: "System baseline".into(),
                what,
                records: format!(
                    "Records your acceptance of the system baseline as shown (pins rev `{short}`)"
                ),
                release: String::new(),
                sprint: String::new(),
                changed_at: state
                    .accepted
                    .as_ref()
                    .map(|a| a.at.clone())
                    .unwrap_or_default(),
                blocked,
                pills,
                prepared: record.map(|r| r.record.id.clone()).unwrap_or_default(),
                batch: record
                    .and_then(|r| r.record.batch.clone())
                    .unwrap_or_default(),
                rev: state.pins_rev.clone(),
                spec_rev: String::new(),
                signoff: String::new(),
                requirement: String::new(),
                test: String::new(),
                held_by: String::new(),
                held_until: String::new(),
            },
            counted: true,
        }]
    }

    /// The rows of the recode group: each open prepared `recode` record (only the newest is open;
    /// a newer one supersedes). Not blocked in conversation mode: confirming it is the person's
    /// own decision, not an item verdict (FTR-056).
    fn recode_rows(&self) -> Vec<Built> {
        self.prepared
            .iter()
            .filter(|v| v.state == crate::prepared::State::Open && v.record.kind == "recode")
            .map(|v| {
                let plan = v
                    .record
                    .fields
                    .get("plan")
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let n = plan.lines().filter(|l| l.contains(" \u{2192} ")).count();
                let mut pills = Vec::new();
                push_record_pills(&mut pills, Some(v));
                Built {
                    row: ReviewRow {
                        key: v.record.id.clone(),
                        code: RECODE.to_string(),
                        title: format!("Recode {}", count_of(n, "item", "items")),
                        what: format!(
                            "{} to their kinds' prefixes, prepared {}",
                            count_of(n, "item", "items"),
                            day(&v.record.at)
                        ),
                        records: format!(
                            "Renames {} to their kinds' prefixes; each keeps its old code as an \
                             alias (plan `{}`)",
                            count_of(n, "item", "items"),
                            v.record.rev.chars().take(7).collect::<String>()
                        ),
                        release: String::new(),
                        sprint: String::new(),
                        changed_at: v.record.at.clone(),
                        blocked: String::new(),
                        pills,
                        prepared: v.record.id.clone(),
                        batch: v.record.batch.clone().unwrap_or_default(),
                        rev: v.record.rev.clone(),
                        spec_rev: String::new(),
                        signoff: String::new(),
                        requirement: String::new(),
                        test: String::new(),
                        held_by: String::new(),
                        held_until: String::new(),
                    },
                    counted: true,
                }
            })
            .collect()
    }

    fn adr_row(&self, a: &Adr) -> Built {
        let blocked = if a.missing.is_empty() {
            String::new()
        } else {
            format!(
                "Answer {} first: ask Claude to fill them.",
                a.missing.join(", ")
            )
        };
        Built {
            row: ReviewRow {
                key: a.id.clone(),
                code: a.id.clone(),
                title: a.title.clone(),
                what: format!("proposed {}", day(&a.date)),
                records: format!("Records {} as accepted", a.id),
                release: String::new(),
                sprint: String::new(),
                changed_at: a.date.clone(),
                blocked,
                pills: Vec::new(),
                prepared: String::new(),
                batch: String::new(),
                rev: String::new(),
                spec_rev: String::new(),
                signoff: String::new(),
                requirement: String::new(),
                test: String::new(),
                held_by: String::new(),
                held_until: String::new(),
            },
            counted: true,
        }
    }

    /// The rows of group `key`, unordered.
    fn rows(&self, key: &str) -> Vec<Built> {
        match key {
            MANUAL => self.manual.iter().map(|m| self.manual_row(m)).collect(),
            DELEGATION => self
                .delegations
                .iter()
                .map(|d| self.delegation_row(d))
                .collect(),
            DELEGATED => self
                .delegated
                .iter()
                .map(|d| self.delegated_row(d))
                .collect(),
            ADR => self.adrs.iter().map(|a| self.adr_row(a)).collect(),
            SYSTEM_BASELINE => self.baseline_rows(),
            RECODE => self.recode_rows(),
            _ => self
                .entries
                .iter()
                .filter(|e| group_keys(e).iter().any(|k| k == key))
                .map(|e| self.entry_row(key, e))
                .collect(),
        }
    }

    /// What Claude holds in the queue, and per revoked or ended delegation how many of the queue's
    /// verdicts it was in scope for that nobody holds now (DEF-104).
    fn holds(&self) -> Holds {
        use crate::decisions::DelegationState;
        let mut out = Holds::default();
        let gone: Vec<&crate::decisions::Delegation> = self
            .all
            .iter()
            .filter(|g| {
                matches!(
                    g.state(&self.now),
                    DelegationState::Revoked | DelegationState::Ended
                )
            })
            .collect();
        let mut returned: Vec<(String, usize)> = gone.iter().map(|g| (g.id.clone(), 0)).collect();
        for e in &self.entries {
            out.held.extend(e.held.iter().map(|h| HeldDecision {
                code: e.code.clone(),
                verdict: h.verdict.clone(),
                delegation: h.delegation.clone(),
                until: h.until.clone(),
            }));
            let Ok(item) = self.meta.feature(&e.code) else {
                continue;
            };
            for v in e.asked() {
                if e.held.iter().any(|h| h.verdict == v.name()) {
                    continue;
                }
                for (g, (_, n)) in gone.iter().zip(returned.iter_mut()) {
                    if g.in_scope(&v, item) {
                        *n += 1;
                    }
                }
            }
        }
        out.returned = returned.into_iter().filter(|(_, n)| *n > 0).collect();
        out
    }

    /// The planned releases, in file order.
    fn choices(&self) -> Vec<String> {
        self.releases
            .iter()
            .filter(|r| r.state == crate::releases::ReleaseState::Planned)
            .map(|r| r.version.clone())
            .collect()
    }

    /// The default order: the release in flight first, then planned releases by version, then no
    /// release, then by code.
    fn order(&self, rows: &mut [ReviewRow]) {
        let in_flight = |v: &str| {
            self.releases.iter().any(|r| {
                r.version == v
                    && r.state == crate::releases::ReleaseState::Planned
                    && !r.stage.is_empty()
                    && r.superseded_by.is_empty()
            })
        };
        rows.sort_by_cached_key(|r| {
            let bucket = if r.release.is_empty() {
                2u8
            } else if in_flight(&r.release) {
                0
            } else {
                1
            };
            (
                !r.held_by.is_empty(),
                bucket,
                crate::releases::version_key(&r.release),
                r.code.clone(),
                r.key.clone(),
            )
        });
    }
}

/// The pills an open prepared record adds: "prepared", and "batch <name>" when it is in one.
fn push_record_pills(pills: &mut Vec<String>, record: Option<&PreparedView>) {
    if let Some(r) = record {
        pills.push("prepared".into());
        if let Some(batch) = r.record.batch.as_deref().filter(|b| !b.is_empty()) {
            pills.push(format!("batch {batch}"));
        }
    }
}

/// A manual check's row key.
fn manual_key(code: &str, requirement: &str, test: &str) -> String {
    format!("{code}/{requirement}/{test}")
}

fn not_waiting(what: String) -> CoreError {
    CoreError::NoSuchRoute(what)
}

fn unknown_group(key: &str) -> CoreError {
    not_waiting(format!("there is no review group '{key}'"))
}

// ---- one read of the board per revision (DEF-106) -----------------------------------------------

/// How many (board, project) snapshots are kept at once.
const SHARED_SLOTS: usize = 4;

/// What a snapshot of the review board was read at: it is reused while all of this is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReviewRev {
    data_dir: PathBuf,
    project: String,
    /// The board repository's HEAD, empty without a commit.
    board_head: String,
    /// The project's uncommitted files with their sizes and modification times, hashed.
    board_dirty: String,
    /// The code repository's fingerprint, empty without one.
    code: String,
    /// The UTC day.
    day: String,
    /// How many granted delegations have ended, so one that lapses mid-day is noticed at once.
    ended: usize,
}

type SharedSlot = Arc<Mutex<Option<(ReviewRev, Arc<Board>)>>>;

static SHARED: Mutex<Vec<((PathBuf, String), SharedSlot)>> = Mutex::new(Vec::new());

#[cfg(test)]
thread_local! {
    static COMPUTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The snapshots computed on this thread, so a test is not counted against by the others.
#[cfg(test)]
fn computed_here() -> u64 {
    COMPUTED.with(std::cell::Cell::get)
}

#[cfg(test)]
fn rev_of(store: &Store, p: &str, repo: Option<&Path>) -> ReviewRev {
    rev_at(store, p, repo, &crate::now_rfc3339())
}

/// [`rev_of`] at the instant `now`: the one place the clock enters a revision, so a test can move
/// it past midnight (DEF-106).
fn rev_at(store: &Store, p: &str, repo: Option<&Path>, now: &str) -> ReviewRev {
    let data_dir = store.data_dir().to_path_buf();
    let prefix = format!("projects/{p}/");
    let mut dirty: Vec<String> = crate::git::dirty_paths(&data_dir)
        .into_iter()
        .filter(|f| f.starts_with(&prefix))
        .collect();
    dirty.sort();
    let mut seen = String::new();
    for f in &dirty {
        let (len, at) = std::fs::metadata(data_dir.join(f)).map_or((0, 0), |m| {
            let at = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            (m.len(), at)
        });
        seen.push_str(&format!("{f}\0{len}\0{at}\n"));
    }
    ReviewRev {
        ended: crate::decisions::ended_granted(store, p, now),
        board_head: crate::git::head_commit(&data_dir).unwrap_or_default(),
        board_dirty: crate::hash::stable_hash(&seen),
        code: repo.map(crate::coderepo::fingerprint).unwrap_or_default(),
        day: now.chars().take(10).collect(),
        project: p.to_string(),
        data_dir,
    }
}

/// The review board of the current revision: read once, and shared by the summary, every slice,
/// the queue, the decisions and the attention badge until the board or the code repository
/// changes. Concurrent callers at one revision compute it once; an error is returned and not kept.
fn shared(store: &Store, p: &str, repo: Option<&Path>) -> Result<Arc<Board>> {
    shared_at(store, p, repo, &crate::now_rfc3339())
}

/// [`shared`] at the instant `now`, which dates both the revision and the board read under it.
fn shared_at(store: &Store, p: &str, repo: Option<&Path>, now: &str) -> Result<Arc<Board>> {
    #[cfg(test)]
    if crate::git::shared_off() {
        return Ok(Arc::new(Board::load_at(store, p, repo, now)?));
    }
    // A board that is not a git repository cannot say when it changed, so nothing is kept.
    if crate::git::head_commit(store.data_dir()).is_none()
        && git2::Repository::open(store.data_dir()).is_err()
    {
        return Ok(Arc::new(Board::load_at(store, p, repo, now)?));
    }
    let rev = rev_at(store, p, repo, now);
    let slot: SharedSlot = {
        let mut slots = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        let key = (rev.data_dir.clone(), p.to_string());
        match slots.iter().position(|(k, _)| *k == key) {
            Some(at) => {
                let kept = slots.remove(at);
                slots.push(kept);
            }
            None => {
                if slots.len() >= SHARED_SLOTS {
                    slots.remove(0);
                }
                slots.push((key, Arc::new(Mutex::new(None))));
            }
        }
        slots.last().map(|(_, s)| s.clone()).unwrap_or_default()
    };
    let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, board)) = held.as_ref()
        && *at == rev
    {
        return Ok(board.clone());
    }
    #[cfg(test)]
    COMPUTED.with(|c| c.set(c.get() + 1));
    let board = Arc::new(Board::load_at(store, p, repo, now)?);
    *held = Some((rev, board.clone()));
    Ok(board)
}

/// The review queue of the current revision, as [`crate::review::pending`] gives it, from the
/// shared read.
pub fn queue_shared(store: &Store, p: &str, repo: Option<&Path>) -> Result<Vec<ReviewEntry>> {
    Ok(shared(store, p, repo)?.entries.clone())
}

/// The review queue of the current revision with the project it was judged on, from the shared
/// read: what `decisions::waiting` needs, so it reads no item itself (DEF-106).
pub(crate) fn queue_and_project(
    store: &Store,
    p: &str,
    repo: Option<&Path>,
) -> Result<(Vec<ReviewEntry>, crate::Project)> {
    let board = shared(store, p, repo)?;
    Ok((board.entries.clone(), board.meta.clone()))
}

/// One decision group the config can have, named before any is counted: a tab the page draws at
/// once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GroupOutline {
    pub key: String,
    pub label: String,
    pub action: String,
}

/// Every group the process's config can have, in the order of the page, from the config alone: no
/// item is loaded. A sign-off asked again after its gate changed is not here; it arrives with the
/// summary.
pub fn outline(store: &Store, p: &str) -> Result<Vec<GroupOutline>> {
    let config = store.read_project_config(p)?;
    Ok(keys_of(&config, &[])
        .into_iter()
        .map(|key| {
            let (label, action) = label_and_action(&key);
            GroupOutline {
                action: action.to_string(),
                label,
                key,
            }
        })
        .collect())
}

// ---- reads -------------------------------------------------------------------------------------

/// Every non-empty decision group of project `p`, counted, in the fixed order. A group whose rows
/// Claude all holds is listed with a count of 0 and its `held` rows (DEF-104).
pub fn summary(store: &Store, p: &str, repo: Option<&Path>) -> Result<Vec<GroupCount>> {
    let board = shared(store, p, repo)?;
    summary_of(&board).map(|x| x.0)
}

/// What Claude holds and what returns to the person, read with the summary (DEF-104).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holds {
    pub held: Vec<HeldDecision>,
    /// Per revoked or ended delegation: how many queue verdicts it was in scope for that nobody
    /// holds now.
    pub returned: Vec<(String, usize)>,
}

/// One decision Claude holds under a delegation in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldDecision {
    pub code: String,
    /// `approve`, `ratify` or `signoff:<name>`.
    pub verdict: String,
    pub delegation: String,
    pub until: String,
}

/// [`summary`] with what Claude holds and what returned to the person, from one read of the board
/// (DEF-104).
pub fn summary_with_holds(
    store: &Store,
    p: &str,
    repo: Option<&Path>,
    now: &str,
) -> Result<(Vec<GroupCount>, Holds)> {
    summary_of(&Board::load_at(store, p, repo, now)?)
}

/// [`summary_with_holds`] for the instant now, from the shared read of the board (DEF-106): what
/// the attention badge counts.
pub fn summary_with_holds_shared(
    store: &Store,
    p: &str,
    repo: Option<&Path>,
) -> Result<(Vec<GroupCount>, Holds)> {
    let board = shared(store, p, repo)?;
    summary_of(&board)
}

fn summary_of(board: &Board) -> Result<(Vec<GroupCount>, Holds)> {
    let groups = board
        .keys()
        .into_iter()
        .filter_map(|key| {
            let rows = board.rows(&key);
            let counted: Vec<&ReviewRow> =
                rows.iter().filter(|b| b.counted).map(|b| &b.row).collect();
            let held = rows.iter().filter(|b| !b.row.held_by.is_empty()).count();
            if counted.is_empty() && held == 0 {
                return None;
            }
            let (label, action) = label_and_action(&key);
            Some(GroupCount {
                count: counted.len(),
                ready: counted.iter().filter(|r| r.blocked.is_empty()).count(),
                prepared: counted.iter().filter(|r| !r.prepared.is_empty()).count(),
                held,
                action: action.to_string(),
                label,
                key,
            })
        })
        .collect();
    Ok((groups, board.holds()))
}

/// One group's rows, in the default order. An unknown key is refused as no such route.
pub fn slice(store: &Store, p: &str, key: &str, repo: Option<&Path>) -> Result<GroupSlice> {
    if !is_known(key) {
        return Err(unknown_group(key));
    }
    let board = shared(store, p, repo)?;
    let mut rows: Vec<ReviewRow> = board.rows(key).into_iter().map(|b| b.row).collect();
    board.order(&mut rows);
    let (label, action) = label_and_action(key);
    Ok(GroupSlice {
        key: key.to_string(),
        label,
        action: action.to_string(),
        rows,
        choices: if key == WHICH_RELEASE {
            board.choices()
        } else {
            Vec::new()
        },
        holding: if key == MANUAL {
            crate::manual::holding(store, p)?
        } else {
            Vec::new()
        },
    })
}

/// One row's full evidence. An item row reads [`crate::review::entry`] for that item alone, never
/// the whole queue.
pub fn evidence(
    store: &Store,
    p: &str,
    key: &str,
    row: &str,
    repo: Option<&Path>,
) -> Result<RowEvidence> {
    if !is_known(key) {
        return Err(unknown_group(key));
    }
    let missing = || not_waiting(format!("{row} is not waiting in the review group '{key}'"));
    let mut out = RowEvidence {
        entry: None,
        manual: None,
        adr: None,
        delegated: None,
        delegation: None,
        prepared: None,
    };
    let open = |kind: &str, item: &str, signoff: &str, requirement: &str, test: &str| {
        crate::prepared::list(
            store,
            p,
            &crate::prepared::ListQuery {
                item: Some(item.to_string()),
                ..Default::default()
            },
            repo,
        )
        .map(|views| {
            views.into_iter().find(|v| {
                let s = &v.record.subject;
                v.record.kind == kind
                    && (signoff.is_empty() || s.signoff == signoff)
                    && (requirement.is_empty() || s.requirement == requirement)
                    && (test.is_empty() || s.test == test)
            })
        })
    };
    match key {
        MANUAL => {
            let mut parts = row.splitn(3, '/');
            let (code, requirement, test) = match (parts.next(), parts.next(), parts.next()) {
                (Some(c), Some(r), Some(t)) => (c, r, t),
                _ => return Err(missing()),
            };
            // A key kept under an earlier code still names the item (FTR-056).
            let project = store.load_meta(p)?;
            let code = crate::aliases::current(&project, code).unwrap_or(code);
            let m = crate::manual::waiting(store, p)?
                .into_iter()
                .find(|m| m.code == code && m.requirement == requirement && m.test == test)
                .ok_or_else(missing)?;
            out.prepared = open("manual-test", code, "", requirement, test)?;
            out.manual = Some(m);
        }
        DELEGATION | DELEGATED => {
            let view = crate::decisions::view_parts(store, p, repo, &crate::now_rfc3339())?;
            if key == DELEGATION {
                out.delegation = Some(
                    view.delegations
                        .into_iter()
                        .find(|d| d.delegation.id == row && d.state == DelegationState::Proposed)
                        .ok_or_else(missing)?,
                );
            } else {
                out.delegated = Some(
                    view.to_review
                        .into_iter()
                        .find(|d| d.decision == row)
                        .ok_or_else(missing)?,
                );
            }
        }
        SYSTEM_BASELINE => {
            if row != SYSTEM_BASELINE {
                return Err(missing());
            }
            let review = crate::review::system_baseline_review(store, p, repo)?;
            if review.prepared.is_none()
                && review.state.kind != crate::systembaseline::StateKind::Awaiting
            {
                return Err(missing());
            }
            out.prepared = review.prepared.and_then(|r| {
                crate::prepared::list(store, p, &crate::prepared::ListQuery::default(), repo)
                    .ok()?
                    .into_iter()
                    .find(|v| v.record.id == r.id)
            });
        }
        RECODE => {
            out.prepared = Some(
                crate::prepared::list(store, p, &crate::prepared::ListQuery::default(), repo)?
                    .into_iter()
                    .find(|v| {
                        v.record.id == row
                            && v.record.kind == "recode"
                            && v.state == crate::prepared::State::Open
                    })
                    .ok_or_else(missing)?,
            );
        }
        ADR => {
            out.adr = Some(
                crate::adr::list(store, p)?
                    .into_iter()
                    .find(|a| {
                        a.id == row
                            && a.status.eq_ignore_ascii_case("proposed")
                            && !a.is_superseded()
                    })
                    .ok_or_else(missing)?,
            );
        }
        _ => {
            let entry = crate::review::entry(store, p, row, repo)?
                .filter(|e| group_keys(e).iter().any(|k| k == key))
                .ok_or_else(missing)?;
            out.prepared = if key == RATIFY {
                open("ratification", row, "", "", "")?
            } else if let Some(name) = signoff_of_key(key) {
                if name == crate::method::DOCS_FOLDED && crate::review::docs_folded_left(&entry) > 0
                {
                    None
                } else {
                    open("signoff", row, &name, "", "")?
                }
            } else {
                None
            };
            out.entry = Some(entry);
        }
    }
    Ok(out)
}

// ---- group decisions -------------------------------------------------------------------------

/// The most rows one group decision may carry: a guard, not a page size.
pub const MAX_DECISION_ITEMS: usize = 200;

/// One decision the person took for several rows of one group (FTR-038 R-3): the body of
/// `POST /projects/{p}/review/decide`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct GroupDecision {
    pub group: String,
    /// The group's action, `undo` for the delegated group's second button, or `reject` to turn
    /// down the selected prepared records.
    pub action: String,
    pub by: String,
    /// The release, for `which-release`.
    pub target: String,
    /// A reason; no group action needs one (R-11), so it is kept only where a route takes it.
    pub reason: String,
    pub items: Vec<DecisionItem>,
}

/// One selected row, with the pins the slice showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct DecisionItem {
    pub key: String,
    pub code: String,
    pub rev: String,
    pub spec_rev: String,
    pub signoff: String,
    pub requirement: String,
    pub test: String,
    pub prepared: String,
}

/// One verdict a group decision makes, through its own route.
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictOp {
    /// The row it decides.
    pub key: String,
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
    /// The answer to keep once the verdict is made, when the row's prepared record made it.
    pub resolution: Option<crate::prepared::Resolution>,
}

/// `s` as one path segment: everything but unreserved characters percent-encoded.
fn segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn refuse(why: String) -> CoreError {
    CoreError::Unsupported(why)
}

/// Each selected row of `d` as its own verdict route, read against the group as it is now. The
/// paths are built from the group key and each row as the board holds it, never from the body.
/// A prepared row expands through [`crate::prepared::expand`], so its record resolves; any other
/// row posts the single route the page posts today. Refused, with nothing to write, when there
/// are no items (or more than [`MAX_DECISION_ITEMS`]), when a row is no longer in the group or
/// cannot be decided, or when two rows would make different verdicts. `repo` is the code
/// repository the page judges blocks with (a docs-folded row's texts left), so a row the page
/// shows blocked is refused here too.
pub fn ops(
    store: &Store,
    p: &str,
    d: &GroupDecision,
    origin: crate::origin::Origin,
    repo: Option<&Path>,
) -> Result<Vec<VerdictOp>> {
    use serde_json::{Value, json};
    if d.items.is_empty() {
        return Err(refuse(
            "a group decision names the rows it decides: none were selected".into(),
        ));
    }
    if d.items.len() > MAX_DECISION_ITEMS {
        return Err(refuse(format!(
            "a group decision takes at most {MAX_DECISION_ITEMS} rows; {} were sent",
            d.items.len()
        )));
    }
    let by = d.by.trim();
    if by.is_empty() {
        return Err(refuse(
            "a group decision must name who gave it: pass `by` (the monitor sends the board's \
             commit identity)"
                .into(),
        ));
    }
    if !is_known(&d.group) {
        return Err(refuse(format!("there is no review group '{}'", d.group)));
    }
    let group = slice(store, p, &d.group, repo)?;
    let action = d.action.trim().to_ascii_lowercase();
    let own = group.action.to_ascii_lowercase();
    let undo = action == "undo" && d.group == DELEGATED;
    let reject = action == "reject";
    if !(action == own || undo || reject) {
        return Err(refuse(format!(
            "'{}' is not a decision the {} group takes: it takes {}",
            d.action, group.label, group.action
        )));
    }
    let target = d.target.trim();
    if d.group == WHICH_RELEASE && !reject && !group.choices.iter().any(|c| c == target) {
        return Err(refuse(format!(
            "plan into which release? '{target}' is not a planned release: choose one of {}",
            group.choices.join(", ")
        )));
    }
    let pins = |item: &DecisionItem| {
        let mut b = json!({"by": by});
        for (k, v) in [("rev", &item.rev), ("spec_rev", &item.spec_rev)] {
            if !v.is_empty() {
                b[k] = Value::String(v.clone());
            }
        }
        b
    };
    let mut out = Vec::with_capacity(d.items.len());
    let mut verdict: Option<Option<crate::origin::Verdict>> = None;
    for item in &d.items {
        let name = if item.code.is_empty() {
            &item.key
        } else {
            &item.code
        };
        let Some(row) = group.rows.iter().find(|r| r.key == item.key) else {
            return Err(refuse(format!(
                "{name} is no longer waiting in {}; the page re-reads",
                group.label
            )));
        };
        if !row.blocked.is_empty() {
            return Err(refuse(format!(
                "{} cannot be decided here: {}",
                row.code, row.blocked
            )));
        }
        if !item.prepared.is_empty() && item.prepared != row.prepared {
            return Err(refuse(format!(
                "{}'s prepared record is no longer {}; the page re-reads",
                row.code, item.prepared
            )));
        }
        let code = segment(&row.code);
        let op = if reject {
            if row.prepared.is_empty() {
                return Err(refuse(format!(
                    "{} has nothing prepared to reject: only prepared records are turned down \
                     from the group",
                    row.code
                )));
            }
            VerdictOp {
                key: row.key.clone(),
                method: "POST".into(),
                path: format!("/projects/{p}/prepared/{}/reject", segment(&row.prepared)),
                body: json!({"by": by}),
                resolution: None,
            }
        } else if !item.prepared.is_empty() || (d.group == MANUAL && !row.prepared.is_empty()) {
            let e = crate::prepared::expand(store, p, &row.prepared, &pins(item), origin)?;
            VerdictOp {
                key: row.key.clone(),
                method: e.method,
                path: e.path,
                body: e.body,
                resolution: Some(e.resolution),
            }
        } else {
            let (path, body) = match d.group.as_str() {
                APPROVE => (format!("/projects/{p}/features/{code}/approve"), pins(item)),
                RATIFY => (format!("/projects/{p}/features/{code}/ratify"), pins(item)),
                WHICH_RELEASE => (
                    format!("/projects/{p}/releases/{}/plan", segment(target)),
                    json!({"items": [row.code]}),
                ),
                DELEGATION => (
                    format!(
                        "/projects/{p}/decisions/delegations/{}/grant",
                        segment(&row.key)
                    ),
                    json!({"by": by, "rev": item.rev}),
                ),
                DELEGATED => (
                    format!("/projects/{p}/decisions/reviews"),
                    json!({
                        "decision": row.key,
                        "verdict": if undo { "undo" } else { "keep" },
                        "by": by,
                    }),
                ),
                ADR => (
                    format!("/projects/{p}/adrs/{}/accept", segment(&row.key)),
                    json!({"by": by}),
                ),
                // The person's acceptance as they saw it: `rev` is the pins' hash they were shown.
                SYSTEM_BASELINE => (format!("/projects/{p}/catalog/baseline/accept"), pins(item)),
                RECODE => {
                    return Err(refuse(
                        "a recode is confirmed from its prepared plan: ask Claude to prepare it \
                         (`kanbanr prepare recode`)"
                            .into(),
                    ));
                }
                MANUAL => {
                    return Err(refuse(format!(
                        "{} cannot be decided here: {DO_THE_CHECK}",
                        row.code
                    )));
                }
                key => match signoff_of_key(key) {
                    Some(name) => (
                        format!("/projects/{p}/features/{code}/signoff/{}", segment(&name)),
                        pins(item),
                    ),
                    None => {
                        return Err(refuse(format!("there is no review group '{key}'")));
                    }
                },
            };
            VerdictOp {
                key: row.key.clone(),
                method: "POST".into(),
                path,
                body,
                resolution: None,
            }
        };
        let v = crate::origin::verdict_of_route(&op.method, &op.path);
        match verdict {
            None => verdict = Some(v),
            Some(first) if first != v => {
                return Err(refuse(format!(
                    "one group decision makes one kind of verdict: {} would make another; \
                     decide it in its own group",
                    row.code
                )));
            }
            Some(_) => {}
        }
        out.push(op);
    }
    Ok(out)
}

/// `Some(project)` for `POST /projects/{p}/review/decide`, the group decision route.
pub fn decide_route(method: &str, path: &str) -> Option<String> {
    if !method.eq_ignore_ascii_case("POST") {
        return None;
    }
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match seg.as_slice() {
        ["projects", p, "review", "decide"] => Some(p.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Gate, ProjectConfig};
    use crate::models::{FeatureDefinition, Requirement, TestRef};

    /// A board in its own folder under the temp dir (L-41), checked before the first write.
    fn board() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-reviewgroups-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        assert!(
            root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
            "a test board lives in the temp dir: {}",
            root.display()
        );
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.cadence.releases = true;
        config.gates.insert(
            "Scheduled".into(),
            Gate {
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        config.gates.insert(
            "Completed".into(),
            Gate {
                signoffs: vec!["docs-folded".into()],
                ..Default::default()
            },
        );
        store.init_project("demo", config).unwrap();
        assert_eq!(std::fs::canonicalize(store.data_dir()).unwrap(), root);
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    #[test]
    fn a_snapshot_is_stale_once_a_granted_delegation_has_lapsed() {
        let (store, dir) = board();
        let stamp = "{by: Ada, at: \"2026-11-01T00:00:00Z\", origin: test}";
        let file = |until: &str, extra: &str| {
            format!(
                "delegations:\n- id: DLG-1\n  verdicts: [approve]\n  all: true\n  until: \"{until}\"\n  proposed: {stamp}\n{extra}"
            )
        };
        let path = store.project_dir("demo").join("decisions.yaml");
        let granted = format!("  granted: {stamp}\n");
        std::fs::write(&path, file("2099-01-01T00:00:00Z", &granted)).unwrap();
        let before = rev_of(&store, "demo", None);
        assert_eq!(before.ended, 0);
        // The same delegation with its time passed: the day has not changed, the revision has.
        std::fs::write(&path, file("2020-01-01T00:00:00Z", &granted)).unwrap();
        let after = rev_of(&store, "demo", None);
        assert_eq!(after.ended, 1);
        assert_ne!(before, after);
        // One that was only proposed, or was revoked, is not counted when it lapses.
        std::fs::write(&path, file("2020-01-01T00:00:00Z", "")).unwrap();
        assert_eq!(rev_of(&store, "demo", None).ended, 0);
        let revoked = format!("{granted}  revoked: {stamp}\n");
        std::fs::write(&path, file("2020-01-01T00:00:00Z", &revoked)).unwrap();
        assert_eq!(rev_of(&store, "demo", None).ended, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-106 R-5: a new UTC day computes the snapshot again, and the board read under it is dated
    /// by the day it was read on; later the same day, the same snapshot is kept.
    #[test]
    fn a_new_day_computes_it_again() {
        let (store, dir) = board_with_decisions();
        let computed = computed_here();
        let morning = shared_at(&store, "demo", None, "2026-11-17T08:00:00Z").unwrap();
        assert_eq!(computed_here() - computed, 1, "the first read computes");
        assert_eq!(morning.now, "2026-11-17T08:00:00Z");
        let evening = shared_at(&store, "demo", None, "2026-11-17T23:59:59Z").unwrap();
        assert!(
            Arc::ptr_eq(&morning, &evening),
            "the same day keeps the snapshot"
        );
        assert_eq!(computed_here() - computed, 1);
        let next = shared_at(&store, "demo", None, "2026-11-18T00:00:01Z").unwrap();
        assert!(!Arc::ptr_eq(&morning, &next), "a new day computes it again");
        assert_eq!(computed_here() - computed, 2);
        assert_eq!(next.now, "2026-11-18T00:00:01Z");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn define(store: &Store, title: &str, def: FeatureDefinition) -> String {
        let f = store.add_feature("demo", title, "", "M", None).unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        f.code
    }

    /// DEF-111 R-1: an Approve row whose approval lapsed through an edit to its tests alone
    /// carries the "tests only" pill after "approval lapsed"; an edit to a requirement's text and
    /// a never-approved item carry none, and no row of another group does.
    #[test]
    fn a_tests_only_approve_row_carries_the_pill() {
        use crate::models::TestState;
        let (store, dir) = board();
        let def = |test: &str, text: &str| FeatureDefinition {
            statement: "s".into(),
            requirements: vec![Requirement {
                id: "R-1".into(),
                text: text.into(),
                tests: vec![TestRef {
                    name: test.into(),
                    kind: "unit".into(),
                    state: TestState::Green,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let approved = |title: &str, edit: FeatureDefinition| {
            let code = define(&store, title, def("a::t", "THE SYSTEM SHALL x"));
            store.approve_feature("demo", &code, "Ada").unwrap();
            store
                .set_feature_definition("demo", &code, Some(edit))
                .unwrap();
            code
        };
        let tests = approved("Tests", def("a::t_renamed", "THE SYSTEM SHALL x"));
        let text = approved("Text", def("a::t_renamed", "THE SYSTEM SHALL y"));
        let never = define(&store, "Never", def("a::t", "THE SYSTEM SHALL x"));

        let approve = slice(&store, "demo", "approve", None).unwrap();
        let row = |c: &str| approve.rows.iter().find(|r| r.code == c).unwrap();
        assert_eq!(
            row(&tests).pills,
            ["approval lapsed", TESTS_ONLY_PILL],
            "{approve:#?}"
        );
        assert_eq!(row(&text).pills, ["approval lapsed"]);
        assert!(row(&never).pills.is_empty(), "{:?}", row(&never).pills);
        for g in summary(&store, "demo", None).unwrap() {
            if g.key == "approve" {
                continue;
            }
            for r in slice(&store, "demo", &g.key, None).unwrap().rows {
                assert!(!r.pills.iter().any(|p| p == TESTS_ONLY_PILL), "{r:?}");
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    fn statement(s: &str) -> FeatureDefinition {
        FeatureDefinition {
            statement: s.into(),
            ..Default::default()
        }
    }

    /// A board in its own git repository with a few waiting decisions: an approval, a lapsed
    /// approval and an item waiting on a sign-off.
    fn board_with_decisions() -> (Store, std::path::PathBuf) {
        let (store, dir) = board();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        define(&store, "Never approved", statement("Never approved"));
        let lapsed = define(&store, "Lapsed", statement("Lapsed"));
        store.approve_feature("demo", &lapsed, "Ada").unwrap();
        store
            .set_feature_definition("demo", &lapsed, Some(statement("Lapsed, changed")))
            .unwrap();
        let signed = define(&store, "Signed", statement("Waits on its sign-offs"));
        store.approve_feature("demo", &signed, "Ada").unwrap();
        assert!(crate::git::commit_local(&dir, "the board"));
        (store, dir)
    }

    /// DEF-106 R-1: the outline names every group the config can have, in the page's order, from
    /// the config alone: no item is loaded.
    #[test]
    fn the_outline_names_every_group_the_config_can_have() {
        let (store, dir) = board();
        let loads = store.loads();
        let out = outline(&store, "demo").unwrap();
        assert_eq!(store.loads(), loads, "the config alone, no board load");
        let keys: Vec<&str> = out.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "system-baseline",
                "approve",
                "ratify",
                "design-review",
                "docs-folded",
                "which-release",
                "manual",
                "delegation",
                "delegated",
                "adr",
                "recode"
            ]
        );
        let docs = out.iter().find(|g| g.key == "docs-folded").unwrap();
        assert_eq!(docs.label, "Sign off docs-folded");
        assert_eq!(docs.action, "Sign off");
        assert_eq!(out[1].label, "Approve");
        // Every group the summary can list is in the outline.
        let (board_store, board_dir) = board_with_decisions();
        for g in summary(&board_store, "demo", None).unwrap() {
            assert!(keys.contains(&g.key.as_str()), "{}", g.key);
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(board_dir);
    }

    /// DEF-106 R-5: the summary, three slices, the queue, the decisions and the attention badge of
    /// one revision are one computation of the queue and one walk of the board's history.
    #[test]
    fn one_revision_computes_the_queue_once() {
        let (store, dir) = board_with_decisions();
        let (computed, walks) = (computed_here(), crate::git::path_history_walks_here());
        let groups = summary(&store, "demo", None).unwrap();
        assert!(!groups.is_empty());
        for key in ["approve", "ratify", "design-review"] {
            slice(&store, "demo", key, None).unwrap();
        }
        let queue = queue_shared(&store, "demo", None).unwrap();
        assert_eq!(
            serde_json::to_value(&queue).unwrap(),
            serde_json::to_value(crate::review::pending(&store, "demo", None).unwrap()).unwrap()
        );
        crate::decisions::view(&store, "demo", None, None, &crate::now_rfc3339()).unwrap();
        crate::attention::attention(&store, "demo", None).unwrap();
        assert_eq!(computed_here() - computed, 1, "one queue for every read");
        assert!(crate::git::path_history_walks_here() - walks <= 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-106 R-5: a board commit, an uncommitted board edit and a code commit each give a new
    /// snapshot; nothing changing gives the same one.
    #[test]
    fn a_board_or_code_change_computes_it_again() {
        use crate::docdrift::testkit::{code_commit, git_env, scratch};
        let (store, dir) = board_with_decisions();
        let repo = scratch("rev-code");
        git_env(&repo, &["init", "-q", "-b", "main"], None);
        code_commit(&repo, "a.txt", "one\n", "one", "2026-11-01T10:00:00+00:00");
        let computed = |f: &dyn Fn()| {
            let before = computed_here();
            f();
            computed_here() - before
        };
        let read = || {
            summary(&store, "demo", Some(&repo)).unwrap();
        };
        assert_eq!(computed(&read), 1, "the first read computes");
        assert_eq!(computed(&read), 0, "nothing changed");
        // An uncommitted board edit.
        define(&store, "Another", statement("Another"));
        assert_eq!(computed(&read), 1, "an uncommitted board edit");
        assert_eq!(computed(&read), 0);
        // A second edit to the same, already modified file set.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let docs = store.project_dir("demo").join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("a.md"), "one").unwrap();
        assert_eq!(computed(&read), 1, "a new uncommitted file");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(docs.join("a.md"), "two and more").unwrap();
        assert_eq!(computed(&read), 1, "a second edit to it");
        // A board commit.
        assert!(crate::git::commit_local(&dir, "board"));
        assert_eq!(computed(&read), 1, "a board commit");
        assert_eq!(computed(&read), 0);
        // A code commit.
        code_commit(&repo, "b.txt", "two\n", "two", "2026-11-02T10:00:00+00:00");
        assert_eq!(computed(&read), 1, "a code commit");
        // An uncommitted edit to the code repository.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        assert_eq!(computed(&read), 1, "an uncommitted code edit");
        assert_eq!(computed(&read), 0);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(repo);
    }

    /// DEF-106 R-6: the summary, every slice and the queue from the shared read equal those of a
    /// read with every shared path switched off.
    #[test]
    fn summary_slices_and_queue_match_the_unshared_answers() {
        let (store, dir) = board_with_decisions();
        let answers = || {
            let summary = summary(&store, "demo", None).unwrap();
            let slices: Vec<serde_json::Value> = outline(&store, "demo")
                .unwrap()
                .iter()
                .map(|g| {
                    serde_json::to_value(slice(&store, "demo", &g.key, None).unwrap()).unwrap()
                })
                .collect();
            let queue = queue_shared(&store, "demo", None).unwrap();
            serde_json::json!({
                "summary": summary,
                "slices": slices,
                "queue": queue,
            })
        };
        let shared = answers();
        crate::git::set_shared_off(true);
        let unshared = answers();
        crate::git::set_shared_off(false);
        assert_eq!(shared, unshared);
        assert!(
            !shared["summary"].as_array().unwrap().is_empty(),
            "the fixture waits on decisions"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-104 (supports R-4): a row whose decision a delegation holds is shown last, with a pill,
    /// and is not counted; a group of only held rows is still listed, with a count of 0.
    #[test]
    fn a_row_a_delegation_holds_is_shown_last_and_not_counted() {
        let (store, dir) = board();
        let a = define(&store, "Alpha", statement("a"));
        let b = define(&store, "Beta", statement("b"));
        let delegate = |items: &[&str]| {
            use crate::decisions::{Decisions, Delegation, Mode, ModeChange, Stamp};
            let stamp = |origin: &str| Stamp {
                by: "Ada".into(),
                at: "2026-11-01T00:00:00+00:00".into(),
                origin: origin.into(),
                ..Default::default()
            };
            let d = Decisions {
                modes: vec![ModeChange {
                    mode: Mode::Delegated,
                    by: "Ada".into(),
                    at: "2026-11-01T00:00:00+00:00".into(),
                    origin: "monitor".into(),
                    reason: String::new(),
                    extra: Default::default(),
                }],
                delegations: vec![Delegation {
                    id: "DLG-001".into(),
                    verdicts: vec!["approve".into()],
                    items: items.iter().map(|s| s.to_string()).collect(),
                    until: "2099-12-04T00:00:00+00:00".into(),
                    proposed: stamp("conversation"),
                    granted: Some(stamp("monitor")),
                    ..Default::default()
                }],
                ..Default::default()
            };
            std::fs::write(
                store.project_dir("demo").join("decisions.yaml"),
                crate::yaml::to_string(&d).unwrap(),
            )
            .unwrap();
        };
        // Alpha is Claude's; Beta is the person's, though it sorts after it by code.
        delegate(&[a.as_str()]);
        let groups = summary(&store, "demo", None).unwrap();
        let approve = groups.iter().find(|g| g.key == "approve").unwrap();
        assert_eq!(
            (approve.count, approve.held, approve.ready, approve.prepared),
            (1, 1, 1, 0)
        );
        let rows = slice(&store, "demo", "approve", None).unwrap().rows;
        assert_eq!(
            rows.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(),
            vec![b.as_str(), a.as_str()],
            "the person's row first, Claude's last"
        );
        assert_eq!(rows[0].held_by, "");
        assert_eq!(rows[1].held_by, "DLG-001");
        assert!(rows[1].held_until.starts_with("2099-12-04"));
        assert!(
            rows[1]
                .pills
                .contains(&"Claude holds \u{b7} DLG-001 until 4 Dec".to_string()),
            "{:?}",
            rows[1].pills
        );
        assert!(rows[1].blocked.is_empty(), "the person may still decide it");
        let (_, holds) = summary_with_holds(&store, "demo", None, NOW).unwrap();
        assert_eq!(
            holds.held,
            vec![HeldDecision {
                code: a.clone(),
                verdict: "approve".into(),
                delegation: "DLG-001".into(),
                until: "2099-12-04T00:00:00+00:00".into(),
            }]
        );
        // Both held: the group is still listed, with nothing counted.
        delegate(&[a.as_str(), b.as_str()]);
        let groups = summary(&store, "demo", None).unwrap();
        let approve = groups.iter().find(|g| g.key == "approve").unwrap();
        assert_eq!(
            (approve.count, approve.held, approve.ready),
            (0, 2, 0),
            "{groups:?}"
        );
        assert_eq!(
            slice(&store, "demo", "approve", None).unwrap().rows.len(),
            2
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    const NOW: &str = "2026-11-17T10:00:00+00:00";

    fn entry(code: &str, approval: ApprovalKind, signoffs: &[&str]) -> ReviewEntry {
        ReviewEntry {
            code: code.into(),
            title: code.into(),
            status: "Planned".into(),
            approval,
            signoffs_needed: signoffs.iter().map(|s| s.to_string()).collect(),
            changes_no_documents: false,
            signoff_lapsed: None,
            to_check: None,
            settled: Vec::new(),
            drift_skipped: None,
            started_unapproved: String::new(),
            definition: FeatureDefinition::default(),
            rev: String::new(),
            in_progress: false,
            finished: false,
            changes: None,
            release_choices: None,
            spec_rev: String::new(),
            spec: None,
            overrides: Vec::new(),
            release: String::new(),
            sprint: String::new(),
            changed_at: String::new(),
            tests_only: false,
            system_baseline_rows: Vec::new(),
            views_changed: Vec::new(),
            held: Vec::new(),
            claudes: false,
        }
    }

    /// FTR-056 R-2: a manual row's key kept under a code the item has since left still opens the
    /// item's evidence.
    #[test]
    fn a_manual_row_keyed_by_an_earlier_code_still_finds_its_evidence() {
        let (store, dir) = board();
        let code = define(
            &store,
            "Checked",
            FeatureDefinition {
                requirements: vec![Requirement {
                    text: "THE SYSTEM SHALL do it".into(),
                    tests: vec![TestRef {
                        name: "manual: the person judges whether it reads clearly".into(),
                        kind: "manual".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..statement("Checked by hand")
            },
        );
        let key = slice(&store, "demo", "manual", None).unwrap().rows[0]
            .key
            .clone();
        assert!(key.starts_with(&format!("{code}/")), "{key}");
        store
            .edit_feature("demo", &code, None, None, None, Some("DEF-900".into()))
            .unwrap();
        let ev = evidence(&store, "demo", "manual", &key, None).unwrap();
        assert_eq!(ev.manual.unwrap().code, "DEF-900");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-038 R-1: every waiting decision sits in the group of the decision it needs; the summary
    /// lists each group once, in the fixed order, with its count; an item with an approval and a
    /// which-release question is in both; an empty group is absent.
    #[test]
    fn waiting_decisions_are_grouped_by_the_decision_they_need() {
        // Each approval kind maps to its group, sign-offs by name.
        let keys = |e: ReviewEntry| group_keys(&e);
        assert_eq!(keys(entry("A", ApprovalKind::Missing, &[])), ["approve"]);
        assert_eq!(keys(entry("A", ApprovalKind::Lapsed, &[])), ["approve"]);
        assert_eq!(keys(entry("A", ApprovalKind::Added, &[])), ["approve"]);
        assert_eq!(keys(entry("A", ApprovalKind::Unratified, &[])), ["ratify"]);
        assert_eq!(
            keys(entry(
                "A",
                ApprovalKind::Signoff,
                &["design-review", "docs-folded", "release", "qa"]
            )),
            [
                "design-review",
                "docs-folded",
                "release-signoff",
                "signoff-qa"
            ]
        );
        assert_eq!(
            keys(entry("A", ApprovalKind::SpecChanged, &["design-review"])),
            ["design-review"]
        );
        let mut both = entry("A", ApprovalKind::Missing, &[]);
        both.release_choices = Some(vec![]);
        assert_eq!(keys(both), ["approve", "which-release"]);
        let mut release = entry("A", ApprovalKind::Release, &[]);
        release.release_choices = Some(vec!["1.0".into()]);
        assert_eq!(keys(release), ["which-release"]);
        // A docs-folded sign-off with texts left is blocked, and says who settles them.
        let mut left = entry("A", ApprovalKind::Signoff, &["docs-folded"]);
        left.to_check = Some(Vec::new());
        assert_eq!(blocked_sentence("docs-folded", &left, Mode::Monitor), "");
        assert_eq!(
            blocked_sentence("docs-folded", &left, Mode::Conversation),
            IN_CONVERSATION
        );
        for key in [
            "approve",
            "ratify",
            "design-review",
            "release-signoff",
            "signoff-qa",
            "which-release",
            "manual",
            "delegation",
            "delegated",
            "adr",
        ] {
            assert!(is_known(key), "{key}");
            assert!(!label_and_action(key).1.is_empty(), "{key}");
        }
        assert!(!is_known("nothing"));

        // A board with one waiting decision of each source.
        let (store, dir) = board();
        let never = define(&store, "Never approved", statement("Never approved"));
        let lapsed = define(&store, "Lapsed", statement("Lapsed"));
        store.approve_feature("demo", &lapsed, "Ada").unwrap();
        store
            .set_feature_definition("demo", &lapsed, Some(statement("Lapsed, changed")))
            .unwrap();
        let signed = define(&store, "Signed", statement("Waits on its sign-offs"));
        store.approve_feature("demo", &signed, "Ada").unwrap();
        // Built and agreed, before the end: its docs-folded sign-off and its release question.
        let built = define(&store, "Built", statement("Built, waits on docs-folded"));
        store.approve_feature("demo", &built, "Ada").unwrap();
        store
            .signoff_feature("demo", &built, "design-review", "Ada", "", "")
            .unwrap();
        // Its one task done while it is still Planned, so finishing it moves nothing.
        let list = store.add_todo_list("demo", &built, "work", None).unwrap();
        store
            .add_task(
                "demo",
                &built,
                &list.code,
                "one",
                None,
                &crate::store::TaskFields::default(),
            )
            .unwrap();
        store
            .set_task_state(
                "demo",
                &built,
                &list.code,
                "T1",
                crate::models::TaskState::Completed,
            )
            .unwrap();
        store.move_feature("demo", &built, "Scheduled").unwrap();
        let bypassed = define(
            &store,
            "Bypassed",
            FeatureDefinition {
                started_unapproved: "built before agreement".into(),
                ..statement("Bypassed")
            },
        );
        store.move_feature("demo", &bypassed, "Scheduled").unwrap();
        store.move_feature("demo", &bypassed, "Completed").unwrap();
        // Unapproved and at the stage before the end: an approval and a release question.
        let ready = define(&store, "Ready", statement("Ready, never approved"));
        store.move_feature("demo", &ready, "Scheduled").unwrap();
        let checked = define(
            &store,
            "Checked",
            FeatureDefinition {
                requirements: vec![Requirement {
                    text: "THE SYSTEM SHALL do it".into(),
                    tests: vec![TestRef {
                        name: "manual: the person judges whether it reads clearly".into(),
                        kind: "manual".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..statement("Checked by hand")
            },
        );
        store
            .approve_feature_as("demo", &checked, "Claude", "delegated", "DLG-1", None)
            .unwrap();
        store.move_feature("demo", &checked, "Scheduled").unwrap();
        store.move_feature("demo", &checked, "Completed").unwrap();
        let now = crate::now_rfc3339();
        let until = (time::OffsetDateTime::now_utc() + time::Duration::days(30))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        crate::decisions::propose(
            &store,
            "demo",
            crate::decisions::NewDelegation {
                verdicts: vec!["approve".into()],
                items: vec![never.clone()],
                until,
                note: "the small ones".into(),
                ..Default::default()
            },
            crate::decisions::Stamp {
                by: "Claude".into(),
                at: now,
                origin: "conversation".into(),
                ..Default::default()
            },
        )
        .unwrap();
        crate::adr::create(&store, "demo", "Keep it simple", Adr::default()).unwrap();

        let groups = summary(&store, "demo", None).unwrap();
        let got: Vec<(&str, usize)> = groups.iter().map(|g| (g.key.as_str(), g.count)).collect();
        assert_eq!(
            got,
            [
                ("approve", 3),
                ("ratify", 1),
                ("design-review", 1),
                ("docs-folded", 1),
                ("which-release", 2),
                ("manual", 1),
                ("delegation", 1),
                ("delegated", 1),
                ("adr", 1),
            ],
            "{groups:#?}"
        );
        let group = |k: &str| groups.iter().find(|g| g.key == k).unwrap();
        assert_eq!(group("approve").action, "Approve");
        assert_eq!(group("docs-folded").label, "Sign off docs-folded");
        assert_eq!(group("which-release").action, "Plan into");
        // Nothing prepared: the manual check is the person's own, so none is ready.
        assert_eq!(group("manual").ready, 0);
        assert_eq!(group("manual").prepared, 0);
        // The scaffolded ADR leaves its sections unanswered.
        assert_eq!(group("adr").ready, 0);
        assert_eq!(group("approve").ready, 3);

        // The item with an approval and a release question is in both groups.
        let approve = slice(&store, "demo", "approve", None).unwrap();
        let codes: Vec<&str> = approve.rows.iter().map(|r| r.code.as_str()).collect();
        let mut want = vec![never.as_str(), lapsed.as_str(), ready.as_str()];
        want.sort();
        assert_eq!(codes, want);
        let lapsed_row = approve.rows.iter().find(|r| r.code == lapsed).unwrap();
        assert_eq!(lapsed_row.what, "approval lapsed");
        assert_eq!(lapsed_row.pills, ["approval lapsed"]);
        assert!(!lapsed_row.rev.is_empty());
        let which = slice(&store, "demo", "which-release", None).unwrap();
        let codes: Vec<&str> = which.rows.iter().map(|r| r.code.as_str()).collect();
        let mut want = vec![built.as_str(), ready.as_str()];
        want.sort();
        assert_eq!(codes, want);
        let docs = slice(&store, "demo", "docs-folded", None).unwrap();
        assert_eq!(docs.rows[0].code, built);
        assert_eq!(docs.rows[0].what, "waiting on docs-folded");
        let manual = slice(&store, "demo", "manual", None).unwrap();
        assert_eq!(manual.rows[0].blocked, DO_THE_CHECK);
        assert_eq!(
            manual.rows[0].key,
            format!("{checked}/R-1/manual: the person judges whether it reads clearly")
        );
        let design = slice(&store, "demo", "design-review", None).unwrap();
        assert_eq!(design.rows[0].code, signed);
        assert_eq!(design.rows[0].signoff, "design-review");
        assert!(slice(&store, "demo", "nothing", None).is_err());

        // A row's evidence is that one entry.
        let ev = evidence(&store, "demo", "approve", &never, None).unwrap();
        assert_eq!(ev.entry.unwrap().code, never);
        let ev = evidence(&store, "demo", "manual", &manual.rows[0].key, None).unwrap();
        assert_eq!(ev.manual.unwrap().code, checked);
        assert!(evidence(&store, "demo", "ratify", &never, None).is_err());

        // Decided, a group empties and leaves the summary.
        let ratify_row = slice(&store, "demo", "ratify", None).unwrap().rows[0].clone();
        assert_eq!(ratify_row.code, bypassed);
        store
            .ratify_feature("demo", &bypassed, "Ada", "agreed after the fact")
            .unwrap();
        let groups = summary(&store, "demo", None).unwrap();
        assert!(
            groups.iter().all(|g| g.key != "ratify"),
            "an empty group is absent: {groups:#?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-038 R-2, DEF-076 R-1: a manual check whose item has not reached its build stage
    /// (`opens_at` set) is shown in the manual group with DEF-076's sentence, but is not counted
    /// among the checks waiting for the person: not in the group's count, and a group holding only
    /// such checks is absent from the summary.
    #[test]
    fn a_manual_check_not_yet_due_is_not_counted_in_its_group() {
        let (store, dir) = board();
        // Entering Scheduled makes the branch, so Scheduled is the build stage.
        let mut config = store.read_project_config("demo").unwrap();
        config
            .gates
            .get_mut("Scheduled")
            .unwrap()
            .on_enter
            .push(crate::config::Action::Branch);
        store.save_config("demo", &config).unwrap();
        let checked = |title: &str| {
            define(
                &store,
                title,
                FeatureDefinition {
                    requirements: vec![Requirement {
                        text: "THE SYSTEM SHALL do it".into(),
                        tests: vec![TestRef {
                            name: "manual: the person judges whether it reads clearly".into(),
                            kind: "manual".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..statement(title)
                },
            )
        };
        // Before its build stage: the check is not yet due.
        let early = checked("Early");
        let groups = summary(&store, "demo", None).unwrap();
        assert!(
            groups.iter().all(|g| g.key != "manual"),
            "a group of checks not yet due is absent: {groups:#?}"
        );
        let manual = slice(&store, "demo", "manual", None).unwrap();
        assert_eq!(manual.rows.len(), 1, "{manual:?}");
        assert_eq!(manual.rows[0].code, early);
        assert!(
            manual.rows[0].blocked.contains("opens at Scheduled"),
            "the row says when it opens: {:?}",
            manual.rows[0]
        );

        // Another item at its build stage: its check is due and is the only one counted.
        let built = checked("Built");
        store.move_feature("demo", &built, "Scheduled").unwrap();
        let groups = summary(&store, "demo", None).unwrap();
        let group = groups
            .iter()
            .find(|g| g.key == "manual")
            .unwrap_or_else(|| panic!("the due check is counted: {groups:#?}"));
        assert_eq!(group.count, 1, "{group:?}");
        assert_eq!(group.ready, 0, "nothing prepared: {group:?}");
        let manual = slice(&store, "demo", "manual", None).unwrap();
        assert_eq!(manual.rows.len(), 2, "{manual:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-038 R-3: a group decision judges blocks exactly as the page does, against the code
    /// repository: a docs-folded row with a text left is blocked on the page and refused by the
    /// decision, with nothing to write; read without the repository the row would look ready.
    #[test]
    fn a_row_the_page_blocks_on_drift_is_refused_by_the_decision() {
        let (store, _d, code, repo) = crate::tests::drift_board();
        let key = crate::method::DOCS_FOLDED;
        let page = slice(&store, "demo", key, Some(&repo)).unwrap();
        let row = page
            .rows
            .iter()
            .find(|r| r.code == code)
            .unwrap_or_else(|| panic!("{code} waits in {key}: {page:?}"))
            .clone();
        assert!(
            row.blocked.contains("settles 1 text"),
            "the page blocks the row on its drift: {row:?}"
        );
        let decision = GroupDecision {
            group: key.into(),
            action: page.action.clone(),
            by: "Ada L".into(),
            items: vec![DecisionItem {
                key: row.key.clone(),
                code: row.code.clone(),
                rev: row.rev.clone(),
                spec_rev: row.spec_rev.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let e = ops(
            &store,
            "demo",
            &decision,
            crate::origin::Origin::Monitor,
            Some(&repo),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains(&code), "{e}");
        assert!(e.contains("cannot be decided here"), "{e}");
        assert!(e.contains(&row.blocked), "{e}");
        // Without the repository the drift is not read, so the same row is not blocked: the
        // decide route must pass the repository for the refusal above to hold.
        let blind = slice(&store, "demo", key, None).unwrap();
        let unread = blind.rows.iter().find(|r| r.code == code).unwrap();
        assert!(unread.blocked.is_empty(), "{unread:?}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-006 R-12: an awaiting baseline is one `system-baseline` group, first in the order, with
    /// one row and the accept op; an accepted or changed one without a prepared record gives none;
    /// the row is not blocked in conversation mode; a group decision from Claude, or any delegation,
    /// is refused.
    #[test]
    fn the_system_baseline_acceptance_is_its_own_group() {
        use crate::catalog::{Catalog, Level, Row, Source, SourceKind};
        let (store, dir) = board();
        let row = |id: &str| Row {
            id: id.into(),
            title: id.into(),
            answers: vec!["what".into()],
            for_: vec!["architect".into()],
            level: Level::System,
            source: Source {
                kind: SourceKind::Written,
                path: format!("architecture/{id}.md"),
                ..Source::default()
            },
            ..Row::default()
        };
        crate::catalog::save(
            &store,
            "demo",
            &Catalog {
                schema: crate::catalog::SCHEMA,
                rows: vec![row("context"), row("security")],
                ..Catalog::default()
            },
        )
        .unwrap();
        // Needs an item waiting in another group, so "first" means something.
        define(&store, "Plain", statement("Never approved"));
        let keys = |store: &Store| -> Vec<String> {
            summary(store, "demo", None)
                .unwrap()
                .into_iter()
                .map(|g| g.key)
                .collect()
        };
        assert_eq!(keys(&store), ["approve"], "no baseline file: no group");

        crate::systembaseline::init(&store, "demo").unwrap();
        // Init turns the build gate on and saves the config; the board is as it was otherwise.
        let g = summary(&store, "demo", None).unwrap();
        assert_eq!(g[0].key, "system-baseline", "{g:?}");
        assert_eq!(
            (g[0].label.as_str(), g[0].action.as_str()),
            ("System baseline", "Accept")
        );
        assert_eq!((g[0].count, g[0].ready), (1, 0), "no source is written yet");
        let slice0 = slice(&store, "demo", "system-baseline", None).unwrap();
        assert_eq!(slice0.rows.len(), 1);
        assert!(
            slice0.rows[0]
                .blocked
                .starts_with("Write the system docs first: context, security"),
            "{}",
            slice0.rows[0].blocked
        );

        for id in ["context", "security"] {
            store
                .write_doc(
                    "demo",
                    &format!("architecture/{id}.md"),
                    &format!("# {id}\n\n{}\n", "The system as it stands. ".repeat(4)),
                )
                .unwrap();
        }
        let slice1 = slice(&store, "demo", "system-baseline", None).unwrap();
        let r = &slice1.rows[0];
        assert!(r.blocked.is_empty(), "{}", r.blocked);
        assert_eq!(r.what, "2 rows, 2 pinned: awaiting acceptance");
        assert!(
            r.records
                .starts_with("Records your acceptance of the system baseline as shown (pins rev `")
        );
        let state =
            crate::systembaseline::state(&store, &store.load_meta("demo").unwrap(), None).unwrap();
        assert_eq!(r.rev, state.pins_rev);

        // Not blocked in conversation mode, where an item row is.
        let mut b = Board::load(&store, "demo", None).unwrap();
        b.mode = Mode::Conversation;
        assert!(b.baseline_rows()[0].row.blocked.is_empty());
        assert_eq!(
            blocked_sentence("approve", &entry("X", ApprovalKind::Missing, &[]), b.mode),
            IN_CONVERSATION
        );

        // The accept op: the route is built from the group key, carrying the rev the person saw.
        let decision = |by: &str| GroupDecision {
            group: "system-baseline".into(),
            action: "Accept".into(),
            by: by.into(),
            items: vec![DecisionItem {
                key: "system-baseline".into(),
                code: "system-baseline".into(),
                rev: r.rev.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let ops = ops(
            &store,
            "demo",
            &decision("Ada"),
            crate::origin::Origin::Monitor,
            None,
        )
        .unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, "/projects/demo/catalog/baseline/accept");
        assert_eq!(ops[0].body["rev"], r.rev.as_str());
        assert_eq!(
            crate::origin::verdict_of_route(&ops[0].method, &ops[0].path),
            Some(crate::origin::Verdict::AcceptSystemBaseline)
        );
        // Claude is refused the group decision and the verdict it expands into, in every mode;
        // no delegation covers either.
        let ctx = crate::origin::DecisionContext::monitor_mode();
        for verdict in [
            crate::origin::Verdict::GroupDecision,
            crate::origin::Verdict::AcceptSystemBaseline,
        ] {
            assert!(
                crate::origin::verdict_refusal(verdict, crate::origin::Taker::Claude, &ctx)
                    .is_some()
            );
        }
        assert!(crate::decisions::is_person_only(
            &ops[0].method,
            &ops[0].path
        ));

        // Accepted: no group; changed: no group; a prepared acceptance brings it back.
        crate::systembaseline::accept(&store, "demo", "Ada", "monitor", "", "", None).unwrap();
        assert_eq!(keys(&store), ["approve"]);
        store
            .write_doc(
                "demo",
                "architecture/context.md",
                "# context\n\nrevised in full. revised in full.\n",
            )
            .unwrap();
        assert_eq!(
            keys(&store),
            ["approve"],
            "a changed baseline waits for a prepared record"
        );
        crate::prepared::prepare(
            &store,
            "demo",
            &serde_json::json!({
                "kind": "system-baseline",
                "fields": {"note": "re-accept at the milestone"},
                "origin": "conversation",
                "sources": [{"label": "the context doc", "url": "doc:architecture/context.md"}],
            }),
            None,
        )
        .unwrap();
        assert_eq!(keys(&store)[0], "system-baseline");
        let prepared = slice(&store, "demo", "system-baseline", None).unwrap();
        assert_eq!(
            prepared.rows[0].what,
            "prepared: re-accept at the milestone"
        );
        assert!(!prepared.rows[0].prepared.is_empty());
        assert!(prepared.rows[0].pills.contains(&"prepared".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-056 R-7: a prepared `recode` record is its own group, last in the order, with one row
    /// and the Confirm action. The group decision expands through the prepared record, so the
    /// verdict is `Recode`, refused to Claude in every mode.
    #[test]
    fn a_prepared_recode_plan_is_its_own_group() {
        let (store, dir) = board();
        for t in ["one", "two"] {
            store
                .add_feature_of_kind("demo", t, "", "M", None, Some("bug"))
                .unwrap();
        }
        store
            .set_codes(
                "demo",
                crate::config::CodesChange {
                    set: [("bug".to_string(), "DEF".to_string())]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
            )
            .unwrap();
        define(&store, "Plain", statement("Never approved"));
        let keys = |store: &Store| -> Vec<String> {
            summary(store, "demo", None)
                .unwrap()
                .into_iter()
                .map(|g| g.key)
                .collect()
        };
        assert_eq!(keys(&store), ["approve"], "nothing prepared: no group");
        let rec = crate::prepared::prepare(
            &store,
            "demo",
            &serde_json::json!({
                "kind": "recode",
                "origin": "conversation",
                "sources": [{"label": "the plan the person was shown", "url": ""}],
            }),
            None,
        )
        .unwrap();
        let g = summary(&store, "demo", None).unwrap();
        let last = g.last().unwrap();
        assert_eq!(last.key, "recode", "{g:?}");
        assert_eq!(
            (last.label.as_str(), last.action.as_str(), last.count),
            ("Recode", "Confirm", 1)
        );
        assert_eq!(last.prepared, 1);
        let s = slice(&store, "demo", "recode", None).unwrap();
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.rows[0].prepared, rec.id);
        assert!(s.rows[0].what.starts_with("2 items"), "{}", s.rows[0].what);
        let ev = evidence(&store, "demo", "recode", &s.rows[0].key, None).unwrap();
        let plan = ev.prepared.unwrap().record.fields["plan"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(plan.contains("FEAT-001 \u{2192} DEF-001"), "{plan}");
        let decision = |by: &str| GroupDecision {
            group: "recode".into(),
            action: "Confirm".into(),
            by: by.into(),
            items: vec![DecisionItem {
                key: s.rows[0].key.clone(),
                code: s.rows[0].code.clone(),
                prepared: rec.id.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let o = ops(
            &store,
            "demo",
            &decision("Ada"),
            crate::origin::Origin::Monitor,
            None,
        )
        .unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].path, "/projects/demo/recode");
        assert_eq!(
            crate::origin::verdict_of_route(&o[0].method, &o[0].path),
            Some(crate::origin::Verdict::Recode)
        );
        // Claude is refused, in every mode.
        let ctx = crate::origin::DecisionContext::monitor_mode();
        assert!(
            crate::origin::verdict_refusal(
                crate::origin::Verdict::Recode,
                crate::origin::Taker::Claude,
                &ctx
            )
            .is_some()
        );
        assert!(
            ops(
                &store,
                "demo",
                &decision("Claude"),
                crate::origin::Origin::Conversation,
                None
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// DEF-106 R-8, R-10: the Review reads on a board shaped like the one that took a minute: many
/// items, prepared records, a long board history and a long code history. The counts (history
/// walks, board loads) are asserted whatever the build; the timings are `#[ignore]`d and run in
/// release by the gate: `cargo test -p kanbanr-core --release reviewgroups::perf -- --ignored`.
#[cfg(test)]
mod perf {
    use super::*;
    use crate::config::{Gate, ProjectConfig};
    use crate::docdrift::testkit::{code_commit, git_env, scratch};
    use crate::models::FeatureDefinition;
    use std::io::Write;
    use std::time::Instant;

    /// The size of a generated board and its repositories.
    struct Shape {
        /// Items waiting for their first approval.
        items: usize,
        /// Prepared records, of every kind, on those items.
        prepared: usize,
        /// Items with one manual check each.
        manual: usize,
        /// Granted delegations.
        delegations: usize,
        /// Items waiting on `docs-folded`.
        docs_folded: usize,
        /// Texts each of those items links.
        texts_each: usize,
        board_commits: usize,
        code_commits: usize,
    }

    /// This board when v0.8.0 was designed: 431 items, 449 prepared records, 11,214 board commits
    /// and 1,902 code commits.
    const THIS_BOARD: Shape = Shape {
        items: 400,
        prepared: 449,
        manual: 19,
        delegations: 2,
        docs_folded: 5,
        texts_each: 6,
        board_commits: 11_214,
        code_commits: 1_902,
    };

    /// A few of everything, for the counts in every build.
    const SMALL: Shape = Shape {
        items: 12,
        prepared: 10,
        manual: 3,
        delegations: 2,
        docs_folded: 3,
        texts_each: 3,
        board_commits: 150,
        code_commits: 60,
    };

    /// `n` commits written by one `git fast-import`: each rewrites two files under `dir`'s
    /// `folder`, so the history is long without the cost of `n` `git commit`s.
    fn fast_history(repo: &std::path::Path, folder: &str, n: usize) {
        let mut stream = String::new();
        for k in 0..n {
            let msg = format!("history {k}\n");
            let at = 1_700_000_000 + k as i64 * 60;
            stream.push_str(&format!(
                "commit refs/heads/main\ncommitter T <t@example.com> {at} +0000\ndata {}\n{msg}",
                msg.len()
            ));
            for file in [k % 211, (k * 7) % 211] {
                let body = format!("version {k}\n");
                stream.push_str(&format!(
                    "M 100644 inline {folder}/f{file}.txt\ndata {}\n{body}",
                    body.len()
                ));
            }
            stream.push('\n');
        }
        let mut child = std::process::Command::new("git")
            .args(["fast-import", "--quiet"])
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success(), "git fast-import");
        git_env(repo, &["reset", "-q", "--hard", "main"], None);
    }

    /// A manual check's name: it says what the person judges.
    const MANUAL_TEST: &str = "manual: the page reads clearly";

    fn definition(text: &str, manual: bool) -> FeatureDefinition {
        let (name, test) = if manual {
            (MANUAL_TEST, "manual")
        } else {
            ("t::x", "unit")
        };
        crate::yaml::from_str(&format!(
            "statement: {text}\nrequirements:\n  - kind: functional\n    text: THE SYSTEM SHALL x.\n    tests:\n      - name: \"{name}\"\n        kind: {test}\n        state: planned\n"
        ))
        .unwrap()
    }

    /// A board with its history and a code repository with its own, shaped by `s`. Returns the
    /// store, the board's folder and the code repository.
    fn generated_board_with_history(s: &Shape) -> (Store, std::path::PathBuf, std::path::PathBuf) {
        let dir = scratch("review-perf");
        git_env(&dir, &["init", "-q", "-b", "main"], None);
        fast_history(&dir, "history", s.board_commits);
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.cadence.releases = true;
        config.gates.insert(
            "Completed".into(),
            Gate {
                signoffs: vec!["docs-folded".into()],
                ..Default::default()
            },
        );
        store.init_project("demo", config).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        crate::charter::save(
            &store,
            "demo",
            &crate::charter::Charter {
                purpose: "p".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut plain = Vec::new();
        for i in 0..s.items {
            let f = store
                .add_feature("demo", &format!("Item {i}"), "# x", "M", None)
                .unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(definition(&format!("Item {i}"), false)),
                )
                .unwrap();
            plain.push(f.code);
        }
        for i in 0..s.manual {
            let f = store
                .add_feature("demo", &format!("Checked {i}"), "# x", "M", None)
                .unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(definition(&format!("Checked {i}"), true)),
                )
                .unwrap();
            store.approve_feature("demo", &f.code, "Ada").unwrap();
            store.move_feature("demo", &f.code, "Scheduled").unwrap();
            // A prepared result for the check, read for what it still waits on.
            let record = crate::prepared::Prepared {
                kind: "manual-test".into(),
                subject: crate::prepared::Subject {
                    item: f.code.clone(),
                    requirement: "R-1".into(),
                    test: MANUAL_TEST.into(),
                    ..Default::default()
                },
                at: format!("2026-11-{:02}T10:{:02}:00Z", 1 + i % 28, i % 60),
                ..Default::default()
            };
            crate::prepared::write_record(&store, "demo", &record).unwrap();
        }
        // Prepared records on plain items, most of them stale.
        for (n, code) in plain
            .iter()
            .cycle()
            .take(s.prepared.saturating_sub(s.manual))
            .enumerate()
        {
            let record = crate::prepared::Prepared {
                kind: "signoff".into(),
                subject: crate::prepared::Subject {
                    item: code.clone(),
                    signoff: "design-review".into(),
                    ..Default::default()
                },
                at: format!(
                    "2026-10-{:02}T{:02}:{:02}:{:02}Z",
                    1 + n % 28,
                    n / 3600 % 24,
                    n / 60 % 60,
                    n % 60
                ),
                rev: "old".into(),
                ..Default::default()
            };
            crate::prepared::write_record(&store, "demo", &record).unwrap();
        }
        let mut folded = Vec::new();
        for i in 0..s.docs_folded {
            let f = store
                .add_feature("demo", &format!("Folded {i}"), "# x", "M", None)
                .unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(definition(&format!("Folded {i}"), false)),
                )
                .unwrap();
            store.approve_feature("demo", &f.code, "Ada").unwrap();
            store.move_feature("demo", &f.code, "Scheduled").unwrap();
            let list = store.add_todo_list("demo", &f.code, "work", None).unwrap();
            store
                .add_task(
                    "demo",
                    &f.code,
                    &list.code,
                    "one",
                    None,
                    &crate::store::TaskFields::default(),
                )
                .unwrap();
            store
                .set_task_state(
                    "demo",
                    &f.code,
                    &list.code,
                    "T1",
                    crate::models::TaskState::Completed,
                )
                .unwrap();
            for j in 0..s.texts_each {
                store
                    .write_doc(
                        "demo",
                        &format!("design/d{i}-{j}.md"),
                        &format!(
                            "---\nitems: [{}]\n---\n\n# D\n\nRun with --flag-{i}-{j}.\n",
                            f.code
                        ),
                    )
                    .unwrap();
            }
            folded.push(f.code);
        }
        // Granted delegations, each with a version of decisions.yaml in the board's history.
        let stamp = "{by: Ada, at: \"2026-11-01T00:00:00Z\", origin: test}";
        let mut file = String::from("delegations:\n");
        for d in 0..s.delegations {
            file.push_str(&format!(
                "- id: DLG-{}\n  verdicts: [approve]\n  all: true\n  until: \"2099-01-01T00:00:00Z\"\n  proposed: {stamp}\n  granted: {stamp}\n",
                d + 1
            ));
            std::fs::write(store.project_dir("demo").join("decisions.yaml"), &file).unwrap();
            assert!(crate::git::commit_local(&dir, "a delegation"));
        }
        // Nothing is left to commit when a delegation's commit took it all.
        let _ = crate::git::commit_local(&dir, "the board");

        let code = scratch("review-perf-code");
        git_env(&code, &["init", "-q", "-b", "main"], None);
        fast_history(&code, "src", s.code_commits);
        for (i, item) in folded.iter().enumerate() {
            let body: String = (0..s.texts_each)
                .map(|j| format!("let a{j} = \"--flag-{i}-{j}\";\n"))
                .collect();
            code_commit(
                &code,
                &format!("lib/f{i}.rs"),
                &body,
                &format!("add\n\nRefs: kanbanr:{item}"),
                "2099-01-01T00:00:00+00:00",
            );
        }
        (store, dir, code)
    }

    /// What one cold read cost: wall time, history walks, board loads.
    struct Cost {
        took: std::time::Duration,
        walks: u64,
        loads: u64,
    }

    fn cold<T>(store: &Store, f: impl FnOnce() -> T) -> (T, Cost) {
        let (walks, loads) = (crate::git::path_history_walks_here(), store.loads());
        let at = Instant::now();
        let out = f();
        let cost = Cost {
            took: at.elapsed(),
            walks: crate::git::path_history_walks_here() - walks,
            loads: store.loads() - loads,
        };
        (out, cost)
    }

    /// The most history walks and board loads one cold summary may cost (R-8). A walk is the
    /// board's one `git log --raw -M`; loads are measured on the real cold path.
    const MAX_WALKS: u64 = 1;
    const MAX_LOADS: u64 = 3;

    /// The counts, whatever the build: a cold summary over a small board and a code repository.
    #[test]
    fn a_cold_summary_walks_history_once_and_loads_the_board_a_few_times() {
        let (store, dir, code) = generated_board_with_history(&SMALL);
        let (groups, cost) = cold(&store, || summary(&store, "demo", Some(&code)).unwrap());
        println!(
            "cold summary: {:?}, {} walks, {} loads",
            cost.took, cost.walks, cost.loads
        );
        assert!(!groups.is_empty());
        assert!(cost.walks <= MAX_WALKS, "{} history walks", cost.walks);
        assert!(cost.loads <= MAX_LOADS, "{} board loads", cost.loads);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(code);
    }

    /// R-8: a cold summary on a board shaped like this one, in release, with the code repository.
    #[test]
    #[ignore = "a timing budget: run with --ignored in release"]
    fn the_summary_answers_cold_within_budget() {
        let (store, dir, code) = generated_board_with_history(&THIS_BOARD);
        let (groups, cost) = cold(&store, || summary(&store, "demo", Some(&code)).unwrap());
        println!(
            "cold summary: {:?}, {} walks, {} loads",
            cost.took, cost.walks, cost.loads
        );
        assert!(!groups.is_empty());
        assert!(cost.walks <= MAX_WALKS, "{} history walks", cost.walks);
        assert!(cost.loads <= MAX_LOADS, "{} board loads", cost.loads);
        assert!(
            cost.took < std::time::Duration::from_millis(3_000),
            "a cold summary took {:?}",
            cost.took
        );
        // Warm: kept until the board or the code repository changes.
        let (_, warm) = cold(&store, || summary(&store, "demo", Some(&code)).unwrap());
        println!("warm summary: {:?}, {} loads", warm.took, warm.loads);
        assert_eq!(warm.loads, 0, "a warm summary loads nothing");
        assert!(
            warm.took < std::time::Duration::from_millis(50),
            "warm {:?}",
            warm.took
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(code);
    }

    /// R-10: the outline from the config alone, cold.
    #[test]
    #[ignore = "a timing budget: run with --ignored in release"]
    fn the_outline_answers_cold_within_budget() {
        let (store, dir, code) = generated_board_with_history(&THIS_BOARD);
        let (out, cost) = cold(&store, || outline(&store, "demo").unwrap());
        println!("cold outline: {:?}, {} loads", cost.took, cost.loads);
        assert!(!out.is_empty());
        assert_eq!(cost.loads, 0, "the outline reads the config alone");
        assert!(
            cost.took < std::time::Duration::from_millis(100),
            "the outline took {:?}",
            cost.took
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(code);
    }
}
