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
        return Ok(SprintStarted {
            sprint: crate::sprints::start(store, id, code)?,
            branch: None,
            reused: Some(branch),
        });
    }
    let base = sprint_base(&sprints, &facts);
    crate::coderepo::create_branch_from(repo, &branch, &base)?;
    match crate::sprints::start(store, id, code) {
        Ok(sprint) => Ok(SprintStarted {
            sprint,
            branch: Some((branch, base)),
            reused: None,
        }),
        Err(e) => {
            let _ = crate::coderepo::delete_branch(repo, &branch);
            Err(e)
        }
    }
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
