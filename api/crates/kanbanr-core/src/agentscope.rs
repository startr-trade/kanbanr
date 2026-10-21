//! Agent scopes (FEAT-214): what one sub-agent may do, recorded on the board and enforced by
//! kanbanr.
//!
//! A scope is issued by the coordinator for one item and one role, and bound to the agent's own
//! git worktree by the agent's first command (`kanbanr scope claim`). From then on every kanbanr
//! process, git hook and Claude Code guard started in that worktree resolves it, and anything
//! outside it is refused with the boundary it crossed named.
//!
//! The board is the authority: `projects/<id>/scopes.yaml` holds every scope, oldest first, never
//! trimmed. The worktree holds only a pointer, `<git-dir>/kanbanr-scope.yaml`, private to that
//! worktree: it cannot be committed, survives `git clean`, and goes when the worktree goes.
//! Resolution fails closed: an agent worktree with no valid scope bound to it is refused.
//!
//! A board inside the system temporary directory is never governed (D7): build agents run the
//! test suite in their worktrees, and its scratch boards live there.

use crate::coderepo::{self, Commit};
use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::scm::{RepoFacts, Verdict};
use crate::store::{Project, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Every scope of a project, in `projects/<id>/`.
pub const SCOPES_FILE: &str = "scopes.yaml";
/// The pointer in a worktree's own git directory.
pub const POINTER_FILE: &str = "kanbanr-scope.yaml";
/// The trailer on every board and code commit made under a scope.
pub const TRAILER: &str = "Agent-Scope";
/// Where Claude Code makes an agent's worktree, relative to the main worktree.
pub const AGENT_WORKTREES: &str = ".claude/worktrees";
/// How long a scope lasts unless the coordinator says otherwise (Q3).
pub const DEFAULT_HOURS: u32 = 8;
/// The longest a scope may last; it is renewable (Q3).
pub const MAX_HOURS: u32 = 72;

// ---- the model ------------------------------------------------------------------------------

/// What kind of agent a scope is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Reads only: every board write and every commit is refused.
    Design,
    /// Records its own item's task and test states, and commits on its item's branch.
    Build,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.trim().to_ascii_lowercase().as_str() {
            "design" => Some(Role::Design),
            "build" => Some(Role::Build),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Design => "design",
            Role::Build => "build",
        }
    }

    /// The operations the role is issued with.
    pub fn operations(self) -> Vec<Operation> {
        match self {
            Role::Design => Vec::new(),
            Role::Build => vec![
                Operation::TaskState,
                Operation::TestState,
                Operation::Commit,
            ],
        }
    }
}

/// An operation a scope may allow. The names are the batch op names, so a batch is checked by
/// the same vocabulary as a single write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    #[serde(rename = "task.state")]
    TaskState,
    #[serde(rename = "test.state")]
    TestState,
    #[serde(rename = "commit")]
    Commit,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::TaskState => "task.state",
            Operation::TestState => "test.state",
            Operation::Commit => "commit",
        }
    }

    pub fn parse(s: &str) -> Option<Operation> {
        match s.trim() {
            "task.state" => Some(Operation::TaskState),
            "test.state" => Some(Operation::TestState),
            "commit" => Some(Operation::Commit),
            _ => None,
        }
    }
}

/// Where a scope is in its life. Derived, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeState {
    /// Issued, not yet claimed by a worktree.
    Issued,
    /// Claimed and not expired.
    Active,
    /// Its expiry has passed.
    Expired,
    /// Revoked; wins over expired.
    Revoked,
}

impl ScopeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeState::Issued => "issued",
            ScopeState::Active => "active",
            ScopeState::Expired => "expired",
            ScopeState::Revoked => "revoked",
        }
    }

    /// Issued or active: a scope still in force.
    pub fn is_live(self) -> bool {
        matches!(self, ScopeState::Issued | ScopeState::Active)
    }
}

/// One scope, as `scopes.yaml` holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    pub id: String,
    /// The project whose `scopes.yaml` holds it: filled in when read, never written.
    #[serde(skip)]
    pub project: String,
    pub role: Role,
    pub item: String,
    #[serde(default)]
    pub operations: Vec<Operation>,
    /// The item branch (build only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The tip of `branch` at issue; drift counts `base..branch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub issued_by: String,
    pub issued_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The bound worktree, relative to the main worktree's root, `/`-separated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
    /// Keys a newer kanbanr wrote, kept (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,
}

impl Scope {
    /// Revoked wins over expired; expired over claimed.
    pub fn state(&self, now: &str) -> ScopeState {
        if self.revoked_at.is_some() {
            ScopeState::Revoked
        } else if !crate::before(now, &self.expires_at) {
            ScopeState::Expired
        } else if self.worktree.is_some() {
            ScopeState::Active
        } else {
            ScopeState::Issued
        }
    }

    pub fn by_agent(&self) -> ByAgent {
        ByAgent {
            scope: self.id.clone(),
            role: self.role.name().to_string(),
            item: self.item.clone(),
        }
    }

    /// Does this scope allow `op` on `item`?
    pub fn allows(&self, op: Operation, item: &str) -> bool {
        self.operations.contains(&op) && self.item == item
    }

    /// What the scope allows, in words for a refusal.
    fn allowed_text(&self) -> String {
        if self.operations.is_empty() {
            return "nothing but reading the board and the code".to_string();
        }
        let mut parts = Vec::new();
        if self.operations.contains(&Operation::TaskState) {
            parts.push(format!("`kanbanr task state {} …`", self.item));
        }
        if self.operations.contains(&Operation::TestState) {
            parts.push(format!("`kanbanr test {} …`", self.item));
        }
        if self.operations.contains(&Operation::Commit) {
            parts.push(format!(
                "commits on {} naming {}",
                self.branch.as_deref().unwrap_or("its branch"),
                self.item
            ));
        }
        parts.join(", ")
    }
}

/// Who a write under a scope was made by: the person, by the agent for the item. Recorded on the
/// activity entry and the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ByAgent {
    /// The scope it was made under, `SC-007`.
    pub scope: String,
    /// `design` or `build`.
    pub role: String,
    /// The item the scope is for.
    pub item: String,
}

impl std::fmt::Display for ByAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "by the agent for {} ({})", self.item, self.scope)
    }
}

/// A scope as the routes answer with it: the record, its project and its derived state.
#[derive(Debug, Clone, Serialize)]
pub struct ScopeView {
    #[serde(flatten)]
    pub scope: Scope,
    pub project: String,
    pub state: ScopeState,
}

impl ScopeView {
    pub fn new(scope: &Scope, now: &str) -> ScopeView {
        ScopeView {
            scope: scope.clone(),
            project: scope.project.clone(),
            state: scope.state(now),
        }
    }
}

/// The pointer in a worktree's git directory. Nothing else: the board holds the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerFile {
    pub scope: String,
    pub project: String,
}

/// What resolution reads about the working tree a process runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeFacts {
    /// This worktree's root.
    pub root: PathBuf,
    /// This worktree's own git directory (`.git/worktrees/<name>` for a linked one).
    pub git_dir: PathBuf,
    /// The main worktree's root.
    pub main_root: PathBuf,
    /// A linked worktree, not the main checkout.
    pub linked: bool,
    /// The checked-out branch; `None` when detached.
    pub branch: Option<String>,
}

impl WorktreeFacts {
    /// This worktree's path relative to the main worktree, `/`-separated; its full path when it
    /// lies elsewhere.
    pub fn rel(&self) -> String {
        match self.root.strip_prefix(&self.main_root) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => self.root.to_string_lossy().replace('\\', "/"),
        }
    }

    /// Is this a worktree Claude Code made for an agent?
    pub fn under_agent_worktrees(&self) -> bool {
        self.linked && self.root.starts_with(self.main_root.join(AGENT_WORKTREES))
    }
}

/// The scope a process works under.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // one per process, resolved once: boxing buys nothing
pub enum ProcessScope {
    /// No scope applies: the main checkout, a board with no scopes, a temporary board.
    Unscoped,
    Scoped(Scope),
    /// An agent context with no valid scope: every write and commit is refused. `may_claim` lets
    /// `scope claim` through.
    Refused {
        why: String,
        may_claim: bool,
    },
}

/// One thing a write asks to do, in the scope vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    /// The operation, when the write is one a scope can allow.
    pub op: Option<Operation>,
    pub project: String,
    pub item: Option<String>,
    /// What to call it in a refusal.
    pub label: String,
}

/// Why a scope refused something. Its text is the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---- time -----------------------------------------------------------------------------------

fn add_hours(now: &str, hours: u32) -> Result<String> {
    let at = OffsetDateTime::parse(now.trim(), &Rfc3339)
        .map_err(|e| CoreError::Unsupported(format!("'{now}' is not a timestamp: {e}")))?;
    (at + time::Duration::hours(i64::from(hours)))
        .format(&Rfc3339)
        .map_err(|e| CoreError::Unsupported(format!("timestamp: {e}")))
}

/// `2026-10-21 18:00 UTC`, for a brief.
fn readable(at: &str) -> String {
    match OffsetDateTime::parse(at.trim(), &Rfc3339) {
        Ok(t) => {
            let t = t.to_offset(time::UtcOffset::UTC);
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                t.year(),
                u8::from(t.month()),
                t.day(),
                t.hour(),
                t.minute()
            )
        }
        Err(_) => at.to_string(),
    }
}

// ---- storage --------------------------------------------------------------------------------

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(SCOPES_FILE)
}

/// A project's scopes, oldest first. None is a normal state.
pub fn load(store: &Store, id: &str) -> Result<Vec<Scope>> {
    let mut scopes: Vec<Scope> = match std::fs::read_to_string(path(store, id)) {
        Ok(s) if !s.trim().is_empty() => crate::yaml::from_str(&s)?,
        _ => Vec::new(),
    };
    for s in &mut scopes {
        s.project = id.to_string();
    }
    Ok(scopes)
}

/// Every project's scopes. A project that cannot be read contributes none.
pub fn load_all(store: &Store) -> Result<Vec<Scope>> {
    let mut out = Vec::new();
    for id in store.list_projects()? {
        out.extend(load(store, &id).unwrap_or_default());
    }
    Ok(out)
}

fn save(store: &Store, id: &str, scopes: &[Scope]) -> Result<()> {
    std::fs::write(path(store, id), crate::yaml::to_string(scopes)?)?;
    Ok(())
}

fn find_mut<'a>(scopes: &'a mut [Scope], id: &str) -> Result<&'a mut Scope> {
    scopes
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::Unsupported(format!("no agent scope {id}")))
}

fn next_id(scopes: &[Scope]) -> String {
    let n = scopes
        .iter()
        .filter_map(|s| s.id.strip_prefix("SC-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    format!("SC-{n:03}")
}

/// Scopes still in force: issued or active. Expiry is derived; nothing is rewritten.
pub fn active<'a>(scopes: &'a [Scope], now: &str) -> Vec<&'a Scope> {
    scopes.iter().filter(|s| s.state(now).is_live()).collect()
}

// ---- lifecycle ------------------------------------------------------------------------------

/// What `POST /projects/:p/scopes` asks for.
#[derive(Debug, Clone)]
pub struct IssueRequest {
    pub role: Role,
    pub item: String,
    /// Default [`DEFAULT_HOURS`], at most [`MAX_HOURS`].
    pub hours: Option<u32>,
    pub note: String,
    /// Bind at once, when the coordinator already knows the agent's worktree.
    pub worktree: Option<PathBuf>,
    /// The CLI's word that the commit hooks and the pre-push hook are installed in the repository:
    /// the hooks are its to write and read.
    pub hooks_ready: bool,
}

/// What issuing made: the scope, the brief for the agent, and whether the branch was created.
#[derive(Debug, Clone, Serialize)]
pub struct Issued {
    pub scope: ScopeView,
    pub brief: String,
    pub branch_created: bool,
}

fn check_hours(hours: u32) -> Result<()> {
    if hours == 0 || hours > MAX_HOURS {
        return Err(CoreError::Unsupported(format!(
            "a scope lasts between 1 and {MAX_HOURS} hours; renew it to go on"
        )));
    }
    Ok(())
}

/// The item's branch: the one that exists, or one made for it now. Returns it and whether it was
/// made.
fn item_branch(
    store: &Store,
    project: &Project,
    item: &FeatureItem,
    root: &Path,
) -> Result<(String, bool)> {
    let config = &project.config;
    let pattern = config.branch_pattern();
    let main = config
        .cadence
        .main_branch
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| coderepo::default_branch(root));
    let facts = coderepo::branch_facts(root, &main)?;
    if let Some(existing) = facts.branches.keys().find(|b| {
        crate::scm::item_code_from_branch(pattern, b).as_deref() == Some(item.code.as_str())
    }) {
        return Ok((existing.clone(), false));
    }
    let name = crate::scm::branch_for(pattern, &item.code, &item.title);
    let base = if config.cadence.git_model {
        let (model, facts) = crate::gitmodel::load(store, &project.id, root)?;
        let releases = crate::releases::load(store, &project.id).unwrap_or_default();
        let sprints = crate::sprints::load(store, &project.id)?;
        let start = crate::gitmodel::start_base(&model, &facts, item, &releases, &sprints);
        if start.create_from_main {
            coderepo::create_branch_from(root, &start.base, &model.main)?;
        }
        start.base
    } else {
        coderepo::default_branch(root)
    };
    coderepo::create_branch_from(root, &name, &base)?;
    Ok((name, true))
}

/// Issue a scope (R-1): record it with its issuer, item, operations, branch, issue time and
/// expiry, and return the brief. A build scope's item must be at the build stage, and its branch
/// is found or made, never checked out.
pub fn issue(
    store: &Store,
    id: &str,
    req: &IssueRequest,
    by: &str,
    repo: Option<&Path>,
    now: &str,
) -> Result<Issued> {
    let hours = req.hours.unwrap_or(DEFAULT_HOURS);
    check_hours(hours)?;
    let project = store.load_meta(id)?;
    let item = project.feature(&req.item)?.clone();
    let root = repo.ok_or_else(|| {
        CoreError::Unsupported(
            "issuing an agent scope reads the code repository: run it from the project folder"
                .into(),
        )
    })?;
    if !req.hooks_ready {
        return Err(CoreError::Unsupported(
            "an agent scope is enforced by kanbanr's git hooks, and they are not all installed \
             here: run `kanbanr git install-hooks` (it adds the pre-push hook) and issue again"
                .into(),
        ));
    }
    if req.role == Role::Build {
        match project.config.role_status(crate::config::Role::Build) {
            Some(build) if build == item.status => {}
            Some(build) => {
                return Err(CoreError::Unsupported(format!(
                    "{} is in {}, and a build agent works on an item at the build stage: move it \
                     first: `kanbanr move {} \"{build}\"`",
                    item.code, item.status, item.code
                )));
            }
            None => {
                return Err(CoreError::Unsupported(
                    "this workflow has no build stage, so there is nothing to build under a scope"
                        .into(),
                ));
            }
        }
    }
    let mut scopes = load(store, id)?;
    let (branch, base, created) = if req.role == Role::Build {
        let (branch, created) = item_branch(store, &project, &item, root)?;
        if let Some(at) = coderepo::worktree_of(root, &branch) {
            let main_root = coderepo::worktree_facts(root).map(|w| w.main_root);
            let agent = main_root.as_ref().is_some_and(|m| {
                crate::project::normalize(&at).starts_with(m.join(AGENT_WORKTREES))
            });
            if !agent {
                return Err(CoreError::Unsupported(format!(
                    "{branch} is checked out at {}: the agent switches to it in its own worktree, \
                     so switch that checkout off it first",
                    at.display()
                )));
            }
        }
        let base = coderepo::rev_parse(root, &branch).ok();
        (Some(branch), base, created)
    } else {
        (None, None, false)
    };
    let scope = Scope {
        id: next_id(&scopes),
        project: id.to_string(),
        role: req.role,
        item: item.code.clone(),
        operations: req.role.operations(),
        branch,
        base,
        issued_by: by.to_string(),
        issued_at: now.to_string(),
        expires_at: add_hours(now, hours)?,
        note: req.note.trim().to_string(),
        worktree: None,
        claimed_at: None,
        revoked_at: None,
        revoked_by: None,
        revoked_reason: None,
        extra: Default::default(),
    };
    scopes.push(scope.clone());
    if let Err(e) = save(store, id, &scopes) {
        if created && let Some(b) = &scope.branch {
            let _ = coderepo::delete_branch(root, b);
        }
        return Err(e);
    }
    let scope = match &req.worktree {
        Some(path) => {
            let wt = coderepo::worktree_facts(path).ok_or_else(|| {
                CoreError::Unsupported(format!("{} is not a git worktree", path.display()))
            })?;
            claim(store, id, &scope.id, &wt, now)?
        }
        None => scope,
    };
    Ok(Issued {
        brief: brief(&scope, &project),
        scope: ScopeView::new(&scope, now),
        branch_created: created,
    })
}

/// Bind a scope to the worktree it is claimed from (R-2): the build agent's worktree switches to
/// the item branch, the board records the worktree, and the pointer is written into the worktree's
/// git directory — in that order, undoing the switch if the board write fails (FEAT-183).
/// Claiming again from the same worktree rewrites a lost pointer.
pub fn claim(
    store: &Store,
    id: &str,
    scope_id: &str,
    wt: &WorktreeFacts,
    now: &str,
) -> Result<Scope> {
    let mut scopes = load(store, id)?;
    let rel = wt.rel();
    let scope = find_mut(&mut scopes, scope_id)?.clone();
    match scope.state(now) {
        ScopeState::Revoked => return Err(CoreError::Unsupported(revoked_message(&scope))),
        ScopeState::Expired => return Err(CoreError::Unsupported(expired_message(&scope))),
        ScopeState::Active if scope.worktree.as_deref() != Some(rel.as_str()) => {
            return Err(CoreError::Unsupported(format!(
                "agent scope {scope_id} is already bound to {}: one scope, one worktree",
                scope.worktree.as_deref().unwrap_or_default()
            )));
        }
        _ => {}
    }
    if !wt.linked {
        return Err(CoreError::Unsupported(
            "a scope is claimed from the agent's own worktree (a linked git worktree), never from \
             the main checkout"
                .into(),
        ));
    }
    let mut switched_from = None;
    if let Some(branch) = &scope.branch
        && wt.branch.as_deref() != Some(branch.as_str())
    {
        if !coderepo::is_clean(&wt.root) {
            return Err(CoreError::Unsupported(format!(
                "this worktree has uncommitted changes, so it cannot switch to {branch}: commit \
                 or discard them, then claim again"
            )));
        }
        coderepo::git(&wt.root, &["switch", "-q", branch])?;
        switched_from = Some(wt.branch.clone());
    }
    let undo = |from: &Option<Option<String>>| {
        if let Some(Some(b)) = from {
            let _ = coderepo::git(&wt.root, &["switch", "-q", b]);
        }
    };
    {
        let s = find_mut(&mut scopes, scope_id)?;
        if s.worktree.is_none() {
            s.worktree = Some(rel.clone());
            s.claimed_at = Some(now.to_string());
        }
    }
    if let Err(e) = save(store, id, &scopes) {
        undo(&switched_from);
        return Err(e);
    }
    write_pointer(
        &wt.git_dir,
        &PointerFile {
            scope: scope_id.to_string(),
            project: id.to_string(),
        },
    )
    .map_err(|e| {
        CoreError::Unsupported(format!(
            "the board records the claim, but the pointer could not be written into {}: {e} — \
             claim again from this worktree",
            wt.git_dir.display()
        ))
    })?;
    Ok(find_mut(&mut scopes, scope_id)?.clone())
}

/// Revoke a scope (R-8): nothing more is recorded under it.
pub fn revoke(
    store: &Store,
    id: &str,
    scope_id: &str,
    by: &str,
    reason: &str,
    now: &str,
) -> Result<Scope> {
    let mut scopes = load(store, id)?;
    let s = find_mut(&mut scopes, scope_id)?;
    if s.revoked_at.is_some() {
        return Err(CoreError::Unsupported(format!(
            "agent scope {scope_id} is already revoked"
        )));
    }
    s.revoked_at = Some(now.to_string());
    s.revoked_by = Some(by.to_string());
    s.revoked_reason = Some(
        Some(reason.trim())
            .filter(|r| !r.is_empty())
            .unwrap_or("revoked")
            .to_string(),
    );
    let out = s.clone();
    save(store, id, &scopes)?;
    Ok(out)
}

/// Renew a scope for `hours` from now. A revoked scope stays revoked.
pub fn renew(store: &Store, id: &str, scope_id: &str, hours: u32, now: &str) -> Result<Scope> {
    check_hours(hours)?;
    let mut scopes = load(store, id)?;
    let s = find_mut(&mut scopes, scope_id)?;
    if s.revoked_at.is_some() {
        return Err(CoreError::Unsupported(format!(
            "agent scope {scope_id} is revoked: issue a new one"
        )));
    }
    s.expires_at = add_hours(now, hours)?;
    let out = s.clone();
    save(store, id, &scopes)?;
    Ok(out)
}

// ---- resolution -----------------------------------------------------------------------------

/// The pointer in a worktree's git directory, if there is a readable one.
pub fn read_pointer(git_dir: &Path) -> Option<PointerFile> {
    let raw = std::fs::read_to_string(git_dir.join(POINTER_FILE)).ok()?;
    crate::yaml::from_str(&raw).ok()
}

pub fn write_pointer(git_dir: &Path, p: &PointerFile) -> std::io::Result<()> {
    let yaml = crate::yaml::to_string(p).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(git_dir.join(POINTER_FILE), yaml)
}

fn expired_message(s: &Scope) -> String {
    format!(
        "refused: agent scope {} expired at {}. The coordinator renews it (`kanbanr scope renew {} \
         --for 4h`) or issues a new one.",
        s.id,
        readable(&s.expires_at),
        s.id
    )
}

fn revoked_message(s: &Scope) -> String {
    format!(
        "refused: agent scope {} was revoked by {} at {} ({}). Nothing more is recorded under it.",
        s.id,
        s.revoked_by.as_deref().unwrap_or("the coordinator"),
        readable(s.revoked_at.as_deref().unwrap_or_default()),
        s.revoked_reason.as_deref().unwrap_or("revoked")
    )
}

fn unbound_message(wt: &WorktreeFacts) -> String {
    format!(
        "refused: this is an agent worktree ({}) and no agent scope is bound to it. If you are the \
         agent, run the claim command from your brief (`kanbanr scope claim SC-…`); otherwise ask \
         the coordinator to issue one (`kanbanr scope issue <design|build> <CODE>`).",
        wt.root.display()
    )
}

/// The scope a process works under, from the board's scopes, the worktree it runs in and the
/// pointer there. Pure. Rules, in order: no scopes on the board, or no work tree, is unscoped; a
/// pointer naming a scope here must match it and it must be in force; an agent worktree with no
/// valid pointer is refused; anything else is unscoped.
pub fn resolve_with(
    scopes: &[Scope],
    wt: Option<&WorktreeFacts>,
    pointer: Option<&PointerFile>,
    now: &str,
) -> ProcessScope {
    if scopes.is_empty() {
        return ProcessScope::Unscoped;
    }
    let Some(wt) = wt else {
        return ProcessScope::Unscoped;
    };
    let rel = wt.rel();
    if let Some(p) = pointer
        && let Some(s) = scopes.iter().find(|s| s.id == p.scope)
    {
        if s.project != p.project {
            return ProcessScope::Refused {
                why: format!(
                    "refused: the scope file here names {} of {}, which is bound to {} of {}.",
                    p.scope, p.project, s.id, s.project
                ),
                may_claim: false,
            };
        }
        match s.worktree.as_deref() {
            None => {
                return ProcessScope::Refused {
                    why: format!(
                        "refused: the scope file here names {}, which is bound to no worktree yet \
                         — claim it (`kanbanr scope claim {}`).",
                        s.id, s.id
                    ),
                    may_claim: true,
                };
            }
            Some(bound) if bound != rel => {
                return ProcessScope::Refused {
                    why: format!(
                        "refused: the scope file here names {}, which is bound to {bound}.",
                        s.id
                    ),
                    may_claim: false,
                };
            }
            Some(_) => {}
        }
        return match s.state(now) {
            ScopeState::Revoked => ProcessScope::Refused {
                why: revoked_message(s),
                may_claim: false,
            },
            ScopeState::Expired => ProcessScope::Refused {
                why: expired_message(s),
                may_claim: false,
            },
            _ => ProcessScope::Scoped(s.clone()),
        };
    }
    let bound_here = scopes
        .iter()
        .any(|s| s.worktree.as_deref() == Some(rel.as_str()));
    let branch_here = wt.branch.as_deref().is_some_and(|b| {
        scopes
            .iter()
            .any(|s| s.revoked_at.is_none() && s.branch.as_deref() == Some(b))
    });
    if wt.linked && (wt.under_agent_worktrees() || bound_here || branch_here) {
        return ProcessScope::Refused {
            why: unbound_message(wt),
            may_claim: true,
        };
    }
    ProcessScope::Unscoped
}

/// Is this board one scopes govern? Never one inside the system temporary directory (D7): test
/// boards live there, and build agents run the tests in their worktrees.
pub fn is_governed_board(data_dir: &Path) -> bool {
    let board = crate::project::normalize(data_dir);
    let temp = crate::project::normalize(&std::env::temp_dir());
    !board.starts_with(&temp)
}

/// The scope a process in `cwd` writes `store` under.
pub fn resolve(store: &Store, cwd: &Path) -> ProcessScope {
    if !is_governed_board(store.data_dir()) {
        return ProcessScope::Unscoped;
    }
    let scopes = load_all(store).unwrap_or_default();
    let wt = coderepo::worktree_facts(cwd);
    let pointer = wt.as_ref().and_then(|w| read_pointer(&w.git_dir));
    resolve_with(
        &scopes,
        wt.as_ref(),
        pointer.as_ref(),
        &crate::now_rfc3339(),
    )
}

/// The worktree `cwd` is in, when it is an agent's: it has a pointer, or Claude Code made it under
/// `.claude/worktrees/`. Needs no board.
pub fn agent_context(cwd: &Path) -> Option<WorktreeFacts> {
    let wt = coderepo::worktree_facts(cwd)?;
    (wt.git_dir.join(POINTER_FILE).is_file() || wt.under_agent_worktrees()).then_some(wt)
}

/// M11 when `cwd` is an agent context: `what` is the command and what it does, e.g.
/// "`kanbanr identity` changes kanbanr itself".
pub fn outside_core_refusal(cwd: &Path, what: &str) -> Option<String> {
    let wt = agent_context(cwd)?;
    let id = read_pointer(&wt.git_dir)
        .map(|p| p.scope)
        .unwrap_or_else(|| "none".to_string());
    Some(format!(
        "refused by agent scope {id}: {what}, which no agent scope allows."
    ))
}

// ---- checking -------------------------------------------------------------------------------

/// What a data-route write asks to do, in scope terms. A read asks nothing.
pub fn requested(method: &str, path: &str, body: Option<&Value>) -> Vec<Requested> {
    if !crate::dispatch::is_mutation(method) {
        return Vec::new();
    }
    let project = crate::write::project_of(path).unwrap_or_default();
    let bare = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    let one = |op: Option<Operation>, item: Option<String>, label: String| Requested {
        op,
        project: project.clone(),
        item,
        label,
    };
    match (method.to_ascii_uppercase().as_str(), segs.as_slice()) {
        ("PUT", ["projects", _, "features", code, "todos", _, "tasks", _]) => vec![one(
            Some(Operation::TaskState),
            Some(code.to_string()),
            "task.state".into(),
        )],
        ("PUT", ["projects", _, "features", code, "tests", _, _]) => vec![one(
            Some(Operation::TestState),
            Some(code.to_string()),
            "test.state".into(),
        )],
        ("POST", ["projects", _, "scopes", _, "claim"]) => {
            vec![one(None, None, "scope.claim".into())]
        }
        ("POST", ["projects", _, "batch"]) => {
            let ops = body
                .and_then(|b| b.get("operations"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if ops.is_empty() {
                return vec![one(None, None, "batch".into())];
            }
            ops.iter()
                .map(|op| {
                    let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
                    let item = op.get("feature").and_then(Value::as_str).map(String::from);
                    match Operation::parse(name).filter(|o| *o != Operation::Commit) {
                        Some(o) => one(Some(o), item, o.as_str().to_string()),
                        None => one(None, item, format!("batch op {name}")),
                    }
                })
                .collect()
        }
        _ => vec![one(
            None,
            crate::write::item_of(path),
            crate::dispatch::commit_message(method, path, body),
        )],
    }
}

/// Is a write under `scope` within it? The first boundary crossed is named (R-3).
pub fn check(scope: &Scope, req: &[Requested], now: &str) -> std::result::Result<(), Refusal> {
    match scope.state(now) {
        ScopeState::Revoked => return Err(Refusal(revoked_message(scope))),
        ScopeState::Expired => return Err(Refusal(expired_message(scope))),
        _ => {}
    }
    let head = format!(
        "refused by agent scope {} ({}, {})",
        scope.id,
        scope.role.name(),
        scope.item
    );
    for r in req {
        let allowed = r.op.is_some_and(|op| scope.operations.contains(&op));
        if !allowed {
            return Err(Refusal(format!(
                "{head}: `{}` is not one of its operations. Under this scope you may: {}. Report \
                 what you needed to the coordinator; do not work around it.",
                r.label,
                scope.allowed_text()
            )));
        }
        let other = if r.project != scope.project {
            Some(format!("project {}", r.project))
        } else if r.item.as_deref() != Some(scope.item.as_str()) {
            Some(r.item.clone().unwrap_or_else(|| "no item".into()))
        } else {
            None
        };
        if let Some(other) = other {
            return Err(Refusal(format!(
                "{head}: `{}` is on {other}; this scope covers {} only.",
                r.label, scope.item
            )));
        }
    }
    Ok(())
}

/// A commit under `p` (R-5): on the scope's branch, naming its item, under a build scope in force.
/// The message is judged where it is known; `None` leaves it to the commit-msg hook.
pub fn commit_verdict(p: &ProcessScope, facts: &RepoFacts, now: &str) -> Verdict {
    let s = match p {
        ProcessScope::Unscoped => return Verdict::allow(),
        ProcessScope::Refused { why, .. } => return Verdict::refuse(why.clone()),
        ProcessScope::Scoped(s) => s,
    };
    match s.state(now) {
        ScopeState::Revoked => return Verdict::refuse(revoked_message(s)),
        ScopeState::Expired => return Verdict::refuse(expired_message(s)),
        _ => {}
    }
    if !s.operations.contains(&Operation::Commit) {
        return Verdict::refuse(format!(
            "refused by agent scope {} ({}, {}): a design agent does not commit or edit code.",
            s.id,
            s.role.name(),
            s.item
        ));
    }
    let want = s.branch.as_deref().unwrap_or_default();
    if facts.branch.as_deref() != Some(want) {
        return Verdict::refuse(format!(
            "refused by agent scope {} (build, {}): this commit is on {}; this agent commits only \
             on {want}.",
            s.id,
            s.item,
            facts.branch.as_deref().unwrap_or("a detached head")
        ));
    }
    if let Some(message) = facts.message.as_deref() {
        let names = crate::scm::parse_refs(message)
            .iter()
            .any(|r| r.code() == s.item);
        if !names {
            return Verdict::refuse(format!(
                "refused by agent scope {} (build, {}): a commit under this scope must name {} \
                 (`Refs: kanbanr:{}/R-n`); use `kanbanr commit -m \"…\"`.",
                s.id, s.item, s.item, s.item
            ));
        }
        if let Some(other) = trailer_scope(message).filter(|id| *id != s.id) {
            return Verdict::refuse(format!(
                "refused by agent scope {}: this message carries the trailer of {other}, another \
                 scope.",
                s.id
            ));
        }
    }
    Verdict::allow()
}

/// A push from an agent context (R-6): never.
pub fn push_verdict(p: &ProcessScope) -> Verdict {
    let id = match p {
        ProcessScope::Unscoped => return Verdict::allow(),
        ProcessScope::Scoped(s) => s.id.clone(),
        ProcessScope::Refused { .. } => "none".to_string(),
    };
    Verdict::refuse(push_refusal(&id))
}

/// M10.
pub fn push_refusal(id: &str) -> String {
    format!(
        "refused by agent scope {id}: agents never push. The coordinator reviews the branch and \
         pushes it."
    )
}

/// The ops of a batch a scope permits, and how many it dropped: what capture records under a
/// scope (D5).
pub fn retain_permitted(scope: &Scope, ops: Vec<Value>) -> (Vec<Value>, usize) {
    let before = ops.len();
    let kept: Vec<Value> = ops
        .into_iter()
        .filter(|op| {
            let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
            let item = op
                .get("feature")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Operation::parse(name)
                .filter(|o| *o != Operation::Commit)
                .is_some_and(|o| scope.allows(o, item))
        })
        .collect();
    let dropped = before - kept.len();
    (kept, dropped)
}

/// `Agent-Scope: SC-007 (build, by the agent for FEAT-214)`.
pub fn trailer(scope: &Scope) -> String {
    format!(
        "{TRAILER}: {} ({}, by the agent for {})",
        scope.id,
        scope.role.name(),
        scope.item
    )
}

/// The scope id an `Agent-Scope:` trailer names, if the message has one.
pub fn trailer_scope(message: &str) -> Option<&str> {
    message.lines().find_map(|l| {
        l.trim()
            .strip_prefix(TRAILER)?
            .strip_prefix(':')?
            .split_whitespace()
            .next()
    })
}

/// `message` with the scope's trailer, once. It joins a trailer block that ends the message, so
/// `Refs:` and `Agent-Scope:` stay together where git reads trailers.
pub fn with_trailer(message: &str, scope: &Scope) -> String {
    if trailer_scope(message).is_some() {
        return message.to_string();
    }
    let body = message.trim_end();
    let last = body.rsplit("\n\n").next().unwrap_or(body);
    let is_trailer = |l: &str| {
        l.split_once(": ").is_some_and(|(k, _)| {
            !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    };
    let joins = body.contains("\n\n") && !last.trim().is_empty() && last.lines().all(is_trailer);
    let sep = if joins { "\n" } else { "\n\n" };
    format!("{body}{sep}{}\n", trailer(scope))
}

/// The boundary table for the agent's brief (R-1), with the claim command.
pub fn brief(scope: &Scope, project: &Project) -> String {
    let _ = project;
    let item = &scope.item;
    let mut out = format!(
        "## Your boundaries — kanbanr agent scope {} ({}, {item}), enforced by kanbanr\n\n",
        scope.id,
        scope.role.name()
    );
    match (&scope.branch, scope.role) {
        (Some(branch), Role::Build) => out.push_str(&format!(
            "First, in your worktree: `kanbanr scope claim {}` (switches you to {branch})\n\n",
            scope.id
        )),
        _ => out.push_str(&format!(
            "First, in your worktree: `kanbanr scope claim {}`\n\n",
            scope.id
        )),
    }
    out.push_str("| Area | Allowed | Refused |\n|---|---|---|\n");
    out.push_str("| Read the board and the code | everything | — |\n");
    match scope.role {
        Role::Build => {
            let branch = scope.branch.as_deref().unwrap_or_default();
            out.push_str(&format!(
                "| {item}'s record | `kanbanr task state {item} <TL> <T> <state>` as each task \
                 starts and lands; `kanbanr test {item} R-n \"<name>\" green --rev <sha>` | moves, \
                 approvals, sign-offs, definitions, specs, docs, config, other items |\n"
            ));
            out.push_str(&format!(
                "| Code | commits on {branch}, each naming {item} (`kanbanr commit`) | commits on \
                 any other branch |\n"
            ));
        }
        Role::Design => {
            out.push_str(&format!("| {item}'s record | nothing | every write |\n"));
            out.push_str("| Code | nothing | every commit, every file edit |\n");
        }
    }
    out.push_str("| Remote | nothing | `git push`, `kanbanr sync` |\n");
    out.push_str("| Tests | temporary boards only | a real board |\n\n");
    if !scope.note.is_empty() {
        out.push_str(&format!("Note: {}\n\n", scope.note));
    }
    out.push_str(&format!(
        "Expires {}. A refusal names the boundary it crossed: report it to the coordinator, never \
         work around it.\n",
        readable(&scope.expires_at)
    ));
    out
}

// ---- drift ----------------------------------------------------------------------------------

/// A build scope whose branch has commits newer than its item's last task change (R-9).
#[derive(Debug, Clone, Serialize)]
pub struct Drift {
    pub scope: String,
    pub item: String,
    pub branch: String,
    /// The later of the item's last task change and the scope's issue time.
    pub since: String,
    pub commits: Vec<Commit>,
}

impl Drift {
    /// One line, for the Stop hook and `git doctor`.
    pub fn line(&self) -> String {
        let commits: Vec<String> = self
            .commits
            .iter()
            .map(|c| format!("{} {}", &c.sha[..c.sha.len().min(7)], c.subject))
            .collect();
        format!(
            "{}: {} commit(s) on {} since its tasks last changed at {} (agent scope {}): {}",
            self.item,
            self.commits.len(),
            self.branch,
            self.since,
            self.scope,
            commits.join("; ")
        )
    }
}

/// When the item's tasks last changed: the latest `changed_at`.
pub fn last_task_change(item: &FeatureItem) -> Option<String> {
    item.todo_lists
        .iter()
        .flat_map(|l| l.tasks.iter())
        .filter_map(|t| t.changed_at.clone())
        .max_by(|a, b| crate::cmp_instants(a, b))
}

/// Every non-revoked build scope whose branch is ahead of its item's tasks. Expired ones count:
/// their work may still be unrecorded.
pub fn drift(store: &Store, id: &str, root: &Path, now: &str) -> Result<Vec<Drift>> {
    let _ = now;
    let project = store.load_meta(id)?;
    let mut out = Vec::new();
    for s in load(store, id)? {
        if s.role != Role::Build || s.revoked_at.is_some() {
            continue;
        }
        let (Some(branch), Ok(item)) = (&s.branch, project.feature(&s.item)) else {
            continue;
        };
        if !coderepo::branch_exists(root, branch) {
            continue;
        }
        let since = match last_task_change(item) {
            Some(t) if crate::before(&s.issued_at, &t) => t,
            _ => s.issued_at.clone(),
        };
        let range = match &s.base {
            Some(base) => format!("{base}..{branch}"),
            None => branch.clone(),
        };
        let commits = coderepo::commits_since(root, &range, &since)?;
        if !commits.is_empty() {
            out.push(Drift {
                scope: s.id.clone(),
                item: s.item.clone(),
                branch: branch.clone(),
                since,
                commits,
            });
        }
    }
    Ok(out)
}

/// `git doctor`'s warnings: each drifting scope, and each active scope whose worktree is gone.
pub fn drift_issues(store: &Store, id: &str, root: &Path, now: &str) -> Vec<crate::doctor::Issue> {
    let warn = |code: &str, message: String| crate::doctor::Issue {
        severity: crate::doctor::Severity::Warning,
        project: id.to_string(),
        code: Some(code.to_string()),
        message,
    };
    let mut out: Vec<crate::doctor::Issue> = drift(store, id, root, now)
        .unwrap_or_default()
        .iter()
        .map(|d| warn(&d.item, d.line()))
        .collect();
    if let Some(main_root) = coderepo::worktree_facts(root).map(|w| w.main_root) {
        for s in load(store, id).unwrap_or_default() {
            if s.state(now) == ScopeState::Active
                && let Some(rel) = &s.worktree
                && !main_root.join(rel).is_dir()
            {
                out.push(warn(
                    &s.item,
                    format!(
                        "agent scope {} is active but its worktree {rel} no longer exists: revoke \
                         it (`kanbanr scope revoke {} --reason done`)",
                        s.id, s.id
                    ),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: &str = "2026-10-21T10:00:00Z";
    const LATER: &str = "2026-10-21T19:00:00Z";

    fn scope(role: Role) -> Scope {
        Scope {
            id: "SC-007".into(),
            project: "kanbanr".into(),
            role,
            item: "FEAT-214".into(),
            operations: role.operations(),
            branch: (role == Role::Build).then(|| "feat/FEAT-214-agent-scopes".to_string()),
            base: None,
            issued_by: "Venkatraman B".into(),
            issued_at: "2026-10-21T09:00:00Z".into(),
            expires_at: "2026-10-21T17:00:00Z".into(),
            note: String::new(),
            worktree: Some(".claude/worktrees/agent-x".into()),
            claimed_at: Some("2026-10-21T09:01:00Z".into()),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            extra: Default::default(),
        }
    }

    fn wt(rel: &str, branch: Option<&str>) -> WorktreeFacts {
        let main_root = PathBuf::from("/repo");
        WorktreeFacts {
            root: if rel.is_empty() {
                main_root.clone()
            } else {
                main_root.join(rel)
            },
            git_dir: PathBuf::from("/repo/.git/worktrees/x"),
            main_root,
            linked: !rel.is_empty(),
            branch: branch.map(String::from),
        }
    }

    fn pointer(id: &str) -> PointerFile {
        PointerFile {
            scope: id.into(),
            project: "kanbanr".into(),
        }
    }

    /// FEAT-214 R-8: issued, active, expired and revoked are derived from the record and the
    /// clock; revoked wins over expired.
    #[test]
    fn scope_state_derives_issued_active_expired_revoked() {
        let mut s = scope(Role::Build);
        assert_eq!(s.state(NOW), ScopeState::Active);
        assert_eq!(
            s.state("2026-10-21T17:00:00Z"),
            ScopeState::Expired,
            "at expiry"
        );
        assert_eq!(s.state(LATER), ScopeState::Expired);
        s.worktree = None;
        assert_eq!(s.state(NOW), ScopeState::Issued);
        s.revoked_at = Some(NOW.into());
        assert_eq!(s.state(NOW), ScopeState::Revoked);
        assert_eq!(
            s.state(LATER),
            ScopeState::Revoked,
            "revoked wins over expired"
        );
        assert!(ScopeState::Issued.is_live() && ScopeState::Active.is_live());
        assert!(!ScopeState::Expired.is_live() && !ScopeState::Revoked.is_live());
    }

    /// FEAT-214 R-3: writes map onto the scope vocabulary — task and test routes, every op of a
    /// batch, and any other route by its commit message.
    #[test]
    fn requested_maps_routes_and_batch_ops() {
        let task = requested(
            "PUT",
            "/projects/kanbanr/features/FEAT-214/todos/TL-001/tasks/T1",
            Some(&json!({"state": "Completed"})),
        );
        assert_eq!(task.len(), 1);
        assert_eq!(task[0].op, Some(Operation::TaskState));
        assert_eq!(task[0].item.as_deref(), Some("FEAT-214"));
        assert_eq!(task[0].project, "kanbanr");

        let test = requested(
            "PUT",
            "/projects/kanbanr/features/FEAT-9/tests/R-1/a%3A%3Ab",
            Some(&json!({"state": "green"})),
        );
        assert_eq!(test[0].op, Some(Operation::TestState));
        assert_eq!(test[0].item.as_deref(), Some("FEAT-9"));

        let moved = requested("POST", "/projects/kanbanr/features/FEAT-214/move", None);
        assert_eq!(moved[0].op, None);
        assert_eq!(moved[0].label, "move feature FEAT-214");
        assert_eq!(moved[0].item.as_deref(), Some("FEAT-214"));

        let batch = requested(
            "POST",
            "/projects/kanbanr/batch",
            Some(&json!({"operations": [
                {"op": "test.state", "feature": "FEAT-214", "requirement": "R-1", "test": "t", "state": "green"},
                {"op": "task.state", "feature": "FEAT-1", "todo": "TL-001", "key": "T1", "state": "Completed"},
                {"op": "feature.move", "code": "FEAT-214", "to": "Done"},
                {"op": "commit"},
            ]})),
        );
        assert_eq!(batch.len(), 4);
        assert_eq!(batch[0].op, Some(Operation::TestState));
        assert_eq!(batch[1].op, Some(Operation::TaskState));
        assert_eq!(batch[1].item.as_deref(), Some("FEAT-1"));
        assert_eq!(batch[2].op, None);
        assert_eq!(batch[2].label, "batch op feature.move");
        assert_eq!(batch[3].op, None, "a commit is never a board op");

        assert!(requested("GET", "/projects/kanbanr/features/FEAT-214", None).is_empty());
        let claim = requested("POST", "/projects/kanbanr/scopes/SC-007/claim", None);
        assert_eq!(claim[0].label, "scope.claim");
    }

    /// FEAT-214 R-3: a build scope allows its own item's task and test states, and nothing else —
    /// another item, another project, a move, an approval, a doc are each refused by name.
    #[test]
    fn build_scope_allows_only_own_item_task_and_test_states() {
        let s = scope(Role::Build);
        let ok = |m: &str, p: &str, b: Value| check(&s, &requested(m, p, Some(&b)), NOW);
        assert!(
            ok(
                "PUT",
                "/projects/kanbanr/features/FEAT-214/todos/TL-001/tasks/T1",
                json!({"state": "Completed"})
            )
            .is_ok()
        );
        assert!(
            ok(
                "PUT",
                "/projects/kanbanr/features/FEAT-214/tests/R-1/x",
                json!({"state": "green"})
            )
            .is_ok()
        );
        assert!(
            ok(
                "POST",
                "/projects/kanbanr/batch",
                json!({"operations": [{"op": "test.state", "feature": "FEAT-214", "requirement": "R-1", "test": "x", "state": "green"}]})
            )
            .is_ok(),
            "a batch of in-scope ops (D5)"
        );

        let other = ok(
            "PUT",
            "/projects/kanbanr/features/FEAT-1/todos/TL-001/tasks/T1",
            json!({"state": "Completed"}),
        )
        .unwrap_err();
        assert!(
            other
                .0
                .starts_with("refused by agent scope SC-007 (build, FEAT-214)"),
            "{other}"
        );
        assert!(
            other
                .0
                .contains("is on FEAT-1; this scope covers FEAT-214 only"),
            "{other}"
        );

        let project = ok(
            "PUT",
            "/projects/other/features/FEAT-214/todos/TL-001/tasks/T1",
            json!({"state": "Completed"}),
        )
        .unwrap_err();
        assert!(project.0.contains("is on project other"), "{project}");

        for (m, p, label) in [
            (
                "POST",
                "/projects/kanbanr/features/FEAT-214/move",
                "move feature FEAT-214",
            ),
            (
                "POST",
                "/projects/kanbanr/features/FEAT-214/approve",
                "approve definition of FEAT-214",
            ),
            ("PUT", "/projects/kanbanr/docs/content", "update document"),
            (
                "PATCH",
                "/projects/kanbanr/features/FEAT-214",
                "edit feature FEAT-214",
            ),
        ] {
            let e = ok(m, p, json!({})).unwrap_err();
            assert!(
                e.0.contains(&format!("`{label}` is not one of its operations")),
                "{e}"
            );
            assert!(
                e.0.contains("Under this scope you may: `kanbanr task state FEAT-214 …`"),
                "{e}"
            );
            assert!(e.0.ends_with("do not work around it."), "{e}");
        }
        let smuggled = ok(
            "POST",
            "/projects/kanbanr/batch",
            json!({"operations": [
                {"op": "task.state", "feature": "FEAT-214", "todo": "TL-001", "key": "T1", "state": "Completed"},
                {"op": "feature.move", "code": "FEAT-214", "to": "Done"},
            ]}),
        )
        .unwrap_err();
        assert!(smuggled.0.contains("`batch op feature.move`"), "{smuggled}");
        assert!(s.allows(Operation::TaskState, "FEAT-214"));
        assert!(!s.allows(Operation::TaskState, "FEAT-1"));
    }

    /// FEAT-214 R-3: a design scope is read-only: every write is refused; reads are not.
    #[test]
    fn design_scope_refuses_every_write() {
        let s = scope(Role::Design);
        assert!(s.operations.is_empty());
        for (m, p) in [
            (
                "PUT",
                "/projects/kanbanr/features/FEAT-214/todos/TL-001/tasks/T1",
            ),
            ("PUT", "/projects/kanbanr/features/FEAT-214/tests/R-1/x"),
            ("POST", "/projects/kanbanr/features/FEAT-214/move"),
            ("PUT", "/projects/kanbanr/features/FEAT-214/definition"),
        ] {
            let e = check(&s, &requested(m, p, Some(&json!({}))), NOW).unwrap_err();
            assert!(
                e.0.starts_with("refused by agent scope SC-007 (design, FEAT-214)"),
                "{e}"
            );
            assert!(e.0.contains("nothing but reading"), "{e}");
        }
        assert!(check(&s, &requested("GET", "/projects/kanbanr", None), NOW).is_ok());
        let v = commit_verdict(&ProcessScope::Scoped(s), &RepoFacts::default(), NOW);
        assert!(
            v.refusal
                .unwrap()
                .contains("a design agent does not commit or edit code")
        );
    }

    /// FEAT-214 R-1: the brief names the scope, the item, its branch, the claim command, what is
    /// allowed and refused, and the expiry.
    #[test]
    fn brief_names_item_branch_and_claim() {
        let project = Project {
            id: "kanbanr".into(),
            config: crate::ProjectConfig::for_new_project("kanbanr"),
            features: Vec::new(),
            milestones: Vec::new(),
        };
        let b = brief(&scope(Role::Build), &project);
        for want in [
            "## Your boundaries — kanbanr agent scope SC-007 (build, FEAT-214), enforced by kanbanr",
            "`kanbanr scope claim SC-007` (switches you to feat/FEAT-214-agent-scopes)",
            "| FEAT-214's record | `kanbanr task state FEAT-214 <TL> <T> <state>`",
            "`kanbanr test FEAT-214 R-n \"<name>\" green --rev <sha>`",
            "moves, approvals, sign-offs, definitions, specs, docs, config, other items",
            "| Code | commits on feat/FEAT-214-agent-scopes, each naming FEAT-214 (`kanbanr commit`)",
            "| Remote | nothing | `git push`, `kanbanr sync` |",
            "| Tests | temporary boards only | a real board |",
            "Expires 2026-10-21 17:00 UTC.",
            "never work around it.",
        ] {
            assert!(b.contains(want), "the brief says {want:?}:\n{b}");
        }
        let d = brief(&scope(Role::Design), &project);
        assert!(d.contains("(design, FEAT-214)"), "{d}");
        assert!(
            d.contains("| FEAT-214's record | nothing | every write |"),
            "{d}"
        );
        assert!(
            d.contains("| Code | nothing | every commit, every file edit |"),
            "{d}"
        );
        assert!(!d.contains("switches you"), "{d}");
    }

    /// FEAT-214 R-4: every `resolve_with` rule, and that it fails closed — a missing, foreign,
    /// mismatched, expired or revoked pointer in an agent worktree is refused, never unscoped.
    #[test]
    fn resolve_fails_closed() {
        let s = scope(Role::Build);
        let scopes = vec![s.clone()];
        let here = wt(
            ".claude/worktrees/agent-x",
            Some("feat/FEAT-214-agent-scopes"),
        );
        let refused = |p: &ProcessScope| matches!(p, ProcessScope::Refused { .. });

        // 1. No scopes on the board: unscoped, even in an agent worktree with a pointer.
        assert_eq!(
            resolve_with(&[], Some(&here), Some(&pointer("SC-007")), NOW),
            ProcessScope::Unscoped
        );
        // 2. No work tree.
        assert_eq!(
            resolve_with(&scopes, None, None, NOW),
            ProcessScope::Unscoped
        );
        // 3. A pointer naming this worktree's scope, in force.
        assert_eq!(
            resolve_with(&scopes, Some(&here), Some(&pointer("SC-007")), NOW),
            ProcessScope::Scoped(s.clone())
        );
        // 3. A pointer naming a scope bound elsewhere (an altered pointer).
        let elsewhere = wt(".claude/worktrees/agent-y", None);
        let r = resolve_with(&scopes, Some(&elsewhere), Some(&pointer("SC-007")), NOW);
        assert!(refused(&r), "{r:?}");
        if let ProcessScope::Refused { why, may_claim } = &r {
            assert!(why.contains("bound to .claude/worktrees/agent-x"), "{why}");
            assert!(!may_claim);
        }
        // 3. Project mismatch.
        let foreign = PointerFile {
            scope: "SC-007".into(),
            project: "other".into(),
        };
        assert!(refused(&resolve_with(
            &scopes,
            Some(&here),
            Some(&foreign),
            NOW
        )));
        // 3. Not yet claimed: refused, and it may claim.
        let mut unclaimed = s.clone();
        unclaimed.worktree = None;
        let r = resolve_with(&[unclaimed], Some(&here), Some(&pointer("SC-007")), NOW);
        assert!(
            matches!(
                r,
                ProcessScope::Refused {
                    may_claim: true,
                    ..
                }
            ),
            "{r:?}"
        );
        // 3. Expired and revoked.
        let r = resolve_with(&scopes, Some(&here), Some(&pointer("SC-007")), LATER);
        assert!(
            matches!(&r, ProcessScope::Refused { why, .. } if why.contains("expired at 2026-10-21 17:00 UTC")),
            "{r:?}"
        );
        let mut revoked = s.clone();
        revoked.revoked_at = Some(NOW.into());
        revoked.revoked_by = Some("V".into());
        revoked.revoked_reason = Some("done".into());
        let r = resolve_with(&[revoked], Some(&here), Some(&pointer("SC-007")), NOW);
        assert!(
            matches!(&r, ProcessScope::Refused { why, .. } if why.contains("was revoked by V") && why.contains("(done)")),
            "{r:?}"
        );
        // 4. A deleted pointer: the worktree is recorded, so refused (M3), and it may re-claim.
        let r = resolve_with(&scopes, Some(&here), None, NOW);
        assert!(
            matches!(&r, ProcessScope::Refused { why, may_claim: true } if why.contains("no agent scope is bound to it")),
            "{r:?}"
        );
        // 4. A pointer naming a scope not on this board, under .claude/worktrees: refused.
        let fresh = wt(".claude/worktrees/agent-z", Some("main"));
        assert!(refused(&resolve_with(
            &scopes,
            Some(&fresh),
            Some(&pointer("SC-999")),
            NOW
        )));
        assert!(refused(&resolve_with(&scopes, Some(&fresh), None, NOW)));
        // 4. A linked worktree elsewhere on the scope's branch.
        let other = WorktreeFacts {
            root: PathBuf::from("/elsewhere/wt"),
            ..wt("x", Some("feat/FEAT-214-agent-scopes"))
        };
        assert!(refused(&resolve_with(&scopes, Some(&other), None, NOW)));
        // 5. The person's own linked worktree elsewhere, and the main checkout: unscoped.
        let mine = WorktreeFacts {
            root: PathBuf::from("/elsewhere/wt"),
            ..wt("x", Some("feat/FEAT-1-other"))
        };
        assert_eq!(
            resolve_with(&scopes, Some(&mine), None, NOW),
            ProcessScope::Unscoped
        );
        assert_eq!(
            resolve_with(&scopes, Some(&wt("", Some("main"))), None, NOW),
            ProcessScope::Unscoped
        );
    }

    /// FEAT-214 R-5: a commit under a build scope lands only on its branch, names its item, and
    /// only while the scope is in force; a foreign trailer is refused.
    #[test]
    fn commit_verdict_scoped() {
        let s = scope(Role::Build);
        let p = ProcessScope::Scoped(s.clone());
        let facts = |branch: &str, msg: Option<&str>| RepoFacts {
            branch: Some(branch.to_string()),
            default_branch: "main".into(),
            merging: false,
            has_commits: true,
            message: msg.map(String::from),
        };
        let on = "feat/FEAT-214-agent-scopes";
        assert!(commit_verdict(&p, &facts(on, None), NOW).is_allowed());
        assert!(
            commit_verdict(&p, &facts(on, Some("x\n\nRefs: kanbanr:FEAT-214/R-5")), NOW)
                .is_allowed()
        );
        let off = commit_verdict(&p, &facts("main", None), NOW)
            .refusal
            .unwrap();
        assert!(
            off.contains(
                "this commit is on main; this agent commits only on feat/FEAT-214-agent-scopes"
            ),
            "{off}"
        );
        let unnamed = commit_verdict(&p, &facts(on, Some("x\n\nRefs: kanbanr:FEAT-1")), NOW)
            .refusal
            .unwrap();
        assert!(unnamed.contains("must name FEAT-214"), "{unnamed}");
        let escaped = commit_verdict(&p, &facts(on, Some("[no-ref] chore")), NOW);
        assert!(!escaped.is_allowed(), "[no-ref] is refused under a scope");
        let foreign = commit_verdict(
            &p,
            &facts(
                on,
                Some(
                    "x\n\nRefs: kanbanr:FEAT-214\nAgent-Scope: SC-001 (build, by the agent for FEAT-214)",
                ),
            ),
            NOW,
        );
        assert!(foreign.refusal.unwrap().contains("another scope"));
        assert!(
            commit_verdict(&p, &facts(on, None), LATER)
                .refusal
                .unwrap()
                .contains("expired")
        );
        let mut revoked = s.clone();
        revoked.revoked_at = Some(NOW.into());
        assert!(
            commit_verdict(&ProcessScope::Scoped(revoked), &facts(on, None), NOW)
                .refusal
                .unwrap()
                .contains("revoked")
        );
        let refused = ProcessScope::Refused {
            why: "M3".into(),
            may_claim: true,
        };
        assert_eq!(
            commit_verdict(&refused, &facts(on, None), NOW)
                .refusal
                .as_deref(),
            Some("M3")
        );
        assert!(commit_verdict(&ProcessScope::Unscoped, &facts("main", None), NOW).is_allowed());
        assert!(
            push_verdict(&p)
                .refusal
                .unwrap()
                .contains("agents never push")
        );
        assert!(push_verdict(&ProcessScope::Unscoped).is_allowed());
    }

    /// The trailer is added once and joins a trailer block.
    #[test]
    fn trailer_joins_the_trailer_block_once() {
        let s = scope(Role::Build);
        let t = trailer(&s);
        assert_eq!(t, "Agent-Scope: SC-007 (build, by the agent for FEAT-214)");
        assert_eq!(
            with_trailer("update task T1", &s),
            format!("update task T1\n\n{t}\n")
        );
        let code = with_trailer("work\n\nRefs: kanbanr:FEAT-214\n", &s);
        assert_eq!(code, format!("work\n\nRefs: kanbanr:FEAT-214\n{t}\n"));
        assert_eq!(with_trailer(&code, &s), code, "idempotent");
        assert_eq!(trailer_scope(&code), Some("SC-007"));
        let (kept, dropped) = retain_permitted(
            &s,
            vec![
                json!({"op": "test.state", "feature": "FEAT-214"}),
                json!({"op": "test.state", "feature": "FEAT-1"}),
            ],
        );
        assert_eq!((kept.len(), dropped), (1, 1));
        assert_eq!(
            s.by_agent().to_string(),
            "by the agent for FEAT-214 (SC-007)"
        );
    }
}
