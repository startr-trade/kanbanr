//! Every guard rail a project has, in one read (FEAT-213): what the monitor's Workflow page lists.
//!
//! Pure assembly. Each rule lives where it always has — the method's parts in [`crate::method`],
//! the commit rules in [`crate::scm`], the protected branches in [`crate::gitmodel`], diagram
//! strictness in [`crate::config::DiagramsConfig`], coverage in [`crate::coverage`], the board's
//! push policy in [`crate::git`] — and this module only reads them together. Internal: served by
//! `GET /projects/{p}/guards`, not part of the published v1 API.

use crate::store::Store;
use crate::{Result, method};
use serde::Serialize;
use std::path::Path;

/// The trailer every commit must carry, naming the item it serves (FEAT-056).
pub const TRAILER: &str = "Refs";
/// How a commit escapes the trailer rule on the record (FEAT-056).
pub const ESCAPE: &str = "[no-ref] <reason>";

/// What `GET /projects/{p}/guards` answers.
#[derive(Debug, Clone, Serialize)]
pub struct Guards {
    /// The method's parts (`kanbanr config method`), each on or off.
    pub method: Vec<method::PartStatus>,
    pub commits: Commits,
    pub diagrams: Diagrams,
    /// Each coverage rule (`kanbanr config coverage`) and how many subjects it leaves uncovered.
    pub coverage: Vec<CoverageGuard>,
    pub push_policy: PushPolicyView,
    /// Who pushes the code repository: always the person, never kanbanr (FEAT-189 R-5).
    pub code_pushes: &'static str,
    /// What sub-agents may do (FEAT-214).
    pub agents: AgentGuards,
}

/// Agent scopes, as a guard rail (FEAT-214): the roles and what each allows, every scope in force,
/// and whether pushing from an agent's worktree is refused by the hook as well as by the guard.
#[derive(Debug, Clone, Serialize)]
pub struct AgentGuards {
    pub roles: Vec<AgentRole>,
    /// Issued and active scopes, across items.
    pub scopes: Vec<crate::agentscope::ScopeView>,
    /// Whether kanbanr's pre-push hook is installed. `None` where it cannot be told: the hooks are
    /// the CLI's to write and read.
    pub push_hook_installed: Option<bool>,
}

/// One role an agent scope is issued for, and the operations it carries.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRole {
    pub role: &'static str,
    pub operations: Vec<&'static str>,
}

/// The commit guard's rules (FEAT-056, FEAT-176, FEAT-189).
#[derive(Debug, Clone, Serialize)]
pub struct Commits {
    /// Work is committed on a branch named for its item.
    pub item_branch: bool,
    /// The item branch name, `{code}` and `{slug}` filled in.
    pub branch_pattern: String,
    /// The trailer naming the item.
    pub trailer: &'static str,
    /// The recorded escape from the trailer rule.
    pub escape: &'static str,
    /// Branches no commit lands on directly: they take merges only.
    pub protected: Vec<String>,
    /// Whether kanbanr's git hooks are installed in the code repository. `None` where it cannot
    /// be told: the hooks are the CLI's to write and read, and the daemon does not look.
    pub hooks_installed: Option<bool>,
}

/// How diagrams are linted (FEAT-200).
#[derive(Debug, Clone, Serialize)]
pub struct Diagrams {
    /// Every written spec and doc is linted; a finding is reported with the write.
    pub lint_on_write: bool,
    /// A write carrying a finding is refused.
    pub strict: bool,
}

/// One coverage rule, judged.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageGuard {
    pub id: String,
    pub subjects: usize,
    pub uncovered: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// The board's push policy (FEAT-034, FEAT-142).
#[derive(Debug, Clone, Serialize)]
pub struct PushPolicyView {
    /// `auto`, `off` or `debounce:N`.
    pub policy: String,
    /// Where it came from: `environment` (`KANBANR_PUSH`), `board` (its own setting) or `default`.
    pub source: &'static str,
}

/// Every guard rail of project `id`. `repo` is the code repository when the caller has one: it
/// names the default branch where the project's config does not.
pub fn report(store: &Store, id: &str, repo: Option<&Path>) -> Result<Guards> {
    let project = store.load_meta(id)?;
    let config = &project.config;
    let sprints = crate::sprints::load(store, id)?;
    let model = crate::gitmodel::preview(config.clone(), None, None, &sprints, repo);

    let coverage = if config.coverage.is_empty() {
        Vec::new()
    } else {
        let snap = crate::coverage::Snapshot::load(store, &project)?;
        snap.report(&project)
            .rules
            .into_iter()
            .map(|r| CoverageGuard {
                id: r.id,
                subjects: r.subjects,
                uncovered: r.uncovered.len(),
                problem: r.problem,
            })
            .collect()
    };

    let (policy, source) = crate::git::PushPolicy::for_board(store.data_dir());
    let source = match source {
        crate::git::PolicySource::Environment => "environment",
        crate::git::PolicySource::Board => "board",
        crate::git::PolicySource::Default => "default",
    };

    Ok(Guards {
        method: method::status(config),
        commits: Commits {
            item_branch: true,
            branch_pattern: config.branch_pattern().to_string(),
            trailer: TRAILER,
            escape: ESCAPE,
            protected: model.protected,
            hooks_installed: None,
        },
        diagrams: Diagrams {
            lint_on_write: true,
            strict: store.read_project_config(id)?.diagrams.strict,
        },
        coverage,
        push_policy: PushPolicyView {
            policy: policy.to_string(),
            source,
        },
        code_pushes: crate::gitmodel::PUSHES,
        agents: agents(store, id)?,
    })
}

/// The agent-scope guard rail of project `id` (FEAT-214).
fn agents(store: &Store, id: &str) -> Result<AgentGuards> {
    use crate::agentscope::{self, Role};
    let now = crate::now_rfc3339();
    let scopes = agentscope::load(store, id)?;
    Ok(AgentGuards {
        roles: [Role::Design, Role::Build]
            .into_iter()
            .map(|r| AgentRole {
                role: r.name(),
                operations: r.operations().iter().map(|o| o.as_str()).collect(),
            })
            .collect(),
        scopes: agentscope::active(&scopes, &now)
            .into_iter()
            .map(|s| agentscope::ScopeView::new(s, &now))
            .collect(),
        push_hook_installed: None,
    })
}
