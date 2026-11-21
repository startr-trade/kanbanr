//! Drift between the board and the code repository's branches (FEAT-191).
//!
//! `kanbanr git doctor` compares what the board says with what the branches hold, in the git
//! model's terms (FEAT-189): an integrated item whose branch never reached its line, commits on a
//! line for work that is not integrated, branches that name nothing on the board, and release
//! fixes not merged forward.
//!
//! Two halves: [`collect`] reads facts with read-only git calls; [`check`] decides over them and is
//! tested without a repository. Nothing here writes to the repository or the board.

use crate::coderepo::Commit;
use crate::config::ProjectConfig;
use crate::doctor::{Issue, Severity};
use crate::gitmodel::{self, BranchFacts, GitModel, Topology};
use crate::releases::Release;
use crate::scm::{RELEASE_PREFIX, SPIKE_PREFIX, SPRINT_PREFIX};
use crate::sprints::Sprint;
use crate::store::Project;
use crate::{Result, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Commits read per line; also the window on main in the trunk topology (Q1).
pub const LIMIT: usize = 200;

/// What the repository holds, as `check` needs it.
#[derive(Debug, Clone, Default)]
pub struct DriftFacts {
    pub branches: BranchFacts,
    /// For each line checked: the local branches merged into it.
    pub merged_into: BTreeMap<String, BTreeSet<String>>,
    /// For each line checked: its commits not on main (main itself: its last `LIMIT` commits).
    pub line_commits: BTreeMap<String, Vec<Commit>>,
    /// For each release branch: its commits the current line lacks.
    pub unforwarded: BTreeMap<String, Vec<Commit>>,
}

/// The lines checked: the current line, every `sprint/*` main does not hold yet, and every release
/// branch of a release on the board; main too in the trunk topology.
pub fn lines(model: &GitModel, facts: &BranchFacts, releases: &[Release]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |b: &str| {
        if facts.branches.contains_key(b) && !out.iter().any(|x| x == b) {
            out.push(b.to_string());
        }
    };
    if let Some(line) = &model.line {
        add(line);
    }
    if model.topology == Topology::Trunk {
        add(&model.main);
    }
    for b in facts.branches.keys() {
        if b.starts_with(SPRINT_PREFIX)
            && !facts.merged_into_main.contains(b)
            && !facts.work_in_main.contains(b)
        {
            add(b);
        }
    }
    for r in releases.iter().filter(|r| !r.branch.is_empty()) {
        add(&r.branch);
    }
    out
}

/// Read the facts, with read-only git calls only: `for-each-ref`, `branch --merged`, `log`.
pub fn collect(root: &Path, model: &GitModel, releases: &[Release]) -> Result<DriftFacts> {
    let branches = crate::coderepo::branch_facts(root, &model.main)?;
    let mut facts = DriftFacts {
        branches,
        ..Default::default()
    };
    for line in lines(model, &facts.branches, releases) {
        let merged = crate::coderepo::branches_merged_into(root, &line)?;
        let range = if line == model.main {
            line.clone()
        } else {
            format!("{}..{line}", model.main)
        };
        let commits = crate::coderepo::commits_limited(root, &range, LIMIT)?;
        facts.merged_into.insert(line.clone(), merged);
        facts.line_commits.insert(line, commits);
    }
    if let Some(line) = model
        .line
        .as_deref()
        .filter(|l| facts.branches.branches.contains_key(*l))
    {
        for r in releases.iter().filter(|r| !r.branch.is_empty()) {
            if r.branch == line || !facts.branches.branches.contains_key(&r.branch) {
                continue;
            }
            let commits =
                crate::coderepo::commits_limited(root, &format!("{line}..{}", r.branch), LIMIT)?;
            facts.unforwarded.insert(r.branch.clone(), commits);
        }
    }
    Ok(facts)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn issue(project: &str, severity: Severity, code: Option<&str>, message: String) -> Issue {
    Issue {
        severity,
        project: project.to_string(),
        code: code.map(str::to_string),
        message,
    }
}

/// The findings, over facts already read.
pub fn check(
    project: &Project,
    sprints: &[Sprint],
    releases: &[Release],
    model: &GitModel,
    facts: &DriftFacts,
) -> Vec<Issue> {
    let config: &ProjectConfig = &project.config;
    let id = project.id.as_str();
    let mut out = Vec::new();

    // Branches: each accounted for, and each item branch where its item says it should be.
    for branch in facts.branches.branches.keys() {
        if *branch == model.main
            || model.integration_branch.as_deref() == Some(branch.as_str())
            || branch.starts_with(SPIKE_PREFIX)
        {
            continue;
        }
        if let Some(sp) = branch.strip_prefix(SPRINT_PREFIX) {
            if !sprints.iter().any(|s| s.code == sp) {
                out.push(issue(
                    id,
                    Severity::Warning,
                    None,
                    format!("{branch} names no sprint"),
                ));
            }
            continue;
        }
        if let Some(v) = branch.strip_prefix(RELEASE_PREFIX) {
            if !releases.iter().any(|r| r.version == v) {
                out.push(issue(
                    id,
                    Severity::Warning,
                    None,
                    format!("{branch} names no release"),
                ));
            }
            continue;
        }
        let Some(code) = crate::scm::item_code_from_branch(config.branch_pattern(), branch) else {
            out.push(issue(
                id,
                Severity::Warning,
                None,
                format!("{branch} names no item"),
            ));
            continue;
        };
        let Some(item) = project.features.iter().find(|f| f.code == code) else {
            out.push(issue(
                id,
                Severity::Warning,
                None,
                format!("{branch} names {code}, which is not on the board"),
            ));
            continue;
        };
        let status = item.status.as_str();
        let line = gitmodel::line_for(model, item, releases);
        let in_line = line
            .as_ref()
            .and_then(|l| facts.merged_into.get(l))
            .is_some_and(|m| m.contains(branch));
        let merged = in_line || facts.branches.merged_into_main.contains(branch);
        if gitmodel::is_integrated(config, status) && !merged {
            let target = line.unwrap_or_else(|| model.main.clone());
            out.push(issue(
                id,
                Severity::Error,
                Some(&code),
                format!(
                    "{code} is {status} (integrated) but {branch} is not merged into {target}: move \
                     it back, or merge it (FEAT-190 does this on the move)"
                ),
            ));
        } else if gitmodel::is_finished(config, status) && merged {
            out.push(issue(
                id,
                Severity::Warning,
                Some(&code),
                format!(
                    "{code} is finished and {branch} is merged: `kanbanr git tidy` lists the merged \
                     branches, and `--yes` deletes them locally after one confirmation (never a \
                     remote)"
                ),
            ));
        } else if gitmodel::is_parked(config, status) {
            out.push(issue(
                id,
                Severity::Warning,
                Some(&code),
                format!("{code} is {status} (parked) but {branch} remains"),
            ));
        }
    }

    // Commits on lines: each serves something that has been integrated.
    for (line, commits) in &facts.line_commits {
        let mut unfinished: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for c in commits {
            let codes: BTreeSet<String> = crate::scm::parse_refs(&c.message)
                .iter()
                .map(|r| r.code().to_string())
                .collect();
            if codes.is_empty() {
                continue; // a `[no-ref]` escape or a generated commit is on the record already
            }
            let mut known = Vec::new();
            for code in &codes {
                match project.features.iter().find(|f| &f.code == code) {
                    Some(f) => known.push(f),
                    None => out.push(issue(
                        id,
                        Severity::Error,
                        Some(code),
                        format!(
                            "commit {} on {line} names {code}, which is not on the board",
                            short(&c.sha)
                        ),
                    )),
                }
            }
            if known.is_empty()
                || known
                    .iter()
                    .any(|f| gitmodel::may_be_integrated(config, &f.status))
            {
                continue;
            }
            for f in known {
                let entry = unfinished
                    .entry(f.code.clone())
                    .or_insert((0, c.sha.clone()));
                entry.0 += 1;
            }
        }
        for (code, (n, newest)) in unfinished {
            let status = project
                .features
                .iter()
                .find(|f| f.code == code)
                .map(|f| f.status.clone())
                .unwrap_or_default();
            out.push(issue(
                id,
                Severity::Error,
                Some(&code),
                format!(
                    "{line} has {n} commit{} serving {code} (newest {}), which is {status}, not \
                     integrated",
                    if n == 1 { "" } else { "s" },
                    short(&newest)
                ),
            ));
        }
    }

    // Release fixes the line lacks.
    for (branch, commits) in &facts.unforwarded {
        let Some(newest) = commits.first() else {
            continue;
        };
        let line = model.line.clone().unwrap_or_default();
        let version = branch.strip_prefix(RELEASE_PREFIX).unwrap_or(branch);
        out.push(issue(
            id,
            Severity::Warning,
            None,
            format!(
                "{branch} has {} commit{} {line} lacks (newest {}): {}",
                commits.len(),
                if commits.len() == 1 { "" } else { "s" },
                short(&newest.sha),
                gitmodel::forward_step(version, &line)
            ),
        ));
    }

    out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
    out
}

/// Load the board and the model, read the repository, and check. Read-only.
pub fn scan(store: &Store, id: &str, root: &Path) -> Result<Vec<Issue>> {
    let project = store.load_meta(id)?;
    let sprints = crate::sprints::load(store, id)?;
    let releases = crate::releases::load(store, id).unwrap_or_default();
    let (model, _) = gitmodel::load(store, id, root)?;
    let facts = collect(root, &model, &releases)?;
    let mut issues = check(&project, &sprints, &releases, &model, &facts);
    // Agent work the board has not caught up with, and scopes whose worktree is gone (FEAT-214).
    issues.extend(crate::agentscope::drift_issues(
        store,
        id,
        root,
        &crate::now_rfc3339(),
    ));
    Ok(issues)
}

/// A local branch `kanbanr git tidy` may delete (DEF-051).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TidyCandidate {
    pub branch: String,
    /// `"item"` or `"agent"`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// The line the branch is merged into.
    pub into: String,
    pub tip: String,
    pub why: String,
    /// A live scope still in force for this finished item: name it so it can be revoked (DEF-096).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
}

/// A live scope's claim on a branch, for tidy (DEF-096).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeHold {
    pub scope: String,
    pub item: String,
}

/// The merged branches that can go: a finished item's merged branch, and a merged
/// `worktree-agent-*` branch no live scope holds. Never main, the integration branch, `sprint/*`,
/// `release/*`, `spike/*`, a branch a worktree has checked out, an unmerged branch, or the branch of
/// a parked or unfinished item. `live_scopes` maps each branch a live scope holds to that scope: a
/// held branch is skipped while the scope's item is unfinished or not on the board; when the item
/// is finished the hold is ignored and a listed candidate carries `held_by` (DEF-096). Sorted by
/// branch name; pure over facts already read.
pub fn tidy_candidates(
    project: &Project,
    releases: &[Release],
    model: &GitModel,
    facts: &DriftFacts,
    checked_out: &dyn Fn(&str) -> bool,
    live_scopes: &BTreeMap<String, ScopeHold>,
) -> Vec<TidyCandidate> {
    let config: &ProjectConfig = &project.config;
    let mut out = Vec::new();
    for (branch, tip) in &facts.branches.branches {
        if *branch == model.main
            || model.integration_branch.as_deref() == Some(branch.as_str())
            || branch.starts_with(SPRINT_PREFIX)
            || branch.starts_with(RELEASE_PREFIX)
            || branch.starts_with(SPIKE_PREFIX)
            || checked_out(branch)
        {
            continue;
        }
        let hold = live_scopes.get(branch);
        let hold_ignored = hold.is_some_and(|h| {
            project
                .features
                .iter()
                .any(|f| f.code == h.item && gitmodel::is_finished(config, &f.status))
        });
        if hold.is_some() && !hold_ignored {
            continue;
        }
        let held_by = hold.filter(|_| hold_ignored).map(|h| h.scope.clone());
        let merged_main = facts.branches.merged_into_main.contains(branch);
        if branch.starts_with(gitmodel::AGENT_BRANCH_PREFIX) {
            if merged_main {
                out.push(TidyCandidate {
                    branch: branch.clone(),
                    kind: "agent".into(),
                    item: None,
                    into: model.main.clone(),
                    tip: tip.clone(),
                    why: format!("a finished agent's branch, merged into {}", model.main),
                    held_by,
                });
            }
            continue;
        }
        let Some(code) = crate::scm::item_code_from_branch(config.branch_pattern(), branch) else {
            continue;
        };
        let Some(item) = project.features.iter().find(|f| f.code == code) else {
            continue;
        };
        if !gitmodel::is_finished(config, &item.status) {
            continue;
        }
        let line = gitmodel::line_for(model, item, releases);
        let into = match line.as_ref().filter(|l| {
            facts
                .merged_into
                .get(*l)
                .is_some_and(|m| m.contains(branch))
        }) {
            Some(l) => l.clone(),
            None if merged_main => model.main.clone(),
            None => continue,
        };
        out.push(TidyCandidate {
            branch: branch.clone(),
            kind: "item".into(),
            item: Some(code.clone()),
            why: format!(
                "{code} is {} and the branch is merged into {into}",
                item.status
            ),
            into,
            tip: tip.clone(),
            held_by,
        });
    }
    out.sort_by(|a, b| a.branch.cmp(&b.branch));
    out
}

/// Read the repository and list what `git tidy` would delete (DEF-051). Read-only.
pub fn tidy_scan(store: &Store, id: &str, root: &Path) -> Result<(Vec<TidyCandidate>, GitModel)> {
    let project = store.load_meta(id)?;
    let releases = crate::releases::load(store, id).unwrap_or_default();
    let (model, _) = gitmodel::load(store, id, root)?;
    let facts = collect(root, &model, &releases)?;
    let now = crate::now_rfc3339();
    let mut live = BTreeMap::new();
    for s in crate::agentscope::load(store, id).unwrap_or_default() {
        if matches!(
            s.state(&now),
            crate::agentscope::ScopeState::Revoked | crate::agentscope::ScopeState::Expired
        ) {
            continue;
        }
        let hold = ScopeHold {
            scope: s.id.clone(),
            item: s.item.clone(),
        };
        for b in [&s.agent_branch, &s.branch].into_iter().flatten() {
            live.insert(b.clone(), hold.clone());
        }
    }
    let current = crate::coderepo::current_branch(root);
    let checked_out =
        |b: &str| current.as_deref() == Some(b) || crate::coderepo::worktree_of(root, b).is_some();
    let list = tidy_candidates(&project, &releases, &model, &facts, &checked_out, &live);
    Ok((list, model))
}

/// What a confirmed tidy did.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct TidyOutcome {
    pub deleted: Vec<TidyDeleted>,
    pub skipped: Vec<TidySkipped>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TidyDeleted {
    pub branch: String,
    pub tip: String,
    pub into: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TidySkipped {
    pub branch: String,
    pub why: String,
}

/// Re-read, re-compute and delete each candidate through the checked helper; a branch that stopped
/// qualifying is skipped, never forced (DEF-051). Local only.
pub fn tidy_run(store: &Store, id: &str, root: &Path) -> Result<TidyOutcome> {
    let (list, _) = tidy_scan(store, id, root)?;
    let mut out = TidyOutcome::default();
    for c in list {
        match crate::coderepo::delete_merged_branch(root, &c.branch, &c.into) {
            Ok(()) => out.deleted.push(TidyDeleted {
                branch: c.branch,
                tip: c.tip,
                into: c.into,
            }),
            Err(e) => out.skipped.push(TidySkipped {
                branch: c.branch,
                why: e.to_string(),
            }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;
    use crate::sprints::SprintState;

    fn project(items: &[(&str, &str)], sprints: bool) -> Project {
        let mut config = ProjectConfig::for_new_project("p");
        config.cadence.sprints = sprints;
        config.cadence.git_model = true;
        let features = items
            .iter()
            .map(|(code, status)| {
                crate::yaml::from_str(&format!(
                    "code: {code}\ntitle: t\nspecification: ''\nstatus: {status}\nmilestone: M\n\
                     created_at: '2026-10-19T00:00:00Z'\nupdated_at: '2026-10-19T00:00:00Z'\n"
                ))
                .unwrap()
            })
            .collect();
        Project {
            id: "p".into(),
            config,
            features,
            milestones: Vec::new(),
        }
    }

    fn facts(branches: &[&str], merged_main: &[&str]) -> BranchFacts {
        BranchFacts {
            main: "main".into(),
            branches: branches
                .iter()
                .map(|b| (b.to_string(), "1".repeat(40)))
                .collect(),
            merged_into_main: merged_main.iter().map(|b| b.to_string()).collect(),
            work_in_main: Default::default(),
        }
    }

    fn commit(sha: char, message: &str) -> Commit {
        Commit {
            sha: sha.to_string().repeat(40),
            subject: message.lines().next().unwrap_or_default().to_string(),
            message: message.to_string(),
        }
    }

    fn messages(issues: &[Issue]) -> Vec<String> {
        issues.iter().map(|i| i.message.clone()).collect()
    }

    /// DEF-051 R-1: each exclusion in "Never listed", and the two kinds that are.
    #[test]
    fn tidy_lists_finished_merged_item_branches_and_merged_agent_branches_only() {
        let p = project(
            &[
                ("FEAT-001", "Completed"),
                ("FEAT-002", "Completed"),
                ("FEAT-003", "In Progress"),
                ("FEAT-004", "Deferred"),
                ("FEAT-005", "Completed"),
            ],
            false,
        );
        let all = [
            "main",
            "sprint/SP-001",
            "release/v1.0.0",
            "spike/x",
            "feat/FEAT-001-merged",
            "feat/FEAT-002-unmerged",
            "feat/FEAT-003-working",
            "feat/FEAT-004-parked",
            "feat/FEAT-005-occupied",
            "feat/FEAT-099-nothing",
            "worktree-agent-merged",
            "worktree-agent-unmerged",
            "worktree-agent-live",
            "worktree-agent-occupied",
            "other",
        ];
        let merged = [
            "main",
            "sprint/SP-001",
            "release/v1.0.0",
            "spike/x",
            "feat/FEAT-001-merged",
            "feat/FEAT-003-working",
            "feat/FEAT-004-parked",
            "feat/FEAT-005-occupied",
            "worktree-agent-merged",
            "worktree-agent-live",
            "worktree-agent-occupied",
            "other",
        ];
        let f = facts(&all, &merged);
        let model = gitmodel::derive(&p.config, &[], &f);
        let d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        // Held for an unfinished item, so still excluded (DEF-096).
        let live: BTreeMap<String, ScopeHold> = [(
            "worktree-agent-live".to_string(),
            ScopeHold {
                scope: "SC-9".into(),
                item: "FEAT-003".into(),
            },
        )]
        .into();
        let occupied = |b: &str| b == "feat/FEAT-005-occupied" || b == "worktree-agent-occupied";
        let got = tidy_candidates(&p, &[], &model, &d, &occupied, &live);
        let names: Vec<_> = got.iter().map(|c| c.branch.as_str()).collect();
        assert_eq!(
            names,
            vec!["feat/FEAT-001-merged", "worktree-agent-merged"],
            "{got:?}"
        );
        assert_eq!(got[0].kind, "item");
        assert_eq!(got[0].item.as_deref(), Some("FEAT-001"));
        assert_eq!(got[0].into, "main");
        assert_eq!(got[1].kind, "agent");
        assert_eq!(got[1].tip, "1".repeat(40));
    }

    fn hold(scope: &str, item: &str) -> ScopeHold {
        ScopeHold {
            scope: scope.into(),
            item: item.into(),
        }
    }

    /// DEF-096 R-1: a finished item's merged branch, and a merged agent branch, held only by a
    /// scope issued for that finished item, are listed and name the scope.
    #[test]
    fn a_finished_items_branch_held_only_by_its_own_scope_is_listed() {
        let p = project(&[("FEAT-001", "Completed")], false);
        let all = ["main", "feat/FEAT-001-x", "worktree-agent-a"];
        let f = facts(&all, &all);
        let model = gitmodel::derive(&p.config, &[], &f);
        let d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        let live: BTreeMap<String, ScopeHold> = [
            ("feat/FEAT-001-x".to_string(), hold("SC-1", "FEAT-001")),
            ("worktree-agent-a".to_string(), hold("SC-2", "FEAT-001")),
        ]
        .into();
        let got = tidy_candidates(&p, &[], &model, &d, &|_| false, &live);
        let names: Vec<_> = got.iter().map(|c| c.branch.as_str()).collect();
        assert_eq!(
            names,
            vec!["feat/FEAT-001-x", "worktree-agent-a"],
            "{got:?}"
        );
        assert_eq!(got[0].held_by.as_deref(), Some("SC-1"));
        assert_eq!(got[0].into, "main");
        assert_eq!(got[1].kind, "agent");
        assert_eq!(got[1].held_by.as_deref(), Some("SC-2"));
    }

    /// DEF-096 R-2: a live scope on unfinished work, or on an item not on the board, still holds
    /// its branch; a checked-out branch is kept whatever its item's status.
    #[test]
    fn a_live_scope_on_unfinished_work_still_holds_its_branch() {
        let p = project(
            &[("FEAT-001", "Completed"), ("FEAT-003", "In Progress")],
            false,
        );
        let all = [
            "main",
            "feat/FEAT-001-x",
            "feat/FEAT-003-working",
            "worktree-agent-ghost",
        ];
        let f = facts(&all, &all);
        let model = gitmodel::derive(&p.config, &[], &f);
        let d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        let live: BTreeMap<String, ScopeHold> = [
            (
                "feat/FEAT-003-working".to_string(),
                hold("SC-3", "FEAT-003"),
            ),
            ("worktree-agent-ghost".to_string(), hold("SC-4", "FEAT-404")),
        ]
        .into();
        let got = tidy_candidates(&p, &[], &model, &d, &|_| false, &live);
        let names: Vec<_> = got.iter().map(|c| c.branch.as_str()).collect();
        assert_eq!(names, vec!["feat/FEAT-001-x"], "{got:?}");
        assert_eq!(got[0].held_by, None);
        let occupied = |b: &str| b == "feat/FEAT-001-x";
        let got = tidy_candidates(&p, &[], &model, &d, &occupied, &live);
        assert!(got.is_empty(), "{got:?}");
    }

    /// DEF-096 R-3: an item branch merged into a release branch that main then took is listed;
    /// a finished item's branch in no merged set is not.
    #[test]
    fn an_item_branch_merged_through_a_release_is_listed() {
        let mut p = project(
            &[("FEAT-001", "Completed"), ("FEAT-002", "Completed")],
            true,
        );
        p.features[0].release = Some("v1".into());
        let releases = [Release {
            version: "v1".into(),
            branch: "release/v1".into(),
            state: crate::releases::ReleaseState::Shipped,
            ..Default::default()
        }];
        let sprints = [Sprint {
            code: "SP-002".into(),
            state: SprintState::Active,
            ..Default::default()
        }];
        let all = [
            "main",
            "sprint/SP-002",
            "release/v1",
            "feat/FEAT-001-x",
            "feat/FEAT-002-y",
        ];
        let f = facts(
            &all,
            &["main", "sprint/SP-002", "release/v1", "feat/FEAT-001-x"],
        );
        let model = gitmodel::derive(&p.config, &sprints, &f);
        let line = model.line.clone().expect("a sprint line");
        let mut d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        d.merged_into
            .insert("release/v1".into(), ["feat/FEAT-001-x".to_string()].into());
        d.merged_into
            .insert(line.clone(), ["feat/FEAT-001-x".to_string()].into());
        let got = tidy_candidates(&p, &releases, &model, &d, &|_| false, &BTreeMap::new());
        let names: Vec<_> = got.iter().map(|c| c.branch.as_str()).collect();
        assert_eq!(names, vec!["feat/FEAT-001-x"], "{got:?}");
        assert!(got[0].into == line || got[0].into == "main", "{got:?}");
        assert_eq!(got[0].held_by, None);
    }

    #[test]
    fn integrated_item_not_merged_is_an_error() {
        let p = project(&[("FEAT-001", "Completed")], false);
        let f = facts(&["main", "feat/FEAT-001-cart"], &["main"]);
        let model = gitmodel::derive(&p.config, &[], &f);
        let d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        let issues = check(&p, &[], &[], &model, &d);
        assert_eq!(issues.len(), 1, "{:?}", messages(&issues));
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(
            issues[0].message.contains("not merged into main"),
            "{}",
            issues[0].message
        );
    }

    #[test]
    fn finished_and_merged_and_parked_branches_are_warnings() {
        let p = project(
            &[("FEAT-001", "Completed"), ("FEAT-002", "Deferred")],
            false,
        );
        let f = facts(
            &["main", "feat/FEAT-001-cart", "feat/FEAT-002-later"],
            &["main", "feat/FEAT-001-cart"],
        );
        let model = gitmodel::derive(&p.config, &[], &f);
        let d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        let m = messages(&check(&p, &[], &[], &model, &d));
        assert!(
            m.iter()
                .any(|x| x.contains("FEAT-001 is finished") && x.contains("kanbanr git tidy")),
            "{m:?}"
        );
        assert!(
            m.iter()
                .any(|x| x.contains("FEAT-002 is Deferred (parked)")),
            "{m:?}"
        );
    }

    #[test]
    fn commits_for_unintegrated_work_on_a_line_are_errors() {
        let p = project(
            &[("FEAT-001", "Completed"), ("FEAT-002", "In Progress")],
            true,
        );
        let sprints = [Sprint {
            code: "SP-001".into(),
            state: SprintState::Active,
            ..Default::default()
        }];
        let f = facts(&["main", "sprint/SP-001"], &["main"]);
        let model = gitmodel::derive(&p.config, &sprints, &f);
        let mut d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        d.line_commits.insert(
            "sprint/SP-001".into(),
            vec![
                commit('a', "wip\n\nRefs: kanbanr:FEAT-002"),
                commit('b', "more\n\nRefs: kanbanr:FEAT-002"),
                commit('c', "both\n\nRefs: kanbanr:FEAT-001, kanbanr:FEAT-002"),
                commit('d', "x\n\n[no-ref] tooling"),
                commit('e', "ghost\n\nRefs: kanbanr:FEAT-404"),
            ],
        );
        let issues = check(&p, &sprints, &[], &model, &d);
        let m = messages(&issues);
        assert!(
            m.iter().any(|x| x == "sprint/SP-001 has 2 commits serving FEAT-002 (newest aaaaaaa), which is In Progress, not integrated"),
            "{m:?}"
        );
        assert!(
            m.iter()
                .any(|x| x.contains("names FEAT-404, which is not on the board")),
            "{m:?}"
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .count(),
            2,
            "{m:?}"
        );
    }

    /// DEF-012 R-1, R-2: with no integrate role, a status after the build stage is not reported as
    /// unintegrated; the build stage itself still is.
    #[test]
    fn commits_for_work_past_build_are_not_errors_without_an_integrate_role() {
        let mut p = project(
            &[
                ("FEAT-001", "Completed"),
                ("FEAT-002", "In Progress"),
                ("FEAT-003", "Ongoing"),
            ],
            true,
        );
        // Only the build role is declared: nothing says where merging happens.
        for gate in p.config.gates.values_mut() {
            gate.roles.retain(|r| r == "build");
        }
        assert!(p.config.roles().status(Role::Integrate).is_none());
        assert_eq!(p.config.roles().status(Role::Build), Some("In Progress"));
        let sprints = [Sprint {
            code: "SP-001".into(),
            state: SprintState::Active,
            ..Default::default()
        }];
        let f = facts(&["main", "sprint/SP-001"], &["main"]);
        let model = gitmodel::derive(&p.config, &sprints, &f);
        let mut d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        d.line_commits.insert(
            "sprint/SP-001".into(),
            vec![
                commit('a', "wip\n\nRefs: kanbanr:FEAT-002"),
                commit('b', "done\n\nRefs: kanbanr:FEAT-001"),
                commit('c', "more\n\nRefs: kanbanr:FEAT-003"),
            ],
        );
        let issues = check(&p, &sprints, &[], &model, &d);
        let m = messages(&issues);
        assert!(
            m.iter()
                .any(|x| x.contains("serving FEAT-002") && x.ends_with("not integrated")),
            "{m:?}"
        );
        assert!(
            !m.iter()
                .any(|x| x.contains("FEAT-001") || x.contains("FEAT-003")),
            "{m:?}"
        );
    }

    #[test]
    fn orphan_branches_and_unforwarded_releases_are_warnings() {
        let p = project(&[("FEAT-001", "In Progress")], false);
        let f = facts(
            &[
                "main",
                "wip",
                "feat/FEAT-999-x",
                "sprint/SP-099",
                "release/v9",
                "release/v1",
                "spike/try",
            ],
            &["main"],
        );
        let model = gitmodel::derive(&p.config, &[], &f);
        let releases = [Release {
            version: "v1".into(),
            branch: "release/v1".into(),
            ..Default::default()
        }];
        let mut d = DriftFacts {
            branches: f,
            ..Default::default()
        };
        d.unforwarded.insert(
            "release/v1".into(),
            vec![commit('f', "fix\n\nRefs: kanbanr:FEAT-001")],
        );
        let m = messages(&check(&p, &[], &releases, &model, &d));
        for want in [
            "wip names no item",
            "feat/FEAT-999-x names FEAT-999, which is not on the board",
            "sprint/SP-099 names no sprint",
            "release/v9 names no release",
            "release/v1 has 1 commit main lacks (newest fffffff): next, for the agent: bring release/v1 into main with `kanbanr git forward v1` — a local merge, nothing is pushed; a conflict stops it with nothing changed",
        ] {
            assert!(m.iter().any(|x| x == want), "missing {want:?} in {m:?}");
        }
        assert!(!m.iter().any(|x| x.contains("spike/try")), "{m:?}");
    }
}
