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
        let _ = std::fs::write(&gitignore, contents);
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

/// Set the repo-local commit identity (used by local mode: `kanbanr identity --name --email`).
pub fn set_identity(dir: &Path, name: &str, email: &str) -> Result<(), String> {
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
    let mut outcome = SyncOutcome::default();
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
}
