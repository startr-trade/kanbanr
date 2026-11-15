//! The runbook (FTR-037 W2, W2b): a board doc whose first fenced `yaml` block holds the run's
//! steps — what to say, how to answer Claude's questions, the decision and the offer each step
//! expects, the pushes, the board and truth claims, and the `MT-n` check markers.
//!
//! A runbook is data: parsing never runs anything, and `lint` says what a run would stumble on
//! before one starts (a check with no definition, a decision with no offer to check first, an
//! answer rule that does not compile).

use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::BTreeMap;

/// One of the eight v0.6.0 checks, or any other test a run can record: the item, the requirement
/// and the test's exact name on the board.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRef {
    pub item: String,
    pub requirement: String,
    pub test: String,
}

/// Which item a decision or an offer is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSel {
    /// The item created last.
    Newest,
    /// By code.
    Code(String),
    /// The item whose title contains this text (case-insensitive); the newest when several do.
    TitleContains(String),
    /// The item whose title matches this pattern (case-insensitive); the newest when several do.
    TitleMatches(String),
}

/// A title pattern compiled case-insensitively; an invalid one matches nothing (lint names it).
pub fn title_regex(pattern: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

impl ItemSel {
    /// How a runbook names it, for findings and logs.
    pub fn describe(&self) -> String {
        match self {
            ItemSel::Newest => "the newest item".into(),
            ItemSel::Code(c) => c.clone(),
            ItemSel::TitleContains(t) => format!("the item titled like \"{t}\""),
            ItemSel::TitleMatches(p) => format!("the item whose title matches /{p}/"),
        }
    }

    /// Does an item with this title satisfy the selector (ignoring Newest and Code)?
    pub fn title_fits(&self, title: &str) -> bool {
        match self {
            ItemSel::TitleContains(t) => title.to_lowercase().contains(&t.to_lowercase()),
            ItemSel::TitleMatches(p) => title_regex(p).is_some_and(|r| r.is_match(title)),
            _ => false,
        }
    }
}

/// Which delegation a grant is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationSel {
    /// Every proposed delegation.
    Proposed,
    /// By id.
    Id(String),
}

/// A decision the scripted person takes through the monitor's write route.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve(ItemSel),
    Signoff {
        item: ItemSel,
        name: String,
    },
    Ratify(ItemSel),
    Adr {
        /// An ADR id, or `newest` for the newest proposed one.
        adr: String,
        /// `accept` or `reject`.
        verdict: String,
    },
    Promote {
        release: String,
        /// The stage; the plan's next one when none.
        to: Option<String>,
    },
    GateResult {
        release: String,
        /// The gate's name, `ci` by default.
        gate: String,
        passed: bool,
        link: Option<String>,
    },
    /// Confirm the newest open prepared record of this kind; with `release`, the one for that
    /// release (a promotion of it, or a CI result for its tip or its tag).
    ConfirmPrepared {
        kind: String,
        release: Option<String>,
    },
    GrantDelegation(DelegationSel),
    SetMode(String),
    RecordManual {
        item: ItemSel,
        /// The requirement, when known; else the waiting test whose name contains `test`.
        requirement: Option<String>,
        /// The test's name, or a part of it.
        test: String,
        passed: bool,
    },
}

impl Decision {
    /// A short name for logs and the decisions table.
    pub fn kind(&self) -> &'static str {
        match self {
            Decision::Approve(_) => "approve",
            Decision::Signoff { .. } => "signoff",
            Decision::Ratify(_) => "ratify",
            Decision::Adr { .. } => "adr",
            Decision::Promote { .. } => "promote",
            Decision::GateResult { .. } => "gate-result",
            Decision::ConfirmPrepared { .. } => "confirm-prepared",
            Decision::GrantDelegation(_) => "grant-delegation",
            Decision::SetMode(_) => "set-mode",
            Decision::RecordManual { .. } => "manual-test",
        }
    }
}

/// Press a button on the rendered page instead of posting the route (the person's choice, 15 Nov
/// 2026: a step is `decide` or `press`, never both).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub page: String,
    pub button: String,
    pub subject: Option<ItemSel>,
}

/// The button the guide says is offered before a decision: checked first; absent is a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// `review` or `releases`.
    pub page: String,
    pub button: String,
    pub subject: Option<ItemSel>,
}

/// A rule answering Claude's question: the first whose `match` finds the question text wins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRule {
    #[serde(rename = "match")]
    pub pattern: String,
    pub pick: String,
}

impl AnswerRule {
    /// The compiled rule, or why it does not compile. Case-insensitive.
    pub fn regex(&self) -> std::result::Result<regex::Regex, String> {
        regex::RegexBuilder::new(&self.pattern)
            .case_insensitive(true)
            .build()
            .map_err(|e| e.to_string())
    }

    /// Does the rule answer this question?
    pub fn matches(&self, question: &str) -> bool {
        self.regex().is_ok_and(|r| r.is_match(question))
    }
}

/// What the board files must say after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardClaim {
    /// The run project exists on the board.
    ProjectExists,
    /// The release is at this stage (`candidate`, `main`, `tag`, `shipped`…).
    Release { version: String, stage: String },
    /// An item exists whose title contains this text (W2b).
    ItemTitleContains(String),
    /// An item exists whose title matches this pattern, case-insensitive (T16).
    ItemTitleMatches(String),
    /// The item is at this status.
    ItemStatus { item: ItemSel, status: String },
    /// Release preparation (notes, changelog, version files) is configured (W2b).
    ReleasePrepareConfigured,
    /// A sprint is running.
    SprintActive,
}

/// What must be so outside the board: the remote and CI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruthClaim {
    /// The release's tag is on the code remote.
    Tag { release: String },
    /// CI passed (or failed) for the release's tag.
    Ci { release: String, passed: bool },
    /// The item's branch is merged into the sprint branch.
    Merged { item: ItemSel },
    /// `kanbanr doctor` reports nothing.
    DoctorClean,
}

/// The rendered page must show (or not show) these texts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageClaim {
    /// A monitor path (`/p/{project}/review`) or a full URL.
    pub url: String,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub absent: Vec<String>,
}

/// What the step's conversation must have done: a question asked (or not), a thing said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TalkClaim {
    /// A question matching this was asked during the step.
    pub asked: Option<String>,
    /// No question matching this was asked during the step.
    pub not_asked: Option<String>,
    /// Claude's final text of the step matches this.
    pub said: Option<String>,
}

/// What a step expects afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expect {
    pub board: Option<BoardClaim>,
    pub truth: Option<TruthClaim>,
    pub page: Option<PageClaim>,
    pub talk: Option<TalkClaim>,
}

impl Expect {
    pub fn is_empty(&self) -> bool {
        self.board.is_none() && self.truth.is_none() && self.page.is_none() && self.talk.is_none()
    }
}

/// Which repository a push is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeOrBoard {
    Code,
    Board,
}

/// A push the guide names: only ever to the run's own remotes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Push {
    pub repo: CodeOrBoard,
    pub refspec: String,
}

/// One step of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub id: String,
    pub say: Option<String>,
    pub answers: Vec<AnswerRule>,
    pub decide: Option<Decision>,
    pub press: Option<Press>,
    pub offer: Option<Offer>,
    pub expect: Option<Expect>,
    pub push: Option<Push>,
    pub check: Option<String>,
    pub stop_on_mismatch: bool,
}

/// A runbook: the first fenced `yaml` block of a board doc.
#[derive(Debug, Clone, PartialEq)]
pub struct Runbook {
    pub name: String,
    pub process: String,
    pub model: String,
    pub budget_usd: f64,
    pub max_minutes: u64,
    pub checks: BTreeMap<String, CheckRef>,
    /// Rules that answer at any step, after the current step's own.
    pub answers: Vec<AnswerRule>,
    pub steps: Vec<Step>,
}

// ---- the YAML shapes --------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawItemSel {
    Word(String),
    Titled { title_contains: String },
    TitledLike { title_matches: String },
}

impl RawItemSel {
    fn into_sel(self) -> ItemSel {
        match self {
            RawItemSel::Word(w) if w.eq_ignore_ascii_case("newest") => ItemSel::Newest,
            RawItemSel::Word(w) => ItemSel::Code(w),
            RawItemSel::Titled { title_contains } => ItemSel::TitleContains(title_contains),
            RawItemSel::TitledLike { title_matches } => ItemSel::TitleMatches(title_matches),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    item: RawItemSel,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignoff {
    item: RawItemSel,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAdr {
    adr: String,
    verdict: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPromote {
    release: String,
    #[serde(default)]
    to: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate {
    release: String,
    #[serde(default)]
    gate: Option<String>,
    passed: bool,
    #[serde(default)]
    link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKind {
    kind: String,
    #[serde(default)]
    release: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrant {
    delegation: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManual {
    item: RawItemSel,
    #[serde(default)]
    requirement: Option<String>,
    test: String,
    passed: bool,
}

/// A decision is a map with one key.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    approve: Option<RawItem>,
    signoff: Option<RawSignoff>,
    ratify: Option<RawItem>,
    adr: Option<RawAdr>,
    promote: Option<RawPromote>,
    gate_result: Option<RawGate>,
    confirm_prepared: Option<RawKind>,
    grant_delegation: Option<RawGrant>,
    set_mode: Option<String>,
    record_manual: Option<RawManual>,
}

impl RawDecision {
    fn into_decision(self, step: &str) -> Result<Decision> {
        let mut out: Vec<Decision> = Vec::new();
        if let Some(a) = self.approve {
            out.push(Decision::Approve(a.item.into_sel()));
        }
        if let Some(s) = self.signoff {
            out.push(Decision::Signoff {
                item: s.item.into_sel(),
                name: s.name,
            });
        }
        if let Some(r) = self.ratify {
            out.push(Decision::Ratify(r.item.into_sel()));
        }
        if let Some(a) = self.adr {
            if !matches!(a.verdict.as_str(), "accept" | "reject") {
                return Err(anyhow!(
                    "step {step}: an ADR verdict is accept or reject, not '{}'",
                    a.verdict
                ));
            }
            out.push(Decision::Adr {
                adr: a.adr,
                verdict: a.verdict,
            });
        }
        if let Some(p) = self.promote {
            out.push(Decision::Promote {
                release: p.release,
                to: p.to,
            });
        }
        if let Some(g) = self.gate_result {
            out.push(Decision::GateResult {
                release: g.release,
                gate: g.gate.unwrap_or_else(|| "ci".into()),
                passed: g.passed,
                link: g.link,
            });
        }
        if let Some(k) = self.confirm_prepared {
            out.push(Decision::ConfirmPrepared {
                kind: k.kind,
                release: k.release,
            });
        }
        if let Some(g) = self.grant_delegation {
            out.push(Decision::GrantDelegation(
                if g.delegation.eq_ignore_ascii_case("proposed") {
                    DelegationSel::Proposed
                } else {
                    DelegationSel::Id(g.delegation)
                },
            ));
        }
        if let Some(m) = self.set_mode {
            out.push(Decision::SetMode(m));
        }
        if let Some(m) = self.record_manual {
            out.push(Decision::RecordManual {
                item: m.item.into_sel(),
                requirement: m.requirement,
                test: m.test,
                passed: m.passed,
            });
        }
        match out.len() {
            1 => Ok(out.remove(0)),
            0 => Err(anyhow!("step {step}: `decide` names no decision")),
            _ => Err(anyhow!(
                "step {step}: `decide` names more than one decision"
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOffer {
    page: String,
    button: String,
    #[serde(default)]
    subject: Option<RawItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelease {
    version: String,
    stage: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItemClaim {
    #[serde(default)]
    title_contains: Option<String>,
    #[serde(default)]
    title_matches: Option<String>,
    #[serde(default)]
    item: Option<RawItemSel>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfigClaim {
    release_prepare: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawBoardClaim {
    project: Option<String>,
    release: Option<RawRelease>,
    item: Option<RawItemClaim>,
    config: Option<RawConfigClaim>,
    sprint: Option<String>,
}

impl RawBoardClaim {
    fn into_claim(self, step: &str) -> Result<BoardClaim> {
        let mut out = Vec::new();
        if let Some(p) = self.project {
            if p != "exists" {
                return Err(anyhow!(
                    "step {step}: a project claim is `exists`, not '{p}'"
                ));
            }
            out.push(BoardClaim::ProjectExists);
        }
        if let Some(r) = self.release {
            out.push(BoardClaim::Release {
                version: r.version,
                stage: r.stage,
            });
        }
        if let Some(i) = self.item {
            match (i.title_contains, i.title_matches, i.item, i.status) {
                (Some(t), None, None, None) => out.push(BoardClaim::ItemTitleContains(t)),
                (None, Some(p), None, None) => out.push(BoardClaim::ItemTitleMatches(p)),
                (None, None, Some(sel), Some(status)) => out.push(BoardClaim::ItemStatus {
                    item: sel.into_sel(),
                    status,
                }),
                _ => {
                    return Err(anyhow!(
                        "step {step}: an item claim is `{{title_contains: …}}`, `{{title_matches: …}}` or `{{item: …, status: …}}`"
                    ));
                }
            }
        }
        if let Some(c) = self.config {
            if c.release_prepare != "configured" {
                return Err(anyhow!(
                    "step {step}: `config.release_prepare` is `configured`, not '{}'",
                    c.release_prepare
                ));
            }
            out.push(BoardClaim::ReleasePrepareConfigured);
        }
        if let Some(s) = self.sprint {
            if s != "active" {
                return Err(anyhow!(
                    "step {step}: a sprint claim is `active`, not '{s}'"
                ));
            }
            out.push(BoardClaim::SprintActive);
        }
        match out.len() {
            1 => Ok(out.remove(0)),
            0 => Err(anyhow!("step {step}: `expect.board` claims nothing")),
            _ => Err(anyhow!(
                "step {step}: `expect.board` makes more than one claim; use one step each"
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCi {
    release: String,
    passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTag {
    release: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawTruthClaim {
    tag: Option<RawTag>,
    ci: Option<RawCi>,
    merged: Option<RawItem>,
    doctor: Option<String>,
}

impl RawTruthClaim {
    fn into_claim(self, step: &str) -> Result<TruthClaim> {
        let mut out = Vec::new();
        if let Some(t) = self.tag {
            out.push(TruthClaim::Tag { release: t.release });
        }
        if let Some(c) = self.ci {
            out.push(TruthClaim::Ci {
                release: c.release,
                passed: c.passed,
            });
        }
        if let Some(m) = self.merged {
            out.push(TruthClaim::Merged {
                item: m.item.into_sel(),
            });
        }
        if let Some(d) = self.doctor {
            if d != "clean" {
                return Err(anyhow!("step {step}: a doctor claim is `clean`, not '{d}'"));
            }
            out.push(TruthClaim::DoctorClean);
        }
        match out.len() {
            1 => Ok(out.remove(0)),
            0 => Err(anyhow!("step {step}: `expect.truth` claims nothing")),
            _ => Err(anyhow!(
                "step {step}: `expect.truth` makes more than one claim; use one step each"
            )),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawTalk {
    asked: Option<String>,
    not_asked: Option<String>,
    said: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawExpect {
    board: Option<RawBoardClaim>,
    truth: Option<RawTruthClaim>,
    page: Option<PageClaim>,
    talk: Option<RawTalk>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPress {
    page: String,
    button: String,
    #[serde(default)]
    subject: Option<RawItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    #[serde(default)]
    id: Option<String>,
    say: Option<String>,
    #[serde(default)]
    answers: Vec<AnswerRule>,
    decide: Option<RawDecision>,
    press: Option<RawPress>,
    offer: Option<RawOffer>,
    expect: Option<RawExpect>,
    push: Option<Push>,
    check: Option<String>,
    #[serde(default)]
    stop_on_mismatch: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRunbook {
    runbook: String,
    process: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    budget_usd: Option<f64>,
    #[serde(default)]
    max_minutes: Option<u64>,
    #[serde(default)]
    checks: BTreeMap<String, CheckRef>,
    #[serde(default)]
    answers: Vec<AnswerRule>,
    #[serde(default)]
    steps: Vec<RawStep>,
}

/// The first fenced `yaml` block of a markdown doc, without its fences.
pub fn yaml_block(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines();
    while let Some(l) = lines.next() {
        let t = l.trim();
        let Some(info) = t.strip_prefix("```") else {
            continue;
        };
        let info = info.trim().to_ascii_lowercase();
        if info != "yaml" && info != "yml" {
            // Some other fence: skip to its end.
            for inner in lines.by_ref() {
                if inner.trim().starts_with("```") {
                    break;
                }
            }
            continue;
        }
        let mut block = String::new();
        for inner in lines.by_ref() {
            if inner.trim().starts_with("```") {
                return Some(block);
            }
            block.push_str(inner);
            block.push('\n');
        }
        return Some(block);
    }
    None
}

impl Runbook {
    /// Read a runbook from a board doc: its first fenced `yaml` block.
    pub fn parse(markdown: &str) -> Result<Runbook> {
        let block = yaml_block(markdown)
            .ok_or_else(|| anyhow!("the runbook has no fenced ```yaml block"))?;
        let raw: RawRunbook = kanbanr_core::yaml::from_str(&block)
            .map_err(|e| anyhow!("the runbook's yaml does not read: {e}"))?;
        let mut steps = Vec::with_capacity(raw.steps.len());
        for (i, s) in raw.steps.into_iter().enumerate() {
            let id = s.id.unwrap_or_else(|| format!("#{}", i + 1));
            let decide = s.decide.map(|d| d.into_decision(&id)).transpose()?;
            let offer = s.offer.map(|o| Offer {
                page: o.page,
                button: o.button,
                subject: o.subject.map(|s| s.item.into_sel()),
            });
            let press = s.press.map(|p| Press {
                page: p.page,
                button: p.button,
                subject: p.subject.map(|s| s.item.into_sel()),
            });
            let expect = match s.expect {
                None => None,
                Some(e) => Some(Expect {
                    board: e.board.map(|b| b.into_claim(&id)).transpose()?,
                    truth: e.truth.map(|t| t.into_claim(&id)).transpose()?,
                    page: e.page,
                    talk: e.talk.map(|t| TalkClaim {
                        asked: t.asked,
                        not_asked: t.not_asked,
                        said: t.said,
                    }),
                }),
            };
            steps.push(Step {
                id,
                say: s.say,
                answers: s.answers,
                decide,
                press,
                offer,
                expect,
                push: s.push,
                check: s.check,
                stop_on_mismatch: s.stop_on_mismatch,
            });
        }
        Ok(Runbook {
            name: raw.runbook,
            process: raw.process,
            model: raw.model.unwrap_or_else(|| "sonnet".into()),
            budget_usd: raw.budget_usd.unwrap_or(40.0),
            max_minutes: raw.max_minutes.unwrap_or(180),
            checks: raw.checks,
            answers: raw.answers,
            steps,
        })
    }

    /// The step with this id.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Every check marker a step names, in step order.
    pub fn check_markers(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter_map(|s| s.check.as_deref())
            .collect()
    }
}

/// One thing a run would stumble on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintError {
    pub step: String,
    pub message: String,
}

impl std::fmt::Display for LintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.step.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "step {}: {}", self.step, self.message)
        }
    }
}

/// Every `title_matches` pattern a step names: in its decision, press, offer and board claim.
fn title_patterns(s: &Step) -> Vec<String> {
    let mut sels: Vec<&ItemSel> = Vec::new();
    if let Some(d) = &s.decide {
        match d {
            Decision::Approve(i) | Decision::Ratify(i) => sels.push(i),
            Decision::Signoff { item, .. } | Decision::RecordManual { item, .. } => sels.push(item),
            _ => {}
        }
    }
    sels.extend(s.press.as_ref().and_then(|p| p.subject.as_ref()));
    sels.extend(s.offer.as_ref().and_then(|o| o.subject.as_ref()));
    let mut out: Vec<String> = sels
        .into_iter()
        .filter_map(|i| match i {
            ItemSel::TitleMatches(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if let Some(e) = &s.expect {
        match &e.board {
            Some(BoardClaim::ItemTitleMatches(p)) => out.push(p.clone()),
            Some(BoardClaim::ItemStatus {
                item: ItemSel::TitleMatches(p),
                ..
            }) => out.push(p.clone()),
            _ => {}
        }
        if let Some(TruthClaim::Merged {
            item: ItemSel::TitleMatches(p),
        }) = &e.truth
        {
            out.push(p.clone());
        }
    }
    out
}

/// Everything a run would stumble on; empty for a runbook a run can follow.
pub fn lint(rb: &Runbook) -> Vec<LintError> {
    let mut out = Vec::new();
    let err = |step: &str, message: String| LintError {
        step: step.to_string(),
        message,
    };
    if rb.name.trim().is_empty() {
        out.push(err("", "the runbook has no name (`runbook:`)".into()));
    }
    if rb.process.trim().is_empty() {
        out.push(err("", "the runbook names no process".into()));
    }
    if rb.steps.is_empty() {
        out.push(err("", "the runbook has no steps".into()));
    }
    if rb.budget_usd <= 0.0 {
        out.push(err("", "budget_usd must be above zero".into()));
    }
    for (marker, c) in &rb.checks {
        if c.item.trim().is_empty() || c.requirement.trim().is_empty() || c.test.trim().is_empty() {
            out.push(err(
                "",
                format!("check {marker} needs an item, a requirement and a test"),
            ));
        }
    }
    for rule in &rb.answers {
        if let Err(e) = rule.regex() {
            out.push(err(
                "",
                format!("answer rule `{}` does not compile: {e}", rule.pattern),
            ));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &rb.steps {
        if s.id.trim().is_empty() || s.id.starts_with('#') {
            out.push(err(&s.id, "the step has no id".into()));
        }
        if !seen.insert(s.id.clone()) {
            out.push(err(&s.id, "the step id is used twice".into()));
        }
        let does_something = s.say.is_some()
            || s.decide.is_some()
            || s.press.is_some()
            || s.push.is_some()
            || s.expect.as_ref().is_some_and(|e| !e.is_empty());
        if !does_something {
            out.push(err(
                &s.id,
                "the step has none of say, decide, press, push or expect".into(),
            ));
        }
        if s.decide.is_some() && s.press.is_some() {
            out.push(err(
                &s.id,
                "a step is `decide` or `press`, never both".into(),
            ));
        }
        if s.decide.is_some() && s.offer.is_none() {
            out.push(err(
                &s.id,
                "a `decide` step names the `offer` the guide promises, to check it first".into(),
            ));
        }
        if let Some(o) = &s.offer
            && !matches!(o.page.as_str(), "review" | "releases")
        {
            out.push(err(
                &s.id,
                format!("an offer's page is review or releases, not '{}'", o.page),
            ));
        }
        if let Some(c) = &s.check
            && !rb.checks.contains_key(c)
        {
            out.push(err(
                &s.id,
                format!("check {c} is not defined under `checks`"),
            ));
        }
        for rule in &s.answers {
            if let Err(e) = rule.regex() {
                out.push(err(
                    &s.id,
                    format!("answer rule `{}` does not compile: {e}", rule.pattern),
                ));
            }
        }
        for p in title_patterns(s) {
            if let Err(why) = regex::RegexBuilder::new(&p).case_insensitive(true).build() {
                out.push(err(
                    &s.id,
                    format!("title_matches `{p}` does not compile: {why}"),
                ));
            }
        }
        if let Some(e) = &s.expect
            && let Some(t) = &e.talk
        {
            for (what, pat) in [
                ("asked", &t.asked),
                ("not_asked", &t.not_asked),
                ("said", &t.said),
            ] {
                if let Some(p) = pat
                    && let Err(why) = regex::RegexBuilder::new(p).case_insensitive(true).build()
                {
                    out.push(err(
                        &s.id,
                        format!("talk.{what} `{p}` does not compile: {why}"),
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small runbook in the Design's shape, with one check and five kinds of step.
    pub(crate) const SMALL: &str = r#"# A runbook

Prose before the block is for the person.

```yaml
runbook: small
process: togaf
model: sonnet
budget_usd: 40
max_minutes: 180
checks:
  MT-1: {item: FTR-034, requirement: R-5, test: "manual: in a walkthrough, every sign-off, promotion and CI result is taken with one click and nothing typed"}
  MT-2: {item: FTR-028, requirement: R-5, test: "manual: a walkthrough's setup proposes delegations for its process and the person accepts them"}
answers:
  - {match: "sprint|release", pick: "Two weeks from today, starting today."}
steps:
  - id: "1.1"
    say: "Start using kanbanr for this project. Use TOGAF, with sprints and releases."
    answers:
      - {match: "where .* board", pick: "../board, with the board remote from remotes.env"}
      - {match: "what .*project.* for", pick: "A small web shop."}
    expect: {board: {project: exists}}
  - id: "1.2"
    decide: {grant_delegation: {delegation: proposed}}
    offer: {page: review, button: Grant}
    check: MT-2
  - id: "4.2"
    decide: {approve: {item: newest}}
    offer: {page: review, button: Approve, subject: {item: newest}}
  - id: "5.8"
    push: {repo: code, refspec: main}
  - id: "5.9"
    say: "CI on main passed."
    decide: {confirm_prepared: {kind: gate-result}}
    offer: {page: releases, button: "Record ci passed"}
    expect: {board: {release: {version: v0.1.0, stage: main}}}
  - id: "5.10"
    decide: {promote: {release: v0.1.0, to: tag}}
    offer: {page: releases, button: "Cut v0.1.0"}
    expect: {truth: {ci: {release: v0.1.0, passed: true}}}
    check: MT-1
  - id: "6.1"
    press: {page: review, button: "Sign off release", subject: {item: {title_contains: basket}}}
    expect: {page: {url: "/p/shop/review", contains: ["Sign off release"], absent: ["Approve"]}}
```
"#;

    /// FTR-037 W2: the Design's example shape parses into the typed steps, and a runbook a run can
    /// follow lints clean.
    #[test]
    fn a_runbook_parses_and_lints_clean() {
        let rb = Runbook::parse(SMALL).unwrap();
        assert_eq!(rb.name, "small");
        assert_eq!(rb.process, "togaf");
        assert_eq!(rb.model, "sonnet");
        assert_eq!(rb.budget_usd, 40.0);
        assert_eq!(rb.max_minutes, 180);
        assert_eq!(rb.checks.len(), 2);
        assert_eq!(rb.checks["MT-1"].item, "FTR-034");
        assert_eq!(rb.steps.len(), 7);
        let first = &rb.steps[0];
        assert_eq!(first.id, "1.1");
        assert_eq!(first.answers.len(), 2);
        assert!(first.answers[0].matches("Where should the board live?"));
        assert!(!first.answers[0].matches("What is the project for?"));
        assert_eq!(
            first.expect.as_ref().unwrap().board,
            Some(BoardClaim::ProjectExists)
        );
        assert_eq!(
            rb.steps[1].decide,
            Some(Decision::GrantDelegation(DelegationSel::Proposed))
        );
        assert_eq!(rb.steps[2].decide, Some(Decision::Approve(ItemSel::Newest)));
        assert_eq!(
            rb.steps[2].offer.as_ref().unwrap().subject,
            Some(ItemSel::Newest)
        );
        assert_eq!(
            rb.steps[3].push,
            Some(Push {
                repo: CodeOrBoard::Code,
                refspec: "main".into()
            })
        );
        assert_eq!(
            rb.steps[4].decide,
            Some(Decision::ConfirmPrepared {
                kind: "gate-result".into(),
                release: None
            })
        );
        assert_eq!(
            rb.steps[4].expect.as_ref().unwrap().board,
            Some(BoardClaim::Release {
                version: "v0.1.0".into(),
                stage: "main".into()
            })
        );
        assert_eq!(
            rb.steps[5].expect.as_ref().unwrap().truth,
            Some(TruthClaim::Ci {
                release: "v0.1.0".into(),
                passed: true
            })
        );
        assert_eq!(rb.steps[5].check.as_deref(), Some("MT-1"));
        let press = rb.steps[6].press.as_ref().unwrap();
        assert_eq!(press.button, "Sign off release");
        assert_eq!(press.subject, Some(ItemSel::TitleContains("basket".into())));
        let page = rb.steps[6].expect.as_ref().unwrap().page.as_ref().unwrap();
        assert_eq!(page.contains, ["Sign off release"]);
        assert_eq!(page.absent, ["Approve"]);
        assert_eq!(rb.check_markers(), ["MT-2", "MT-1"]);
        assert_eq!(lint(&rb), Vec::<LintError>::new());
        // No block, no runbook.
        assert!(
            Runbook::parse("# nothing here\n\n```sh\necho hi\n```\n")
                .unwrap_err()
                .to_string()
                .contains("no fenced ```yaml block")
        );
    }

    /// FTR-037 W2: lint names a check a step uses without a definition, a decision with no offer,
    /// a step with no id or nothing to do, an answer rule that does not compile, and a step that is
    /// both decide and press.
    #[test]
    fn lint_names_a_check_without_a_definition_and_a_decide_without_an_offer() {
        let text = r#"```yaml
runbook: faulty
process: agile
steps:
  - id: "1"
    decide: {approve: {item: newest}}
    check: MT-9
  - id: "2"
    say: "hello"
    answers:
      - {match: "(unclosed", pick: "x"}
  - say: "no id"
  - id: "4"
    check: MT-9
  - id: "4"
    decide: {approve: {item: newest}}
    press: {page: review, button: Approve}
    offer: {page: board, button: Approve}
```"#;
        let rb = Runbook::parse(text).unwrap();
        let errors = lint(&rb);
        let text_of = |e: &LintError| e.to_string();
        let all: Vec<String> = errors.iter().map(text_of).collect();
        let has = |needle: &str| all.iter().any(|e| e.contains(needle));
        assert!(
            has("step 1: check MT-9 is not defined under `checks`"),
            "{all:?}"
        );
        assert!(has("step 1: a `decide` step names the `offer`"), "{all:?}");
        assert!(has("does not compile"), "{all:?}");
        assert!(has("the step has no id"), "{all:?}");
        assert!(has("none of say, decide, press, push or expect"), "{all:?}");
        assert!(has("the step id is used twice"), "{all:?}");
        assert!(has("never both"), "{all:?}");
        assert!(has("page is review or releases, not 'board'"), "{all:?}");
        // A decision naming two things, or none, does not even parse.
        let two = "```yaml\nrunbook: x\nprocess: y\nsteps:\n  - id: a\n    decide: {approve: {item: newest}, ratify: {item: newest}}\n    offer: {page: review, button: Approve}\n```";
        assert!(
            Runbook::parse(two)
                .unwrap_err()
                .to_string()
                .contains("more than one decision")
        );
    }

    /// The two guide runbooks that ship as fixtures (the board copies are made from them): each
    /// parses, lints clean, and places every check it defines — SPK-004's MT-1 to MT-5 and DEF-056's
    /// MT-9 for TOGAF, MT-1 to MT-8 for agile; a marker on several steps is one check — and names
    /// no person, account or server.
    #[test]
    fn the_guide_runbooks_parse_and_lint_clean_with_their_eight_checks() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("walkthroughs");
        for (file, process, markers) in
            [("togaf.md", "togaf", 6usize), ("agile.md", "agile", 8usize)]
        {
            let text = std::fs::read_to_string(fixtures.join(file)).unwrap();
            let rb = Runbook::parse(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(rb.process, process);
            assert_eq!(rb.model, "sonnet");
            assert_eq!(rb.checks.len(), markers, "{file}");
            let mut used = rb.check_markers();
            used.sort();
            used.dedup();
            let mut defined: Vec<&str> = rb.checks.keys().map(String::as_str).collect();
            defined.sort();
            assert_eq!(used, defined, "{file}: every check is placed");
            assert_eq!(lint(&rb), Vec::<LintError>::new(), "{file}");
            for word in ["/home/", "github.com", "@", "http://", "https://"] {
                assert!(
                    !text.contains(word),
                    "{file} names something private: {word}"
                );
            }
        }
    }

    /// FTR-037 W2b: the board claims DEF-056's runbook needs before its promote step — release
    /// preparation configured, and an item whose title contains a text — read from the block, as
    /// do the item-status and sprint claims.
    #[test]
    fn board_claims_cover_release_prepare_and_item_titles() {
        let text = r#"```yaml
runbook: claims
process: agile
steps:
  - id: "1"
    expect: {board: {config: {release_prepare: configured}}}
  - id: "2"
    expect: {board: {item: {title_contains: "pipeline"}}}
  - id: "3"
    expect: {board: {item: {item: FTR-001, status: Migration}}}
  - id: "4"
    expect: {board: {sprint: active}}
  - id: "5"
    expect: {truth: {tag: {release: v0.1.0}}}
  - id: "6"
    expect: {talk: {asked: "style|look", not_asked: "push order", said: "stage.*cach"}}
```"#;
        let rb = Runbook::parse(text).unwrap();
        let board = |i: usize| rb.steps[i].expect.as_ref().unwrap().board.clone().unwrap();
        assert_eq!(board(0), BoardClaim::ReleasePrepareConfigured);
        assert_eq!(board(1), BoardClaim::ItemTitleContains("pipeline".into()));
        assert_eq!(
            board(2),
            BoardClaim::ItemStatus {
                item: ItemSel::Code("FTR-001".into()),
                status: "Migration".into()
            }
        );
        assert_eq!(board(3), BoardClaim::SprintActive);
        assert_eq!(
            rb.steps[4].expect.as_ref().unwrap().truth,
            Some(TruthClaim::Tag {
                release: "v0.1.0".into()
            })
        );
        let talk = rb.steps[5].expect.as_ref().unwrap().talk.clone().unwrap();
        assert_eq!(talk.asked.as_deref(), Some("style|look"));
        assert_eq!(talk.not_asked.as_deref(), Some("push order"));
        assert_eq!(lint(&rb), Vec::<LintError>::new());
        // T16: a title pattern, in a board claim and in a selector; lint names a bad one.
        let re = Runbook::parse(
            "```yaml\nrunbook: x\nprocess: y\nsteps:\n  - id: a\n    expect: {board: {item: {title_matches: \"basket|cart\"}}}\n  - id: b\n    decide: {approve: {item: {title_matches: \"^saved\"}}}\n    offer: {page: review, button: Approve, subject: {item: {title_matches: \"^saved\"}}}\n  - id: c\n    expect: {board: {item: {title_matches: \"(unclosed\"}}}\n```",
        )
        .unwrap();
        assert_eq!(
            re.steps[0].expect.as_ref().unwrap().board,
            Some(BoardClaim::ItemTitleMatches("basket|cart".into()))
        );
        assert_eq!(
            re.steps[1].decide,
            Some(Decision::Approve(ItemSel::TitleMatches("^saved".into())))
        );
        assert!(ItemSel::TitleMatches("^saved".into()).title_fits("Saved basket"));
        assert!(!ItemSel::TitleMatches("^saved".into()).title_fits("A saved basket"));
        let errors = lint(&re);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .to_string()
                .starts_with("step c: title_matches `(unclosed` does not compile"),
            "{}",
            errors[0]
        );
        // A wrong word is refused at parse time, with the step named.
        let bad = "```yaml\nrunbook: x\nprocess: y\nsteps:\n  - id: a\n    expect: {board: {config: {release_prepare: maybe}}}\n```";
        let why = Runbook::parse(bad).unwrap_err().to_string();
        assert!(
            why.contains("step a") && why.contains("configured"),
            "{why}"
        );
    }
}
