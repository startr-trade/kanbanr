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

/// Credentials callback covering the common cases: SSH (agent, then default), and HTTPS via the
/// configured credential helper. Best-effort.
fn credentials_cb(
    git_config: git2::Config,
) -> impl FnMut(&str, Option<&str>, CredentialType) -> Result<Cred, git2::Error> {
    move |url, username_from_url, allowed| {
        if allowed.contains(CredentialType::USERNAME) {
            return Cred::username(username_from_url.unwrap_or("git"));
        }
        if allowed.contains(CredentialType::SSH_KEY) {
            return Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"));
        }
        Cred::credential_helper(&git_config, url, username_from_url).or_else(|_| Cred::default())
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

/// Sync with every configured remote: pull (integrate, assuming no conflicts) then push the
/// current branch. Best-effort and **safe**: the change is already committed locally before this
/// runs, so nothing is ever lost. Returns a warning per remote that could not be pushed (a
/// divergence or conflict), each with the exact git commands to resolve it by hand — kanbanr never
/// auto-resolves. (A failed *pull* on the very first sync is benign: the push then creates the
/// branch; only a failed push is surfaced.)
#[must_use]
pub fn sync_all(dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(repo) = Repository::open(dir) else {
        return warnings;
    };
    let branch = current_branch(&repo);
    let Ok(remotes) = repo.remotes() else {
        return warnings;
    };
    let shown = dir.display();
    // git2 0.21 reports a name that is not UTF-8 as an error, not a missing value; neither is a
    // remote we could name to the user, so both are skipped.
    for name in remotes.iter().filter_map(|n| n.ok().flatten()) {
        let _ = pull(&repo, name, &branch); // best-effort integrate (first-push fetch errors are benign)
        if let Err(e) = push(&repo, name, &branch) {
            warnings.push(format!(
                "remote '{name}': could not push ({}). Your change is committed locally, so nothing \
                 is lost. To sync, resolve in the data folder with normal git:\n    \
                 git -C {shown} pull --no-rebase {name} {branch}\n    \
                 git -C {shown} push {name} {branch}",
                e.message()
            ));
        }
    }
    warnings
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
}
