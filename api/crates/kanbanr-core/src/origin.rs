//! Where a verdict was taken (DEF-028).
//!
//! Every approval, sign-off, decision, promotion and cut records its origin: a click on the
//! monitor, an answer given to Claude, or a command in the person's own terminal. The front end
//! that receives a verdict names the origin; a request cannot. It is an honest label, not
//! authentication: ADR-0001 keeps kanbanr free of accounts.

use crate::models::{FeatureDefinition, FeatureItem};
use serde_json::Value;

/// Where a verdict was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A click on the monitor's Review, Releases or item page.
    Monitor,
    /// A command that ran inside a Claude Code session.
    Conversation,
    /// A command that ran in the person's own terminal.
    Terminal,
    /// Taken under a recorded delegation (reserved; nothing writes it yet).
    Delegated,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Monitor => "monitor",
            Origin::Conversation => "conversation",
            Origin::Terminal => "terminal",
            Origin::Delegated => "delegated",
        }
    }

    /// Exact and lower case; anything else is `None`.
    pub fn parse(s: &str) -> Option<Origin> {
        match s {
            "monitor" => Some(Origin::Monitor),
            "conversation" => Some(Origin::Conversation),
            "terminal" => Some(Origin::Terminal),
            "delegated" => Some(Origin::Delegated),
            _ => None,
        }
    }

    /// The origin of a command line run here: the conversation when Claude Code set `CLAUDECODE`
    /// in this shell, the terminal otherwise.
    pub fn of_cli() -> Origin {
        Origin::of_cli_in(|k| std::env::var(k).ok())
    }

    /// As [`Origin::of_cli`], reading the environment through `env` so a test needs no mutation.
    pub fn of_cli_in(env: impl Fn(&str) -> Option<String>) -> Origin {
        match env("CLAUDECODE") {
            Some(v) if !v.trim().is_empty() => Origin::Conversation,
            _ => Origin::Terminal,
        }
    }
}

/// A stored origin as words: "" is "unknown"; a value this version does not know is returned as
/// it is.
pub fn label(stored: &str) -> &str {
    if stored.is_empty() { "unknown" } else { stored }
}

/// A stored origin as it ends a line: " via the monitor", or " — origin unknown" for none.
pub fn via(stored: &str) -> String {
    match stored {
        "" => " — origin unknown".to_string(),
        "monitor" | "conversation" | "terminal" => format!(" via the {stored}"),
        "delegated" => " under a delegation".to_string(),
        other => format!(" via {other}"),
    }
}

/// Is this route one that records a verdict? The person-only decisions routes (the mode, a
/// grant, a review) count, so they are stamped and judged as verdicts are (FTR-028).
pub fn is_verdict_route(method: &str, path: &str) -> bool {
    if crate::decisions::is_person_only(method, path) {
        return true;
    }
    if method != "POST" {
        return false;
    }
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    matches!(
        seg.as_slice(),
        [
            "projects",
            _,
            "features",
            _,
            "approve" | "ratify" | "unapprove"
        ] | ["projects", _, "features", _, "signoff", _]
            | ["projects", _, "features", _, "signoff", _, "withdraw"]
            | ["projects", _, "adrs", _, "accept" | "reject"]
            | [
                "projects",
                _,
                "releases",
                _,
                "promote" | "cut" | "patch-from-tag" | "undo"
            ]
            | ["projects", _, "gate-results"]
            | ["projects", _, "prepared", _, "confirm" | "reject"]
            | ["projects", _, "prepared", "batch", _, "confirm" | "reject"]
            | ["projects", _, "review", "decide"]
            // Accepting the system baseline (FTR-006); initialising it is not a verdict.
            | ["projects", _, "catalog", "baseline", "accept"]
            // A planning decision from a page (FTR-049); declining one is not a verdict.
            | ["projects", _, "planning"]
    )
}

/// Routes whose body the front end stamps with its origin: every verdict route,
/// `POST /projects/{p}/prepared`, so a prepared record says who prepared it (FTR-034), the
/// tests route, so a manual result says where it was recorded (DEF-040), and the move and batch
/// routes, so an override says where it was made (DEF-043).
pub fn stamps_origin(method: &str, path: &str) -> bool {
    if is_verdict_route(method, path) {
        return true;
    }
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match method {
        // A proposal, a revoke and the setup defaults say where they came from too (FTR-028);
        // a move, alone or in a batch, records where an override was made (DEF-043).
        "POST" => matches!(
            seg.as_slice(),
            ["projects", _, "prepared"]
                | ["projects", _, "decisions", "delegations" | "defaults"]
                | ["projects", _, "decisions", "delegations", _, "revoke"]
                | ["projects", _, "features", _, "move"]
                | ["projects", _, "batch"]
                // A baseline proposal, a withdrawal and a pack added say where they came from
                // (FTR-033).
                | ["projects", _, "baseline", "revisions" | "packs"]
                | ["projects", _, "baseline", "revisions", _, "withdraw"]
                // A planning decision's decline says where it was declined (FTR-049).
                | ["projects", _, "planning", _, "decline"]
                // A plan route says where a planning decision it carries out was carried out
                // (FTR-049): the outcome's origin is stamped, never taken from the body.
                | ["projects", _, "sprints", _, "plan"]
                | ["projects", _, "sprints", "unplan"]
                | ["projects", _, "releases", _, "plan"]
        ),
        // A pack removed says where it was removed (FTR-033).
        "DELETE" => matches!(seg.as_slice(), ["projects", _, "baseline", "packs", _]),
        // A manual test's kept result says where it was recorded (DEF-040).
        "PUT" => matches!(
            seg.as_slice(),
            ["projects", _, "features", _, "tests", _, _]
        ),
        _ => false,
    }
}

/// A decision that is the person's own (FTR-024, FTR-034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    Ratify,
    Unapprove,
    Signoff,
    WithdrawSignoff,
    AcceptAdr,
    RejectAdr,
    Promote,
    Cut,
    PatchFromTag,
    GateResult,
    ManualTest,
    ConfirmPrepared,
    RejectPrepared,
    /// Setting the project's decision mode (FTR-028).
    SetDecisionMode,
    /// Granting a proposed delegation (FTR-028).
    GrantDelegation,
    /// Keeping or undoing a delegated decision (FTR-028).
    ReviewDecision,
    /// One decision for several rows of a Review page group (FTR-038): each row's own verdict,
    /// in one commit. The person's alone in every mode.
    GroupDecision,
    /// Accepting a proposed revision of the project's engineering baseline (FTR-033). The
    /// person's alone in every mode: no delegation covers it.
    AcceptBaseline,
    /// Accepting the project's system baseline, the pinned system-level catalog rows (FTR-006).
    /// The person's alone in every mode: no delegation covers it. Claude prepares it instead.
    AcceptSystemBaseline,
    /// Planning items into a sprint or a release, or out of their sprint, from a monitor page
    /// (FTR-049). The person's alone in every mode; Claude carries it out or declines it.
    PlanningDecision,
}

impl Verdict {
    /// Every verdict, for a check that must cover them all.
    pub const ALL: [Verdict; 21] = [
        Verdict::Approve,
        Verdict::Ratify,
        Verdict::Unapprove,
        Verdict::Signoff,
        Verdict::WithdrawSignoff,
        Verdict::AcceptAdr,
        Verdict::RejectAdr,
        Verdict::Promote,
        Verdict::Cut,
        Verdict::PatchFromTag,
        Verdict::GateResult,
        Verdict::ManualTest,
        Verdict::ConfirmPrepared,
        Verdict::RejectPrepared,
        Verdict::SetDecisionMode,
        Verdict::GrantDelegation,
        Verdict::ReviewDecision,
        Verdict::GroupDecision,
        Verdict::AcceptBaseline,
        Verdict::AcceptSystemBaseline,
        Verdict::PlanningDecision,
    ];

    /// Is this verdict taken on the Releases page (rather than the Review page)?
    pub fn on_releases_page(self) -> bool {
        matches!(
            self,
            Verdict::Promote | Verdict::Cut | Verdict::PatchFromTag | Verdict::GateResult
        )
    }
}

/// Who would take a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taker {
    Person,
    Claude,
}

impl Taker {
    /// The conversation, and a delegation until FTR-028 decides what one allows, are Claude; the
    /// monitor and the person's own terminal are the person.
    pub fn of_origin(o: Origin) -> Taker {
        match o {
            Origin::Conversation | Origin::Delegated => Taker::Claude,
            Origin::Monitor | Origin::Terminal => Taker::Person,
        }
    }
}

/// What the one guard judges a verdict against (FTR-028): the project's decisions and the route
/// that records it. [`DecisionContext::monitor_mode`] reads no board, so it judges as `monitor`
/// mode, where every verdict is the person's: the hook's view, which knows no mode.
#[derive(Debug, Clone, Default)]
pub struct DecisionContext<'a> {
    pub decisions: Option<&'a crate::decisions::Decisions>,
    pub project: Option<&'a crate::store::Project>,
    pub method: &'a str,
    pub path: &'a str,
    pub body: Option<&'a Value>,
    /// The front end's own label, which tells the monitor from the terminal; when `None` it is
    /// taken from the taker.
    pub origin: Option<Origin>,
    pub now: String,
}

impl DecisionContext<'_> {
    /// No decisions read: every verdict is the person's.
    pub fn monitor_mode() -> Self {
        DecisionContext::default()
    }
}

/// Why Claude may not record a planning decision (FTR-049 R-2).
pub const PLANNING_REFUSAL: &str = "a planning decision is the person's: ask the person to make \
                                    it on the Plan page, or plan it yourself with `kanbanr sprint \
                                    plan` when the person asked you to in the conversation";

const PREPARE_INSTEAD: &str = "Prepare its record instead (`kanbanr prepare …`) and tell the \
                               person where it waits.";

/// Whether `taker` may take `verdict`: the reason it may not, or `None`. The only place this is
/// decided (FTR-034, FTR-028): the hook and the write path both ask here. With the project's
/// decisions in `ctx`, it applies their table (who may take what, in which mode, under which
/// delegation); without them, every verdict is the person's. When Claude may take it, the route's
/// body names the delegation it is taken under.
pub fn verdict_refusal(verdict: Verdict, taker: Taker, ctx: &DecisionContext) -> Option<String> {
    let decisions_route = matches!(
        verdict,
        Verdict::SetDecisionMode | Verdict::GrantDelegation | Verdict::ReviewDecision
    );
    // A manual test's result says a person checked it (DEF-040): never Claude's, in any mode, and
    // no delegation covers it.
    if verdict == Verdict::ManualTest {
        return match taker {
            Taker::Person => None,
            Taker::Claude => Some(format!(
                "a verdict is the person's own, recorded on the monitor, never by Claude \
                 (FTR-024). {PREPARE_INSTEAD} A manual test's result says a person checked it, \
                 so the person records it on the Review page (DEF-040). If they told you the \
                 result, prepare it: `kanbanr prepare test <CODE> <R-n> \"<test>\" --passed \
                 --source \"conversation: …\"`."
            )),
        };
    }
    // Accepting a baseline revision is the person's in every mode (FTR-033): no delegation covers
    // it, and there is no record to prepare; the revision itself waits on the Baseline page.
    if verdict == Verdict::AcceptBaseline && taker == Taker::Claude {
        return Some(crate::decisions::ACCEPT_BASELINE_REFUSAL.to_string());
    }
    // The system baseline's acceptance is the person's in every mode (FTR-006): treated as a
    // manual test's result is, with a record to prepare instead.
    if verdict == Verdict::AcceptSystemBaseline {
        return match taker {
            Taker::Person => None,
            Taker::Claude => Some(crate::decisions::ACCEPT_SYSTEM_BASELINE_REFUSAL.to_string()),
        };
    }
    // A group decision is the person's in every mode (FTR-038): no delegation covers it as a
    // whole, and each verdict it expands into is judged on its own by the write path.
    if verdict == Verdict::GroupDecision {
        return match taker {
            Taker::Person => None,
            Taker::Claude => Some(format!(
                "a verdict is the person's own, recorded on the monitor, never by Claude \
                 (FTR-024). {PREPARE_INSTEAD} A group decision on the Review page is the \
                 person's in every mode: link them the group (FTR-038)."
            )),
        };
    }
    // A planning decision is the person's in every mode (FTR-049): no delegation covers it, and
    // Claude's own plan changes go through the plan commands, not this route.
    if verdict == Verdict::PlanningDecision && taker == Taker::Claude {
        return Some(PLANNING_REFUSAL.to_string());
    }
    let (Some(d), Some(project)) = (ctx.decisions, ctx.project) else {
        return match taker {
            Taker::Person => None,
            Taker::Claude if decisions_route => Some(
                "this is the person's own, on the Review page or in their terminal: Claude \
                 cannot set the decision mode, grant a delegation or review a decision"
                    .to_string(),
            ),
            Taker::Claude => Some(format!(
                "a verdict is the person's own, recorded on the monitor, never by Claude \
                 (FTR-024). {PREPARE_INSTEAD}"
            )),
        };
    };
    let origin = ctx.origin.unwrap_or(match taker {
        Taker::Person => Origin::Terminal,
        Taker::Claude => Origin::Conversation,
    });
    match crate::decisions::authorize(d, project, ctx.method, ctx.path, ctx.body, origin, &ctx.now)
    {
        Ok(_) => None,
        Err(why) if taker == Taker::Claude && !decisions_route => {
            Some(format!("{why} {PREPARE_INSTEAD}"))
        }
        Err(why) => Some(why),
    }
}

/// The verdict a route records, for [`verdict_refusal`]; `None` for a route that records none.
pub fn verdict_of_route(method: &str, path: &str) -> Option<Verdict> {
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    Some(match (method, seg.as_slice()) {
        ("PUT", ["projects", _, "decisions", "mode"]) => Verdict::SetDecisionMode,
        ("POST", ["projects", _, "decisions", "delegations", _, "grant"]) => {
            Verdict::GrantDelegation
        }
        ("POST", ["projects", _, "decisions", "reviews"]) => Verdict::ReviewDecision,
        ("POST", ["projects", _, "features", _, "approve"]) => Verdict::Approve,
        ("POST", ["projects", _, "features", _, "ratify"]) => Verdict::Ratify,
        ("POST", ["projects", _, "features", _, "unapprove"]) => Verdict::Unapprove,
        ("POST", ["projects", _, "features", _, "signoff", _]) => Verdict::Signoff,
        ("POST", ["projects", _, "features", _, "signoff", _, "withdraw"]) => {
            Verdict::WithdrawSignoff
        }
        ("POST", ["projects", _, "adrs", _, "accept"]) => Verdict::AcceptAdr,
        ("POST", ["projects", _, "adrs", _, "reject"]) => Verdict::RejectAdr,
        ("POST", ["projects", _, "releases", _, "promote"]) => Verdict::Promote,
        ("POST", ["projects", _, "releases", _, "cut"]) => Verdict::Cut,
        ("POST", ["projects", _, "releases", _, "patch-from-tag"]) => Verdict::PatchFromTag,
        // Stepping a stage back is the person's, as taking it was (DEF-034).
        ("POST", ["projects", _, "releases", _, "undo"]) => Verdict::Promote,
        ("POST", ["projects", _, "gate-results"]) => Verdict::GateResult,
        ("POST", ["projects", _, "prepared", _, "confirm"]) => Verdict::ConfirmPrepared,
        ("POST", ["projects", _, "prepared", _, "reject"]) => Verdict::RejectPrepared,
        // A whole batch at once (FTR-037): the same verdicts, the person's alone.
        ("POST", ["projects", _, "prepared", "batch", _, "confirm"]) => Verdict::ConfirmPrepared,
        ("POST", ["projects", _, "prepared", "batch", _, "reject"]) => Verdict::RejectPrepared,
        // One decision for several rows of a Review group (FTR-038).
        ("POST", ["projects", _, "review", "decide"]) => Verdict::GroupDecision,
        // Accepting a baseline revision (FTR-033).
        ("POST", ["projects", _, "baseline", "revisions", _, "accept"]) => Verdict::AcceptBaseline,
        // Accepting the system baseline (FTR-006).
        ("POST", ["projects", _, "catalog", "baseline", "accept"]) => Verdict::AcceptSystemBaseline,
        // A planning decision from a page (FTR-049).
        ("POST", ["projects", _, "planning"]) => Verdict::PlanningDecision,
        _ => return None,
    })
}

/// The verdict a write request would record that its route does not show: a manual test's
/// result (DEF-040), by the tests route or by a `test.state` operation in a batch. Setting a
/// test back to `planned` is not a verdict. `None` for every other request.
pub fn guarded_verdict(
    store: &crate::store::Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Option<Verdict> {
    use crate::dispatch::{is_manual_test, percent_decode};
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let result = |s: Option<&str>| matches!(s, Some("green" | "red"));
    match (method, seg.as_slice()) {
        ("PUT", ["projects", p, "features", code, "tests", req, test]) => {
            let state = body.and_then(|b| b.get("state")).and_then(|v| v.as_str());
            (result(state)
                && is_manual_test(store, p, code, &percent_decode(req), &percent_decode(test)))
            .then_some(Verdict::ManualTest)
        }
        ("POST", ["projects", p, "batch"]) => {
            let ops = body?.get("operations")?.as_array()?;
            let text = |op: &Value, k: &str| op.get(k).and_then(|v| v.as_str()).map(String::from);
            ops.iter()
                .any(|op| {
                    text(op, "op").as_deref() == Some("test.state")
                        && result(text(op, "state").as_deref())
                        && matches!(
                            (text(op, "feature"), text(op, "requirement"), text(op, "test")),
                            (Some(c), Some(r), Some(t)) if is_manual_test(store, p, &c, &r, &t)
                        )
                })
                .then_some(Verdict::ManualTest)
        }
        _ => None,
    }
}

/// The refusal for Claude editing another item's approved specification outside a proposed
/// change (DEF-050): the sentence, or `None` when the request is allowed. A spec write is a
/// `PATCH` of an item with a `specification`, or a batch `feature.edit` with `spec` or
/// `specification`. It is refused when Claude (the conversation or a delegation) writes it, the
/// target's definition is approved, and `current` (the item being worked on) is a different item.
/// With no current item nothing is refused: the design stage has no branch yet.
pub fn guarded_spec_edit(
    store: &crate::store::Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    taker: Taker,
    current: Option<&str>,
) -> Option<String> {
    if taker != Taker::Claude {
        return None;
    }
    let current = current?;
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let has = |v: &Value, keys: &[&str]| keys.iter().any(|k| v.get(*k).is_some());
    let mut targets: Vec<(String, String)> = Vec::new();
    match (method, seg.as_slice()) {
        ("PATCH", ["projects", p, "features", code]) => {
            if body.is_some_and(|b| has(b, &["specification"])) {
                targets.push((p.to_string(), crate::dispatch::percent_decode(code)));
            }
        }
        ("POST", ["projects", p, "batch"]) => {
            for op in body
                .and_then(|b| b.get("operations"))
                .and_then(|o| o.as_array())
                .into_iter()
                .flatten()
            {
                if op.get("op").and_then(|v| v.as_str()) == Some("feature.edit")
                    && has(op, &["spec", "specification"])
                    && let Some(code) = op.get("code").and_then(|v| v.as_str())
                {
                    targets.push((p.to_string(), code.to_string()));
                }
            }
        }
        _ => {}
    }
    targets.into_iter().find_map(|(p, code)| {
        if code == current {
            return None;
        }
        let project = store.load_meta(&p).ok()?;
        let agreed = project
            .feature(&code)
            .ok()?
            .definition
            .as_ref()
            .is_some_and(|d| d.approval_state().is_agreed());
        agreed.then(|| {
            format!(
                "refused: {code}'s specification belongs to an approved item. Edit it only as a \
                 proposed change: ask the person to unapprove {code} first (the Review page), \
                 then edit and bring it back through review. Work on {current} stays on {current}."
            )
        })
    })
}

/// The verdict a kanbanr command line records, from its subcommand path (`["release", "cut"]`)
/// and its words after `kanbanr`; `None` for a command that records no verdict.
pub fn verdict_of_command(path: &[&str], rest: &[String]) -> Option<Verdict> {
    let has = |flag: &str| rest.iter().any(|w| w == flag);
    match path {
        ["approve"] => Some(Verdict::Approve),
        ["ratify"] => Some(Verdict::Ratify),
        ["unapprove"] => Some(Verdict::Unapprove),
        ["signoff"] if has("--withdraw") => Some(Verdict::WithdrawSignoff),
        ["signoff"] => Some(Verdict::Signoff),
        ["adr", "accept"] => Some(Verdict::AcceptAdr),
        ["adr", "reject"] => Some(Verdict::RejectAdr),
        ["release", "promote"] if !has("--plan") => Some(Verdict::Promote),
        ["release", "cut"] => Some(Verdict::Cut),
        // Undoing a stage taken by mistake is the person's, as the promotion was (DEF-034).
        ["release", "undo"] => Some(Verdict::Promote),
        // Starting a patch from a red tag is the person's decision (FTR-026). `promote` to a
        // stage after the tag is covered above; `config tag-gate` is configuration, not a verdict.
        ["release", "add"] if has("--from-failed-tag") => Some(Verdict::PatchFromTag),
        // A gate result recorded by hand is a verdict on the release (FTR-002).
        ["gate", "record"] => Some(Verdict::GateResult),
        // Accepting a baseline revision is the person's (FTR-033).
        ["baseline", "accept"] => Some(Verdict::AcceptBaseline),
        // Accepting the system baseline is the person's (FTR-006).
        ["catalog", "baseline", "accept"] => Some(Verdict::AcceptSystemBaseline),
        _ => None,
    }
}

/// The request body with `origin` set to `origin`, overwriting whatever the client sent, for a
/// route [`stamps_origin`] names; `None` for any other route, so no other body is touched.
pub fn stamp(method: &str, path: &str, body: Option<&Value>, origin: Origin) -> Option<Value> {
    if !stamps_origin(method, path) {
        return None;
    }
    let mut v = match body {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => Value::Object(Default::default()),
    };
    v["origin"] = Value::String(origin.as_str().to_string());
    Some(v)
}

/// The origin a verdict body carries, as a stored string: a known value, else "".
pub fn from_body(body: &Value) -> String {
    body.get("origin")
        .and_then(|v| v.as_str())
        .and_then(Origin::parse)
        .map(|o| o.as_str().to_string())
        .unwrap_or_default()
}

/// The delegation a verdict body names (`DLG-001`), trimmed; "" when it names none.
pub fn delegation_from_body(b: &Value) -> String {
    b.get("delegation")
        .and_then(|v| v.as_str())
        .map(|d| d.trim().to_string())
        .unwrap_or_default()
}

/// The pass a verdict body names (`P-3f2a9c1d7b40`), trimmed; "" when it names none (FTR-035).
pub fn pass_from_body(b: &Value) -> String {
    b.get("pass")
        .and_then(|v| v.as_str())
        .map(|d| d.trim().to_string())
        .unwrap_or_default()
}

/// The answer a conversation verdict was recorded from, as `apply_as` stamped it into the body
/// (FTR-035). `None` when the body carries none, or one that is not an object of the right shape.
pub fn heard_from_body(b: &Value) -> Option<crate::models::Heard> {
    let h = b.get("heard")?;
    if !h.is_object() {
        return None;
    }
    serde_json::from_value(h.clone()).ok()
}

/// The record a repeat verdict collides with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earlier {
    pub what: String,
    pub by: String,
    pub at: String,
    pub origin: String,
}

impl Earlier {
    /// `approved by Ada on 2026-11-07 12:26:00 from the monitor`.
    pub fn sentence(&self) -> String {
        let at: String = self
            .at
            .chars()
            .take(19)
            .collect::<String>()
            .replace('T', " ");
        let from = match self.origin.as_str() {
            "" => "from an unknown origin".to_string(),
            o @ ("monitor" | "conversation" | "terminal") => format!("from the {o}"),
            "delegated" => "from a delegation".to_string(),
            o => format!("from {o}"),
        };
        format!("{} by {} on {} {}", self.what, self.by, at, from)
    }

    /// The `already_recorded` object an answer carries.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "what": self.what, "by": self.by, "at": self.at, "origin": self.origin,
            "sentence": self.sentence(),
        })
    }
}

/// Would `verdict` (`approved` or `ratified`) repeat the latest one on the definition as it
/// stands? Returns the earlier record when it would.
pub fn duplicate_approval(def: &FeatureDefinition, verdict: &str) -> Option<Earlier> {
    let standing = def.approval.as_ref()?;
    let rev = def.content_rev();
    if standing.rev != rev {
        return None;
    }
    let last = def.approvals.last()?;
    if last.verdict != verdict || last.rev != rev {
        return None;
    }
    Some(Earlier {
        what: last.verdict.clone(),
        by: last.by.clone(),
        at: last.at.clone(),
        origin: last.origin.clone(),
    })
}

/// Would ratifying `item` repeat the latest ratification on the definition as it stands? Never
/// while an override Claude made waits for one (DEF-064): each override is ratified on its own,
/// even at a revision the person already ratified. Returns the earlier record when it would.
pub fn duplicate_ratification(item: &FeatureItem) -> Option<Earlier> {
    if !crate::readiness::open_overrides(item).is_empty() {
        return None;
    }
    duplicate_approval(item.definition.as_ref()?, "ratified")
}

/// Would giving sign-off `id` with `note` and `doc` repeat the current one? A repeat that brings a
/// new note or a doc adds something and is not a duplicate.
pub fn duplicate_signoff(item: &FeatureItem, id: &str, note: &str, doc: &str) -> Option<Earlier> {
    let s = item.signoff_current(id)?;
    let (note, doc) = (note.trim(), doc.trim());
    if (!note.is_empty() && note != s.note) || (!doc.is_empty() && doc != s.doc) {
        return None;
    }
    Some(Earlier {
        what: format!("signed off {id}"),
        by: s.by.clone(),
        at: s.at.clone(),
        origin: s.origin.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_front_end_names_the_origin_not_the_request() {
        let claimed = json!({"by": "Ada", "origin": "terminal"});
        let s = stamp(
            "POST",
            "/projects/p/features/FEAT-1/approve",
            Some(&claimed),
            Origin::Monitor,
        )
        .unwrap();
        assert_eq!(from_body(&s), "monitor");
        assert_eq!(s["by"], "Ada");
        // No body at all is stamped all the same.
        let s = stamp(
            "POST",
            "/projects/p/adrs/ADR-0001/accept",
            None,
            Origin::Terminal,
        )
        .unwrap();
        assert_eq!(from_body(&s), "terminal");
        // Other routes are left alone.
        assert!(
            stamp(
                "POST",
                "/projects/p/features/FEAT-1/todos",
                Some(&claimed),
                Origin::Monitor
            )
            .is_none()
        );
        assert!(
            stamp(
                "GET",
                "/projects/p/features/FEAT-1/approve",
                None,
                Origin::Monitor
            )
            .is_none()
        );
        for p in [
            "/projects/p/features/F-1/ratify",
            "/projects/p/features/F-1/unapprove",
            "/projects/p/features/F-1/signoff/design-review",
            "/projects/p/features/F-1/signoff/design-review/withdraw",
            "/projects/p/adrs/ADR-0001/reject",
            "/projects/p/releases/v1/promote",
            "/projects/p/releases/v1/cut",
            "/projects/p/releases/v1/patch-from-tag",
        ] {
            assert!(is_verdict_route("POST", p), "{p}");
        }
        assert!(!is_verdict_route("POST", "/projects/p/releases/v1/plan"));
        // The CLI's origin follows CLAUDECODE.
        let with = |v: Option<&str>| {
            Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| v.map(String::from)).flatten())
        };
        assert_eq!(with(Some("1")), Origin::Conversation);
        assert_eq!(with(Some("")), Origin::Terminal);
        assert_eq!(with(None), Origin::Terminal);
        // A value the request invents is not an origin.
        assert_eq!(from_body(&json!({"origin": "elsewhere"})), "");
        assert_eq!(Origin::parse("Monitor"), None);
        assert_eq!(label(""), "unknown");
        assert_eq!(label("scheduled"), "scheduled");
    }

    /// FTR-034 R-4: the one guard refuses Claude every verdict and a person none.
    #[test]
    fn claude_is_refused_every_verdict_and_a_person_none() {
        for v in Verdict::ALL {
            let ctx = DecisionContext::monitor_mode();
            assert_eq!(verdict_refusal(v, Taker::Person, &ctx), None, "{v:?}");
            let why = verdict_refusal(v, Taker::Claude, &ctx).unwrap_or_else(|| panic!("{v:?}"));
            // The decisions routes have no record to prepare: they are the person's own (FTR-028).
            let decisions = matches!(
                v,
                Verdict::SetDecisionMode
                    | Verdict::GrantDelegation
                    | Verdict::ReviewDecision
                    | Verdict::AcceptBaseline
            );
            let says = if v == Verdict::PlanningDecision {
                "Plan page"
            } else if decisions {
                "the person's own"
            } else {
                "kanbanr prepare"
            };
            assert!(why.contains(says), "{why}");
        }
        assert_eq!(Taker::of_origin(Origin::Conversation), Taker::Claude);
        assert_eq!(Taker::of_origin(Origin::Delegated), Taker::Claude);
        assert_eq!(Taker::of_origin(Origin::Monitor), Taker::Person);
        assert_eq!(Taker::of_origin(Origin::Terminal), Taker::Person);
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // FTR-006 R-3: the system baseline's acceptance is a stamped verdict route and command,
        // refused to Claude with the text that names `prepare system-baseline`, and judged by
        // the one guard in every origin.
        let accept = "/projects/p/catalog/baseline/accept";
        assert!(is_verdict_route("POST", accept));
        assert!(stamps_origin("POST", accept));
        assert!(crate::decisions::is_person_only("POST", accept));
        assert_eq!(
            verdict_of_route("POST", accept),
            Some(Verdict::AcceptSystemBaseline)
        );
        assert_eq!(
            verdict_of_command(&["catalog", "baseline", "accept"], &w(&["catalog"])),
            Some(Verdict::AcceptSystemBaseline)
        );
        // Initialising it, and reading it, are no verdicts.
        assert!(!is_verdict_route(
            "POST",
            "/projects/p/catalog/baseline/init"
        ));
        assert_eq!(
            verdict_of_route("POST", "/projects/p/catalog/baseline/init"),
            None
        );
        assert_eq!(verdict_of_route("GET", accept), None);
        assert_eq!(
            verdict_of_command(&["catalog", "baseline"], &w(&["catalog"])),
            None
        );
        let why = verdict_refusal(
            Verdict::AcceptSystemBaseline,
            Taker::Claude,
            &DecisionContext::monitor_mode(),
        )
        .unwrap();
        assert!(why.contains("kanbanr prepare system-baseline"), "{why}");
        for o in [Origin::Conversation, Origin::Delegated] {
            let ctx = DecisionContext {
                origin: Some(o),
                method: "POST",
                path: accept,
                ..DecisionContext::monitor_mode()
            };
            assert!(
                verdict_refusal(Verdict::AcceptSystemBaseline, Taker::of_origin(o), &ctx).is_some(),
                "{o:?}"
            );
        }
        assert_eq!(
            verdict_of_command(&["signoff"], &w(&["signoff", "F", "x", "--withdraw"])),
            Some(Verdict::WithdrawSignoff)
        );
        assert_eq!(
            verdict_of_command(&["release", "promote"], &w(&["--plan"])),
            None
        );
        assert_eq!(verdict_of_command(&["prepare"], &w(&["gate"])), None);
        // The prepared confirm and reject routes and a gate result are verdict routes; preparing
        // is not, but its body is stamped.
        for p in [
            "/projects/p/prepared/x/confirm",
            "/projects/p/prepared/x/reject",
            "/projects/p/gate-results",
        ] {
            assert!(is_verdict_route("POST", p), "{p}");
        }
        assert!(!is_verdict_route("POST", "/projects/p/prepared"));
        assert!(stamps_origin("POST", "/projects/p/prepared"));
        assert!(!stamps_origin("GET", "/projects/p/prepared"));
    }

    /// DEF-034 R-5: `release undo` is a verdict, the person's as the promotion is: the command and
    /// the route both map to `Verdict::Promote` (so core's guard refuses the route without the
    /// hook), the route is stamped with its origin, and Claude is refused it.
    #[test]
    fn release_undo_is_a_verdict_refused_to_claude() {
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let rest = w(&["release", "undo", "v0.6.0", "--reason", "wrong release"]);
        assert_eq!(
            verdict_of_command(&["release", "undo"], &rest),
            Some(Verdict::Promote)
        );
        let route = "/projects/p/releases/v0.6.0/undo";
        assert_eq!(verdict_of_route("POST", route), Some(Verdict::Promote));
        assert_eq!(verdict_of_route("GET", route), None);
        assert!(is_verdict_route("POST", route));
        assert!(!is_verdict_route("GET", route));
        assert!(stamps_origin("POST", route));
        let s = stamp(
            "POST",
            route,
            Some(&json!({"by": "Ada", "reason": "wrong release", "origin": "monitor"})),
            Origin::Conversation,
        )
        .unwrap();
        assert_eq!(from_body(&s), "conversation");

        // Run by Claude (CLAUDECODE set), the undo is refused; run by the person, it is not.
        let claude = Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| "1".to_string()));
        assert_eq!(claude, Origin::Conversation);
        let ctx = DecisionContext::monitor_mode();
        let verdict = verdict_of_command(&["release", "undo"], &rest).unwrap();
        let why = verdict_refusal(verdict, Taker::of_origin(claude), &ctx)
            .expect("Claude is refused the undo");
        assert!(why.contains("kanbanr prepare"), "{why}");
        assert_eq!(verdict_refusal(verdict, Taker::Person, &ctx), None);
    }

    /// FTR-038 R-4: a group decision is a verdict route, stamped with its origin, and refused to
    /// Claude in every mode, a granted delegation for every verdict included; a person is never
    /// refused it.
    #[test]
    fn a_group_decision_is_a_verdict_refused_to_claude() {
        let route = "/projects/demo/review/decide";
        assert_eq!(
            verdict_of_route("POST", route),
            Some(Verdict::GroupDecision)
        );
        assert_eq!(verdict_of_route("GET", route), None);
        assert!(is_verdict_route("POST", route));
        assert!(!is_verdict_route("GET", route));
        assert!(stamps_origin("POST", route));
        assert!(!is_verdict_route("POST", "/projects/demo/review"));
        assert!(Verdict::ALL.contains(&Verdict::GroupDecision));
        assert_eq!(
            Verdict::ALL.len(),
            Verdict::ALL
                .iter()
                .filter(|v| **v != Verdict::GroupDecision)
                .count()
                + 1
        );

        // Run by Claude (CLAUDECODE set), as the sibling tests read it.
        let claude = Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| "1".to_string()));
        assert_eq!(Taker::of_origin(claude), Taker::Claude);

        let (store, dir) = board();
        let project = store.load_meta("demo").unwrap();
        let ctx = DecisionContext::monitor_mode();
        let why = verdict_refusal(Verdict::GroupDecision, Taker::of_origin(claude), &ctx)
            .expect("Claude is refused a group decision");
        assert!(why.contains("kanbanr prepare"), "{why}");
        assert_eq!(
            verdict_refusal(Verdict::GroupDecision, Taker::Person, &ctx),
            None
        );
        let body = json!({"by": "Claude", "delegation": "DLG-001", "group": "approve"});
        for mode in ["monitor", "delegated", "conversation"] {
            let d: crate::decisions::Decisions = crate::yaml::from_str(&format!(
                "modes:\n- {{mode: {mode}, by: Ada, at: '2026-11-01T00:00:00Z', origin: monitor}}\n\
                 delegations:\n- id: DLG-001\n  verdicts: [approve, ratify]\n  all: true\n  \
                 until: '2099-01-01T00:00:00Z'\n  \
                 proposed: {{by: Claude, at: '2026-11-01T00:00:00Z', origin: conversation}}\n  \
                 granted: {{by: Ada, at: '2026-11-01T00:00:00Z', origin: monitor}}\n"
            ))
            .unwrap();
            for origin in [Origin::Conversation, Origin::Delegated] {
                let ctx = DecisionContext {
                    decisions: Some(&d),
                    project: Some(&project),
                    method: "POST",
                    path: route,
                    body: Some(&body),
                    origin: Some(origin),
                    now: "2026-11-17T00:00:00Z".into(),
                };
                let why = verdict_refusal(Verdict::GroupDecision, Taker::of_origin(origin), &ctx)
                    .unwrap_or_else(|| panic!("{mode}/{origin:?}: Claude took a group decision"));
                assert!(why.contains("group decision"), "{mode}: {why}");
            }
            for origin in [Origin::Monitor, Origin::Terminal] {
                let ctx = DecisionContext {
                    decisions: Some(&d),
                    project: Some(&project),
                    method: "POST",
                    path: route,
                    body: Some(&body),
                    origin: Some(origin),
                    now: "2026-11-17T00:00:00Z".into(),
                };
                assert_eq!(
                    verdict_refusal(Verdict::GroupDecision, Taker::Person, &ctx),
                    None,
                    "{mode}/{origin:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-2: recording a planning decision is a verdict route, stamped with its origin, and
    /// refused to Claude in every mode, a granted delegation for every verdict included; a person
    /// is never refused it. Declining one is not a verdict: Claude declines.
    #[test]
    fn a_planning_decision_is_refused_to_claude() {
        let route = "/projects/demo/planning";
        assert_eq!(
            verdict_of_route("POST", route),
            Some(Verdict::PlanningDecision)
        );
        assert_eq!(verdict_of_route("GET", route), None);
        assert!(is_verdict_route("POST", route));
        assert!(!is_verdict_route("GET", route));
        assert!(stamps_origin("POST", route));
        assert!(Verdict::ALL.contains(&Verdict::PlanningDecision));
        assert_eq!(
            Verdict::ALL.len(),
            Verdict::ALL
                .iter()
                .filter(|v| **v != Verdict::PlanningDecision)
                .count()
                + 1
        );
        // Declining, and the reads, are not verdicts.
        let decline = "/projects/demo/planning/PD-1/decline";
        assert_eq!(verdict_of_route("POST", decline), None);
        assert!(!is_verdict_route("POST", decline));
        assert!(stamps_origin("POST", decline));
        assert_eq!(
            verdict_of_route("GET", "/projects/demo/planning/PD-1/check"),
            None
        );
        // No CLI command records one.
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        for path in [
            &["planning", "list"][..],
            &["planning", "decline"],
            &["sprint", "plan"],
        ] {
            assert_eq!(verdict_of_command(path, &w(&[])), None, "{path:?}");
        }

        let claude = Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| "1".to_string()));
        assert_eq!(Taker::of_origin(claude), Taker::Claude);
        let ctx = DecisionContext::monitor_mode();
        let why = verdict_refusal(Verdict::PlanningDecision, Taker::of_origin(claude), &ctx)
            .expect("Claude is refused a planning decision");
        assert_eq!(why, PLANNING_REFUSAL);
        assert!(
            why.contains("Plan page") && why.contains("kanbanr sprint plan"),
            "{why}"
        );
        assert_eq!(
            verdict_refusal(Verdict::PlanningDecision, Taker::Person, &ctx),
            None
        );

        let (store, dir) = board();
        let project = store.load_meta("demo").unwrap();
        let claudes = json!({"by": "Claude", "delegation": "DLG-001", "action": "into-sprint"});
        let persons = json!({"by": "Ada", "action": "into-sprint"});
        for mode in ["monitor", "delegated", "conversation"] {
            let d: crate::decisions::Decisions = crate::yaml::from_str(&format!(
                "modes:\n- {{mode: {mode}, by: Ada, at: '2026-11-01T00:00:00Z', origin: monitor}}\n\
                 delegations:\n- id: DLG-001\n  verdicts: [approve, ratify]\n  all: true\n  \
                 until: '2099-01-01T00:00:00Z'\n  \
                 proposed: {{by: Claude, at: '2026-11-01T00:00:00Z', origin: conversation}}\n  \
                 granted: {{by: Ada, at: '2026-11-01T00:00:00Z', origin: monitor}}\n"
            ))
            .unwrap();
            for origin in [
                Origin::Conversation,
                Origin::Delegated,
                Origin::Monitor,
                Origin::Terminal,
            ] {
                let taker = Taker::of_origin(origin);
                let body = match taker {
                    Taker::Claude => &claudes,
                    Taker::Person => &persons,
                };
                let ctx = DecisionContext {
                    decisions: Some(&d),
                    project: Some(&project),
                    method: "POST",
                    path: route,
                    body: Some(body),
                    origin: Some(origin),
                    now: "2026-11-17T00:00:00Z".into(),
                };
                let got = verdict_refusal(Verdict::PlanningDecision, taker, &ctx);
                match taker {
                    Taker::Claude => {
                        assert_eq!(got.as_deref(), Some(PLANNING_REFUSAL), "{mode}/{origin:?}")
                    }
                    Taker::Person => assert_eq!(got, None, "{mode}/{origin:?}"),
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-9: the plan routes that carry out a planning decision are stamped with their
    /// origin, so the outcome says where it was carried out, whatever the body claims; they are
    /// not verdicts, and their reads are not stamped.
    #[test]
    fn a_plan_route_carrying_out_a_decision_is_stamped_with_its_origin() {
        for route in [
            "/projects/demo/sprints/SP-001/plan",
            "/projects/demo/sprints/unplan",
            "/projects/demo/releases/v0.9.0/plan",
        ] {
            assert!(stamps_origin("POST", route), "{route}");
            assert!(!stamps_origin("GET", route), "{route}");
            assert!(!is_verdict_route("POST", route), "{route}");
            assert_eq!(verdict_of_route("POST", route), None, "{route}");
            let claimed = json!({"items": ["FEAT-001"], "decision": "PD-1", "origin": "monitor"});
            let stamped = stamp("POST", route, Some(&claimed), Origin::Terminal).unwrap();
            assert_eq!(from_body(&stamped), "terminal", "{route}");
            assert_eq!(stamped["decision"], "PD-1", "{route}");
        }
        assert!(!stamps_origin(
            "POST",
            "/projects/demo/sprints/SP-001/start"
        ));
    }

    // ---- a scratch board -----------------------------------------------------------------

    use crate::config::ProjectConfig;
    use crate::models::FeatureDefinition;
    use crate::store::Store;

    fn board() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-origin-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    /// An item with a definition (and, with `bypass`, a recorded unapproved start).
    fn item(store: &Store, bypass: bool) -> String {
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    started_unapproved: if bypass {
                        "urgent".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                }),
            )
            .unwrap();
        f.code
    }

    fn def(store: &Store, code: &str) -> FeatureDefinition {
        store
            .load("demo")
            .unwrap()
            .feature(code)
            .unwrap()
            .definition
            .clone()
            .unwrap()
    }

    #[test]
    fn an_approval_a_ratification_a_withdrawal_and_a_signoff_record_their_origin() {
        let (store, dir) = board();
        let code = item(&store, true);
        store
            .approve_feature_as("demo", &code, "Ada", "monitor", "", None)
            .unwrap();
        let d = def(&store, &code);
        assert_eq!(d.approval.as_ref().unwrap().origin, "monitor");
        assert_eq!(d.approvals.last().unwrap().origin, "monitor");

        store
            .unapprove_feature_as("demo", &code, "Ada", "wrong item", "terminal")
            .unwrap();
        let d = def(&store, &code);
        assert!(d.approval.is_none());
        assert_eq!(d.approvals.last().unwrap().origin, "terminal");

        store
            .ratify_feature_as(
                "demo",
                &code,
                "Ada",
                "built first",
                "conversation",
                "",
                None,
            )
            .unwrap();
        let d = def(&store, &code);
        assert_eq!(d.approval.as_ref().unwrap().origin, "conversation");
        assert_eq!(d.approvals.last().unwrap().origin, "conversation");
        let origins: Vec<&str> = d.approvals.iter().map(|e| e.origin.as_str()).collect();
        assert_eq!(origins, ["monitor", "terminal", "conversation"]);

        store
            .signoff_feature_as("demo", &code, "design-review", "Ada", "", "", "monitor")
            .unwrap();
        store
            .withdraw_signoff_as(
                "demo",
                &code,
                "design-review",
                "Ada",
                "too early",
                "terminal",
            )
            .unwrap();
        let d = def(&store, &code);
        let origins: Vec<&str> = d.signoffs.iter().map(|e| e.origin.as_str()).collect();
        assert_eq!(origins, ["monitor", "terminal"]);
        // The old names record none: they read as unknown.
        store.approve_feature("demo", &code, "Ada").unwrap();
        assert_eq!(def(&store, &code).approvals.last().unwrap().origin, "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_adr_verdict_records_its_origin() {
        use crate::adr::{Adr, create, decide_as, get, update};
        let (store, dir) = board();
        let filled = |title: &str| {
            let mut adr = create(&store, "demo", title, Adr::default()).unwrap();
            adr.body = format!(
                "# {title}\n\n## Context\n\nc\n\n## Decision\n\nd\n\n\
                 ## Alternatives considered\n\na\n\n## Consequences\n\nq\n\n## Compliance\n\nt\n"
            );
            update(&store, "demo", &adr).unwrap();
            adr.id
        };
        let (yes, no) = (filled("Keep it"), filled("Drop it"));
        decide_as(&store, "demo", &yes, true, "Ada", "", "monitor").unwrap();
        decide_as(&store, "demo", &no, false, "Ada", "too dear", "terminal").unwrap();
        assert_eq!(get(&store, "demo", &yes).unwrap().decided_origin, "monitor");
        assert_eq!(get(&store, "demo", &no).unwrap().decided_origin, "terminal");
        // It is in the front matter, beside who decided and when.
        let raw = store
            .read_doc("demo", &get(&store, "demo", &yes).unwrap().path)
            .unwrap();
        assert!(raw.contains("decided_origin: monitor"), "{raw}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_old_record_reads_as_unknown_and_reading_does_not_rewrite_it() {
        let (store, dir) = board();
        let code = item(&store, false);
        // Records as an older kanbanr wrote them: through the names that pass no origin.
        store.approve_feature("demo", &code, "Ada").unwrap();
        store
            .signoff_feature("demo", &code, "design-review", "Ada", "", "")
            .unwrap();
        let snapshot = |root: &std::path::Path| {
            fn walk(p: &std::path::Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
                for e in std::fs::read_dir(p).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else {
                        out.push((p.clone(), std::fs::read(&p).unwrap()));
                    }
                }
            }
            let mut out = Vec::new();
            walk(root, &mut out);
            out.sort();
            out
        };
        let before = snapshot(&dir);
        let project = store.load("demo").unwrap();
        let d = project.feature(&code).unwrap().definition.clone().unwrap();
        assert_eq!(label(&d.approvals[0].origin), "unknown");
        assert_eq!(label(&d.signoffs[0].origin), "unknown");
        assert!(
            !before
                .iter()
                .any(|(_, b)| String::from_utf8_lossy(b).contains("origin"))
        );
        let shown = crate::export::definition_sections(project.feature(&code).unwrap(), &d);
        assert!(shown.contains("origin unknown"), "{shown}");
        assert_eq!(before, snapshot(&dir), "reading rewrote the board");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_origin_an_older_kanbanr_does_not_know_is_kept_when_it_writes() {
        use crate::models::{ApprovalEvent, Signoff};
        let e: ApprovalEvent =
            crate::yaml::from_str("at: a\nby: b\nverdict: approved\norigin: scheduled\nnewer: 1\n")
                .unwrap();
        assert_eq!(e.origin, "scheduled");
        let out = crate::yaml::to_string(&e).unwrap();
        assert!(
            out.contains("origin: scheduled") && out.contains("newer: 1"),
            "{out}"
        );
        let s: Signoff =
            crate::yaml::from_str("id: x\nby: b\nat: a\nrev: r\norigin: scheduled\n").unwrap();
        assert!(
            crate::yaml::to_string(&s)
                .unwrap()
                .contains("origin: scheduled")
        );
        let c: crate::releases::StageChange =
            crate::yaml::from_str("stage: s\nby: b\nat: a\norigin: scheduled\nnewer: 2\n").unwrap();
        let out = crate::yaml::to_string(&c).unwrap();
        assert!(
            out.contains("origin: scheduled") && out.contains("newer: 2"),
            "{out}"
        );
        // A value the monitor or CLI would not send is shown as it is.
        assert_eq!(label(&c.origin), "scheduled");
        assert!(via(&c.origin).contains("scheduled"));
    }

    // ---- duplicates and refusals ---------------------------------------------------------

    fn post(
        store: &Store,
        path: &str,
        body: serde_json::Value,
        origin: Origin,
    ) -> serde_json::Value {
        let body = stamp("POST", path, Some(&body), origin).unwrap();
        let out = crate::dispatch::dispatch(store, "POST", path, Some(&body)).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn a_second_identical_approval_records_nothing() {
        let (store, dir) = board();
        let code = item(&store, true);
        let path = format!("/projects/demo/features/{code}/approve");
        let first = post(&store, &path, json!({"by": "Ada"}), Origin::Monitor);
        assert!(first.get("already_recorded").is_none());
        // The same verdict again, from a terminal: nothing is recorded; the answer says who, when
        // and from where the first came.
        let again = post(&store, &path, json!({"by": "Ada"}), Origin::Terminal);
        let said = &again["already_recorded"];
        assert_eq!(
            (said["by"].as_str(), said["origin"].as_str()),
            (Some("Ada"), Some("monitor"))
        );
        assert!(
            said["sentence"]
                .as_str()
                .unwrap()
                .ends_with("from the monitor"),
            "{said}"
        );
        let d = def(&store, &code);
        assert_eq!(d.approvals.len(), 1);
        let earlier = duplicate_approval(&d, "approved").unwrap();
        assert_eq!(
            (earlier.by.as_str(), earlier.origin.as_str()),
            ("Ada", "monitor")
        );
        // Ratifying an approved item is a different verdict.
        assert!(duplicate_approval(&d, "ratified").is_none());
        let rat = post(
            &store,
            &format!("/projects/demo/features/{code}/ratify"),
            json!({"by": "Ada", "reason": "built first"}),
            Origin::Terminal,
        );
        assert!(rat.get("already_recorded").is_none());
        assert_eq!(def(&store, &code).approvals.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-064 R-1: an override Claude made after the latest ratification, at the revision that
    /// ratification covered, keeps a ratification from being a repeat; one new ratification
    /// closes every override waiting on it, two at the same revision included.
    #[test]
    fn a_ratification_after_a_second_override_at_the_same_revision_is_not_a_repeat() {
        use crate::models::{Approval, ApprovalEvent, Transition};
        let (store, dir) = board();
        let code = item(&store, true);
        let mut f = store.load("demo").unwrap().feature(&code).unwrap().clone();
        let _ = std::fs::remove_dir_all(dir);
        let mut d = f.definition.clone().unwrap();
        let rev = d.content_rev();
        let event = |at: &str, verdict: &str| ApprovalEvent {
            at: at.into(),
            by: "Ada".into(),
            verdict: verdict.into(),
            rev: rev.clone(),
            origin: "monitor".into(),
            ..Default::default()
        };
        d.approvals = vec![
            event("2026-11-13T10:00:00.000Z", "approved"),
            event("2026-11-14T12:00:00.000Z", "ratified"),
        ];
        d.approval = Some(Approval {
            by: "Ada".into(),
            at: "2026-11-14T12:00:00.000Z".into(),
            rev: rev.clone(),
            ..Default::default()
        });
        d.started_unapproved = "why".into();
        f.definition = Some(d);
        let moved = |at: &str, why: &str| Transition {
            at: at.into(),
            from: "Planned".into(),
            to: "Scheduled".into(),
            override_reason: Some(why.into()),
            origin: "conversation".into(),
            ..Default::default()
        };
        f.history = vec![
            moved("2026-11-14T10:00:00.000Z", "why 1"),
            moved("2026-11-15T09:00:00.000Z", "why 2"),
        ];

        let open: Vec<&str> = crate::readiness::open_overrides(&f)
            .iter()
            .map(|t| t.at.as_str())
            .collect();
        assert_eq!(open, ["2026-11-15T09:00:00.000Z"]);
        // The old rule alone calls this a repeat: the defect.
        assert!(duplicate_approval(f.definition.as_ref().unwrap(), "ratified").is_some());
        assert_eq!(duplicate_ratification(&f), None);

        // A second override at the same ratified revision: both wait.
        f.history.push(moved("2026-11-15T09:10:00.000Z", "why 3"));
        assert_eq!(crate::readiness::open_overrides(&f).len(), 2);
        assert_eq!(duplicate_ratification(&f), None);

        // One ratification closes both; another would be the repeat.
        f.definition
            .as_mut()
            .unwrap()
            .approvals
            .push(event("2026-11-15T09:30:00.000Z", "ratified"));
        assert!(crate::readiness::open_overrides(&f).is_empty());
        let earlier = duplicate_ratification(&f).expect("a repeat");
        assert_eq!(
            (earlier.what.as_str(), earlier.at.as_str()),
            ("ratified", "2026-11-15T09:30:00.000Z")
        );
    }

    /// DEF-064 R-1, R-2: a second override Claude made at a revision the person already ratified
    /// is ratified again through the route, and leaves the queue; a true repeat still records
    /// nothing. Each override is one its gate needed, or it would not be recorded at all (DEF-056
    /// R-2): Scheduled and Completed ask for an estimate, which the item never gets, and an
    /// estimate is no part of the definition, so the revision the person ratified stays the same.
    #[test]
    fn ratifying_a_second_override_at_a_ratified_revision_records_it_and_leaves_the_queue() {
        let (store, dir) = board();
        assert!(dir.starts_with(std::env::temp_dir()));
        // Gates are enforced only under an adopted charter, and only on items made after it.
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
        let code = item(&store, false);
        let estimated = || crate::config::Gate {
            requires: vec![crate::readiness::Condition::Check(
                crate::readiness::Check::Estimated,
            )],
            ..Default::default()
        };
        store
            .set_gates(
                "demo",
                [
                    ("Scheduled".to_string(), estimated()),
                    ("Completed".to_string(), estimated()),
                ]
                .into_iter()
                .collect(),
            )
            .unwrap();
        let base = format!("/projects/demo/features/{code}");
        let listed = |store: &Store| {
            crate::review::pending(store, "demo", None)
                .unwrap()
                .into_iter()
                .find(|e| e.code == code)
        };
        let first = post(
            &store,
            &format!("{base}/approve"),
            json!({"by": "Ada"}),
            Origin::Monitor,
        );
        assert!(first.get("already_recorded").is_none());

        assert!(
            store
                .move_feature_gated_as("demo", &code, "Scheduled", None, "conversation")
                .is_err(),
            "the gate blocks, so the override below is needed"
        );
        store
            .move_feature_gated_as("demo", &code, "Scheduled", Some("why 1"), "conversation")
            .unwrap();
        let rat = post(
            &store,
            &format!("{base}/ratify"),
            json!({"by": "Ada", "reason": "agreed"}),
            Origin::Monitor,
        );
        assert!(rat.get("already_recorded").is_none(), "{rat}");
        assert_eq!(def(&store, &code).approvals.len(), 2);
        assert!(listed(&store).is_none());

        assert!(
            store
                .move_feature_gated_as("demo", &code, "Completed", None, "conversation")
                .is_err(),
            "the gate blocks, so the override below is needed"
        );
        store
            .move_feature_gated_as("demo", &code, "Completed", Some("why 2"), "conversation")
            .unwrap();
        let entry = listed(&store).expect("asked again");
        assert_eq!(entry.approval, crate::review::ApprovalKind::Unratified);
        assert_eq!(entry.overrides.len(), 1);

        // The defect: this answered `already_recorded` and recorded nothing.
        let rat = post(
            &store,
            &format!("{base}/ratify"),
            json!({"by": "Ada", "reason": "agreed again"}),
            Origin::Monitor,
        );
        assert!(rat.get("already_recorded").is_none(), "{rat}");
        let d = def(&store, &code);
        assert_eq!(d.approvals.len(), 3);
        let last = d.approvals.last().unwrap();
        assert_eq!(
            (
                last.verdict.as_str(),
                last.origin.as_str(),
                last.rev.clone()
            ),
            ("ratified", "monitor", d.content_rev())
        );
        let f = store.load("demo").unwrap().feature(&code).unwrap().clone();
        assert!(crate::readiness::open_overrides(&f).is_empty());
        assert!(listed(&store).is_none());

        // R-2: with nothing open, a repeat from a terminal records nothing and says who first.
        let again = post(
            &store,
            &format!("{base}/ratify"),
            json!({"by": "Ada", "reason": "once more"}),
            Origin::Terminal,
        );
        let said = &again["already_recorded"];
        assert_eq!(
            (said["by"].as_str(), said["origin"].as_str()),
            (Some("Ada"), Some("monitor"))
        );
        assert!(
            said["sentence"]
                .as_str()
                .unwrap()
                .ends_with("from the monitor"),
            "{said}"
        );
        assert_eq!(def(&store, &code).approvals.len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_identical_signoff_records_nothing_but_a_new_note_or_doc_does() {
        let (store, dir) = board();
        let code = item(&store, false);
        let path = format!("/projects/demo/features/{code}/signoff/design-review");
        let go = |body: serde_json::Value, o| post(&store, &path, body, o);
        let first = go(json!({"by": "Ada"}), Origin::Monitor);
        assert!(first.get("already_recorded").is_none());
        let again = go(json!({"by": "Ada"}), Origin::Conversation);
        assert_eq!(again["already_recorded"]["origin"], "monitor");
        // The same note is still the same sign-off; a new note, or a doc, adds something.
        assert_eq!(def(&store, &code).signoffs.len(), 1);
        let noted = go(
            json!({"by": "Ada", "note": "minutes agreed"}),
            Origin::Terminal,
        );
        assert!(noted.get("already_recorded").is_none());
        let same_note = go(
            json!({"by": "Ada", "note": "minutes agreed"}),
            Origin::Terminal,
        );
        assert!(same_note.get("already_recorded").is_some());
        let doc = go(json!({"by": "Ada", "doc": "minutes.md"}), Origin::Terminal);
        assert!(doc.get("already_recorded").is_none());
        assert_eq!(def(&store, &code).signoffs.len(), 3);
        // After a withdrawal the next one is new.
        let w = format!("{path}/withdraw");
        post(
            &store,
            &w,
            json!({"by": "Ada", "reason": "wrong"}),
            Origin::Terminal,
        );
        let item_now = store.load("demo").unwrap().feature(&code).unwrap().clone();
        assert!(duplicate_signoff(&item_now, "design-review", "", "").is_none());
        let back = go(json!({"by": "Ada"}), Origin::Terminal);
        assert!(back.get("already_recorded").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_same_verdict_on_changed_text_is_recorded() {
        let (store, dir) = board();
        let code = item(&store, false);
        let path = format!("/projects/demo/features/{code}/approve");
        post(&store, &path, json!({"by": "Ada"}), Origin::Monitor);
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 14 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        assert!(duplicate_approval(&def(&store, &code), "approved").is_none());
        let again = post(&store, &path, json!({"by": "Ada"}), Origin::Monitor);
        assert!(again.get("already_recorded").is_none());
        assert_eq!(def(&store, &code).approvals.len(), 2);
        // And a sign-off on text that has changed since is not a repeat either.
        let sp = format!("/projects/demo/features/{code}/signoff/design-review");
        post(&store, &sp, json!({"by": "Ada"}), Origin::Monitor);
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 30 days".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let item_now = store.load("demo").unwrap().feature(&code).unwrap().clone();
        assert!(duplicate_signoff(&item_now, "design-review", "", "").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_already_done_refusal_names_who_when_and_from_where() {
        use crate::adr::{Adr, create, decide_as, update};
        use crate::releases::tests::{STAGED, board_and_repo, clean, staged};
        let (store, dir) = board();
        // A decision decided twice.
        let mut adr = create(&store, "demo", "Keep it", Adr::default()).unwrap();
        adr.body = "# Keep it\n\n## Context\n\nc\n\n## Decision\n\nd\n\n## Alternatives considered\n\na\n\n## Consequences\n\nq\n\n## Compliance\n\nt\n".into();
        update(&store, "demo", &adr).unwrap();
        decide_as(&store, "demo", &adr.id, true, "Ada", "", "monitor").unwrap();
        let why = decide_as(&store, "demo", &adr.id, true, "Bo", "", "terminal")
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("decided by Ada on") && why.contains("from the monitor"),
            "{why}"
        );
        // An approval withdrawn twice, and a sign-off withdrawn twice.
        let code = item(&store, false);
        store
            .approve_feature_as("demo", &code, "Ada", "monitor", "", None)
            .unwrap();
        store
            .unapprove_feature_as("demo", &code, "Ada", "no", "conversation")
            .unwrap();
        let why = store
            .unapprove_feature_as("demo", &code, "Bo", "no", "terminal")
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("withdrawn by Ada on") && why.contains("from the conversation"),
            "{why}"
        );
        store
            .signoff_feature_as("demo", &code, "dr", "Ada", "", "", "monitor")
            .unwrap();
        store
            .withdraw_signoff_as("demo", &code, "dr", "Ada", "no", "terminal")
            .unwrap();
        let why = store
            .withdraw_signoff_as("demo", &code, "dr", "Bo", "no", "monitor")
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("already withdrawn: withdrawn by Ada on")
                && why.contains("from the terminal"),
            "{why}"
        );
        let _ = std::fs::remove_dir_all(dir);
        // A cut twice.
        let (rstore, rdir, rrepo) = board_and_repo("def028-twice");
        crate::releases::cut_as(&rstore, "shop", "v1", "Grace", "terminal", &[]).unwrap();
        let why = crate::releases::cut_as(&rstore, "shop", "v1", "Bo", "monitor", &[])
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("already shipped")
                && why.contains("cut by Grace on")
                && why.contains("from the terminal"),
            "{why}"
        );
        clean(&rdir, &rrepo);
        // A promotion to the stage a release is at.
        let (pstore, pdir, prepo) = staged("def028-again", STAGED);
        promote_as(&pstore, &prepo, "candidate", "Ada", "monitor").unwrap();
        let why = promote_as(&pstore, &prepo, "candidate", "Bo", "terminal")
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("already at candidate")
                && why.contains("moved there by Ada on")
                && why.contains("from the monitor"),
            "{why}"
        );
        clean(&pdir, &prepo);
    }

    // ---- releases ------------------------------------------------------------------------

    fn promote_as(
        store: &Store,
        repo: &std::path::Path,
        to: &str,
        by: &str,
        origin: &str,
    ) -> crate::Result<crate::releases::Promoted> {
        crate::releases::promote_with(
            store,
            "shop",
            "v1",
            Some(to),
            by,
            "",
            Some(repo),
            crate::releases::PromoteOptions {
                expect_tip: None,
                fetch: crate::releases::Fetch::No,
                origin,
                accept_untested: &[],
            },
        )
    }

    #[test]
    fn a_promotion_a_confirmation_a_patch_and_a_cut_record_their_origin() {
        use crate::releases::tests::{board_and_repo, clean, now_result, staged, v1};
        // A promotion, a confirmation (a stage after the tag, which only confirms) in turn.
        let (store, dir, repo) = staged(
            "def028-origins",
            "release_stages: [candidate, main, tag, test, staging]\n\
             release_gates:\n  candidate: { on_enter: [branch] }\n  \
             main: { on_enter: [move_main] }\n  tag: { on_enter: [tag] }\n",
        );
        promote_as(&store, &repo, "candidate", "Ada", "monitor").unwrap();
        promote_as(&store, &repo, "main", "Ada", "terminal").unwrap();
        promote_as(&store, &repo, "tag", "Ada", "conversation").unwrap();
        now_result(&store, &v1(&store).tag_commit, true);
        promote_as(&store, &repo, "test", "Grace", "monitor").unwrap();
        let seen: Vec<(String, String)> = v1(&store)
            .stages
            .iter()
            .map(|c| (c.stage.clone(), c.origin.clone()))
            .collect();
        let want: Vec<(String, String)> = [
            ("candidate", "monitor"),
            ("main", "terminal"),
            ("tag", "conversation"),
            ("test", "monitor"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(seen, want);
        clean(&dir, &repo);

        // A patch started from a failed tag: its first stage change carries the origin.
        let (store, dir, repo, sha) =
            crate::releases::tests::tagged("def028-patch", crate::releases::tests::STAGED);
        now_result(&store, &sha, false);
        let made = crate::releases::patch_from_failed_tag(
            &store,
            "shop",
            "v1",
            "v1.0.1",
            "Ada",
            "",
            "two red",
            Some(&repo),
            "terminal",
        )
        .unwrap();
        assert_eq!(made.release.stages[0].origin, "terminal");
        clean(&dir, &repo);

        // A cut by `release cut` names who cut it and from where.
        let (store, dir, repo) = board_and_repo("def028-cut");
        let cut =
            crate::releases::cut_as(&store, "shop", "v1", "Grace", "conversation", &[]).unwrap();
        assert_eq!(
            (
                cut.release.shipped_by.as_str(),
                cut.release.shipped_origin.as_str()
            ),
            ("Grace", "conversation")
        );
        clean(&dir, &repo);
    }

    #[test]
    fn a_cut_made_by_promoting_to_the_last_stage_records_the_same_origin() {
        use crate::releases::tests::{STAGED, clean, now_result, tagged, v1};
        let (store, dir, repo, sha) = tagged("def028-ship", STAGED);
        now_result(&store, &sha, true);
        promote_as(&store, &repo, "staging", "Ada", "monitor").unwrap();
        let r = v1(&store);
        assert_eq!(r.state, crate::releases::ReleaseState::Shipped);
        assert_eq!(
            (r.shipped_by.as_str(), r.shipped_origin.as_str()),
            ("Ada", "monitor")
        );
        assert_eq!(r.stages.last().unwrap().origin, "monitor");
        clean(&dir, &repo);
    }

    // ---- the write recipe ----------------------------------------------------------------

    fn commits(dir: &std::path::Path) -> usize {
        let out = std::process::Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    #[test]
    fn the_activity_entry_and_the_event_carry_the_origin() {
        use crate::{activity, eventing, git::PushPolicy, write};
        let (store, dir) = board();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let code = item(&store, false);
        let scope = crate::agentscope::ProcessScope::Unscoped;
        let go = |path: &str, body: serde_json::Value, o: Origin| {
            write::apply_as(
                &store,
                &dir,
                PushPolicy::Off,
                "POST",
                path,
                Some(&body),
                None,
                &eventing::NullSender,
                &scope,
                o,
            )
            .unwrap()
        };
        let path = format!("/projects/demo/features/{code}/approve");
        // The request claims to be a terminal; the front end says monitor, and wins.
        let first = go(
            &path,
            json!({"by": "Ada", "origin": "terminal"}),
            Origin::Monitor,
        );
        assert!(!first.body.contains("already_recorded"));
        let d = def(&store, &code);
        assert_eq!(d.approvals[0].origin, "monitor");
        let said = activity::read(&dir, "demo", 10);
        assert_eq!(said[0].origin, "monitor", "{said:?}");
        let events = eventing::read(&dir, "demo", 10);
        assert!(
            events.iter().any(|e| e.origin == "monitor"),
            "the event carries it: {events:?}"
        );
        // A repeat from a command records nothing: no entry, no event, no commit.
        let (n_commits, n_said, n_events) = (
            commits(&dir),
            activity::read(&dir, "demo", 50).len(),
            eventing::read(&dir, "demo", 50).len(),
        );
        let again = go(&path, json!({"by": "Ada"}), Origin::Terminal);
        assert!(again.body.contains("already_recorded"));
        assert_eq!(def(&store, &code).approvals.len(), 1);
        assert_eq!(commits(&dir), n_commits, "a commit was made");
        assert_eq!(activity::read(&dir, "demo", 50).len(), n_said);
        assert_eq!(eventing::read(&dir, "demo", 50).len(), n_events);
        // A write that is not a verdict carries no origin.
        go(
            &format!("/projects/demo/features/{code}/todos"),
            json!({"description": "d"}),
            Origin::Terminal,
        );
        assert_eq!(activity::read(&dir, "demo", 1)[0].origin, "");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-028 R-3: a record taken under a delegation reads as such, where the origin shows.
    #[test]
    fn delegated_reads_as_under_a_delegation() {
        assert_eq!(via("delegated"), " under a delegation");
        let e = Earlier {
            what: "signed off design-review".into(),
            by: "Claude".into(),
            at: "2026-11-12T10:00:00+00:00".into(),
            origin: "delegated".into(),
        };
        assert!(
            e.sentence().ends_with("from a delegation"),
            "{}",
            e.sentence()
        );
        let b = json!({"delegation": " DLG-001 "});
        assert_eq!(delegation_from_body(&b), "DLG-001");
        assert_eq!(delegation_from_body(&json!({})), "");
        assert_eq!(delegation_from_body(&json!({"delegation": 3})), "");
    }
}
