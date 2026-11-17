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

/// The git directory of the repository whose worktree is `dir` (FTR-035): `dir/.git` when it is a
/// folder; the folder a `.git` file's `gitdir:` line names when it is a linked worktree's pointer
/// (relative to `dir` when the line is relative); `dir/.git` otherwise. Read from the file system
/// only, with no git process, so a hook can call it on every question it hears.
#[must_use]
pub fn git_dir(dir: &Path) -> std::path::PathBuf {
    let dot_git = dir.join(".git");
    if dot_git.is_file()
        && let Ok(text) = std::fs::read_to_string(&dot_git)
        && let Some(line) = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))
    {
        let named = Path::new(line.trim());
        if !named.as_os_str().is_empty() {
            return if named.is_absolute() {
                named.to_path_buf()
            } else {
                dir.join(named)
            };
        }
    }
    dot_git
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

/// The board repository's HEAD commit, 40 hex; None when `dir` is not a repository or has no
/// commit yet (FTR-003: the board commit in a view's facts part).
pub fn head_commit(dir: &Path) -> Option<String> {
    let repo = Repository::open(dir).ok()?;
    let head = repo.head().ok()?;
    Some(head.peel_to_commit().ok()?.id().to_string())
}

/// The HEAD commit's committer time as RFC 3339 UTC; None when `dir` is not a repository or has
/// no commit yet (FTR-040: a board's "last changed" on the board list).
pub fn head_time(dir: &Path) -> Option<String> {
    let repo = Repository::open(dir).ok()?;
    let head = repo.head().ok()?;
    let secs = head.peel_to_commit().ok()?.time().seconds();
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
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
    commit_write(dir, message).is_ok_and(|o| o.is_some())
}

/// How long a commit waits for git's own locks (`index.lock`, `HEAD.lock`, a ref's lock) before
/// giving up (DEF-102). `KANBANR_COMMIT_WAIT_MS` overrides it, in milliseconds.
pub const COMMIT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The wait in force: `COMMIT_WAIT` unless `KANBANR_COMMIT_WAIT_MS` names another.
fn commit_wait() -> std::time::Duration {
    std::env::var("KANBANR_COMMIT_WAIT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(COMMIT_WAIT, std::time::Duration::from_millis)
}

/// Why a board commit was not made (DEF-102).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// Nobody to author the commit.
    NoIdentity,
    /// A git lock file stayed held for the whole wait.
    Locked {
        file: std::path::PathBuf,
        waited: std::time::Duration,
    },
    /// git refused for any other reason.
    Git(String),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::NoIdentity => write!(f, "nobody to author the commit: {NO_IDENTITY}"),
            CommitError::Locked { file, waited } => write!(
                f,
                "git's {} was held by another program for {waited:.1?}; if no git is running, \
                 remove that file",
                file.display()
            ),
            CommitError::Git(msg) => write!(f, "git refused the commit: {msg}"),
        }
    }
}

impl std::error::Error for CommitError {}

#[cfg(test)]
type BeforeCommitHook = Box<dyn FnOnce()>;

#[cfg(test)]
thread_local! {
    /// A test seam: run once, between staging and `repo.commit`, to move HEAD under the commit.
    static BEFORE_COMMIT: std::cell::RefCell<Option<BeforeCommitHook>> =
        const { std::cell::RefCell::new(None) };
}

/// The lock file a git2 "locked" error names: the quoted path ending in `.lock`, else the index's.
fn lock_file_of(repo_dir: &Path, e: &git2::Error) -> std::path::PathBuf {
    let msg = e.message();
    msg.split('\'')
        .find(|part| part.ends_with(".lock"))
        .map_or_else(
            || git_dir(repo_dir).join("index.lock"),
            std::path::PathBuf::from,
        )
}

/// One staging-and-commit attempt on a freshly opened repository.
fn commit_attempt(dir: &Path, message: &str) -> Result<Option<git2::Oid>, CommitAttempt> {
    let repo = Repository::open(dir).map_err(|e| CommitAttempt::Failed(e.message().to_string()))?;
    let sig = signature(&repo).map_err(|_| CommitAttempt::NoIdentity)?;
    let tree_oid = stage_all(&repo).map_err(CommitAttempt::Retry)?;
    let tree = repo.find_tree(tree_oid).map_err(CommitAttempt::Retry)?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    // Nothing to commit if the staged tree equals the current HEAD tree.
    if let Some(p) = &parent
        && p.tree_id() == tree_oid
    {
        return Ok(None);
    }
    crate::write::fault("staged");
    #[cfg(test)]
    BEFORE_COMMIT.with(|h| {
        if let Some(hook) = h.borrow_mut().take() {
            hook();
        }
    });
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map(Some)
        .map_err(CommitAttempt::Retry)
}

enum CommitAttempt {
    NoIdentity,
    Failed(String),
    /// A git2 error: retried when it is a lock or a moved HEAD.
    Retry(git2::Error),
}

/// Stage everything and commit on the current HEAD with the board's identity (DEF-102).
/// `Ok(None)` when the tree equals HEAD's. A git lock is waited out for `COMMIT_WAIT`; a HEAD that
/// moved under the commit is read again and the commit made on the new tip.
pub fn commit_write(dir: &Path, message: &str) -> Result<Option<git2::Oid>, CommitError> {
    commit_write_within(dir, message, commit_wait())
}

/// `commit_write` with the wait given, for tests.
pub(crate) fn commit_write_within(
    dir: &Path,
    message: &str,
    wait: std::time::Duration,
) -> Result<Option<git2::Oid>, CommitError> {
    use git2::ErrorCode;
    crate::project::assert_inside_fence(dir); // FEAT-195
    let started = std::time::Instant::now();
    let mut pause = std::time::Duration::from_millis(10);
    loop {
        match commit_attempt(dir, message) {
            Ok(oid) => return Ok(oid),
            Err(CommitAttempt::NoIdentity) => return Err(CommitError::NoIdentity),
            Err(CommitAttempt::Failed(msg)) => return Err(CommitError::Git(msg)),
            Err(CommitAttempt::Retry(e)) => {
                let waiting = matches!(e.code(), ErrorCode::Locked | ErrorCode::Modified);
                if waiting && started.elapsed() < wait {
                    let left = wait.saturating_sub(started.elapsed());
                    std::thread::sleep(pause.min(left));
                    pause = (pause * 2).min(std::time::Duration::from_millis(200));
                    continue;
                }
                return Err(if e.code() == ErrorCode::Locked {
                    CommitError::Locked {
                        file: lock_file_of(dir, &e),
                        waited: started.elapsed(),
                    }
                } else {
                    CommitError::Git(e.message().to_string())
                });
            }
        }
    }
}

/// The board's changed paths (DEF-102): `is_clean`'s list, ignored files excluded and untracked
/// ones included, sorted. Empty when `dir` is not a repository.
pub fn changed_paths(dir: &Path) -> Vec<String> {
    let mut paths = is_clean(dir).err().unwrap_or_default();
    paths.sort();
    paths.dedup();
    paths
}

/// Put each of `paths` back as HEAD has it (DEF-102): the index entry and the file. A path HEAD
/// does not have is deleted, and the folder kanbanr made for it with it when that is left empty.
pub fn restore_to_head(dir: &Path, paths: &[String]) -> Result<(), git2::Error> {
    crate::project::assert_inside_fence(dir); // FEAT-195
    if paths.is_empty() {
        return Ok(());
    }
    for p in paths {
        let rel = Path::new(p);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(git2::Error::from_str(&format!(
                "{p} is outside the board, so it is not put back"
            )));
        }
    }
    let repo = Repository::open(dir)?;
    let head = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    match &head {
        Some(c) => repo.reset_default(Some(c.as_object()), paths.iter())?,
        None => repo.reset_default(None, paths.iter())?,
    }
    let tree = head.as_ref().and_then(|c| c.tree().ok());
    let in_head: Vec<&String> = paths
        .iter()
        .filter(|p| {
            tree.as_ref()
                .is_some_and(|t| t.get_path(Path::new(p)).is_ok())
        })
        .collect();
    if !in_head.is_empty() {
        let mut co = git2::build::CheckoutBuilder::new();
        co.force();
        for p in &in_head {
            co.path(p.as_str());
        }
        repo.checkout_head(Some(&mut co))?;
    }
    for p in paths.iter().filter(|p| !in_head.contains(p)) {
        let full = dir.join(p);
        match std::fs::remove_file(&full) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(git2::Error::from_str(&format!("{}: {e}", full.display()))),
        }
        // A folder that is empty now was made for this file; remove it, up to the board's root.
        let mut parent = full.parent();
        while let Some(d) = parent {
            if d == dir || std::fs::remove_dir(d).is_err() {
                break;
            }
            parent = d.parent();
        }
    }
    Ok(())
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

/// `git@github.com:o/r.git` and `https://github.com/o/r.git` both become `https://github.com/o/r`;
/// a local path or an unfamiliar scheme is no address a reader can open.
pub fn web_url(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = trimmed.strip_prefix("ssh://git@") {
        return Some(format!("https://{rest}"));
    }
    trimmed.starts_with("https://").then(|| trimmed.to_string())
}

/// The URL of the remote named `origin`, else the first remote, else `None`.
pub fn remote_url(dir: &Path) -> Option<String> {
    let remotes = list_remotes(dir);
    remotes
        .iter()
        .find(|(n, _)| n == "origin")
        .or_else(|| remotes.first())
        .map(|(_, u)| u.clone())
        .filter(|u| !u.is_empty())
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

/// One view of a board's git history for judging many docs (DEF-022). The history is the shared
/// [`PathHistory`] walk, kept per HEAD across instances (MNT-014), so a first doc judged walks only
/// when no one has walked this HEAD; a history with merges is walked by libgit2 instead, once per
/// instance. Each doc's blob now is read from the working tree and found once. Make one per
/// evaluation and drop it with the evaluation: the working-tree side is never kept across
/// requests, so a doc written a moment ago is seen.
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
    /// The shared walk of a linear history: no object is read from the repository.
    Shared {
        history: std::sync::Arc<PathHistory>,
        workdir: std::path::PathBuf,
    },
    /// libgit2's walk, for a history with merges.
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

    /// Whether the shared walk answers (and not libgit2's), for tests.
    #[cfg(test)]
    fn is_shared(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| matches!(s.repo, Some(Opened::Shared { .. })))
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
                Ok(repo) => match repo.workdir().and_then(resolved) {
                    Some(workdir) => match shared_walk(path.parent()?) {
                        Some((history, walked)) => {
                            state.stats.walks += usize::from(walked);
                            Opened::Shared { history, workdir }
                        }
                        None => {
                            state.stats.walks += 1;
                            let commits = walk_commits(&repo);
                            Opened::Repo {
                                repo,
                                workdir,
                                commits,
                            }
                        }
                    },
                    None => Opened::Failed,
                },
                Err(_) => Opened::Failed,
            });
        }
        let workdir = match state.repo.as_ref()? {
            Opened::Failed => return None,
            Opened::Shared { workdir, .. } | Opened::Repo { workdir, .. } => workdir,
        };
        let full = resolved_file(path)?;
        let Ok(rel) = full.strip_prefix(workdir) else {
            // Outside the repository this history opened: not a board doc, judge it alone.
            drop(guard);
            return may_fall_back
                .then(|| DocHistory::default().judge(path, t, false))
                .flatten();
        };
        let rel = std::path::PathBuf::from(slash_joined(rel));
        let (repo, commits) = match state.repo.as_ref()? {
            Opened::Shared { history, .. } => {
                if history.commits.is_empty() {
                    // No commits at all: anything on disk is new.
                    return Some(full.exists());
                }
                let rel = slash_joined(&rel);
                let then = history
                    .baseline(since)
                    .and_then(|i| history.blob_at(&rel, i))
                    .and_then(|b| git2::Oid::from_str(b).ok());
                let now = *state.now.entry(full.clone()).or_insert_with(|| {
                    full.is_file()
                        .then(|| git2::Oid::hash_file(git2::ObjectType::Blob, &full).ok())
                        .flatten()
                });
                return Some(then != now);
            }
            Opened::Repo { repo, commits, .. } => (repo, commits),
            Opened::Failed => return None,
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
            Some(tree) => *state.then.entry((tree, rel.clone())).or_insert_with(|| {
                repo.find_tree(tree)
                    .ok()?
                    .get_path(&rel)
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

/// The shared walk of the repository holding `dir`, and whether this call walked it, when the
/// history is linear; `None` when it has merges or cannot be walked (MNT-014).
fn shared_walk(dir: &Path) -> Option<(std::sync::Arc<PathHistory>, bool)> {
    let before = path_history_walks_of(dir);
    let history = path_history(dir).ok().filter(|h| h.is_linear())?;
    Some((history, path_history_walks_of(dir) > before))
}

/// `path` resolved through `canonicalize`, without Windows' verbatim prefix (`\\?\`), so it joins
/// and compares with the other resolved paths (MNT-007).
fn resolved(path: &Path) -> Option<std::path::PathBuf> {
    path.canonicalize()
        .ok()
        .map(crate::project::without_verbatim_prefix)
}

/// A file's path with its folder resolved; the file itself need not exist.
fn resolved_file(path: &Path) -> Option<std::path::PathBuf> {
    Some(resolved(path.parent()?)?.join(path.file_name()?))
}

/// A path inside the workdir as git names it: its components joined with `/` on every system.
fn slash_joined(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
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

/// Why a board's git history could not be read (DEF-080).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryReason {
    NotARepository,
    OwnedByAnotherUser,
    /// libgit2's message for the commit it could not read.
    Incomplete(String),
    /// libgit2's message.
    Other(String),
}

/// A history read that failed: the repository (its workdir when it opened, else the folder
/// given) and why (DEF-080).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryError {
    pub repo: std::path::PathBuf,
    pub reason: HistoryReason,
}

impl HistoryError {
    /// The reason alone, without the path, for pages served to others (DEF-080).
    pub fn reason_text(&self) -> String {
        match &self.reason {
            HistoryReason::NotARepository => "it is not a git repository".to_string(),
            HistoryReason::OwnedByAnotherUser => format!(
                "it is owned by another user, and git refuses such a repository (its \
                 safe.directory rule); run kanbanr as the folder's owner, or trust it with git \
                 config --global --add safe.directory {}",
                self.repo.display()
            ),
            HistoryReason::Incomplete(m) => {
                format!("its history is incomplete (a shallow or partial clone?): {m}")
            }
            HistoryReason::Other(m) => m.clone(),
        }
    }
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot read the git history of {}: {}",
            self.repo.display(),
            self.reason_text()
        )
    }
}

impl std::error::Error for HistoryError {}

/// The reason a libgit2 error gives for a history that cannot be read (DEF-080).
pub fn reason_of(e: &git2::Error) -> HistoryReason {
    match (e.code(), e.class()) {
        (git2::ErrorCode::Owner, _) => HistoryReason::OwnedByAnotherUser,
        (git2::ErrorCode::NotFound, git2::ErrorClass::Repository) => HistoryReason::NotARepository,
        _ => HistoryReason::Other(e.message().to_string()),
    }
}

/// Open the repository holding `dir` for a history read; the one place a failure to open it is
/// classified (DEF-080).
pub fn open_history(dir: &Path) -> Result<Repository, HistoryError> {
    Repository::discover(dir).map_err(|e| HistoryError {
        repo: dir.to_path_buf(),
        reason: reason_of(&e),
    })
}

/// Write `note: <text>` to stderr once per `key` per process (DEF-080).
pub fn warn_once(key: &str, text: &str) {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let first = match seen.lock() {
        Ok(mut s) => s.insert(key.to_string()),
        // A poisoned lock still prints.
        Err(_) => true,
    };
    if first {
        eprintln!("note: {text}");
    }
}

/// A revwalk from HEAD, newest first, for a history read. `None` when HEAD is unborn (no commits
/// yet): an empty history (DEF-080).
fn history_walk<'r>(
    repo: &'r Repository,
    err: &dyn Fn(HistoryReason) -> HistoryError,
) -> Result<Option<git2::Revwalk<'r>>, HistoryError> {
    let other = |e: git2::Error| err(HistoryReason::Other(e.message().to_string()));
    let mut walk = repo.revwalk().map_err(other)?;
    match walk.push_head() {
        Ok(()) => {}
        // libgit2 reports an unborn HEAD to push_head as a missing reference; HEAD itself
        // says UnbornBranch.
        Err(e)
            if e.code() == git2::ErrorCode::UnbornBranch
                || repo
                    .head()
                    .is_err_and(|h| h.code() == git2::ErrorCode::UnbornBranch) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(other(e)),
    }
    walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .map_err(other)?;
    Ok(Some(walk))
}

/// The commit `item` names, or the walk stops as incomplete (DEF-080).
fn walked_commit<'r>(
    repo: &'r Repository,
    item: Result<git2::Oid, git2::Error>,
    err: &dyn Fn(HistoryReason) -> HistoryError,
) -> Result<git2::Commit<'r>, HistoryError> {
    item.and_then(|oid| repo.find_commit(oid))
        .map_err(|e| err(HistoryReason::Incomplete(e.message().to_string())))
}

/// The `HistoryError` maker for an opened repository: its workdir, else `dir` (DEF-080).
fn history_err(repo: &Repository, dir: &Path) -> impl Fn(HistoryReason) -> HistoryError {
    let at = repo.workdir().unwrap_or(dir).to_path_buf();
    move |reason| HistoryError {
        repo: at.clone(),
        reason,
    }
}

/// Each distinct content `rel` has had in the board repository's history from HEAD, newest first,
/// deduplicated by blob (DEF-036). A path absent from the history gives an empty list; a
/// repository that cannot be read, or whose history cannot be walked, is an error naming it and
/// why (DEF-080).
pub fn file_versions(dir: &Path, rel: &str) -> Result<Vec<String>, HistoryError> {
    if let Ok(history) = path_history(dir)
        && history.is_linear()
    {
        let mut seen: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        for blob in history.touches(rel).into_iter().filter_map(|t| t.blob) {
            if seen.contains(&blob) {
                continue;
            }
            seen.push(blob);
            if let Some(text) = history.blob_text(dir, blob) {
                out.push(text);
            }
        }
        return Ok(out);
    }
    file_versions_walk(dir, rel)
}

/// [`file_versions`] by walking every commit's tree: the answer for a history with merges, and the
/// reference the shared walk is checked against (MNT-014).
fn file_versions_walk(dir: &Path, rel: &str) -> Result<Vec<String>, HistoryError> {
    let repo = open_history(dir)?;
    let err = history_err(&repo, dir);
    let Some(walk) = history_walk(&repo, &err)? else {
        return Ok(Vec::new());
    };
    let mut seen: Vec<git2::Oid> = Vec::new();
    let mut out = Vec::new();
    for item in walk {
        let commit = walked_commit(&repo, item, &err)?;
        let Some(entry) = commit
            .tree()
            .ok()
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
    Ok(out)
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
/// listed. A path absent from the history gives an empty list; a repository that cannot be read,
/// or whose history cannot be walked, is an error naming it and why (DEF-080).
pub fn file_history(dir: &Path, rel: &str) -> Result<Vec<FileVersion>, HistoryError> {
    if let Ok(history) = path_history(dir)
        && history.is_linear()
    {
        let touches = history.touches(rel);
        let mut out = Vec::new();
        for (i, touch) in touches.iter().enumerate() {
            let Some(blob) = touch.blob else { continue };
            // A mode-only change leaves the blob as the next older touch had it.
            if touches.get(i + 1).and_then(|older| older.blob) == Some(blob) {
                continue;
            }
            if let Some(content) = history.blob_text(dir, blob) {
                out.push(FileVersion {
                    sha: touch.commit.sha.clone(),
                    message: history.message(dir, &touch.commit.sha).unwrap_or_default(),
                    content,
                });
            }
        }
        return Ok(out);
    }
    file_history_walk(dir, rel)
}

/// [`file_history`] by walking every commit's tree: the answer for a history with merges, and the
/// reference the shared walk is checked against (MNT-014).
fn file_history_walk(dir: &Path, rel: &str) -> Result<Vec<FileVersion>, HistoryError> {
    let repo = open_history(dir)?;
    let err = history_err(&repo, dir);
    let Some(walk) = history_walk(&repo, &err)? else {
        return Ok(Vec::new());
    };
    let blob_in = |c: &git2::Commit| {
        c.tree()
            .ok()
            .and_then(|t| t.get_path(Path::new(rel)).ok())
            .map(|e| e.id())
    };
    let mut out = Vec::new();
    for item in walk {
        let commit = walked_commit(&repo, item, &err)?;
        let oid = commit.id();
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
    Ok(out)
}

// ---- one rename-aware walk of a repository's history per HEAD (DEF-106) ----

/// How many repositories' walks are kept at once (least recently used out).
pub const PATH_HISTORY_SLOTS: usize = 4;

/// A commit of a [`PathHistory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMeta {
    pub sha: String,
    /// The committer date, RFC 3339 (`%cI`).
    pub at: String,
    pub subject: String,
    /// The committer time in seconds (`%ct`), the time [`DocHistory`] compares (MNT-014).
    pub unix: i64,
    /// More than one parent (`%P`). A merge lists no touches.
    pub merge: bool,
}

/// One change to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Touch {
    /// Index into `commits`, newest first.
    commit: usize,
    /// 40 hex; empty when the path was deleted.
    blob: String,
    /// The path it was renamed from.
    from: Option<String>,
}

/// One version of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathVersion {
    pub sha: String,
    pub at: String,
    pub subject: String,
    pub path: String,
    /// Empty when this commit deleted the path.
    pub blob: String,
}

/// The history of a repository at one HEAD, from one `git log --raw -M`.
#[derive(Debug, Default)]
pub struct PathHistory {
    head: String,
    commits: Vec<CommitMeta>,
    touches: std::collections::HashMap<String, Vec<Touch>>,
    /// Commits with more than one parent.
    merges: usize,
}

/// One change to a path, as [`PathHistory::touches`] lists it (MNT-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathTouch<'a> {
    pub commit: &'a CommitMeta,
    /// The commit's index in walk order, newest first.
    pub index: usize,
    /// The path's blob after the commit; `None` when the commit deleted it.
    pub blob: Option<&'a str>,
}

impl PathHistory {
    /// No commit has more than one parent. Only then do `baseline` and `blob_at` match a walk of
    /// the trees (MNT-014).
    pub fn is_linear(&self) -> bool {
        self.merges == 0
    }

    /// The changes to exactly `rel`, newest first, without following renames. A rename into `rel`
    /// is a touch with its blob; a rename away from it is a delete (MNT-014).
    pub fn touches(&self, rel: &str) -> Vec<PathTouch<'_>> {
        self.touches
            .get(rel)
            .map(|list| {
                list.iter()
                    .map(|t| PathTouch {
                        commit: &self.commits[t.commit],
                        index: t.commit,
                        blob: (!t.blob.is_empty()).then_some(t.blob.as_str()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The index of the first commit, in walk order, committed at or before `unix`; `None` when
    /// every commit is newer. The rule [`DocHistory`] judges by (MNT-014).
    pub fn baseline(&self, unix: i64) -> Option<usize> {
        self.commits.iter().position(|c| c.unix <= unix)
    }

    /// `rel`'s blob in commit `index`'s tree: the blob of its newest touch at or before `index`.
    /// `None` when that touch is a delete or there is none. Valid on a linear history only
    /// (MNT-014).
    pub fn blob_at(&self, rel: &str, index: usize) -> Option<&str> {
        let list = self.touches.get(rel)?;
        let i = list.partition_point(|t| t.commit < index);
        let t = list.get(i)?;
        (!t.blob.is_empty()).then_some(t.blob.as_str())
    }

    /// The commit the history was walked at (empty for a repository with no commits).
    pub fn head(&self) -> &str {
        &self.head
    }

    /// The newest commit touching any of `rels` (repository-relative, `/`-separated); the answer of
    /// `git log -1 --format=%cI -- <rels>`.
    pub fn last_change(&self, rels: &[&str]) -> Option<&CommitMeta> {
        rels.iter()
            .filter_map(|r| self.touches.get(*r)?.first())
            .map(|t| t.commit)
            .min()
            .map(|i| &self.commits[i])
    }

    /// Each version of `rel`, newest first, following renames backwards as
    /// `git log --follow -- <rel>` does. A deletion is an entry with an empty blob.
    pub fn versions(&self, rel: &str) -> Vec<PathVersion> {
        let mut out = Vec::new();
        let mut cur = rel.to_string();
        let mut newest_allowed = 0usize;
        while let Some(list) = self.touches.get(&cur) {
            let mut next: Option<(String, usize)> = None;
            for t in list.iter().filter(|t| t.commit >= newest_allowed) {
                let c = &self.commits[t.commit];
                out.push(PathVersion {
                    sha: c.sha.clone(),
                    at: c.at.clone(),
                    subject: c.subject.clone(),
                    path: cur.clone(),
                    blob: t.blob.clone(),
                });
                if let Some(from) = &t.from {
                    // The rename commit also deleted the old name: take the old name's history from
                    // the commits before it.
                    next = Some((from.clone(), t.commit + 1));
                    break;
                }
            }
            match next {
                Some((from, at)) => {
                    cur = from;
                    newest_allowed = at;
                }
                None => break,
            }
        }
        out
    }

    /// A blob's UTF-8 text from the repository holding `dir`; `None` if it is missing or not text.
    pub fn blob_text(&self, dir: &Path, blob: &str) -> Option<String> {
        let repo = Repository::discover(dir).ok()?;
        let oid = git2::Oid::from_str(blob).ok()?;
        let b = repo.find_blob(oid).ok()?;
        std::str::from_utf8(b.content()).ok().map(str::to_string)
    }

    /// A commit's full message from the repository holding `dir`.
    pub fn message(&self, dir: &Path, sha: &str) -> Option<String> {
        let repo = Repository::discover(dir).ok()?;
        let oid = git2::Oid::from_str(sha).ok()?;
        let c = repo.find_commit(oid).ok()?;
        Some(String::from_utf8_lossy(c.message_bytes()).into_owned())
    }
}

type HistorySlot = std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<PathHistory>>>>;

static PATH_HISTORY_CACHE: std::sync::Mutex<Vec<((std::path::PathBuf, String), HistorySlot)>> =
    std::sync::Mutex::new(Vec::new());
static PATH_HISTORY_WALKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Walks run, by repository (its canonical git folder).
static PATH_HISTORY_WALKS_BY_REPO: std::sync::Mutex<Vec<(std::path::PathBuf, u64)>> =
    std::sync::Mutex::new(Vec::new());

/// Extensions run, by repository.
static PATH_HISTORY_EXTENDS_BY_REPO: std::sync::Mutex<Vec<(std::path::PathBuf, u64)>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
thread_local! {
    static WALKS_HERE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The walks run in this process so far, for tests and the bench.
pub fn path_history_walks() -> u64 {
    PATH_HISTORY_WALKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The walks run for the repository holding `dir`, so tests running side by side do not see each
/// other's walks (MNT-014). `path_history_walks` is the sum over every repository.
pub fn path_history_walks_of(dir: &Path) -> u64 {
    let Some(git_dir) = git_dir_of(dir) else {
        return 0;
    };
    PATH_HISTORY_WALKS_BY_REPO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(d, _)| *d == git_dir)
        .map_or(0, |(_, n)| *n)
}

/// Extensions run for the repository holding `dir`: walks of the new commits only, on top of a
/// kept history (MNT-014).
pub fn path_history_extends_of(dir: &Path) -> u64 {
    let Some(git_dir) = git_dir_of(dir) else {
        return 0;
    };
    PATH_HISTORY_EXTENDS_BY_REPO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(d, _)| *d == git_dir)
        .map_or(0, |(_, n)| *n)
}

/// The canonical git folder of the repository holding `dir`.
fn git_dir_of(dir: &Path) -> Option<std::path::PathBuf> {
    let repo = Repository::discover(dir).ok()?;
    Some(
        repo.path()
            .canonicalize()
            .unwrap_or_else(|_| repo.path().to_path_buf()),
    )
}

/// The walks run by this thread, so a test is not counted against by the others running beside it.
#[cfg(test)]
pub(crate) fn path_history_walks_here() -> u64 {
    WALKS_HERE.with(std::cell::Cell::get)
}

/// The history of the repository holding `dir`, at its HEAD, from one `git log --raw -M`. It is
/// kept for the last [`PATH_HISTORY_SLOTS`] (repository, HEAD) pairs; a second caller for one pair
/// waits for the first and shares its answer.
pub fn path_history(dir: &Path) -> Result<std::sync::Arc<PathHistory>, HistoryError> {
    let repo = open_history(dir)?;
    let git_dir = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let head = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .unwrap_or_default();
    let root = repo.workdir().unwrap_or(repo.path()).to_path_buf();
    let counted = git_dir.clone();
    let key = (git_dir, head.clone());

    let slot: HistorySlot = {
        let mut cache = PATH_HISTORY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(i);
            cache.insert(0, entry);
        } else {
            cache.insert(0, (key, HistorySlot::default()));
            cache.truncate(PATH_HISTORY_SLOTS);
        }
        cache[0].1.clone()
    };
    let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = held.as_ref() {
        return Ok(h.clone());
    }
    // A kept history of an older HEAD of this repository that this HEAD descends from grows by the
    // new commits (MNT-014); anything else walks in full.
    if !head.is_empty()
        && let Some(grown) = extend_kept(&root, &counted, &head)
    {
        bump(&PATH_HISTORY_EXTENDS_BY_REPO, &counted);
        let built = std::sync::Arc::new(grown);
        *held = Some(built.clone());
        return Ok(built);
    }
    bump(&PATH_HISTORY_WALKS_BY_REPO, &counted);
    let built = std::sync::Arc::new(walk_path_history(&root, &head).map_err(|m| HistoryError {
        repo: root.clone(),
        reason: HistoryReason::Other(m),
    })?);
    *held = Some(built.clone());
    Ok(built)
}

fn bump(counts: &std::sync::Mutex<Vec<(std::path::PathBuf, u64)>>, git_dir: &Path) {
    let mut by = counts.lock().unwrap_or_else(|e| e.into_inner());
    match by.iter_mut().find(|(d, _)| d == git_dir) {
        Some((_, n)) => *n += 1,
        None => by.push((git_dir.to_path_buf(), 1)),
    }
}

/// The kept history of this repository whose HEAD is an ancestor of `head`, grown by the commits
/// `H0..head`. `None` when there is none, or any step fails: the caller walks in full.
fn extend_kept(root: &Path, git_dir: &Path, head: &str) -> Option<PathHistory> {
    let kept: Vec<std::sync::Arc<PathHistory>> = {
        let cache = PATH_HISTORY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .iter()
            .filter(|((d, h), _)| d == git_dir && !h.is_empty() && h != head)
            // A slot being built is skipped; this thread holds its own and never waits.
            .filter_map(|(_, slot)| slot.try_lock().ok()?.clone())
            .collect()
    };
    for old in kept {
        let base = old.head.as_str();
        if crate::coderepo::git_raw(root, &["merge-base", "--is-ancestor", base, head]).is_err() {
            continue;
        }
        let range = format!("{base}..{head}");
        let Ok(raw) = crate::coderepo::git_raw(
            root,
            &[
                "-c",
                "core.quotePath=false",
                "log",
                "--raw",
                "-M",
                "--no-abbrev",
                "--format=%x1e%H%x1f%cI%x1f%ct%x1f%P%x1f%s",
                &range,
            ],
        ) else {
            continue;
        };
        let mut new = PathHistory {
            head: head.to_string(),
            ..PathHistory::default()
        };
        parse_path_history(&String::from_utf8_lossy(&raw), &mut new);
        let shift = new.commits.len();
        let mut touches = new.touches;
        for (path, list) in &old.touches {
            let mut shifted: Vec<Touch> = list
                .iter()
                .map(|t| Touch {
                    commit: t.commit + shift,
                    ..t.clone()
                })
                .collect();
            touches
                .entry(path.clone())
                .or_default()
                .append(&mut shifted);
        }
        let mut commits = new.commits;
        commits.extend(old.commits.iter().cloned());
        return Some(PathHistory {
            head: head.to_string(),
            commits,
            touches,
            merges: new.merges + old.merges,
        });
    }
    None
}

/// Run the one walk and parse it.
fn walk_path_history(root: &Path, head: &str) -> Result<PathHistory, String> {
    PATH_HISTORY_WALKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    WALKS_HERE.with(|c| c.set(c.get() + 1));
    let mut h = PathHistory {
        head: head.to_string(),
        ..PathHistory::default()
    };
    if head.is_empty() {
        return Ok(h);
    }
    let raw = crate::coderepo::git_raw(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "log",
            "--root",
            "--raw",
            "-M",
            "--no-abbrev",
            "--format=%x1e%H%x1f%cI%x1f%ct%x1f%P%x1f%s",
        ],
    )
    .map_err(|e| e.to_string())?;
    parse_path_history(&String::from_utf8_lossy(&raw), &mut h);
    Ok(h)
}

/// Parse the log's records (split on `\x1e`): the first line is
/// `sha\x1fdate\x1funix\x1fparents\x1fsubject`, each
/// later line starting with `:` is a raw entry `:<mode> <mode> <old> <new> <status>\t<path>[\t<path2>]`.
fn parse_path_history(text: &str, h: &mut PathHistory) {
    for rec in text.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let Some(first) = lines.next() else { continue };
        let mut f = first.splitn(5, '\u{1f}');
        let (Some(sha), Some(at), Some(unix), Some(parents), subject) = (
            f.next(),
            f.next(),
            f.next(),
            f.next(),
            f.next().unwrap_or_default(),
        ) else {
            continue;
        };
        let idx = h.commits.len();
        let merge = parents.split_whitespace().count() > 1;
        h.merges += usize::from(merge);
        h.commits.push(CommitMeta {
            sha: sha.to_string(),
            at: at.to_string(),
            subject: subject.to_string(),
            unix: unix.parse().unwrap_or_default(),
            merge,
        });
        for line in lines.filter(|l| l.starts_with(':')) {
            let mut cols = line.split('\t');
            let Some(meta) = cols.next() else { continue };
            let mut m = meta.split(' ');
            let new = m.nth(3).unwrap_or_default();
            let status = m.next().unwrap_or_default();
            let Some(path) = cols.next() else { continue };
            let mut touch = |path: &str, blob: &str, from: Option<String>| {
                h.touches.entry(path.to_string()).or_default().push(Touch {
                    commit: idx,
                    blob: blob.to_string(),
                    from,
                });
            };
            match status.chars().next() {
                Some('D') => touch(path, "", None),
                Some('R') => {
                    if let Some(to) = cols.next() {
                        touch(path, "", None);
                        touch(to, new, Some(path.to_string()));
                    }
                }
                Some('C') => {
                    if let Some(to) = cols.next() {
                        touch(to, new, None);
                    }
                }
                Some('A' | 'M' | 'T') => touch(path, new, None),
                _ => {}
            }
        }
    }
}

/// `file` relative to the root of the repository holding `dir`, `/`-joined.
pub fn rel_path(dir: &Path, file: &Path) -> Option<String> {
    let repo = Repository::discover(dir).ok()?;
    let workdir = resolved(repo.workdir()?)?;
    let full = resolved_file(file)?;
    Some(slash_joined(full.strip_prefix(&workdir).ok()?))
}

/// The paths with uncommitted changes (modified, staged, deleted or untracked) in the repository
/// holding `dir`, relative to its root. An error gives none.
pub fn dirty_paths(dir: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Some(root) = Repository::discover(dir)
        .ok()
        .and_then(|r| r.workdir().map(Path::to_path_buf))
    else {
        return out;
    };
    let Ok(raw) = crate::coderepo::git_raw(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) else {
        return out;
    };
    let text = String::from_utf8_lossy(&raw);
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    while let Some(e) = entries.next() {
        if e.len() < 4 {
            continue;
        }
        out.insert(e[3..].to_string());
        // A rename or copy lists its origin as the next entry.
        if (matches!(e.as_bytes()[0], b'R' | b'C') || matches!(e.as_bytes()[1], b'R' | b'C'))
            && let Some(from) = entries.next()
        {
            out.insert(from.to_string());
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
    let workdir = resolved(repo.workdir()?)?;
    let full = resolved_file(path)?;
    let rel = std::path::PathBuf::from(slash_joined(full.strip_prefix(&workdir).ok()?));

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
    let then = baseline.and_then(|c| c.tree().ok()?.get_path(&rel).ok().map(|entry| entry.id()));
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

    /// DEF-053 (and FEAT-136): a remote becomes the address a reader opens; a local path does not.
    #[test]
    fn a_remote_maps_to_its_web_address() {
        assert_eq!(
            web_url("git@git.example.invalid:o/r.git").as_deref(),
            Some("https://git.example.invalid/o/r")
        );
        assert_eq!(
            web_url("ssh://git@git.example.invalid/o/r.git").as_deref(),
            Some("https://git.example.invalid/o/r")
        );
        assert_eq!(
            web_url("https://git.example.invalid/o/r.git/").as_deref(),
            Some("https://git.example.invalid/o/r")
        );
        assert_eq!(web_url("/srv/boards/shop.git"), None);

        let dir = temp("remoteurl");
        ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        assert_eq!(remote_url(&dir), None);
        add_remote(&dir, "other", "https://example.invalid/a/other.git").unwrap();
        add_remote(&dir, "origin", "https://example.invalid/a/main.git").unwrap();
        assert_eq!(
            remote_url(&dir).as_deref(),
            Some("https://example.invalid/a/main.git")
        );
        let _ = std::fs::remove_dir_all(dir);
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
    /// back is listed once, where it was last; a missing path gives nothing, a missing repository
    /// an error (DEF-080).
    #[test]
    fn file_versions_lists_each_distinct_content_newest_first() {
        let dir = temp("fileversions");
        let repo = Repository::init(&dir).unwrap();
        for (i, text) in ["a", "b", "b", "a", "c"].iter().enumerate() {
            std::fs::write(dir.join("f.txt"), text).unwrap();
            commit_at(&repo, &["f.txt"], &[], T0 + i as i64);
        }
        assert_eq!(file_versions(&dir, "f.txt").unwrap(), ["c", "a", "b"]);
        assert!(file_versions(&dir, "nope.txt").unwrap().is_empty());
        assert!(file_versions(&temp("fileversions-none"), "f.txt").is_err());
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
        let h = file_history(&dir, "f.txt").unwrap();
        let got: Vec<(&str, &str)> = h
            .iter()
            .map(|v| (v.content.as_str(), v.message.trim()))
            .collect();
        assert_eq!(got, [("c", "three"), ("b", "two"), ("a", "one")]);
        assert!(h.iter().all(|v| v.sha.len() == 40));
        assert!(file_history(&dir, "nope.txt").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-080 R-1: a folder that is no repository is named, and so is the reason.
    #[test]
    fn file_history_names_the_folder_and_says_it_is_not_a_repository() {
        let dir = temp("history-notrepo");
        assert!(dir.starts_with(std::env::temp_dir()));
        for e in [
            file_history(&dir, "f.txt").unwrap_err(),
            file_versions(&dir, "f.txt").unwrap_err(),
        ] {
            assert_eq!(e.repo, dir);
            assert_eq!(e.reason, HistoryReason::NotARepository);
            let text = e.to_string();
            assert!(text.contains("is not a git repository"), "{text}");
            assert!(text.contains(&dir.display().to_string()), "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-080 R-1: git's owner check reads as another user's repository, with the fix.
    #[test]
    fn an_owner_refusal_is_named_as_another_users_repository() {
        let e = git2::Error::new(git2::ErrorCode::Owner, git2::ErrorClass::Config, "x");
        assert_eq!(reason_of(&e), HistoryReason::OwnedByAnotherUser);
        let err = HistoryError {
            repo: PathBuf::from("/srv/board"),
            reason: reason_of(&e),
        };
        let text = err.to_string();
        assert!(text.contains("safe.directory"), "{text}");
        assert!(text.contains("/srv/board"), "{text}");
        assert!(err.reason_text().contains("owned by another user"));
    }

    /// DEF-080 R-1 (L-29): a repository with no commits has an empty history, not an error.
    #[test]
    fn a_repository_with_no_commits_has_an_empty_history() {
        let dir = temp("history-unborn");
        Repository::init(&dir).unwrap();
        assert_eq!(file_history(&dir, "f.txt"), Ok(vec![]));
        assert_eq!(file_versions(&dir, "f.txt"), Ok(vec![]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-080 R-1: a commit the walk cannot read makes the history incomplete, not shorter.
    #[test]
    fn a_walk_that_cannot_read_a_commit_is_reported_incomplete() {
        let dir = temp("history-incomplete");
        let repo = Repository::init(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "a").unwrap();
        commit_at(&repo, &["f.txt"], &[], T0);
        let first = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        std::fs::write(dir.join("f.txt"), "b").unwrap();
        commit_at(&repo, &["f.txt"], &[], T0 + 1);
        let loose = dir.join(".git/objects").join(&first[..2]).join(&first[2..]);
        std::fs::remove_file(&loose).unwrap();
        assert!(
            matches!(
                file_history(&dir, "f.txt"),
                Err(HistoryError {
                    reason: HistoryReason::Incomplete(_),
                    ..
                })
            ),
            "{:?}",
            file_history(&dir, "f.txt")
        );
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

    /// MNT-007 R-8: a file two levels into the workdir is named relative to it, `/`-joined, and
    /// neither resolved side carries Windows' verbatim prefix (`\\?\`), which would make every
    /// board doc look outside its repository.
    #[test]
    fn a_path_inside_the_workdir_is_relative_and_slash_joined_without_a_verbatim_prefix() {
        let dir = temp("slash-joined");
        let repo = Repository::init(&dir).unwrap();
        std::fs::create_dir_all(dir.join("a")).unwrap();
        let file = dir.join("a").join("b.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let workdir = resolved(repo.workdir().unwrap()).unwrap();
        let full = resolved_file(&file).unwrap();
        for p in [&workdir, &full] {
            assert!(!p.to_string_lossy().starts_with(r"\\?\"), "{}", p.display());
        }
        assert_eq!(slash_joined(full.strip_prefix(&workdir).unwrap()), "a/b.rs");
        // On every system: the prefix a Windows canonicalize adds is what `resolved` strips.
        assert_eq!(
            crate::project::without_verbatim_prefix(PathBuf::from(r"\\?\C:\w\a\b.rs")),
            PathBuf::from(r"C:\w\a\b.rs")
        );
        #[cfg(windows)]
        assert_eq!(slash_joined(Path::new(r"a\b.rs")), "a/b.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-059 R-9: kanbanr appends its lines to the board's `.gitignore` and never rewrites,
    /// drops or reorders a line someone else put there; a second run changes nothing.
    #[test]
    fn the_board_gitignore_gains_lines_and_never_loses_or_reorders_one() {
        let dir = temp("gitignore-append");
        let repo = Repository::init(&dir).unwrap();
        let path = dir.join(".gitignore");
        std::fs::write(&path, "b\n# mine\na").unwrap();

        ensure_secret_ignored(&repo, &dir);
        let first = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = first.lines().collect();
        assert_eq!(lines[..3], ["b", "# mine", "a"], "{first}");
        let ours = [
            SECRET_FILE,
            WRITE_LOCK_FILE,
            UNPUSHED_FILE,
            crate::eventing::CONFIG_FILE,
            SESSIONS_DIR,
            "*.tmp",
        ];
        assert_eq!(
            lines[3..],
            ours,
            "appended once each, after the others: {first}"
        );
        assert!(first.ends_with('\n'), "{first:?}");

        ensure_secret_ignored(&repo, &dir);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            first,
            "a second run changes nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- DEF-106: the shared path history ----

    /// Run the real `git` in `dir` with a fixed identity and, when given, a committer date.
    fn g(dir: &Path, date: Option<&str>, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir);
        if let Some(d) = date {
            cmd.env("GIT_COMMITTER_DATE", d).env("GIT_AUTHOR_DATE", d);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A commit of `files` (path, text) on the day `n` of January 2026.
    fn put(dir: &Path, n: u32, msg: &str, files: &[(&str, &str)]) {
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        g(dir, None, &["add", "-A"]);
        g(
            dir,
            Some(&format!("2026-01-{n:02}T10:00:00+00:00")),
            &["commit", "-q", "-m", msg],
        );
    }

    /// A repository with edits, a rename, a delete and a merge.
    fn fixture(name: &str) -> PathBuf {
        let dir = temp(name);
        g(&dir, None, &["init", "-q"]);
        put(
            &dir,
            1,
            "add a and c",
            &[("docs/a.md", "one\ntwo\nthree\nfour\n"), ("c.txt", "c1\n")],
        );
        put(
            &dir,
            2,
            "edit a",
            &[("docs/a.md", "one\ntwo\nthree\nfour\nfive\n")],
        );
        g(&dir, None, &["checkout", "-q", "-b", "side"]);
        put(&dir, 3, "side adds s", &[("s.txt", "s\n")]);
        g(&dir, None, &["checkout", "-q", "main"]);
        put(&dir, 4, "edit c", &[("c.txt", "c2\n")]);
        g(
            &dir,
            Some("2026-01-05T10:00:00+00:00"),
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        );
        g(&dir, None, &["mv", "docs/a.md", "docs/b.md"]);
        g(
            &dir,
            Some("2026-01-06T10:00:00+00:00"),
            &["commit", "-q", "-m", "rename a to b"],
        );
        put(
            &dir,
            7,
            "edit b",
            &[("docs/b.md", "one\ntwo\nthree\nfour\nfive\nsix\n")],
        );
        put(&dir, 8, "add d", &[("d.txt", "d\n")]);
        g(&dir, None, &["rm", "-q", "d.txt"]);
        g(
            &dir,
            Some("2026-01-09T10:00:00+00:00"),
            &["commit", "-q", "-m", "drop d"],
        );
        dir
    }

    /// DEF-106 R-6: the newest change over any of some paths is what `git log -1` says.
    #[test]
    fn path_history_last_change_matches_git_log_one() {
        let dir = fixture("ph-last");
        let h = path_history(&dir).unwrap();
        for rels in [
            vec!["docs/b.md"],
            vec!["c.txt"],
            vec!["s.txt"],
            vec!["docs/a.md"],
            vec!["d.txt"],
            // Not ["c.txt", "s.txt"]: git shows the merge itself for paths changed on both of its
            // sides, and the walk leaves merges out by design (a board's history is linear).
            vec!["c.txt", "d.txt"],
            vec!["c.txt", "docs/b.md"],
        ] {
            let mut args = vec!["log", "-1", "--format=%cI", "--"];
            args.extend(rels.iter().copied());
            let want = g(&dir, None, &args);
            let got = h
                .last_change(&rels)
                .map(|c| c.at.clone())
                .unwrap_or_default();
            assert_eq!(got, want, "{rels:?}");
        }
        assert!(h.last_change(&["never.md"]).is_none());
        assert!(h.last_change(&[]).is_none());
        assert_eq!(h.head(), g(&dir, None, &["rev-parse", "HEAD"]));
        assert_eq!(
            rel_path(&dir, &dir.join("docs").join("b.md")).as_deref(),
            Some("docs/b.md")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-106 R-6: a path's versions run back through its renames as `git log --follow` does.
    #[test]
    fn path_history_versions_follow_renames_as_git_log_follow() {
        let dir = fixture("ph-follow");
        let h = path_history(&dir).unwrap();
        let want: Vec<String> = g(
            &dir,
            None,
            &["log", "--follow", "--format=%H", "--", "docs/b.md"],
        )
        .lines()
        .map(str::to_string)
        .collect();
        let got = h.versions("docs/b.md");
        assert_eq!(got.iter().map(|v| v.sha.clone()).collect::<Vec<_>>(), want);
        assert_eq!(want.len(), 4, "edit b, rename, edit a, add: {want:?}");
        assert_eq!(got[0].path, "docs/b.md");
        assert_eq!(
            got[1].path, "docs/b.md",
            "the rename commit lands on the new name"
        );
        assert_eq!(got[2].path, "docs/a.md");
        assert_eq!(got[3].subject, "add a and c");
        // The blob of each version is what the commit holds, readable in-process.
        let text = h.blob_text(&dir, &got[0].blob).unwrap();
        assert!(text.ends_with("six\n"), "{text}");
        assert!(h.blob_text(&dir, &got[3].blob).unwrap().starts_with("one"));
        assert!(
            h.message(&dir, &got[3].sha)
                .unwrap()
                .starts_with("add a and c")
        );
        // A deletion is an entry without a blob.
        let d = h.versions("d.txt");
        assert_eq!(d.len(), 2);
        assert!(d[0].blob.is_empty() && !d[1].blob.is_empty());
        assert!(h.versions("never.md").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-106 R-5: any number of paths asked of one revision is one walk.
    #[test]
    fn one_revision_walks_history_once_for_every_path_asked() {
        let dir = fixture("ph-once");
        let before = path_history_walks_here();
        let total = path_history_walks();
        for _ in 0..5 {
            let h = path_history(&dir).unwrap();
            for rel in ["docs/b.md", "c.txt", "s.txt", "d.txt"] {
                let _ = h.last_change(&[rel]);
                let _ = h.versions(rel);
            }
        }
        assert_eq!(path_history_walks_here() - before, 1);
        assert!(path_history_walks() > total);
        // A second thread asking at the same moment shares the answer.
        let d2 = dir.clone();
        let other = std::thread::spawn(move || path_history(&d2).unwrap().head().to_string())
            .join()
            .unwrap();
        assert_eq!(other, path_history(&dir).unwrap().head());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-106 R-5: a commit moves HEAD, and the next ask sees it. Since MNT-014 it grows the kept
    /// walk by the new commit rather than walking again.
    #[test]
    fn a_new_head_walks_again() {
        let dir = fixture("ph-new");
        let first = path_history(&dir).unwrap();
        let before = path_history_walks_here();
        let extends = path_history_extends_of(&dir);
        put(&dir, 10, "add e", &[("e.txt", "e\n")]);
        let second = path_history(&dir).unwrap();
        assert_eq!(path_history_walks_here() - before, 0);
        assert_eq!(path_history_extends_of(&dir) - extends, 1);
        assert_ne!(first.head(), second.head());
        assert!(first.last_change(&["e.txt"]).is_none());
        assert_eq!(second.last_change(&["e.txt"]).unwrap().subject, "add e");
        // And an unchanged HEAD does not.
        let _ = path_history(&dir).unwrap();
        assert_eq!(path_history_extends_of(&dir) - extends, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: a path's exact touches, newest first, without following renames; a rename away
    /// is a delete, a rename in carries the blob.
    #[test]
    fn touches_list_exact_path_changes_newest_first() {
        let dir = fixture("ph-touches");
        let h = path_history(&dir).unwrap();
        let a = h.touches("docs/a.md");
        let subjects: Vec<&str> = a.iter().map(|t| t.commit.subject.as_str()).collect();
        assert_eq!(subjects, ["rename a to b", "edit a", "add a and c"]);
        assert!(a[0].blob.is_none(), "a rename away is a delete");
        assert!(a[1].blob.is_some() && a[2].blob.is_some());
        assert!(a[0].index < a[1].index && a[1].index < a[2].index);
        assert_eq!(h.commits[a[1].index].sha, a[1].commit.sha);
        let b = h.touches("docs/b.md");
        let subjects: Vec<&str> = b.iter().map(|t| t.commit.subject.as_str()).collect();
        assert_eq!(subjects, ["edit b", "rename a to b"]);
        assert_eq!(
            b[1].blob.unwrap(),
            g(
                &dir,
                None,
                &["rev-parse", &format!("{}:docs/b.md", b[1].commit.sha)]
            )
        );
        let d = h.touches("d.txt");
        assert_eq!(d.len(), 2);
        assert!(d[0].blob.is_none() && d[1].blob.is_some());
        assert!(h.touches("never.md").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: `blob_at` is the blob in a commit's tree, and `baseline` finds the commit
    /// `DocHistory` judges against.
    #[test]
    fn blob_at_gives_the_tree_state_at_a_commit() {
        let dir = temp("ph-blobat");
        g(&dir, None, &["init", "-q"]);
        put(&dir, 1, "add", &[("a.md", "1\n"), ("b.md", "b\n")]);
        put(&dir, 2, "edit a", &[("a.md", "2\n")]);
        put(&dir, 3, "other", &[("c.md", "c\n")]);
        g(&dir, None, &["rm", "-q", "a.md"]);
        g(
            &dir,
            Some("2026-01-04T10:00:00+00:00"),
            &["commit", "-q", "-m", "drop a"],
        );
        put(&dir, 5, "re-add a", &[("a.md", "2\n")]);
        let h = path_history(&dir).unwrap();
        assert!(h.is_linear());
        assert_eq!(h.commits.len(), 5);
        for (i, c) in h.commits.iter().enumerate() {
            for rel in ["a.md", "b.md", "c.md", "never.md"] {
                let want = std::process::Command::new("git")
                    .args(["rev-parse", "-q", "--verify", &format!("{}:{rel}", c.sha)])
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                let want = want
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&want.stdout).trim().to_string());
                assert_eq!(h.blob_at(rel, i).map(str::to_string), want, "{rel} at {i}");
            }
        }
        // baseline: the first commit, in walk order, at or before the time.
        let last = h.commits[0].unix;
        assert_eq!(h.baseline(last), Some(0));
        assert_eq!(h.baseline(last + 1_000_000), Some(0));
        assert_eq!(h.baseline(h.commits[4].unix), Some(4));
        assert_eq!(h.baseline(h.commits[4].unix - 1), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: recording merges leaves `last_change` and `versions` as they were, and says
    /// the history is not linear.
    #[test]
    fn path_history_versions_ignore_merge_records() {
        let dir = fixture("ph-merges");
        let h = path_history(&dir).unwrap();
        assert!(!h.is_linear());
        let merge = h.commits.iter().find(|c| c.merge).unwrap();
        assert_eq!(merge.subject, "merge side");
        assert!(h.commits.iter().filter(|c| c.merge).count() == 1);
        assert!(
            h.touches
                .values()
                .flatten()
                .all(|t| !h.commits[t.commit].merge)
        );
        for rel in ["c.txt", "s.txt", "docs/b.md", "d.txt"] {
            let want: Vec<String> = g(
                &dir,
                None,
                &["log", "--no-merges", "--follow", "--format=%H", "--", rel],
            )
            .lines()
            .map(str::to_string)
            .collect();
            let got: Vec<String> = h.versions(rel).into_iter().map(|v| v.sha).collect();
            assert_eq!(got, want, "{rel}");
        }
        assert_eq!(
            h.last_change(&["c.txt"]).unwrap().at,
            g(
                &dir,
                None,
                &["log", "-1", "--no-merges", "--format=%cI", "--", "c.txt"]
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A linear repository with an add, edits, a mode-only change, a delete and a re-add of the
    /// same text, a rename away from a path and into another, a non-UTF-8 version, and commits in
    /// one second (MNT-014).
    fn shared_fixture(name: &str) -> PathBuf {
        let dir = temp(name);
        g(&dir, None, &["init", "-q"]);
        let one = "l1\nl2\nl3\nl4\nl5\nl6\n";
        let two = "l1\nl2\nl3\nl4\nl5\nl6\nl7\n";
        let old = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n";
        put(&dir, 1, "add", &[("f.md", one), ("old.md", old)]);
        put(&dir, 2, "edit f", &[("f.md", two)]);
        g(&dir, None, &["update-index", "--chmod=+x", "f.md"]);
        g(
            &dir,
            Some("2026-01-03T10:00:00+00:00"),
            &["commit", "-q", "-m", "mode only"],
        );
        put(&dir, 3, "edit f back", &[("f.md", one)]);
        g(&dir, None, &["rm", "-q", "f.md"]);
        g(
            &dir,
            Some("2026-01-04T10:00:00+00:00"),
            &["commit", "-q", "-m", "drop f"],
        );
        put(&dir, 5, "re-add f", &[("f.md", two)]);
        g(&dir, None, &["mv", "old.md", "f2.md"]);
        g(
            &dir,
            Some("2026-01-06T10:00:00+00:00"),
            &["commit", "-q", "-m", "rename old to f2"],
        );
        std::fs::write(
            dir.join("f2.md"),
            b"alpha\nbeta\ngamma\ndelta\nepsilon\n\xff\xfe\n",
        )
        .unwrap();
        g(&dir, None, &["add", "-A"]);
        g(
            &dir,
            Some("2026-01-07T10:00:00+00:00"),
            &["commit", "-q", "-m", "binary f2"],
        );
        put(
            &dir,
            7,
            "text f2 same second",
            &[("f2.md", "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\n")],
        );
        dir
    }

    /// MNT-014 R-1: any number of history questions at one HEAD are one walk.
    #[test]
    fn one_read_walks_history_once() {
        let dir = shared_fixture("ph-oneread");
        let before = path_history_walks_of(&dir);
        let t = "2026-01-04T12:00:00Z";
        let mut stats = Vec::new();
        for _ in 0..3 {
            let history = DocHistory::default();
            for i in 0..20 {
                let doc = dir.join(if i % 2 == 0 { "f.md" } else { "f2.md" });
                assert!(history.changed_since(&doc, t).is_some());
            }
            stats.push(history.stats());
        }
        for _ in 0..5 {
            assert!(!file_history(&dir, "f.md").unwrap().is_empty());
        }
        for _ in 0..2 {
            assert!(!file_versions(&dir, "f2.md").unwrap().is_empty());
        }
        assert_eq!(path_history_walks_of(&dir) - before, 1);
        assert_eq!(stats[0], HistoryStats { opens: 1, walks: 1 });
        assert_eq!(stats[1], HistoryStats { opens: 1, walks: 0 });
        assert_eq!(stats[2], HistoryStats { opens: 1, walks: 0 });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every field of two histories, for the extension tests.
    fn same_history(a: &PathHistory, b: &PathHistory) {
        assert_eq!(a.head, b.head);
        assert_eq!(a.commits, b.commits);
        assert_eq!(a.merges, b.merges);
        assert_eq!(a.touches, b.touches);
    }

    /// MNT-014 R-1: a new commit grows the kept walk instead of walking again.
    #[test]
    fn a_new_commit_extends_the_walk() {
        let dir = shared_fixture("ph-extend");
        let first = path_history(&dir).unwrap();
        let walks = path_history_walks_of(&dir);
        let extends = path_history_extends_of(&dir);
        put(&dir, 20, "new f", &[("f.md", "x1\nx2\nx3\nx4\nx5\nx6\n")]);
        put(
            &dir,
            21,
            "new g",
            &[("g.md", "g\n"), ("f2.md", "changed\n")],
        );
        let grown = path_history(&dir).unwrap();
        assert_eq!(path_history_walks_of(&dir), walks, "no full walk");
        assert_eq!(path_history_extends_of(&dir) - extends, 1);
        assert_eq!(grown.commits.len(), first.commits.len() + 2);
        // The extended history is a fresh full walk, field by field.
        let fresh = walk_path_history(
            Repository::discover(&dir).unwrap().workdir().unwrap(),
            &grown.head,
        )
        .unwrap();
        same_history(&grown, &fresh);
        // And it answers.
        assert_eq!(grown.touches("g.md").len(), 1);
        assert_eq!(grown.touches("g.md")[0].index, 0);
        // The same HEAD again is a hit.
        let again = path_history(&dir).unwrap();
        assert!(std::sync::Arc::ptr_eq(&grown, &again));
        assert_eq!(path_history_extends_of(&dir) - extends, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-1: a HEAD that is not a descendant of the kept one walks in full.
    #[test]
    fn a_rewritten_head_walks_in_full() {
        let dir = shared_fixture("ph-rewrite");
        put(&dir, 20, "extra one", &[("e1.md", "1\n")]);
        put(&dir, 21, "extra two", &[("e2.md", "2\n")]);
        let _ = path_history(&dir).unwrap();
        let walks = path_history_walks_of(&dir);
        let extends = path_history_extends_of(&dir);
        g(&dir, None, &["reset", "-q", "--hard", "HEAD~2"]);
        put(&dir, 22, "other", &[("o.md", "o\n")]);
        let rewritten = path_history(&dir).unwrap();
        assert_eq!(path_history_walks_of(&dir) - walks, 1, "a full walk");
        assert_eq!(path_history_extends_of(&dir), extends);
        let fresh = walk_path_history(
            Repository::discover(&dir).unwrap().workdir().unwrap(),
            &rewritten.head,
        )
        .unwrap();
        same_history(&rewritten, &fresh);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: `file_history` from the shared walk is what the tree walk says.
    #[test]
    fn shared_walk_gives_the_answers_of_file_history() {
        let dir = shared_fixture("ph-fh");
        assert!(path_history(&dir).unwrap().is_linear());
        for rel in ["f.md", "old.md", "f2.md", "never.md"] {
            let got = file_history(&dir, rel).unwrap();
            assert_eq!(got, file_history_walk(&dir, rel).unwrap(), "{rel}");
            if rel == "f.md" {
                // add, edit, edit back, re-add; the mode-only change is not a version.
                assert_eq!(got.len(), 4, "{got:?}");
            }
        }
        // A folder that is no repository, and a repository with no commit.
        assert_eq!(
            file_history(&temp("ph-fh-none"), "f.md")
                .unwrap_err()
                .reason,
            HistoryReason::NotARepository
        );
        let empty = temp("ph-fh-empty");
        g(&empty, None, &["init", "-q"]);
        assert!(file_history(&empty, "f.md").unwrap().is_empty());
        assert!(file_versions(&empty, "f.md").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(empty);
    }

    /// MNT-014 R-2: `file_versions` from the shared walk is what the tree walk says.
    #[test]
    fn shared_walk_gives_the_answers_of_file_versions() {
        let dir = shared_fixture("ph-fv");
        for rel in ["f.md", "old.md", "f2.md", "never.md"] {
            assert_eq!(
                file_versions(&dir, rel).unwrap(),
                file_versions_walk(&dir, rel).unwrap(),
                "{rel}"
            );
        }
        // One text for the edit back and the first add.
        assert_eq!(file_versions(&dir, "f.md").unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: `DocHistory` over the shared walk answers as the single walk does, at every
    /// commit's time and a second either side of it.
    #[test]
    fn doc_history_from_the_shared_walk_matches_the_single_walk() {
        let dir = shared_fixture("ph-dh");
        std::fs::write(dir.join("f.md"), "edited now").unwrap();
        let times: Vec<i64> = path_history(&dir)
            .unwrap()
            .commits
            .iter()
            .flat_map(|c| [c.unix - 1, c.unix, c.unix + 1])
            .collect();
        let history = DocHistory::default();
        let mut answers = Vec::new();
        for unix in times {
            let at = time::OffsetDateTime::from_unix_timestamp(unix)
                .unwrap()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap();
            for rel in ["f.md", "old.md", "f2.md", "never.md"] {
                let doc = dir.join(rel);
                let want = doc_changed_since_single_walk(&doc, &at);
                assert_eq!(history.changed_since(&doc, &at), want, "{rel} at {at}");
                answers.push(want);
            }
        }
        assert!(history.is_shared());
        assert!(answers.contains(&Some(true)) && answers.contains(&Some(false)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-014 R-2: a history with a merge is answered by the tree walks, with the same answers.
    #[test]
    fn a_history_with_merges_falls_back_to_the_full_walk() {
        let dir = fixture("ph-fallback");
        assert!(!path_history(&dir).unwrap().is_linear());
        for rel in ["docs/b.md", "c.txt", "s.txt", "d.txt"] {
            assert_eq!(
                file_history(&dir, rel).unwrap(),
                file_history_walk(&dir, rel).unwrap(),
                "{rel}"
            );
            assert_eq!(
                file_versions(&dir, rel).unwrap(),
                file_versions_walk(&dir, rel).unwrap(),
                "{rel}"
            );
        }
        let history = DocHistory::default();
        for unix in [1_767_000_000, 1_768_000_000, 1_769_000_000, 1_770_000_000] {
            let at = time::OffsetDateTime::from_unix_timestamp(unix)
                .unwrap()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap();
            for rel in ["docs/b.md", "c.txt", "s.txt", "d.txt"] {
                let doc = dir.join(rel);
                assert_eq!(
                    history.changed_since(&doc, &at),
                    doc_changed_since_single_walk(&doc, &at),
                    "{rel} at {at}"
                );
            }
        }
        assert!(!history.is_shared());
        assert_eq!(history.stats(), HistoryStats { opens: 1, walks: 1 });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An empty repository has an empty history, and a changed file shows as dirty.
    #[test]
    fn path_history_of_an_empty_repository_is_empty_and_dirty_paths_lists_edits() {
        let dir = temp("ph-empty");
        g(&dir, None, &["init", "-q"]);
        let h = path_history(&dir).unwrap();
        assert!(h.head().is_empty() && h.last_change(&["x"]).is_none());
        put(&dir, 1, "x", &[("x.txt", "1")]);
        assert!(dirty_paths(&dir).is_empty());
        std::fs::write(dir.join("x.txt"), "2").unwrap();
        std::fs::write(dir.join("new file.txt"), "n").unwrap();
        let dirty = dirty_paths(&dir);
        assert!(
            dirty.contains("x.txt") && dirty.contains("new file.txt"),
            "{dirty:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// DEF-102 T2: the commit that waits, reports and can be undone.
#[cfg(test)]
mod commit_write_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn board(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanbanr-cw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ensure_repo_as(&dir, Some(("Ada Lovelace", "ada@example.com")));
        let repo = Repository::open(&dir).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Ada Lovelace").unwrap();
        cfg.set_str("user.email", "ada@example.com").unwrap();
        std::fs::write(dir.join("kept.txt"), "kept\n").unwrap();
        assert!(commit_local(&dir, "seed"));
        dir
    }

    fn commits(dir: &Path) -> usize {
        let repo = Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.count()
    }

    fn head_has(dir: &Path, rel: &str) -> bool {
        let repo = Repository::open(dir).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        tree.get_path(Path::new(rel)).is_ok()
    }

    /// DEF-102 R-1: git holds `index.lock` for a moment; the commit waits it out and lands once.
    #[test]
    fn commit_write_waits_out_a_held_index_lock() {
        let dir = board("waits");
        let before = commits(&dir);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        let lock = git_dir(&dir).join("index.lock");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .unwrap();
        let holder = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                std::fs::remove_file(lock).unwrap();
            })
        };
        let started = Instant::now();
        let oid = commit_write_within(&dir, "add a", Duration::from_secs(5))
            .expect("the commit waits and lands");
        holder.join().unwrap();
        assert!(oid.is_some());
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert_eq!(commits(&dir), before + 1, "exactly one new commit");
        assert!(head_has(&dir, "a.txt"));
        assert!(changed_paths(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-1: HEAD moves between reading the parent and updating the ref; the commit reads
    /// HEAD again and lands on the new tip with both changes.
    #[test]
    fn commit_write_retries_when_head_moved_under_it() {
        let dir = board("moved");
        std::fs::write(dir.join("mine.txt"), "mine\n").unwrap();
        let other = dir.clone();
        BEFORE_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                // A non-kanbanr git commits a file of its own on the same branch.
                let repo = Repository::open(&other).unwrap();
                let parent = repo.head().unwrap().peel_to_commit().unwrap();
                std::fs::write(other.join("theirs.txt"), "theirs\n").unwrap();
                let blob = repo.blob(b"theirs\n").unwrap();
                let mut builder = repo.treebuilder(Some(&parent.tree().unwrap())).unwrap();
                builder.insert("theirs.txt", blob, 0o100_644).unwrap();
                let tree = repo.find_tree(builder.write().unwrap()).unwrap();
                let sig = Signature::now("Other", "o@example.com").unwrap();
                repo.commit(Some("HEAD"), &sig, &sig, "theirs", &tree, &[&parent])
                    .unwrap();
            }));
        });
        let oid = commit_write_within(&dir, "add mine", Duration::from_secs(5))
            .expect("the commit retries on the new HEAD");
        assert!(oid.is_some());
        assert!(head_has(&dir, "mine.txt"));
        assert!(head_has(&dir, "theirs.txt"));
        assert!(changed_paths(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-2: with the lock held throughout, the commit gives up after its wait and the
    /// error names the lock file.
    #[test]
    fn commit_write_gives_up_after_its_wait_and_names_the_lock() {
        let dir = board("gives-up");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        let lock = git_dir(&dir).join("index.lock");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .unwrap();
        let before = commits(&dir);
        let started = Instant::now();
        let err = commit_write_within(&dir, "add a", Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(200));
        match &err {
            CommitError::Locked { file, waited } => {
                assert!(file.ends_with("index.lock"), "{file:?}");
                assert!(*waited >= Duration::from_millis(200));
            }
            other => panic!("expected Locked, got {other:?}"),
        }
        assert!(err.to_string().contains("index.lock"));
        assert_eq!(commits(&dir), before, "no commit was made");
        std::fs::remove_file(&lock).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-2: a modified tracked file, a new file and a staged change are each put back, and
    /// the index matches HEAD.
    #[test]
    fn restore_to_head_puts_back_changed_and_removes_new_files() {
        let dir = board("restore");
        std::fs::write(dir.join("kept.txt"), "changed\n").unwrap();
        std::fs::create_dir_all(dir.join("fresh")).unwrap();
        std::fs::write(dir.join("fresh").join("new.txt"), "new\n").unwrap();
        std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
        {
            let repo = Repository::open(&dir).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        let mut paths = changed_paths(&dir);
        paths.sort();
        assert_eq!(paths, ["fresh/new.txt", "kept.txt", "staged.txt"]);
        restore_to_head(&dir, &paths).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("kept.txt")).unwrap(),
            "kept\n"
        );
        assert!(!dir.join("fresh").join("new.txt").exists());
        assert!(
            !dir.join("fresh").exists(),
            "the folder kanbanr made goes too"
        );
        assert!(!dir.join("staged.txt").exists());
        assert!(changed_paths(&dir).is_empty());
        let repo = Repository::open(&dir).unwrap();
        assert!(
            repo.index()
                .unwrap()
                .get_path(Path::new("staged.txt"), 0)
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
