//! Releases: planned up front, cut from finished work (FEAT-120).
//!
//! Only for projects that switch them on (`cadence.releases`, FEAT-121). A release is planned —
//! items are assigned to it as they are to a sprint — and **cut**: its planned items that are
//! finished ship, its release notes are written from their definitions, and what did not make it
//! is carried to the next planned release. Feedback afterwards points back at the version
//! (`feature add --found-in <version>`).
//!
//! A release also has a **stage** (FEAT-183): one of `cadence.release_stages`, by default
//! `candidate → test → staging → production`, with each move recorded. `promote` moves it; the
//! first stage makes the release branch in the code repository, the last is the cut.
//!
//! Stored in `projects/<id>/releases.yaml`; notes go to the board doc `releases/<version>.md`.

use crate::coderepo;
use crate::error::{CoreError, Result};
use crate::models::{Extra, FeatureItem};
use crate::releasegates::{ReleaseAction, ReleaseCheck};
use crate::store::{Capability, Project, Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RELEASES_FILE: &str = "releases.yaml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    #[default]
    Planned,
    Shipped,
}

/// A planned item that did not ship, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(rename = "ReleaseCarry")
)]
pub struct Carry {
    pub code: String,
    /// The release it moved to, or `unplanned`.
    pub to: String,
    /// Why it did not ship.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Release {
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Target date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(default)]
    pub state: ReleaseState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shipped_at: String,
    /// What shipped in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shipped: Vec<String>,
    /// The board doc holding its release notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes_doc: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<Carry>,
    /// Where the release is now, from `cadence.release_stages` (FEAT-183). Empty until first
    /// promoted: a release added before stages existed reads as "not yet a candidate".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    /// Every stage change, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<StageChange>,
    /// The code branch, set when promoted to the first stage (`release/<V>`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// The cut release this patches (FEAT-184); empty for an ordinary release.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// Keys this version does not know, kept and written back unchanged, so an older binary never
    /// deletes what a newer one recorded (as `Cadence` does, FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One move of a release between stages: where to, who moved it, and when (FEAT-183). A move back
/// carries the reason it was made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StageChange {
    pub stage: String,
    pub by: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// A release with what it holds and how much of that is finished, as the monitor shows it
/// (FEAT-138). Derived, never stored.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseReport {
    #[serde(flatten)]
    pub release: Release,
    /// Planned into it, or shipped in it.
    pub items: Vec<String>,
    /// How many of `items` are finished, by the same rule as the sprint's: an end status, or a stage
    /// marked `done` (FEAT-137). The page used to keep its own copy of the rule, counting only the
    /// end status — a Done item in a planned scrum release read as unfinished.
    pub finished: usize,
    /// For a patch: what its parent shipped, which the patch keeps (FEAT-184).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub included: Vec<String>,
}

/// Every release, reported.
pub fn report_all(store: &Store, id: &str) -> Result<Vec<ReleaseReport>> {
    let project = store.load_meta(id)?;
    let all = load(store, id)?;
    Ok(all
        .clone()
        .into_iter()
        .map(|release| {
            let items: Vec<&crate::FeatureItem> = project
                .features
                .iter()
                .filter(|f| {
                    f.release.as_deref() == Some(release.version.as_str())
                        || release.shipped.contains(&f.code)
                })
                .collect();
            let finished = items
                .iter()
                .filter(|f| project.config.counts_as_done(&f.status))
                .count();
            let included = all
                .iter()
                .find(|r| !release.parent.is_empty() && r.version == release.parent)
                .map(|r| r.shipped.clone())
                .unwrap_or_default();
            ReleaseReport {
                items: items.iter().map(|f| f.code.clone()).collect(),
                finished,
                included,
                release,
            }
        })
        .collect())
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(RELEASES_FILE)
}

/// A project's releases, in the order they were added. None is a normal state.
pub fn load(store: &Store, id: &str) -> Result<Vec<Release>> {
    match std::fs::read_to_string(path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(crate::yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, releases: &[Release]) -> Result<()> {
    std::fs::write(path(store, id), crate::yaml::to_string(releases)?)?;
    Ok(())
}

fn cadence_on(store: &Store, id: &str) -> Result<Project> {
    let project = store.load(id)?;
    Store::require_cadence(&project, Capability::Releases)?;
    Ok(project)
}

/// Add a planned release.
pub fn add(store: &Store, id: &str, version: &str, target: &str, name: &str) -> Result<Release> {
    cadence_on(store, id)?;
    let version = version.trim();
    if version.is_empty() {
        return Err(CoreError::Unsupported(
            "a release needs a version (e.g. v1.2.0)".into(),
        ));
    }
    if !target.trim().is_empty() && crate::gantt::parse_ymd(target).is_none() {
        return Err(CoreError::Unsupported(format!(
            "'{target}' is not a date (YYYY-MM-DD)"
        )));
    }
    let mut releases = load(store, id)?;
    if releases.iter().any(|r| r.version == version) {
        return Err(CoreError::Unsupported(format!("{version} already exists")));
    }
    let release = Release {
        version: version.to_string(),
        name: name.trim().to_string(),
        target: target.trim().to_string(),
        ..Release::default()
    };
    releases.push(release.clone());
    save(store, id, &releases)?;
    Ok(release)
}

/// Add a patch of a cut release (FEAT-184): it records its parent, branches `release/<V>` from the
/// parent's tag, and starts at the first stage. Any cut release can be patched (Q1), an older
/// supported one included; the version is the user's to name.
#[allow(clippy::too_many_arguments)]
pub fn add_patch(
    store: &Store,
    id: &str,
    version: &str,
    target: &str,
    name: &str,
    parent: &str,
    by: &str,
    repo: Option<&Path>,
) -> Result<Release> {
    let project = cadence_on(store, id)?;
    let releases = load(store, id)?;
    let parent = parent.trim();
    let not_cut = || {
        CoreError::Unsupported(format!(
            "{parent} has not been cut (or has no tag), so there is nothing to patch"
        ))
    };
    let p = releases
        .iter()
        .find(|r| r.version == parent)
        .ok_or_else(not_cut)?;
    if p.state != ReleaseState::Shipped {
        return Err(not_cut());
    }
    let first = project
        .config
        .cadence
        .stages()
        .first()
        .cloned()
        .unwrap_or_default();
    let repo = repo.ok_or_else(|| needs_repo(version, &first))?;
    if !coderepo::tag_exists(repo, parent) {
        return Err(not_cut());
    }
    let branch = format!("release/{}", version.trim());
    if coderepo::branch_exists(repo, &branch) {
        return Err(CoreError::Unsupported(format!(
            "{branch} already exists in {} — nothing was changed",
            repo.display()
        )));
    }
    // The board first refuses what it would (a duplicate version, a bad date), then the branch.
    let added = add(store, id, version, target, name)?;
    if let Err(e) = coderepo::create_branch_from(repo, &branch, &format!("refs/tags/{parent}")) {
        let mut all = load(store, id)?;
        all.retain(|r| r.version != added.version);
        save(store, id, &all)?;
        return Err(e);
    }
    let mut all = load(store, id)?;
    // The first stage's other actions run as they would on promotion (FEAT-212): a patch that
    // prepares itself commits its version, changelog and notes onto its branch now.
    let prepares = project
        .config
        .cadence
        .effective_release_gates()
        .first()
        .is_some_and(|(_, g)| g.on_enter.contains(&ReleaseAction::Prepare))
        && !project.config.cadence.release_prepare.writes_nothing();
    let mut prep_commit = None;
    if prepares {
        let undo_add = |all: &mut Vec<Release>| {
            let _ = coderepo::delete_branch(repo, &branch);
            all.retain(|r| r.version != added.version);
            let _ = save(store, id, all);
        };
        let index = all
            .iter()
            .position(|r| r.version == added.version)
            .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
        all[index].parent = parent.to_string();
        let worked = coderepo::rev_parse(repo, &branch)
            .map_err(|e| e.to_string())
            .and_then(|tip| {
                work_prepare(
                    store,
                    id,
                    &project,
                    &all,
                    index,
                    repo,
                    &tip,
                    &branch,
                    &format!("kanbanr release add {version} --patch-of {parent}"),
                )
            });
        let made = match worked {
            Ok(w) if w.files.is_empty() => Ok(None),
            Ok(w) => coderepo::commit_files(repo, &branch, &w.files, &w.message).map(Some),
            Err(why) => Err(CoreError::Unsupported(format!(
                "{why} — nothing was changed"
            ))),
        };
        match made {
            Ok(m) => prep_commit = m,
            Err(e) => {
                undo_add(&mut all);
                return Err(e);
            }
        }
    }
    let r = all
        .iter_mut()
        .find(|r| r.version == added.version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    r.parent = parent.to_string();
    r.branch = branch.clone();
    r.stage = first.clone();
    r.stages.push(StageChange {
        stage: first,
        by: by.trim().to_string(),
        at: crate::now_rfc3339(),
        reason: "on creation, as a patch".to_string(),
    });
    let release = r.clone();
    if let Err(e) = save(store, id, &all) {
        if let Some((old, new)) = &prep_commit {
            let _ = coderepo::undo_commit(repo, &branch, old, new);
        }
        let _ = coderepo::delete_branch(repo, &branch);
        return Err(e);
    }
    Ok(release)
}

/// Plan items into a release.
pub fn plan(store: &Store, id: &str, version: &str, items: &[String]) -> Result<Release> {
    cadence_on(store, id)?;
    let releases = load(store, id)?;
    let release = releases
        .iter()
        .find(|r| r.version == version)
        .cloned()
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if release.state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has shipped — plan into a coming release"
        )));
    }
    for item in items {
        store.set_feature_release(id, item, Some(version.to_string()))?;
    }
    Ok(release)
}

/// Whether an item can ship, and to which end status — or why not. It ships if it is already
/// finished, or if its work is done (every task complete) and the workflow allows it to an end
/// status whose gate it meets. A workflow that declares no gates holds it to what `finish` asks.
fn ship_to(
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    f: &FeatureItem,
) -> std::result::Result<Option<String>, String> {
    let config = &project.config;
    let is_end = |s: &str| crate::graph::is_terminal_status(config, s) && !config.is_no_op(s);
    if is_end(&f.status) {
        return Ok(None); // already finished: it ships as it is
    }
    let open = f
        .todo_lists
        .iter()
        .flat_map(|l| &l.tasks)
        .filter(|t| t.state != crate::models::TaskState::Completed)
        .count();
    if open > 0 {
        return Err(format!("{open} task(s) still open"));
    }
    let Some(end) = config
        .statuses
        .iter()
        .find(|s| is_end(s) && config.transition_allowed(&f.status, s))
    else {
        return Err(format!("not finished — it is at {}", f.status));
    };
    if !config.declares_gates() {
        let gaps = crate::readiness::evaluate(f, None, crate::readiness::CHECK);
        if let Some(first) = gaps.first() {
            return Err(first.message.clone());
        }
    }
    Store::check_gate(project, charter, ctx, &f.code, end, None).map_err(|e| e.to_string())?;
    Ok(Some(end.clone()))
}

/// What a cut did.
#[derive(Debug, Clone, Serialize)]
pub struct Cut {
    pub release: Release,
    pub notes: String,
}

/// Cut a release: its planned items that are finished ship (moving to their end status through its
/// gate), release notes are written from their definitions, and the rest is carried to the next
/// planned release — or back to unplanned — with the reason it did not make it.
pub fn cut(store: &Store, id: &str, version: &str) -> Result<Cut> {
    cut_with(store, id, version, None)
}

/// The cut, recording the stage change that made it when there is one (FEAT-183). Either way the
/// release ends at the last stage: a cut release is in production, whichever command cut it.
fn cut_with(store: &Store, id: &str, version: &str, change: Option<StageChange>) -> Result<Cut> {
    let project = cadence_on(store, id)?;
    let charter = crate::charter::load(store, id)?;
    let ctx = crate::readiness::Context::load(store, id, &project.config);
    let mut releases = load(store, id)?;
    let index = releases
        .iter()
        .position(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if releases[index].state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has already shipped"
        )));
    }
    let next = releases
        .iter()
        .skip(index + 1)
        .find(|r| r.state == ReleaseState::Planned)
        .map(|r| r.version.clone());

    let mut planned: Vec<FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.release.as_deref() == Some(version))
        .cloned()
        .collect();
    // In code order, so the notes and the record read the same way every time.
    planned.sort_by(|a, b| a.code.cmp(&b.code));
    let mut shipped: Vec<FeatureItem> = Vec::new();
    let mut carried: Vec<Carry> = Vec::new();
    for f in &planned {
        match ship_to(&project, &charter, &ctx, f) {
            Ok(end) => {
                let moved = match end {
                    Some(end) => store.move_feature_approved(id, &f.code, &end, None)?,
                    None => f.clone(),
                };
                shipped.push(moved);
            }
            Err(why) => {
                store.set_feature_release(id, &f.code, next.clone())?;
                carried.push(Carry {
                    code: f.code.clone(),
                    to: next.clone().unwrap_or_else(|| "unplanned".to_string()),
                    why,
                });
            }
        }
    }

    let doc = format!("releases/{version}.md");
    let parent = releases
        .iter()
        .find(|r| !releases[index].parent.is_empty() && r.version == releases[index].parent);
    let notes = notes(&releases[index], parent, &shipped, &carried);
    store.write_doc(id, &doc, &notes)?;
    let release = &mut releases[index];
    release.state = ReleaseState::Shipped;
    release.shipped_at = crate::now_rfc3339();
    release.shipped = shipped.iter().map(|f| f.code.clone()).collect();
    release.notes_doc = doc;
    release.carried = carried;
    if let Some(last) = project.config.cadence.stages().last() {
        release.stage = last.clone();
    }
    if let Some(change) = change {
        release.stages.push(change);
    }
    let release = release.clone();
    save(store, id, &releases)?;
    Ok(Cut { release, notes })
}

/// What a cut would do with one planned item, worked out without writing anything (FEAT-183).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CutPreview {
    pub code: String,
    /// It ships; otherwise it is carried over, for `why`.
    pub ships: bool,
    /// The end status it would move to, when it is not already at one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// What cutting `version` now would ship and carry over: the cut's own rule (`ship_to`), with
/// nothing written. Production promotion works it out before it touches anything.
pub fn cut_preview(store: &Store, id: &str, version: &str) -> Result<Vec<CutPreview>> {
    let project = cadence_on(store, id)?;
    let charter = crate::charter::load(store, id)?;
    let ctx = crate::readiness::Context::load(store, id, &project.config);
    if !load(store, id)?.iter().any(|r| r.version == version) {
        return Err(CoreError::Unsupported(format!("no release {version}")));
    }
    let mut planned: Vec<&FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.release.as_deref() == Some(version))
        .collect();
    planned.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(planned
        .into_iter()
        .map(|f| match ship_to(&project, &charter, &ctx, f) {
            Ok(end) => CutPreview {
                code: f.code.clone(),
                ships: true,
                to: end.unwrap_or_default(),
                why: String::new(),
            },
            Err(why) => CutPreview {
                code: f.code.clone(),
                ships: false,
                to: String::new(),
                why,
            },
        })
        .collect())
}

/// What a promotion did (FEAT-183).
#[derive(Debug, Clone, Serialize)]
pub struct Promoted {
    pub release: Release,
    /// The stage it left; empty when it had none yet.
    pub from: String,
    pub to: String,
    /// The release branch made on entering the first stage.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch_created: String,
    /// The tag made at the cut.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tagged: String,
    /// The main branch fast-forwarded to the release, at whichever stage moved it (FEAT-212).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub main_branch: String,
    /// At the cut: what ships and what is carried over, as worked out before anything changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preview: Vec<CutPreview>,
    /// At the cut: the release notes written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The release's commits the integration branch lacks, once main has moved or the release is
    /// tagged (FEAT-184, FEAT-212). kanbanr lists them; `kanbanr git forward` or the user merges
    /// them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forward_merge: Vec<CommitLine>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub forward_into: String,
    /// The steps this promotion took, in the order it took them (FEAT-212).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<PlanStep>,
    /// The release preparation commit, when one was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepared: Option<Prepared>,
    /// Main's tips before and after it moved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_moved: Option<(String, String)>,
    /// What to do next, in a sentence.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub next: String,
    /// What `pushed` was judged against, when a stage required it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginState>,
}

/// The release preparation commit a promotion made (FEAT-212).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prepared {
    pub sha: String,
    pub files: Vec<String>,
    /// The notes draft written, if any.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

/// One commit, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CommitLine {
    pub sha: String,
    pub subject: String,
}

fn needs_repo(version: &str, stage: &str) -> CoreError {
    CoreError::Unsupported(needs_repo_text(version, stage))
}

fn needs_repo_text(version: &str, stage: &str) -> String {
    format!(
        "promoting {version} to {stage} needs the code repository: use the CLI from the project \
         folder"
    )
}

/// Whether a promotion may fetch main's remote before judging `pushed` (FEAT-212).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// Judge from the remote-tracking refs as they are.
    No,
    /// Fetch main first; if that fails, judge from the last-fetched refs and say so.
    Try,
}

/// One condition of a stage entered or passed, and whether it holds (FEAT-212).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanCondition {
    pub check: ReleaseCheck,
    /// The stage that requires it.
    pub stage: String,
    pub holds: bool,
    /// What was found.
    pub detail: String,
    /// What to do about it, when it does not hold.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fix: String,
}

/// One step a promotion takes, and what it will do (FEAT-212).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanStep {
    pub action: ReleaseAction,
    /// The stage whose entry does it.
    pub stage: String,
    /// What it will do, in a sentence.
    pub does: String,
    /// It changes nothing (already done, or nothing to do).
    pub noop: bool,
    /// Why it cannot be taken, when it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

/// The remote-tracking branch `pushed` was judged against, and whether it was fetched first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OriginState {
    pub tracking: String,
    pub fetched: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fetch_error: String,
}

/// What promoting a release would do, worked out with nothing changed (FEAT-212): what the button
/// says, each step and condition, and — for the cut — what ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleasePlan {
    pub version: String,
    /// The stage it is at; empty when it has none yet.
    pub from: String,
    pub to: String,
    /// `forward`, `back` or `again`.
    pub direction: String,
    /// This promotion is the cut.
    pub cut: bool,
    /// What the one button says: "Promote to main", "Cut v0.2.1", "Promote to main again".
    pub button: String,
    pub conditions: Vec<PlanCondition>,
    pub steps: Vec<PlanStep>,
    /// The cut only: what ships and what is carried over.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preview: Vec<CutPreview>,
    /// The commit the tag will name, when this promotion tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_commit: Option<CommitLine>,
    /// The release branch's tip now (empty while it does not exist): `expect` on promote.
    pub release_tip: String,
    /// Each thing missing before it can go, as what to do about it.
    pub missing: Vec<String>,
    pub ready: bool,
    /// A move back, which needs a reason.
    pub needs_reason: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginState>,
    /// What comes after this promotion, e.g. "push main, wait for CI, then Cut v0.2.1".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after: String,
    /// Whether the code repository was there to read.
    pub repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Back,
    Again,
}

/// What `prepare` writes, worked out from the release tip.
#[derive(Debug, Clone, Default)]
struct PrepWork {
    files: Vec<(String, String)>,
    message: String,
    notes: String,
    does: String,
}

/// A plan with what applying it needs.
struct Worked {
    plan: ReleasePlan,
    releases: Vec<Release>,
    index: usize,
    direction: Direction,
    stage: String,
    branch: String,
    /// Where the branch step makes the branch from, and the line to make first (git model).
    base: String,
    make_line: Option<(String, String)>,
    main: String,
    prep: Option<PrepWork>,
    /// Refusals of the steps, in the order the cut always checked them.
    blocks: Vec<String>,
    patch: bool,
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn today_ymd() -> String {
    let d = crate::sprints::today();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// The release's integration branch: where its fixes must be merged forward into.
fn integration_of(store: &Store, id: &str, project: &Project, repo: &Path, main: &str) -> String {
    if project.config.cadence.git_model {
        crate::gitmodel::load(store, id, repo)
            .ok()
            .and_then(|(m, _)| m.integration_branch.or(m.line))
            .unwrap_or_else(|| main.to_string())
    } else {
        project
            .config
            .cadence
            .integration_branch
            .clone()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| main.to_string())
    }
}

/// What promoting `version` to `to` (default: the next stage) would do, with nothing changed
/// (FEAT-212): the stages it enters or passes, their conditions judged against the repository as
/// it is (a condition counts as met when an action of the same promotion makes it true), the
/// steps in the order they would be taken, and what is missing.
pub fn plan_promotion(
    store: &Store,
    id: &str,
    version: &str,
    to: Option<&str>,
    reason: &str,
    repo: Option<&Path>,
    fetch: Fetch,
) -> Result<ReleasePlan> {
    work_out(store, id, version, to, reason, repo, fetch).map(|w| w.plan)
}

#[allow(clippy::too_many_arguments)]
fn work_out(
    store: &Store,
    id: &str,
    version: &str,
    to: Option<&str>,
    reason: &str,
    repo: Option<&Path>,
    fetch: Fetch,
) -> Result<Worked> {
    let project = cadence_on(store, id)?;
    let cadence = &project.config.cadence;
    let stages = cadence.stages();
    let gates = cadence.effective_release_gates();
    let releases = load(store, id)?;
    let index = releases
        .iter()
        .position(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    let release = releases[index].clone();
    if release.state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has shipped: a release never moves back from production"
        )));
    }
    let from = release.stage.clone();
    let current = stages.iter().position(|s| *s == from);
    let target = match to.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => stages.iter().position(|s| s == name).ok_or_else(|| {
            CoreError::Unsupported(format!(
                "'{name}' is not a release stage — this project's are {}",
                stages.join(", ")
            ))
        })?,
        None => current.map_or(0, |c| c + 1),
    };
    let Some(stage) = stages.get(target).cloned() else {
        return Err(CoreError::Unsupported(format!(
            "{version} is at {from}, the last stage — there is no next one"
        )));
    };
    let direction = if current == Some(target) {
        Direction::Again
    } else if current.is_some_and(|c| target < c) {
        Direction::Back
    } else {
        Direction::Forward
    };
    let last = stages.len() - 1;
    let cut = direction == Direction::Forward && target == last;
    let entered: Vec<usize> = match direction {
        Direction::Forward => (current.map_or(0, |c| c + 1)..=target).collect(),
        Direction::Again => vec![target],
        Direction::Back => vec![],
    };
    let mut actions: Vec<(ReleaseAction, String)> = Vec::new();
    let mut requires: Vec<(ReleaseCheck, String)> = Vec::new();
    for &i in &entered {
        let (name, gate) = &gates[i];
        for a in &gate.on_enter {
            if !actions.iter().any(|(x, _)| x == a) {
                actions.push((*a, name.clone()));
            }
        }
        for c in &gate.requires {
            if !requires.iter().any(|(x, _)| x == c) {
                requires.push((*c, name.clone()));
            }
        }
    }
    // Every forward move needs the release's branch: a move that skipped the stage making it, or
    // a process whose one stage is also the cut, makes it first (FEAT-184, R-4).
    if direction == Direction::Forward
        && release.branch.is_empty()
        && !actions.iter().any(|(a, _)| *a == ReleaseAction::Branch)
    {
        actions.push((ReleaseAction::Branch, stage.clone()));
    }
    actions.sort_by_key(|(a, _)| *a);
    let has = |a: ReleaseAction| actions.iter().any(|(x, _)| *x == a);
    let stage_holding = |a: ReleaseAction| {
        gates
            .iter()
            .find(|(_, g)| g.on_enter.contains(&a))
            .map(|(s, _)| s.clone())
    };

    let branch = if release.branch.is_empty() {
        format!("release/{version}")
    } else {
        release.branch.clone()
    };
    let patch = !release.parent.is_empty();
    let main = match repo {
        Some(repo) => cadence
            .main_branch
            .clone()
            .unwrap_or_else(|| coderepo::default_branch(repo)),
        None => cadence
            .main_branch
            .clone()
            .unwrap_or_else(|| "main".to_string()),
    };
    let mut blocks: Vec<String> = Vec::new();
    let mut steps: Vec<PlanStep> = Vec::new();
    let mut base = String::new();
    let mut make_line = None;
    let mut prep = None;
    let mut release_tip = String::new();
    let mut origin = None;
    let mut conditions: Vec<PlanCondition> = Vec::new();
    let preview = cut_preview(store, id, version)?;

    let step =
        |action: ReleaseAction, does: String, noop: bool, blocked: Option<String>| PlanStep {
            action,
            stage: actions
                .iter()
                .find(|(a, _)| *a == action)
                .map(|(_, s)| s.clone())
                .unwrap_or_default(),
            does,
            noop,
            blocked,
        };

    match repo {
        None => {
            if !actions.is_empty() {
                blocks.push(needs_repo_text(version, &stage));
            }
            for (a, _) in &actions {
                let does = a.does().replace("release/<V>", &branch);
                steps.push(step(
                    *a,
                    does,
                    false,
                    Some(needs_repo_text(version, &stage)),
                ));
            }
            for (c, s) in &requires {
                let (holds, detail) = match c {
                    ReleaseCheck::ItemsFinished => items_finished(&preview),
                    ReleaseCheck::Prepared if cadence.release_prepare.writes_nothing() => {
                        (true, "nothing is configured to prepare".to_string())
                    }
                    ReleaseCheck::Prepared if has(ReleaseAction::Prepare) => {
                        (true, "this promotion prepares it".to_string())
                    }
                    ReleaseCheck::MainAtRelease if has(ReleaseAction::MoveMain) => (
                        true,
                        format!("this promotion fast-forwards {main} to {branch}"),
                    ),
                    _ => (
                        false,
                        "needs the code repository, which is not here (the monitor reads it only \
                         when started in the project folder)"
                            .to_string(),
                    ),
                };
                let fix = if holds {
                    String::new()
                } else if *c == ReleaseCheck::ItemsFinished {
                    carried_fix(&preview)
                } else {
                    needs_repo_text(version, &stage)
                };
                conditions.push(PlanCondition {
                    check: *c,
                    stage: s.clone(),
                    holds,
                    detail,
                    fix,
                });
            }
        }
        Some(repo) => {
            let branch_exists = coderepo::branch_exists(repo, &branch);
            if branch_exists {
                release_tip = coderepo::rev_parse(repo, &branch).unwrap_or_default();
            }
            let mut branch_made_here = false;
            // 1. The branch.
            if has(ReleaseAction::Branch) {
                let recorded = release.branch == branch && branch_exists;
                if recorded {
                    steps.push(step(
                        ReleaseAction::Branch,
                        format!("{branch} already exists"),
                        true,
                        None,
                    ));
                } else if branch_exists {
                    let why = format!(
                        "{branch} already exists in {} — nothing was changed",
                        repo.display()
                    );
                    blocks.push(why.clone());
                    steps.push(step(
                        ReleaseAction::Branch,
                        format!("makes {branch}"),
                        false,
                        Some(why),
                    ));
                } else {
                    // With the git model on, from the line its topology says (FEAT-189); else from
                    // the integration branch or the default branch, as before.
                    let from_line: std::result::Result<String, String> = if cadence.git_model {
                        crate::gitmodel::load(store, id, repo)
                            .map(|(model, facts)| {
                                let sprints = crate::sprints::load(store, id).unwrap_or_default();
                                let base = crate::gitmodel::release_base(&model, &sprints, &facts);
                                // A line not made yet is made from main, as `start` makes it
                                // (FEAT-190).
                                if !facts.branches.contains_key(&base) && base != model.main {
                                    make_line = Some((base.clone(), model.main.clone()));
                                }
                                base
                            })
                            .map_err(|e| e.to_string())
                    } else {
                        Ok(cadence
                            .integration_branch
                            .clone()
                            .unwrap_or_else(|| coderepo::default_branch(repo)))
                    };
                    match from_line {
                        Ok(b) => {
                            base = b;
                            let made_from = match &make_line {
                                Some((line, from)) => {
                                    format!("{line} (made from {from} first)")
                                }
                                None => base.clone(),
                            };
                            let at = match &make_line {
                                Some((_, from)) => from.clone(),
                                None => base.clone(),
                            };
                            match coderepo::rev_parse(repo, &at) {
                                Ok(sha) => {
                                    branch_made_here = true;
                                    steps.push(step(
                                        ReleaseAction::Branch,
                                        format!("makes {branch} from {made_from}"),
                                        false,
                                        None,
                                    ));
                                    // The tip the later steps see is where the branch will start.
                                    release_tip = sha;
                                }
                                Err(e) => {
                                    let why = e.to_string();
                                    blocks.push(why.clone());
                                    steps.push(step(
                                        ReleaseAction::Branch,
                                        format!("makes {branch} from {made_from}"),
                                        false,
                                        Some(why),
                                    ));
                                }
                            }
                        }
                        Err(why) => {
                            blocks.push(why.clone());
                            steps.push(step(
                                ReleaseAction::Branch,
                                format!("makes {branch}"),
                                false,
                                Some(why),
                            ));
                        }
                    }
                }
            }
            // The tip the steps after the branch work from: the branch's, or where it will start.
            let tip_before: Option<String> = (!release_tip.is_empty()).then(|| release_tip.clone());
            if branch_made_here {
                // `release_tip` names the branch as it is now, which is not yet at all.
                release_tip.clear();
            }
            let will_have_branch = branch_exists || branch_made_here;

            // 2. The preparation.
            let mut prepares = false;
            if has(ReleaseAction::Prepare) {
                let cfg = &cadence.release_prepare;
                if cfg.writes_nothing() {
                    steps.push(step(
                        ReleaseAction::Prepare,
                        "nothing to prepare: no version files configured: `kanbanr config \
                         release-prepare --version-file <path>`"
                            .to_string(),
                        true,
                        None,
                    ));
                } else {
                    let worked = match (&tip_before, coderepo::worktree_of(repo, &branch)) {
                        (None, _) => Err(format!("{branch} does not exist to prepare")),
                        (Some(_), Some(wt)) if branch_exists => Err(format!(
                            "{branch} is checked out in {}: kanbanr commits the release \
                             preparation without checking anything out, so switch that worktree \
                             to another branch first",
                            wt.display()
                        )),
                        (Some(tip), _) => work_prepare(
                            store,
                            id,
                            &project,
                            &releases,
                            index,
                            repo,
                            tip,
                            &branch,
                            &format!("kanbanr release promote {version} {stage}"),
                        ),
                    };
                    match worked {
                        Ok(w) if w.files.is_empty() => {
                            steps.push(step(ReleaseAction::Prepare, w.does.clone(), true, None));
                        }
                        Ok(w) => {
                            prepares = true;
                            steps.push(step(ReleaseAction::Prepare, w.does.clone(), false, None));
                            prep = Some(w);
                        }
                        Err(why) => {
                            let why = format!("{why} — nothing was changed");
                            blocks.push(why.clone());
                            steps.push(step(
                                ReleaseAction::Prepare,
                                format!("commits the release preparation to {branch}"),
                                false,
                                Some(why),
                            ));
                        }
                    }
                }
            }

            // 3 and 4. Main and the tag, refused in the order the cut always checked them.
            let main_exists = coderepo::branch_exists(repo, &main);
            let main_rev = coderepo::rev_parse(repo, &main).ok();
            let main_at_tip = !prepares && main_rev.is_some() && main_rev == tip_before;
            let main_in_release = tip_before
                .as_deref()
                .is_some_and(|t| main_exists && coderepo::is_ancestor(repo, &main, t));
            let left_behind = patch && tip_before.is_some() && !main_in_release;
            let mut move_block: Option<String> = None;
            let mut tag_block: Option<String> = None;
            let moves = has(ReleaseAction::MoveMain);
            let tags = has(ReleaseAction::Tag);
            if moves && !main_at_tip && !coderepo::is_clean(repo) {
                let why = format!(
                    "{} has uncommitted changes: commit or stash them first — nothing was changed",
                    repo.display()
                );
                blocks.push(why.clone());
                move_block = Some(why);
            }
            if (moves || tags) && !will_have_branch {
                let why = format!(
                    "{branch} does not exist in {}: promote {version} to its first stage, which \
                     makes it — nothing was changed",
                    repo.display()
                );
                blocks.push(why.clone());
                move_block.get_or_insert(why.clone());
                tag_block.get_or_insert(why);
            }
            if tags && coderepo::tag_exists(repo, version) {
                let why = format!(
                    "the tag {version} already exists in {} — nothing was changed",
                    repo.display()
                );
                blocks.push(why.clone());
                tag_block.get_or_insert(why);
            }
            if moves && !main_exists {
                let why = format!(
                    "the main branch {main} does not exist in {} — nothing was changed",
                    repo.display()
                );
                blocks.push(why.clone());
                move_block.get_or_insert(why);
            }
            // main must fast-forward to the release, so its tip afterwards is exactly the release
            // (decided 19 Oct 2026). A release missing main's commits is refused, never merged here
            // — except a patch of an older release, which leaves main where it is (FEAT-184, R-5).
            if moves && main_exists && !patch && !main_in_release && will_have_branch {
                let missing = tip_before
                    .as_deref()
                    .and_then(|t| coderepo::log_between(repo, t, &main).ok())
                    .unwrap_or_default();
                let why = format!(
                    "{main} has commits {branch} lacks: merge {main} into the release branch first \
                     — nothing was changed{}",
                    if missing.is_empty() {
                        String::new()
                    } else {
                        format!("\n  {}", missing.join("\n  "))
                    }
                );
                blocks.push(why.clone());
                move_block.get_or_insert(why);
            }
            // The tag names main's commit when main moved at an earlier stage (FEAT-212): main
            // must still be at the release tip.
            let earlier_move = gates[..target.min(gates.len())]
                .iter()
                .any(|(_, g)| g.on_enter.contains(&ReleaseAction::MoveMain));
            if tags && !moves && earlier_move && !left_behind && will_have_branch && !main_at_tip {
                let why = if main_in_release || !main_exists {
                    format!(
                        "{main} is not at {branch}: fast-forward it (`git merge --ff-only \
                         {branch}` on {main}) — nothing was changed"
                    )
                } else {
                    format!(
                        "{main} has commits {branch} lacks: merge {main} into the release branch \
                         first — nothing was changed"
                    )
                };
                blocks.push(why.clone());
                tag_block.get_or_insert(why);
            }
            if moves {
                let (does, noop) = if main_at_tip {
                    (format!("{main} is already at {branch}"), true)
                } else if left_behind {
                    (
                        format!(
                            "{main} has moved on, so this patch of an older release leaves it \
                             where it is"
                        ),
                        true,
                    )
                } else if prepares {
                    (
                        format!("fast-forwards {main} to {branch} after its preparation commit"),
                        false,
                    )
                } else {
                    (
                        format!(
                            "fast-forwards {main} to {branch} ({})",
                            short(tip_before.as_deref().unwrap_or_default())
                        ),
                        false,
                    )
                };
                steps.push(step(ReleaseAction::MoveMain, does, noop, move_block));
            }
            if tags {
                let does = if prepares {
                    format!("tags the preparation commit \"Release {version}\" as {version}")
                } else {
                    format!(
                        "tags {} \"Release {version}\" as {version}",
                        short(tip_before.as_deref().unwrap_or_default())
                    )
                };
                steps.push(step(ReleaseAction::Tag, does, false, tag_block));
            }

            // The conditions, against the repository as it is before the move.
            let pushed_in_play = requires.iter().any(|(c, _)| *c == ReleaseCheck::Pushed);
            if pushed_in_play {
                let remote = coderepo::remote_of(repo, &main);
                let (fetched, fetch_error) = match fetch {
                    Fetch::Try => match coderepo::fetch_branch(repo, &remote, &main) {
                        Ok(()) => (true, String::new()),
                        Err(e) => (false, e.to_string()),
                    },
                    Fetch::No => (false, String::new()),
                };
                origin = Some(OriginState {
                    tracking: coderepo::upstream_of(repo, &main)
                        .unwrap_or_else(|| format!("{remote}/{main}")),
                    fetched,
                    fetch_error,
                });
            }
            let move_stage = stage_holding(ReleaseAction::MoveMain);
            for (c, s) in &requires {
                let (holds, detail, fix) = match c {
                    ReleaseCheck::ItemsFinished => {
                        let (holds, detail) = items_finished(&preview);
                        (holds, detail, carried_fix(&preview))
                    }
                    ReleaseCheck::Prepared => {
                        let fix = match stage_holding(ReleaseAction::Prepare) {
                            Some(p) => format!("promote {version} to {p}, which prepares it"),
                            None => "give a stage the prepare action: `kanbanr config \
                                     release-stage <stage> --on-enter prepare`"
                                .to_string(),
                        };
                        if cadence.release_prepare.writes_nothing() {
                            (true, "nothing is configured to prepare".to_string(), fix)
                        } else if has(ReleaseAction::Prepare) {
                            (true, "this promotion prepares it".to_string(), fix)
                        } else {
                            match &tip_before {
                                None => (false, format!("{branch} does not exist yet"), fix),
                                Some(tip) => {
                                    let (holds, detail) =
                                        prepared_at(repo, tip, &project, version, &branch);
                                    (holds, detail, fix)
                                }
                            }
                        }
                    }
                    ReleaseCheck::MainAtRelease => {
                        if moves {
                            (
                                true,
                                format!("this promotion fast-forwards {main} to {branch}"),
                                String::new(),
                            )
                        } else if main_at_tip {
                            (true, format!("{main} is at {branch}"), String::new())
                        } else if left_behind {
                            (
                                true,
                                format!("{main} has moved on; a patch leaves it where it is"),
                                String::new(),
                            )
                        } else if tip_before.is_some() && main_exists && !main_in_release {
                            let missing = tip_before
                                .as_deref()
                                .and_then(|t| coderepo::log_between(repo, t, &main).ok())
                                .unwrap_or_default();
                            (
                                false,
                                format!("{main} has commits {branch} lacks"),
                                format!(
                                    "merge {main} into {branch} first{}",
                                    if missing.is_empty() {
                                        String::new()
                                    } else {
                                        format!(": {}", missing.join("; "))
                                    }
                                ),
                            )
                        } else {
                            (
                                false,
                                format!("{main} is not at {branch}"),
                                match &move_stage {
                                    Some(m) => format!(
                                        "promote {version} to {m}, which fast-forwards {main}"
                                    ),
                                    None => format!("fast-forward {main} to {branch}"),
                                },
                            )
                        }
                    }
                    ReleaseCheck::Pushed => {
                        let remote = coderepo::remote_of(repo, &main);
                        let fix =
                            format!("push {main} (`git push {remote} {main}`), then wait for CI");
                        let tracking = origin
                            .as_ref()
                            .map(|o| o.tracking.clone())
                            .unwrap_or_default();
                        if prepares {
                            (
                                false,
                                "the preparation commit this promotion makes is not pushed"
                                    .to_string(),
                                fix,
                            )
                        } else {
                            match &tip_before {
                                None => (false, format!("{branch} does not exist yet"), fix),
                                Some(tip) if left_behind => {
                                    let on = coderepo::remote_branches_containing(repo, tip)
                                        .unwrap_or_default();
                                    match on.first() {
                                        Some(r) => (true, format!("{} is on {r}", short(tip)), fix),
                                        None => (
                                            false,
                                            format!("{} is on no remote branch", short(tip)),
                                            format!("push {branch} (`git push {remote} {branch}`)"),
                                        ),
                                    }
                                }
                                Some(tip) => match coderepo::upstream_of(repo, &main) {
                                    Some(up) if coderepo::is_ancestor(repo, tip, &up) => {
                                        (true, format!("{} is on {up}", short(tip)), fix)
                                    }
                                    Some(up) => {
                                        (false, format!("{} is not on {up}", short(tip)), fix)
                                    }
                                    None => (
                                        false,
                                        format!(
                                            "{main} has no remote-tracking branch ({tracking})"
                                        ),
                                        fix,
                                    ),
                                },
                            }
                        }
                    }
                };
                conditions.push(PlanCondition {
                    check: *c,
                    stage: s.clone(),
                    holds,
                    detail,
                    fix: if holds { String::new() } else { fix },
                });
            }
        }
    }

    let reason = reason.trim();
    let needs_reason = direction == Direction::Back && reason.is_empty();
    let mut missing: Vec<String> = Vec::new();
    if needs_reason {
        missing.push(format!(
            "moving {version} back from {from} to {stage} needs a reason (--reason)"
        ));
    }
    if direction == Direction::Again && steps.iter().all(|s| s.noop) {
        missing.push(format!("{version} is already at {stage}"));
    }
    for c in conditions.iter().filter(|c| !c.holds) {
        if !missing.contains(&c.fix) {
            missing.push(c.fix.clone());
        }
    }
    for b in &blocks {
        if !missing.contains(b) {
            missing.push(b.clone());
        }
    }
    let button_for = |i: usize| {
        if i == last {
            format!("Cut {version}")
        } else {
            format!("Promote to {}", stages[i])
        }
    };
    let button = match direction {
        Direction::Forward => button_for(target),
        Direction::Again => format!("Promote to {stage} again"),
        Direction::Back => format!("Move back to {stage}"),
    };
    let after = if direction == Direction::Back {
        String::new()
    } else if target == last {
        if has(ReleaseAction::Tag) {
            format!("push the tag: `git push origin {version}`")
        } else {
            String::new()
        }
    } else {
        let next = &gates[target + 1].1;
        let push = if next.requires.contains(&ReleaseCheck::Pushed) {
            format!("push {main}, wait for CI, then ")
        } else {
            "then ".to_string()
        };
        format!("{push}{}", button_for(target + 1))
    };
    let tag_commit = match (repo, has(ReleaseAction::Tag)) {
        (Some(repo), true) => {
            if prep.is_some() {
                Some(CommitLine {
                    sha: String::new(),
                    subject: format!(
                        "Release {version}: version, changelog and notes (made by this promotion)"
                    ),
                })
            } else {
                let at = if release_tip.is_empty() {
                    base.clone()
                } else {
                    release_tip.clone()
                };
                coderepo::git(repo, &["log", "-1", "--format=%H%x1f%s", &at])
                    .ok()
                    .and_then(|l| {
                        let (sha, subject) = l.split_once('\u{1f}')?;
                        Some(CommitLine {
                            sha: sha.to_string(),
                            subject: subject.to_string(),
                        })
                    })
            }
        }
        _ => None,
    };
    let plan = ReleasePlan {
        version: version.to_string(),
        from,
        to: stage.clone(),
        direction: match direction {
            Direction::Forward => "forward",
            Direction::Back => "back",
            Direction::Again => "again",
        }
        .to_string(),
        cut,
        button,
        ready: missing.is_empty(),
        missing,
        conditions,
        steps,
        preview: if cut { preview } else { Vec::new() },
        tag_commit,
        release_tip,
        needs_reason,
        origin,
        after,
        repo: repo.is_some(),
    };
    Ok(Worked {
        plan,
        releases,
        index,
        direction,
        stage,
        branch,
        base,
        make_line,
        main,
        prep,
        blocks,
        patch,
    })
}

fn items_finished(preview: &[CutPreview]) -> (bool, String) {
    let carried: Vec<&str> = preview
        .iter()
        .filter(|p| !p.ships)
        .map(|p| p.code.as_str())
        .collect();
    if carried.is_empty() {
        (
            true,
            format!("{} planned item(s), all finished", preview.len()),
        )
    } else {
        (false, format!("{} would carry over", carried.join(", ")))
    }
}

fn carried_fix(preview: &[CutPreview]) -> String {
    preview
        .iter()
        .filter(|p| !p.ships)
        .map(|p| {
            format!(
                "{} carries over: {}. Finish it, or plan it into another release",
                p.code, p.why
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Whether the release tip already carries what `prepare` writes (the `prepared` check).
fn prepared_at(
    repo: &Path,
    tip: &str,
    project: &Project,
    version: &str,
    branch: &str,
) -> (bool, String) {
    let cfg = &project.config.cadence.release_prepare;
    let ph = match crate::releaseprep::Placeholders::new(version, "", &today_ymd()) {
        Ok(p) => p,
        Err(e) => return (false, e),
    };
    let read = |path: &str| {
        coderepo::read_blob(repo, tip, path)
            .ok()
            .flatten()
            .map(|(_, t)| t)
    };
    let mut lacking = Vec::new();
    for f in &cfg.version_files {
        let ok = read(&f.path).is_some_and(|t| {
            crate::releaseprep::names_version(&t, &f.path, f.pattern.as_deref(), &ph.number)
        });
        if !ok {
            lacking.push(f.path.clone());
        }
    }
    if let Some(c) = &cfg.changelog
        && !read(c).is_some_and(|t| crate::releaseprep::has_heading(&t, &ph.number))
    {
        lacking.push(format!("{c} (## [{}])", ph.number));
    }
    if let Some(n) = &cfg.notes {
        let path = crate::releaseprep::expand(n, &ph);
        if read(&path).is_none() {
            lacking.push(path);
        }
    }
    if lacking.is_empty() {
        (true, format!("{branch} carries {}", ph.number))
    } else {
        (
            false,
            format!(
                "{branch} does not carry {} yet: {}",
                ph.number,
                lacking.join(", ")
            ),
        )
    }
}

/// What `prepare` would commit onto `branch` at `tip`, worked out without writing anything: the
/// files that change and the commit message. A missing file or a match that finds nothing is a
/// refusal naming the file.
#[allow(clippy::too_many_arguments)]
fn work_prepare(
    store: &Store,
    id: &str,
    project: &Project,
    releases: &[Release],
    index: usize,
    repo: &Path,
    tip: &str,
    branch: &str,
    command: &str,
) -> std::result::Result<PrepWork, String> {
    use crate::releaseprep as prep;
    let cfg = &project.config.cadence.release_prepare;
    let release = &releases[index];
    let ph = prep::Placeholders::new(&release.version, &release.name, &today_ymd())?;
    let mut files: Vec<(String, String)> = Vec::new();
    fn put(files: &mut Vec<(String, String)>, path: &str, content: String) {
        match files.iter_mut().find(|(p, _)| p == path) {
            Some(f) => f.1 = content,
            None => files.push((path.to_string(), content)),
        }
    }
    let read = |files: &Vec<(String, String)>, path: &str| -> std::result::Result<String, String> {
        prep::check_repo_path(path)?;
        if let Some((_, c)) = files.iter().find(|(p, _)| p == path) {
            return Ok(c.clone());
        }
        coderepo::read_blob(repo, tip, path)
            .map_err(|e| e.to_string())?
            .map(|(_, t)| t)
            .ok_or_else(|| format!("{path} does not exist on {branch}"))
    };
    let mut parts: Vec<String> = Vec::new();
    let mut notes_said: Vec<String> = Vec::new();
    let mut version_changed = false;
    for f in &cfg.version_files {
        let text = read(&files, &f.path)?;
        let set = prep::set_version(&text, &f.path, f.pattern.as_deref(), &ph.number)?;
        if set != text {
            version_changed = true;
            put(&mut files, &f.path, set);
        }
    }
    if version_changed {
        parts.push(format!("the version {}", ph.number));
    }
    if cfg.version_files.is_empty() {
        notes_said.push(
            "no version files configured: `kanbanr config release-prepare --version-file <path>`"
                .to_string(),
        );
    }
    if let Some(c) = &cfg.changelog {
        let text = read(&files, c)?;
        match prep::date_changelog(&text, &ph)? {
            Some(dated) => {
                for copy in &cfg.changelog_copies {
                    prep::check_repo_path(copy)?;
                    put(&mut files, copy, dated.clone());
                }
                put(&mut files, c, dated);
                parts.push("the changelog heading".to_string());
            }
            None if prep::has_heading(&text, &ph.number) => {
                notes_said.push(format!("{c} already has ## [{}]", ph.number));
            }
            None => notes_said.push(format!(
                "{c} has no ## [Unreleased] heading, so it is left as it is"
            )),
        }
    }
    let mut notes_path = String::new();
    let mut drafted_from: Vec<String> = Vec::new();
    if let Some(n) = &cfg.notes {
        let path = prep::expand(n, &ph);
        prep::check_repo_path(&path)?;
        let there = files.iter().any(|(p, _)| *p == path)
            || coderepo::read_blob(repo, tip, &path)
                .map_err(|e| e.to_string())?
                .is_some();
        if there {
            notes_said.push(format!("{path} is already written, so it is kept"));
        } else {
            let shipping_codes: Vec<String> = cut_preview(store, id, &release.version)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|p| p.ships)
                .map(|p| p.code)
                .collect();
            let mut shipping: Vec<FeatureItem> = project
                .features
                .iter()
                .filter(|f| shipping_codes.contains(&f.code))
                .cloned()
                .collect();
            shipping.sort_by(|a, b| a.code.cmp(&b.code));
            drafted_from = shipping.iter().map(|f| f.code.clone()).collect();
            let parent = releases
                .iter()
                .find(|r| !release.parent.is_empty() && r.version == release.parent);
            put(&mut files, &path, notes(release, parent, &shipping, &[]));
            parts.push("draft notes".to_string());
            notes_path = path;
        }
    }
    let mut listed = false;
    for l in &cfg.notes_listings {
        let path = prep::expand(&l.path, &ph);
        let text = read(&files, &path)?;
        if let Some(new) = prep::add_listing(
            &text,
            &prep::expand(&l.after, &ph),
            &prep::expand(&l.entry, &ph),
        )
        .map_err(|e| format!("{path} {e}"))?
        {
            listed = true;
            put(&mut files, &path, new);
        }
    }
    if listed && notes_path.is_empty() {
        parts.push("the notes listing".to_string());
    }
    let does = if files.is_empty() {
        let mut said = vec!["nothing to change: it is already prepared".to_string()];
        said.extend(notes_said);
        said.join("; ")
    } else {
        let joined = match parts.len() {
            0 => "its files".to_string(),
            1 => parts[0].clone(),
            n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
        };
        let mut s = format!("commits {joined} to {branch}");
        if !notes_said.is_empty() {
            s.push_str(&format!(" ({})", notes_said.join("; ")));
        }
        s
    };
    let mut message = format!(
        "Release {}: version, changelog and notes\n\nPrepared by `{command}`:\n",
        release.version
    );
    for (p, _) in &files {
        message.push_str(&format!("- {p}\n"));
    }
    if !notes_path.is_empty() {
        if drafted_from.is_empty() {
            message.push_str("Draft notes: nothing planned has finished yet.\n");
        } else {
            message.push_str(&format!("Draft notes from {}.\n", drafted_from.join(", ")));
        }
    }
    message.push_str(&format!(
        "\n[no-ref] release preparation, made by kanbanr\nKanbanr-Release: {}\n",
        release.version
    ));
    Ok(PrepWork {
        files,
        message,
        notes: notes_path,
        does,
    })
}

/// How [`promote_with`] runs.
#[derive(Debug, Clone, Copy)]
pub struct PromoteOptions<'a> {
    /// The release tip the caller's plan showed: if the branch has moved since, the promotion is
    /// refused.
    pub expect_tip: Option<&'a str>,
    pub fetch: Fetch,
}

/// A git step taken, and how to take it back.
enum Undo {
    DeleteBranch(String),
    RemoveCommit {
        branch: String,
        old: String,
        new: String,
    },
    ResetMain {
        branch: String,
        to: String,
    },
    DeleteTag(String),
}

/// Take back every step, newest first; the error that stopped the promotion says whether that
/// worked. With nothing taken, the error is as it was.
fn undo_all(repo: Option<&Path>, undo: Vec<Undo>, e: CoreError) -> CoreError {
    let Some(repo) = repo else {
        return e;
    };
    if undo.is_empty() {
        return e;
    }
    let mut first = None;
    for u in undo.into_iter().rev() {
        let r = match u {
            Undo::DeleteBranch(b) => coderepo::delete_branch(repo, &b),
            Undo::RemoveCommit { branch, old, new } => {
                coderepo::undo_commit(repo, &branch, &old, &new)
            }
            Undo::ResetMain { branch, to } => coderepo::reset_branch_to(repo, &branch, &to),
            Undo::DeleteTag(t) => coderepo::delete_tag(repo, &t),
        };
        if let Err(err) = r {
            first.get_or_insert(err);
        }
    }
    rolled_back(e, first)
}

/// Move a release to the next stage, or to the named one (FEAT-183), doing what the stages it
/// enters say (FEAT-212). See [`promote_with`]; this fetches main when `pushed` is in play and
/// does not compare the release tip with a plan.
pub fn promote(
    store: &Store,
    id: &str,
    version: &str,
    to: Option<&str>,
    by: &str,
    reason: &str,
    repo: Option<&Path>,
) -> Result<Promoted> {
    promote_with(
        store,
        id,
        version,
        to,
        by,
        reason,
        repo,
        PromoteOptions {
            expect_tip: None,
            fetch: Fetch::Try,
        },
    )
}

/// Move a release to the next stage, or to the named one (FEAT-183, FEAT-212).
///
/// A forward move takes the entry actions of every stage it passes or enters — `branch`,
/// `prepare`, `move_main`, `tag`, in that order — and needs every condition those stages require.
/// By default (no `cadence.release_gates`) the first stage makes the release branch and the last
/// fast-forwards main and tags the release, as before. The last stage is always the cut. A move to
/// an earlier stage needs a `reason` and is a board change only; nothing ever moves back from
/// production, and main never moves back. Promoting to the stage it is at takes that stage's
/// actions again, when any of them would change something.
///
/// **Nothing changes unless everything can**: the whole plan is worked out first and refused with
/// every missing condition if it is not ready; then the git steps; then the board — and if a step
/// or the board fails, every git step taken is undone, newest first.
#[allow(clippy::too_many_arguments)]
pub fn promote_with(
    store: &Store,
    id: &str,
    version: &str,
    to: Option<&str>,
    by: &str,
    reason: &str,
    repo: Option<&Path>,
    opts: PromoteOptions,
) -> Result<Promoted> {
    cadence_on(store, id)?;
    let by = by.trim();
    if by.is_empty() {
        return Err(CoreError::Unsupported(
            "a stage change must name who made it: pass `by` (the CLI defaults it to the board's \
             commit identity)"
                .into(),
        ));
    }
    let w = work_out(store, id, version, to, reason, repo, opts.fetch)?;
    if !w.plan.ready {
        return Err(refusal(&w));
    }
    if let Some(expect) = opts.expect_tip.map(str::trim).filter(|e| !e.is_empty())
        && expect != w.plan.release_tip
    {
        return Err(CoreError::Unsupported(format!(
            "{} moved since the plan you saw (now {}): look again",
            w.branch,
            if w.plan.release_tip.is_empty() {
                "gone"
            } else {
                short(&w.plan.release_tip)
            }
        )));
    }
    let taken: Vec<&PlanStep> = w.plan.steps.iter().filter(|s| !s.noop).collect();
    let reason = reason.trim();
    let change = StageChange {
        stage: w.stage.clone(),
        by: by.to_string(),
        at: crate::now_rfc3339(),
        reason: if w.direction == Direction::Again {
            let names: Vec<&str> = taken.iter().map(|s| s.action.name()).collect();
            let again = format!("again: {}", names.join(", "));
            if reason.is_empty() {
                again
            } else {
                format!("{again} — {reason}")
            }
        } else {
            reason.to_string()
        },
    };
    let mut releases = w.releases.clone();
    let index = w.index;
    let original = releases.clone();
    let mut done = Promoted {
        release: releases[index].clone(),
        from: w.plan.from.clone(),
        to: w.stage.clone(),
        branch_created: String::new(),
        tagged: String::new(),
        main_branch: String::new(),
        preview: Vec::new(),
        notes: None,
        forward_merge: Vec::new(),
        forward_into: String::new(),
        steps: Vec::new(),
        prepared: None,
        main_moved: None,
        next: String::new(),
        origin: w.plan.origin.clone(),
    };

    // Back a stage: a decision someone made, so the reason is required (FEAT-183, Q2). Board only.
    if w.direction == Direction::Back {
        let r = &mut releases[index];
        r.stage = w.stage.clone();
        r.stages.push(change);
        done.release = r.clone();
        save(store, id, &releases)?;
        return Ok(done);
    }

    // The git steps, in order, each with its undo.
    let mut undo: Vec<Undo> = Vec::new();
    for step in &taken {
        let Some(repo) = repo else {
            return Err(needs_repo(version, &w.stage));
        };
        let r: Result<()> = match step.action {
            ReleaseAction::Branch => (|| {
                if let Some((line, from)) = &w.make_line {
                    coderepo::create_branch_from(repo, line, from)?;
                }
                coderepo::create_branch_from(repo, &w.branch, &w.base)?;
                undo.push(Undo::DeleteBranch(w.branch.clone()));
                done.branch_created = w.branch.clone();
                releases[index].branch = w.branch.clone();
                Ok(())
            })(),
            ReleaseAction::Prepare => (|| {
                let prep = w
                    .prep
                    .as_ref()
                    .ok_or_else(|| CoreError::Unsupported("nothing to prepare".into()))?;
                let (old, new) =
                    coderepo::commit_files(repo, &w.branch, &prep.files, &prep.message)?;
                undo.push(Undo::RemoveCommit {
                    branch: w.branch.clone(),
                    old,
                    new: new.clone(),
                });
                done.prepared = Some(Prepared {
                    sha: new,
                    files: prep.files.iter().map(|(p, _)| p.clone()).collect(),
                    notes: prep.notes.clone(),
                });
                Ok(())
            })(),
            ReleaseAction::MoveMain => (|| {
                let tip = coderepo::rev_parse(repo, &w.branch)?;
                let previous = coderepo::fast_forward(repo, &w.main, &tip)?;
                undo.push(Undo::ResetMain {
                    branch: w.main.clone(),
                    to: previous.clone(),
                });
                done.main_moved = Some((previous, tip));
                done.main_branch = w.main.clone();
                Ok(())
            })(),
            ReleaseAction::Tag => (|| {
                let tip = coderepo::rev_parse(repo, &w.branch)?;
                coderepo::create_tag(repo, version, &tip, &format!("Release {version}"))?;
                undo.push(Undo::DeleteTag(version.to_string()));
                done.tagged = version.to_string();
                Ok(())
            })(),
        };
        if let Err(e) = r {
            return Err(undo_all(Some(repo), undo, e));
        }
        done.steps.push((*step).clone());
    }

    // What the release has that its integration branch lacks, once main moved or it was tagged.
    let mut forward: Vec<CommitLine> = Vec::new();
    let mut integration = String::new();
    if let Some(repo) = repo
        && (done.main_moved.is_some() || !done.tagged.is_empty())
    {
        let project = store.load_meta(id)?;
        integration = integration_of(store, id, &project, repo, &w.main);
        let main_at_tip =
            coderepo::rev_parse(repo, &w.main).ok() == coderepo::rev_parse(repo, &w.branch).ok();
        if !(integration == w.main && main_at_tip) {
            forward = coderepo::commits_between(repo, &integration, &w.branch)
                .unwrap_or_default()
                .into_iter()
                .map(|c| CommitLine {
                    sha: c.sha,
                    subject: c.subject,
                })
                .collect();
        }
    }

    // The board.
    if w.plan.cut {
        // The cut reads the release from the board, so a branch just made is recorded first; if
        // the cut is refused, the branch and the record are both taken back.
        let mut saved = false;
        if !done.branch_created.is_empty() {
            if let Err(e) = save(store, id, &releases) {
                return Err(undo_all(repo, undo, e));
            }
            saved = true;
            done.release = releases[index].clone();
        }
        match cut_with(store, id, version, Some(change)) {
            Ok(cut) => {
                let mut notes = cut.notes;
                if w.patch && !forward.is_empty() {
                    notes.push_str(&format!("\n## To merge forward into {integration}\n\n"));
                    for c in &forward {
                        notes.push_str(&format!(
                            "- {} {}\n",
                            &c.sha[..c.sha.len().min(12)],
                            c.subject
                        ));
                    }
                    store.write_doc(id, &cut.release.notes_doc, &notes)?;
                }
                done.release = cut.release;
                done.notes = Some(notes);
                done.preview = w.plan.preview.clone();
            }
            Err(e) => {
                let err = undo_all(repo, undo, e);
                if saved {
                    let _ = save(store, id, &original);
                }
                return Err(err);
            }
        }
    } else {
        let r = &mut releases[index];
        r.stage = w.stage.clone();
        r.stages.push(change);
        done.release = r.clone();
        if let Err(e) = save(store, id, &releases) {
            return Err(undo_all(repo, undo, e));
        }
    }
    done.forward_merge = forward;
    done.forward_into = integration;
    done.next = next_hint(store, id, version, repo, &done, &w.main);
    Ok(done)
}

/// Why a plan that is not ready is refused: a back move's reason or an "already at" as before;
/// every missing condition with its fix when a stage requires something; else the first step's
/// refusal, word for word as the cut always gave it.
fn refusal(w: &Worked) -> CoreError {
    let p = &w.plan;
    if p.needs_reason || (w.direction == Direction::Again && p.steps.iter().all(|s| s.noop)) {
        return CoreError::Unsupported(p.missing[0].clone());
    }
    let failing: Vec<&PlanCondition> = p.conditions.iter().filter(|c| !c.holds).collect();
    if failing.is_empty()
        && let Some(first) = w.blocks.first()
    {
        return CoreError::Unsupported(first.clone());
    }
    let mut out = format!(
        "{} cannot go to {} yet — nothing was changed:",
        p.version, p.to
    );
    for c in failing {
        out.push_str(&format!(
            "\n  - {} ({}): {} — {}",
            c.check.name(),
            c.stage,
            c.detail,
            c.fix
        ));
    }
    for b in &w.blocks {
        out.push_str(&format!("\n  - {b}"));
    }
    CoreError::Unsupported(out)
}

/// What to do after a promotion, in a sentence.
fn next_hint(
    store: &Store,
    id: &str,
    version: &str,
    repo: Option<&Path>,
    done: &Promoted,
    main: &str,
) -> String {
    let remote = repo.map_or_else(|| "origin".to_string(), |r| coderepo::remote_of(r, main));
    if done.release.state == ReleaseState::Shipped {
        return match (done.tagged.is_empty(), done.main_moved.is_some()) {
            (false, true) => {
                format!("push {main} and the tag: `git push {remote} {main} {version}`")
            }
            (false, false) => format!("push the tag: `git push {remote} {version}`"),
            _ => String::new(),
        };
    }
    match plan_promotion(store, id, version, None, "", repo, Fetch::No) {
        Ok(p) if p.ready => format!("next: `kanbanr release promote {version}` ({})", p.button),
        Ok(p) => format!(
            "next: {}, then {}",
            p.missing.first().cloned().unwrap_or_default(),
            p.button
        ),
        Err(_) => String::new(),
    }
}

/// The error that stopped a promotion, saying whether undoing the repository steps worked too.
fn rolled_back(e: CoreError, undo: Option<CoreError>) -> CoreError {
    match undo {
        None => CoreError::Unsupported(format!("{e} — the code repository was put back as it was")),
        Some(u) => CoreError::Unsupported(format!(
            "{e} — and putting the code repository back failed too: {u}"
        )),
    }
}

/// Release notes, from what each shipped item said it was for and what it had to do.
fn notes(
    release: &Release,
    parent: Option<&Release>,
    shipped: &[FeatureItem],
    carried: &[Carry],
) -> String {
    let mut out = format!("# {}", release.version);
    if !release.name.is_empty() {
        out.push_str(&format!(" — {}", release.name));
    }
    out.push_str(&format!(
        "\n\n_Released {}._\n\n",
        crate::gantt::fmt_date(crate::sprints::today())
    ));
    // A patch says what it patches and what it adds to it (FEAT-184).
    match parent {
        Some(p) => {
            out.push_str(&format!(
                "Patch of {}.\n\n## What this patch adds\n\n",
                p.version
            ));
        }
        None => out.push_str("## What shipped\n\n"),
    }
    if shipped.is_empty() {
        out.push_str("Nothing planned for this release was finished.\n");
    }
    for f in shipped {
        out.push_str(&format!("### {} — {}\n\n", f.code, f.title));
        if let Some(def) = &f.definition {
            if !def.statement.trim().is_empty() {
                out.push_str(&format!("{}\n\n", def.statement.trim()));
            }
            for r in &def.requirements {
                out.push_str(&format!("- {}\n", r.text.trim()));
            }
            if !def.requirements.is_empty() {
                out.push('\n');
            }
        }
    }
    if let Some(p) = parent {
        out.push_str(&format!("## Included from {}\n\n", p.version));
        if p.shipped.is_empty() {
            out.push_str("Nothing shipped in it.\n");
        }
        for code in &p.shipped {
            out.push_str(&format!("- {code}\n"));
        }
        out.push('\n');
    }
    if !carried.is_empty() {
        out.push_str("## Carried over\n\n");
        for c in carried {
            out.push_str(&format!("- {} → {} ({})\n", c.code, c.to, c.why));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::{ProjectConfig, Store};

    /// FEAT-184 R-2: a patch's notes say what it patches, what it adds, and what it keeps from
    /// its parent.
    #[test]
    fn patch_release_notes_show_parent_and_additions() {
        let parent = super::Release {
            version: "v1.0.0".into(),
            shipped: vec!["FEAT-001".into(), "FEAT-002".into()],
            ..Default::default()
        };
        let patch = super::Release {
            version: "v1.0.1".into(),
            parent: "v1.0.0".into(),
            ..Default::default()
        };
        let fix: crate::FeatureItem = crate::yaml::from_str(
            "code: FEAT-007\ntitle: Cart loses items\nspecification: ''\nstatus: Completed\n\
             milestone: M\ncreated_at: '2026-10-19T00:00:00Z'\nupdated_at: '2026-10-19T00:00:00Z'\n",
        )
        .unwrap();
        let text = super::notes(&patch, Some(&parent), &[fix], &[]);
        assert!(text.contains("Patch of v1.0.0."), "{text}");
        assert!(text.contains("## What this patch adds"), "{text}");
        assert!(text.contains("### FEAT-007 — Cart loses items"), "{text}");
        assert!(text.contains("## Included from v1.0.0"), "{text}");
        assert!(
            text.contains("- FEAT-001") && text.contains("- FEAT-002"),
            "{text}"
        );
        let plain = super::notes(&parent, None, &[], &[]);
        assert!(
            plain.contains("## What shipped") && !plain.contains("Patch of"),
            "{plain}"
        );
    }

    /// FEAT-138: a planned release's progress counts what is Done in a scrum project, as the sprint
    /// does — not only what has shipped.
    #[test]
    fn a_release_counts_done_items_as_finished() {
        let dir = std::env::temp_dir().join(format!("kanbanr-releases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let scrum = crate::config::preset("scrum").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = scrum.statuses;
        config.default_state = scrum.default_state;
        config.terminal_states = scrum.terminal_states;
        config.no_op_states = scrum.no_op_states;
        config.transitions = scrum.transitions;
        config.gates = scrum.gates;
        store.init_project("shop", config).unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let codes: Vec<String> = ["A", "B", "C"]
            .iter()
            .map(|t| store.add_feature("shop", t, "", "M", None).unwrap().code)
            .collect();
        super::add(&store, "shop", "v0.2.0", "", "").unwrap();
        super::plan(&store, "shop", "v0.2.0", &codes).unwrap();
        for stage in ["Ready", "In Progress", "Review", "Testing", "Done"] {
            store.move_feature("shop", &codes[0], stage).unwrap();
        }
        let reports = super::report_all(&store, "shop").unwrap();
        let r = reports
            .iter()
            .find(|r| r.release.version == "v0.2.0")
            .unwrap();
        assert_eq!(r.items.len(), 3);
        assert_eq!(r.finished, 1, "Done counts, as it does for the sprint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A board with releases on and v1 added, beside a scratch code repository on `main`.
    fn board_and_repo(tag: &str) -> (Store, std::path::PathBuf, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-promote-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("shop", ProjectConfig::default_for("shop"))
            .unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        super::add(&store, "shop", "v1", "", "").unwrap();
        let repo = crate::coderepo::tests::scratch_repo(&format!("promote-{tag}"));
        (store, dir, repo)
    }

    /// FEAT-183: the cut tags the release and fast-forwards main to exactly the tagged commit.
    #[test]
    fn promote_to_production_tags_and_fast_forwards_main() {
        use crate::coderepo::{self, tests::commit_file};
        let (store, dir, repo) = board_and_repo("prod");
        let promote =
            |to: &str| super::promote(&store, "shop", "v1", Some(to), "Ada", "", Some(&repo));
        let candidate = promote("candidate").unwrap();
        assert_eq!(candidate.branch_created, "release/v1");
        coderepo::git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "fix", "fixed\n", "fix on the release");
        coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();
        promote("staging").unwrap();
        let cut = promote("production").unwrap();
        assert_eq!(
            (cut.tagged.as_str(), cut.main_branch.as_str()),
            ("v1", "main")
        );
        assert_eq!(cut.release.state, super::ReleaseState::Shipped);
        let tip = coderepo::rev_parse(&repo, "release/v1").unwrap();
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), tip);
        assert_eq!(coderepo::rev_parse(&repo, "v1").unwrap(), tip);
        assert!(
            repo.join("fix").exists(),
            "the checked-out main moved with its files"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-183: a board step that fails after the tag and the fast-forward puts the repository
    /// back — no tag, main where it was — and the release stays where it was.
    #[test]
    fn a_failed_cut_rolls_back_the_tag_and_main() {
        use crate::coderepo::{self, tests::commit_file};
        let (store, dir, repo) = board_and_repo("rollback");
        super::promote(
            &store,
            "shop",
            "v1",
            Some("candidate"),
            "Ada",
            "",
            Some(&repo),
        )
        .unwrap();
        coderepo::git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "fix", "fixed\n", "fix on the release");
        coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        // The notes go to the board doc releases/v1.md: a folder of that name where the file
        // belongs makes the board step fail after the repository steps have run.
        store
            .write_doc("shop", "releases/v1.md/in-the-way.md", "x")
            .unwrap();

        let err = super::promote(
            &store,
            "shop",
            "v1",
            Some("production"),
            "Ada",
            "",
            Some(&repo),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("put back as it was"), "{err}");
        assert!(!coderepo::tag_exists(&repo, "v1"), "the tag was deleted");
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_before);
        assert!(!repo.join("fix").exists());
        assert!(coderepo::is_clean(&repo));
        let r = super::load(&store, "shop").unwrap();
        assert_eq!(
            (r[0].stage.as_str(), r[0].state),
            ("candidate", super::ReleaseState::Planned)
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    // ---- FEAT-212: release stages that act ----

    use crate::coderepo::{self, tests::commit_file};
    use std::path::{Path, PathBuf};

    /// Set the release stages, gates and preparation from a cadence fragment.
    fn configure(store: &Store, yaml: &str) {
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        let given: crate::config::Cadence = crate::yaml::from_str(yaml).unwrap();
        c.release_stages = given.release_stages;
        c.release_gates = given.release_gates;
        c.release_prepare = given.release_prepare;
        store.set_cadence("shop", None, c).unwrap();
    }

    /// The flow the user asked for: candidate makes the branch, main moves main, production tags.
    const THREE: &str = "release_stages: [candidate, main, production]\n\
         release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
         main: { requires: [items_finished], on_enter: [move_main] }\n  \
         production: { requires: [main_at_release, pushed], on_enter: [tag] }\n";

    /// A bare remote beside `repo`, as `origin`, with main pushed to it by plain git.
    fn with_remote(repo: &Path) -> PathBuf {
        let bare = repo.with_extension("remote.git");
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        coderepo::git(&bare, &["init", "-q", "--bare"]).unwrap();
        coderepo::git(repo, &["remote", "add", "origin", &bare.to_string_lossy()]).unwrap();
        plain_git(repo, &["push", "-q", "origin", "main"]);
        bare
    }

    /// git run by the test itself — kanbanr's own runner never pushes.
    fn plain_git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn on_release(repo: &Path, branch: &str, file: &str, text: &str) {
        coderepo::git(repo, &["checkout", "-q", branch]).unwrap();
        commit_file(repo, file, text, "fix on the release");
        coderepo::git(repo, &["checkout", "-q", "main"]).unwrap();
    }

    /// FEAT-212 R-4: a condition an action of the same promotion makes true counts as met —
    /// `prepare` makes `prepared`, `move_main` makes `main_at_release` — but nothing makes `pushed`
    /// true, so skipping to the cut still waits for the push. Planning changes nothing.
    #[test]
    fn condition_met_by_same_promotion_counts() {
        let (store, dir, repo) = board_and_repo("same-move");
        commit_file(
            &repo,
            "Cargo.toml",
            "[package]\nversion = \"0.1.0\"\n",
            "manifest",
        );
        configure(
            &store,
            "release_stages: [candidate, main, production]\n\
             release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
             main: { requires: [prepared, main_at_release], on_enter: [move_main] }\n  \
             production: { requires: [main_at_release, pushed], on_enter: [tag] }\n\
             release_prepare: { version_files: [{ path: Cargo.toml }] }\n",
        );
        let plan = super::plan_promotion(
            &store,
            "shop",
            "v1",
            Some("main"),
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        assert!(plan.ready, "{:#?}", plan.missing);
        assert_eq!(plan.button, "Promote to main");
        let held: Vec<(&str, bool)> = plan
            .conditions
            .iter()
            .map(|c| (c.check.name(), c.holds))
            .collect();
        assert_eq!(held, [("prepared", true), ("main_at_release", true)]);
        assert!(plan.conditions[0].detail.contains("this promotion"));
        let steps: Vec<&str> = plan.steps.iter().map(|s| s.action.name()).collect();
        assert_eq!(steps, ["branch", "prepare", "move_main"]);
        assert_eq!(plan.after, "push main, wait for CI, then Cut v1");

        // Straight to the cut: main_at_release is made by this move, pushed is not.
        let cut = super::plan_promotion(
            &store,
            "shop",
            "v1",
            Some("production"),
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        assert!(!cut.ready);
        assert_eq!(cut.button, "Cut v1");
        let pushed = cut
            .conditions
            .iter()
            .find(|c| c.check == crate::releasegates::ReleaseCheck::Pushed)
            .unwrap();
        assert!(!pushed.holds);
        assert!(pushed.fix.contains("push main"), "{}", pushed.fix);
        assert!(
            cut.conditions
                .iter()
                .any(|c| c.check.name() == "main_at_release" && c.holds)
        );
        assert!(
            !coderepo::branch_exists(&repo, "release/v1"),
            "a plan changes nothing"
        );
        assert!(super::load(&store, "shop").unwrap()[0].stage.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-5: entering a stage with `move_main` fast-forwards main to the release tip and
    /// tags nothing; a release missing main's commits is refused and nothing changes.
    #[test]
    fn move_main_stage_fast_forwards_without_tagging() {
        let (store, dir, repo) = board_and_repo("move-main");
        configure(&store, THREE);
        let promote =
            |v: &str, to: &str| super::promote(&store, "shop", v, Some(to), "Ada", "", Some(&repo));
        promote("v1", "candidate").unwrap();
        on_release(&repo, "release/v1", "fix", "fixed\n");
        let moved = promote("v1", "main").unwrap();
        let tip = coderepo::rev_parse(&repo, "release/v1").unwrap();
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), tip);
        assert!(
            repo.join("fix").exists(),
            "the checked-out main moved with its files"
        );
        assert!(!coderepo::tag_exists(&repo, "v1"), "no tag at main");
        assert_eq!(moved.tagged, "");
        assert_eq!(moved.main_branch, "main");
        assert_eq!(
            moved.main_moved.as_ref().map(|m| m.1.as_str()),
            Some(tip.as_str())
        );
        assert_eq!(moved.release.stage, "main");
        assert_eq!(moved.release.state, super::ReleaseState::Planned);
        assert!(
            moved
                .next
                .contains("push main (`git push origin main`), then wait for CI")
                && moved.next.ends_with("then Cut v1"),
            "{}",
            moved.next
        );

        // Main with commits the release lacks: refused, nothing moves.
        super::add(&store, "shop", "v2", "", "").unwrap();
        promote("v2", "candidate").unwrap();
        on_release(&repo, "release/v2", "two", "two\n");
        commit_file(&repo, "hotfix", "on main\n", "hotfix on main");
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        let err = promote("v2", "main").unwrap_err().to_string();
        assert!(err.contains("main has commits release/v2 lacks"), "{err}");
        assert!(err.contains("nothing was changed"), "{err}");
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_before);
        let r = super::load(&store, "shop").unwrap();
        assert_eq!(r[1].stage, "candidate");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-6: the cut tags only once main is at the release tip and that commit is on
    /// origin/main; until then it is refused with the push to make, and nothing is tagged. kanbanr
    /// never pushes: the remote has no tag afterwards.
    #[test]
    fn cut_tags_main_only_when_pushed() {
        let (store, dir, repo) = board_and_repo("cut-pushed");
        let bare = with_remote(&repo);
        configure(&store, THREE);
        let promote =
            |to: &str| super::promote(&store, "shop", "v1", Some(to), "Ada", "", Some(&repo));
        promote("candidate").unwrap();
        on_release(&repo, "release/v1", "fix", "fixed\n");
        promote("main").unwrap();
        let err = promote("production").unwrap_err().to_string();
        assert!(
            err.contains("pushed (production)")
                && err.contains("push main (`git push origin main`), then wait for CI"),
            "{err}"
        );
        assert!(err.contains("nothing was changed"), "{err}");
        assert!(!coderepo::tag_exists(&repo, "v1"));
        assert_eq!(super::load(&store, "shop").unwrap()[0].stage, "main");

        plain_git(&repo, &["push", "-q", "origin", "main"]);
        let plan = super::plan_promotion(
            &store,
            "shop",
            "v1",
            None,
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        assert!(plan.ready, "{:#?}", plan.missing);
        let tip = coderepo::rev_parse(&repo, "main").unwrap();
        assert_eq!(
            plan.tag_commit.as_ref().map(|c| c.sha.as_str()),
            Some(tip.as_str())
        );
        let cut = promote("production").unwrap();
        assert_eq!(cut.tagged, "v1");
        assert_eq!(cut.main_branch, "", "main was already there");
        assert_eq!(
            coderepo::rev_parse(&repo, "v1").unwrap(),
            tip,
            "main's tip is tagged"
        );
        assert_eq!(cut.release.state, super::ReleaseState::Shipped);
        assert_eq!(cut.next, "push the tag: `git push origin v1`");
        assert_eq!(
            plain_git(&repo, &["ls-remote", "--tags", &bare.to_string_lossy()]),
            "",
            "nothing was pushed"
        );
        for d in [&dir, &repo, &bare] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// FEAT-212 R-7: applying fetches main first, so a push made elsewhere counts; with the remote
    /// out of reach it judges from the last-fetched refs and says the fetch failed.
    #[test]
    fn promote_fetches_and_works_offline() {
        let (store, dir, repo) = board_and_repo("fetch");
        let bare = with_remote(&repo);
        configure(&store, THREE);
        let promote =
            |v: &str, to: &str| super::promote(&store, "shop", v, Some(to), "Ada", "", Some(&repo));
        promote("v1", "candidate").unwrap();
        on_release(&repo, "release/v1", "fix", "fixed\n");
        promote("v1", "main").unwrap();
        // Pushed by path, so this clone's origin/main does not know yet.
        plain_git(&repo, &["push", "-q", &bare.to_string_lossy(), "main:main"]);
        let stale = super::plan_promotion(
            &store,
            "shop",
            "v1",
            None,
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        assert!(
            !stale.ready,
            "the remote-tracking ref has not seen the push"
        );
        assert_eq!(stale.origin.as_ref().map(|o| o.fetched), Some(false));
        let cut = promote("v1", "production").unwrap();
        assert_eq!(cut.tagged, "v1");
        let origin = cut.origin.unwrap();
        assert!(
            origin.fetched && origin.fetch_error.is_empty(),
            "{origin:?}"
        );
        assert_eq!(origin.tracking, "origin/main");

        // Offline: the fetch fails, the last-fetched refs decide.
        super::add(&store, "shop", "v2", "", "").unwrap();
        promote("v2", "candidate").unwrap();
        on_release(&repo, "release/v2", "two", "two\n");
        promote("v2", "main").unwrap();
        plain_git(&repo, &["push", "-q", "origin", "main"]);
        plain_git(
            &repo,
            &[
                "remote",
                "set-url",
                "origin",
                "/nonexistent/kanbanr-offline.git",
            ],
        );
        let cut = promote("v2", "production").unwrap();
        assert_eq!(cut.tagged, "v2");
        let origin = cut.origin.unwrap();
        assert!(
            !origin.fetched && !origin.fetch_error.is_empty(),
            "{origin:?}"
        );
        for d in [&dir, &repo, &bare] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// The files a prepared release is made from, committed on main.
    fn prepare_fixture(repo: &Path) {
        std::fs::create_dir_all(repo.join("web")).unwrap();
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            repo.join("web/package.json"),
            "{\n  \"name\": \"w\",\n  \"version\": \"0.1.0\"\n}\n",
        )
        .unwrap();
        let changelog = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Gates.\n\n\
                         [Unreleased]: https://example.com/r/compare/v0.1.0...HEAD\n";
        std::fs::write(repo.join("CHANGELOG.md"), changelog).unwrap();
        std::fs::write(repo.join("docs/changelog.md"), changelog).unwrap();
        std::fs::write(
            repo.join("docs/SUMMARY.md"),
            "- [Intro](intro.md)\n- [Release notes](releases/index.md)\n",
        )
        .unwrap();
        coderepo::git(repo, &["add", "-A"]).unwrap();
        coderepo::git(repo, &["commit", "-q", "-m", "files"]).unwrap();
    }

    const PREPARE: &str = "release_prepare:\n  version_files:\n    - path: Cargo.toml\n    \
         - path: web/package.json\n  changelog: CHANGELOG.md\n  changelog_copies: [docs/changelog.md]\n  \
         notes: docs/releases/{version}.md\n  notes_listings:\n    \
         - { path: docs/SUMMARY.md, after: '- [Release notes](releases/index.md)', entry: '  - [{number}](releases/{version}.md)' }\n";

    /// FEAT-212 R-8: entering the stage with `prepare` makes one commit on release/<V> with the
    /// version files, the dated changelog heading and its copy, the notes draft and its listing —
    /// nothing checked out, the working tree as it was — carrying `[no-ref]` and no `Refs`.
    #[test]
    fn prepare_commits_version_changelog_and_notes() {
        let (store, dir, repo) = board_and_repo("prepare");
        prepare_fixture(&repo);
        super::add(&store, "shop", "v0.2.0", "", "Gates").unwrap();
        configure(&store, &format!("{THREE}{PREPARE}"));
        let plan = super::plan_promotion(
            &store,
            "shop",
            "v0.2.0",
            None,
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        let prep = plan
            .steps
            .iter()
            .find(|s| s.action.name() == "prepare")
            .unwrap();
        assert_eq!(
            prep.does,
            "commits the version 0.2.0, the changelog heading and draft notes to release/v0.2.0"
        );
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        let done = super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        assert_eq!(done.branch_created, "release/v0.2.0");
        let prepared = done.prepared.clone().unwrap();
        assert_eq!(
            prepared.files,
            [
                "Cargo.toml",
                "web/package.json",
                "docs/changelog.md",
                "CHANGELOG.md",
                "docs/releases/v0.2.0.md",
                "docs/SUMMARY.md"
            ]
        );
        assert_eq!(prepared.notes, "docs/releases/v0.2.0.md");
        let tip = coderepo::rev_parse(&repo, "release/v0.2.0").unwrap();
        assert_eq!(tip, prepared.sha);
        assert_eq!(
            coderepo::git(
                &repo,
                &["rev-list", "--count", &format!("{main_before}..{tip}")]
            )
            .unwrap(),
            "1",
            "one commit"
        );
        let at = |path: &str| coderepo::read_blob(&repo, &tip, path).unwrap().unwrap().1;
        assert!(at("Cargo.toml").contains("version = \"0.2.0\""));
        assert!(at("web/package.json").contains("\"version\": \"0.2.0\""));
        let changelog = at("CHANGELOG.md");
        assert!(
            changelog.contains("## [Unreleased]\n\n## [0.2.0] - "),
            "{changelog}"
        );
        assert!(
            changelog.contains(
                "[Unreleased]: https://example.com/r/compare/v0.2.0...HEAD\n\
                 [0.2.0]: https://example.com/r/compare/v0.1.0...v0.2.0"
            ),
            "{changelog}"
        );
        assert_eq!(at("docs/changelog.md"), changelog);
        assert!(at("docs/releases/v0.2.0.md").starts_with("# v0.2.0 — Gates"));
        assert!(
            at("docs/SUMMARY.md").contains(
                "- [Release notes](releases/index.md)\n  - [0.2.0](releases/v0.2.0.md)\n"
            )
        );
        let message = coderepo::git(&repo, &["log", "-1", "--format=%B", &tip]).unwrap();
        assert!(
            message.starts_with("Release v0.2.0: version, changelog and notes"),
            "{message}"
        );
        assert!(message.contains("Prepared by `kanbanr release promote v0.2.0 candidate`:"));
        assert!(message.contains("Kanbanr-Release: v0.2.0"));
        assert!(crate::scm::escape_reason(&message).is_some(), "{message}");
        assert!(!message.contains("Refs:"));
        // Nothing checked out moved.
        assert_eq!(coderepo::current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_before);
        assert!(
            std::fs::read_to_string(repo.join("Cargo.toml"))
                .unwrap()
                .contains("0.1.0")
        );
        assert!(coderepo::is_clean(&repo));
        // Prepared once: entering the stage again has nothing left to do.
        let err = super::promote(
            &store,
            "shop",
            "v0.2.0",
            Some("candidate"),
            "Ada",
            "",
            Some(&repo),
        )
        .unwrap_err()
        .to_string();
        assert!(err.ends_with("v0.2.0 is already at candidate"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-9: a version file that is missing, or whose match finds nothing, refuses the
    /// promotion, naming the file, and changes nothing — not even the branch.
    #[test]
    fn prepare_refuses_unmatched_version_file() {
        let (store, dir, repo) = board_and_repo("prepare-refused");
        commit_file(
            &repo,
            "Cargo.toml",
            "[package]\nname = \"x\"\n",
            "no version",
        );
        configure(
            &store,
            &format!("{THREE}release_prepare: {{ version_files: [{{ path: Cargo.toml }}] }}\n"),
        );
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Cargo.toml") && err.contains("matches nothing"),
            "{err}"
        );
        assert!(err.contains("nothing was changed"), "{err}");
        assert!(!coderepo::branch_exists(&repo, "release/v1"));
        assert!(super::load(&store, "shop").unwrap()[0].stage.is_empty());

        configure(
            &store,
            &format!("{THREE}release_prepare: {{ version_files: [{{ path: absent.toml }}] }}\n"),
        );
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("absent.toml does not exist on release/v1"),
            "{err}"
        );
        assert!(!coderepo::branch_exists(&repo, "release/v1"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-10: with nothing configured to prepare, `prepare` bumps nothing, the plan says
    /// so, and no commit is made.
    #[test]
    fn prepare_with_nothing_configured_makes_no_commit() {
        let (store, dir, repo) = board_and_repo("prepare-nothing");
        configure(&store, THREE);
        let plan = super::plan_promotion(
            &store,
            "shop",
            "v1",
            None,
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        let prep = plan
            .steps
            .iter()
            .find(|s| s.action.name() == "prepare")
            .unwrap();
        assert!(prep.noop);
        assert!(
            prep.does.contains("no version files configured"),
            "{}",
            prep.does
        );
        let done = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo)).unwrap();
        assert!(done.prepared.is_none());
        assert_eq!(
            coderepo::rev_parse(&repo, "release/v1").unwrap(),
            coderepo::rev_parse(&repo, "main").unwrap(),
            "no commit"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-11: when the board step fails after every git step — branch, preparation,
    /// main, tag — each is undone, newest first: no branch, main and its files as they were, no
    /// tag, and the release where it was.
    #[test]
    fn failed_promotion_undoes_every_step() {
        let (store, dir, repo) = board_and_repo("undo-all");
        prepare_fixture(&repo);
        configure(
            &store,
            &format!(
                "release_stages: [ship]\nrelease_gates: {{ ship: {{ on_enter: [branch, prepare, \
                 move_main, tag] }} }}\n{PREPARE}"
            ),
        );
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        let releases_before = super::load(&store, "shop").unwrap();
        // The notes go to the board doc releases/v1.md: a folder of that name where the file
        // belongs makes the board step fail after the repository steps have run.
        store
            .write_doc("shop", "releases/v1.md/in-the-way.md", "x")
            .unwrap();
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(err.contains("put back as it was"), "{err}");
        assert!(
            !coderepo::branch_exists(&repo, "release/v1"),
            "the branch is gone"
        );
        assert!(!coderepo::tag_exists(&repo, "v1"), "the tag is gone");
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_before);
        assert!(
            std::fs::read_to_string(repo.join("Cargo.toml"))
                .unwrap()
                .contains("0.1.0")
        );
        assert!(!repo.join("docs/releases").exists());
        assert!(coderepo::is_clean(&repo));
        assert_eq!(super::load(&store, "shop").unwrap(), releases_before);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-14: a promotion carrying the release tip its caller's plan showed is refused if
    /// the branch has moved since.
    #[test]
    fn promote_refuses_when_release_moved_since_plan() {
        let (store, dir, repo) = board_and_repo("expect");
        super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo)).unwrap();
        let plan = super::plan_promotion(
            &store,
            "shop",
            "v1",
            Some("production"),
            "",
            Some(&repo),
            super::Fetch::No,
        )
        .unwrap();
        assert!(plan.ready && plan.cut, "{:#?}", plan.missing);
        on_release(&repo, "release/v1", "late", "late\n");
        let opts = super::PromoteOptions {
            expect_tip: Some(&plan.release_tip),
            fetch: super::Fetch::No,
        };
        let err = super::promote_with(
            &store,
            "shop",
            "v1",
            Some("production"),
            "Ada",
            "",
            Some(&repo),
            opts,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("release/v1 moved since the plan you saw"),
            "{err}"
        );
        assert!(!coderepo::tag_exists(&repo, "v1"));
        assert_eq!(super::load(&store, "shop").unwrap()[0].stage, "candidate");
        // With the tip it shows now, it goes.
        let now = coderepo::rev_parse(&repo, "release/v1").unwrap();
        let opts = super::PromoteOptions {
            expect_tip: Some(&now),
            fetch: super::Fetch::No,
        };
        super::promote_with(
            &store,
            "shop",
            "v1",
            Some("production"),
            "Ada",
            "",
            Some(&repo),
            opts,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-4: a promotion whose stage requires what is not there is refused with every
    /// missing condition and its fix, before anything changes.
    #[test]
    fn promotion_refused_lists_every_missing_condition() {
        let (store, dir, repo) = board_and_repo("missing");
        commit_file(
            &repo,
            "Cargo.toml",
            "[package]\nversion = \"0.1.0\"\n",
            "manifest",
        );
        configure(
            &store,
            "release_stages: [candidate, production]\n\
             release_gates:\n  production: { requires: [items_finished, prepared, main_at_release, \
             pushed], on_enter: [tag] }\n\
             release_prepare: { version_files: [{ path: Cargo.toml }] }\n",
        );
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let code = store
            .add_feature("shop", "Unfinished", "", "M", None)
            .unwrap()
            .code;
        super::plan(&store, "shop", "v1", std::slice::from_ref(&code)).unwrap();
        super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo)).unwrap();
        on_release(&repo, "release/v1", "fix", "fixed\n");
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("v1 cannot go to production yet — nothing was changed:"),
            "{err}"
        );
        for want in [
            format!("items_finished (production): {code} would carry over — {code} carries over:"),
            "Finish it, or plan it into another release".to_string(),
            "prepared (production): release/v1 does not carry 1 yet: Cargo.toml — give a stage \
             the prepare action"
                .to_string(),
            "main_at_release (production): main is not at release/v1 — fast-forward main to \
             release/v1"
                .to_string(),
            "pushed (production): main has no remote-tracking branch".to_string(),
            "push main (`git push origin main`), then wait for CI".to_string(),
        ] {
            assert!(err.contains(&want), "missing {want:?} in\n{err}");
        }
        assert!(!coderepo::tag_exists(&repo, "v1"));
        assert_eq!(super::load(&store, "shop").unwrap()[0].stage, "candidate");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
