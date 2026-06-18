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

/// The repo-local fallback identity (set in `ensure_repo`); used for merge commits and local-mode
/// commits when no explicit identity is given.
fn fallback_signature(repo: &Repository) -> Signature<'static> {
    repo.signature()
        .or_else(|_| Signature::now("kanbanr", "kanbanr@local"))
        .expect("signature")
}

/// Secrets that must NEVER be committed (the data repo is pushed to remotes). `security.yaml`
/// holds every user's auth_key/auth_secret and the JWT signing secret.
const SECRET_FILE: &str = "security.yaml";

/// The CLI's cross-process write lock file (FEAT-029). It lives at the data-dir root but is
/// machine-local state, so it must never be tracked or pushed.
pub const WRITE_LOCK_FILE: &str = ".kanbanr.lock";

/// Ensure the data repo ignores machine-local files (the secrets file and the write lock), and
/// stop tracking the secrets file if a pre-existing repo happened to track it (the file stays on
/// disk; it just leaves the index/history going forward).
fn ensure_secret_ignored(repo: &Repository, dir: &Path) {
    let gitignore = dir.join(".gitignore");
    let mut contents = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let mut changed = false;
    for entry in [SECRET_FILE, WRITE_LOCK_FILE] {
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
    if let Ok(mut index) = repo.index() {
        if index.get_path(Path::new(SECRET_FILE), 0).is_some() {
            let _ = index.remove_path(Path::new(SECRET_FILE));
            let _ = index.write();
        }
    }
}

/// Initialize the data dir as a git repo (and make an initial commit) if it isn't one yet.
pub fn ensure_repo(dir: &Path) {
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
    if fresh {
        // Repo-local fallback identity so merge commits from pulls always have an author.
        if let Ok(mut cfg) = repo.config() {
            let _ = cfg.set_str("user.name", "kanbanr");
            let _ = cfg.set_str("user.email", "kanbanr@local");
        }
    }
    // Always: keep credentials out of the repo (write .gitignore before the first commit so the
    // secret file is never tracked in the first place).
    ensure_secret_ignored(&repo, dir);
    if fresh {
        let _ = (|| -> Result<(), git2::Error> {
            let tree_oid = stage_all(&repo)?;
            let tree = repo.find_tree(tree_oid)?;
            let sig = fallback_signature(&repo);
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

/// The configured commit identity (name, email) from the repo's git config, if any.
pub fn identity(dir: &Path) -> Option<(String, String)> {
    let repo = Repository::open(dir).ok()?;
    let cfg = repo.config().ok()?;
    let name = cfg.get_string("user.name").ok()?;
    let email = cfg.get_string("user.email").ok()?;
    Some((name, email))
}

fn do_commit(repo: &Repository, sig: &Signature, message: &str) -> Result<bool, git2::Error> {
    let tree_oid = stage_all(repo)?;
    let tree = repo.find_tree(tree_oid)?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    // Nothing to commit if the staged tree equals the current HEAD tree.
    if let Some(p) = &parent {
        if p.tree_id() == tree_oid {
            return Ok(false);
        }
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
    let sig = fallback_signature(&repo);
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
        .and_then(|h| h.shorthand().map(String::from))
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
    let sig = fallback_signature(repo);
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
    for name in remotes.iter().flatten() {
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
        .flatten()
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
