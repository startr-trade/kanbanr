//! The git model a project's cadence implies (FEAT-189).
//!
//! One vocabulary for where code goes, shared by `start`, the hooks, sprints and releases:
//! - **main**: `cadence.main_branch`, else the repository's default branch. Production is what
//!   main holds.
//! - **item branches**: `feat/<CODE>-<slug>` (the project's pattern), its `fix/` twin for defects,
//!   and `spike/*`.
//! - the **line of integration**, where finished items are merged, which follows the topology:
//!
//! | Topology | Cadence | Line | Production at |
//! |---|---|---|---|
//! | trunk | neither | main | the integrate merge |
//! | sprints | sprints only | `sprint/<SP>` of the active sprint | sprint close |
//! | releases | releases only | the integration branch (`integration`) | release cut |
//! | sprints and releases | both | `sprint/<SP>` of the active sprint | release cut |
//!
//! - **release branches**: `release/<V>`, made when a release reaches its first stage.
//! - **protected branches**: main, the integration branch, `sprint/*`, `release/*`: they take only
//!   merges that say which items they bring.
//!
//! The model is always derived and can always be shown. It changes what happens — where `start`
//! branches, sprint branches, protection of lines — only when `cadence.git_model` is on (Q1).
//! Nothing here ever pushes.

use crate::Store;
use crate::config::{Cadence, ProjectConfig, Role};
use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::releases::Release;
use crate::scm::{RELEASE_PREFIX, SPIKE_PREFIX, SPRINT_PREFIX};
use crate::sprints::{Sprint, SprintState};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The integration branch a releases-only project uses when it names none (Q5).
pub const DEFAULT_INTEGRATION_BRANCH: &str = "integration";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    Trunk,
    Sprints,
    Releases,
    SprintsAndReleases,
}

/// Where production moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Production {
    IntegrateMerge,
    SprintClose,
    ReleaseCut,
}

/// Facts read from the code repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BranchFacts {
    pub main: String,
    /// Local branch → tip.
    pub branches: BTreeMap<String, String>,
    /// Branches main already contains.
    pub merged_into_main: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitModel {
    pub topology: Topology,
    pub production: Production,
    /// `cadence.git_model`: whether the model drives branching, or only describes it.
    pub automated: bool,
    pub main: String,
    /// The releases topology's line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_branch: Option<String>,
    /// The current line; None with sprints on and no sprint running.
    pub line: Option<String>,
    pub item_patterns: Vec<String>,
    pub protected: Vec<String>,
}

pub fn topology(c: &Cadence) -> Topology {
    match (c.sprints, c.releases) {
        (false, false) => Topology::Trunk,
        (true, false) => Topology::Sprints,
        (false, true) => Topology::Releases,
        (true, true) => Topology::SprintsAndReleases,
    }
}

/// A sprint's branch.
pub fn sprint_branch(code: &str) -> String {
    format!("{SPRINT_PREFIX}{code}")
}

/// The model of a project, from its config, its sprints and what its repository holds.
pub fn derive(config: &ProjectConfig, sprints: &[Sprint], facts: &BranchFacts) -> GitModel {
    let c = &config.cadence;
    let topology = topology(c);
    let main = c
        .main_branch
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| facts.main.clone());
    let integration_branch = (topology == Topology::Releases).then(|| {
        c.integration_branch
            .clone()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INTEGRATION_BRANCH.to_string())
    });
    let active = crate::sprints::active(sprints).map(|s| sprint_branch(&s.code));
    let line = match topology {
        Topology::Trunk => Some(main.clone()),
        Topology::Releases => integration_branch.clone(),
        Topology::Sprints | Topology::SprintsAndReleases => active,
    };
    let production = match topology {
        Topology::Trunk => Production::IntegrateMerge,
        Topology::Sprints => Production::SprintClose,
        Topology::Releases | Topology::SprintsAndReleases => Production::ReleaseCut,
    };
    let pattern = config.branch_pattern().to_string();
    let mut item_patterns = vec![pattern.clone()];
    item_patterns.extend(crate::scm::fix_twin(&pattern));
    item_patterns.push(format!("{SPIKE_PREFIX}*"));
    let automated = c.git_model;
    let mut protected = vec![main.clone()];
    if automated {
        protected.extend(integration_branch.clone());
        protected.push(format!("{SPRINT_PREFIX}*"));
        protected.push(format!("{RELEASE_PREFIX}*"));
    }
    GitModel {
        topology,
        production,
        automated,
        main,
        integration_branch,
        line,
        item_patterns,
        protected,
    }
}

/// The model and the facts it was derived from, for a project and its repository.
pub fn load(store: &Store, id: &str, repo: &Path) -> Result<(GitModel, BranchFacts)> {
    let config = store.load_meta(id)?.config;
    let sprints = crate::sprints::load(store, id)?;
    let main = config
        .cadence
        .main_branch
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| crate::coderepo::default_branch(repo));
    let facts = crate::coderepo::branch_facts(repo, &main)?;
    Ok((derive(&config, &sprints, &facts), facts))
}

/// The line an item integrates into: its release's branch when it is planned into an unshipped
/// release that has one (a fix during test), else the current line.
pub fn line_for(model: &GitModel, item: &FeatureItem, releases: &[Release]) -> Option<String> {
    if let Some(version) = item.release.as_deref()
        && let Some(r) = releases.iter().find(|r| r.version == version)
        && r.state != crate::releases::ReleaseState::Shipped
        && !r.branch.is_empty()
    {
        return Some(r.branch.clone());
    }
    model.line.clone()
}

/// Where an item's branch starts, and what has to exist first (FEAT-189, R-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartBase {
    /// The branch to branch from.
    pub base: String,
    /// The line has not been made yet: make it from `main` first.
    pub create_from_main: bool,
    /// Something the person should know, e.g. that integrating will need a sprint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Where `item` starts: its line (its release's branch for a fix during test), made from main if
/// it is not there yet; with sprints on and none running, the sprint base, and a note that
/// integrating needs a sprint (Q2).
pub fn start_base(
    model: &GitModel,
    facts: &BranchFacts,
    item: &FeatureItem,
    releases: &[Release],
    sprints: &[Sprint],
) -> StartBase {
    match line_for(model, item, releases) {
        Some(line) => StartBase {
            create_from_main: line != model.main && !facts.branches.contains_key(&line),
            base: line,
            note: None,
        },
        None => StartBase {
            base: sprint_base(sprints, facts),
            create_from_main: false,
            note: Some(
                "no sprint is running: the item can be built, but integrating it needs a sprint                  (`kanbanr sprint start <SP>`)"
                    .into(),
            ),
        },
    }
}

/// A sprint branch that exists and main does not yet contain.
fn unreleased(sprint: &Sprint, facts: &BranchFacts) -> Option<String> {
    let b = sprint_branch(&sprint.code);
    (facts.branches.contains_key(&b) && !facts.merged_into_main.contains(&b)).then_some(b)
}

/// Where `sprint/<SP>` is made: the latest sprint branch main does not contain yet, else main —
/// so a sprint builds on work that has not shipped.
pub fn sprint_base(sprints: &[Sprint], facts: &BranchFacts) -> String {
    sprints
        .iter()
        .rev()
        .find_map(|s| unreleased(s, facts))
        .unwrap_or_else(|| facts.main.clone())
}

/// Where `release/<V>` is made: the releases topology's integration branch; with sprints, the most
/// recently closed sprint that has not shipped, else the active sprint's branch; trunk, main.
pub fn release_base(model: &GitModel, sprints: &[Sprint], facts: &BranchFacts) -> String {
    match model.topology {
        Topology::Trunk => model.main.clone(),
        Topology::Releases => model
            .integration_branch
            .clone()
            .unwrap_or_else(|| model.main.clone()),
        Topology::Sprints | Topology::SprintsAndReleases => sprints
            .iter()
            .rev()
            .filter(|s| s.state == SprintState::Closed)
            .find_map(|s| unreleased(s, facts))
            .or_else(|| crate::sprints::active(sprints).map(|s| sprint_branch(&s.code)))
            .unwrap_or_else(|| model.main.clone()),
    }
}

/// Does `branch` take merges only?
pub fn is_protected(model: &GitModel, branch: &str) -> bool {
    model.protected.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => branch.starts_with(prefix),
        None => p == branch,
    })
}

/// What the hooks say about a commit on a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineVerdict {
    /// Not a line of integration: the item-branch rules apply.
    NotALine,
    Allowed,
    Refused(String),
}

/// A commit on `branch`: a line of integration takes only merges, and a merge must say which
/// items it brings — in a `Refs:` trailer, or `[no-ref] <why>` on the record (Q4). `message` is
/// None where it is not known yet (pre-commit); commit-msg passes it.
pub fn line_commit_verdict(
    model: &GitModel,
    branch: &str,
    merging: bool,
    message: Option<&str>,
) -> LineVerdict {
    if !is_protected(model, branch) {
        return LineVerdict::NotALine;
    }
    if !merging {
        return LineVerdict::Refused(format!(
            "{branch} is a line of integration: work belongs on an item branch (kanbanr start \
             <CODE>); it receives merges only"
        ));
    }
    let Some(message) = message else {
        return LineVerdict::Allowed;
    };
    if !crate::scm::parse_refs(message).is_empty() || crate::scm::escape_reason(message).is_some() {
        return LineVerdict::Allowed;
    }
    LineVerdict::Refused(format!(
        "a merge into {branch} must say which items it brings: Refs: kanbanr:<CODE> (or \
         `[no-ref] <why>`)"
    ))
}

/// Is an item at `status` integrated? At the integrate or release stage, or at an end that is not
/// a no-op.
pub fn is_integrated(config: &ProjectConfig, status: &str) -> bool {
    let roles = config.roles();
    roles.status(Role::Integrate) == Some(status)
        || roles.status(Role::Release) == Some(status)
        || (crate::graph::is_terminal_status(config, status) && !config.is_no_op(status))
}

/// What starting a sprint did.
#[derive(Debug, Clone, Serialize)]
pub struct SprintStarted {
    pub sprint: Sprint,
    /// `(branch, base)` when a sprint branch was made; None when none was (automation off) or it
    /// already existed (`reused`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused: Option<String>,
    /// Carried items brought up to date with the new sprint (FEAT-190).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub catch_up: Vec<CatchUp>,
}

/// Start a sprint, with its branch when the model is automated: preflight, then the branch, then
/// the board, taking the branch away again if the board step fails.
pub fn sprint_start(
    store: &Store,
    id: &str,
    code: &str,
    repo: Option<&Path>,
) -> Result<SprintStarted> {
    let config = store.load_meta(id)?.config;
    if !config.cadence.git_model {
        return Ok(SprintStarted {
            sprint: crate::sprints::start(store, id, code)?,
            branch: None,
            reused: None,
            catch_up: Vec::new(),
        });
    }
    let repo = repo.ok_or_else(|| {
        CoreError::Unsupported(format!(
            "starting {code} makes its branch, which needs the code repository: run it from the \
             project folder"
        ))
    })?;
    // Preflight the board before touching the repository.
    let sprints = crate::sprints::load(store, id)?;
    let sprint = sprints
        .iter()
        .find(|s| s.code == code)
        .ok_or_else(|| CoreError::Unsupported(format!("no sprint {code}")))?;
    if sprint.state == SprintState::Closed {
        return Err(CoreError::Unsupported(format!("{code} is already closed")));
    }
    if let Some(running) = crate::sprints::active(&sprints)
        && running.code != code
    {
        return Err(CoreError::Unsupported(format!(
            "{} is still active — close it first (`kanbanr sprint close {}`)",
            running.code, running.code
        )));
    }
    let (_, facts) = load(store, id, repo)?;
    let branch = sprint_branch(code);
    if facts.branches.contains_key(&branch) {
        let sprint = crate::sprints::start(store, id, code)?;
        return Ok(SprintStarted {
            sprint,
            branch: None,
            reused: Some(branch),
            catch_up: catch_up(store, id, code, repo).unwrap_or_default(),
        });
    }
    let base = sprint_base(&sprints, &facts);
    crate::coderepo::create_branch_from(repo, &branch, &base)?;
    match crate::sprints::start(store, id, code) {
        Ok(sprint) => Ok(SprintStarted {
            sprint,
            branch: Some((branch, base)),
            reused: None,
            catch_up: catch_up(store, id, code, repo).unwrap_or_default(),
        }),
        Err(e) => {
            let _ = crate::coderepo::delete_branch(repo, &branch);
            Err(e)
        }
    }
}

// ---- merges driven by the board (FEAT-190) ------------------------------------------------------

/// The local branches that name `code`, through the project's pattern or its `fix/` twin.
fn item_branches(config: &ProjectConfig, facts: &BranchFacts, code: &str) -> Vec<String> {
    facts
        .branches
        .keys()
        .filter(|b| {
            crate::scm::item_code_from_branch(config.branch_pattern(), b).as_deref() == Some(code)
        })
        .cloned()
        .collect()
}

/// A merge kanbanr made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Merged {
    pub branch: String,
    pub into: String,
    pub sha: String,
    /// The line already held the branch: nothing was merged.
    pub already: bool,
}

/// A move, and the merge that integrating it made.
#[derive(Debug, Clone, Serialize)]
pub struct Moved {
    pub feature: FeatureItem,
    pub warnings: Vec<crate::readiness::Gap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<Merged>,
    /// The item was integrated into `release/<V>`: the line lacks the fix until `kanbanr git
    /// forward <V>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_needed: Option<String>,
    /// Something to know, e.g. that the item has no branch, so nothing was merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Move an item. With the git model on, a move that integrates it — into the integrate stage, or
/// past it — merges its branch into its line first, refusing before anything changes if the merge
/// would conflict (R-1, R-2). Preflight, then git, then the board, putting the line back if the
/// board step fails.
pub fn move_item(
    store: &Store,
    id: &str,
    code: &str,
    to: &str,
    reason: Option<&str>,
    repo: Option<&Path>,
) -> Result<Moved> {
    let project = store.load_meta(id)?;
    let config = &project.config;
    let feature = project.feature(code)?.clone();
    let integrates = config.cadence.git_model
        && !config.is_no_op(to)
        && !is_integrated(config, &feature.status)
        && is_integrated(config, to);
    let plain = |note: Option<String>| -> Result<Moved> {
        let (feature, warnings) = store.move_feature_gated(id, code, to, reason)?;
        Ok(Moved {
            feature,
            warnings,
            merged: None,
            forward_needed: None,
            note,
        })
    };
    if !integrates {
        return plain(None);
    }
    let repo = repo.ok_or_else(|| {
        CoreError::Unsupported(format!(
            "integrating {code} merges its branch: run it from the project folder"
        ))
    })?;
    // Preflight: nothing is written until every reason to refuse has been looked for.
    store.gate_preview(id, code, to, reason)?;
    let (model, facts) = load(store, id, repo)?;
    let branches = item_branches(config, &facts, code);
    let branch = match branches.as_slice() {
        [] => return plain(Some(format!("{code} has no branch: nothing merged"))),
        [one] => one.clone(),
        many => {
            return Err(CoreError::Unsupported(format!(
                "{code} has more than one branch ({}): keep one, then move it again",
                many.join(", ")
            )));
        }
    };
    let releases = crate::releases::load(store, id).unwrap_or_default();
    let line = line_for(&model, &feature, &releases).ok_or_else(|| {
        CoreError::Unsupported(format!(
            "no sprint is running, so {code} has no line to integrate into: start a sprint first \
             (`kanbanr sprint start <SP>`)"
        ))
    })?;
    let make_line = !facts.branches.contains_key(&line);
    if make_line && line == model.main {
        return Err(CoreError::Unsupported(format!(
            "{line} does not exist in {}",
            repo.display()
        )));
    }
    let already = !make_line && crate::coderepo::is_ancestor(repo, &branch, &line);
    if !already {
        let into = if make_line {
            model.main.as_str()
        } else {
            line.as_str()
        };
        let preview = crate::coderepo::merge_preview(repo, into, &branch)?;
        if !preview.clean {
            return Err(CoreError::Unsupported(format!(
                "cannot integrate {code}: merging {branch} into {line} conflicts in: {}. Merge \
                 {line} into {branch}, resolve, commit, then move again.",
                preview.conflicts.join(", ")
            )));
        }
        if let Some(worktree) = crate::coderepo::worktree_of(repo, &line)
            && !crate::coderepo::is_clean(&worktree)
        {
            return Err(CoreError::Unsupported(format!(
                "{line} is checked out in {} with uncommitted changes: commit or stash them, then \
                 move again",
                worktree.display()
            )));
        }
    }

    // Git.
    if make_line {
        crate::coderepo::create_branch_from(repo, &line, &model.main)?;
    }
    let undo_line = |merged_from: Option<&str>| {
        if let Some(old) = merged_from {
            let _ = crate::coderepo::reset_branch_to(repo, &line, old);
        }
        if make_line {
            let _ = crate::coderepo::delete_branch(repo, &line);
        }
    };
    let (old, sha) = if already {
        (None, crate::coderepo::rev_parse(repo, &line)?)
    } else {
        let message = format!(
            "Merge {branch} into {line}\n\n{code}: {}\n\nRefs: kanbanr:{code}",
            feature.title
        );
        match crate::coderepo::merge_no_ff(repo, &line, &branch, &message) {
            Ok((old, new)) => (Some(old), new),
            Err(e) => {
                undo_line(None);
                return Err(e);
            }
        }
    };

    // Board.
    let (moved, warnings) = match store.move_feature_gated(id, code, to, reason) {
        Ok(done) => done,
        Err(e) => {
            undo_line(old.as_deref());
            return Err(e);
        }
    };
    let forward_needed = line.strip_prefix(RELEASE_PREFIX).map(str::to_string);
    Ok(Moved {
        feature: moved,
        warnings,
        merged: Some(Merged {
            branch,
            into: line,
            sha,
            already,
        }),
        forward_needed,
        note: None,
    })
}

/// Would this move merge a branch? A batch refuses those (they must run alone).
pub fn move_would_merge(config: &ProjectConfig, from: &str, to: &str) -> bool {
    config.cadence.git_model
        && !config.is_no_op(to)
        && !is_integrated(config, from)
        && is_integrated(config, to)
}

/// What bringing a carried item's branch up to date did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "detail", rename_all = "snake_case")]
pub enum CatchUpOutcome {
    Merged(String),
    UpToDate,
    Conflicts(Vec<String>),
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatchUp {
    pub code: String,
    pub branch: String,
    pub outcome: CatchUpOutcome,
}

/// Bring the branches of the items carried into `sprint` up to date with `sprint/<SP>` (R-3): a
/// merge into each item branch, never a rebase, never a deletion. Each item is on its own; a
/// conflict is reported and leaves its branch as it was.
pub fn catch_up(store: &Store, id: &str, sprint: &str, repo: &Path) -> Result<Vec<CatchUp>> {
    let project = store.load_meta(id)?;
    let config = &project.config;
    let sprints = crate::sprints::load(store, id)?;
    let line = sprint_branch(sprint);
    let main = config
        .cadence
        .main_branch
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| crate::coderepo::default_branch(repo));
    let facts = crate::coderepo::branch_facts(repo, &main)?;
    if !facts.branches.contains_key(&line) {
        return Ok(Vec::new());
    }
    let mut codes: Vec<String> = sprints
        .iter()
        .filter(|s| s.state == SprintState::Closed)
        .flat_map(|s| s.carried.iter())
        .filter(|c| c.to == sprint)
        .map(|c| c.code.clone())
        .collect();
    codes.sort();
    codes.dedup();
    let mut out = Vec::new();
    for code in codes {
        let Ok(feature) = project.feature(&code) else {
            continue;
        };
        if is_integrated(config, &feature.status) {
            continue;
        }
        let branches = item_branches(config, &facts, &code);
        let (branch, outcome) = match branches.as_slice() {
            [] => continue,
            [one] => (one.clone(), catch_up_one(repo, &code, one, &line)),
            many => (
                many.join(", "),
                CatchUpOutcome::Skipped("more than one branch".into()),
            ),
        };
        out.push(CatchUp {
            code,
            branch,
            outcome,
        });
    }
    Ok(out)
}

fn catch_up_one(repo: &Path, code: &str, branch: &str, line: &str) -> CatchUpOutcome {
    if crate::coderepo::is_ancestor(repo, line, branch) {
        return CatchUpOutcome::UpToDate;
    }
    if let Some(worktree) = crate::coderepo::worktree_of(repo, branch)
        && !crate::coderepo::is_clean(&worktree)
    {
        return CatchUpOutcome::Skipped("uncommitted changes".into());
    }
    match crate::coderepo::merge_preview(repo, branch, line) {
        Ok(p) if !p.clean => return CatchUpOutcome::Conflicts(p.conflicts),
        Err(e) => return CatchUpOutcome::Skipped(e.to_string()),
        Ok(_) => {}
    }
    let message = format!("Merge {line} into {branch}\n\nRefs: kanbanr:{code}");
    match crate::coderepo::merge_no_ff(repo, branch, line, &message) {
        Ok((_, new)) => CatchUpOutcome::Merged(new),
        Err(e) => CatchUpOutcome::Skipped(e.to_string()),
    }
}

/// What closing a sprint did.
#[derive(Debug, Clone, Serialize)]
pub struct SprintClosed {
    pub sprint: Sprint,
    /// The tag made at the sprint's tip when its close moved production (sprints topology).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagged: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub catch_up: Vec<CatchUp>,
}

/// Close a sprint. In the sprints topology with the model on, the close is production: main
/// fast-forwards to `sprint/<SP>`, tagged with the sprint's code (Q4) — refused before anything
/// changes if main has commits the sprint lacks. Then carried items catch up with the sprint they
/// went to, when its branch exists.
pub fn sprint_close(
    store: &Store,
    id: &str,
    code: &str,
    carry_to: Option<&str>,
    repo: Option<&Path>,
) -> Result<SprintClosed> {
    let config = store.load_meta(id)?.config;
    let automated = config.cadence.git_model;
    let line = sprint_branch(code);
    let mut tagged = None;
    let sprint = match repo {
        Some(repo)
            if automated
                && topology(&config.cadence) == Topology::Sprints
                && crate::coderepo::branch_exists(repo, &line) =>
        {
            let sprints = crate::sprints::load(store, id)?;
            match sprints.iter().find(|s| s.code == code) {
                None => return Err(CoreError::Unsupported(format!("no sprint {code}"))),
                Some(s) if s.state == SprintState::Closed => {
                    return Err(CoreError::Unsupported(format!("{code} is already closed")));
                }
                Some(_) => {}
            }
            let main = config
                .cadence
                .main_branch
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| crate::coderepo::default_branch(repo));
            if !crate::coderepo::is_ancestor(repo, &main, &line) {
                return Err(CoreError::Unsupported(format!(
                    "main has commits {line} lacks, so closing {code} cannot fast-forward it: merge \
                     {main} into {line}, then close again"
                )));
            }
            if crate::coderepo::tag_exists(repo, code) {
                return Err(CoreError::Unsupported(format!(
                    "the tag {code} already exists — nothing was changed"
                )));
            }
            if let Some(worktree) = crate::coderepo::worktree_of(repo, &main)
                && !crate::coderepo::is_clean(&worktree)
            {
                return Err(CoreError::Unsupported(format!(
                    "{main} is checked out in {} with uncommitted changes: commit or stash them \
                     first",
                    worktree.display()
                )));
            }
            crate::coderepo::create_tag(repo, code, &line, &format!("sprint {code}"))?;
            let old = match crate::coderepo::fast_forward(repo, &main, &line) {
                Ok(old) => old,
                Err(e) => {
                    let _ = crate::coderepo::delete_tag(repo, code);
                    return Err(e);
                }
            };
            match crate::sprints::close(store, id, code, carry_to) {
                Ok(sprint) => {
                    tagged = Some(code.to_string());
                    sprint
                }
                Err(e) => {
                    let _ = crate::coderepo::reset_branch_to(repo, &main, &old);
                    let _ = crate::coderepo::delete_tag(repo, code);
                    return Err(e);
                }
            }
        }
        _ => crate::sprints::close(store, id, code, carry_to)?,
    };
    let catch_up = match (repo, carry_to.map(str::trim)) {
        (Some(repo), Some(next)) if automated && !next.is_empty() && next != "backlog" => {
            catch_up(store, id, next, repo).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Ok(SprintClosed {
        sprint,
        tagged,
        catch_up,
    })
}

/// What a forward merge did.
#[derive(Debug, Clone, Serialize)]
pub struct Forwarded {
    pub from: String,
    pub into: String,
    /// The merge commit; None when the line already had everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    pub refs: Vec<String>,
}

/// Merge a release's fixes forward into the current line (R-4), naming in `Refs:` every item the
/// release's own commits name. Asked for, never automatic (Q1).
pub fn forward(store: &Store, id: &str, version: &str, repo: &Path) -> Result<Forwarded> {
    let releases = crate::releases::load(store, id)?;
    let release = releases
        .iter()
        .find(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if release.branch.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "{version} has no branch yet: it is made when the release reaches its first stage"
        )));
    }
    let from = release.branch.clone();
    let (model, _) = load(store, id, repo)?;
    let into = model.line.clone().ok_or_else(|| {
        CoreError::Unsupported("no sprint is running, so there is no line to forward into".into())
    })?;
    let commits = crate::coderepo::commits_between(repo, &into, &from)?;
    if commits.is_empty() {
        return Ok(Forwarded {
            from,
            into,
            sha: None,
            refs: Vec::new(),
        });
    }
    let preview = crate::coderepo::merge_preview(repo, &into, &from)?;
    if !preview.clean {
        return Err(CoreError::Unsupported(format!(
            "merging {from} forward into {into} conflicts in: {}. Merge it by hand, with a Refs: \
             trailer.",
            preview.conflicts.join(", ")
        )));
    }
    let mut refs: Vec<String> = commits
        .iter()
        .flat_map(|c| crate::scm::parse_refs(&c.message))
        .map(|r| format!("kanbanr:{}", r.code()))
        .collect();
    refs.sort();
    refs.dedup();
    let trailer = if refs.is_empty() {
        format!("[no-ref] forward merge of {from}")
    } else {
        format!("Refs: {}", refs.join(", "))
    };
    let message = format!("Merge {from} forward into {into}\n\n{trailer}");
    let (_, sha) = crate::coderepo::merge_no_ff(repo, &into, &from, &message)?;
    Ok(Forwarded {
        from,
        into,
        sha: Some(sha),
        refs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(sprints: bool, releases: bool) -> ProjectConfig {
        let mut c = ProjectConfig::default_for("p");
        c.cadence.sprints = sprints;
        c.cadence.releases = releases;
        c.cadence.git_model = true;
        c
    }

    fn sprint(code: &str, state: SprintState) -> Sprint {
        Sprint {
            code: code.into(),
            state,
            ..Default::default()
        }
    }

    fn facts(branches: &[&str], merged: &[&str]) -> BranchFacts {
        BranchFacts {
            main: "main".into(),
            branches: branches
                .iter()
                .map(|b| (b.to_string(), "0".repeat(40)))
                .collect(),
            merged_into_main: merged.iter().map(|b| b.to_string()).collect(),
        }
    }

    /// FEAT-189 R-1: the four cadences give their topology, line, production and protected
    /// branches; a sprint builds on the sprint that has not shipped; a release branches from the
    /// line its topology says.
    #[test]
    fn git_model_follows_cadence() {
        let f = facts(&["main"], &["main"]);
        let running = [sprint("SP-001", SprintState::Active)];

        let trunk = derive(&config(false, false), &[], &f);
        assert_eq!(trunk.topology, Topology::Trunk);
        assert_eq!(trunk.line.as_deref(), Some("main"));
        assert_eq!(trunk.production, Production::IntegrateMerge);

        let sprints = derive(&config(true, false), &running, &f);
        assert_eq!(sprints.topology, Topology::Sprints);
        assert_eq!(sprints.line.as_deref(), Some("sprint/SP-001"));
        assert_eq!(sprints.production, Production::SprintClose);
        assert_eq!(
            derive(&config(true, false), &[], &f).line,
            None,
            "no sprint running"
        );

        let releases = derive(&config(false, true), &[], &f);
        assert_eq!(releases.topology, Topology::Releases);
        assert_eq!(releases.line.as_deref(), Some("integration"));
        assert_eq!(releases.production, Production::ReleaseCut);
        assert!(is_protected(&releases, "integration"));
        assert!(is_protected(&releases, "release/1.0.0"));
        assert!(is_protected(&releases, "main"));
        assert!(!is_protected(&releases, "feat/FEAT-001-x"));

        let both = derive(&config(true, true), &running, &f);
        assert_eq!(both.topology, Topology::SprintsAndReleases);
        assert_eq!(both.line.as_deref(), Some("sprint/SP-001"));
        assert_eq!(both.production, Production::ReleaseCut);
        assert!(is_protected(&both, "sprint/SP-002"));

        // Off, the model describes but protects only main.
        let mut off = config(true, true);
        off.cadence.git_model = false;
        let described = derive(&off, &running, &f);
        assert!(!described.automated);
        assert!(!is_protected(&described, "sprint/SP-001"));
        assert!(is_protected(&described, "main"));

        // sprint_base: the unshipped sprint; once main has it, main.
        let history = [
            sprint("SP-001", SprintState::Closed),
            sprint("SP-002", SprintState::Planned),
        ];
        assert_eq!(
            sprint_base(&history, &facts(&["main", "sprint/SP-001"], &["main"])),
            "sprint/SP-001"
        );
        assert_eq!(
            sprint_base(
                &history,
                &facts(&["main", "sprint/SP-001"], &["main", "sprint/SP-001"])
            ),
            "main"
        );
        assert_eq!(sprint_base(&[], &f), "main");

        // release_base per topology.
        assert_eq!(release_base(&trunk, &[], &f), "main");
        assert_eq!(release_base(&releases, &[], &f), "integration");
        let closed = [
            sprint("SP-001", SprintState::Closed),
            sprint("SP-002", SprintState::Active),
        ];
        let model = derive(&config(true, true), &closed, &f);
        assert_eq!(
            release_base(
                &model,
                &closed,
                &facts(&["main", "sprint/SP-001", "sprint/SP-002"], &["main"])
            ),
            "sprint/SP-001",
            "the closed sprint that has not shipped"
        );
        assert_eq!(
            release_base(
                &model,
                &closed,
                &facts(
                    &["main", "sprint/SP-001", "sprint/SP-002"],
                    &["main", "sprint/SP-001"]
                )
            ),
            "sprint/SP-002",
            "else the active sprint's"
        );
    }

    #[test]
    fn line_commit_verdicts() {
        let model = derive(
            &config(true, false),
            &[sprint("SP-001", SprintState::Active)],
            &facts(&["main"], &[]),
        );
        assert_eq!(
            line_commit_verdict(&model, "feat/FEAT-001-x", false, None),
            LineVerdict::NotALine
        );
        assert!(matches!(
            line_commit_verdict(&model, "sprint/SP-001", false, None),
            LineVerdict::Refused(_)
        ));
        assert_eq!(
            line_commit_verdict(&model, "sprint/SP-001", true, None),
            LineVerdict::Allowed,
            "pre-commit does not know the message yet"
        );
        assert_eq!(
            line_commit_verdict(
                &model,
                "main",
                true,
                Some("Merge FEAT-001\n\nRefs: kanbanr:FEAT-001\n")
            ),
            LineVerdict::Allowed
        );
        assert_eq!(
            line_commit_verdict(
                &model,
                "main",
                true,
                Some("x\n\n[no-ref] hotfix, item filed after")
            ),
            LineVerdict::Allowed
        );
        assert!(matches!(
            line_commit_verdict(&model, "main", true, Some("Merge branch 'feat/FEAT-001-x'")),
            LineVerdict::Refused(_)
        ));
    }

    /// FEAT-189 R-5: kanbanr never pushes. The code repository wrapper refuses, and nothing here
    /// or there passes `push` to git.
    #[test]
    fn git_model_never_pushes() {
        let dir = std::env::temp_dir().join(format!("kanbanr-nopush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = crate::coderepo::git(&dir, &["push", "origin", "main"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("never pushes"), "{err}");
        for (name, source) in [
            ("coderepo.rs", include_str!("coderepo.rs")),
            ("gitmodel.rs", include_str!("gitmodel.rs")),
        ] {
            // The code, not its tests: the test above passes it on purpose, to see it refused.
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            let call = concat!("[\"pu", "sh\"");
            assert!(!code.contains(call), "{name} passes push to git");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
