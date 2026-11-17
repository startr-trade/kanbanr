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
use std::path::Path;

/// Approve a definition never approved, lapsed, or added to.
const APPROVE: &str = "approve";
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
        SYSTEM_BASELINE | APPROVE | RATIFY | WHICH_RELEASE | MANUAL | DELEGATION | DELEGATED | ADR
    ) || signoff_of_key(key).is_some()
}

/// `n` and the noun, plural when `n` is not one.
fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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
}

impl Board {
    fn load(store: &Store, p: &str, repo: Option<&Path>) -> Result<Board> {
        let decisions = crate::decisions::view(store, p, repo, None, &crate::now_rfc3339())?;
        let meta = store.load_meta(p)?;
        let baseline = match crate::systembaseline::load(store, p)? {
            Some(_) => Some(crate::systembaseline::state(store, &meta, repo)?),
            None => None,
        };
        Ok(Board {
            baseline,
            entries: crate::review::pending(store, p, repo)?,
            manual: crate::manual::waiting(store, p)?,
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
            prepared: crate::prepared::list(
                store,
                p,
                &crate::prepared::ListQuery::default(),
                repo,
            )?,
            mode: decisions.mode,
            meta,
            releases: crate::releases::load(store, p)?,
        })
    }

    /// Every group key in the fixed order: system-baseline, approve, ratify, the sign-offs in the
    /// order the process's gates name them, which-release, manual, delegation, delegated, adr.
    fn keys(&self) -> Vec<String> {
        let config = &self.meta.config;
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
        let mut others: Vec<String> = self
            .entries
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
            [WHICH_RELEASE, MANUAL, DELEGATION, DELEGATED, ADR]
                .into_iter()
                .map(String::from),
        );
        keys
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
        if e.finished {
            pills.push("finished".into());
        }
        push_record_pills(&mut pills, record);
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
            },
            counted: true,
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
            },
            counted: true,
        }]
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
            _ => self
                .entries
                .iter()
                .filter(|e| group_keys(e).iter().any(|k| k == key))
                .map(|e| self.entry_row(key, e))
                .collect(),
        }
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

// ---- reads -------------------------------------------------------------------------------------

/// Every non-empty decision group of project `p`, counted, in the fixed order.
pub fn summary(store: &Store, p: &str, repo: Option<&Path>) -> Result<Vec<GroupCount>> {
    let board = Board::load(store, p, repo)?;
    Ok(board
        .keys()
        .into_iter()
        .filter_map(|key| {
            let rows = board.rows(&key);
            let counted: Vec<&ReviewRow> =
                rows.iter().filter(|b| b.counted).map(|b| &b.row).collect();
            if counted.is_empty() {
                return None;
            }
            let (label, action) = label_and_action(&key);
            Some(GroupCount {
                count: counted.len(),
                ready: counted.iter().filter(|r| r.blocked.is_empty()).count(),
                prepared: counted.iter().filter(|r| !r.prepared.is_empty()).count(),
                action: action.to_string(),
                label,
                key,
            })
        })
        .collect())
}

/// One group's rows, in the default order. An unknown key is refused as no such route.
pub fn slice(store: &Store, p: &str, key: &str, repo: Option<&Path>) -> Result<GroupSlice> {
    if !is_known(key) {
        return Err(unknown_group(key));
    }
    let board = Board::load(store, p, repo)?;
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
            let m = crate::manual::waiting(store, p)?
                .into_iter()
                .find(|m| m.code == code && m.requirement == requirement && m.test == test)
                .ok_or_else(missing)?;
            out.prepared = open("manual-test", code, "", requirement, test)?;
            out.manual = Some(m);
        }
        DELEGATION | DELEGATED => {
            let view = crate::decisions::view(store, p, repo, None, &crate::now_rfc3339())?;
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

    fn define(store: &Store, title: &str, def: FeatureDefinition) -> String {
        let f = store.add_feature("demo", title, "", "M", None).unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        f.code
    }

    fn statement(s: &str) -> FeatureDefinition {
        FeatureDefinition {
            statement: s.into(),
            ..Default::default()
        }
    }

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
        }
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
            .approve_feature_as("demo", &checked, "Claude", "delegated", "DLG-1")
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
}
