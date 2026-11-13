//! Git-backed data folder. The data directory is a git repository; every write commits authored
//! as the acting user (`name <email>`), and (if any remotes are configured) the commit is pushed
//! to them after integrating their changes. Shared by the server and the CLI's local mode.
//!
//! Implemented with `git2` (libgit2, vendored + statically linked) so nothing needs an external
//! `git` binary. All operations are best-effort and never panic on a normal git failure.

use git2::{
    AnnotatedCommit, AutotagOption, Cred, CredentialType, FetchOptions, IndexAddOption,
    PushOptions, RemoteCallbacks, Repository, Signature,
};
use std::path::Path;

/// Stage every change in the working tree (additions, edits, and deletions) into the index and
/// write it. Equivalent to `git add -A`.
fn stage_all(repo: &Repository) -> Result<git2::Oid, git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.write()?;
    index.write_tree()
}

/// The placeholder email older versions wrote into every new data repository (FEAT-128). It is
/// nobody's identity, so wherever it is found it counts as none.
pub const PLACEHOLDER_EMAIL: &str = "kanbanr@local";

/// Why a commit was refused: nobody to author it (FEAT-128).
pub const NO_IDENTITY: &str = "no commit identity for this board, so nothing was committed. Set \
     yours with `kanbanr identity --name \"Your Name\" --email you@example.com` (or set git's \
     user.name and user.email)";

/// `(name, email)` from one git config, unless it is missing, blank or the placeholder.
fn identity_in(cfg: &git2::Config) -> Option<(String, String)> {
    let name = cfg.get_string("user.name").ok()?;
    let email = cfg.get_string("user.email").ok()?;
    (!name.trim().is_empty() && !email.trim().is_empty() && email.trim() != PLACEHOLDER_EMAIL)
        .then_some((name, email))
}

/// Who commits: the repository's own identity, else the user's git identity. The second is asked
/// separately because a placeholder in the repository's config would otherwise hide it.
fn resolve_identity(
    repo: Option<&git2::Config>,
    user: Option<&git2::Config>,
) -> Option<(String, String)> {
    repo.and_then(identity_in)
        .or_else(|| user.and_then(identity_in))
}

/// The signature for a commit in `repo` — never a placeholder (FEAT-128). With no identity to be
/// found, the error names the command that sets one.
fn signature(repo: &Repository) -> Result<Signature<'static>, git2::Error> {
    let user = git2::Config::open_default().ok();
    let (name, email) = resolve_identity(repo.config().ok().as_ref(), user.as_ref())
        .ok_or_else(|| git2::Error::from_str(NO_IDENTITY))?;
    Signature::now(&name, &email)
}

/// Secrets that must NEVER be committed (the data repo is pushed to remotes). `security.yaml`
/// holds every user's auth_key/auth_secret and the JWT signing secret.
const SECRET_FILE: &str = "security.yaml";

/// The CLI's cross-process write lock file (FEAT-029). It lives at the data-dir root but is
/// machine-local state, so it must never be tracked or pushed.
pub const WRITE_LOCK_FILE: &str = ".kanbanr.lock";

/// The debounced-push state file (FEAT-034). It records that there are local commits not yet
/// pushed to any remote, so a later `kanbanr sync` (or a debounce tick) knows there is work to do.
/// Like the lock file it is machine-local and must never be tracked or pushed.
pub const UNPUSHED_FILE: &str = ".kanbanr.unpushed";

/// Where session summaries live, beside the board and never in its history (FEAT-135). A summary is
/// a condensed conversation — local paths, other projects, half-formed ideas — and a board is what
/// a team shares through a remote, so the folder is ignored like the secrets file and the lock.
pub const SESSIONS_DIR: &str = ".sessions/";

/// Mark the data dir as having local commits not yet pushed to remotes (FEAT-034). Cheap and
/// best-effort; called after a local commit when push is debounced rather than immediate.
pub fn mark_unpushed(dir: &Path) {
    let _ = std::fs::write(dir.join(UNPUSHED_FILE), b"");
}

/// Clear the "unpushed" marker after a successful (or attempted) push (FEAT-034).
pub fn clear_unpushed(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(UNPUSHED_FILE));
}

/// Whether there are local commits awaiting a push (the debounce marker exists). (FEAT-034)
#[must_use]
pub fn has_unpushed(dir: &Path) -> bool {
    dir.join(UNPUSHED_FILE).exists()
}

/// Ensure the data repo ignores machine-local files (the secrets file, the write lock, and the
/// debounced-push marker), and stop tracking the secrets file if a pre-existing repo happened to
/// track it (the file stays on disk; it just leaves the index/history going forward).
fn ensure_secret_ignored(repo: &Repository, dir: &Path) {
    let gitignore = dir.join(".gitignore");
    let mut contents = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let mut changed = false;
    // `events.config.yaml` (FEAT-036) may hold webhook endpoint URLs — keep it machine-local and
    // out of the pushed repo, like the secrets file and the lock/unpushed markers.
    for entry in [
        SECRET_FILE,
        WRITE_LOCK_FILE,
        UNPUSHED_FILE,
        crate::eventing::CONFIG_FILE,
        SESSIONS_DIR,
        // Leftovers of an interrupted atomic write (DEF-025) are never board data.
        "*.tmp",
    ] {
        if !contents.lines().any(|l| l.trim() == entry) {
            if !contents.is_empty() && !contents.ends_with('\n') {
                contents.push('\n');
            }
            contents.push_str(entry);
            contents.push('\n');
            changed = true;
        }
    }
    if changed {
        let _ = crate::atomic::write(&gitignore, contents);
    }
    if let Ok(mut index) = repo.index()
        && index.get_path(Path::new(SECRET_FILE), 0).is_some()
    {
        let _ = index.remove_path(Path::new(SECRET_FILE));
        let _ = index.write();
    }
}

/// Initialize the data dir as a git repo (and make an initial commit) if it isn't one yet.
pub fn ensure_repo(dir: &Path) {
    ensure_repo_as(dir, None);
}

/// [`ensure_repo`], recording `identity` first when one is given, so that the first commit is
/// authored by it (FEAT-128). With no identity anywhere, the repository is created without a
/// commit; the first write that has an author makes it.
pub fn ensure_repo_as(dir: &Path, identity: Option<(&str, &str)>) {
    crate::project::assert_inside_fence(dir); // FEAT-195
    let fresh = !dir.join(".git").exists();
    let repo = if fresh {
        match Repository::init(dir) {
            Ok(r) => r,
            Err(_) => return,
        }
    } else {
        match Repository::open(dir) {
            Ok(r) => r,
            Err(_) => return,
        }
    };
    // No placeholder identity is written here any more (FEAT-128): it authored every new board's
    // first commit, and, being repository-local, hid the user's own git identity from every
    // commit after it.
    if let Some((name, email)) = identity {
        let _ = set_identity(dir, name, email);
    }
    // Always: keep credentials out of the repo (write .gitignore before the first commit so the
    // secret file is never tracked in the first place).
    ensure_secret_ignored(&repo, dir);
    if fresh {
        let _ = (|| -> Result<(), git2::Error> {
            let tree_oid = stage_all(&repo)?;
            let tree = repo.find_tree(tree_oid)?;
            let sig = signature(&repo)?;
            repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initialize data repository",
                &tree,
                &[],
            )?;
            Ok(())
        })();
    }
}

/// M11 for a change to the board `dir` made from an agent's worktree (FEAT-214), where the board is
/// one scopes govern (never a temporary one, D7).
fn agent_refusal(dir: &Path, what: &str) -> Option<String> {
    if !crate::agentscope::is_governed_board(dir) {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;
    crate::agentscope::outside_core_refusal(&cwd, what, crate::agentscope::Caller::of_process())
}

/// The scope id (or `none`) of an agent context this process runs in, when it would push a governed
/// board (FEAT-214 R-6).
fn agent_pusher(dir: &Path) -> Option<String> {
    if !crate::agentscope::Caller::of_process().is_governed()
        || !crate::agentscope::is_governed_board(dir)
    {
        return None;
    }
    let wt = crate::agentscope::agent_context(&std::env::current_dir().ok()?)?;
    Some(
        crate::agentscope::read_pointer(&wt.git_dir)
            .map(|p| p.scope)
            .unwrap_or_else(|| "none".to_string()),
    )
}

/// The board's files changed outside a kanbanr write (FEAT-214): every path git reports as
/// modified, added, deleted or untracked, ignored files aside. `Ok` when there are none.
pub fn is_clean(dir: &Path) -> Result<(), Vec<String>> {
    let Ok(repo) = Repository::open(dir) else {
        return Ok(());
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return Ok(());
    };
    let paths: Vec<String> = statuses
        .iter()
        .filter(|e| e.status() != git2::Status::CURRENT)
        .filter_map(|e| e.path().ok().map(String::from))
        .collect();
    if paths.is_empty() { Ok(()) } else { Err(paths) }
}

/// Set the repo-local commit identity (used by local mode: `kanbanr identity --name --email`).
pub fn set_identity(dir: &Path, name: &str, email: &str) -> Result<(), String> {
    crate::project::assert_inside_fence(dir); // FEAT-195
    // Who the board's commits are by is never an agent's to change (FEAT-214).
    if let Some(refusal) = agent_refusal(dir, "`kanbanr identity` changes kanbanr itself") {
        return Err(refusal);
    }
    let repo = Repository::open(dir).map_err(|e| e.message().to_string())?;
    let mut cfg = repo.config().map_err(|e| e.message().to_string())?;
    cfg.set_str("user.name", name)
        .map_err(|e| e.message().to_string())?;
    cfg.set_str("user.email", email)
        .map_err(|e| e.message().to_string())?;
    Ok(())
}

/// The identity commits here are authored by: the repository's own, else the user's git identity.
/// A placeholder left by an older version counts as none (FEAT-128).
pub fn identity(dir: &Path) -> Option<(String, String)> {
    let repo_cfg = Repository::open(dir).ok().and_then(|r| r.config().ok());
    let user = git2::Config::open_default().ok();
    resolve_identity(repo_cfg.as_ref(), user.as_ref())
}

/// [`identity`], or the reason nothing can be committed (FEAT-128). Writers ask this *before*
/// changing anything, so a refused commit never leaves a change on disk that no commit records.
pub fn require_identity(dir: &Path) -> Result<(String, String), String> {
    identity(dir).ok_or_else(|| NO_IDENTITY.to_string())
}

/// Who a verdict (an approval, a sign-off, a waiver) is recorded as given by (FEAT-176): the name
/// given, trimmed, else the board's commit identity. A verdict that cannot say who gave it is
/// refused, with how to fix that.
pub fn verdict_author(dir: &Path, given: Option<&str>) -> crate::Result<String> {
    if let Some(name) = given.filter(|n| !n.trim().is_empty()) {
        return Ok(name.trim().to_string());
    }
    identity(dir)
        .map(|(name, _)| name)
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| {
            crate::CoreError::Refused(
                "this board has no commit identity, so a verdict could not say who gave it.\n\
                 Set one once:  kanbanr identity --name \"You\" --email you@example.com\n\
                 or pass it explicitly with --by."
                    .into(),
            )
        })
}

/// Does this repository's own config still carry the placeholder identity? (FEAT-128)
pub fn has_placeholder_identity(dir: &Path) -> bool {
    Repository::open(dir)
        .ok()
        .and_then(|r| r.config().ok())
        .and_then(|c| c.open_level(git2::ConfigLevel::Local).ok())
        .and_then(|c| c.get_string("user.email").ok())
        .is_some_and(|e| e.trim() == PLACEHOLDER_EMAIL)
}

fn do_commit(repo: &Repository, sig: &Signature, message: &str) -> Result<bool, git2::Error> {
    let tree_oid = stage_all(repo)?;
    let tree = repo.find_tree(tree_oid)?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    // Nothing to commit if the staged tree equals the current HEAD tree.
    if let Some(p) = &parent
        && p.tree_id() == tree_oid
    {
        return Ok(false);
    }
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), sig, sig, message, &tree, &parents)?;
    Ok(true)
}

/// Stage everything and commit authored as `name <email>` with `message`. Returns true if a commit
/// was made (false if nothing changed or on error).
pub fn commit(dir: &Path, name: &str, email: &str, message: &str) -> bool {
    crate::project::assert_inside_fence(dir); // FEAT-195
    let Ok(repo) = Repository::open(dir) else {
        return false;
    };
    let Ok(sig) = Signature::now(name, email) else {
        return false;
    };
    do_commit(&repo, &sig, message).unwrap_or(false)
}

/// Stage everything and commit using the repo's configured identity (local mode). Returns true if
/// a commit was made.
pub fn commit_local(dir: &Path, message: &str) -> bool {
    crate::project::assert_inside_fence(dir); // FEAT-195
    let Ok(repo) = Repository::open(dir) else {
        return false;
    };
    let Ok(sig) = signature(&repo) else {
        return false;
    };
    do_commit(&repo, &sig, message).unwrap_or(false)
}

/// Credentials for a remote (FEAT-142): SSH through the user's agent, then their default key
/// files; HTTPS through git's credential helper. libgit2 calls this again after each refusal, so it
/// steps through the options once and then gives up, rather than offering the same key forever.
fn credentials_cb(
    git_config: git2::Config,
) -> impl FnMut(&str, Option<&str>, CredentialType) -> Result<Cred, git2::Error> {
    let mut ssh_attempt = 0usize;
    let mut https_tried = false;
    move |url, username_from_url, allowed| {
        let user = username_from_url.unwrap_or("git");
        if allowed.contains(CredentialType::USERNAME) {
            return Cred::username(user);
        }
        if allowed.contains(CredentialType::SSH_KEY) {
            ssh_attempt += 1;
            if ssh_attempt == 1 {
                return Cred::ssh_key_from_agent(user);
            }
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(std::path::PathBuf::from);
            let keys: Vec<std::path::PathBuf> = home
                .map(|h| {
                    ["id_ed25519", "id_ecdsa", "id_rsa"]
                        .iter()
                        .map(|k| h.join(".ssh").join(k))
                        .filter(|k| k.exists())
                        .collect()
                })
                .unwrap_or_default();
            return match keys.get(ssh_attempt - 2) {
                Some(key) => Cred::ssh_key(user, None, key, None),
                None => Err(git2::Error::from_str(
                    "no SSH key was accepted: load one into ssh-agent (ssh-add), or check the \
                     remote's access",
                )),
            };
        }
        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) && !https_tried {
            https_tried = true;
            return Cred::credential_helper(&git_config, url, username_from_url);
        }
        if allowed.contains(CredentialType::DEFAULT) {
            return Cred::default();
        }
        Err(git2::Error::from_str(
            "no credentials for this remote: configure a git credential helper, or use an SSH URL",
        ))
    }
}

fn current_branch(repo: &Repository) -> String {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().ok().map(String::from))
        .unwrap_or_else(|| "master".to_string())
}

/// Fetch `branch` from `remote` and integrate it (fast-forward, or a merge commit). Assumes no
/// conflicts; on conflict the merge state is cleaned up and the local branch is left untouched.
fn pull(repo: &Repository, remote_name: &str, branch: &str) -> Result<(), git2::Error> {
    let mut remote = repo.find_remote(remote_name)?;

    let mut cb = RemoteCallbacks::new();
    cb.credentials(credentials_cb(repo.config()?));
    let mut fo = FetchOptions::new();
    fo.remote_callbacks(cb).download_tags(AutotagOption::All);
    remote.fetch(&[branch], Some(&mut fo), None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let their: AnnotatedCommit = repo.reference_to_annotated_commit(&fetch_head)?;
    let (analysis, _) = repo.merge_analysis(&[&their])?;

    if analysis.is_up_to_date() {
        return Ok(());
    }
    let refname = format!("refs/heads/{branch}");
    if analysis.is_fast_forward() {
        match repo.find_reference(&refname) {
            Ok(mut r) => {
                r.set_target(their.id(), "fast-forward")?;
            }
            Err(_) => {
                repo.reference(&refname, their.id(), true, "fast-forward")?;
            }
        }
        repo.set_head(&refname)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        return Ok(());
    }

    // Normal merge.
    repo.merge(&[&their], None, None)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        repo.checkout_index(
            Some(&mut index),
            Some(git2::build::CheckoutBuilder::default().force()),
        )?;
        repo.cleanup_state()?;
        return Ok(());
    }
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let head = repo.head()?.peel_to_commit()?;
    let theirs = repo.find_commit(their.id())?;
    let sig = signature(repo)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "merge remote changes",
        &tree,
        &[&head, &theirs],
    )?;
    repo.cleanup_state()?;
    Ok(())
}

/// Push the current branch to `remote`.
fn push(repo: &Repository, remote_name: &str, branch: &str) -> Result<(), git2::Error> {
    let mut remote = repo.find_remote(remote_name)?;
    let mut cb = RemoteCallbacks::new();
    cb.credentials(credentials_cb(repo.config()?));
    let mut opts = PushOptions::new();
    opts.remote_callbacks(cb);
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    remote.push(&[refspec.as_str()], Some(&mut opts))
}

/// What a sync did: the remotes it pushed to, and a message per remote it could not push to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub pushed: Vec<String>,
    pub failures: Vec<String>,
}

/// Sync with every configured remote: pull (integrate, assuming no conflicts) then push the
/// current branch. Best-effort and **safe**: the change is already committed locally before this
/// runs, so nothing is ever lost. Each remote that could not be pushed (a divergence, a conflict,
/// a credential or network failure) gets a message with the exact git commands to finish by hand —
/// kanbanr never auto-resolves. (A failed *pull* on the very first sync is benign: the push then
/// creates the branch; only a failed push is surfaced.)
#[must_use]
pub fn sync_all(dir: &Path) -> SyncOutcome {
    crate::project::assert_inside_fence(dir); // FEAT-195
    let mut outcome = SyncOutcome::default();
    // An agent never pushes the board (FEAT-214): the coordinator reviews and pushes.
    if let Some(id) = agent_pusher(dir) {
        outcome.failures.push(crate::agentscope::push_refusal(&id));
        return outcome;
    }
    let Ok(repo) = Repository::open(dir) else {
        return outcome;
    };
    let branch = current_branch(&repo);
    let Ok(remotes) = repo.remotes() else {
        return outcome;
    };
    let shown = dir.display();
    // git2 0.21 reports a name that is not UTF-8 as an error, not a missing value; neither is a
    // remote we could name to the user, so both are skipped.
    for name in remotes.iter().filter_map(|n| n.ok().flatten()) {
        let _ = pull(&repo, name, &branch); // best-effort integrate (first-push fetch errors are benign)
        if let Err(e) = push(&repo, name, &branch) {
            outcome.failures.push(format!(
                "remote '{name}': could not push ({}). Your change is committed locally, so nothing \
                 is lost. To sync, resolve in the data folder with normal git:\n    \
                 git -C {shown} pull --no-rebase {name} {branch}\n    \
                 git -C {shown} push {name} {branch}",
                e.message()
            ));
        } else {
            outcome.pushed.push(name.to_string());
        }
    }
    outcome
}

/// How many commits the board is ahead of its remotes: the most it is ahead of any one, counted
/// from git rather than remembered (FEAT-142) — each `kanbanr` command is its own process, and a
/// count kept in memory started from zero every time, so a batched push never fired. A remote it
/// has never been pushed to counts every commit. `None` when there is no remote.
pub fn ahead_of_remotes(dir: &Path) -> Option<usize> {
    let repo = Repository::open(dir).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?.id();
    let branch = current_branch(&repo);
    let names = repo.remotes().ok()?;
    let names: Vec<String> = names
        .iter()
        .filter_map(|n| n.ok().flatten())
        .map(String::from)
        .collect();
    if names.is_empty() {
        return None;
    }
    names
        .iter()
        .map(|name| {
            let tracking = format!("refs/remotes/{name}/{branch}");
            match repo.find_reference(&tracking).ok().and_then(|r| r.target()) {
                Some(theirs) => repo
                    .graph_ahead_behind(head, theirs)
                    .map(|(a, _)| a)
                    .unwrap_or(0),
                None => {
                    let mut walk = match repo.revwalk() {
                        Ok(w) => w,
                        Err(_) => return 0,
                    };
                    if walk.push(head).is_err() {
                        return 0;
                    }
                    walk.count()
                }
            }
        })
        .max()
}

/// The git-config key holding a board's push policy (FEAT-142). In the board's own repository
/// config: machine-local and never pushed, beside its commit identity — how often a machine pushes
/// is that machine's choice, not something a shared file should decide for every contributor.
pub const PUSH_SETTING: &str = "kanbanr.push";

/// When a board's commits reach its remotes (FEAT-034, FEAT-142).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PushPolicy {
    /// Push after every commit.
    Auto,
    /// Push once the board is `every` commits ahead of a remote. The default.
    Debounce { every: u32 },
    /// Never push on its own; `kanbanr sync` does.
    Off,
}

/// Where a board's push policy came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolicySource {
    /// `KANBANR_PUSH`, a one-off override.
    Environment,
    /// The board's `kanbanr.push` setting.
    Board,
    /// Neither: the default.
    Default,
}

const DEBOUNCE_DEFAULT_EVERY: u32 = 10;

impl PushPolicy {
    /// `auto`, `off`, `debounce` or `debounce:N`; anything else is no policy.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "auto" => Some(PushPolicy::Auto),
            "off" => Some(PushPolicy::Off),
            "debounce" => Some(PushPolicy::Debounce {
                every: DEBOUNCE_DEFAULT_EVERY,
            }),
            s => {
                let n: u32 = s.strip_prefix("debounce:")?.parse().ok()?;
                (n > 0).then_some(PushPolicy::Debounce { every: n })
            }
        }
    }

    /// The environment overrides the board's setting, which overrides the default.
    pub fn resolve(env: Option<&str>, board: Option<&str>) -> (Self, PolicySource) {
        if let Some(p) = env.and_then(Self::parse) {
            return (p, PolicySource::Environment);
        }
        if let Some(p) = board.and_then(Self::parse) {
            return (p, PolicySource::Board);
        }
        (
            PushPolicy::Debounce {
                every: DEBOUNCE_DEFAULT_EVERY,
            },
            PolicySource::Default,
        )
    }

    /// This board's policy, from `KANBANR_PUSH` and its own setting.
    pub fn for_board(dir: &Path) -> (Self, PolicySource) {
        let env = std::env::var("KANBANR_PUSH").ok();
        Self::resolve(env.as_deref(), push_setting(dir).as_deref())
    }
}

impl std::fmt::Display for PushPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PushPolicy::Auto => write!(f, "auto"),
            PushPolicy::Off => write!(f, "off"),
            PushPolicy::Debounce { every } => write!(f, "debounce:{every}"),
        }
    }
}

/// The board's own push setting, if it has one.
pub fn push_setting(dir: &Path) -> Option<String> {
    let repo = Repository::open(dir).ok()?;
    let cfg = repo
        .config()
        .ok()?
        .open_level(git2::ConfigLevel::Local)
        .ok()?;
    cfg.get_string(PUSH_SETTING).ok()
}

/// Record the board's push policy in its own git config.
pub fn set_push_setting(dir: &Path, policy: PushPolicy) -> Result<(), String> {
    crate::project::assert_inside_fence(dir); // FEAT-195
    let repo = Repository::open(dir).map_err(|e| e.message().to_string())?;
    let mut cfg = repo.config().map_err(|e| e.message().to_string())?;
    cfg.set_str(PUSH_SETTING, &policy.to_string())
        .map_err(|e| e.message().to_string())
}

/// Push every remote now. The board stays marked unpushed unless every push succeeded — a failed
/// push used to clear the mark, and `sync` then reported there was nothing to send.
pub fn push_pending(dir: &Path) -> SyncOutcome {
    let outcome = sync_all(dir);
    if outcome.failures.is_empty() {
        clear_unpushed(dir);
    } else {
        mark_unpushed(dir);
    }
    outcome
}

/// After a commit, apply the board's policy. `Some` when a push ran.
pub fn after_commit(dir: &Path, policy: PushPolicy) -> Option<SyncOutcome> {
    match policy {
        PushPolicy::Auto => Some(push_pending(dir)),
        PushPolicy::Off => {
            mark_unpushed(dir);
            None
        }
        PushPolicy::Debounce { every } => {
            mark_unpushed(dir);
            let due = ahead_of_remotes(dir).is_some_and(|n| n >= every as usize);
            due.then(|| push_pending(dir))
        }
    }
}

/// List configured remotes as (name, url) pairs.
pub fn list_remotes(dir: &Path) -> Vec<(String, String)> {
    let Ok(repo) = Repository::open(dir) else {
        return Vec::new();
    };
    let Ok(names) = repo.remotes() else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|n| n.ok().flatten())
        .filter_map(|name| {
            repo.find_remote(name)
                .ok()
                .map(|r| (name.to_string(), r.url().unwrap_or_default().to_string()))
        })
        .collect()
}

/// Add a remote. Returns the git error text on failure.
pub fn add_remote(dir: &Path, name: &str, url: &str) -> Result<(), String> {
    let repo = Repository::open(dir).map_err(|e| e.message().to_string())?;
    repo.remote(name, url)
        .map_err(|e| e.message().to_string())?;
    Ok(())
}

/// Remove a remote. Returns the git error text on failure.
pub fn remove_remote(dir: &Path, name: &str) -> Result<(), String> {
    let repo = Repository::open(dir).map_err(|e| e.message().to_string())?;
    repo.remote_delete(name)
        .map_err(|e| e.message().to_string())?;
    Ok(())
}

/// Has the board file at `path` changed since `t` (RFC 3339) (FEAT-180)? Compares the file now —
/// the working tree, so a doc written in the same batch as a move still counts — with its blob in
/// the newest commit at or before `t`. Absent then and present now counts as changed; present then
/// and absent now does too. `None` when the board is not a git repository or `t` is not a time.
/// Judging several docs? Share one [`DocHistory`] instead: this opens and walks for each call.
pub fn doc_changed_since(path: &Path, t: &str) -> Option<bool> {
    DocHistory::default().changed_since(path, t)
}

/// What a [`DocHistory`] has done, for tests and measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryStats {
    /// Times the repository was opened.
    pub opens: usize,
    /// Times the history was walked.
    pub walks: usize,
}

/// One view of a board's git history for judging many docs (DEF-022). The repository is opened and
/// its history walked once, on the first doc judged; each baseline commit, each doc's blob at it
/// and each doc's blob now are found once. Make one per evaluation and drop it with the evaluation:
/// nothing is kept across requests, so a doc written a moment ago is seen.
#[derive(Default)]
pub struct DocHistory {
    state: std::sync::Mutex<HistoryState>,
}

#[derive(Default)]
struct HistoryState {
    /// `None` until the first doc is judged.
    repo: Option<Opened>,
    stats: HistoryStats,
    /// Each baseline, by the time asked for: the tree of the first commit at or before it.
    baselines: std::collections::HashMap<i64, Option<git2::Oid>>,
    /// A doc's blob in a baseline tree.
    then: std::collections::HashMap<(git2::Oid, std::path::PathBuf), Option<git2::Oid>>,
    /// A doc's blob in the working tree.
    now: std::collections::HashMap<std::path::PathBuf, Option<git2::Oid>>,
}

enum Opened {
    /// The repository could not be opened: there is no history to judge by.
    Failed,
    Repo {
        repo: Repository,
        workdir: std::path::PathBuf,
        /// `(commit time, tree)` in the order a `Sort::TIME | Sort::TOPOLOGICAL` walk from HEAD visits; empty when the
        /// repository has no commits.
        commits: Vec<(i64, git2::Oid)>,
    },
}

impl std::fmt::Debug for DocHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocHistory")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl DocHistory {
    /// What this history has opened and walked so far.
    pub fn stats(&self) -> HistoryStats {
        self.state.lock().map(|s| s.stats).unwrap_or_default()
    }

    /// The contract of [`doc_changed_since`], answered from the shared history.
    pub fn changed_since(&self, path: &Path, t: &str) -> Option<bool> {
        self.judge(path, t, true)
    }

    fn judge(&self, path: &Path, t: &str, may_fall_back: bool) -> Option<bool> {
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;
        let since = OffsetDateTime::parse(t.trim(), &Rfc3339)
            .ok()?
            .unix_timestamp();
        // A poisoned lock means a reader panicked mid-read: judge this doc on a fresh history.
        let Ok(mut guard) = self.state.lock() else {
            return DocHistory::default().judge(path, t, false);
        };
        let state = &mut *guard;
        if state.repo.is_none() {
            state.stats.opens += 1;
            state.repo = Some(match Repository::discover(path.parent()?) {
                Ok(repo) => match repo.workdir().and_then(|w| w.canonicalize().ok()) {
                    Some(workdir) => {
                        state.stats.walks += 1;
                        let commits = walk_commits(&repo);
                        Opened::Repo {
                            repo,
                            workdir,
                            commits,
                        }
                    }
                    None => Opened::Failed,
                },
                Err(_) => Opened::Failed,
            });
        }
        let Some(Opened::Repo {
            repo,
            workdir,
            commits,
        }) = &state.repo
        else {
            return None;
        };
        let full = path.parent()?.canonicalize().ok()?.join(path.file_name()?);
        let Ok(rel) = full.strip_prefix(workdir) else {
            // Outside the repository this history opened: not a board doc, judge it alone.
            drop(guard);
            return may_fall_back
                .then(|| DocHistory::default().judge(path, t, false))
                .flatten();
        };
        if commits.is_empty() {
            // No commits at all: anything on disk is new.
            return Some(full.exists());
        }
        // The baseline: the newest commit at or before `t`. None before the first commit.
        let tree = *state.baselines.entry(since).or_insert_with(|| {
            commits
                .iter()
                .find(|(time, _)| *time <= since)
                .map(|(_, tree)| *tree)
        });
        let then = match tree {
            Some(tree) => *state
                .then
                .entry((tree, rel.to_path_buf()))
                .or_insert_with(|| {
                    repo.find_tree(tree)
                        .ok()?
                        .get_path(rel)
                        .ok()
                        .map(|entry| entry.id())
                }),
            None => None,
        };
        let now = *state.now.entry(full.clone()).or_insert_with(|| {
            full.is_file()
                .then(|| git2::Oid::hash_file(git2::ObjectType::Blob, &full).ok())
                .flatten()
        });
        Some(then != now)
    }
}

/// The history from HEAD as `(commit time, tree)`, newest first by commit time. Of commits made in
/// the same second a child comes before its parent (DEF-035).
fn walk_commits(repo: &Repository) -> Vec<(i64, git2::Oid)> {
    let Ok(mut walk) = repo.revwalk() else {
        return Vec::new();
    };
    if walk.push_head().is_err()
        || walk
            .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
            .is_err()
    {
        return Vec::new();
    }
    walk.flatten()
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|c| (c.time().seconds(), c.tree_id()))
        .collect()
}

/// Each distinct content `rel` has had in the board repository's history from HEAD, newest first,
/// deduplicated by blob (DEF-036). An unreadable repository or path gives an empty list.
pub fn file_versions(dir: &Path, rel: &str) -> Vec<String> {
    let Ok(repo) = Repository::discover(dir) else {
        return Vec::new();
    };
    let Ok(mut walk) = repo.revwalk() else {
        return Vec::new();
    };
    if walk.push_head().is_err()
        || walk
            .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
            .is_err()
    {
        return Vec::new();
    }
    let mut seen: Vec<git2::Oid> = Vec::new();
    let mut out = Vec::new();
    for oid in walk.flatten() {
        let Some(entry) = repo
            .find_commit(oid)
            .ok()
            .and_then(|c| c.tree().ok())
            .and_then(|t| t.get_path(Path::new(rel)).ok())
        else {
            continue;
        };
        if seen.contains(&entry.id()) {
            continue;
        }
        seen.push(entry.id());
        if let Ok(blob) = repo.find_blob(entry.id())
            && let Ok(text) = std::str::from_utf8(blob.content())
        {
            out.push(text.to_string());
        }
    }
    out
}

/// One commit that changed a file, and the file as that commit left it (FTR-028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    pub sha: String,
    pub message: String,
    pub content: String,
}

/// Each commit from HEAD that changed `rel` (added it, or left it different from its first
/// parent), newest first, with its message and the file's text. A commit that removed it is not
/// listed. An unreadable repository or path gives an empty list.
pub fn file_history(dir: &Path, rel: &str) -> Vec<FileVersion> {
    let Ok(repo) = Repository::discover(dir) else {
        return Vec::new();
    };
    let Ok(mut walk) = repo.revwalk() else {
        return Vec::new();
    };
    if walk.push_head().is_err()
        || walk
            .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
            .is_err()
    {
        return Vec::new();
    }
    let blob_in = |c: &git2::Commit| {
        c.tree()
            .ok()
            .and_then(|t| t.get_path(Path::new(rel)).ok())
            .map(|e| e.id())
    };
    let mut out = Vec::new();
    for oid in walk.flatten() {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let Some(id) = blob_in(&commit) else {
            continue;
        };
        let before = commit.parent(0).ok().and_then(|p| blob_in(&p));
        if before == Some(id) {
            continue;
        }
        if let Ok(blob) = repo.find_blob(id)
            && let Ok(text) = std::str::from_utf8(blob.content())
        {
            out.push(FileVersion {
                sha: oid.to_string(),
                message: commit.message().unwrap_or_default().to_string(),
                content: text.to_string(),
            });
        }
    }
    out
}

/// The one-doc walk `DocHistory` replaced, kept as the reference its answers are checked against
/// (DEF-022).
#[cfg(test)]
pub(crate) fn doc_changed_since_single_walk(path: &Path, t: &str) -> Option<bool> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    let since = OffsetDateTime::parse(t.trim(), &Rfc3339)
        .ok()?
        .unix_timestamp();
    let repo = Repository::discover(path.parent()?).ok()?;
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let full = path.parent()?.canonicalize().ok()?.join(path.file_name()?);
    let rel = full.strip_prefix(&workdir).ok()?;

    // The baseline: the newest commit at or before `t`. None before the first commit.
    let mut walk = repo.revwalk().ok()?;
    if walk.push_head().is_err() {
        // No commits at all: anything on disk is new.
        return Some(full.exists());
    }
    walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .ok()?;
    let baseline = walk
        .flatten()
        .filter_map(|oid| repo.find_commit(oid).ok())
        .find(|c| c.time().seconds() <= since);
    let then = baseline.and_then(|c| c.tree().ok()?.get_path(rel).ok().map(|entry| entry.id()));
    let now = full
        .is_file()
        .then(|| git2::Oid::hash_file(git2::ObjectType::Blob, &full).ok())
        .flatten();
    Some(then != now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanbanr-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// FEAT-180: a doc is judged against the board as it stood when the item entered its stage.
    #[test]
    fn doc_changed_since_compares_with_the_commit_at_entry() {
        let dir = temp("docchanged");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let doc = dir.join("docs").join("data.md");
        std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
        std::fs::write(&doc, "v1").unwrap();
        assert!(commit(&dir, "Ada", "ada@example.com", "v1"));
        let future = "2999-01-01T00:00:00Z";
        let past = "2000-01-01T00:00:00Z";

        // Unchanged since the latest commit: not changed.
        assert_eq!(doc_changed_since(&doc, future), Some(false));
        // An uncommitted edit counts.
        std::fs::write(&doc, "v2").unwrap();
        assert_eq!(doc_changed_since(&doc, future), Some(true));
        // Before any commit held it, the doc was absent: it changed.
        assert_eq!(doc_changed_since(&doc, past), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    const T0: i64 = 1_700_000_000;

    /// Stage `paths` (relative to the repository) and commit them as of `secs`.
    fn commit_at(repo: &Repository, paths: &[&str], gone: &[&str], secs: i64) {
        let mut index = repo.index().unwrap();
        for p in paths {
            index.add_path(Path::new(p)).unwrap();
        }
        for p in gone {
            index.remove_path(Path::new(p)).unwrap();
        }
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::new("Ada", "ada@example.com", &git2::Time::new(secs, 0)).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
            .unwrap();
    }

    /// Three commits: an empty initial one and one adding `doc.md`, both made at `T0`, then a
    /// later one at `T0 + 5` that leaves the doc alone. Returns the repository folder, the doc and
    /// the RFC 3339 form of `T0`.
    fn same_second_board(name: &str) -> (PathBuf, PathBuf, String) {
        let dir = temp(name);
        let repo = Repository::init(&dir).unwrap();
        commit_at(&repo, &[], &[], T0);
        let doc = dir.join("doc.md");
        std::fs::write(&doc, "v1").unwrap();
        commit_at(&repo, &["doc.md"], &[], T0);
        std::fs::write(dir.join("other.md"), "x").unwrap();
        commit_at(&repo, &["other.md"], &[], T0 + 5);
        let t = time::OffsetDateTime::from_unix_timestamp(T0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        (dir, doc, t)
    }

    /// DEF-036: each distinct content of a file in the history, newest first; a content that came
    /// back is listed once, where it was last; a missing path or repository gives nothing.
    #[test]
    fn file_versions_lists_each_distinct_content_newest_first() {
        let dir = temp("fileversions");
        let repo = Repository::init(&dir).unwrap();
        for (i, text) in ["a", "b", "b", "a", "c"].iter().enumerate() {
            std::fs::write(dir.join("f.txt"), text).unwrap();
            commit_at(&repo, &["f.txt"], &[], T0 + i as i64);
        }
        assert_eq!(file_versions(&dir, "f.txt"), ["c", "a", "b"]);
        assert!(file_versions(&dir, "nope.txt").is_empty());
        assert!(file_versions(&temp("fileversions-none"), "f.txt").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-028 T9: each commit that changed a file, newest first, with its message and content.
    #[test]
    fn file_history_lists_each_commit_newest_first() {
        let dir = temp("filehistory");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        for (text, msg) in [("a", "one"), ("b", "two"), ("c", "three")] {
            std::fs::write(dir.join("f.txt"), text).unwrap();
            assert!(commit(&dir, "Ada", "ada@example.com", msg));
        }
        // A commit that leaves the file alone is not part of its history.
        std::fs::write(dir.join("g.txt"), "other").unwrap();
        assert!(commit(&dir, "Ada", "ada@example.com", "other file"));
        let h = file_history(&dir, "f.txt");
        let got: Vec<(&str, &str)> = h
            .iter()
            .map(|v| (v.content.as_str(), v.message.trim()))
            .collect();
        assert_eq!(got, [("c", "three"), ("b", "two"), ("a", "one")]);
        assert!(h.iter().all(|v| v.sha.len() == 40));
        assert!(file_history(&dir, "nope.txt").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-035: of commits made in the same second the newest, by ancestry, is the baseline.
    #[test]
    fn the_baseline_is_the_newest_of_commits_made_in_the_same_second() {
        let (dir, doc, t) = same_second_board("samesecond");
        assert_eq!(DocHistory::default().changed_since(&doc, &t), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-035: the shared history and the single-walk reference judge alike.
    #[test]
    fn doc_history_agrees_with_the_single_walk_on_same_second_commits() {
        let (dir, doc, t) = same_second_board("samesecondagree");
        assert_eq!(
            DocHistory::default().changed_since(&doc, &t),
            doc_changed_since_single_walk(&doc, &t)
        );
        assert_eq!(doc_changed_since_single_walk(&doc, &t), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A repository of 60 commits, two of them dated before their parents, and the docs it holds.
    fn history_board(name: &str) -> (PathBuf, Vec<PathBuf>) {
        let dir = temp(name);
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let repo = Repository::open(&dir).unwrap();
        let docs = dir.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        for i in 0..60i64 {
            let mut paths = vec![format!("docs/d{}.md", i % 5)];
            let mut gone = Vec::new();
            std::fs::write(docs.join(format!("d{}.md", i % 5)), format!("v{i}")).unwrap();
            if i == 5 {
                std::fs::write(docs.join("deleted.md"), "x").unwrap();
                paths.push("docs/deleted.md".into());
            }
            if i == 30 {
                std::fs::write(docs.join("added.md"), "x").unwrap();
                paths.push("docs/added.md".into());
            }
            if i == 45 {
                std::fs::remove_file(docs.join("deleted.md")).unwrap();
                gone.push("docs/deleted.md".to_string());
            }
            // Two commits dated before their parents: the walk's order is not the commit order.
            let at = if i == 20 || i == 40 { i - 30 } else { i };
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            let gone: Vec<&str> = gone.iter().map(String::as_str).collect();
            commit_at(&repo, &paths, &gone, T0 + at * 100);
        }
        // Edited in the working tree, left untracked, never there.
        std::fs::write(docs.join("d1.md"), "edited").unwrap();
        std::fs::write(docs.join("untracked.md"), "new").unwrap();
        let mut all: Vec<PathBuf> = [
            "d0",
            "d1",
            "d2",
            "d3",
            "d4",
            "added",
            "deleted",
            "untracked",
            "never",
        ]
        .iter()
        .map(|n| docs.join(format!("{n}.md")))
        .collect();
        all.push(dir.join("outside.md"));
        (dir, all)
    }

    /// DEF-022 R-1: judging many docs against one shared history gives, for each, the answer of
    /// judging it alone with its own walk.
    #[test]
    fn shared_history_gives_the_answers_of_the_single_doc_walk() {
        let (dir, docs) = history_board("shared-answers");
        let history = DocHistory::default();
        // Before the first commit, at commits, between them, and after the last.
        let times = [
            -500, 0, 50, 100, 1_000, 2_050, 2_450, 3_000, 4_100, 5_000, 5_900, 9_999,
        ];
        let mut answers = Vec::new();
        for offset in times {
            let at = time::OffsetDateTime::from_unix_timestamp(T0 + offset)
                .unwrap()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap();
            for doc in &docs {
                let want = doc_changed_since_single_walk(doc, &at);
                assert_eq!(history.changed_since(doc, &at), want, "{doc:?} at {at}");
                answers.push(want);
            }
        }
        // Not vacuous: both answers occur.
        assert!(answers.contains(&Some(true)) && answers.contains(&Some(false)));
        assert_eq!(history.changed_since(&docs[0], "not a time"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-022 R-2: any number of docs and entry times open the repository once and walk it once.
    #[test]
    fn shared_history_opens_the_repository_and_walks_once() {
        let (dir, docs) = history_board("shared-once");
        let history = DocHistory::default();
        assert_eq!(history.stats(), HistoryStats { opens: 0, walks: 0 });
        let mut calls = 0;
        for offset in [0, 1_000, 2_000, 3_000, 4_000] {
            let at = time::OffsetDateTime::from_unix_timestamp(T0 + offset)
                .unwrap()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap();
            for doc in docs.iter().take(6) {
                history.changed_since(doc, &at);
                calls += 1;
            }
        }
        assert_eq!(calls, 30);
        assert_eq!(history.stats(), HistoryStats { opens: 1, walks: 1 });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_changed_since_absent_and_deleted() {
        let dir = temp("docabsent");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let future = "2999-01-01T00:00:00Z";
        let missing = dir.join("docs").join("never.md");
        std::fs::create_dir_all(missing.parent().unwrap()).unwrap();
        // Absent then and absent now: nothing changed.
        assert_eq!(doc_changed_since(&missing, future), Some(false));
        let doc = dir.join("docs").join("gone.md");
        std::fs::write(&doc, "here").unwrap();
        assert!(commit(&dir, "Ada", "ada@example.com", "add"));
        std::fs::remove_file(&doc).unwrap();
        assert_eq!(
            doc_changed_since(&doc, future),
            Some(true),
            "a deletion is a change"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_changed_since_without_a_repository() {
        let dir = temp("docnorepo");
        let doc = dir.join("data.md");
        std::fs::write(&doc, "x").unwrap();
        // A temp folder outside any repository: there is no history to judge by.
        if Repository::discover(&dir).is_err() {
            assert_eq!(doc_changed_since(&doc, "2999-01-01T00:00:00Z"), None);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-128: every new board's first commit was authored `kanbanr <kanbanr@local>`, because
    /// the repository was created and committed before `init --author/--email` recorded anyone.
    #[test]
    fn the_first_commit_carries_the_users_identity() {
        let dir = temp("first");
        ensure_repo_as(&dir, Some(("Ada Lovelace", "ada@example.com")));
        let repo = Repository::open(&dir).unwrap();
        let first = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            first.parent_count(),
            0,
            "this is the repository's first commit"
        );
        for who in [first.author(), first.committer()] {
            assert_eq!(who.name().ok(), Some("Ada Lovelace"));
            assert_eq!(who.email().ok(), Some("ada@example.com"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-128: the placeholder was written into the repository's own config, where it hid the
    /// user's git identity from every later commit too.
    #[test]
    fn a_new_repository_gets_no_placeholder_identity() {
        let dir = temp("fresh");
        ensure_repo(&dir);
        let local = Repository::open(&dir)
            .unwrap()
            .config()
            .unwrap()
            .open_level(git2::ConfigLevel::Local)
            .unwrap();
        assert!(local.get_string("user.name").is_err());
        assert!(local.get_string("user.email").is_err());
        assert!(!has_placeholder_identity(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-128: with nobody to author it, a commit used to go out as the placeholder. Now there is
    /// no commit, and the reason names the command that fixes it.
    #[test]
    fn no_identity_refuses_to_commit() {
        let dir = temp("none");
        let config = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            git2::Config::open(&path).unwrap()
        };
        let placeholder = config(
            "placeholder",
            "[user]\n\tname = kanbanr\n\temail = kanbanr@local\n",
        );
        let user = config("user", "[user]\n\tname = Ada\n\temail = ada@example.com\n");

        // The placeholder is nobody; neither is an empty config.
        assert_eq!(resolve_identity(Some(&placeholder), None), None);
        assert_eq!(resolve_identity(None, None), None);
        // And it no longer hides the user's own identity.
        assert_eq!(
            resolve_identity(Some(&placeholder), Some(&user)),
            Some(("Ada".into(), "ada@example.com".into()))
        );
        assert!(NO_IDENTITY.contains("kanbanr identity"));

        // End to end, where this machine's own git config names nobody either.
        let repo_dir = temp("none-repo");
        ensure_repo(&repo_dir);
        if identity(&repo_dir).is_none() {
            std::fs::write(repo_dir.join("a.txt"), "a").unwrap();
            assert!(!commit_local(&repo_dir, "a change"));
            assert!(
                require_identity(&repo_dir)
                    .unwrap_err()
                    .contains("kanbanr identity")
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo_dir);
    }

    /// FEAT-135: a summary written beside the board never reaches its history, and an existing
    /// board learns that on its next write.
    #[test]
    fn a_board_ignores_its_session_summaries() {
        let dir = temp("sessions");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let notes = dir.join(".sessions").join("shop");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("2026-10-01-120000-abcd1234.md"), "a summary").unwrap();
        std::fs::write(dir.join("item.yaml"), "x: 1").unwrap();
        assert!(commit_local(&dir, "a write"));
        let repo = Repository::open(&dir).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(
            tree.get_name("item.yaml").is_some(),
            "the write was committed"
        );
        assert!(tree.get_name(".sessions").is_none(), "the summary was not");
        let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l.trim() == SESSIONS_DIR), "{ignore}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-025 R-6: a leftover of an interrupted write never reaches the board's history.
    #[test]
    fn the_board_ignores_leftover_temporary_files() {
        let dir = temp("leftover-tmp");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        std::fs::write(dir.join(".x.1.1.tmp"), "half").unwrap();
        std::fs::write(dir.join("item.yaml"), "x: 1").unwrap();
        assert!(commit_local(&dir, "a write"));
        let repo = Repository::open(&dir).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(
            tree.get_name("item.yaml").is_some(),
            "the write was committed"
        );
        assert!(
            tree.get_name(".x.1.1.tmp").is_none(),
            "the leftover was not"
        );
        let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l.trim() == "*.tmp"), "{ignore}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
