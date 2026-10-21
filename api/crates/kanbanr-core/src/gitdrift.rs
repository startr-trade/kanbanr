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
        if b.starts_with(SPRINT_PREFIX) && !facts.merged_into_main.contains(b) {
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
                    "{code} is finished and {branch} is merged: delete it with `git branch -d \
                     {branch}`"
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
                    .any(|f| gitmodel::is_integrated(config, &f.status))
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
                "{branch} has {} commit{} {line} lacks (newest {}): `kanbanr git forward {version}`",
                commits.len(),
                if commits.len() == 1 { "" } else { "s" },
                short(&newest.sha)
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

#[cfg(test)]
mod tests {
    use super::*;
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
                .any(|x| x.contains("FEAT-001 is finished") && x.contains("git branch -d")),
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
            "release/v1 has 1 commit main lacks (newest fffffff): `kanbanr git forward v1`",
        ] {
            assert!(m.iter().any(|x| x == want), "missing {want:?} in {m:?}");
        }
        assert!(!m.iter().any(|x| x.contains("spike/try")), "{m:?}");
    }
}
