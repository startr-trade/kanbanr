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
use crate::store::{Capability, Project, Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RELEASES_FILE: &str = "releases.yaml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    #[default]
    Planned,
    Shipped,
}

/// A planned item that did not ship, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carry {
    pub code: String,
    /// The release it moved to, or `unplanned`.
    pub to: String,
    /// Why it did not ship.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    pub extra: Extra,
}

/// One move of a release between stages: where to, who moved it, and when (FEAT-183). A move back
/// carries the reason it was made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, releases: &[Release]) -> Result<()> {
    std::fs::write(path(store, id), serde_yaml::to_string(releases)?)?;
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
    /// The main branch fast-forwarded to the release at the cut.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub main_branch: String,
    /// At the cut: what ships and what is carried over, as worked out before anything changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preview: Vec<CutPreview>,
    /// At the cut: the release notes written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// A patch at the cut: its commits the integration branch lacks (FEAT-184). kanbanr lists them;
    /// `kanbanr git forward` or the user merges them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forward_merge: Vec<CommitLine>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub forward_into: String,
}

/// One commit, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitLine {
    pub sha: String,
    pub subject: String,
}

fn needs_repo(version: &str, stage: &str) -> CoreError {
    CoreError::Unsupported(format!(
        "promoting {version} to {stage} needs the code repository: use the CLI from the project \
         folder"
    ))
}

/// Move a release to the next stage, or to the named one (FEAT-183).
///
/// A forward move goes later in `cadence.release_stages`. Entering the first stage makes the
/// release branch `release/<V>` from the integration branch; a middle stage is a board change
/// only; the last stage is the cut, which also tags the code repository with the version and
/// fast-forwards the main branch to the release. A move to an earlier stage needs a `reason` and
/// is a board change only; nothing ever moves back from production.
///
/// The first and last stages need the code repository (`repo`); without one they are refused.
/// For the cut, **nothing changes unless everything can**: every refusal that can be known is
/// checked first, then the tag and the fast-forward, then the board — and if the board step still
/// fails, the repository steps are undone before the error is returned.
pub fn promote(
    store: &Store,
    id: &str,
    version: &str,
    to: Option<&str>,
    by: &str,
    reason: &str,
    repo: Option<&Path>,
) -> Result<Promoted> {
    let project = cadence_on(store, id)?;
    let by = by.trim();
    if by.is_empty() {
        return Err(CoreError::Unsupported(
            "a stage change must name who made it: pass `by` (the CLI defaults it to the board's \
             commit identity)"
                .into(),
        ));
    }
    let stages = project.config.cadence.stages();
    let mut releases = load(store, id)?;
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
    if current == Some(target) {
        return Err(CoreError::Unsupported(format!(
            "{version} is already at {stage}"
        )));
    }
    let change = StageChange {
        stage: stage.clone(),
        by: by.to_string(),
        at: crate::now_rfc3339(),
        reason: reason.trim().to_string(),
    };
    let mut done = Promoted {
        release: release.clone(),
        from: from.clone(),
        to: stage.clone(),
        branch_created: String::new(),
        tagged: String::new(),
        main_branch: String::new(),
        preview: Vec::new(),
        notes: None,
        forward_merge: Vec::new(),
        forward_into: String::new(),
    };

    // Back a stage: a decision someone made, so the reason is required (FEAT-183, Q2).
    if current.is_some_and(|c| target < c) {
        if change.reason.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "moving {version} back from {from} to {stage} needs a reason (--reason)"
            )));
        }
        let r = &mut releases[index];
        r.stage = stage;
        r.stages.push(change);
        done.release = r.clone();
        save(store, id, &releases)?;
        return Ok(done);
    }

    // Every forward move needs the release's branch: the first stage makes it, and so does a move
    // that skipped the first stage, or a process whose one stage is also the cut (FEAT-184, R-4).
    let original = releases.clone();
    let mut made = None;
    if target == 0 || release.branch.is_empty() {
        let repo = repo.ok_or_else(|| needs_repo(version, &stage))?;
        let branch = format!("release/{version}");
        // Moved back to a stage before this one and forward again, it already has its branch.
        let recorded = release.branch == branch && coderepo::branch_exists(repo, &branch);
        if !recorded {
            if coderepo::branch_exists(repo, &branch) {
                return Err(CoreError::Unsupported(format!(
                    "{branch} already exists in {} — nothing was changed",
                    repo.display()
                )));
            }
            // With the git model on, from the line its topology says (FEAT-189); else from the
            // integration branch or the default branch, as before.
            let from_branch = if project.config.cadence.git_model {
                let (model, facts) = crate::gitmodel::load(store, id, repo)?;
                let sprints = crate::sprints::load(store, id)?;
                let base = crate::gitmodel::release_base(&model, &sprints, &facts);
                // A line not made yet is made from main, as `start` makes it (FEAT-190).
                if !facts.branches.contains_key(&base) && base != model.main {
                    coderepo::create_branch_from(repo, &base, &model.main)?;
                }
                base
            } else {
                project
                    .config
                    .cadence
                    .integration_branch
                    .clone()
                    .unwrap_or_else(|| coderepo::default_branch(repo))
            };
            coderepo::create_branch_from(repo, &branch, &from_branch)?;
            done.branch_created = branch.clone();
            made = Some((repo, branch.clone()));
        }
        releases[index].branch = branch;
    }
    if target == stages.len() - 1 {
        // The cut reads the release from the board, so a branch just made is recorded first; if
        // the cut is refused, the branch and the record are both taken back.
        if made.is_some() {
            save(store, id, &releases)?;
            done.release = releases[index].clone();
        }
        let cut = promote_to_production(store, id, version, change, repo, done);
        if cut.is_err()
            && let Some((repo, branch)) = made
        {
            let _ = coderepo::delete_branch(repo, &branch);
            let _ = save(store, id, &original);
        }
        return cut;
    }
    let r = &mut releases[index];
    r.stage = stage;
    r.stages.push(change);
    done.release = r.clone();
    if let Err(e) = save(store, id, &releases) {
        // The board did not record the branch, so it would be a stray: take it away again.
        if let Some((repo, branch)) = made {
            let _ = coderepo::git(repo, &["branch", "-D", &branch]);
        }
        return Err(e);
    }
    Ok(done)
}

/// The cut, with the code repository brought along (FEAT-183): preflight, tag, fast-forward main,
/// then the board — undoing the repository steps if the board step fails.
fn promote_to_production(
    store: &Store,
    id: &str,
    version: &str,
    change: StageChange,
    repo: Option<&Path>,
    mut done: Promoted,
) -> Result<Promoted> {
    let repo = repo.ok_or_else(|| needs_repo(version, &change.stage))?;
    let project = store.load_meta(id)?;

    // 1. Preflight: every refusal that can be known before anything changes (R-4).
    let preview = cut_preview(store, id, version)?;
    if !coderepo::is_clean(repo) {
        return Err(CoreError::Unsupported(format!(
            "{} has uncommitted changes: commit or stash them first — nothing was changed",
            repo.display()
        )));
    }
    let branch = if done.release.branch.is_empty() {
        format!("release/{version}")
    } else {
        done.release.branch.clone()
    };
    if !coderepo::branch_exists(repo, &branch) {
        return Err(CoreError::Unsupported(format!(
            "{branch} does not exist in {}: promote {version} to its first stage, which makes it \
             — nothing was changed",
            repo.display()
        )));
    }
    if coderepo::tag_exists(repo, version) {
        return Err(CoreError::Unsupported(format!(
            "the tag {version} already exists in {} — nothing was changed",
            repo.display()
        )));
    }
    let main = project
        .config
        .cadence
        .main_branch
        .clone()
        .unwrap_or_else(|| coderepo::default_branch(repo));
    if !coderepo::branch_exists(repo, &main) {
        return Err(CoreError::Unsupported(format!(
            "the main branch {main} does not exist in {} — nothing was changed",
            repo.display()
        )));
    }
    // main must fast-forward to the release, so its tip after the cut is exactly the tagged commit
    // (decided 19 Oct 2026). A release missing main's commits is refused, never merged here — except
    // a patch of an older release, which is tagged and leaves main where it is (FEAT-184, R-5).
    let patch = !done.release.parent.is_empty();
    let moves_main = coderepo::is_ancestor(repo, &main, &branch);
    if !moves_main && !patch {
        let missing = coderepo::log_between(repo, &branch, &main).unwrap_or_default();
        return Err(CoreError::Unsupported(format!(
            "{main} has commits {branch} lacks: merge {main} into the release branch first — \
             nothing was changed{}",
            if missing.is_empty() {
                String::new()
            } else {
                format!("\n  {}", missing.join("\n  "))
            }
        )));
    }

    // For a patch: its commits the integration branch lacks, to be merged forward (R-3). When the
    // integration branch is main and main moves to the patch, the move brings them.
    let integration = if project.config.cadence.git_model {
        crate::gitmodel::load(store, id, repo)
            .ok()
            .and_then(|(m, _)| m.integration_branch.or(m.line))
            .unwrap_or_else(|| main.clone())
    } else {
        project
            .config
            .cadence
            .integration_branch
            .clone()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| main.clone())
    };
    let forward = if patch && !(integration == main && moves_main) {
        coderepo::commits_between(repo, &integration, &branch)
            .unwrap_or_default()
            .into_iter()
            .map(|c| CommitLine {
                sha: c.sha,
                subject: c.subject,
            })
            .collect()
    } else {
        Vec::new()
    };

    // 2. The repository: tag the release tip, then fast-forward main onto it.
    let tip = coderepo::rev_parse(repo, &branch)?;
    coderepo::create_tag(repo, version, &tip, &format!("Release {version}"))?;
    let previous = if moves_main {
        match coderepo::fast_forward(repo, &main, &tip) {
            Ok(previous) => Some(previous),
            Err(e) => return Err(rolled_back(e, coderepo::delete_tag(repo, version).err())),
        }
    } else {
        None
    };

    // 3. The board: the cut, recording this stage change.
    match cut_with(store, id, version, Some(change)) {
        Ok(cut) => {
            let mut notes = cut.notes;
            if !forward.is_empty() {
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
            done.tagged = version.to_string();
            if moves_main {
                done.main_branch = main;
            }
            done.preview = preview;
            done.forward_merge = forward;
            done.forward_into = integration;
            Ok(done)
        }
        // 4. The board refused after the repository moved: put the repository back.
        Err(e) => {
            let reset = previous
                .and_then(|previous| coderepo::reset_branch_to(repo, &main, &previous).err());
            let untag = coderepo::delete_tag(repo, version).err();
            Err(rolled_back(e, reset.or(untag)))
        }
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
        let fix: crate::FeatureItem = serde_yaml::from_str(
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
}
