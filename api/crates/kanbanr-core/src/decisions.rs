//! Where decisions are taken, and what Claude may decide under a delegation (FTR-028).
//!
//! A project has one decision mode, kept with its delegations and the reviews of what was decided
//! under them in `projects/<id>/decisions.yaml`. Every list in it is append-only. The person
//! records the mode and grants delegations; Claude may propose a delegation and, when one in force
//! covers a verdict, take that verdict with `--under DLG-n`. A decision taken that way is
//! recorded as `delegated` and waits for the person to keep or undo it.
//!
//! [`authorize`] is the one table that says who may take which verdict. It is pure; the write
//! recipe calls it under the board lock. The origin it judges is an honest label, not
//! authentication (ADR-0001).

use crate::models::{Extra, FeatureItem, SignoffVerdict};
use crate::origin::{Origin, delegation_from_body, pass_from_body};
use crate::store::{Project, Store};
use crate::{CoreError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const DECISIONS_FILE: &str = "decisions.yaml";
/// The longest a delegation may run, from the day it is proposed.
pub const MAX_DELEGATION_DAYS: i64 = 90;
/// How long a delegation runs when the person names no end (the default sprint length).
pub const DEFAULT_DELEGATION_DAYS: i64 = 14;

/// Where a project's decisions are taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// On the monitor, by the person: today's behaviour, and the default.
    #[default]
    Monitor,
    /// On the monitor, and by Claude within a granted delegation.
    Delegated,
    /// In the conversation (FTR-035; recording it is refused until that is built).
    Conversation,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Monitor => "monitor",
            Mode::Delegated => "delegated",
            Mode::Conversation => "conversation",
        }
    }

    /// Exact and lower case; anything else is `None`.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "monitor" => Some(Mode::Monitor),
            "delegated" => Some(Mode::Delegated),
            "conversation" => Some(Mode::Conversation),
            _ => None,
        }
    }
}

/// Who did something, when, and from where; a grant also pins the revision it granted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Stamp {
    pub by: String,
    pub at: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One entry of the mode history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModeChange {
    pub mode: Mode,
    pub by: String,
    pub at: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What a delegation is in, at a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DelegationState {
    Proposed,
    Granted,
    Ended,
    Revoked,
}

impl DelegationState {
    pub fn as_str(self) -> &'static str {
        match self {
            DelegationState::Proposed => "proposed",
            DelegationState::Granted => "granted",
            DelegationState::Ended => "ended",
            DelegationState::Revoked => "revoked",
        }
    }
}

/// A delegation: the verdicts Claude may take, on which items, until when. Never edited once
/// proposed; a wider or longer one is a new proposal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Delegation {
    pub id: String,
    /// `approve`, `ratify` or `signoff:<name>`.
    pub verdicts: Vec<String>,
    /// Statuses the item is in when the verdict is taken; empty is any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// Item codes; empty is any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// Item kinds; empty is any. An item with no kind is a `feature`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// A milestone code; empty is any.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub milestone: String,
    /// Names no items, kinds or milestone on purpose: every item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// RFC 3339; the delegation is in force until then.
    pub until: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub proposed: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<Stamp>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A person's review of one delegated decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Review {
    /// `<CODE>/<approve|ratify|signoff>/<name or ->/<record at>`.
    pub decision: String,
    pub delegation: String,
    pub verdict: ReviewVerdict,
    pub by: String,
    pub at: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ReviewVerdict {
    Kept,
    Undone,
}

/// The whole `decisions.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Decisions {
    #[serde(default)]
    pub modes: Vec<ModeChange>,
    #[serde(default)]
    pub delegations: Vec<Delegation>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

fn path(store: &Store, p: &str) -> PathBuf {
    store.project_dir(p).join(DECISIONS_FILE)
}

/// A project's decisions. No file, or an empty one, is a normal state: the default mode and
/// nothing delegated. A file that does not parse is an error and is never rewritten (ADR-0005).
pub fn load(store: &Store, p: &str) -> Result<Decisions> {
    match std::fs::read_to_string(path(store, p)) {
        Ok(s) if !s.trim().is_empty() => Ok(crate::yaml::from_str(&s)?),
        _ => Ok(Decisions::default()),
    }
}

fn save(store: &Store, p: &str, d: &Decisions) -> Result<()> {
    crate::atomic::write(path(store, p), crate::yaml::to_string(d)?)?;
    Ok(())
}

impl Decisions {
    /// The last recorded mode; `monitor` when none was.
    pub fn mode(&self) -> Mode {
        self.modes.last().map(|m| m.mode).unwrap_or_default()
    }
}

fn instant(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s.trim(), &Rfc3339).ok()
}

/// Is `until` still ahead of `now`? A value that is not a time is never in force.
fn ahead(until: &str, now: &str) -> bool {
    match (instant(until), instant(now)) {
        (Some(u), Some(n)) => n < u,
        _ => false,
    }
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|s| s.trim().to_string()).collect();
    v.sort();
    v.dedup();
    v
}

impl Delegation {
    /// A hash of what the delegation lets Claude do (its content, not who stamped it), over
    /// canonical JSON with the lists sorted. A grant is pinned to it.
    pub fn rev(&self) -> String {
        let content = serde_json::json!({
            "verdicts": sorted(&self.verdicts),
            "steps": sorted(&self.steps),
            "items": sorted(&self.items),
            "kinds": sorted(&self.kinds),
            "milestone": self.milestone.trim(),
            "all": self.all,
            "until": self.until.trim(),
            "note": self.note.trim(),
        });
        crate::hash::stable_hash(&crate::hash::canonical_json(&content))
    }

    pub fn state(&self, now: &str) -> DelegationState {
        if self.revoked.is_some() {
            DelegationState::Revoked
        } else if !ahead(&self.until, now) {
            DelegationState::Ended
        } else if self.granted.is_none() {
            DelegationState::Proposed
        } else {
            DelegationState::Granted
        }
    }

    /// Why this delegation does not let Claude take `v` on `item` at `now`; `None` when it does.
    fn why_not(&self, v: &Verdict, item: &FeatureItem, now: &str) -> Option<String> {
        let id = &self.id;
        match self.state(now) {
            DelegationState::Granted => {}
            DelegationState::Proposed => {
                return Some(format!(
                    "{id} is only proposed: the person has not granted it"
                ));
            }
            DelegationState::Ended => return Some(format!("{id} has ended ({})", self.until)),
            DelegationState::Revoked => return Some(format!("{id} was revoked")),
        }
        self.out_of_scope(v, item)
    }

    /// Is `v` on `item` inside what this delegation names, whatever its state or time? The selector
    /// half of [`Delegation::why_not`]: the verdict is named and the steps, items (with earlier
    /// codes), kinds and milestone match (DEF-104).
    pub fn in_scope(&self, v: &Verdict, item: &FeatureItem) -> bool {
        self.out_of_scope(v, item).is_none()
    }

    fn out_of_scope(&self, v: &Verdict, item: &FeatureItem) -> Option<String> {
        let id = &self.id;
        let name = v.name();
        if !self.verdicts.iter().any(|x| x.trim() == name) {
            return Some(format!("{id} does not cover `{name}`"));
        }
        let kind = item.kind.as_deref().unwrap_or("feature");
        let any = |list: &[String], value: &str| {
            list.is_empty() || list.iter().any(|x| x.trim() == value)
        };
        if !any(&self.steps, &item.status) {
            return Some(format!(
                "{id} does not cover an item in {} (it covers {})",
                item.status,
                self.steps.join(", ")
            ));
        }
        // FTR-056: a grant names the code it was given under; the item's earlier codes count.
        if !item.codes().any(|c| any(&self.items, c)) {
            return Some(format!("{id} does not cover {}", item.code));
        }
        if !any(&self.kinds, kind) {
            return Some(format!("{id} does not cover a {kind}"));
        }
        let milestone = self.milestone.trim();
        if !milestone.is_empty() && item.milestone != milestone {
            return Some(format!("{id} does not cover milestone {}", item.milestone));
        }
        None
    }

    /// Does this delegation let Claude take `v` on `item` at `now`? True when it is granted and in
    /// force, names the verdict, and every selector it sets matches.
    pub fn covers(&self, v: &Verdict, item: &FeatureItem, now: &str) -> bool {
        self.why_not(v, item, now).is_none()
    }
}

/// The three item verdicts a delegation can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictKind {
    Approve,
    Ratify,
    Signoff,
}

/// A verdict on an item, read from the route that records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub code: String,
    /// The sign-off's name; empty for an approval or ratification.
    pub signoff: String,
}

impl Verdict {
    /// How a delegation names it: `approve`, `ratify` or `signoff:<name>`.
    pub fn name(&self) -> String {
        match self.kind {
            VerdictKind::Approve => "approve".to_string(),
            VerdictKind::Ratify => "ratify".to_string(),
            VerdictKind::Signoff => format!("signoff:{}", self.signoff),
        }
    }
}

fn segments(path: &str) -> Vec<&str> {
    path.split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

/// `%XX` escapes decoded, as a route's name segment arrives.
fn decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 3 <= b.len()
            && let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(x);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The delegable item verdict this request records, if it is one: approve, ratify and sign-off.
/// A withdrawal, an ADR verdict and a release verdict are never delegable.
pub fn verdict_of(method: &str, path: &str) -> Option<Verdict> {
    if method != "POST" {
        return None;
    }
    match segments(path).as_slice() {
        ["projects", _, "features", code, "approve"] => Some(Verdict {
            kind: VerdictKind::Approve,
            code: decoded(code),
            signoff: String::new(),
        }),
        ["projects", _, "features", code, "ratify"] => Some(Verdict {
            kind: VerdictKind::Ratify,
            code: decoded(code),
            signoff: String::new(),
        }),
        ["projects", _, "features", code, "signoff", name] => Some(Verdict {
            kind: VerdictKind::Signoff,
            code: decoded(code),
            signoff: decoded(name),
        }),
        _ => None,
    }
}

/// Why Claude may not accept a baseline revision (FTR-033): the person-only wording, shared by
/// the guard and the write path.
pub const ACCEPT_BASELINE_REFUSAL: &str = "accepting a baseline revision is the person's own, on the Baseline page or in their \
     terminal: no mode or delegation lets Claude take it. Propose it (`kanbanr baseline \
     propose`) and tell the person where it waits.";

/// Why Claude may not accept the system baseline (FTR-006): the person's in every mode; Claude
/// prepares the acceptance for them. Shared by the guard and the write path.
pub const ACCEPT_SYSTEM_BASELINE_REFUSAL: &str = "a verdict is the person's own, recorded on the \
     monitor, never by Claude (FTR-024). The system baseline is accepted by the person on the \
     Review page. If they agreed to it, prepare it: `kanbanr prepare system-baseline --note \"…\" \
     --source \"doc:architecture/context.md\"`.";

/// Is this a route only the person may use: the mode, a grant, a review, the answer to a
/// prepared record (FTR-034), and accepting a baseline revision (FTR-033)?
pub fn is_person_only(method: &str, path: &str) -> bool {
    matches!(
        (method, segments(path).as_slice()),
        ("PUT", ["projects", _, "decisions", "mode"])
            | (
                "POST",
                ["projects", _, "baseline", "revisions", _, "accept"]
            )
            | ("POST", ["projects", _, "prepared", _, "confirm" | "reject"])
            // The system baseline's acceptance (FTR-006).
            | ("POST", ["projects", _, "catalog", "baseline", "accept"])
            | (
                "POST",
                ["projects", _, "decisions", "delegations", _, "grant"]
            )
            | ("POST", ["projects", _, "decisions", "reviews"])
    )
}

/// What [`authorize`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorized {
    /// Record it as the front end labelled it.
    AsIs,
    /// Record it as `delegated`, by Claude, under this delegation.
    Delegated { delegation: String },
    /// Record it as the conversation's, by Claude, carrying the person's answer: the pass is
    /// matched and is consumed with the verdict (FTR-035).
    Heard { pass: String },
}

/// Who may take what: the one table, behind [`crate::origin::verdict_refusal`], which is the only
/// caller outside this module's tests. `Ok` says how to record the request; `Err` is the refusal,
/// in words that name the mode, where the person decides and, for Claude, any delegation that
/// would cover the verdict. Routes that are neither a verdict nor person-only pass as they are.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authorize(
    d: &Decisions,
    project: &Project,
    method: &str,
    path: &str,
    body: Option<&Value>,
    origin: Origin,
    now: &str,
    pass: Option<&crate::passes::Pass>,
) -> std::result::Result<Authorized, String> {
    let person_only = is_person_only(method, path);
    let verdict = verdict_of(method, path);
    if !person_only && verdict.is_none() && !crate::origin::is_verdict_route(method, path) {
        return Ok(Authorized::AsIs);
    }
    let mode = d.mode();
    let named = body.map(delegation_from_body).unwrap_or_default();
    let named_pass = body.map(pass_from_body).unwrap_or_default();
    let by_the_person = matches!(origin, Origin::Monitor | Origin::Terminal);
    if person_only {
        // The person can never lock themselves out: both of their front ends, in every mode.
        return if by_the_person && named.is_empty() && named_pass.is_empty() {
            Ok(Authorized::AsIs)
        } else if by_the_person && named.is_empty() {
            Err("`--pass` is Claude's; a person needs none".to_string())
        } else if by_the_person {
            Err("`--under` is Claude's; a person needs none".to_string())
        } else if matches!(
            segments(path).as_slice(),
            ["projects", _, "catalog", "baseline", "accept"]
        ) {
            Err(ACCEPT_SYSTEM_BASELINE_REFUSAL.to_string())
        } else if matches!(segments(path).as_slice(), ["projects", _, "baseline", ..]) {
            Err(ACCEPT_BASELINE_REFUSAL.to_string())
        } else if matches!(segments(path).as_slice(), ["projects", _, "prepared", ..]) {
            Err(
                "answering a prepared record is the person's own, recorded on the monitor, never \
                 by Claude (FTR-024, FTR-034)."
                    .to_string(),
            )
        } else {
            Err(
                "this is the person's own, on the Review page or in their terminal: \
                 Claude cannot set the decision mode, grant a delegation or review a decision"
                    .to_string(),
            )
        };
    }
    if by_the_person {
        if !named.is_empty() {
            return Err("`--under` is Claude's; a person needs none".to_string());
        }
        if !named_pass.is_empty() {
            return Err("`--pass` is Claude's; a person needs none".to_string());
        }
        if mode == Mode::Conversation && verdict.is_some() && origin == Origin::Monitor {
            return Err(
                "decisions are taken in the conversation in this project, not on the monitor"
                    .to_string(),
            );
        }
        return Ok(Authorized::AsIs);
    }
    // From here on the verdict is Claude's: a session, or a record already labelled delegated.
    let place = match mode {
        Mode::Monitor => "on the monitor's Review page",
        Mode::Delegated => "on the monitor's Review page, or by a delegation they granted",
        Mode::Conversation => {
            "in the conversation, by asking with `kanbanr decisions question <verdict> <CODE>`"
        }
    };
    let covering = |v: &Verdict| -> String {
        project
            .feature(&v.code)
            .ok()
            .and_then(|item| {
                d.delegations
                    .iter()
                    .find(|g| g.covers(v, item, now))
                    .map(|g| {
                        format!(
                            " Delegation {} covers it: take it with `--under {}`.",
                            g.id, g.id
                        )
                    })
            })
            .unwrap_or_default()
    };
    let Some(v) = verdict else {
        return Err(format!(
            "this verdict is the person's, {place}, and no delegation can cover it \
             (mode: {})",
            mode.as_str()
        ));
    };
    if !named_pass.is_empty() {
        if !named.is_empty() {
            return Err(format!(
                "a verdict names a pass or a delegation, not both ({named_pass} and {named})"
            ));
        }
        if mode != Mode::Conversation {
            return Err(format!(
                "this project is not in conversation mode (mode: {}), so a pass cannot be used;                  decisions are taken {place}",
                mode.as_str()
            ));
        }
        let Some(pass) = pass.filter(|p| p.id == named_pass) else {
            return Err(format!(
                "pass {named_pass} is unknown, already used or expired (passes last {} minutes): \
                 ask again with `kanbanr decisions question {} {}`",
                crate::passes::TTL_MINUTES,
                v.name().split(':').next().unwrap_or("approve"),
                v.code
            ));
        };
        if pass.project != project.id {
            return Err(format!("pass {} was made for another project", pass.id));
        }
        if pass.code != v.code {
            return Err(format!(
                "pass {} was made for {}, not {}",
                pass.id, pass.code, v.code
            ));
        }
        if pass.verdict != v.name() {
            return Err(format!(
                "pass {} was made for `{}`, not `{}`",
                pass.id,
                pass.verdict,
                v.name()
            ));
        }
        let rev = body
            .and_then(|b| b.get("rev"))
            .and_then(|r| r.as_str())
            .map(str::trim)
            .unwrap_or("");
        if rev.is_empty() || rev != pass.rev {
            return Err(
                "a conversation verdict names the revision it was asked on: pass `--rev` as the \
                 question's tag has it"
                    .to_string(),
            );
        }
        return Ok(Authorized::Heard {
            pass: pass.id.clone(),
        });
    }
    if named.is_empty() {
        let hint = if mode == Mode::Delegated {
            covering(&v)
        } else {
            String::new()
        };
        return Err(format!(
            "this project takes decisions {place} (mode: {}); `{}` on {} is the person's.{hint}",
            mode.as_str(),
            v.name(),
            v.code
        ));
    }
    if mode != Mode::Delegated {
        return Err(format!(
            "this project is not in delegated mode (mode: {}), so {named} cannot be used; \
             decisions are taken {place}",
            mode.as_str()
        ));
    }
    let Some(g) = d.delegations.iter().find(|g| g.id == named) else {
        return Err(format!("there is no delegation {named} in this project"));
    };
    let item = project
        .feature(&v.code)
        .map_err(|_| format!("there is no item {}", v.code))?;
    // Ratifying an override Claude made is the person's answer to it (DEF-043): no delegation
    // covers it. Claude prepares the ratification; the person confirms it.
    if v.kind == VerdictKind::Ratify && !crate::readiness::open_overrides(item).is_empty() {
        return Err(format!(
            "{} has an override Claude made that waits for the person's ratification (DEF-043), \
             so `ratify` on {} stays the person's: prepare it with `kanbanr prepare ratify {} \
             --reason \"…\" --source S`",
            v.code, v.code, v.code
        ));
    }
    match g.why_not(&v, item, now) {
        None => Ok(Authorized::Delegated {
            delegation: g.id.clone(),
        }),
        Some(why) => Err(format!(
            "{why}, so `{}` on {} stays the person's",
            v.name(),
            v.code
        )),
    }
}

/// What a new delegation asks for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NewDelegation {
    #[serde(default)]
    pub verdicts: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub milestone: String,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub until: String,
    #[serde(default)]
    pub note: String,
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T> {
    Err(CoreError::Unsupported(msg.into()))
}

/// A decision of the person's is stamped by one of their own front ends, whatever route reached
/// here.
pub(crate) fn person_stamp(stamp: &Stamp, what: &str) -> Result<()> {
    match Origin::parse(&stamp.origin) {
        Some(Origin::Monitor | Origin::Terminal) => Ok(()),
        _ => unsupported(format!(
            "{what} is the person's own, on the Review page or in their terminal"
        )),
    }
}

/// Record a proposal. It is inert until the person grants it. Refused when it names a verdict that
/// can never be delegated, a sign-off no gate of the project asks for, no selector, or an end that
/// is missing, past, or more than [`MAX_DELEGATION_DAYS`] ahead.
pub fn propose(store: &Store, p: &str, new: NewDelegation, stamp: Stamp) -> Result<Delegation> {
    let project = store.load_meta(p)?;
    let verdicts = sorted(&new.verdicts);
    if verdicts.is_empty() {
        return unsupported(
            "a delegation needs at least one verdict: approve, ratify or signoff:<name>",
        );
    }
    let gates = project.config.effective_gates();
    for v in &verdicts {
        match v.as_str() {
            "approve" | "ratify" => {}
            s if s.starts_with("signoff:") && !s["signoff:".len()..].trim().is_empty() => {
                let name = s["signoff:".len()..].trim();
                if !gates.values().any(|g| g.signoffs.iter().any(|x| x == name)) {
                    return unsupported(format!(
                        "no gate of this project asks for sign-off `{name}`, so there is nothing to delegate"
                    ));
                }
            }
            other => {
                return unsupported(format!(
                    "`{other}` cannot be delegated: only approve, ratify and signoff:<name> can \
                     (releases, withdrawals, ADR verdicts and decisions routes stay the person's)"
                ));
            }
        }
    }
    let (steps, items, kinds) = (sorted(&new.steps), sorted(&new.items), sorted(&new.kinds));
    let milestone = new.milestone.trim().to_string();
    if items.is_empty() && kinds.is_empty() && milestone.is_empty() && !new.all {
        return unsupported(
            "a delegation must name items, kinds or a milestone, or say it covers every item (`all`)",
        );
    }
    for s in &steps {
        if !project.config.statuses.iter().any(|x| x == s) {
            return unsupported(format!("`{s}` is not a status of this project"));
        }
    }
    for c in &items {
        project.feature(c)?;
    }
    if !milestone.is_empty() && !project.milestones.iter().any(|m| m.code == milestone) {
        return unsupported(format!("there is no milestone {milestone}"));
    }
    let now = if stamp.at.is_empty() {
        crate::now_rfc3339()
    } else {
        stamp.at.clone()
    };
    let (Some(now_t), Some(until_t)) = (instant(&now), instant(&new.until)) else {
        return unsupported(
            "a delegation needs an end time (RFC 3339), such as 2026-11-26T23:59:59+00:00",
        );
    };
    if until_t <= now_t {
        return unsupported("a delegation must end later than now");
    }
    if until_t > now_t + time::Duration::days(MAX_DELEGATION_DAYS) {
        return unsupported(format!(
            "a delegation lasts at most {MAX_DELEGATION_DAYS} days; propose a shorter one"
        ));
    }
    let mut d = load(store, p)?;
    let next = d
        .delegations
        .iter()
        .filter_map(|g| g.id.strip_prefix("DLG-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let delegation = Delegation {
        id: format!("DLG-{next:03}"),
        verdicts,
        steps,
        items,
        kinds,
        milestone,
        all: new.all,
        until: new.until.trim().to_string(),
        note: new.note.trim().to_string(),
        proposed: stamp,
        ..Default::default()
    };
    d.delegations.push(delegation.clone());
    save(store, p, &d)?;
    Ok(delegation)
}

/// The statuses an item is in when `signoff` is offered on a forward move: every status with a
/// transition into a later status (in the order `statuses` lists them) whose gate asks for it. A
/// delegation's `steps` are the statuses the item is in when the verdict is taken, so these are
/// what make a default delegation cover anything. A back-move into the gate is left out: "forward
/// moves ok; for backward moves ask before making changes" (decided by the maintainer, 13 Nov
/// 2026).
fn offered_from(config: &crate::config::ProjectConfig, signoff: &str) -> Vec<String> {
    let gates = config.effective_gates();
    let at = |s: &str| config.statuses.iter().position(|x| x == s);
    config
        .statuses
        .iter()
        .filter(|s| {
            config.transitions.get(*s).is_some_and(|to| {
                to.iter().any(|t| {
                    at(t) > at(s)
                        && gates
                            .get(t)
                            .is_some_and(|g| g.signoffs.iter().any(|x| x == signoff))
                })
            })
        })
        .cloned()
        .collect()
}

/// What setup proposes for a process (R-5): a mode and default delegations, worked out from stage
/// roles so saved and custom processes get them too. Only forward moves are delegated (decided by
/// the maintainer, 13 Nov 2026; see `offered_from`). The Defaults table names each sign-off by the
/// stage whose move it guards ("at the integrate-role stage", "at Migration"): the delegation's
/// steps are the stages that move leaves from, where the item is when it is signed. Approvals,
/// ratifications, ADRs and release verdicts are never proposed; a sign-off the process does not
/// ask for is left out. Each runs [`DEFAULT_DELEGATION_DAYS`] from `today`. The mode is
/// `delegated` when anything is proposed, else `monitor`.
pub fn proposed_defaults(
    config: &crate::config::ProjectConfig,
    process: Option<&str>,
    today: &str,
) -> (Mode, Vec<NewDelegation>) {
    use crate::method::{DESIGN_REVIEW, DOCS_FOLDED};
    let process = process
        .map(str::to_string)
        .or_else(|| config.process.as_ref().map(|p| p.name.clone()))
        .unwrap_or_default();
    let until = until_of("", today).unwrap_or_default();
    let wanted: &[&str] = match process.as_str() {
        // A design-control project keeps its reviews with a person.
        "design-control" => &[],
        // Business Arch and System Design stay the person's (the spec).
        "togaf" => &[DOCS_FOLDED],
        _ => &[DESIGN_REVIEW, DOCS_FOLDED],
    };
    let mut out = Vec::new();
    for &name in wanted {
        let steps = offered_from(config, name);
        if steps.is_empty() {
            continue;
        }
        let small = name == DESIGN_REVIEW;
        out.push(NewDelegation {
            verdicts: vec![format!("signoff:{name}")],
            steps,
            kinds: if small {
                vec!["chore".into(), "defect".into()]
            } else {
                Vec::new()
            },
            all: !small,
            until: until.clone(),
            note: if small {
                "Design reviews of defects and chores (proposed at setup)".into()
            } else {
                "The docs fold-in of every item (proposed at setup)".into()
            },
            ..Default::default()
        });
    }
    let mode = if out.is_empty() {
        Mode::Monitor
    } else {
        Mode::Delegated
    };
    (mode, out)
}

/// What `POST /projects/{p}/decisions/defaults` answers with `dry_run`: the proposal, unsaved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposedDefaults {
    pub mode: Mode,
    pub delegations: Vec<NewDelegation>,
}

/// Propose the process's default delegations (inert until the person grants them); with
/// `dry_run`, only say what they would be.
pub fn propose_defaults(
    store: &Store,
    p: &str,
    process: Option<&str>,
    stamp: Stamp,
    dry_run: bool,
) -> Result<std::result::Result<Vec<Delegation>, ProposedDefaults>> {
    let config = store.load_meta(p)?.config;
    let now = if stamp.at.is_empty() {
        crate::now_rfc3339()
    } else {
        stamp.at.clone()
    };
    let (mode, delegations) = proposed_defaults(&config, process, &now);
    if dry_run {
        return Ok(Err(ProposedDefaults { mode, delegations }));
    }
    delegations
        .into_iter()
        .map(|d| propose(store, p, d, stamp.clone()))
        .collect::<Result<Vec<_>>>()
        .map(Ok)
}

/// Where a delegation's grant came from, as the board's history shows it (T9, DEF-080).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// kanbanr wrote the grant, in a committed file.
    ThroughKanbanr,
    /// Written into the file by hand, or not committed.
    Elsewhere,
    /// The board's history cannot be read, so nobody can tell: why, naming the repository.
    Unverifiable(String),
}

/// Where delegation `id`'s grant came from (T9). Through kanbanr only when `decisions.yaml` has
/// no change outside a commit, and the first commit in its history that shows `id` granted
/// carries `Kanbanr-Origin: monitor` or `terminal`, the trailer kanbanr's write recipe adds to a
/// grant. A history that cannot be read is unverifiable, and says why (DEF-080).
pub fn grant_provenance(store: &Store, p: &str, id: &str) -> Provenance {
    let rel = format!("projects/{p}/{DECISIONS_FILE}");
    let dir = store.data_dir();
    if let Err(dirty) = crate::git::is_clean(dir)
        && dirty.iter().any(|f| f == &rel)
    {
        return Provenance::Elsewhere;
    }
    #[cfg(test)]
    if crate::git::shared_off() {
        return grant_provenance_unshared(dir, &rel, id);
    }
    // One walk of the board's history, shared with every other reader (DEF-106).
    let history = match crate::git::path_history(dir) {
        Ok(h) => h,
        Err(e) => return Provenance::Unverifiable(e.to_string()),
    };
    let granted = |blob: &str| {
        history.blob_text(dir, blob).is_some_and(|text| {
            crate::yaml::from_str::<Decisions>(&text)
                .ok()
                .is_some_and(|d| {
                    d.delegations
                        .iter()
                        .any(|g| g.id == id && g.granted.is_some())
                })
        })
    };
    let versions = history.versions(&rel);
    let Some(first) = versions
        .iter()
        .rev()
        .find(|v| !v.blob.is_empty() && granted(&v.blob))
    else {
        return Provenance::Elsewhere;
    };
    let message = history.message(dir, &first.sha).unwrap_or_default();
    if through_kanbanr(&message) {
        Provenance::ThroughKanbanr
    } else {
        Provenance::Elsewhere
    }
}

/// Does a commit message carry the trailer kanbanr's write recipe adds to a grant?
fn through_kanbanr(message: &str) -> bool {
    message.lines().any(|l| {
        matches!(
            l.trim(),
            "Kanbanr-Origin: monitor" | "Kanbanr-Origin: terminal"
        )
    })
}

/// Today's reading of the grant's history, one `file_history` walk per call: the reference the
/// shared history is checked against.
#[cfg(test)]
pub(crate) fn grant_provenance_unshared(dir: &std::path::Path, rel: &str, id: &str) -> Provenance {
    let history = match crate::git::file_history(dir, rel) {
        Ok(h) => h,
        Err(e) => return Provenance::Unverifiable(e.to_string()),
    };
    let Some(first) = history.iter().rev().find(|v| {
        crate::yaml::from_str::<Decisions>(&v.content)
            .ok()
            .is_some_and(|d| {
                d.delegations
                    .iter()
                    .any(|g| g.id == id && g.granted.is_some())
            })
    }) else {
        return Provenance::Elsewhere;
    };
    if through_kanbanr(&first.message) {
        Provenance::ThroughKanbanr
    } else {
        Provenance::Elsewhere
    }
}

/// Was delegation `id` granted through kanbanr (T9)? A grant written into the file by hand, one
/// not committed, or one whose history cannot be read is not honoured: it fails closed (DEF-080).
pub fn granted_through_kanbanr(store: &Store, p: &str, id: &str) -> bool {
    grant_provenance(store, p, id) == Provenance::ThroughKanbanr
}

/// A delegation's end as a person gives it: `YYYY-MM-DD` is the end of that day (UTC), RFC 3339
/// is taken as it is, and nothing is [`DEFAULT_DELEGATION_DAYS`] from `now`, at the end of that
/// day.
pub fn until_of(given: &str, now: &str) -> Result<String> {
    let given = given.trim();
    let end_of = |d: time::Date| format!("{d}T23:59:59+00:00");
    if given.is_empty() {
        let Some(now) = instant(now) else {
            return unsupported(format!("'{now}' is not a time"));
        };
        let day = (now + time::Duration::days(DEFAULT_DELEGATION_DAYS))
            .to_offset(time::UtcOffset::UTC)
            .date();
        return Ok(end_of(day));
    }
    if instant(given).is_some() {
        return Ok(given.to_string());
    }
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    match time::Date::parse(given, &fmt) {
        Ok(d) => Ok(end_of(d)),
        Err(_) => unsupported(format!(
            "--until takes a date (YYYY-MM-DD) or an RFC 3339 time, not '{given}'"
        )),
    }
}

fn find<'a>(d: &'a mut Decisions, id: &str) -> Result<&'a mut Delegation> {
    d.delegations
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| CoreError::Unsupported(format!("there is no delegation {id}")))
}

/// The person grants a proposal, pinned to the `rev` they saw. Refused unless it is proposed and
/// not ended, and unless `rev` is the proposal's current one.
pub fn grant(store: &Store, p: &str, id: &str, rev: &str, mut stamp: Stamp) -> Result<Delegation> {
    person_stamp(&stamp, "granting a delegation")?;
    let mut d = load(store, p)?;
    let now = if stamp.at.is_empty() {
        crate::now_rfc3339()
    } else {
        stamp.at.clone()
    };
    let g = find(&mut d, id)?;
    match g.state(&now) {
        DelegationState::Proposed => {}
        other => {
            return unsupported(format!(
                "{id} is {}, so it cannot be granted",
                other.as_str()
            ));
        }
    }
    if g.rev() != rev.trim() {
        return unsupported(format!(
            "{id} changed since you saw it: read it again before granting"
        ));
    }
    stamp.rev = g.rev();
    g.granted = Some(stamp);
    let out = g.clone();
    save(store, p, &d)?;
    Ok(out)
}

/// Anyone may revoke, with a reason. A repeat is refused. The delegation stays in the file.
pub fn revoke(
    store: &Store,
    p: &str,
    id: &str,
    reason: &str,
    mut stamp: Stamp,
) -> Result<Delegation> {
    if reason.trim().is_empty() {
        return unsupported("revoking a delegation needs a reason");
    }
    let mut d = load(store, p)?;
    let g = find(&mut d, id)?;
    if g.revoked.is_some() {
        return unsupported(format!("{id} is already revoked"));
    }
    stamp.reason = reason.trim().to_string();
    g.revoked = Some(stamp);
    let out = g.clone();
    save(store, p, &d)?;
    Ok(out)
}

/// A decision waiting for someone, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Monitor,
    Conversation,
    Delegated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waiting {
    pub code: String,
    pub what: String,
    pub place: Place,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The answer to a mode change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModeChanged {
    pub mode: Mode,
    /// The mode already stood, so nothing was recorded. Absent when it was recorded, so the write
    /// recipe commits it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub already_recorded: bool,
    pub waiting: Vec<Waiting>,
}

/// Record a new mode, by the person. Leaving `conversation` drops every pass of the project
/// (FTR-035). A repeat of the standing mode records nothing. Either way the answer lists what waits for a
/// decision, each in its place under the mode now standing.
pub fn set_mode(
    store: &Store,
    p: &str,
    mode: Mode,
    reason: &str,
    stamp: Stamp,
    review_url: Option<&str>,
) -> Result<ModeChanged> {
    person_stamp(&stamp, "changing the decision mode")?;
    let mut d = load(store, p)?;
    let now = if stamp.at.is_empty() {
        crate::now_rfc3339()
    } else {
        stamp.at.clone()
    };
    if d.mode() == mode {
        return Ok(ModeChanged {
            mode,
            already_recorded: true,
            waiting: waiting(store, p, None, &d, review_url, &now)?,
        });
    }
    // A pass is a ticket for the mode it was made in: leaving conversation mode drops them all.
    if d.mode() == Mode::Conversation {
        crate::passes::clear(store, p);
    }
    d.modes.push(ModeChange {
        mode,
        by: stamp.by,
        at: stamp.at,
        origin: stamp.origin,
        reason: reason.trim().to_string(),
        extra: Extra::default(),
    });
    save(store, p, &d)?;
    Ok(ModeChanged {
        mode,
        already_recorded: false,
        waiting: waiting(store, p, None, &d, review_url, &now)?,
    })
}

/// The delegation under which Claude holds `v` on `item` at `now`, if any: the one rule for "Claude
/// holds this" (DEF-104). `None` unless the mode is `delegated`; a ratify with open overrides is
/// never delegable (DEF-043); otherwise the first delegation, in file order, that covers it. It
/// does not check how the grant was recorded.
pub fn holder<'a>(
    d: &'a Decisions,
    v: &Verdict,
    item: &FeatureItem,
    now: &str,
) -> Option<&'a Delegation> {
    if d.mode() != Mode::Delegated {
        return None;
    }
    if v.kind == VerdictKind::Ratify && !crate::readiness::open_overrides(item).is_empty() {
        return None;
    }
    d.delegations.iter().find(|g| g.covers(v, item, now))
}

/// Everything waiting for a decision, each in its place under `d`'s mode: the review queue, the
/// proposed ADRs, the proposed delegations, the delegated decisions to review and, with the code
/// repository, the releases ready to promote. An item verdict a delegation in force covers waits
/// for Claude under it in `delegated` mode; in `conversation` mode everything waits there; the
/// rest waits on the monitor, at `{review_url}#{code}` (the Releases page for a release).
pub fn waiting(
    store: &Store,
    p: &str,
    repo: Option<&std::path::Path>,
    d: &Decisions,
    review_url: Option<&str>,
    now: &str,
) -> Result<Vec<Waiting>> {
    use crate::review::ApprovalKind as K;
    let mode = d.mode();
    let project = store.load(p)?;
    let releases_url = review_url.map(|u| match u.strip_suffix("/review") {
        Some(base) => format!("{base}/releases"),
        None => u.to_string(),
    });
    let mut out: Vec<Waiting> = Vec::new();
    let mut push = |code: &str, what: String, verdict: Option<Verdict>, person_only: bool| {
        let covering = verdict.as_ref().and_then(|v| {
            let item = project.feature(&v.code).ok()?;
            holder(d, v, item, now).map(|g| g.id.clone())
        });
        let (place, delegation, url) = match (mode, covering) {
            (Mode::Conversation, _) if !person_only => (Place::Conversation, None, None),
            (Mode::Delegated, Some(id)) => (Place::Delegated, Some(id), None),
            _ => (
                Place::Monitor,
                None,
                review_url.map(|u| format!("{u}#{code}")),
            ),
        };
        out.push(Waiting {
            code: code.to_string(),
            what,
            place,
            delegation,
            url,
        });
    };
    for e in crate::reviewgroups::queue_shared(store, p, repo)? {
        let of = |kind: VerdictKind, signoff: &str| Verdict {
            kind,
            code: e.code.clone(),
            signoff: signoff.to_string(),
        };
        match e.approval {
            K::Signoff | K::SpecChanged if !e.signoffs_needed.is_empty() => {
                for name in &e.signoffs_needed {
                    push(
                        &e.code,
                        format!("sign off {name}"),
                        Some(of(VerdictKind::Signoff, name)),
                        false,
                    );
                }
            }
            K::Unratified => push(
                &e.code,
                "ratify the work done under a bypass".into(),
                Some(of(VerdictKind::Ratify, "")),
                false,
            ),
            K::Missing | K::Lapsed | K::Added => push(
                &e.code,
                "approve its definition".into(),
                Some(of(VerdictKind::Approve, "")),
                false,
            ),
            K::Release => push(&e.code, "choose its release".into(), None, false),
            _ => push(&e.code, "a decision on the Review page".into(), None, false),
        }
    }
    for a in crate::adr::list(store, p)? {
        if a.status == "proposed" {
            push(
                &a.id,
                "accept or reject the proposed ADR".into(),
                None,
                false,
            );
        }
    }
    for g in &d.delegations {
        if g.state(now) == DelegationState::Proposed {
            push(
                &g.id,
                format!(
                    "grant or discard the proposed delegation ({})",
                    g.verdicts.join(", ")
                ),
                None,
                true,
            );
        }
    }
    // Each proposed baseline revision waits on the Baseline page, the person's in every mode
    // (FTR-033).
    let baseline_url = review_url.map(|u| match u.strip_suffix("/review") {
        Some(base) => format!("{base}/baseline"),
        None => u.to_string(),
    });
    for r in crate::baseline::load(store, p)?.revisions {
        if r.state() != crate::baseline::RevisionState::Proposed {
            continue;
        }
        let areas: Vec<&str> = r.answers.iter().map(|a| a.area.as_str()).collect();
        out.push(Waiting {
            code: r.id.clone(),
            what: format!(
                "accept or withdraw the baseline revision ({})",
                areas.join(", ")
            ),
            place: Place::Monitor,
            delegation: None,
            url: baseline_url.as_ref().map(|u| format!("{u}#{}", r.id)),
        });
    }
    for x in to_review(&project, d) {
        let what = match x.signoff.as_str() {
            "" => format!(
                "keep or undo the {} taken under {}",
                x.verdict, x.delegation
            ),
            name => format!(
                "keep or undo the sign-off {name} taken under {}",
                x.delegation
            ),
        };
        out.push(Waiting {
            code: x.code.clone(),
            what,
            place: Place::Monitor,
            delegation: Some(x.delegation.clone()),
            url: review_url.map(|u| format!("{u}#{}", x.code)),
        });
    }
    if let Some(repo) = repo
        && project.config.cadence.releases
    {
        for r in crate::releases::load(store, p)? {
            if r.state != crate::releases::ReleaseState::Planned {
                continue;
            }
            let Ok(plan) = crate::releases::plan_promotion(
                store,
                p,
                &r.version,
                None,
                "",
                Some(repo),
                crate::releases::Fetch::No,
            ) else {
                continue;
            };
            if plan.ready {
                out.push(Waiting {
                    code: r.version.clone(),
                    what: plan.button.clone(),
                    place: Place::Monitor,
                    delegation: None,
                    url: releases_url.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// A delegation with where it stands now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DelegationView {
    #[serde(flatten)]
    pub delegation: Delegation,
    pub state: DelegationState,
    /// Granted, not ended and not revoked: Claude may act under it.
    pub in_force: bool,
    /// What a grant is pinned to: [`Delegation::rev`], sent back with the grant.
    pub rev: String,
    /// Granted, but not through kanbanr (T9): written into the file by hand, or not committed.
    /// Claude's verdicts under it are refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_granted_through_kanbanr: bool,
    /// Granted, but the board's history cannot be read, so where the grant came from cannot be
    /// checked: why, naming the repository (DEF-080). Claude's verdicts under it are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_unverifiable: Option<String>,
    /// The Claude Code allow rules this delegation's verdicts need to run in auto mode (DEF-060),
    /// whatever its state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_rules: Vec<String>,
    /// The mode is delegated, the delegation is granted and the code repository is known, so
    /// `permission_rules_missing` says something.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission_rules_checked: bool,
    /// The rules none of the project's or the user's Claude settings allows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_rules_missing: Vec<String>,
}

/// `GET /projects/{p}/decisions`: the mode and its history, every delegation, the delegated
/// decisions to review, and what waits for a decision where.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DecisionsView {
    pub mode: Mode,
    pub modes: Vec<ModeChange>,
    pub delegations: Vec<DelegationView>,
    pub to_review: Vec<DelegatedDecision>,
    pub waiting: Vec<Waiting>,
}

/// [`DecisionsView`] without what waits: the mode, the delegations and the decisions to review.
/// Read when the queue is not needed, so the Review groups do not compute it twice (DEF-106).
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionsParts {
    pub mode: Mode,
    pub modes: Vec<ModeChange>,
    pub delegations: Vec<DelegationView>,
    pub to_review: Vec<DelegatedDecision>,
}

/// [`view`] without `waiting`.
pub fn view_parts(
    store: &Store,
    p: &str,
    repo: Option<&std::path::Path>,
    now: &str,
) -> Result<DecisionsParts> {
    let d = load(store, p)?;
    parts_of(store, p, repo, now, &d)
}

/// The project's decisions as the Review page and `kanbanr decisions show` read them.
pub fn view(
    store: &Store,
    p: &str,
    repo: Option<&std::path::Path>,
    review_url: Option<&str>,
    now: &str,
) -> Result<DecisionsView> {
    let d = load(store, p)?;
    let parts = parts_of(store, p, repo, now, &d)?;
    Ok(DecisionsView {
        mode: parts.mode,
        modes: parts.modes,
        delegations: parts.delegations,
        to_review: parts.to_review,
        waiting: waiting(store, p, repo, &d, review_url, now)?,
    })
}

fn parts_of(
    store: &Store,
    p: &str,
    repo: Option<&std::path::Path>,
    now: &str,
    d: &Decisions,
) -> Result<DecisionsParts> {
    let project = store.load(p)?;
    Ok(DecisionsParts {
        mode: d.mode(),
        modes: d.modes.clone(),
        delegations: d
            .delegations
            .iter()
            .map(|g| {
                let state = g.state(now);
                let mut rules_of_verdicts: Vec<String> = g
                    .verdicts
                    .iter()
                    .filter_map(|v| crate::claude::permission_rule(v))
                    .collect();
                rules_of_verdicts.sort();
                rules_of_verdicts.dedup();
                let checked = d.mode() == Mode::Delegated
                    && state == DelegationState::Granted
                    && repo.is_some();
                let missing = match repo {
                    Some(repo) if checked => crate::claude::rules_missing(
                        &crate::claude::settings_files(repo),
                        &rules_of_verdicts,
                    ),
                    _ => Vec::new(),
                };
                let granted = g.granted.is_some();
                let prov = granted.then(|| grant_provenance(store, p, &g.id));
                DelegationView {
                    delegation: g.clone(),
                    state,
                    in_force: state == DelegationState::Granted,
                    rev: g.rev(),
                    not_granted_through_kanbanr: granted
                        && matches!(prov, Some(Provenance::Elsewhere)),
                    grant_unverifiable: match prov {
                        Some(Provenance::Unverifiable(r)) => Some(r),
                        _ => None,
                    },
                    permission_rules: rules_of_verdicts,
                    permission_rules_checked: checked,
                    permission_rules_missing: missing,
                }
            })
            .collect(),
        to_review: to_review(&project, d),
    })
}

/// A delegated decision waiting for the person to keep or undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DelegatedDecision {
    /// `<CODE>/<approve|ratify|signoff>/<name or ->/<record at>`.
    pub decision: String,
    pub code: String,
    pub title: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signoff: String,
    pub delegation: String,
    pub at: String,
    pub status: String,
    pub undo_route: String,
}

/// Every approval, ratification and sign-off recorded as `delegated` that no review answers,
/// oldest first.
pub fn to_review(project: &Project, d: &Decisions) -> Vec<DelegatedDecision> {
    let mut out = Vec::new();
    for item in &project.features {
        let Some(def) = &item.definition else {
            continue;
        };
        let made = |verdict: &str, signoff: &str, delegation: &str, at: &str, route: String| {
            DelegatedDecision {
                decision: format!(
                    "{}/{verdict}/{}/{at}",
                    item.code,
                    if signoff.is_empty() { "-" } else { signoff }
                ),
                code: item.code.clone(),
                title: item.title.clone(),
                verdict: verdict.to_string(),
                signoff: signoff.to_string(),
                delegation: delegation.to_string(),
                at: at.to_string(),
                status: item.status.clone(),
                undo_route: route,
            }
        };
        for e in def.approvals.iter().filter(|e| e.origin == "delegated") {
            let verdict = match e.verdict.as_str() {
                "approved" => "approve",
                "ratified" => "ratify",
                _ => continue,
            };
            out.push(made(
                verdict,
                "",
                &e.delegation,
                &e.at,
                format!(
                    "POST /projects/{}/features/{}/unapprove",
                    project.id, item.code
                ),
            ));
        }
        for s in def
            .signoffs
            .iter()
            .filter(|s| s.origin == "delegated" && s.verdict == SignoffVerdict::Given)
        {
            out.push(made(
                "signoff",
                &s.id,
                &s.delegation,
                &s.at,
                format!(
                    "POST /projects/{}/features/{}/signoff/{}/withdraw",
                    project.id, item.code, s.id
                ),
            ));
        }
    }
    out.retain(|x| !d.reviews.iter().any(|r| r.decision == x.decision));
    out.sort_by(|a, b| crate::cmp_instants(&a.at, &b.at).then_with(|| a.decision.cmp(&b.decision)));
    out
}

/// The person keeps or undoes a delegated decision. Undo is the existing withdrawal, with the
/// person's stamp. Refused for an unknown or already reviewed reference. An undo of something
/// already superseded or withdrawn records the review only.
pub fn review(
    store: &Store,
    p: &str,
    decision: &str,
    keep: bool,
    reason: &str,
    stamp: Stamp,
) -> Result<Review> {
    person_stamp(&stamp, "reviewing a delegated decision")?;
    let mut d = load(store, p)?;
    if d.reviews.iter().any(|r| r.decision == decision) {
        return unsupported(format!("{decision} was already reviewed"));
    }
    let project = store.load(p)?;
    let Some(found) = to_review(&project, &d)
        .into_iter()
        .find(|x| x.decision == decision)
    else {
        return unsupported(format!(
            "there is no delegated decision {decision} waiting for review"
        ));
    };
    if !keep {
        let why = format!(
            "undone on review of a delegated decision ({}): {}",
            found.delegation,
            reason.trim()
        );
        let item = project.feature(&found.code)?;
        let def = item.definition.as_ref();
        if found.verdict == "signoff" {
            let standing = item
                .signoff_current(&found.signoff)
                .is_some_and(|s| s.at == found.at);
            if standing {
                store.withdraw_signoff_as(
                    p,
                    &found.code,
                    &found.signoff,
                    &stamp.by,
                    &why,
                    &stamp.origin,
                )?;
            }
        } else if def
            .and_then(|x| x.approval.as_ref())
            .is_some_and(|a| a.at == found.at)
        {
            store.unapprove_feature_as(p, &found.code, &stamp.by, &why, &stamp.origin)?;
        }
    }
    let review = Review {
        decision: decision.to_string(),
        delegation: found.delegation,
        verdict: if keep {
            ReviewVerdict::Kept
        } else {
            ReviewVerdict::Undone
        },
        by: stamp.by,
        at: stamp.at,
        origin: stamp.origin,
        reason: reason.trim().to_string(),
        extra: Extra::default(),
    };
    d.reviews.push(review.clone());
    save(store, p, &d)?;
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Gate, ProjectConfig};
    use crate::models::FeatureDefinition;
    use serde_json::json;

    const NOW: &str = "2026-11-12T10:00:00+00:00";
    const LATER: &str = "2026-11-20T10:00:00+00:00";

    /// A board whose `Planned` gate asks for `design-review`, with a defect, a feature and a
    /// milestone, and the codes of the two items.
    fn board() -> (Store, std::path::PathBuf, String, String) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-decisions-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.gates.insert(
            "Planned".into(),
            Gate {
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        store.init_project("demo", config).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let defect = store
            .add_feature_of_kind("demo", "Bug", "spec", "M", None, Some("defect"))
            .unwrap()
            .code;
        let feature = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap()
            .code;
        (store, dir, defect, feature)
    }

    fn who(origin: &str) -> Stamp {
        Stamp {
            by: if origin == "conversation" {
                "Claude"
            } else {
                "Ada"
            }
            .into(),
            at: NOW.into(),
            origin: origin.into(),
            ..Default::default()
        }
    }

    fn defects() -> NewDelegation {
        NewDelegation {
            verdicts: vec!["signoff:design-review".into()],
            kinds: vec!["defect".into()],
            until: LATER.into(),
            note: "small defects".into(),
            ..Default::default()
        }
    }

    fn route(code: &str, tail: &str) -> String {
        format!("/projects/demo/features/{code}/{tail}")
    }

    fn judge(
        store: &Store,
        method: &str,
        path: &str,
        body: Option<&Value>,
        origin: Origin,
    ) -> std::result::Result<Authorized, String> {
        let d = load(store, "demo").unwrap();
        let project = store.load("demo").unwrap();
        authorize(&d, &project, method, path, body, origin, NOW, None)
    }

    fn grant_it(store: &Store, id: &str) -> Delegation {
        let rev = load(store, "demo")
            .unwrap()
            .delegations
            .iter()
            .find(|g| g.id == id)
            .unwrap()
            .rev();
        grant(store, "demo", id, &rev, who("monitor")).unwrap()
    }

    /// DEF-060 R-1: the view names each granted delegation's allow rules and whether Claude Code's
    /// settings allow them.
    #[test]
    fn the_view_names_each_granted_delegations_permission_rules_and_whether_they_are_allowed() {
        let (store, dir, _, _) = board();
        let rule = "Bash(kanbanr signoff:*)".to_string();
        let repo = dir.join("work");
        std::fs::create_dir_all(&repo).unwrap();
        propose(&store, "demo", defects(), who("conversation")).unwrap();
        grant_it(&store, "DLG-001");
        let one = |repo: Option<&std::path::Path>| {
            view(&store, "demo", repo, None, NOW)
                .unwrap()
                .delegations
                .remove(0)
        };

        // Monitor mode: the rules are named, nothing is checked.
        let g = one(Some(&repo));
        assert_eq!(g.permission_rules, vec![rule.clone()]);
        assert!(!g.permission_rules_checked && g.permission_rules_missing.is_empty());

        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        let g = one(Some(&repo));
        assert!(g.permission_rules_checked);
        assert_eq!(g.permission_rules_missing, vec![rule.clone()]);

        crate::claude::add_permission_rules(
            &repo.join(".claude").join("settings.local.json"),
            std::slice::from_ref(&rule),
        )
        .unwrap();
        let g = one(Some(&repo));
        assert!(g.permission_rules_checked && g.permission_rules_missing.is_empty());

        // No repository: the rules are named and nothing is claimed.
        let g = one(None);
        assert_eq!(g.permission_rules, vec![rule]);
        assert!(!g.permission_rules_checked && g.permission_rules_missing.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-033 R-3: accepting a baseline revision is the person's in every mode. The monitor and
    /// the terminal take it in monitor and delegated mode alike (conversation mode cannot be set
    /// yet); Claude is refused
    /// it in each, with or without a delegation named; the guard words it as the person's own; the
    /// route is a stamped verdict route; and a proposed revision waits on the Baseline page.
    #[test]
    fn accepting_a_baseline_revision_is_the_persons_in_every_mode() {
        use crate::origin::{DecisionContext, Taker, Verdict, verdict_of_route, verdict_refusal};
        let (store, dir, _, _) = board();
        let accept = "/projects/demo/baseline/revisions/REV-001/accept";
        assert!(is_person_only("POST", accept));
        assert!(!is_person_only("GET", accept));
        assert!(crate::origin::is_verdict_route("POST", accept));
        assert!(crate::origin::stamps_origin("POST", accept));
        assert_eq!(
            verdict_of_route("POST", accept),
            Some(Verdict::AcceptBaseline)
        );
        // The withdrawal and the proposal are no verdicts, but say where they came from.
        for (m, path) in [
            ("POST", "/projects/demo/baseline/revisions"),
            ("POST", "/projects/demo/baseline/revisions/REV-001/withdraw"),
            ("POST", "/projects/demo/baseline/packs"),
            ("DELETE", "/projects/demo/baseline/packs/data"),
        ] {
            assert!(!is_person_only(m, path), "{m} {path}");
            assert!(crate::origin::stamps_origin(m, path), "{m} {path}");
        }

        propose(&store, "demo", defects(), who("conversation")).unwrap();
        grant_it(&store, "DLG-001");
        let under = json!({"delegation": "DLG-001"});
        // Conversation mode cannot be set until FTR-035 builds it; the person-only rule is
        // judged before the mode is read, so it holds there too.
        for mode in [Mode::Monitor, Mode::Delegated] {
            set_mode(&store, "demo", mode, "", who("monitor"), None).unwrap();
            for origin in [Origin::Monitor, Origin::Terminal] {
                assert_eq!(
                    judge(&store, "POST", accept, None, origin),
                    Ok(Authorized::AsIs),
                    "{mode:?} {origin:?}"
                );
            }
            for body in [None, Some(&under)] {
                for origin in [Origin::Conversation, Origin::Delegated] {
                    let why = judge(&store, "POST", accept, body, origin).unwrap_err();
                    assert!(
                        why.contains("accepting a baseline revision is the person's own")
                            && why.contains("Baseline page"),
                        "{mode:?} {origin:?}: {why}"
                    );
                }
            }
            // The one guard, with the project's decisions, says the same.
            let d = load(&store, "demo").unwrap();
            let project = store.load("demo").unwrap();
            let ctx = DecisionContext {
                decisions: Some(&d),
                project: Some(&project),
                method: "POST",
                path: accept,
                body: None,
                origin: None,
                now: NOW.into(),
                pass: None,
            };
            assert_eq!(
                verdict_refusal(Verdict::AcceptBaseline, Taker::Person, &ctx),
                None
            );
            let why = verdict_refusal(Verdict::AcceptBaseline, Taker::Claude, &ctx).unwrap();
            assert!(why.contains("the person's own"), "{why}");
        }

        // A proposed revision waits on the Baseline page; an accepted one does not.
        let file = store
            .project_dir("demo")
            .join(crate::baseline::BASELINE_FILE);
        std::fs::write(
            &file,
            "revisions:\n  - id: REV-001\n    answers: [{area: database, later: true}, \
             {area: kubernetes, later: true}]\n    proposed: {by: Claude, at: '2026-11-12', \
             origin: conversation}\n  - id: REV-002\n    answers: [{area: ui-style, later: \
             true}]\n    proposed: {by: Claude, at: '2026-11-12', origin: conversation}\n    \
             accepted: {by: Ada, at: '2026-11-12', origin: monitor}\n",
        )
        .unwrap();
        let d = load(&store, "demo").unwrap();
        let w = waiting(
            &store,
            "demo",
            None,
            &d,
            Some("http://localhost:8080/p/demo/review"),
            NOW,
        )
        .unwrap();
        let rev: Vec<&Waiting> = w.iter().filter(|x| x.code.starts_with("REV-")).collect();
        assert_eq!(rev.len(), 1, "{w:?}");
        assert_eq!(
            rev[0].what,
            "accept or withdraw the baseline revision (database, kubernetes)"
        );
        assert_eq!(rev[0].place, Place::Monitor);
        assert_eq!(
            rev[0].url.as_deref(),
            Some("http://localhost:8080/p/demo/baseline#REV-001")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-1: the table against the project's mode.
    #[test]
    fn a_decision_outside_the_projects_mode_is_refused() {
        let (store, dir, _, feature) = board();
        let approve = route(&feature, "approve");
        // monitor mode: the monitor and the terminal decide; a session does not.
        assert_eq!(
            judge(&store, "POST", &approve, None, Origin::Monitor),
            Ok(Authorized::AsIs)
        );
        assert_eq!(
            judge(&store, "POST", &approve, None, Origin::Terminal),
            Ok(Authorized::AsIs)
        );
        let why = judge(&store, "POST", &approve, None, Origin::Conversation).unwrap_err();
        assert!(
            why.contains("mode: monitor") && why.contains("Review page"),
            "{why}"
        );
        // A route that is no verdict passes whoever sends it.
        let todos = route(&feature, "todos");
        assert_eq!(
            judge(&store, "POST", &todos, None, Origin::Conversation),
            Ok(Authorized::AsIs)
        );
        // delegated mode: a session without a delegation is still refused.
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        assert!(judge(&store, "POST", &approve, None, Origin::Conversation).is_err());
        assert_eq!(
            judge(&store, "POST", &approve, None, Origin::Terminal),
            Ok(Authorized::AsIs)
        );
        assert_eq!(
            judge(&store, "POST", &approve, None, Origin::Monitor),
            Ok(Authorized::AsIs)
        );
        // conversation mode (hand-edited: setting it is refused): the monitor's item verdicts are
        // refused, the terminal still decides, a session has no pass yet.
        let mut d = load(&store, "demo").unwrap();
        d.modes.push(ModeChange {
            mode: Mode::Conversation,
            by: "Ada".into(),
            at: NOW.into(),
            origin: "terminal".into(),
            reason: String::new(),
            extra: Default::default(),
        });
        save(&store, "demo", &d).unwrap();
        let why = judge(&store, "POST", &approve, None, Origin::Monitor).unwrap_err();
        assert!(why.contains("in the conversation"), "{why}");
        assert_eq!(
            judge(&store, "POST", &approve, None, Origin::Terminal),
            Ok(Authorized::AsIs)
        );
        let why = judge(&store, "POST", &approve, None, Origin::Conversation).unwrap_err();
        assert!(
            why.contains("mode: conversation") && why.contains("kanbanr decisions question"),
            "{why}"
        );
        // Conversation mode is built (FTR-035): setting it is accepted.
        let changed =
            set_mode(&store, "demo", Mode::Conversation, "", who("monitor"), None).unwrap();
        assert!(
            changed.already_recorded,
            "the hand-edited mode already stands"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-043 under FTR-028: ratifying an override Claude made is the person's, whatever a
    /// delegation covers; with no open override the delegated ratify stands.
    #[test]
    fn ratifying_claudes_override_is_never_delegated() {
        let (store, dir, _, _) = board();
        // The gate is enforced only once a charter is adopted and blocks, and an override only counts
        // when a gate needed it (DEF-056): so the defect is made after the charter.
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Because reasons".into(),
                goals: vec![crate::Goal {
                    statement: "Ship the thing".into(),
                    ..crate::Goal::default()
                }],
                ..crate::Charter::default()
            },
        )
        .unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Scheduled".to_string(),
            Gate {
                signoffs: vec!["design-review".into()],
                enforce: crate::config::Enforce::Block,
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        let defect = store
            .add_feature_of_kind("demo", "Bug two", "spec", "M", None, Some("defect"))
            .unwrap()
            .code;
        let ratify = route(&defect, "ratify");
        let under = json!({"delegation": "DLG-001"});
        let take = || judge(&store, "POST", &ratify, Some(&under), Origin::Conversation);
        propose(
            &store,
            "demo",
            NewDelegation {
                verdicts: vec!["ratify".into()],
                ..defects()
            },
            who("conversation"),
        )
        .unwrap();
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        grant_it(&store, "DLG-001");
        assert_eq!(
            take(),
            Ok(Authorized::Delegated {
                delegation: "DLG-001".into()
            })
        );
        store
            .move_feature_gated_as(
                "demo",
                &defect,
                "Scheduled",
                Some("CI after push"),
                "conversation",
            )
            .unwrap();

        let why = take().unwrap_err();
        assert!(
            why.contains("DEF-043") && why.contains("prepare ratify"),
            "{why}"
        );
        // The person still ratifies it.
        assert_eq!(
            judge(&store, "POST", &ratify, None, Origin::Monitor),
            Ok(Authorized::AsIs)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-104 R-1, R-2: one rule for "Claude holds this verdict": delegated mode, a granted
    /// delegation in force that is in scope, and no open override for a ratify.
    #[test]
    fn holder_needs_delegated_mode_a_grant_in_force_and_no_open_override() {
        let (store, dir, defect, feature) = board();
        let item = |code: &str| store.load("demo").unwrap().feature(code).unwrap().clone();
        let v = |code: &str| Verdict {
            kind: VerdictKind::Signoff,
            code: code.into(),
            signoff: "design-review".into(),
        };
        let g = propose(&store, "demo", defects(), who("conversation")).unwrap();
        // Proposed, monitor mode: nobody holds it.
        let d = load(&store, "demo").unwrap();
        assert!(holder(&d, &v(&defect), &item(&defect), NOW).is_none());
        grant_it(&store, &g.id);
        // Granted, but the mode is monitor.
        let d = load(&store, "demo").unwrap();
        assert!(holder(&d, &v(&defect), &item(&defect), NOW).is_none());
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        let d = load(&store, "demo").unwrap();
        assert_eq!(
            holder(&d, &v(&defect), &item(&defect), NOW).map(|g| g.id.as_str()),
            Some("DLG-001")
        );
        // The feature is not a defect: outside the delegation's kinds.
        assert!(holder(&d, &v(&feature), &item(&feature), NOW).is_none());
        // Ended: after `until`.
        assert!(holder(&d, &v(&defect), &item(&defect), "2026-12-01T00:00:00+00:00").is_none());
        // A verdict it does not name.
        let approve = Verdict {
            kind: VerdictKind::Approve,
            code: defect.clone(),
            signoff: String::new(),
        };
        assert!(holder(&d, &approve, &item(&defect), NOW).is_none());
        // Revoked.
        revoke(&store, "demo", "DLG-001", "enough", who("monitor")).unwrap();
        let d = load(&store, "demo").unwrap();
        assert!(holder(&d, &v(&defect), &item(&defect), NOW).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A delegated-mode board with a defect that carries an open override, and DLG-001 granted
    /// for `ratify` (as `ratifying_claudes_override_is_never_delegated` sets it up).
    fn board_with_open_override() -> (Store, std::path::PathBuf, String) {
        let (store, dir, _, _) = board();
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Because reasons".into(),
                goals: vec![crate::Goal {
                    statement: "Ship the thing".into(),
                    ..crate::Goal::default()
                }],
                ..crate::Charter::default()
            },
        )
        .unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Scheduled".to_string(),
            Gate {
                signoffs: vec!["design-review".into()],
                enforce: crate::config::Enforce::Block,
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        let defect = store
            .add_feature_of_kind("demo", "Bug two", "spec", "M", None, Some("defect"))
            .unwrap()
            .code;
        store
            .set_feature_definition(
                "demo",
                &defect,
                Some(FeatureDefinition {
                    statement: "Fix the bug".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        propose(
            &store,
            "demo",
            NewDelegation {
                verdicts: vec!["ratify".into()],
                ..defects()
            },
            who("conversation"),
        )
        .unwrap();
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        grant_it(&store, "DLG-001");
        store
            .move_feature_gated_as(
                "demo",
                &defect,
                "Scheduled",
                Some("CI after push"),
                "conversation",
            )
            .unwrap();
        (store, dir, defect)
    }

    /// DEF-104: a ratify with open overrides waits on the monitor, as `authorize` decides, not
    /// for Claude under the delegation.
    #[test]
    fn waiting_leaves_a_ratify_with_open_overrides_on_the_monitor() {
        let (store, dir, defect) = board_with_open_override();
        let d = load(&store, "demo").unwrap();
        let list = waiting(&store, "demo", None, &d, None, NOW).unwrap();
        let w = list
            .iter()
            .find(|w| w.code == defect && w.what.contains("ratify"))
            .unwrap_or_else(|| panic!("no ratify waiting: {list:?}"));
        assert_eq!((w.place, w.delegation.clone()), (Place::Monitor, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A board in conversation mode (set through the write path, as the person would), with a
    /// git identity, and a defined feature and defect; returns what the pass tests need.
    fn conversation_board() -> (Store, std::path::PathBuf, String, String) {
        let (store, dir, defect, feature) = board();
        assert!(dir.starts_with(std::env::temp_dir()));
        for code in [&defect, &feature] {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Do the thing".into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::commit_local(&dir, "the board");
        (store, dir, defect, feature)
    }

    fn take(
        store: &Store,
        dir: &std::path::Path,
        method: &str,
        path: &str,
        body: Value,
        origin: Origin,
    ) -> std::result::Result<crate::write::Applied, crate::write::WriteError> {
        crate::write::apply_as(
            store,
            dir,
            crate::git::PushPolicy::Off,
            method,
            path,
            Some(&body),
            None,
            &crate::eventing::NullSender,
            &crate::agentscope::ProcessScope::Unscoped,
            origin,
        )
    }

    fn rev_of(store: &Store, code: &str) -> String {
        store
            .load("demo")
            .unwrap()
            .feature(code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .content_rev()
    }

    /// A pass for `verdict` on `code`, made as the hook makes one.
    fn pass_for(store: &Store, code: &str, verdict: &str) -> crate::passes::Pass {
        let signoff = verdict.strip_prefix("signoff:").unwrap_or("").to_string();
        let tag = crate::passes::Tag {
            verdict: verdict.into(),
            code: code.into(),
            signoff,
            rev: rev_of(store, code),
            project: "demo".into(),
        };
        let question = format!("Is it right, Ada? {}", crate::passes::tag_text(&tag));
        let answered = crate::passes::Answered {
            question,
            answer: crate::passes::approving_label(verdict).into(),
            tool_use_id: "toolu_01TEST".into(),
            session_id: "session-1".into(),
            tag,
        };
        crate::passes::issue(store, &answered, &crate::now_rfc3339())
            .unwrap()
            .unwrap()
    }

    /// FTR-035 R-2: in conversation mode Claude's verdict is recorded only with a pass that
    /// matches the project, item, verdict and revision; the record keeps the question and the
    /// answer, the pass is used once, and the person's own front ends need none.
    #[test]
    fn a_conversation_verdict_needs_a_matching_pass_and_keeps_the_question_and_answer() {
        let (store, dir, defect, feature) = conversation_board();
        // The person sets the mode (refused until FTR-035 built it).
        take(
            &store,
            &dir,
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "conversation", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        assert_eq!(load(&store, "demo").unwrap().mode(), Mode::Conversation);
        let approve = route(&feature, "approve");
        let rev = rev_of(&store, &feature);
        let refused =
            |r: std::result::Result<crate::write::Applied, crate::write::WriteError>| match r {
                Err(crate::write::WriteError::Refused(why)) => why,
                other => panic!("not refused: {other:?}"),
            };

        // No pass: refused, and the text points to the board's question.
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev}),
            Origin::Conversation,
        ));
        assert!(why.contains("kanbanr decisions question"), "{why}");
        // A pass that does not exist.
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": "P-000000000000"}),
            Origin::Conversation,
        ));
        assert!(why.contains("unknown, already used or expired"), "{why}");

        let pass = pass_for(&store, &feature, "approve");
        // Another item, another verdict, another revision: each refused, the pass kept.
        let other = route(&defect, "approve");
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &other,
            json!({"by": "Ada", "rev": rev_of(&store, &defect), "pass": pass.id}),
            Origin::Conversation,
        ));
        assert!(why.contains(&feature) || why.contains("pass"), "{why}");
        let ratify = route(&feature, "ratify");
        refused(take(
            &store,
            &dir,
            "POST",
            &ratify,
            json!({"by": "Ada", "rev": rev, "pass": pass.id}),
            Origin::Conversation,
        ));
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": "deadbeef", "pass": pass.id}),
            Origin::Conversation,
        ));
        assert!(why.contains("--rev"), "{why}");
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "pass": pass.id}),
            Origin::Conversation,
        ));
        assert!(why.contains("--rev"), "{why}");
        // A pass and a delegation together.
        refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": pass.id, "delegation": "DLG-001"}),
            Origin::Conversation,
        ));
        // The terminal needs no pass, and may not offer one.
        refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": pass.id}),
            Origin::Terminal,
        ));
        assert!(crate::passes::load(&store, &pass.id, &crate::now_rfc3339()).is_some());
        // A body that carries its own `heard` gets it stripped: the record says what the pass says.
        take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": pass.id, "origin": "terminal",
                   "heard": {"question": "forged", "answer": "forged", "pass": "P-1", "heard_at": "x"}}),
            Origin::Conversation,
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let def = project
            .feature(&feature)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        let approval = def.approval.clone().unwrap();
        assert_eq!(approval.origin, "conversation");
        assert_eq!(approval.by, "Ada");
        let heard = approval.heard.expect("the answer is kept");
        assert_eq!(heard.answer, "Approve");
        assert_eq!(heard.pass, pass.id);
        assert_eq!(heard.question, pass.question);
        assert_eq!(heard.tool_use_id, "toolu_01TEST");
        assert_eq!(heard.session_id, "session-1");
        assert_eq!(heard.heard_at, pass.at);
        assert_eq!(def.approvals.last().unwrap().heard.as_ref(), Some(&heard));
        // The pass is used once: gone, and a second use is refused.
        assert!(crate::passes::load(&store, &pass.id, &crate::now_rfc3339()).is_none());
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": pass.id}),
            Origin::Conversation,
        ));
        assert!(why.contains("unknown, already used or expired"), "{why}");
        // The commit says which pass it used.
        let log = std::process::Command::new("git")
            .args(["log", "-1", "--format=%B"])
            .current_dir(&dir)
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout).to_string();
        assert!(log.contains(&format!("Kanbanr-Pass: {}", pass.id)), "{log}");
        assert!(log.contains("Kanbanr-Origin: conversation"), "{log}");

        // A sign-off takes its pass the same way, named after the verdict.
        let sign = pass_for(&store, &defect, "signoff:design-review");
        take(
            &store,
            &dir,
            "POST",
            &route(&defect, "signoff/design-review"),
            json!({"by": "Ada", "rev": rev_of(&store, &defect), "pass": sign.id}),
            Origin::Conversation,
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let signed = project
            .feature(&defect)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        let given = signed.signoffs.last().unwrap();
        assert_eq!(given.origin, "conversation");
        assert_eq!(
            given.heard.as_ref().map(|h| h.pass.as_str()),
            Some(sign.id.as_str())
        );
        // A pass outside conversation mode is refused.
        take(
            &store,
            &dir,
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "monitor", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        let why = refused(take(
            &store,
            &dir,
            "POST",
            &approve,
            json!({"by": "Ada", "rev": rev, "pass": "P-000000000000"}),
            Origin::Conversation,
        ));
        assert!(why.contains("not in conversation mode"), "{why}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-035 R-2: leaving conversation mode drops every pass of the project, and only its own.
    #[test]
    fn leaving_conversation_mode_drops_every_pass() {
        let (store, dir, _, feature) = conversation_board();
        take(
            &store,
            &dir,
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "conversation", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        let now = crate::now_rfc3339();
        let pass = pass_for(&store, &feature, "approve");
        // A pass of another project on the same board.
        let foreign = crate::passes::Pass {
            id: "P-aaaaaaaaaaaa".into(),
            project: "other".into(),
            code: "X-1".into(),
            verdict: "approve".into(),
            rev: "abcdef12".into(),
            expires: "2999-01-01T00:00:00+00:00".into(),
            ..Default::default()
        };
        let folder = crate::passes::dir(&store);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("P-aaaaaaaaaaaa.json"),
            serde_json::to_string(&foreign).unwrap(),
        )
        .unwrap();
        assert!(crate::passes::load(&store, &pass.id, &now).is_some());
        // A repeat of the standing mode drops nothing.
        take(
            &store,
            &dir,
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "conversation", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        assert!(crate::passes::load(&store, &pass.id, &now).is_some());
        take(
            &store,
            &dir,
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "delegated", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        assert!(crate::passes::load(&store, &pass.id, &now).is_none());
        assert!(crate::passes::load(&store, "P-aaaaaaaaaaaa", &now).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-2: within the delegation, and every way outside it.
    #[test]
    fn a_delegated_verdict_is_accepted_within_the_delegation_and_refused_outside_it() {
        let (store, dir, defect, feature) = board();
        let signoff = route(&defect, "signoff/design-review");
        let under = json!({"delegation": "DLG-001"});
        let take = |path: &str, now: &str| {
            let d = load(&store, "demo").unwrap();
            let project = store.load("demo").unwrap();
            authorize(
                &d,
                &project,
                "POST",
                path,
                Some(&under),
                Origin::Conversation,
                now,
                None,
            )
        };
        let g = propose(&store, "demo", defects(), who("conversation")).unwrap();
        assert_eq!(g.id, "DLG-001");
        // Proposed, not granted; and the mode is not delegated.
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        let why = take(&signoff, NOW).unwrap_err();
        assert!(why.contains("only proposed"), "{why}");
        grant_it(&store, "DLG-001");
        assert_eq!(
            take(&signoff, NOW),
            Ok(Authorized::Delegated {
                delegation: "DLG-001".into()
            })
        );
        // The same delegation names the way to take it, when Claude forgot.
        let why = judge(&store, "POST", &signoff, None, Origin::Conversation).unwrap_err();
        assert!(why.contains("--under DLG-001"), "{why}");
        // Outside it: another verdict, another sign-off, another kind, after the end.
        assert!(
            take(&route(&defect, "approve"), NOW)
                .unwrap_err()
                .contains("does not cover `approve`")
        );
        assert!(take(&route(&defect, "signoff/docs-folded"), NOW).is_err());
        let why = take(&route(&feature, "signoff/design-review"), NOW).unwrap_err();
        assert!(why.contains("does not cover a feature"), "{why}");
        let why = take(&signoff, "2026-12-01T00:00:00+00:00").unwrap_err();
        assert!(why.contains("has ended"), "{why}");
        // A step, an item and a milestone narrow it further.
        let narrow = propose(
            &store,
            "demo",
            NewDelegation {
                verdicts: vec!["approve".into()],
                steps: vec!["Planned".into()],
                items: vec![feature.clone()],
                milestone: "M".into(),
                until: LATER.into(),
                ..Default::default()
            },
            who("conversation"),
        )
        .unwrap();
        grant_it(&store, &narrow.id);
        let d = load(&store, "demo").unwrap();
        let project = store.load("demo").unwrap();
        let g = d.delegations.iter().find(|g| g.id == narrow.id).unwrap();
        let v = verdict_of("POST", &route(&feature, "approve")).unwrap();
        let item = project.feature(&feature).unwrap();
        assert_eq!(item.status, "Planned");
        assert!(g.covers(&v, item, NOW));
        assert!(
            !g.covers(&v, project.feature(&defect).unwrap(), NOW),
            "other item"
        );
        let mut elsewhere = item.clone();
        elsewhere.status = "In Progress".into();
        assert!(!g.covers(&v, &elsewhere, NOW), "other step");
        elsewhere.status = "Planned".into();
        elsewhere.milestone = "N".into();
        assert!(!g.covers(&v, &elsewhere, NOW), "other milestone");
        // Revoked: refused, and it stays in the file.
        revoke(
            &store,
            "demo",
            "DLG-001",
            "no longer wanted",
            who("terminal"),
        )
        .unwrap();
        let why = take(&signoff, NOW).unwrap_err();
        assert!(why.contains("was revoked"), "{why}");
        assert_eq!(load(&store, "demo").unwrap().delegations.len(), 2);
        // Not in delegated mode: refused, naming the mode.
        set_mode(&store, "demo", Mode::Monitor, "", who("monitor"), None).unwrap();
        let why = take(&route(&feature, "approve"), NOW).unwrap_err();
        assert!(why.contains("not in delegated mode"), "{why}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-2: a session proposes, and that is all; the person's routes and the rules on a proposal
    /// hold.
    #[test]
    fn claude_cannot_record_or_widen_a_delegation() {
        let (store, dir, defect, _) = board();
        let g = propose(&store, "demo", defects(), who("conversation")).unwrap();
        assert!(g.granted.is_none(), "a proposal is inert");
        assert_eq!(g.state(NOW), DelegationState::Proposed);
        // Grant, mode and review are the person's, from a session or a record labelled delegated.
        let grant_route = "/projects/demo/decisions/delegations/DLG-001/grant";
        for (method, path) in [
            ("POST", grant_route),
            ("PUT", "/projects/demo/decisions/mode"),
            ("POST", "/projects/demo/decisions/reviews"),
        ] {
            assert!(is_person_only(method, path), "{path}");
            for o in [Origin::Conversation, Origin::Delegated] {
                assert!(
                    judge(&store, method, path, None, o).is_err(),
                    "{path} {o:?}"
                );
            }
            for o in [Origin::Monitor, Origin::Terminal] {
                assert_eq!(judge(&store, method, path, None, o), Ok(Authorized::AsIs));
            }
        }
        // The functions refuse a session's stamp too, whatever route reached them.
        let rev = g.rev();
        assert!(grant(&store, "demo", "DLG-001", &rev, who("conversation")).is_err());
        assert!(
            set_mode(
                &store,
                "demo",
                Mode::Delegated,
                "",
                who("conversation"),
                None
            )
            .is_err()
        );
        assert!(
            review(
                &store,
                "demo",
                "X/approve/-/t",
                true,
                "",
                who("conversation")
            )
            .is_err()
        );
        // A stale rev is refused; the right one grants; a granted one cannot be granted again.
        let why = grant(&store, "demo", "DLG-001", "0000", who("monitor")).unwrap_err();
        assert!(
            why.to_string().contains("changed since you saw it"),
            "{why}"
        );
        grant(&store, "demo", "DLG-001", &rev, who("monitor")).unwrap();
        assert!(grant(&store, "demo", "DLG-001", &rev, who("monitor")).is_err());
        // Widening is a new proposal with its own id and its own rev; the granted one is unchanged.
        let mut wider = defects();
        wider.kinds = vec!["defect".into(), "chore".into()];
        let w = propose(&store, "demo", wider, who("conversation")).unwrap();
        assert_eq!(w.id, "DLG-002");
        assert_ne!(w.rev(), rev);
        let d = load(&store, "demo").unwrap();
        assert_eq!(d.delegations[0].kinds, ["defect"]);
        assert!(d.delegations[1].granted.is_none());
        // What can never be delegated, and what a proposal must say.
        for bad in [
            "promote",
            "cut",
            "unapprove",
            "withdraw",
            "adr-accept",
            "gate:record",
            "signoff:",
        ] {
            let mut n = defects();
            n.verdicts = vec![bad.into()];
            assert!(
                propose(&store, "demo", n, who("conversation")).is_err(),
                "{bad}"
            );
        }
        let mut n = defects();
        n.verdicts = vec!["signoff:nobody-asks".into()];
        assert!(
            propose(&store, "demo", n, who("conversation")).is_err(),
            "no gate asks for it"
        );
        let mut n = defects();
        n.kinds.clear();
        assert!(
            propose(&store, "demo", n.clone(), who("conversation")).is_err(),
            "no selector"
        );
        n.all = true;
        assert!(
            propose(&store, "demo", n, who("conversation")).is_ok(),
            "all says so"
        );
        let mut n = defects();
        n.until = "2026-11-01T00:00:00+00:00".into();
        assert!(
            propose(&store, "demo", n, who("conversation")).is_err(),
            "in the past"
        );
        let mut n = defects();
        n.until = "2027-06-01T00:00:00+00:00".into();
        assert!(
            propose(&store, "demo", n, who("conversation")).is_err(),
            "over 90 days"
        );
        let mut n = defects();
        n.until = String::new();
        assert!(
            propose(&store, "demo", n, who("conversation")).is_err(),
            "no end"
        );
        // Release verdicts, withdrawals and ADR verdicts are the person's whatever is granted.
        let _ = defect;
        for path in [
            "/projects/demo/releases/v1/promote",
            "/projects/demo/adrs/ADR-0001/accept",
        ] {
            assert!(
                judge(&store, "POST", path, None, Origin::Conversation).is_err(),
                "{path}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-2: `--under` is Claude's; the person's own front ends refuse it.
    #[test]
    fn under_from_a_terminal_is_refused() {
        let (store, dir, defect, _) = board();
        propose(&store, "demo", defects(), who("conversation")).unwrap();
        grant_it(&store, "DLG-001");
        set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), None).unwrap();
        let signoff = route(&defect, "signoff/design-review");
        let under = json!({"delegation": "DLG-001"});
        for o in [Origin::Terminal, Origin::Monitor] {
            let why = judge(&store, "POST", &signoff, Some(&under), o).unwrap_err();
            assert!(why.contains("`--under` is Claude's"), "{why}");
        }
        // The same body from a session is accepted.
        assert!(matches!(
            judge(&store, "POST", &signoff, Some(&under), Origin::Conversation),
            Ok(Authorized::Delegated { .. })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_delegated_record_waits_for_review_until_it_is_reviewed() {
        let (store, dir, _, feature) = board();
        store
            .set_feature_definition(
                "demo",
                &feature,
                Some(FeatureDefinition {
                    statement: "Keep a cart".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        store
            .signoff_feature_checked_as(
                "demo",
                &feature,
                "design-review",
                "Claude",
                "",
                "",
                "delegated",
                "DLG-001",
                &[],
                None,
            )
            .unwrap();
        let project = store.load("demo").unwrap();
        let list = to_review(&project, &load(&store, "demo").unwrap());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].delegation, "DLG-001");
        assert_eq!(list[0].verdict, "signoff");
        let id = list[0].decision.clone();
        review(&store, "demo", &id, true, "fine", who("monitor")).unwrap();
        let d = load(&store, "demo").unwrap();
        assert!(to_review(&store.load("demo").unwrap(), &d).is_empty());
        assert!(
            review(&store, "demo", &id, true, "", who("monitor")).is_err(),
            "only once"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-3: a verdict Claude takes under a delegation, through the write recipe every front end
    /// uses, is recorded as `delegated`, by Claude, naming the delegation, with the commit's
    /// trailers saying so; it then waits for review. Without `--under`, or outside the
    /// delegation, the same verdict is refused and nothing is recorded.
    #[test]
    fn a_delegated_verdict_records_its_delegation() {
        use crate::agentscope::ProcessScope;
        use crate::git::PushPolicy;
        use crate::write::{WriteError, apply_as};
        let (store, dir, defect, feature) = board();
        store
            .set_feature_definition(
                "demo",
                &defect,
                Some(FeatureDefinition {
                    statement: "Fix the bug".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::commit_local(&dir, "the board");
        let now = crate::now_rfc3339();
        let until = (OffsetDateTime::now_utc() + time::Duration::days(7))
            .format(&Rfc3339)
            .unwrap();
        let at_now = |origin: &str| Stamp {
            at: now.clone(),
            ..who(origin)
        };
        set_mode(&store, "demo", Mode::Delegated, "", at_now("monitor"), None).unwrap();
        let g = propose(
            &store,
            "demo",
            NewDelegation { until, ..defects() },
            at_now("conversation"),
        )
        .unwrap();
        // Granted through the write recipe, as the monitor and the terminal grant (T9).
        apply_as(
            &store,
            &dir,
            PushPolicy::Off,
            "POST",
            &format!("/projects/demo/decisions/delegations/{}/grant", g.id),
            Some(&json!({"rev": g.rev(), "by": "Ada"})),
            None,
            &crate::eventing::NullSender,
            &ProcessScope::Unscoped,
            Origin::Monitor,
        )
        .unwrap();
        let take = |code: &str, body: Value| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Off,
                "POST",
                &route(code, "signoff/design-review"),
                Some(&body),
                None,
                &crate::eventing::NullSender,
                &ProcessScope::Unscoped,
                Origin::Conversation,
            )
        };
        let head = || crate::coderepo::git(&dir, &["rev-parse", "HEAD"]).unwrap();
        let before = head();
        // Without naming the delegation: refused, and told which one covers it.
        match take(&defect, json!({})) {
            Err(WriteError::Refused(why)) => {
                assert!(why.contains("--under DLG-001"), "{why}")
            }
            other => panic!("not refused: {:?}", other.map(|a| a.body)),
        }
        // A feature is outside a delegation for defects.
        assert!(matches!(
            take(&feature, json!({"delegation": "DLG-001"})),
            Err(WriteError::Refused(_))
        ));
        assert_eq!(head(), before, "a refused verdict made a commit");
        // Under it: recorded as delegated, by Claude, naming DLG-001, whatever the body claimed.
        take(
            &defect,
            json!({"delegation": "DLG-001", "by": "Ada", "origin": "monitor"}),
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let item = project.feature(&defect).unwrap();
        let s = item.signoff_current("design-review").unwrap();
        assert_eq!(
            (s.origin.as_str(), s.by.as_str(), s.delegation.as_str()),
            ("delegated", "Claude", "DLG-001")
        );
        let log = crate::coderepo::git(&dir, &["log", "-1", "--format=%B"]).unwrap();
        assert!(
            log.contains("Kanbanr-Origin: delegated")
                && log.contains("Kanbanr-Delegation: DLG-001"),
            "{log}"
        );
        let list = to_review(&project, &load(&store, "demo").unwrap());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].code, defect);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A board for the waiting list: `Scheduled` asks for `design-review`; the defect is defined
    /// and approved (its sign-off waits), the feature is defined and not approved, an ADR is
    /// proposed, and the feature carries a sign-off taken under DLG-001. DLG-001 covers the
    /// defects' design reviews and is granted.
    fn waiting_board() -> (Store, std::path::PathBuf, String, String) {
        let (store, dir, defect, feature) = board();
        let mut config = store.load_meta("demo").unwrap().config;
        config.gates.insert(
            "Scheduled".into(),
            Gate {
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        store.save_config("demo", &config).unwrap();
        for (code, statement) in [(&defect, "Fix the bug"), (&feature, "Keep a cart")] {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: statement.into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        store
            .approve_feature_as("demo", &defect, "Ada", "terminal", "", None)
            .unwrap();
        crate::adr::create(&store, "demo", "Use one table", Default::default()).unwrap();
        let g = propose(&store, "demo", defects(), who("conversation")).unwrap();
        grant_it(&store, &g.id);
        store
            .signoff_feature_checked_as(
                "demo",
                &feature,
                "design-review",
                "Claude",
                "",
                "",
                "delegated",
                "DLG-001",
                &[],
                None,
            )
            .unwrap();
        (store, dir, defect, feature)
    }

    /// R-4: each waiting decision is listed in its place under the mode: the monitor in `monitor`
    /// mode; for Claude under DLG-001 the sign-off it covers in `delegated` mode; everything in the
    /// conversation in `conversation` mode; the delegated decision to review on the monitor always.
    #[test]
    fn switching_mode_lists_the_waiting_decisions_in_the_new_place() {
        let (store, dir, defect, feature) = waiting_board();
        let url = Some("http://h:1/p/demo/review");
        let place = |list: &[Waiting], code: &str, what: &str| -> (Place, Option<String>) {
            let w = list
                .iter()
                .find(|w| w.code == code && w.what.contains(what))
                .unwrap_or_else(|| panic!("{code} {what} not waiting: {list:?}"));
            (w.place, w.delegation.clone())
        };
        let d = load(&store, "demo").unwrap();
        let before = waiting(&store, "demo", None, &d, url, NOW).unwrap();
        assert_eq!(
            place(&before, &defect, "sign off design-review"),
            (Place::Monitor, None)
        );
        let at = before.iter().find(|w| w.code == defect).unwrap();
        assert_eq!(
            at.url.as_deref(),
            Some(format!("http://h:1/p/demo/review#{defect}").as_str())
        );
        assert_eq!(place(&before, &feature, "approve"), (Place::Monitor, None));
        assert_eq!(place(&before, "ADR-0001", "ADR"), (Place::Monitor, None));
        assert_eq!(
            place(&before, &feature, "keep or undo"),
            (Place::Monitor, Some("DLG-001".into()))
        );

        let changed = set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), url).unwrap();
        assert!(!changed.already_recorded);
        let after = &changed.waiting;
        assert_eq!(
            place(after, &defect, "sign off design-review"),
            (Place::Delegated, Some("DLG-001".into()))
        );
        assert_eq!(place(after, &feature, "approve"), (Place::Monitor, None));
        assert_eq!(place(after, "ADR-0001", "ADR"), (Place::Monitor, None));
        // A repeat records nothing, and still says what waits where.
        let again = set_mode(&store, "demo", Mode::Delegated, "", who("monitor"), url).unwrap();
        assert!(again.already_recorded && !again.waiting.is_empty());

        // Conversation mode (FTR-035): everything waits in the conversation except the reviews,
        // which are the person's.
        let talk = set_mode(&store, "demo", Mode::Conversation, "", who("monitor"), url)
            .unwrap()
            .waiting;
        assert_eq!(
            place(&talk, &defect, "sign off design-review"),
            (Place::Conversation, None)
        );
        assert_eq!(place(&talk, "ADR-0001", "ADR"), (Place::Conversation, None));
        assert_eq!(
            place(&talk, &feature, "keep or undo"),
            (Place::Monitor, Some("DLG-001".into()))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-3: the person keeps one delegated decision and undoes another; an undo withdraws it, and
    /// each is reviewed once.
    #[test]
    fn keep_and_undo_review_a_delegated_decision() {
        let (store, dir, defect, feature) = waiting_board();
        store
            .approve_feature_as("demo", &feature, "Claude", "delegated", "DLG-001", None)
            .unwrap();
        let _ = defect;
        let list = to_review(&store.load("demo").unwrap(), &load(&store, "demo").unwrap());
        assert_eq!(list.len(), 2, "{list:?}");
        let signoff = list
            .iter()
            .find(|x| x.verdict == "signoff")
            .unwrap()
            .clone();
        let approval = list
            .iter()
            .find(|x| x.verdict == "approve")
            .unwrap()
            .clone();
        assert!(
            review(
                &store,
                "demo",
                &signoff.decision,
                true,
                "",
                who("conversation")
            )
            .is_err(),
            "a review is the person's"
        );
        let kept = review(
            &store,
            "demo",
            &signoff.decision,
            true,
            "fine",
            who("monitor"),
        )
        .unwrap();
        assert_eq!(kept.verdict, ReviewVerdict::Kept);
        let undone = review(
            &store,
            "demo",
            &approval.decision,
            false,
            "too early",
            who("monitor"),
        )
        .unwrap();
        assert_eq!(undone.verdict, ReviewVerdict::Undone);
        let project = store.load("demo").unwrap();
        let item = project.feature(&feature).unwrap();
        assert!(
            item.signoff_current("design-review").is_some(),
            "the kept sign-off stands"
        );
        let def = item.definition.as_ref().unwrap();
        assert!(def.approval.is_none(), "the undone approval is withdrawn");
        assert!(
            def.approvals.iter().any(|e| e
                .reason
                .contains("undone on review of a delegated decision (DLG-001)")),
            "{:?}",
            def.approvals
        );
        assert!(to_review(&project, &load(&store, "demo").unwrap()).is_empty());
        assert!(review(&store, "demo", &approval.decision, true, "", who("monitor")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// T9: a delegation is honoured only when kanbanr wrote its grant: written by hand into the
    /// file (and committed with no origin trailer), or left uncommitted, it is refused.
    #[test]
    fn a_grant_written_by_hand_is_not_honoured() {
        use crate::agentscope::ProcessScope;
        use crate::git::PushPolicy;
        use crate::write::{WriteError, apply_as};
        let (store, dir, defect, _) = board();
        store
            .set_feature_definition(
                "demo",
                &defect,
                Some(FeatureDefinition {
                    statement: "Fix the bug".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::commit_local(&dir, "the board");
        let now = crate::now_rfc3339();
        let until = until_of("", &now).unwrap();
        let write = |method: &str, path: &str, body: Value, origin: Origin| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Off,
                method,
                path,
                Some(&body),
                None,
                &crate::eventing::NullSender,
                &ProcessScope::Unscoped,
                origin,
            )
        };
        write(
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "delegated", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        for _ in 0..2 {
            write(
                "POST",
                "/projects/demo/decisions/delegations",
                json!({"verdicts": ["signoff:design-review"], "kinds": ["defect"], "until": until}),
                Origin::Conversation,
            )
            .unwrap();
        }
        // DLG-001 is granted by hand: the file edited and committed without kanbanr.
        let mut d = load(&store, "demo").unwrap();
        let g = d
            .delegations
            .iter_mut()
            .find(|g| g.id == "DLG-001")
            .unwrap();
        g.granted = Some(Stamp {
            by: "Ada".into(),
            at: now.clone(),
            origin: "monitor".into(),
            rev: g.rev(),
            ..Default::default()
        });
        save(&store, "demo", &d).unwrap();
        crate::git::commit_local(&dir, "grant it");
        assert!(!granted_through_kanbanr(&store, "demo", "DLG-001"));
        let take = |id: &str| {
            write(
                "POST",
                &route(&defect, "signoff/design-review"),
                json!({"delegation": id}),
                Origin::Conversation,
            )
        };
        match take("DLG-001") {
            Err(WriteError::Refused(why)) => {
                assert!(why.contains("not granted through kanbanr"), "{why}")
            }
            other => panic!("not refused: {:?}", other.map(|a| a.body)),
        }
        // DLG-002 is granted through kanbanr, but the file then changes outside it.
        let rev = load(&store, "demo").unwrap().delegations[1].rev();
        write(
            "POST",
            "/projects/demo/decisions/delegations/DLG-002/grant",
            json!({"rev": rev, "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        assert!(granted_through_kanbanr(&store, "demo", "DLG-002"));
        let file = dir.join("projects/demo").join(DECISIONS_FILE);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, format!("{text}\n")).unwrap();
        assert!(!granted_through_kanbanr(&store, "demo", "DLG-002"));
        assert!(matches!(take("DLG-002"), Err(WriteError::Refused(_))));
        // Put back, it is honoured.
        std::fs::write(&file, text).unwrap();
        take("DLG-002").unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-080: a board whose DLG-002 was granted through kanbanr.
    fn granted_through_kanbanr_board() -> (Store, std::path::PathBuf, String) {
        use crate::agentscope::ProcessScope;
        use crate::git::PushPolicy;
        use crate::write::apply_as;
        let (store, dir, defect, _) = board();
        assert!(dir.starts_with(std::env::temp_dir()));
        store
            .set_feature_definition(
                "demo",
                &defect,
                Some(FeatureDefinition {
                    statement: "Fix the bug".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::commit_local(&dir, "the board");
        let until = until_of("", &crate::now_rfc3339()).unwrap();
        let write = |method: &str, path: &str, body: Value, origin: Origin| {
            apply_as(
                &store,
                &dir,
                PushPolicy::Off,
                method,
                path,
                Some(&body),
                None,
                &crate::eventing::NullSender,
                &ProcessScope::Unscoped,
                origin,
            )
        };
        write(
            "PUT",
            "/projects/demo/decisions/mode",
            json!({"mode": "delegated", "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        for _ in 0..2 {
            write(
                "POST",
                "/projects/demo/decisions/delegations",
                json!({"verdicts": ["signoff:design-review"], "kinds": ["defect"], "until": until}),
                Origin::Conversation,
            )
            .unwrap();
        }
        let rev = load(&store, "demo").unwrap().delegations[1].rev();
        write(
            "POST",
            "/projects/demo/decisions/delegations/DLG-002/grant",
            json!({"rev": rev, "by": "Ada"}),
            Origin::Terminal,
        )
        .unwrap();
        assert_eq!(
            grant_provenance(&store, "demo", "DLG-002"),
            Provenance::ThroughKanbanr
        );
        (store, dir, defect)
    }

    /// DEF-080 R-2: a grant whose history cannot be read is unverifiable, naming the folder, and
    /// is not shown as granted outside kanbanr.
    #[test]
    fn an_unreadable_history_reads_as_unverifiable_not_as_not_granted() {
        let (store, dir, _) = granted_through_kanbanr_board();
        std::fs::rename(dir.join(".git"), dir.join(".git-away")).unwrap();
        match grant_provenance(&store, "demo", "DLG-002") {
            Provenance::Unverifiable(why) => {
                assert!(why.contains(&dir.display().to_string()), "{why}");
                assert!(why.contains("is not a git repository"), "{why}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!granted_through_kanbanr(&store, "demo", "DLG-002"));
        let v = view(&store, "demo", None, None, &crate::now_rfc3339()).unwrap();
        let g = v
            .delegations
            .iter()
            .find(|g| g.delegation.id == "DLG-002")
            .unwrap();
        assert!(!g.not_granted_through_kanbanr);
        assert!(g.grant_unverifiable.is_some());
        // Put back, it is honoured.
        std::fs::rename(dir.join(".git-away"), dir.join(".git")).unwrap();
        assert_eq!(
            grant_provenance(&store, "demo", "DLG-002"),
            Provenance::ThroughKanbanr
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-080 R-2, R-4: Claude's verdict under a grant nobody can check is refused, saying why.
    #[test]
    fn a_verdict_under_a_grant_whose_history_cannot_be_read_is_refused_naming_the_reason() {
        use crate::write::WriteError;
        let (store, dir, defect) = granted_through_kanbanr_board();
        std::fs::rename(dir.join(".git"), dir.join(".git-away")).unwrap();
        // The write's own guard, asked directly: a whole write would first make a fresh
        // repository where `.git` is missing, and refuse for want of a commit identity where it
        // cannot open one, before it reaches the delegation.
        let body = json!({"delegation": "DLG-002"});
        match crate::write::judge(
            &store,
            "POST",
            &route(&defect, "signoff/design-review"),
            Some(&body),
            Origin::Conversation,
        ) {
            Err(WriteError::Refused(why)) => {
                assert!(why.contains("could not be checked"), "{why}");
                assert!(why.contains("is not a git repository"), "{why}");
            }
            other => panic!("not refused: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-106 R-6: the provenance read from the shared history is the one `file_history` gave,
    /// for a grant through kanbanr, one never granted, one written by hand and committed, and a
    /// history that cannot be read.
    #[test]
    fn grant_provenance_from_shared_history_matches_file_history() {
        let (store, dir, _) = granted_through_kanbanr_board();
        let rel = format!("projects/demo/{DECISIONS_FILE}");
        let both = |id: &str| {
            let shared = grant_provenance(&store, "demo", id);
            let again = grant_provenance_unshared(&dir, &rel, id);
            assert_eq!(shared, again, "{id}");
            shared
        };
        assert_eq!(both("DLG-002"), Provenance::ThroughKanbanr);
        assert_eq!(both("DLG-001"), Provenance::Elsewhere, "never granted");
        assert_eq!(both("DLG-099"), Provenance::Elsewhere, "no such delegation");

        // DLG-001 granted by hand into the file and committed without kanbanr's trailer.
        let mut d = load(&store, "demo").unwrap();
        d.delegations[0].granted = d.delegations[1].granted.clone();
        std::fs::write(dir.join(&rel), crate::yaml::to_string(&d).unwrap()).unwrap();
        assert!(crate::git::commit_local(&dir, "by hand"));
        assert_eq!(both("DLG-001"), Provenance::Elsewhere, "granted by hand");
        assert_eq!(both("DLG-002"), Provenance::ThroughKanbanr);

        // A history that cannot be read.
        std::fs::rename(dir.join(".git"), dir.join(".git-away")).unwrap();
        let shared = grant_provenance(&store, "demo", "DLG-002");
        assert!(matches!(shared, Provenance::Unverifiable(_)), "{shared:?}");
        assert_eq!(shared, grant_provenance_unshared(&dir, &rel, "DLG-002"));
        std::fs::rename(dir.join(".git-away"), dir.join(".git")).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-5: each built-in process proposes the Defaults table: design-control nothing; togaf the
    /// docs fold-in for every kind; the rest also the design review of defects and chores. Every
    /// proposed step is a status from which the move into the gate that asks for the sign-off
    /// exists, so a default can never be inert; each runs the default 14 days.
    #[test]
    fn setup_proposes_defaults_for_each_process() {
        for (name, _) in crate::config::presets() {
            let config = crate::config::preset(name).unwrap().into_config(name);
            let (mode, proposed) = proposed_defaults(&config, Some(name), NOW);
            let names: Vec<&str> = proposed.iter().map(|d| d.verdicts[0].as_str()).collect();
            match name {
                "design-control" => {
                    assert_eq!(mode, Mode::Monitor, "{name}");
                    assert!(proposed.is_empty(), "{name}: {proposed:?}");
                }
                "togaf" => assert_eq!(names, ["signoff:docs-folded"], "{name}"),
                _ => assert_eq!(
                    names,
                    ["signoff:design-review", "signoff:docs-folded"],
                    "{name}"
                ),
            }
            if !proposed.is_empty() {
                assert_eq!(mode, Mode::Delegated, "{name}");
            }
            let gates = config.effective_gates();
            for d in &proposed {
                let signoff = d.verdicts[0].trim_start_matches("signoff:");
                assert_eq!(d.until, until_of("", NOW).unwrap(), "{name}");
                assert!(!d.steps.is_empty(), "{name} {signoff}: no steps");
                // Every step is the source of a forward move into a gate that asks for the
                // sign-off: back-moves are the person's to decide (13 Nov 2026).
                let at = |s: &str| config.statuses.iter().position(|x| x == s);
                for step in &d.steps {
                    let forward = config.transitions.get(step).is_some_and(|to| {
                        to.iter().any(|t| {
                            at(t) > at(step)
                                && gates
                                    .get(t)
                                    .is_some_and(|g| g.signoffs.iter().any(|x| x == signoff))
                        })
                    });
                    assert!(
                        forward,
                        "{name}: {step} has no forward move into a gate asking {signoff}"
                    );
                }
                match signoff {
                    "design-review" => {
                        assert_eq!(d.kinds, ["chore", "defect"], "{name}");
                        assert!(!d.all, "{name}");
                    }
                    _ => assert!(d.all && d.kinds.is_empty(), "{name}"),
                }
            }
        }
        // A process that asks for neither sign-off proposes nothing.
        let (mode, none) = proposed_defaults(&ProjectConfig::default_for("demo"), None, NOW);
        assert_eq!((mode, none.len()), (Mode::Monitor, 0));
    }
}
