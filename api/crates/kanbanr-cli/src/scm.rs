//! Git guardrails for the project's own repository (FEAT-056).
//!
//! These run against the **code** repo, not the board's data folder — kanbanr authors that one
//! itself. Everything here shells out to `git` rather than using libgit2, because the point is to
//! behave exactly like the developer's git does: their hooks, their signing key, their config.
//!
//! The guardrails are cheap to obey and cheap to escape *on the record*: `kanbanr commit` fills in
//! the reference so obeying takes no thought, and `[no-ref] <reason>` in a message passes the hook
//! while leaving the reason in git history forever. An escape nobody records just teaches
//! `--no-verify`, which removes the check and the record together.

use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Marker line identifying a hook kanbanr wrote, so an update never clobbers someone else's.
const MARKER: &str = "# kanbanr hook (FEAT-056)";
const HOOKS: [(&str, &str); 2] = [
    ("commit-msg", "check-msg \"$1\""),
    ("pre-commit", "check-branch"),
];

pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| anyhow!("could not run git: {e}"))?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The working tree root of the repo the current directory is in.
pub fn repo_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    git(&cwd, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

/// The checked-out branch, or `None` when the head is detached (a rebase, a bisect) — states where
/// a branch rule would only get in the way.
pub fn current_branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}

/// Is a merge waiting to be committed? (FEAT-094)
///
/// A merge into the default branch is the normal, intended end of every item's branch, so the
/// "no commits on the default branch" rule must not apply to the commit that completes one. It
/// already didn't for a one-command `git merge -m …`, which makes its own commit without calling
/// `git commit` — only the recovery path, finishing a merge whose message a hook rejected, was
/// refused, which is the worst arrangement of the two. Asked of git rather than by looking for a
/// `MERGE_HEAD` file, so it holds in a linked worktree too.
pub fn merge_in_progress(root: &Path) -> bool {
    git(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

/// Does the repository have a commit yet? A fresh `git init` has an unborn HEAD: a branch name
/// with nothing behind it, from which no item branch can be made.
pub fn has_commits(root: &Path) -> bool {
    git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok()
}

/// The branch work is *not* supposed to land on directly: in a repository with no commits, the
/// branch HEAD is waiting on (FEAT-105) — whatever `git init` chose, which is what the first commit
/// will create; else the remote's head, else the first of the configured default, main and master
/// that exists here.
pub fn default_branch(root: &Path) -> String {
    if !has_commits(root)
        && let Some(unborn) = current_branch(root)
    {
        return unborn;
    }
    if let Ok(head) = git(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) && let Some(name) = head.rsplit('/').next()
    {
        return name.to_string();
    }
    // `init.defaultBranch` says what NEW repositories are called, not what this one is: it only
    // wins when a branch of that name exists here, or a repo made on `master` is told its default is
    // a `main` it has never had (FEAT-105).
    let configured = git(root, &["config", "--get", "init.defaultBranch"])
        .ok()
        .filter(|n| !n.is_empty());
    for name in configured
        .iter()
        .map(String::as_str)
        .chain(["main", "master"])
    {
        if git(root, &["rev-parse", "--verify", "--quiet", name]).is_ok() {
            return name.to_string();
        }
    }
    configured.unwrap_or_else(|| "main".to_string())
}

fn hooks_dir(root: &Path) -> Result<PathBuf> {
    // Honour an existing core.hooksPath so a repo that already redirects its hooks keeps working.
    if let Ok(path) = git(root, &["config", "--get", "core.hooksPath"])
        && !path.is_empty()
    {
        let path = PathBuf::from(&path);
        return Ok(if path.is_absolute() {
            path
        } else {
            root.join(path)
        });
    }
    let common = git(root, &["rev-parse", "--git-common-dir"])?;
    let common = PathBuf::from(&common);
    Ok(if common.is_absolute() {
        common.join("hooks")
    } else {
        root.join(common).join("hooks")
    })
}

fn script(command: &str, name: &str) -> String {
    format!(
        "#!/bin/sh\n\
         {MARKER} — remove with `kanbanr git uninstall-hooks`\n\
         # Chain whatever hook was here before kanbanr installed this one.\n\
         prior=\"$(dirname \"$0\")/{name}.pre-kanbanr\"\n\
         [ -x \"$prior\" ] && {{ \"$prior\" \"$@\" || exit $?; }}\n\
         # No CLI on PATH: skip the check rather than blocking the commit.\n\
         command -v kanbanr >/dev/null 2>&1 || exit 0\n\
         exec kanbanr git {command}\n"
    )
}

fn is_ours(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| s.contains(MARKER))
}

/// Install the commit-msg and pre-commit hooks. Idempotent; an existing hook that is not ours is
/// preserved — moved aside and chained with `--force`, or reported and left alone without it.
pub fn install_hooks(root: &Path, force: bool) -> Result<Vec<String>> {
    let dir = hooks_dir(root)?;
    std::fs::create_dir_all(&dir)?;
    let mut installed = Vec::new();
    for (name, command) in HOOKS {
        let path = dir.join(name);
        if path.exists() && !is_ours(&path) {
            if !force {
                bail!(
                    "{} already has a {name} hook that kanbanr did not write. Re-run with --force \
                     to keep it: it is moved to {name}.pre-kanbanr and runs first.",
                    dir.display()
                );
            }
            std::fs::rename(&path, dir.join(format!("{name}.pre-kanbanr")))?;
        }
        std::fs::write(&path, script(command, name))?;
        make_executable(&path)?;
        installed.push(name.to_string());
    }
    Ok(installed)
}

pub fn uninstall_hooks(root: &Path) -> Result<usize> {
    let dir = hooks_dir(root)?;
    let mut removed = 0;
    for (name, _) in HOOKS {
        let path = dir.join(name);
        if path.exists() && is_ours(&path) {
            std::fs::remove_file(&path)?;
            removed += 1;
            // Put back whatever we moved aside, so uninstalling is a real undo.
            let prior = dir.join(format!("{name}.pre-kanbanr"));
            if prior.exists() {
                std::fs::rename(&prior, &path)?;
            }
        }
    }
    Ok(removed)
}

pub fn hooks_installed(root: &Path) -> bool {
    hooks_dir(root).is_ok_and(|dir| HOOKS.iter().all(|(name, _)| is_ours(&dir.join(name))))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanbanr-scm-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "--initial-branch=main"]).unwrap();
        dir
    }

    #[test]
    fn hooks_install_without_clobbering_what_was_there() {
        let repo = scratch_repo("install");
        let dir = hooks_dir(&repo).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("commit-msg"), "#!/bin/sh\necho mine\n").unwrap();

        // Someone else's hook is not overwritten silently.
        let err = install_hooks(&repo, false).unwrap_err().to_string();
        assert!(err.contains("--force"), "{err}");
        assert!(
            std::fs::read_to_string(dir.join("commit-msg"))
                .unwrap()
                .contains("echo mine")
        );

        // With --force it is kept and chained.
        assert_eq!(install_hooks(&repo, true).unwrap().len(), 2);
        assert!(
            std::fs::read_to_string(dir.join("commit-msg.pre-kanbanr"))
                .unwrap()
                .contains("echo mine")
        );
        assert!(is_ours(&dir.join("commit-msg")) && is_ours(&dir.join("pre-commit")));
        // Without the CLI on PATH the hook must step aside: a missing tool may not make a
        // repository uncommittable for someone who never installed kanbanr.
        let installed = std::fs::read_to_string(dir.join("commit-msg")).unwrap();
        assert!(
            installed.contains("command -v kanbanr >/dev/null 2>&1 || exit 0"),
            "{installed}"
        );
        assert!(hooks_installed(&repo));
        // Installing twice changes nothing.
        assert_eq!(install_hooks(&repo, false).unwrap().len(), 2);
        assert!(!dir.join("commit-msg.pre-kanbanr.pre-kanbanr").exists());

        // Uninstalling restores the original.
        assert_eq!(uninstall_hooks(&repo).unwrap(), 2);
        assert!(
            std::fs::read_to_string(dir.join("commit-msg"))
                .unwrap()
                .contains("echo mine")
        );
        assert!(!dir.join("pre-commit").exists());
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-094: finishing an interrupted merge was refused as "a commit straight to main".
    #[test]
    fn the_guard_lets_a_merge_be_completed() {
        let repo = scratch_repo("merge");
        for args in [
            &["config", "user.email", "t@example.com"][..],
            &["config", "user.name", "T"],
            &["commit", "-q", "--allow-empty", "-m", "base"],
            &["checkout", "-q", "-b", "feat/FEAT-001-thing"],
            &["commit", "-q", "--allow-empty", "-m", "work"],
            &["checkout", "-q", "main"],
        ] {
            git(&repo, args).unwrap();
        }
        // An ordinary state on the default branch: no merge, so the rule applies.
        assert!(!merge_in_progress(&repo));

        // A merge staged but not committed — exactly what a rejected merge message leaves behind.
        git(
            &repo,
            &["merge", "--no-ff", "--no-commit", "feat/FEAT-001-thing"],
        )
        .unwrap();
        assert!(
            merge_in_progress(&repo),
            "a staged merge must be recognised"
        );
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));

        // Aborting it returns to the ordinary state, where the rule applies again.
        git(&repo, &["merge", "--abort"]).unwrap();
        assert!(!merge_in_progress(&repo));
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-105 R-1: a fresh `git init` on `master` has `master` as its default, not an assumed
    /// `main` that does not exist — and once the first commit lands, the ordinary rules resume.
    #[test]
    fn an_unborn_head_is_the_default_branch() {
        let repo = std::env::temp_dir().join(format!("kanbanr-scm-unborn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "master"]).unwrap();
        // A configured default that disagrees with what this repo was created on must not win.
        git(&repo, &["config", "init.defaultBranch", "main"]).unwrap();
        assert!(!has_commits(&repo));
        assert_eq!(default_branch(&repo), "master");
        for args in [
            &["config", "user.email", "t@example.com"][..],
            &["config", "user.name", "T"],
            &["commit", "-q", "--allow-empty", "-m", "root"],
        ] {
            git(&repo, args).unwrap();
        }
        assert!(has_commits(&repo));
        git(&repo, &["checkout", "-q", "-b", "feat/FEAT-001-thing"]).unwrap();
        assert_eq!(
            default_branch(&repo),
            "master",
            "the existing branch still wins over main"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn the_branch_rules_read_the_repo_they_are_given() {
        let repo = scratch_repo("branch");
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(default_branch(&repo), "main");
        git(&repo, &["checkout", "-q", "-b", "feat/FEAT-001-thing"]).unwrap();
        assert_eq!(
            current_branch(&repo).as_deref(),
            Some("feat/FEAT-001-thing")
        );
        let _ = std::fs::remove_dir_all(&repo);
    }
}
