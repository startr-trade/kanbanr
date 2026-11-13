//! Release stages that act (FEAT-212): each stage a release moves through may declare conditions
//! (`requires`) and entry actions (`on_enter`), as an item's status declares a gate.
//!
//! The rules sit beside the stage names, in `cadence.release_gates`, keyed by stage name, so
//! `cadence.release_stages` stays a list of names: an older binary keeps the new keys in
//! `Cadence.extra` (FEAT-151) rather than failing to read the board. A stage that declares no gate
//! acts as every stage did before: the first makes the release branch, the last fast-forwards main
//! and tags the release (see [`Cadence::effective_release_gates`]).
//!
//! `cadence.release_prepare` says what the `prepare` action writes onto `release/<V>`: the version
//! files, the dated changelog heading and its copies, a notes draft and where it is listed.

use crate::config::{Cadence, RECOMMENDED_RELEASE_STAGES};
use crate::models::Extra;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What entering a release stage does. The order of the variants is the order a promotion applies
/// them in, whichever stages hold them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseAction {
    /// Make `release/<V>` from the line of integration, without checking it out.
    Branch,
    /// Commit the version files, the changelog heading, the notes draft and its listings onto
    /// `release/<V>`, without checking anything out.
    Prepare,
    /// Fast-forward main to the release tip, refused if main has commits the release lacks.
    MoveMain,
    /// Tag the release tip with the version (an annotated tag, `Release <V>`).
    Tag,
}

impl ReleaseAction {
    pub const ALL: [ReleaseAction; 4] = [
        ReleaseAction::Branch,
        ReleaseAction::Prepare,
        ReleaseAction::MoveMain,
        ReleaseAction::Tag,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReleaseAction::Branch => "branch",
            ReleaseAction::Prepare => "prepare",
            ReleaseAction::MoveMain => "move_main",
            ReleaseAction::Tag => "tag",
        }
    }

    /// What the action does, in a sentence.
    pub fn does(self) -> &'static str {
        match self {
            ReleaseAction::Branch => "makes release/<V> from the line of integration",
            ReleaseAction::Prepare => {
                "commits the version files, the dated changelog heading and draft notes to \
                 release/<V>"
            }
            ReleaseAction::MoveMain => "fast-forwards main to the release tip",
            ReleaseAction::Tag => "tags the release tip with the version",
        }
    }
}

/// Read by hand so a misspelt action is named with the ones there are.
impl<'de> Deserialize<'de> for ReleaseAction {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        ReleaseAction::ALL
            .into_iter()
            .find(|a| a.name() == name.trim())
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "'{name}' is not a release stage action — the actions are: {}",
                    ReleaseAction::ALL.map(ReleaseAction::name).join(", ")
                ))
            })
    }
}

/// A condition a release stage can require before a release enters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseCheck {
    /// Nothing planned into the release would be carried over by the cut.
    ItemsFinished,
    /// The release tip carries the version, the dated changelog heading and the notes.
    Prepared,
    /// Main is at the release tip.
    MainAtRelease,
    /// The release tip is on main's remote-tracking branch.
    Pushed,
    /// Every gate in `cadence.commit_gates` has a passed result recorded for the release tip.
    GatePassed,
    /// The newest result of the tag gate for the tag commit, recorded since the tag, is passed
    /// (FTR-026). Only a stage after the tag can require it.
    TagPassed,
    /// Every item planned into the release is at the integrate stage or later.
    ItemsMerged,
    /// The release has been promoted into a stage a gate names in `after`. Only a plan names it
    /// (a gate lists `after`, not this), so it is not in `ALL`.
    StagePassed,
    /// Every item planned into the release whose code is merged can ship: the cut would not carry
    /// merged work away (DEF-029). Only a plan names it, so it is not in `ALL`.
    MergedItemsShip,
    /// Every test of the release's items checked at this stage or before it is green (DEF-043).
    /// Only a plan names it, so it is not in `ALL`.
    LaterTestsGreen,
    /// The person accepted shipping exactly the requirements whose tests were not run (DEF-043).
    /// Only a plan names it, at the cut, so it is not in `ALL`.
    UntestedAccepted,
}

impl ReleaseCheck {
    pub const ALL: [ReleaseCheck; 7] = [
        ReleaseCheck::ItemsFinished,
        ReleaseCheck::Prepared,
        ReleaseCheck::MainAtRelease,
        ReleaseCheck::Pushed,
        ReleaseCheck::GatePassed,
        ReleaseCheck::TagPassed,
        ReleaseCheck::ItemsMerged,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReleaseCheck::ItemsFinished => "items_finished",
            ReleaseCheck::Prepared => "prepared",
            ReleaseCheck::MainAtRelease => "main_at_release",
            ReleaseCheck::Pushed => "pushed",
            ReleaseCheck::GatePassed => "gate_passed",
            ReleaseCheck::TagPassed => "tag_passed",
            ReleaseCheck::ItemsMerged => "items_merged",
            ReleaseCheck::StagePassed => "stage_passed",
            ReleaseCheck::MergedItemsShip => "merged_items_ship",
            ReleaseCheck::LaterTestsGreen => "later_tests_green",
            ReleaseCheck::UntestedAccepted => "untested_accepted",
        }
    }

    /// When the condition holds, in a sentence.
    pub fn passes_when(self) -> &'static str {
        match self {
            ReleaseCheck::ItemsFinished => "the cut would carry nothing over",
            ReleaseCheck::Prepared => {
                "at the release tip every version file names the version, the changelog has its \
                 heading and the notes file exists (always, when nothing is configured to prepare)"
            }
            ReleaseCheck::MainAtRelease => {
                "main is at the release tip (or, for a patch, main has moved on)"
            }
            ReleaseCheck::Pushed => {
                "the release tip is on main's remote-tracking branch (for a patch left behind, on \
                 any remote-tracking branch)"
            }
            ReleaseCheck::GatePassed => {
                "every gate in commit_gates has a passed result for the release tip"
            }
            ReleaseCheck::TagPassed => {
                "your CI's newest result for the tag commit, recorded since the tag, is passed"
            }
            ReleaseCheck::ItemsMerged => {
                "every item planned into the release is at the integrate stage or later"
            }
            ReleaseCheck::StagePassed => "the release has been promoted into the stage",
            ReleaseCheck::MergedItemsShip => {
                "every item planned into the release whose code is merged can ship"
            }
            ReleaseCheck::LaterTestsGreen => {
                "every test of the release's items checked at this stage or before it is green"
            }
            ReleaseCheck::UntestedAccepted => {
                "every shipping requirement has a green test, or the person accepted shipping \
                 those without one"
            }
        }
    }
}

/// Read by hand so a misspelt check is named with the ones there are (as `Condition`, FEAT-168).
impl<'de> Deserialize<'de> for ReleaseCheck {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        ReleaseCheck::ALL
            .into_iter()
            .find(|c| c.name() == name.trim())
            .ok_or_else(|| {
                serde::de::Error::custom(if name.trim() == "stage_passed" {
                    "'stage_passed' is not a check a stage can require: use `after` to require an \
                     earlier stage"
                        .to_string()
                } else {
                    format!(
                        "'{name}' is not a check a release stage can require — the checks are: {}",
                        ReleaseCheck::ALL.map(ReleaseCheck::name).join(", ")
                    )
                })
            })
    }
}

/// One release stage's rules: what it is for, what must hold to enter it, and what entering it
/// does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseGate {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<ReleaseCheck>,
    /// Earlier stages the release must have been promoted into before it enters this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_enter: Vec<ReleaseAction>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A file whose version `prepare` sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VersionFile {
    /// Relative to the repository root.
    pub path: String,
    /// A template with exactly one `{version}`. Absent: `^version = "{version}"` for `*.toml`,
    /// `"version": "{version}"` for `*.json`, first match only. Given: every match.
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// Where the notes draft is listed: `entry` goes directly below the first line equal to `after`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NotesListing {
    pub path: String,
    pub after: String,
    pub entry: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What the `prepare` action writes onto `release/<V>`. Empty by default: `prepare` is then a
/// no-op the plan names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleasePrepare {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_files: Vec<VersionFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changelog_copies: Vec<String>,
    /// A path template naming `{version}` or `{number}`, e.g. `docs/src/releases/{version}.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes_listings: Vec<NotesListing>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl ReleasePrepare {
    /// Nothing configured, nothing kept: what an absent key means, and why it stays absent.
    pub fn is_empty(&self) -> bool {
        *self == ReleasePrepare::default()
    }

    /// Nothing configured that `prepare` would write.
    pub fn writes_nothing(&self) -> bool {
        self.version_files.is_empty()
            && self.changelog.is_none()
            && self.notes.is_none()
            && self.notes_listings.is_empty()
    }
}

/// A release stage with the gate in force, as `release stages` and the monitor show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseStageView {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    pub requires: Vec<ReleaseCheck>,
    /// Earlier stages the release must have entered first.
    pub after: Vec<String>,
    pub on_enter: Vec<ReleaseAction>,
    /// The stage holding the tag action: entering it is the cut. With no stage after it, the last.
    pub cut: bool,
    /// A stage after the cut: a confirmation by the user, which runs no action (FTR-026).
    pub after_cut: bool,
    /// The project declares this stage's gate; otherwise it is the default.
    pub declared: bool,
    /// The stage asks nothing and does nothing: promoting to it only records that the release
    /// passed it (FTR-020).
    #[serde(default)]
    pub inert: bool,
}

impl ReleaseGate {
    /// Nothing is required before entering and nothing happens on entering (FTR-020).
    pub fn is_inert(&self) -> bool {
        self.requires.is_empty() && self.after.is_empty() && self.on_enter.is_empty()
    }
}

impl Cadence {
    /// The release stages with their gates in force, in order.
    pub fn release_stage_views(&self) -> Vec<ReleaseStageView> {
        let gates = self.effective_release_gates();
        let cut = self.cut_index();
        gates
            .into_iter()
            .enumerate()
            .map(|(i, (name, g))| ReleaseStageView {
                declared: self.declares_release_gate(&name),
                inert: i < cut && g.is_inert(),
                name,
                purpose: g.purpose,
                requires: g.requires,
                after: g.after,
                on_enter: g.on_enter,
                cut: i == cut,
                after_cut: i > cut,
            })
            .collect()
    }

    /// The stages, gates and commit gates a project gets when it turns releases on and names none
    /// (FTR-020): candidate branches and prepares once the items are merged (FTR-021), main moves
    /// main once the items are finished, and tag, the cut, needs main at the release and pushed. With `has_ci` it also needs a
    /// passed `ci` result for that commit, and `ci` is the commit gate.
    pub fn recommended(has_ci: bool) -> (Vec<String>, BTreeMap<String, ReleaseGate>, Vec<String>) {
        let gate =
            |purpose: &str, requires: &[ReleaseCheck], on_enter: &[ReleaseAction]| ReleaseGate {
                purpose: purpose.to_string(),
                requires: requires.to_vec(),
                on_enter: on_enter.to_vec(),
                ..ReleaseGate::default()
            };
        let mut tag_requires = vec![ReleaseCheck::MainAtRelease, ReleaseCheck::Pushed];
        if has_ci {
            tag_requires.push(ReleaseCheck::GatePassed);
        }
        let stages = RECOMMENDED_RELEASE_STAGES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let gates = BTreeMap::from([
            (
                "candidate".to_string(),
                gate(
                    "Make the release branch and prepare it once its items are merged",
                    &[ReleaseCheck::ItemsMerged],
                    &[ReleaseAction::Branch, ReleaseAction::Prepare],
                ),
            ),
            (
                "main".to_string(),
                gate(
                    "Move main to the release once its items are finished",
                    &[ReleaseCheck::ItemsFinished],
                    &[ReleaseAction::MoveMain],
                ),
            ),
            (
                "tag".to_string(),
                gate(
                    "Cut the release: tag it once main is at the release and pushed",
                    &tag_requires,
                    &[ReleaseAction::Tag],
                ),
            ),
        ]);
        let commit_gates = if has_ci {
            vec!["ci".to_string()]
        } else {
            vec![]
        };
        (stages, gates, commit_gates)
    }

    /// What to tell a project that has just said it has CI but whose stages do not wait for it:
    /// they were chosen by the project, so no gate was changed (FTR-020).
    pub fn has_ci_note(&self, previous: &Cadence) -> Option<String> {
        let waits = self
            .effective_release_gates()
            .iter()
            .any(|(_, g)| g.requires.contains(&ReleaseCheck::GatePassed));
        (self.has_ci && !previous.has_ci && self.releases && !waits).then(|| {
            "no release stage waits for CI. To require it at the tag: kanbanr config release-stage \
             tag --requires main_at_release,pushed,gate_passed and kanbanr config commit-gates ci"
                .to_string()
        })
    }

    /// This cadence without what kanbanr wrote itself when releases were turned on or CI was
    /// declared: the recommended stages and gates, the `ci` commit gate and the flag (FTR-020).
    /// What a process is compared with, since a process names none of them.
    pub fn without_recommended_defaults(&self) -> Cadence {
        let mut c = self.clone();
        if c.is_recommended() {
            c.release_stages.clear();
            c.release_gates.clear();
            if c.has_ci && c.commit_gates == ["ci"] {
                c.commit_gates.clear();
            }
            c.has_ci = false;
        }
        c
    }

    /// Whether the stage names and gates are exactly the recommended ones for this project's
    /// `has_ci`, so nothing in them was chosen by the project.
    pub fn is_recommended(&self) -> bool {
        let (stages, gates, _) = Cadence::recommended(self.has_ci);
        self.release_stages == stages && self.release_gates == gates
    }

    /// The stages before the cut that ask nothing and do nothing, in order. The cut always acts,
    /// and a stage after it is a confirmation the user gives (FTR-026), so neither is listed.
    pub fn inert_stages(&self) -> Vec<String> {
        let cut = self.cut_index();
        self.effective_release_gates()
            .into_iter()
            .take(cut)
            .filter(|(_, g)| g.is_inert())
            .map(|(name, _)| name)
            .collect()
    }

    /// What to say of an inert stage, worded once for the CLI, the API and the monitor.
    pub fn inert_warning(stage: &str) -> String {
        format!(
            "release stage '{stage}' has no condition and no action, so promoting to it only \
             records that the release passed it. Give it one with kanbanr config release-stage \
             {stage} --requires <check> --on-enter <action>, or leave it out of \
             cadence.release_stages"
        )
    }

    /// The index of the stage holding the `tag` action, the cut (FTR-026); the last stage when
    /// none holds it. Stages after it are confirmations by the user.
    pub fn cut_index(&self) -> usize {
        let gates = self.effective_release_gates();
        gates
            .iter()
            .position(|(_, g)| g.on_enter.contains(&ReleaseAction::Tag))
            .unwrap_or(gates.len().saturating_sub(1))
    }

    /// The release stage a test checked at `at` waits for (DEF-043): "" → None; "cut" → the
    /// stage holding the tag (`cut_index`); "pushed" → the first stage whose gate requires
    /// `pushed`, else the cut's; a stage of this project → that stage; anything else → the
    /// cut's stage.
    pub fn stage_for_check(&self, at: &str) -> Option<String> {
        let at = at.trim();
        if at.is_empty() {
            return None;
        }
        let gates = self.effective_release_gates();
        let cut = gates.get(self.cut_index()).map(|(name, _)| name.clone());
        match at {
            "cut" => cut,
            "pushed" => gates
                .iter()
                .find(|(_, g)| g.requires.contains(&ReleaseCheck::Pushed))
                .map(|(name, _)| name.clone())
                .or(cut),
            _ => gates
                .iter()
                .find(|(name, _)| name == at)
                .map(|(name, _)| name.clone())
                .or(cut),
        }
    }

    /// The gate whose result judges a release's tag: `tag_gate`, else the first commit gate.
    pub fn tag_gate_name(&self) -> Option<String> {
        self.tag_gate
            .clone()
            .filter(|g| !g.trim().is_empty())
            .or_else(|| self.commit_gates.first().cloned())
    }

    /// Every release stage with the gate in force: its declared one, or the default that reproduces
    /// what stages did before they could act (FEAT-183):
    ///
    /// - the first stage `[branch]`, unless a declared gate holds `branch`;
    /// - the last stage `[move_main, tag]`, without `move_main` if a declared gate holds it and
    ///   without `tag` if a declared gate holds that (the stages after the tag only confirm,
    ///   FTR-026);
    /// - a single stage `[branch, move_main, tag]`;
    /// - a middle stage nothing.
    pub fn effective_release_gates(&self) -> Vec<(String, ReleaseGate)> {
        let stages = self.stages();
        let n = stages.len();
        let declared_holds = |a: ReleaseAction| {
            self.release_gates
                .iter()
                .any(|(k, g)| stages.contains(k) && g.on_enter.contains(&a))
        };
        let mut gates = stages
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if let Some(g) = self.release_gates.get(s) {
                    return (s.clone(), g.clone());
                }
                let on_enter = if n == 1 {
                    vec![
                        ReleaseAction::Branch,
                        ReleaseAction::MoveMain,
                        ReleaseAction::Tag,
                    ]
                } else if i == 0 {
                    if declared_holds(ReleaseAction::Branch) {
                        vec![]
                    } else {
                        vec![ReleaseAction::Branch]
                    }
                } else if i == n - 1 {
                    let mut v = vec![];
                    // `move_main` after a declared `tag` would be refused: that stage only confirms.
                    if !declared_holds(ReleaseAction::MoveMain)
                        && !declared_holds(ReleaseAction::Tag)
                    {
                        v.push(ReleaseAction::MoveMain);
                    }
                    if !declared_holds(ReleaseAction::Tag) {
                        v.push(ReleaseAction::Tag);
                    }
                    v
                } else {
                    vec![]
                };
                (
                    s.clone(),
                    ReleaseGate {
                        on_enter,
                        ..ReleaseGate::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        // The first stage after the tag with no declared gate asks for the tag's passed result.
        if let Some(cut) = gates
            .iter()
            .position(|(_, g)| g.on_enter.contains(&ReleaseAction::Tag))
            && let Some((name, g)) = gates.get_mut(cut + 1)
            && !self.release_gates.contains_key(name)
            && !self.commit_gates.is_empty()
        {
            g.requires = vec![ReleaseCheck::TagPassed];
            g.purpose = "Confirm what you checked here".to_string();
        }
        gates
    }

    /// Whether `stage` declares its own gate (rather than taking the default).
    pub fn declares_release_gate(&self, stage: &str) -> bool {
        self.release_gates.contains_key(stage)
    }

    /// The release gates and preparation must fit the stages: every gate names a stage, each
    /// action is placed once, `tag` only at the last stage, `move_main` not after `tag`, `prepare`
    /// not after `move_main`, and every path and template is one `prepare` can safely use.
    pub fn validate_release_gates(&self) -> Result<(), String> {
        let stages = self.stages();
        for key in self.release_gates.keys() {
            if !stages.contains(key) {
                return Err(format!(
                    "release stage '{key}' has a gate but is not one of this project's release \
                     stages ({}): clear it first with `kanbanr config release-stage {key} --clear`",
                    stages.join(", ")
                ));
            }
        }
        let effective = self.effective_release_gates();
        if self.commit_gates.is_empty()
            && let Some((stage, _)) = effective.iter().find(|(_, g)| {
                g.requires.contains(&ReleaseCheck::GatePassed)
                    || g.requires.contains(&ReleaseCheck::TagPassed)
            })
        {
            return Err(format!(
                "release stage '{stage}' requires a passed gate result but no commit gate is named: run \
                 `kanbanr config commit-gates ci` first"
            ));
        }
        for (i, (stage, gate)) in effective.iter().enumerate() {
            for a in &gate.after {
                let at = stages.iter().position(|s| s == a);
                if at == Some(i) {
                    return Err(format!(
                        "release stage '{stage}' lists itself in after: it must name an earlier \
                         stage"
                    ));
                }
                if !at.is_some_and(|j| j < i) {
                    return Err(format!(
                        "release stage '{stage}' lists '{a}' in after, but {a} is not an earlier \
                         stage of this project ({})",
                        stages.join(", ")
                    ));
                }
            }
        }
        for g in &self.commit_gates {
            crate::gates::check_name(g).map_err(|e| format!("commit gate: {e}"))?;
        }
        let mut at: std::collections::BTreeMap<ReleaseAction, usize> = Default::default();
        for (i, (stage, gate)) in effective.iter().enumerate() {
            for a in &gate.on_enter {
                if let Some(prev) = at.insert(*a, i) {
                    return Err(if prev == i {
                        format!("release stage '{stage}' lists '{}' twice", a.name())
                    } else {
                        format!(
                            "'{}' is placed at both '{}' and '{stage}': each action happens at \
                             one stage",
                            a.name(),
                            effective[prev].0
                        )
                    });
                }
            }
        }
        let cut = self.cut_index();
        for (i, (stage, gate)) in effective.iter().enumerate() {
            if i > cut
                && let Some(a) = gate.on_enter.first()
            {
                return Err(format!(
                    "release stage '{stage}' comes after the tag and can only confirm: it cannot {}",
                    a.name()
                ));
            }
            if i <= cut && gate.requires.contains(&ReleaseCheck::TagPassed) {
                return Err(format!(
                    "release stage '{stage}' requires tag_passed, but it does not come after the \
                     tag: only a stage after the tag ({}) can",
                    effective[cut].0
                ));
            }
        }
        if let Some(g) = &self.tag_gate
            && !self.commit_gates.contains(g)
        {
            return Err(format!(
                "the tag gate '{g}' is not one of the commit gates ({}): name it with `kanbanr \
                 config commit-gates` first",
                self.commit_gates.join(", ")
            ));
        }
        let place = |a: ReleaseAction| at.get(&a).copied();
        if let (Some(m), Some(t)) = (place(ReleaseAction::MoveMain), place(ReleaseAction::Tag))
            && m > t
        {
            return Err(format!(
                "'move_main' at '{}' comes after 'tag' at '{}': main moves before it is tagged",
                effective[m].0, effective[t].0
            ));
        }
        if let (Some(p), Some(m)) = (
            place(ReleaseAction::Prepare),
            place(ReleaseAction::MoveMain),
        ) && p > m
        {
            return Err(format!(
                "'prepare' at '{}' comes after 'move_main' at '{}': a release is prepared before \
                 main moves to it",
                effective[p].0, effective[m].0
            ));
        }
        let prep = &self.release_prepare;
        let path_ok = |what: &str, path: &str| {
            crate::releaseprep::check_repo_path(path).map_err(|e| format!("{what}: {e}"))
        };
        for f in &prep.version_files {
            path_ok("version file", &f.path)?;
            if let Some(pattern) = &f.pattern
                && pattern.matches("{version}").count() != 1
            {
                return Err(format!(
                    "the match for {} must name {{version}} exactly once: '{pattern}'",
                    f.path
                ));
            }
        }
        if let Some(c) = &prep.changelog {
            path_ok("changelog", c)?;
        }
        for c in &prep.changelog_copies {
            path_ok("changelog copy", c)?;
        }
        if let Some(notes) = &prep.notes {
            path_ok("notes", notes)?;
            if !notes.contains("{version}") && !notes.contains("{number}") {
                return Err(format!(
                    "the notes path must name {{version}} or {{number}}, so each release has its \
                     own page: '{notes}'"
                ));
            }
        }
        for l in &prep.notes_listings {
            path_ok("notes listing", &l.path)?;
            if l.after.trim().is_empty() || l.entry.trim().is_empty() {
                return Err(format!(
                    "the notes listing in {} needs both `after` (the line it goes below) and \
                     `entry`",
                    l.path
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cadence(yaml: &str) -> Result<Cadence, String> {
        crate::yaml::from_str::<Cadence>(yaml).map_err(|e| e.to_string())
    }

    fn actions(c: &Cadence) -> Vec<(String, Vec<&'static str>)> {
        c.effective_release_gates()
            .into_iter()
            .map(|(s, g)| (s, g.on_enter.iter().map(|a| a.name()).collect()))
            .collect()
    }

    /// DEF-043 R-1: a test's `at` names the release stage it waits for.
    #[test]
    fn a_checked_at_value_names_its_stage() {
        let c = cadence(
            "releases: true\nrelease_stages: [candidate, main, production]\n\
             release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
             main: { requires: [items_finished], on_enter: [move_main] }\n  \
             production: { requires: [main_at_release, pushed], on_enter: [tag] }\n",
        )
        .unwrap();
        assert_eq!(c.stage_for_check(""), None);
        assert_eq!(c.stage_for_check("cut").as_deref(), Some("production"));
        assert_eq!(c.stage_for_check("pushed").as_deref(), Some("production"));
        assert_eq!(c.stage_for_check("main").as_deref(), Some("main"));
        assert_eq!(c.stage_for_check("gone").as_deref(), Some("production"));
        // `pushed` with no stage requiring it waits for the cut.
        let c = cadence(
            "releases: true\nrelease_stages: [candidate, main, production, confirmed]\n\
             release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
             main: { requires: [items_finished], on_enter: [move_main] }\n  \
             production: { requires: [main_at_release], on_enter: [tag] }\n  \
             confirmed: { requires: [tag_passed] }\n",
        )
        .unwrap();
        assert_eq!(c.stage_for_check("pushed").as_deref(), Some("production"));
        assert_eq!(c.stage_for_check("confirmed").as_deref(), Some("confirmed"));
    }

    /// FEAT-212 R-1: the gates are read from `release_gates`, keyed by stage name, while
    /// `release_stages` stays a list of names; both survive a write unchanged.
    #[test]
    fn release_gates_read_beside_stage_names() {
        let c = cadence(
            "releases: true\nrelease_stages: [candidate, main, production]\n\
             release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
             main: { purpose: 'main holds the release', requires: [items_finished], on_enter: [move_main] }\n  \
             production: { requires: [main_at_release, pushed], on_enter: [tag] }\n\
             release_prepare:\n  version_files:\n    - path: api/Cargo.toml\n    \
             - { path: api/Cargo.lock, match: \"name = \\\"x\\\"\\nversion = \\\"{version}\\\"\" }\n  \
             changelog: CHANGELOG.md\n  notes: docs/src/releases/{version}.md\n",
        )
        .unwrap();
        assert_eq!(c.release_stages, ["candidate", "main", "production"]);
        assert_eq!(
            c.release_gates["main"].requires,
            [ReleaseCheck::ItemsFinished]
        );
        assert_eq!(c.release_gates["main"].purpose, "main holds the release");
        assert_eq!(
            actions(&c),
            [
                ("candidate".to_string(), vec!["branch", "prepare"]),
                ("main".to_string(), vec!["move_main"]),
                ("production".to_string(), vec!["tag"]),
            ]
        );
        assert_eq!(
            c.release_prepare.version_files[1].pattern.as_deref(),
            Some("name = \"x\"\nversion = \"{version}\"")
        );
        c.validate_release_gates().unwrap();
        let text = crate::yaml::to_string(&c).unwrap();
        assert_eq!(cadence(&text).unwrap(), c, "round trip\n{text}");
        // A cadence that does not use them writes neither key.
        let plain = cadence("releases: true\nrelease_stages: [a, b]\n").unwrap();
        let text = crate::yaml::to_string(&plain).unwrap();
        assert!(
            !text.contains("release_gates") && !text.contains("release_prepare"),
            "{text}"
        );
        // An unknown key inside a gate is kept, as the cadence keeps its own (FEAT-151).
        let newer =
            cadence("release_stages: [a]\nrelease_gates:\n  a: { on_enter: [branch], later: 1 }\n")
                .unwrap();
        assert!(crate::yaml::to_string(&newer).unwrap().contains("later"));
    }

    /// FEAT-212 R-2: a gate on no stage, an unknown check or action, or actions out of order are
    /// refused, saying why.
    #[test]
    fn release_gates_validated() {
        let refused = |yaml: &str, says: &str| {
            let err = match cadence(yaml) {
                Err(e) => e,
                Ok(c) => c.validate_release_gates().unwrap_err(),
            };
            assert!(err.contains(says), "{yaml}\n→ {err}");
        };
        refused(
            "release_stages: [a, b]\nrelease_gates: { c: { on_enter: [branch] } }\n",
            "`kanbanr config release-stage c --clear`",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { requires: [pushd] } }\n",
            "'pushd' is not a check a release stage can require — the checks are: \
             items_finished, prepared, main_at_release, pushed",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [merge] } }\n",
            "'merge' is not a release stage action — the actions are: branch, prepare, \
             move_main, tag",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [branch, tag] }, b: { on_enter: [move_main] } }\n",
            "release stage 'b' comes after the tag and can only confirm: it cannot move_main",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { a: { on_enter: [branch] }, b: { on_enter: [branch] } }\n",
            "'branch' is placed at both 'a' and 'b'",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [move_main] }, c: { on_enter: [prepare, tag] } }\n",
            "'prepare' at 'c' comes after 'move_main' at 'b'",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [prepare, prepare] } }\n",
            "lists 'prepare' twice",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: ../x.toml }] }\n",
            "version file",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: /etc/x.toml }] }\n",
            "absolute",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { changelog: .git/config }\n",
            ".git",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: a.txt, match: 'v = {version} {version}' }] }\n",
            "exactly once",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { notes: docs/notes.md }\n",
            "{version} or {number}",
        );
        // Moving the cut's main step to an earlier stage is allowed: the last stage then only tags.
        let ok =
            cadence("release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [move_main] } }\n")
                .unwrap();
        ok.validate_release_gates().unwrap();
        assert_eq!(actions(&ok)[2].1, ["tag"]);
    }

    /// FEAT-212 R-3: with no gate declared, the first stage makes the branch and the last moves
    /// main and tags, as before; a single stage does all three.
    #[test]
    fn default_release_gates_reproduce_today() {
        let plain = Cadence::default();
        assert_eq!(
            actions(&plain),
            [
                ("candidate".to_string(), vec!["branch"]),
                ("test".to_string(), vec![]),
                ("staging".to_string(), vec![]),
                ("production".to_string(), vec!["move_main", "tag"]),
            ]
        );
        assert!(
            plain
                .effective_release_gates()
                .iter()
                .all(|(_, g)| g.requires.is_empty()),
            "no synthesised conditions"
        );
        let one = cadence("release_stages: [ship]\n").unwrap();
        assert_eq!(actions(&one)[0].1, ["branch", "move_main", "tag"]);
        plain.validate_release_gates().unwrap();
        one.validate_release_gates().unwrap();
        // A declared gate taking the branch leaves the first stage without it.
        let moved =
            cadence("release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [branch] } }\n")
                .unwrap();
        assert_eq!(
            actions(&moved),
            [
                ("a".to_string(), vec![]),
                ("b".to_string(), vec!["branch"]),
                ("c".to_string(), vec!["move_main", "tag"]),
            ]
        );
    }
    /// FTR-020 R-5: a stage before the cut that requires nothing, follows nothing and does nothing
    /// is inert; the default four have two of them, the cut and a confirmation after it never are.
    #[test]
    fn a_stage_asking_and_doing_nothing_is_inert() {
        let old = Cadence::default();
        assert_eq!(old.inert_stages(), ["test", "staging"]);
        let (stages, gates, _) = Cadence::recommended(false);
        let rec = Cadence {
            release_stages: stages,
            release_gates: gates,
            ..Cadence::default()
        };
        assert!(rec.inert_stages().is_empty());
        // A stage after the tag only confirms, so it is never inert.
        let after = cadence(
            "release_stages: [a, b, c, d]\nrelease_gates: { b: { on_enter: [move_main, tag] } }\n",
        )
        .unwrap();
        assert_eq!(after.inert_stages(), Vec::<String>::new());
        // A condition, an `after` or an action each make a stage act.
        let c = cadence(
            "release_stages: [a, b, c, d, e]\nrelease_gates: { b: { requires: [pushed] }, c: { after: [b] } }\n",
        )
        .unwrap();
        assert_eq!(c.inert_stages(), ["d"]);
        assert!(ReleaseGate::default().is_inert());
        let w = Cadence::inert_warning("test");
        assert!(
            w.contains("'test'") && w.contains("kanbanr config release-stage test --requires"),
            "{w}"
        );
    }

    /// FTR-020 R-6: the stage view marks the inert stages.
    #[test]
    fn the_stage_view_marks_an_inert_stage() {
        let views = Cadence::default().release_stage_views();
        let inert: Vec<_> = views.iter().map(|v| v.inert).collect();
        assert_eq!(inert, [false, true, true, false]);
    }

    /// FTR-026 R-1: a stage after the tag can only confirm; `tag_passed` belongs only to such a
    /// stage and `tag_gate` must be a commit gate.
    #[test]
    fn a_stage_after_the_tag_can_only_confirm() {
        let refused = |yaml: &str, says: &str| {
            let err = cadence(yaml).unwrap().validate_release_gates().unwrap_err();
            assert!(err.contains(says), "{yaml}\n-> {err}");
        };
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [tag] }, c: { on_enter: [prepare] } }\n",
            "release stage 'c' comes after the tag and can only confirm: it cannot prepare",
        );
        refused(
            "release_stages: [a, b, c]\ncommit_gates: [ci]\nrelease_gates: { b: { requires: [tag_passed] } }\n",
            "requires tag_passed",
        );
        refused(
            "release_stages: [a, b]\ncommit_gates: [ci]\ntag_gate: other\n",
            "the tag gate 'other' is not one of the commit gates",
        );
        let ok = cadence(
            "release_stages: [a, b, c, d]\ncommit_gates: [ci]\nrelease_gates: { b: { on_enter: [move_main] }, c: { on_enter: [tag] } }\n",
        )
        .unwrap();
        ok.validate_release_gates().unwrap();
        let views = ok.release_stage_views();
        assert!(views[2].cut && !views[2].after_cut);
        assert!(views[3].after_cut && !views[3].cut);
        // The first stage after the tag asks for the tag's passed result when a commit gate is named.
        assert_eq!(views[3].requires, [ReleaseCheck::TagPassed]);
        assert_eq!(ok.cut_index(), 2);
        assert_eq!(ok.tag_gate_name().as_deref(), Some("ci"));
    }

    /// FTR-026 R-1: the tag may sit at any stage; the default places it at the last unless a
    /// declared gate holds it.
    #[test]
    fn tag_at_a_middle_stage_is_allowed() {
        let c = cadence(
            "release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [move_main, tag] } }\n",
        )
        .unwrap();
        c.validate_release_gates().unwrap();
        assert_eq!(
            actions(&c),
            [
                ("a".to_string(), vec!["branch"]),
                ("b".to_string(), vec!["move_main", "tag"]),
                ("c".to_string(), vec![]),
            ]
        );
        assert_eq!(c.cut_index(), 1);
        // With no commit gate named, no condition is synthesised.
        assert!(c.effective_release_gates()[2].1.requires.is_empty());
        assert_eq!(Cadence::default().cut_index(), 3);
    }

    /// FTR-021 R-4: `after` names an earlier stage of the project, and the check list names
    /// `items_merged`.
    #[test]
    fn after_must_name_an_earlier_stage_of_the_project() {
        let refused = |yaml: &str, says: &str| {
            let err = cadence(yaml).unwrap().validate_release_gates().unwrap_err();
            assert!(err.contains(says), "{yaml}\n→ {err}");
        };
        refused(
            "release_stages: [candidate, main, production]\nrelease_gates: { production: { after: [ci] } }\n",
            "release stage 'production' lists 'ci' in after, but ci is not an earlier stage of this \
             project (candidate, main, production)",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { b: { after: [c] } }\n",
            "'c' in after, but c is not an earlier stage",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { b: { after: [b] } }\n",
            "lists itself in after",
        );
        cadence("release_stages: [a, b, c]\nrelease_gates: { c: { after: [a, b] } }\n")
            .unwrap()
            .validate_release_gates()
            .unwrap();
    }

    /// FTR-021 R-4: a misspelt check is refused with a list that includes `items_merged`, and
    /// `stage_passed` points to `after`.
    #[test]
    fn an_unknown_check_lists_items_merged() {
        let err = cadence("release_stages: [a, b]\nrelease_gates: { a: { requires: [merged] } }\n")
            .unwrap_err();
        assert!(err.contains("items_merged"), "{err}");
        let err =
            cadence("release_stages: [a, b]\nrelease_gates: { b: { requires: [stage_passed] } }\n")
                .unwrap_err();
        assert!(err.contains("`after`"), "{err}");
        let ok =
            cadence("release_stages: [a, b]\nrelease_gates: { a: { requires: [items_merged] } }\n")
                .unwrap();
        assert_eq!(ok.release_gates["a"].requires, [ReleaseCheck::ItemsMerged]);
    }

    /// FTR-021 R-6: `after` is read, written and shown, and an older binary keeps it as an unknown
    /// key of the gate.
    #[test]
    fn after_survives_a_round_trip_through_an_older_gate() {
        let c = cadence(
            "release_stages: [a, b, c]\nrelease_gates: { c: { after: [b], on_enter: [tag] } }\n",
        )
        .unwrap();
        assert_eq!(c.release_gates["c"].after, ["b"]);
        assert_eq!(c.release_stage_views()[2].after, ["b"]);
        let text = crate::yaml::to_string(&c).unwrap();
        assert_eq!(cadence(&text).unwrap(), c, "{text}");
        // An older gate (no `after` field) holds it in `extra` and writes it back.
        #[derive(Debug, Default, Deserialize, Serialize)]
        struct Older {
            #[serde(default)]
            on_enter: Vec<ReleaseAction>,
            #[serde(flatten)]
            extra: Extra,
        }
        let older: Older = crate::yaml::from_str("after: [b]\non_enter: [tag]\n").unwrap();
        assert!(crate::yaml::to_string(&older).unwrap().contains("after"));
        // Absent when empty.
        let plain =
            cadence("release_stages: [a, b]\nrelease_gates: { a: { on_enter: [branch] } }\n")
                .unwrap();
        assert!(!crate::yaml::to_string(&plain).unwrap().contains("after"));
    }
}
