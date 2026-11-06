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
use crate::config::Cadence;
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
    /// When the stage holding the tag action was entered (FTR-026), RFC 3339. A release that is
    /// tagged and still `Planned` is waiting for the stages after the tag.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tagged_at: String,
    /// The full sha the tag names (FTR-026).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag_commit: String,
    /// The patch release that replaced this one after its tag's CI run failed (FTR-026). The tag
    /// stays in git; the release never ships.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub superseded_by: String,
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

/// Which release an item shipped in, or will (FTR-001). None when the project does not work in
/// releases. A release that shipped the item wins, the earliest one, so a patch's parent is named
/// rather than the patch that keeps it.
pub fn release_line(project: &Project, releases: &[Release], item: &FeatureItem) -> Option<String> {
    if !project.config.cadence.releases {
        return None;
    }
    if let Some(r) = releases.iter().find(|r| r.shipped.contains(&item.code)) {
        return Some(format!("Released in {}", r.version));
    }
    match item
        .release
        .as_deref()
        .and_then(|v| releases.iter().find(|r| r.version == v))
    {
        Some(r) if !r.superseded_by.is_empty() => {
            Some(format!("Superseded by {}", r.superseded_by))
        }
        Some(r) if !r.tagged_at.is_empty() => {
            Some(format!("Tagged in {}, not released yet", r.version))
        }
        Some(r) => Some(format!("Planned for {}, not released yet", r.version)),
        None => Some("Not in a release".to_string()),
    }
}

/// What starting a patch release from a failed tag did (FTR-026).
#[derive(Debug, Clone, Serialize)]
pub struct PatchFromTag {
    pub release: Release,
    /// The defect filed, carrying the failed run's link and the user's notes.
    pub defect: String,
    /// The items planned into the tagged release that moved into the patch, the defect first.
    pub planned: Vec<String>,
}

/// Start a patch release from a tag whose recorded result is failed (FTR-026): file a defect
/// carrying the failed run's link and the user's notes, plan it and the tagged release's items into
/// a new patch branched from the tag, and mark the tagged release as superseded by it (its tag
/// stays in the repository). Nothing is written unless the patch's branch is made; if a later
/// board write fails the patch is taken back.
#[allow(clippy::too_many_arguments)]
pub fn patch_from_failed_tag(
    store: &Store,
    id: &str,
    parent: &str,
    version: &str,
    by: &str,
    run_link: &str,
    notes: &str,
    repo: Option<&Path>,
) -> Result<PatchFromTag> {
    let project = cadence_on(store, id)?;
    let parent = parent.trim();
    let releases = load(store, id)?;
    let p = releases
        .iter()
        .find(|r| r.version == parent)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {parent}")))?;
    if p.tagged_at.is_empty() || p.state == ReleaseState::Shipped || !p.superseded_by.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "{parent} is not a tagged release waiting on its CI result, so there is no failed tag \
             to patch"
        )));
    }
    let failed = tag_result(store, id, &project.config.cadence, p)
        .and_then(|t| t.result)
        .filter(|r| !r.passed);
    let Some(failed) = failed else {
        return Err(CoreError::Unsupported(format!(
            "{parent}'s tag has no failed result recorded: record the failed run first \
             (kanbanr gate record <gate> --failed --commit <sha>)"
        )));
    };
    let link = if run_link.trim().is_empty() {
        failed.url.clone()
    } else {
        run_link.trim().to_string()
    };
    let moved: Vec<String> = {
        let mut codes: Vec<String> = project
            .features
            .iter()
            .filter(|f| f.release.as_deref() == Some(parent))
            .map(|f| f.code.clone())
            .collect();
        codes.sort();
        codes
    };
    let milestone = moved
        .first()
        .and_then(|c| project.features.iter().find(|f| f.code == *c))
        .map(|f| f.milestone.clone())
        .or_else(|| project.milestones.first().map(|m| m.code.clone()))
        .ok_or_else(|| {
            CoreError::Unsupported("the defect needs a milestone, and the project has none".into())
        })?;
    let patch = add_patch(store, id, version, "", "", parent, by, repo)?;
    let undo = |store: &Store, e: CoreError| {
        let _ = remove_release(store, id, &patch.version);
        if let Some(repo) = repo {
            let _ = coderepo::delete_branch(repo, &format!("release/{}", patch.version));
        }
        e
    };
    let mut spec = format!("The CI run for the {parent} tag failed.\n");
    if !link.is_empty() {
        spec.push_str(&format!("\nRun: {link}\n"));
    }
    if !notes.trim().is_empty() {
        spec.push_str(&format!("\n{}\n", notes.trim()));
    }
    let defect = match store.add_feature_of_kind(
        id,
        &format!("Fix the failed CI run on {parent}"),
        &spec,
        &milestone,
        None,
        Some("defect"),
    ) {
        Ok(d) => d,
        Err(e) => return Err(undo(store, e)),
    };
    let mut planned = vec![defect.code.clone()];
    planned.extend(moved);
    for code in &planned {
        if let Err(e) = store.set_feature_release(id, code, Some(patch.version.clone())) {
            for c in planned.iter().skip(1) {
                let _ = store.set_feature_release(id, c, Some(parent.to_string()));
            }
            return Err(undo(store, e));
        }
    }
    let mut all = load(store, id)?;
    if let Some(r) = all.iter_mut().find(|r| r.version == parent) {
        r.superseded_by = patch.version.clone();
    }
    if let Err(e) = save(store, id, &all) {
        return Err(undo(store, e));
    }
    let release = load(store, id)?
        .into_iter()
        .find(|r| r.version == patch.version)
        .unwrap_or(patch);
    Ok(PatchFromTag {
        release,
        defect: defect.code,
        planned,
    })
}

fn remove_release(store: &Store, id: &str, version: &str) -> Result<()> {
    let mut all = load(store, id)?;
    all.retain(|r| r.version != version);
    save(store, id, &all)
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
    // A tag is a cut even before the release ships (FTR-026): a patch may start from it.
    if p.state != ReleaseState::Shipped && p.tagged_at.is_empty() {
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
    ship_release(store, id, version, None)
}

/// Record that the stage holding the tag action was entered (FTR-026): `tagged_at`, the commit the
/// tag names (read from the repository when there is one) and the stage change. Ships nothing: the
/// release stays `Planned` and its items stay where they are until the last stage is confirmed.
pub fn tag_release(
    store: &Store,
    id: &str,
    version: &str,
    repo: Option<&Path>,
    change: Option<StageChange>,
) -> Result<()> {
    cadence_on(store, id)?;
    let mut releases = load(store, id)?;
    let release = releases
        .iter_mut()
        .find(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    release.tagged_at = crate::now_rfc3339();
    release.tag_commit = repo
        .and_then(|r| coderepo::rev_parse(r, &format!("refs/tags/{version}")).ok())
        .unwrap_or_default();
    if let Some(change) = change {
        release.stage = change.stage.clone();
        release.stages.push(change);
    }
    save(store, id, &releases)
}

/// Ship a release (FTR-026): its planned items that are finished move to their end status, the
/// notes are written, `shipped_at` is recorded and what could not ship is carried. Records the
/// stage change that made it when there is one (FEAT-183). Either way the release ends at the last
/// stage: a shipped release is in production, whichever command shipped it.
fn ship_release(
    store: &Store,
    id: &str,
    version: &str,
    change: Option<StageChange>,
) -> Result<Cut> {
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
                    Some(end) => store.move_feature_for_cut(id, &f.code, &end)?,
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
    /// This promotion enters the stage that tags: the cut.
    pub cut: bool,
    /// This promotion enters a stage after the cut: a confirmation by the user (FTR-026).
    pub confirm: bool,
    /// This promotion enters the last stage: it ships the release (FTR-026).
    pub ships: bool,
    /// The recorded CI result for the tag, once the release is tagged and not shipped (FTR-026).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<TagResult>,
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
    /// Nothing is planned into the release, so it cannot be promoted (R-17).
    pub empty: bool,
    /// Things worth knowing that do not stop the promotion, e.g. a stale release date (DEF-001).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// The newest result recorded for a release's tag (FTR-026), read from the board only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TagResult {
    /// The commit the tag names.
    pub commit: String,
    /// The gate that judges it (`cadence.tag_gate`, else the first commit gate).
    pub gate: String,
    /// `passed`, `failed` or `none`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<crate::gates::GateResult>,
}

/// The tag's recorded result: the newest result of the tag gate for the tag commit, recorded since
/// the tag was made, so a pass from before the tag does not stand in for a run on the tagged
/// commit. A release not tagged yet has none. Results that cannot be read count as none.
pub fn tag_result(
    store: &Store,
    id: &str,
    cadence: &Cadence,
    release: &Release,
) -> Option<TagResult> {
    if release.tagged_at.is_empty() || release.state == ReleaseState::Shipped {
        return None;
    }
    let gate = cadence.tag_gate_name().unwrap_or_default();
    let result = if gate.is_empty() || release.tag_commit.is_empty() {
        None
    } else {
        crate::gates::results(store, id, None)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                r.name == gate
                    && r.commit == release.tag_commit
                    && crate::cmp_instants(&r.at, &release.tagged_at) != std::cmp::Ordering::Less
            })
            .next_back()
    };
    let state = match &result {
        None => "none",
        Some(r) if r.passed => "passed",
        Some(_) => "failed",
    };
    Some(TagResult {
        commit: release.tag_commit.clone(),
        gate,
        state: state.to_string(),
        result,
    })
}

/// Whether the tag's recorded result is passed: held, what was found and what to do.
fn tag_passed(
    store: &Store,
    id: &str,
    cadence: &Cadence,
    release: &Release,
) -> (bool, String, String) {
    let fix = "run your CI on the tag, then record its verdict (kanbanr gate record <gate> \
               --passed --commit <sha> [--url <link>]); if it failed: start a patch release"
        .to_string();
    let Some(t) = tag_result(store, id, cadence, release) else {
        return (
            false,
            "the release has not been tagged yet".to_string(),
            "promote the release to the stage that tags it first".to_string(),
        );
    };
    let sha = short(&t.commit).to_string();
    let fix = fix.replace("<sha>", &sha).replace("<gate>", &t.gate);
    match t.result {
        None => (
            false,
            "no result recorded for the tag since it was made".to_string(),
            fix,
        ),
        Some(r) if r.passed => (
            true,
            format!("{} passed on {sha} (recorded by {})", t.gate, r.by),
            String::new(),
        ),
        Some(r) => {
            let note = if r.note.is_empty() {
                String::new()
            } else {
                format!(": {}", r.note)
            };
            (false, format!("{} failed on {sha}{note}", t.gate), fix)
        }
    }
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
    /// Nothing is planned into it (R-17).
    empty: bool,
    patch: bool,
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// Whether every gate in `commit_gates` has a passed result recorded for `tip` (FTR-002). Reads
/// the board's recorded results only: no server is asked. Returns held, the detail and the fix.
fn gate_passed(
    store: &Store,
    id: &str,
    commit_gates: &[String],
    tip: Option<&str>,
    prepares: bool,
    version: &str,
    branch: &str,
) -> (bool, String, String) {
    let fix_for = |sha: &str| {
        format!(
            "look at your CI for {}, then record its verdict: kanbanr gate record <gate> \
             --passed --commit {sha} [--url <link>], or the Record result form on the Releases \
             page",
            short(sha)
        )
    };
    if prepares {
        return (
            false,
            "the preparation commit this promotion makes has no result yet".to_string(),
            format!(
                "promote {version} to the stage that prepares it first, and record the result \
                 once it is on the branch"
            ),
        );
    }
    let Some(tip) = tip else {
        return (
            false,
            format!("{branch} does not exist yet"),
            fix_for("<sha>"),
        );
    };
    let results = crate::gates::results(store, id, None).unwrap_or_default();
    let mut recorded = Vec::new();
    for name in commit_gates {
        match crate::gates::newest(&results, name, tip) {
            None => {
                return (
                    false,
                    format!("{name} has no recorded result for {}", short(tip)),
                    fix_for(tip),
                );
            }
            Some(r) if !r.passed => {
                let note = if r.note.is_empty() {
                    String::new()
                } else {
                    format!(": {}", r.note)
                };
                return (
                    false,
                    format!("{name} failed on {}{note}", short(tip)),
                    fix_for(tip),
                );
            }
            Some(r) => recorded.push(format!(
                "{name} passed on {} (recorded by {}, {})",
                short(tip),
                r.by,
                r.at.get(..10).unwrap_or(&r.at)
            )),
        }
    }
    (true, recorded.join("; "), String::new())
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
    if !release.superseded_by.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "{version} was superseded by {}: it never ships, and its tag stays in the repository",
            release.superseded_by
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
    let cut_stage = cadence.cut_index();
    let cut = direction == Direction::Forward
        && current.is_none_or(|c| c < cut_stage)
        && target >= cut_stage;
    let confirm = direction == Direction::Forward && target > cut_stage;
    let ships = direction == Direction::Forward && target == last;
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
    // A release with nothing planned into it is never promoted (FEAT-212 R-17): it would ship
    // nothing. A patch carries its parent's fixes, so it goes with none of its own.
    let empty = release.parent.is_empty()
        && !project
            .features
            .iter()
            .any(|f| f.release.as_deref() == Some(version));
    if empty {
        actions.clear();
        requires.clear();
    }
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
    let mut warnings: Vec<String> = Vec::new();
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
                    ReleaseCheck::TagPassed => {
                        let (holds, detail, _) = tag_passed(store, id, cadence, &release);
                        (holds, detail)
                    }
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
                } else if *c == ReleaseCheck::TagPassed {
                    tag_passed(store, id, cadence, &release).2
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
            // The notes and changelog carry the date a candidate was prepared on; main moving is
            // when they are dated today (DEF-001). A promotion that prepares writes today's date.
            let today = today_ymd();
            let stale = match (&tip_before, will_have_branch && !prepares) {
                (Some(t), true) => stale_dates(&project, &releases[index], repo, t, &today),
                _ => Vec::new(),
            };
            let redates = moves && !main_at_tip && !left_behind && !stale.is_empty();
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
                } else if redates {
                    (
                        format!("dates the release {today} and fast-forwards {main} to {branch}"),
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
            if cut && !stale.is_empty() && !redates {
                let mut dates = stale.clone();
                dates.sort();
                dates.dedup();
                warnings.push(format!(
                    "{version}'s notes and changelog are dated {}, and today is {today}: run \
                     `kanbanr release redate {version}` to date them today{}",
                    dates.join(" and "),
                    if main_at_tip {
                        format!(", then push {main} again")
                    } else {
                        String::new()
                    }
                ));
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
                    ReleaseCheck::GatePassed => gate_passed(
                        store,
                        id,
                        &cadence.commit_gates,
                        tip_before.as_deref(),
                        prepares,
                        version,
                        &branch,
                    ),
                    ReleaseCheck::TagPassed => tag_passed(store, id, cadence, &release),
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
    if empty {
        missing.push(empty_release(version));
    }
    if needs_reason {
        missing.push(format!(
            "moving {version} back from {from} to {stage} needs a reason (--reason)"
        ));
    }
    if direction == Direction::Again && steps.iter().all(|s| s.noop) && !empty {
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
        if i == cut_stage {
            format!("Cut {version}")
        } else if i > cut_stage && i == last {
            format!("Confirm {}: ship {version}", stages[i])
        } else if i > cut_stage {
            format!("Confirm {}", stages[i])
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
    } else if target == cut_stage {
        let push = if has(ReleaseAction::Tag) {
            format!("push the tag: `git push origin {version}`")
        } else {
            String::new()
        };
        if cut_stage < last {
            let then = format!(
                "run your CI on the tag and record the result, then {}",
                button_for(target + 1)
            );
            if push.is_empty() {
                then
            } else {
                format!("{push}, {then}")
            }
        } else {
            push
        }
    } else if target == last {
        String::new()
    } else if target > cut_stage {
        format!("then {}", button_for(target + 1))
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
        confirm,
        ships,
        tag: tag_result(store, id, cadence, &release),
        button,
        ready: missing.is_empty(),
        missing,
        conditions,
        steps,
        preview: if ships { preview } else { Vec::new() },
        tag_commit,
        release_tip,
        needs_reason,
        origin,
        after,
        repo: repo.is_some(),
        empty,
        warnings,
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
        empty,
        patch,
    })
}

/// Why a release with nothing planned into it cannot be promoted, and how to plan into it.
pub fn empty_release(version: &str) -> String {
    format!("{version} has no items: plan some in with `kanbanr release plan {version} <CODE>…`")
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
        // Several entries may name one file (a lockfile's crates); it is listed once.
        if !ok && !lacking.contains(&f.path) {
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
                // Main moves with the date of the day it moves (DEF-001).
                let project = store.load_meta(id)?;
                let at = coderepo::rev_parse(repo, &w.branch)?;
                let ymd = today_ymd();
                let files = redate_files(&project, &releases[index], repo, &at, &ymd)?;
                if !files.is_empty() {
                    let message = redate_message(version, &ymd, by);
                    let (old, new) = coderepo::commit_files(repo, &w.branch, &files, &message)?;
                    undo.push(Undo::RemoveCommit {
                        branch: w.branch.clone(),
                        old,
                        new,
                    });
                }
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
    if w.plan.ships || w.plan.cut {
        // Tagging and shipping read the release from the board, so a branch just made is recorded
        // first; if either is refused, the branch and the record are both taken back.
        let mut saved = false;
        if !done.branch_created.is_empty() {
            if let Err(e) = save(store, id, &releases) {
                return Err(undo_all(repo, undo, e));
            }
            saved = true;
            done.release = releases[index].clone();
        }
        // The stage holding the tag action was entered: the tag is a fact about the code, and
        // the release is not shipped by it unless it is also the last stage (FTR-026).
        if w.plan.cut {
            let tag_change = (!w.plan.ships).then(|| change.clone());
            if let Err(e) = tag_release(store, id, version, repo, tag_change) {
                let err = undo_all(repo, undo, e);
                if saved {
                    let _ = save(store, id, &original);
                }
                return Err(err);
            }
            saved = true;
            if !w.plan.ships {
                done.release = load(store, id)?
                    .into_iter()
                    .find(|r| r.version == version)
                    .unwrap_or_else(|| done.release.clone());
            }
        }
        if !w.plan.ships {
            done.forward_merge = forward;
            done.forward_into = integration;
            done.next = next_hint(store, id, version, repo, &done, &w.main);
            return Ok(done);
        }
        match ship_release(store, id, version, Some(change)) {
            Ok(cut) => {
                let mut notes = cut.notes;
                // The notes that shipped, when there are some at the tip, replace the generated
                // text; a read that fails keeps the generated notes.
                let shipped_notes =
                    repo.and_then(|r| shipped_notes_at(r, store, id, &w, &cut.release));
                let wrote = shipped_notes.is_some();
                if let Some(text) = shipped_notes {
                    notes = text;
                }
                if w.patch && !forward.is_empty() {
                    notes.push_str(&format!("\n## To merge forward into {integration}\n\n"));
                    for c in &forward {
                        notes.push_str(&format!(
                            "- {} {}\n",
                            &c.sha[..c.sha.len().min(12)],
                            c.subject
                        ));
                    }
                }
                if wrote || (w.patch && !forward.is_empty()) {
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

/// The configured notes file at the release tip, when there is one to read (DEF-005).
fn shipped_notes_at(
    repo: &Path,
    store: &Store,
    id: &str,
    w: &Worked,
    release: &Release,
) -> Option<String> {
    let project = store.load_meta(id).ok()?;
    let template = project.config.cadence.release_prepare.notes.as_ref()?;
    let ph = crate::releaseprep::Placeholders::new(&release.version, &release.name, &today_ymd())
        .ok()?;
    let path = crate::releaseprep::expand(template, &ph);
    crate::releaseprep::check_repo_path(&path).ok()?;
    let tip = coderepo::rev_parse(repo, &w.branch).ok()?;
    coderepo::read_blob(repo, &tip, &path)
        .ok()
        .flatten()
        .map(|(_, t)| t)
}

/// Why a plan that is not ready is refused: a back move's reason or an "already at" as before;
/// every missing condition with its fix when a stage requires something; else the first step's
/// refusal, word for word as the cut always gave it.
fn refusal(w: &Worked) -> CoreError {
    let p = &w.plan;
    if w.empty {
        return CoreError::Unsupported(empty_release(&p.version));
    }
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
    // A tag just made is pushed first, then the stages after it are confirmed (FTR-026).
    let pushed = if done.tagged.is_empty() {
        String::new()
    } else if done.main_moved.is_some() {
        format!("push {main} and the tag: `git push {remote} {main} {version}`; ")
    } else {
        format!("push the tag: `git push {remote} {version}`; ")
    };
    match plan_promotion(store, id, version, None, "", repo, Fetch::No) {
        Ok(p) if p.ready => format!(
            "{pushed}next: `kanbanr release promote {version}` ({})",
            p.button
        ),
        Ok(p) => format!(
            "{pushed}next: {}, then {}",
            p.missing.first().cloned().unwrap_or_default(),
            p.button
        ),
        Err(_) => pushed.trim_end_matches("; ").to_string(),
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

/// A release's notes as they are on its release branch (FEAT-216): where they are, what they say,
/// and whether they can still be edited. Derived, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseNotes {
    pub version: String,
    /// The release branch, `release/<V>`.
    pub branch: String,
    /// The notes file in the code repository: `release_prepare.notes` for this version.
    pub path: String,
    pub text: String,
    /// The release branch's tip the text was read at, seven characters.
    pub at: String,
    /// The release has a branch and has not shipped.
    pub editable: bool,
    /// Why not, when not editable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why_not: String,
}

/// The release, its branch and its notes path, or why its notes cannot be found.
fn notes_target(
    store: &Store,
    id: &str,
    version: &str,
) -> Result<(Project, Release, crate::releaseprep::Placeholders, String)> {
    let project = cadence_on(store, id)?;
    let release = load(store, id)?
        .into_iter()
        .find(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    let Some(template) = project.config.cadence.release_prepare.notes.clone() else {
        return Err(CoreError::Unsupported(
            "no release notes path is configured: `kanbanr config release-prepare --notes \
             <path with {version}>`"
                .into(),
        ));
    };
    if release.branch.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "{version} has no release branch yet: promote it to candidate first"
        )));
    }
    let ph = crate::releaseprep::Placeholders::new(&release.version, &release.name, &today_ymd())
        .map_err(CoreError::Unsupported)?;
    let path = crate::releaseprep::expand(&template, &ph);
    crate::releaseprep::check_repo_path(&path).map_err(CoreError::Unsupported)?;
    Ok((project, release, ph, path))
}

fn notes_need_repo(version: &str) -> CoreError {
    CoreError::Unsupported(format!(
        "{version}'s notes are on its release branch, in the code repository: use the CLI from \
         the project folder, or a monitor started there"
    ))
}

fn branch_tip(repo: &Path, release: &Release) -> Result<String> {
    coderepo::rev_parse(repo, &release.branch).map_err(|_| {
        CoreError::Unsupported(format!(
            "{} is not in this repository: run it from {}'s project folder",
            release.branch, release.version
        ))
    })
}

/// The notes of `release` as read at `tip`.
fn notes_at(release: &Release, path: &str, text: String, tip: &str) -> ReleaseNotes {
    let shipped = release.state == ReleaseState::Shipped;
    ReleaseNotes {
        version: release.version.clone(),
        branch: release.branch.clone(),
        path: path.to_string(),
        text,
        at: short(tip).to_string(),
        editable: !shipped,
        why_not: if shipped {
            format!(
                "{} has shipped: its notes are as they shipped",
                release.version
            )
        } else {
            String::new()
        },
    }
}

/// A release's notes as they are at the tip of `release/<V>`, read without checking anything out
/// (FEAT-216). Refused, plainly, when no notes path is configured, the release has no branch yet,
/// there is no repository, or the file is not on the branch.
pub fn release_notes(
    store: &Store,
    id: &str,
    version: &str,
    repo: Option<&Path>,
) -> Result<ReleaseNotes> {
    let (_, release, _, path) = notes_target(store, id, version)?;
    let repo = repo.ok_or_else(|| notes_need_repo(version))?;
    let tip = branch_tip(repo, &release)?;
    let (_, text) = coderepo::read_blob(repo, &tip, &path)?.ok_or_else(|| {
        CoreError::Unsupported(format!(
            "{path} is not on {} at {}: nothing has written {version}'s notes yet",
            release.branch,
            short(&tip)
        ))
    })?;
    Ok(notes_at(&release, &path, text, &tip))
}

/// The dates before `today` on the lines of a release's notes and changelog that carry one, at
/// `tip`: what `redate_files` would change.
fn stale_dates(
    project: &Project,
    release: &Release,
    repo: &Path,
    tip: &str,
    today: &str,
) -> Vec<String> {
    dated_lines(project, release, repo, tip)
        .into_iter()
        .filter(|l| l.date.as_str() < today)
        .map(|l| l.date)
        .collect()
}

/// A file of a release that carries its date.
struct DatedLine {
    path: String,
    text: String,
    /// `YYYY-MM-DD`.
    date: String,
    /// The changelog heading (else the notes line).
    changelog: bool,
}

/// The release notes and the changelog (with its copies) at `tip`, each with the date it carries;
/// a file that is missing or has no date line is left out.
fn dated_lines(project: &Project, release: &Release, repo: &Path, tip: &str) -> Vec<DatedLine> {
    use crate::releaseprep as prep;
    let cfg = &project.config.cadence.release_prepare;
    let Ok(ph) = prep::Placeholders::new(&release.version, &release.name, &today_ymd()) else {
        return Vec::new();
    };
    let read = |path: &str| {
        prep::check_repo_path(path).ok()?;
        coderepo::read_blob(repo, tip, path)
            .ok()
            .flatten()
            .map(|(_, t)| t)
    };
    let mut out = Vec::new();
    if let Some(n) = &cfg.notes {
        let path = prep::expand(n, &ph);
        if let Some(text) = read(&path)
            && let Some(date) = prep::notes_date(&text)
        {
            out.push(DatedLine {
                path,
                text,
                date,
                changelog: false,
            });
        }
    }
    for path in cfg.changelog.iter().chain(cfg.changelog_copies.iter()) {
        if let Some(text) = read(path)
            && let Some(date) = prep::changelog_date(&text, &ph.number)
        {
            out.push(DatedLine {
                path: path.clone(),
                text,
                date,
                changelog: true,
            });
        }
    }
    out
}

/// The files of `release` at `tip` whose date is before `ymd`, with the text they should have:
/// the notes, the changelog and each of its copies. Empty when there is nothing to date.
fn redate_files(
    project: &Project,
    release: &Release,
    repo: &Path,
    tip: &str,
    ymd: &str,
) -> Result<Vec<(String, String)>> {
    use crate::releaseprep as prep;
    let ph = prep::Placeholders::new(&release.version, &release.name, ymd)
        .map_err(CoreError::Unsupported)?;
    Ok(dated_lines(project, release, repo, tip)
        .into_iter()
        .filter(|l| l.date.as_str() < ymd)
        .filter_map(|l| {
            let new = if l.changelog {
                prep::redate_changelog(&l.text, &ph.number, ymd)
            } else {
                prep::redate_notes(&l.text, ymd)
            };
            new.map(|n| (l.path, n))
        })
        .collect())
}

fn redate_message(version: &str, ymd: &str, by: &str) -> String {
    let by = if by.trim().is_empty() {
        "someone"
    } else {
        by.trim()
    };
    format!(
        "Release {version}: dated {ymd}\n\nDated through kanbanr by {by}.\n\n\
         [no-ref] release date, written by kanbanr\nKanbanr-Release: {version}\n"
    )
}

/// Whether main holds the release at `tip` and so moves with a commit on `branch`; main past the
/// release cannot, and is refused before anything is committed. `carried` names what the commit
/// carries, for the message.
fn main_follows(repo: &Path, main: &str, branch: &str, tip: &str, carried: &str) -> Result<bool> {
    let main_tip = coderepo::rev_parse(repo, main).ok();
    let moves_main = main_tip.as_deref() == Some(tip);
    if let Some(at) = main_tip
        .as_deref()
        .filter(|at| !moves_main && coderepo::is_ancestor(repo, tip, at))
    {
        return Err(CoreError::Unsupported(format!(
            "{main} is at {}, past {branch} at {}: {carried} saved on {branch} would not reach the \
             commit to be tagged, so nothing was saved",
            short(at),
            short(tip)
        )));
    }
    Ok(moves_main)
}

/// Commit `files` onto `branch` and, when `moves_main`, fast-forward main to the commit. Returns
/// the new tip and what to tell the person about main.
#[allow(clippy::too_many_arguments)]
fn commit_and_follow(
    repo: &Path,
    main: &str,
    branch: &str,
    version: &str,
    tip: &str,
    moves_main: bool,
    files: &[(String, String)],
    message: &str,
    carried: &str,
) -> Result<(String, String)> {
    let (_, new) = coderepo::commit_files(repo, branch, files, message)?;
    let mut main_moved = String::new();
    if moves_main {
        if let Err(e) = coderepo::fast_forward(repo, main, &new) {
            // Leave both as they were rather than main behind the commit.
            let _ = coderepo::reset_branch_to(repo, branch, tip);
            return Err(CoreError::Unsupported(format!(
                "{main} held {version} but could not move with its {carried} ({e}), so nothing \
                 was saved"
            )));
        }
        main_moved = format!(
            "{main} held {version}, so it moved with the {carried} to {}: push {main} again and \
             wait for CI before the cut",
            short(&new)
        );
    }
    Ok((new, main_moved))
}

/// Date a release's notes and changelog today, in one commit onto `release/<V>`, without checking
/// it out (DEF-001). Main moves with it when it holds the release; `main_moved` then says to push
/// again. Nothing to date (no notes or changelog, no date line, already today's) is no commit.
/// Refused once the release has shipped.
pub fn redate_release(
    store: &Store,
    id: &str,
    version: &str,
    repo: Option<&Path>,
    by: &str,
) -> Result<NotesSaved> {
    use crate::releaseprep as prep;
    let project = cadence_on(store, id)?;
    let release = load(store, id)?
        .into_iter()
        .find(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if release.state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has shipped: its notes and changelog are as they shipped, and are not \
             re-dated"
        )));
    }
    if release.branch.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "{version} has no release branch yet: promote it to candidate first"
        )));
    }
    let repo = repo.ok_or_else(|| notes_need_repo(version))?;
    let branch = release.branch.clone();
    let tip = branch_tip(repo, &release)?;
    let ymd = today_ymd();
    let path = match &project.config.cadence.release_prepare.notes {
        Some(n) => {
            let ph = prep::Placeholders::new(&release.version, &release.name, &ymd)
                .map_err(CoreError::Unsupported)?;
            prep::expand(n, &ph)
        }
        None => String::new(),
    };
    let notes_now = |at: &str| {
        let text = coderepo::read_blob(repo, at, &path)
            .ok()
            .flatten()
            .map(|(_, t)| t)
            .unwrap_or_default();
        notes_at(&release, &path, text, at)
    };
    let files = redate_files(&project, &release, repo, &tip, &ymd)?;
    if files.is_empty() {
        return Ok(NotesSaved {
            sha: tip.clone(),
            notes: notes_now(&tip),
            main_moved: String::new(),
        });
    }
    let main = project
        .config
        .cadence
        .main_branch
        .clone()
        .unwrap_or_else(|| coderepo::default_branch(repo));
    let moves_main = main_follows(repo, &main, &branch, &tip, "a new date")?;
    let (new, main_moved) = commit_and_follow(
        repo,
        &main,
        &branch,
        version,
        &tip,
        moves_main,
        &files,
        &redate_message(version, &ymd, by),
        "new date",
    )?;
    Ok(NotesSaved {
        sha: new.clone(),
        notes: notes_now(&new),
        main_moved,
    })
}

/// What saving a release's notes did (FEAT-216).
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotesSaved {
    /// The release branch's tip after the save: the tip as it was when nothing changed.
    pub sha: String,
    pub notes: ReleaseNotes,
    /// Set when main held the release and was moved with the notes: what to do next.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub main_moved: String,
}

/// Commit `text` as a release's notes onto `release/<V>`, without checking it out (FEAT-216), and
/// re-apply the configured notes listings. Nothing to change (the same text, every listing there)
/// is no commit: the sha is then the tip as it was. Refused once the release has shipped, and when
/// the branch is checked out in a worktree.
///
/// When main already holds the release (the stage that moves main has run), main is moved with
/// the notes, so the commit to be tagged has them, and `main_moved` says to push it again; were
/// main past the release, the save is refused before anything is committed.
pub fn set_release_notes(
    store: &Store,
    id: &str,
    version: &str,
    repo: Option<&Path>,
    text: &str,
    by: &str,
) -> Result<NotesSaved> {
    use crate::releaseprep as prep;
    let (project, release, ph, path) = notes_target(store, id, version)?;
    if release.state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has shipped: its notes are as they shipped, and are not edited on its branch"
        )));
    }
    let repo = repo.ok_or_else(|| notes_need_repo(version))?;
    let branch = release.branch.clone();
    let tip = branch_tip(repo, &release)?;
    let read = |path: &str| -> Result<Option<String>> {
        Ok(coderepo::read_blob(repo, &tip, path)?.map(|(_, t)| t))
    };
    let main = project
        .config
        .cadence
        .main_branch
        .clone()
        .unwrap_or_else(|| coderepo::default_branch(repo));
    // Main at the tip holds the release and moves with the notes; main past it cannot.
    let moves_main = main_follows(repo, &main, &branch, &tip, "notes")?;
    let mut files: Vec<(String, String)> = Vec::new();
    if read(&path)?.as_deref() != Some(text) {
        files.push((path.clone(), text.to_string()));
    }
    for l in &project.config.cadence.release_prepare.notes_listings {
        let listing = prep::expand(&l.path, &ph);
        prep::check_repo_path(&listing).map_err(CoreError::Unsupported)?;
        let current = match files.iter().find(|(p, _)| *p == listing) {
            Some((_, c)) => c.clone(),
            None => read(&listing)?.ok_or_else(|| {
                CoreError::Unsupported(format!("{listing} does not exist on {branch}"))
            })?,
        };
        if let Some(new) = prep::add_listing(
            &current,
            &prep::expand(&l.after, &ph),
            &prep::expand(&l.entry, &ph),
        )
        .map_err(|e| CoreError::Unsupported(format!("{listing} {e}")))?
        {
            match files.iter_mut().find(|(p, _)| *p == listing) {
                Some(f) => f.1 = new,
                None => files.push((listing, new)),
            }
        }
    }
    if files.is_empty() {
        return Ok(NotesSaved {
            sha: tip.clone(),
            notes: notes_at(&release, &path, text.to_string(), &tip),
            main_moved: String::new(),
        });
    }
    let by = if by.trim().is_empty() {
        "someone"
    } else {
        by.trim()
    };
    let message = format!(
        "Release {version}: notes\n\nEdited through kanbanr release notes by {by}.\n\n\
         [no-ref] release notes, written by kanbanr\nKanbanr-Release: {version}\n"
    );
    let (new, main_moved) = commit_and_follow(
        repo, &main, &branch, version, &tip, moves_main, &files, &message, "notes",
    )?;
    Ok(NotesSaved {
        sha: new.clone(),
        notes: notes_at(&release, &path, text.to_string(), &new),
        main_moved,
    })
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
        // A release with nothing planned into it is never promoted (FEAT-212 R-17).
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        plan_done(&store, "v1");
        let repo = crate::coderepo::tests::scratch_repo(&format!("promote-{tag}"));
        (store, dir, repo)
    }

    /// A finished item planned into `version` (FEAT-212 R-17).
    fn plan_done(store: &Store, version: &str) -> String {
        let code = store
            .add_feature("shop", &format!("Ships in {version}"), "", "M", None)
            .unwrap()
            .code;
        for status in ["Scheduled", "Completed"] {
            store
                .move_feature_approved("shop", &code, status, None)
                .unwrap();
        }
        super::plan(store, "shop", version, std::slice::from_ref(&code)).unwrap();
        code
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

    // ---- FTR-002: gate_passed ----

    /// Candidate makes the branch; production tags and needs `ci` to have passed on the tip.
    const GATED: &str = "release_stages: [candidate, production]\n\
         release_gates:\n  candidate: { on_enter: [branch] }\n  \
         production: { requires: [gate_passed], on_enter: [move_main, tag] }\n";

    fn gated(tag: &str, gates: &[&str]) -> (Store, PathBuf, PathBuf, String) {
        gated_after(tag, gates, |_| {})
    }

    /// [`gated`], with `before` run on the repository first (a commit the release branch starts from).
    fn gated_after(
        tag: &str,
        gates: &[&str],
        before: impl FnOnce(&Path),
    ) -> (Store, PathBuf, PathBuf, String) {
        let (store, dir, repo) = board_and_repo(tag);
        before(&repo);
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        let given: crate::config::Cadence = crate::yaml::from_str(GATED).unwrap();
        c.release_stages = given.release_stages;
        c.release_gates = given.release_gates;
        c.commit_gates = gates.iter().map(|g| g.to_string()).collect();
        store.set_cadence("shop", None, c).unwrap();
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
        let tip = coderepo::rev_parse(&repo, "release/v1").unwrap();
        (store, dir, repo, tip)
    }

    fn gate_condition(store: &Store, repo: &Path) -> super::PlanCondition {
        let plan = super::plan_promotion(
            store,
            "shop",
            "v1",
            Some("production"),
            "",
            Some(repo),
            super::Fetch::No,
        )
        .unwrap();
        plan.conditions
            .into_iter()
            .find(|c| c.check == crate::releasegates::ReleaseCheck::GatePassed)
            .unwrap()
    }

    fn record_gate(store: &Store, name: &str, commit: &str, passed: bool, at: &str) {
        let r = crate::gates::GateResult {
            name: name.into(),
            commit: commit.into(),
            passed,
            by: "Ada L".into(),
            at: at.into(),
            source: "person".into(),
            note: if passed { "" } else { "two tests red" }.into(),
            ..Default::default()
        };
        crate::gates::record(store, "shop", &r).unwrap();
    }

    #[test]
    fn gate_passed_holds_on_a_recorded_pass() {
        let (store, dir, repo, tip) = gated("gp-holds", &["ci"]);
        record_gate(&store, "ci", &tip, true, "2026-11-04T10:00:00Z");
        let c = gate_condition(&store, &repo);
        assert!(c.holds, "{c:?}");
        assert!(c.detail.contains("ci passed on"), "{}", c.detail);
        assert!(c.detail.contains("Ada L"), "{}", c.detail);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn gate_passed_refuses_without_a_pass() {
        let (store, dir, repo, tip) = gated("gp-refuse", &["ci", "local"]);
        let c = gate_condition(&store, &repo);
        assert!(!c.holds);
        assert!(
            c.detail.contains("ci has no recorded result"),
            "{}",
            c.detail
        );
        assert!(c.fix.contains("kanbanr gate record"), "{}", c.fix);
        // a failure is named with its note
        record_gate(&store, "ci", &tip, false, "2026-11-04T10:00:00Z");
        let c = gate_condition(&store, &repo);
        assert!(!c.holds);
        assert!(c.detail.contains("ci failed on"), "{}", c.detail);
        assert!(c.detail.contains("two tests red"), "{}", c.detail);
        // every gate must pass: ci passed, local still has nothing
        record_gate(&store, "ci", &tip, true, "2026-11-04T11:00:00Z");
        let c = gate_condition(&store, &repo);
        assert!(!c.holds);
        assert!(
            c.detail.contains("local has no recorded result"),
            "{}",
            c.detail
        );
        record_gate(&store, "local", &tip, true, "2026-11-04T11:05:00Z");
        assert!(gate_condition(&store, &repo).holds);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_pass_for_another_commit_does_not_count() {
        let (store, dir, repo, _tip) = gated("gp-other", &["ci"]);
        record_gate(&store, "ci", &"d".repeat(40), true, "2026-11-04T10:00:00Z");
        let c = gate_condition(&store, &repo);
        assert!(!c.holds);
        assert!(c.detail.contains("no recorded result"), "{}", c.detail);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn gate_passed_does_not_cover_the_preparation_commit() {
        let (store, dir, repo, tip) = gated_after("gp-prep", &["ci"], |repo| {
            commit_file(
                repo,
                "Cargo.toml",
                "[package]\nversion = \"0.1.0\"\n",
                "manifest",
            );
        });
        record_gate(&store, "ci", &tip, true, "2026-11-04T10:00:00Z");
        // production now also prepares: the commit it adds cannot have a result yet
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        c.release_gates.get_mut("production").unwrap().on_enter = vec![
            crate::releasegates::ReleaseAction::Prepare,
            crate::releasegates::ReleaseAction::MoveMain,
            crate::releasegates::ReleaseAction::Tag,
        ];
        c.release_prepare = crate::yaml::from_str::<crate::config::Cadence>(
            "release_prepare: { version_files: [{ path: Cargo.toml }] }\n",
        )
        .unwrap()
        .release_prepare;
        store.set_cadence("shop", None, c).unwrap();
        let c = gate_condition(&store, &repo);
        assert!(!c.holds);
        assert!(c.detail.contains("preparation commit"), "{}", c.detail);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn no_gate_required_changes_nothing() {
        // No stage requires gate_passed and commit_gates is empty: the plan has no such
        // condition and the cadence reads and writes as before.
        let (store, dir, repo) = board_and_repo("gp-none");
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
        assert!(
            plan.conditions
                .iter()
                .all(|c| c.check != crate::releasegates::ReleaseCheck::GatePassed)
        );
        let c = store.load_meta("shop").unwrap().config.cadence;
        assert!(c.commit_gates.is_empty());
        assert!(!crate::yaml::to_string(&c).unwrap().contains("commit_gates"));
        // and a stage cannot require it while no gate is named
        let mut c = c;
        c.release_gates.insert(
            "production".into(),
            crate::releasegates::ReleaseGate {
                requires: vec![crate::releasegates::ReleaseCheck::GatePassed],
                ..Default::default()
            },
        );
        let err = c.validate_release_gates().unwrap_err();
        assert!(err.contains("kanbanr config commit-gates"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn gate_passed_reads_only_recorded_results() {
        // The judgement is a read of the board's own files: the same plan, with the repository's
        // remotes taken away and no network in reach, still judges from what was recorded.
        let (store, dir, repo, tip) = gated("gp-local", &["ci"]);
        assert!(!gate_condition(&store, &repo).holds);
        record_gate(&store, "ci", &tip, true, "2026-11-04T10:00:00Z");
        assert!(gate_condition(&store, &repo).holds);
        assert!(coderepo::git(&repo, &["remote"]).unwrap().trim().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
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
        plan_done(&store, "v2");
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
        // Pushed, but not yet fetched: this clone's origin/main does not know yet. A push by path
        // leaves origin/main alone on Linux, but Windows's git matches the path to `origin` and
        // moves it (FEAT-219), so it is put back to where it was.
        let before = plain_git(&repo, &["rev-parse", "refs/remotes/origin/main"]);
        plain_git(&repo, &["push", "-q", &bare.to_string_lossy(), "main:main"]);
        plain_git(&repo, &["update-ref", "refs/remotes/origin/main", &before]);
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
        plan_done(&store, "v2");
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
        plan_done(&store, "v0.2.0");
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

    /// DEF-005 R-1: the cut writes the notes file at the tagged commit to the board's release
    /// document; the generated text is only for a release with no such file.
    #[test]
    fn cut_copies_the_shipped_notes_to_the_board() {
        let (store, dir, repo) = board_and_repo("shipped-notes");
        prepare_fixture(&repo);
        super::add(&store, "shop", "v0.2.0", "", "Gates").unwrap();
        plan_done(&store, "v0.2.0");
        configure(
            &store,
            &format!(
                "release_stages: [candidate, production]\nrelease_gates:\n  \
                 candidate: {{ on_enter: [branch, prepare] }}\n  production: {{ on_enter: [tag] }}\n{PREPARE}"
            ),
        );
        super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        // The notes are written by hand on the release branch before the cut.
        let written = "# v0.2.0 — Gates\n\nWritten by hand, and shipped.\n";
        coderepo::commit_files(
            &repo,
            "release/v0.2.0",
            &[("docs/releases/v0.2.0.md".to_string(), written.to_string())],
            "notes",
        )
        .unwrap();
        let done = super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        assert_eq!(done.tagged, "v0.2.0");
        assert_eq!(done.notes.as_deref(), Some(written));
        assert_eq!(
            store.read_doc("shop", "releases/v0.2.0.md").unwrap(),
            written
        );
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

    /// FEAT-212 R-8: a file listed several times, each with its own match — a lockfile naming
    /// three of the project's crates — gets every entry applied, in order, and is written once, in
    /// the one preparation commit; the `prepared` check then holds for each entry.
    #[test]
    fn prepare_applies_every_entry_for_a_file() {
        let (store, dir, repo) = board_and_repo("prepare-lockfile");
        let pkg = |name: &str, version: &str| {
            format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\n")
        };
        let lock = format!(
            "version = 4\n\n{}{}{}{}",
            pkg("kanbanr-cli", "0.1.0"),
            pkg("kanbanr-core", "0.1.0"),
            pkg("kanbanr-server", "0.1.0"),
            pkg("serde", "0.1.0"),
        );
        commit_file(&repo, "Cargo.lock", &lock, "lockfile");
        let entry = |name: &str| {
            format!(
                "    - path: Cargo.lock\n      match: \"name = \\\"{name}\\\"\\nversion = \\\"{{version}}\\\"\"\n"
            )
        };
        configure(
            &store,
            &format!(
                "{THREE}release_prepare:\n  version_files:\n{}{}{}",
                entry("kanbanr-core"),
                entry("kanbanr-cli"),
                entry("kanbanr-server")
            ),
        );
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        let done = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo)).unwrap();
        let prepared = done.prepared.clone().unwrap();
        assert_eq!(prepared.files, ["Cargo.lock"], "written once");
        let tip = coderepo::rev_parse(&repo, "release/v1").unwrap();
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
        let got = coderepo::read_blob(&repo, &tip, "Cargo.lock")
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(
            got,
            format!(
                "version = 4\n\n{}{}{}{}",
                pkg("kanbanr-cli", "1"),
                pkg("kanbanr-core", "1"),
                pkg("kanbanr-server", "1"),
                pkg("serde", "0.1.0"),
            ),
            "every entry applied, nothing else touched"
        );
        let project = store.load_meta("shop").unwrap();
        let (ok, said) = super::prepared_at(&repo, &tip, &project, "v1", "release/v1");
        assert!(ok, "{said}");
        // Before preparation no entry is carried: the check fails, naming the file once.
        let (ok, said) = super::prepared_at(&repo, &main_before, &project, "v1", "release/v1");
        assert!(!ok);
        assert_eq!(said.matches("Cargo.lock").count(), 1, "{said}");
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

    /// A board and repository of this test's own, by canonical path, under the system temporary
    /// directory — never a real board.
    fn assert_own(store: &Store, dir: &Path, repo: &Path) {
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let board = store.data_dir().canonicalize().unwrap();
        assert_eq!(board, dir.canonicalize().unwrap(), "the test's own board");
        assert!(board.starts_with(&tmp), "{}", board.display());
        assert!(repo.canonicalize().unwrap().starts_with(&tmp));
    }

    /// A release promoted to candidate with `prepare`, its notes drafted on release/v0.2.0.
    fn notes_fixture(tag: &str) -> (Store, PathBuf, PathBuf) {
        let (store, dir, repo) = board_and_repo(tag);
        assert_own(&store, &dir, &repo);
        prepare_fixture(&repo);
        super::add(&store, "shop", "v0.2.0", "", "Gates").unwrap();
        plan_done(&store, "v0.2.0");
        configure(&store, &format!("{THREE}{PREPARE}"));
        super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        (store, dir, repo)
    }

    /// FEAT-216 R-1: a release's notes are read at the tip of release/<V>, from the path the
    /// config names, without checking it out; what is missing is said plainly.
    #[test]
    fn release_notes_read_from_the_release_branch() {
        let (store, dir, repo) = notes_fixture("notes-read");
        let tip = coderepo::rev_parse(&repo, "release/v0.2.0").unwrap();
        let notes = super::release_notes(&store, "shop", "v0.2.0", Some(&repo)).unwrap();
        assert_eq!(notes.path, "docs/releases/v0.2.0.md");
        assert_eq!(notes.branch, "release/v0.2.0");
        assert_eq!(notes.at, tip[..7]);
        assert!(notes.text.starts_with("# v0.2.0 — Gates"), "{}", notes.text);
        assert_eq!(
            notes.text,
            coderepo::read_blob(&repo, &tip, "docs/releases/v0.2.0.md")
                .unwrap()
                .unwrap()
                .1
        );
        assert!(notes.editable && notes.why_not.is_empty());
        // Nothing checked out: the notes are not in the working tree.
        assert_eq!(coderepo::current_branch(&repo).as_deref(), Some("main"));
        assert!(!repo.join("docs/releases/v0.2.0.md").exists());

        let err = |v: &str, r: Option<&Path>| {
            super::release_notes(&store, "shop", v, r)
                .unwrap_err()
                .to_string()
        };
        assert!(err("v9", Some(&repo)).contains("no release v9"));
        assert!(
            err("v0.2.0", None).contains("in the code repository"),
            "no repository"
        );
        super::add(&store, "shop", "v0.3.0", "", "").unwrap();
        assert!(
            err("v0.3.0", Some(&repo)).contains("promote it to candidate first"),
            "no branch yet"
        );
        // The file absent at the tip: a branch nothing has prepared.
        let mut rs = super::load(&store, "shop").unwrap();
        rs.iter_mut()
            .find(|r| r.version == "v0.3.0")
            .unwrap()
            .branch = "release/v0.3.0".into();
        super::save(&store, "shop", &rs).unwrap();
        coderepo::create_branch_from(&repo, "release/v0.3.0", "main").unwrap();
        let absent = err("v0.3.0", Some(&repo));
        assert!(
            absent.contains("docs/releases/v0.3.0.md is not on release/v0.3.0"),
            "{absent}"
        );
        // No notes path configured.
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        c.release_prepare.notes = None;
        store.set_cadence("shop", None, c).unwrap();
        assert!(err("v0.2.0", Some(&repo)).contains("no release notes path is configured"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-216 R-2: edited notes are one commit onto release/<V>, nothing checked out; the same
    /// text is no commit; a listing added to the config later is applied; a shipped release is
    /// refused.
    #[test]
    fn set_release_notes_commits_onto_the_release_branch() {
        let (store, dir, repo) = notes_fixture("notes-set");
        let main_before = coderepo::rev_parse(&repo, "main").unwrap();
        let tip = coderepo::rev_parse(&repo, "release/v0.2.0").unwrap();
        let text = "# v0.2.0 — Gates\n\nRewritten by hand.\n";
        let saved =
            super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), text, "Ada").unwrap();
        let (sha, notes) = (saved.sha, saved.notes);
        assert!(
            saved.main_moved.is_empty(),
            "main does not hold the release"
        );
        assert_eq!(coderepo::rev_parse(&repo, "release/v0.2.0").unwrap(), sha);
        assert_eq!(notes.at, sha[..7]);
        assert_eq!(notes.text, text);
        assert_eq!(
            coderepo::git(&repo, &["rev-list", "--count", &format!("{tip}..{sha}")]).unwrap(),
            "1",
            "one commit"
        );
        assert_eq!(
            coderepo::read_blob(&repo, &sha, "docs/releases/v0.2.0.md")
                .unwrap()
                .unwrap()
                .1,
            text
        );
        let message = coderepo::git(&repo, &["log", "-1", "--format=%B", &sha]).unwrap();
        assert!(message.starts_with("Release v0.2.0: notes\n"), "{message}");
        assert!(message.contains("Edited through kanbanr release notes by Ada."));
        assert!(message.contains("[no-ref] release notes, written by kanbanr"));
        assert!(message.contains("Kanbanr-Release: v0.2.0"));
        assert!(crate::scm::escape_reason(&message).is_some(), "{message}");
        assert_eq!(coderepo::current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_before);
        assert!(coderepo::is_clean(&repo));

        // The same text: no commit.
        let again = super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), text, "Ada")
            .unwrap()
            .sha;
        assert_eq!(again, sha, "nothing to change");
        assert_eq!(coderepo::rev_parse(&repo, "release/v0.2.0").unwrap(), sha);

        // A listing added to the config later reaches the candidate on the next save.
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        c.release_prepare
            .notes_listings
            .push(crate::releasegates::NotesListing {
                path: "docs/SUMMARY.md".into(),
                after: "- [Intro](intro.md)".into(),
                entry: "  - [Notes {number}](releases/{version}.md)".into(),
                ..Default::default()
            });
        store.set_cadence("shop", None, c).unwrap();
        let listed = super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), text, "Ada")
            .unwrap()
            .sha;
        assert_ne!(listed, sha);
        let summary = coderepo::read_blob(&repo, &listed, "docs/SUMMARY.md")
            .unwrap()
            .unwrap()
            .1;
        assert!(
            summary.contains("- [Intro](intro.md)\n  - [Notes 0.2.0](releases/v0.2.0.md)\n"),
            "{summary}"
        );
        assert_eq!(
            summary.matches("  - [0.2.0](releases/v0.2.0.md)").count(),
            1,
            "the first listing is not repeated: {summary}"
        );

        // Checked out in a worktree: refused, as the prepare commit is.
        let wt = repo.with_extension("wt");
        let _ = std::fs::remove_dir_all(&wt);
        coderepo::git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                &wt.to_string_lossy(),
                "release/v0.2.0",
            ],
        )
        .unwrap();
        let busy = super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), "x\n", "Ada")
            .unwrap_err()
            .to_string();
        assert!(busy.contains("is checked out in"), "{busy}");
        coderepo::git(
            &repo,
            &["worktree", "remove", "--force", &wt.to_string_lossy()],
        )
        .unwrap();

        // Shipped: refused, and read as shipped.
        let mut rs = super::load(&store, "shop").unwrap();
        rs.iter_mut().find(|r| r.version == "v0.2.0").unwrap().state = super::ReleaseState::Shipped;
        super::save(&store, "shop", &rs).unwrap();
        let shipped = super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), "y\n", "Ada")
            .unwrap_err()
            .to_string();
        assert!(shipped.contains("has shipped"), "{shipped}");
        assert_eq!(
            coderepo::rev_parse(&repo, "release/v0.2.0").unwrap(),
            listed
        );
        let read = super::release_notes(&store, "shop", "v0.2.0", Some(&repo)).unwrap();
        assert!(!read.editable && read.why_not.contains("has shipped"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-216 T6 (v0.3.0, 22 Oct 2026): notes saved once main holds the release move main with
    /// them, and say to push it again; with main past the release, nothing is saved.
    #[test]
    fn saving_notes_moves_main_that_holds_the_release() {
        let (store, dir, repo) = notes_fixture("notes-main");
        let tip = coderepo::rev_parse(&repo, "release/v0.2.0").unwrap();
        coderepo::fast_forward(&repo, "main", &tip).unwrap();
        let saved = super::set_release_notes(
            &store,
            "shop",
            "v0.2.0",
            Some(&repo),
            "# v0.2.0\n\nAfter main moved.\n",
            "Ada",
        )
        .unwrap();
        assert_ne!(saved.sha, tip);
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), saved.sha);
        assert!(
            saved.main_moved.contains("main held v0.2.0")
                && saved.main_moved.contains("push main again"),
            "{}",
            saved.main_moved
        );
        assert!(
            coderepo::is_clean(&repo),
            "the checked-out main moved cleanly"
        );

        // Main past the release: refused, and neither branch moves.
        coderepo::git(&repo, &["commit", "-q", "--allow-empty", "-m", "on main"]).unwrap();
        let main_now = coderepo::rev_parse(&repo, "main").unwrap();
        let past = super::set_release_notes(&store, "shop", "v0.2.0", Some(&repo), "z\n", "Ada")
            .unwrap_err()
            .to_string();
        assert!(
            past.contains("past release/v0.2.0") && past.contains("nothing was saved"),
            "{past}"
        );
        assert_eq!(
            coderepo::rev_parse(&repo, "release/v0.2.0").unwrap(),
            saved.sha
        );
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), main_now);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// The notes and both changelogs of v0.2.0 dated `ymd`, committed onto its release branch, as
    /// a candidate prepared on an earlier day has them.
    fn date_candidate(repo: &Path, ymd: &str) -> String {
        let tip = coderepo::rev_parse(repo, "release/v0.2.0").unwrap();
        let at = |path: &str| coderepo::read_blob(repo, &tip, path).unwrap().unwrap().1;
        let files: Vec<(String, String)> = vec![
            (
                "docs/releases/v0.2.0.md".to_string(),
                crate::releaseprep::redate_notes(&at("docs/releases/v0.2.0.md"), ymd).unwrap(),
            ),
            (
                "CHANGELOG.md".to_string(),
                crate::releaseprep::redate_changelog(&at("CHANGELOG.md"), "0.2.0", ymd).unwrap(),
            ),
            (
                "docs/changelog.md".to_string(),
                crate::releaseprep::redate_changelog(&at("docs/changelog.md"), "0.2.0", ymd)
                    .unwrap(),
            ),
        ];
        coderepo::commit_files(repo, "release/v0.2.0", &files, "an earlier day")
            .unwrap()
            .1
    }

    /// DEF-001 R-1: the stage that moves main dates the notes and the changelog today, in one
    /// commit onto the release branch, before main moves with it; the plan says so; the prepared
    /// check still holds afterwards.
    #[test]
    fn move_main_redates_the_release() {
        let (store, dir, repo) = notes_fixture("redate-move");
        let old = date_candidate(&repo, "2026-01-01");
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
        let step = plan.steps.iter().find(|s| s.action.name() == "move_main");
        assert!(
            step.unwrap().does.starts_with("dates the release "),
            "{:?}",
            step
        );
        let done = super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        let tip = coderepo::rev_parse(&repo, "release/v0.2.0").unwrap();
        assert_ne!(tip, old, "one commit dated the release");
        assert_eq!(
            coderepo::git(&repo, &["rev-list", "--count", &format!("{old}..{tip}")]).unwrap(),
            "1"
        );
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), tip);
        assert_eq!(done.main_moved.unwrap().1, tip);
        let today = super::today_ymd();
        let at = |path: &str| coderepo::read_blob(&repo, &tip, path).unwrap().unwrap().1;
        assert!(at("docs/releases/v0.2.0.md").contains(&format!("_Released {today}._")));
        for c in ["CHANGELOG.md", "docs/changelog.md"] {
            assert!(at(c).contains(&format!("## [0.2.0] - {today}")), "{c}");
        }
        let message = coderepo::git(&repo, &["log", "-1", "--format=%B", &tip]).unwrap();
        assert!(
            message.starts_with(&format!("Release v0.2.0: dated {today}\n")),
            "{message}"
        );
        assert!(message.contains("[no-ref]") && message.contains("Kanbanr-Release: v0.2.0"));
        assert!(crate::scm::escape_reason(&message).is_some(), "{message}");
        let project = store.load_meta("shop").unwrap();
        assert!(super::prepared_at(&repo, &tip, &project, "v0.2.0", "release/v0.2.0").0);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-001 R-2: the cut's plan warns of a stale date, naming it and today and the command,
    /// without refusing; `redate` commits today's date, moves main with it and says to push
    /// again; with nothing stale it commits nothing; once shipped it refuses.
    #[test]
    fn cut_plan_warns_of_a_stale_date() {
        let (store, dir, repo) = notes_fixture("redate-warn");
        super::promote(&store, "shop", "v0.2.0", None, "Ada", "", Some(&repo)).unwrap();
        let today = super::today_ymd();
        let plan = |to: Option<&str>| {
            super::plan_promotion(
                &store,
                "shop",
                "v0.2.0",
                to,
                "",
                Some(&repo),
                super::Fetch::No,
            )
            .unwrap()
        };
        assert!(plan(None).warnings.is_empty(), "dated today");
        let old = date_candidate(&repo, "2026-01-01");
        // Main held the release; it follows the stale commit, as it would have in time.
        coderepo::fast_forward(&repo, "main", &old).unwrap();
        let cut = plan(None);
        assert!(cut.cut && cut.warnings.len() == 1, "{:?}", cut.warnings);
        let said = &cut.warnings[0];
        assert!(
            said.contains("2026-01-01")
                && said.contains(&today)
                && said.contains("kanbanr release redate v0.2.0"),
            "{said}"
        );

        let saved = super::redate_release(&store, "shop", "v0.2.0", Some(&repo), "Ada").unwrap();
        assert_ne!(saved.sha, old);
        assert_eq!(
            coderepo::rev_parse(&repo, "release/v0.2.0").unwrap(),
            saved.sha
        );
        assert_eq!(coderepo::rev_parse(&repo, "main").unwrap(), saved.sha);
        assert!(
            saved.main_moved.contains("push main again"),
            "{}",
            saved.main_moved
        );
        assert!(saved.notes.text.contains(&format!("_Released {today}._")));
        assert!(plan(None).warnings.is_empty(), "no longer stale");
        let again = super::redate_release(&store, "shop", "v0.2.0", Some(&repo), "Ada").unwrap();
        assert_eq!(again.sha, saved.sha, "nothing to date");

        let mut rs = super::load(&store, "shop").unwrap();
        rs.iter_mut().find(|r| r.version == "v0.2.0").unwrap().state = super::ReleaseState::Shipped;
        super::save(&store, "shop", &rs).unwrap();
        let shipped = super::redate_release(&store, "shop", "v0.2.0", Some(&repo), "Ada")
            .unwrap_err()
            .to_string();
        assert!(shipped.contains("has shipped"), "{shipped}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
    // ---- FTR-026: release stages that follow the tag ----

    /// Candidate branches, main moves, `tag` tags; `staging` after it only confirms.
    const STAGED: &str = "release_stages: [candidate, main, tag, staging]\n\
         release_gates:\n  candidate: { on_enter: [branch] }\n  \
         main: { on_enter: [move_main] }\n  tag: { on_enter: [tag] }\n";

    fn staged(name: &str, yaml: &str) -> (Store, PathBuf, PathBuf) {
        let (store, dir, repo) = board_and_repo(name);
        configure(&store, yaml);
        let mut c = store.load_meta("shop").unwrap().config.cadence;
        c.commit_gates = vec!["ci".into()];
        store.set_cadence("shop", None, c).unwrap();
        (store, dir, repo)
    }

    fn go(store: &Store, repo: &Path, to: &str) -> super::Promoted {
        super::promote(store, "shop", "v1", Some(to), "Ada", "", Some(repo)).unwrap()
    }

    fn v1(store: &Store) -> super::Release {
        super::load(store, "shop")
            .unwrap()
            .into_iter()
            .find(|r| r.version == "v1")
            .unwrap()
    }

    fn tagged(name: &str, yaml: &str) -> (Store, PathBuf, PathBuf, String) {
        let (store, dir, repo) = staged(name, yaml);
        go(&store, &repo, "candidate");
        go(&store, &repo, "main");
        go(&store, &repo, "tag");
        let sha = v1(&store).tag_commit;
        (store, dir, repo, sha)
    }

    fn now_result(store: &Store, commit: &str, passed: bool) {
        record_gate(store, "ci", commit, passed, &crate::now_rfc3339());
    }

    fn plan_to(store: &Store, repo: &Path, to: &str) -> super::ReleasePlan {
        super::plan_promotion(
            store,
            "shop",
            "v1",
            Some(to),
            "",
            Some(repo),
            super::Fetch::No,
        )
        .unwrap()
    }

    fn clean(dir: &Path, repo: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(repo);
    }

    /// FTR-026 R-1: the stage that tags records `tagged_at` and the tag's commit, and ships nothing.
    #[test]
    fn the_tag_stage_tags_but_does_not_ship_when_stages_follow() {
        let (store, dir, repo) = staged("ftr026-tag", STAGED);
        go(&store, &repo, "candidate");
        go(&store, &repo, "main");
        let t = go(&store, &repo, "tag");
        assert_eq!(t.tagged, "v1");
        let r = v1(&store);
        assert_eq!(r.state, super::ReleaseState::Planned);
        assert!(!r.tagged_at.is_empty());
        assert_eq!(r.tag_commit, coderepo::rev_parse(&repo, "v1").unwrap());
        assert_eq!((r.stage.as_str(), r.shipped_at.as_str()), ("tag", ""));
        assert!(coderepo::tag_exists(&repo, "v1"));
        assert!(t.notes.is_none() && t.preview.is_empty());
        clean(&dir, &repo);
    }

    /// FTR-026 R-1: between the tag and the last stage the items stay where they are.
    #[test]
    fn items_stay_held_between_the_tag_and_the_last_stage() {
        let (store, dir, repo, sha) = tagged("ftr026-held", STAGED);
        let before = store.load("shop").unwrap().features.clone();
        let r = v1(&store);
        assert!(r.shipped.is_empty() && r.notes_doc.is_empty() && r.carried.is_empty());
        let after = store.load("shop").unwrap().features;
        assert_eq!(
            before
                .iter()
                .map(|f| (&f.code, &f.status, &f.release))
                .collect::<Vec<_>>(),
            after
                .iter()
                .map(|f| (&f.code, &f.status, &f.release))
                .collect::<Vec<_>>()
        );
        // The plan for the last stage is a confirmation that ships, and waits for a result.
        let plan = plan_to(&store, &repo, "staging");
        assert!(plan.confirm && plan.ships && !plan.cut && !plan.ready);
        assert_eq!(plan.button, "Confirm staging: ship v1");
        now_result(&store, &sha, true);
        assert!(plan_to(&store, &repo, "staging").ready);
        clean(&dir, &repo);
    }

    /// FTR-026 R-2: a confirmation is a stage change with who, when and the note, and runs nothing.
    #[test]
    fn a_confirmation_records_who_when_and_the_note() {
        let (store, dir, repo, sha) = tagged(
            "ftr026-note",
            "release_stages: [candidate, main, tag, test, staging]\n\
             release_gates:\n  candidate: { on_enter: [branch] }\n  \
             main: { on_enter: [move_main] }\n  tag: { on_enter: [tag] }\n",
        );
        now_result(&store, &sha, true);
        let plan = plan_to(&store, &repo, "test");
        assert!(
            plan.confirm && !plan.ships && plan.steps.is_empty(),
            "{plan:#?}"
        );
        assert_eq!(plan.button, "Confirm test");
        let done = super::promote(
            &store,
            "shop",
            "v1",
            Some("test"),
            "Grace",
            "smoke test passed",
            Some(&repo),
        )
        .unwrap();
        assert!(done.steps.is_empty());
        let r = v1(&store);
        let last = r.stages.last().unwrap();
        assert_eq!(
            (last.stage.as_str(), last.by.as_str(), last.reason.as_str()),
            ("test", "Grace", "smoke test passed")
        );
        assert!(!last.at.is_empty());
        assert_eq!(r.state, super::ReleaseState::Planned);
        clean(&dir, &repo);
    }

    /// FTR-026 R-3: entering the last stage ships the release: items move, notes are written.
    #[test]
    fn the_last_stage_ships_the_release_and_moves_the_items() {
        let (store, dir, repo, sha) = tagged("ftr026-ship", STAGED);
        let code = store
            .load("shop")
            .unwrap()
            .features
            .iter()
            .find(|f| f.release.as_deref() == Some("v1"))
            .unwrap()
            .code
            .clone();
        now_result(&store, &sha, true);
        let done =
            super::promote(&store, "shop", "v1", None, "Ada", "all good", Some(&repo)).unwrap();
        assert_eq!(done.release.state, super::ReleaseState::Shipped);
        assert!(!done.release.shipped_at.is_empty());
        assert_eq!(done.release.shipped, vec![code]);
        assert_eq!(done.release.stage, "staging");
        assert!(done.notes.is_some() && !done.release.notes_doc.is_empty());
        assert_eq!(done.release.tag_commit, sha);
        clean(&dir, &repo);
    }

    /// FTR-026 R-3, R-8: a project whose tag stage is the last tags and ships in one move, as
    /// before.
    #[test]
    fn a_project_whose_tag_stage_is_last_ships_at_the_tag_as_before() {
        let (store, dir, repo) = board_and_repo("ftr026-asbefore");
        let plan = plan_to(&store, &repo, "production");
        assert!(plan.cut && plan.ships && !plan.confirm);
        go(&store, &repo, "candidate");
        let cut = go(&store, &repo, "production");
        assert_eq!(cut.release.state, super::ReleaseState::Shipped);
        assert_eq!(cut.tagged, "v1");
        assert!(coderepo::tag_exists(&repo, "v1"));
        assert_eq!(cut.release.shipped.len(), 1);
        clean(&dir, &repo);
    }

    /// FTR-026 R-8: a release written before the tag fields existed reads and writes back unchanged.
    #[test]
    fn an_older_board_reads_without_change() {
        let old = "- version: v0.9.0\n  state: shipped\n  shipped_at: 2026-10-01T00:00:00Z\n  \
                   shipped:\n  - FEAT-001\n  stage: production\n";
        let releases: Vec<super::Release> = crate::yaml::from_str(old).unwrap();
        assert!(releases[0].tagged_at.is_empty() && releases[0].superseded_by.is_empty());
        assert_eq!(crate::yaml::to_string(&releases).unwrap(), old);
    }

    /// FTR-026 R-4: the plan carries the tag's newest recorded result since the tag.
    #[test]
    fn the_plan_carries_the_tag_result() {
        let (store, dir, repo, sha) = tagged("ftr026-result", STAGED);
        let tag = |store: &Store| plan_to(store, &repo, "staging").tag.unwrap();
        let t = tag(&store);
        assert_eq!(
            (t.state.as_str(), t.commit.as_str(), t.gate.as_str()),
            ("none", sha.as_str(), "ci")
        );
        // A pass from before the tag does not stand in for a run on the tagged commit.
        record_gate(&store, "ci", &sha, true, "2020-01-01T00:00:00Z");
        assert_eq!(tag(&store).state, "none");
        now_result(&store, &sha, false);
        let t = tag(&store);
        assert_eq!(t.state, "failed");
        assert_eq!(t.result.unwrap().note, "two tests red");
        now_result(&store, &sha, true);
        assert_eq!(tag(&store).state, "passed");
        // A release not tagged yet has none.
        let (store2, dir2, repo2) = staged("ftr026-untagged", STAGED);
        go(&store2, &repo2, "candidate");
        assert!(plan_to(&store2, &repo2, "main").tag.is_none());
        clean(&dir, &repo);
        clean(&dir2, &repo2);
    }

    /// FTR-026 R-5: a red or missing result withholds the next stage.
    #[test]
    fn a_red_tag_result_withholds_the_next_stage() {
        let (store, dir, repo, sha) = tagged("ftr026-red", STAGED);
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no result recorded for the tag since it was made"),
            "{err}"
        );
        now_result(&store, &sha, false);
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("ci failed on") && err.contains("two tests red"),
            "{err}"
        );
        assert!(err.contains("start a patch release"), "{err}");
        assert_eq!(v1(&store).stage, "tag");
        clean(&dir, &repo);
    }

    /// FTR-026 R-6: a patch from a red tag files a defect, plans it and the items into the patch,
    /// and supersedes the parent.
    #[test]
    fn a_patch_from_a_red_tag_files_a_defect_and_supersedes_the_parent() {
        let (store, dir, repo, sha) = tagged("ftr026-patch", STAGED);
        let item = store.load("shop").unwrap().features[0].code.clone();
        let refused = super::patch_from_failed_tag(
            &store,
            "shop",
            "v1",
            "v1.0.1",
            "Ada",
            "",
            "",
            Some(&repo),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("no failed result"), "{refused}");
        now_result(&store, &sha, true);
        assert!(
            super::patch_from_failed_tag(
                &store,
                "shop",
                "v1",
                "v1.0.1",
                "Ada",
                "",
                "",
                Some(&repo)
            )
            .is_err()
        );
        now_result(&store, &sha, false);
        let made = super::patch_from_failed_tag(
            &store,
            "shop",
            "v1",
            "v1.0.1",
            "Ada",
            "https://ci.example/run/9",
            "two tests red on arm",
            Some(&repo),
        )
        .unwrap();
        assert_eq!(made.release.parent, "v1");
        assert!(coderepo::branch_exists(&repo, "release/v1.0.1"));
        assert_eq!(made.planned, vec![made.defect.clone(), item.clone()]);
        let board = store.load("shop").unwrap();
        let defect = board
            .features
            .iter()
            .find(|f| f.code == made.defect)
            .unwrap();
        assert_eq!(defect.release.as_deref(), Some("v1.0.1"));
        assert_eq!(defect.kind.as_deref(), Some("defect"));
        assert!(defect.specification.contains("https://ci.example/run/9"));
        assert!(defect.specification.contains("two tests red on arm"));
        let moved = board.features.iter().find(|f| f.code == item).unwrap();
        assert_eq!(moved.release.as_deref(), Some("v1.0.1"));
        let parent = v1(&store);
        assert_eq!(parent.superseded_by, "v1.0.1");
        assert!(coderepo::tag_exists(&repo, "v1"), "the tag stays");
        let err = super::promote(&store, "shop", "v1", None, "Ada", "", Some(&repo))
            .unwrap_err()
            .to_string();
        assert!(err.contains("superseded by v1.0.1"), "{err}");
        clean(&dir, &repo);
    }
}
