//! The project's **code** repository, as release promotion needs it (FEAT-183).
//!
//! Not the board's data folder (that is `git.rs`, through libgit2): this is the developer's own
//! repository, so everything here shells out to `git`, as the CLI's `scm.rs` does, to behave
//! exactly like their git — their hooks, their signing key, their config. It lives in core so the
//! CLI and the daemon promote a release the same way. FEAT-176 moves the rest of `scm.rs`'s
//! plumbing here and removes the CLI's copy.
//!
//! Nothing here pushes. A tag or a moved branch stays local; publishing it is whoever pushes.

use crate::error::{CoreError, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| CoreError::Git(format!("could not run git: {e}")))
}

/// Run git in `root`, returning its trimmed stdout. A failure carries git's own reason, so the
/// person sees why git refused rather than that something did.
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = run(root, args)?;
    if !out.status.success() {
        return Err(CoreError::Git(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn succeeds(root: &Path, args: &[&str]) -> bool {
    run(root, args).is_ok_and(|o| o.status.success())
}

/// The working tree root of the repository `dir` is in.
pub fn repo_root_in(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

fn current_branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}

fn has_commits(root: &Path) -> bool {
    succeeds(root, &["rev-parse", "--verify", "--quiet", "HEAD"])
}

/// The repository's default branch: in a repository with no commits, the branch HEAD is waiting
/// on; else the remote's head; else the first of the configured default, main and master that
/// exists here. The same rule as `scm::default_branch` (FEAT-105), which FEAT-176 retires.
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
    let configured = git(root, &["config", "--get", "init.defaultBranch"])
        .ok()
        .filter(|n| !n.is_empty());
    for name in configured
        .iter()
        .map(String::as_str)
        .chain(["main", "master"])
    {
        if branch_exists(root, name) {
            return name.to_string();
        }
    }
    configured.unwrap_or_else(|| "main".to_string())
}

/// The commit a revision names.
pub fn rev_parse(root: &Path, rev: &str) -> Result<String> {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .map_err(|_| CoreError::Git(format!("'{rev}' names no commit in {}", root.display())))
}

pub fn branch_exists(root: &Path, name: &str) -> bool {
    succeeds(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
}

/// Create branch `name` at `from`, without checking it out: promotion must not move the person's
/// working tree to another branch under them.
pub fn create_branch_from(root: &Path, name: &str, from: &str) -> Result<()> {
    git(root, &["branch", name, from]).map(|_| ())
}

pub fn tag_exists(root: &Path, tag: &str) -> bool {
    succeeds(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}"),
        ],
    )
}

/// An annotated tag `tag` on `at`, as `cut --tag` always made one.
pub fn create_tag(root: &Path, tag: &str, at: &str, message: &str) -> Result<()> {
    git(root, &["tag", "-a", tag, "-m", message, at]).map(|_| ())
}

pub fn delete_tag(root: &Path, tag: &str) -> Result<()> {
    git(root, &["tag", "-d", tag]).map(|_| ())
}

/// No changes to tracked files. Untracked files do not count: a fast-forward never touches them
/// unless the release adds the same path, and git refuses that case itself.
pub fn is_clean(root: &Path) -> bool {
    git(root, &["status", "--porcelain", "--untracked-files=no"]).is_ok_and(|s| s.is_empty())
}

/// Would merging `from` into `into` be free of conflicts? Asked without touching any branch, index
/// or working tree (`git merge-tree --write-tree`, git 2.38 or later).
pub fn merge_is_clean(root: &Path, into: &str, from: &str) -> Result<bool> {
    let out = run(root, &["merge-tree", "--write-tree", into, from])?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(CoreError::Git(format!(
            "git merge-tree --write-tree {into} {from}: {} (needs git 2.38 or later)",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

/// Is `ancestor` contained in `of`? That is, can `of` be reached from `ancestor` by fast-forward.
pub fn is_ancestor(root: &Path, ancestor: &str, of: &str) -> bool {
    succeeds(root, &["merge-base", "--is-ancestor", ancestor, of])
}

/// The worktree that has `branch` checked out, if any. A branch checked out somewhere must be
/// moved there, by git's merge or reset, or that worktree's files no longer match its branch.
fn checked_out_in(root: &Path, branch: &str) -> Option<PathBuf> {
    let list = git(root, &["worktree", "list", "--porcelain"]).ok()?;
    let want = format!("branch refs/heads/{branch}");
    let mut path = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if line == want {
            return path;
        }
    }
    None
}

/// Merge `from` into `into` with a merge commit, refusing a conflicted merge before it starts.
/// Where `into` is checked out, git's own merge runs there; elsewhere the merge commit is made
/// from the merged tree and the branch moved to it, so no working tree changes branch.
pub fn merge_no_ff(root: &Path, into: &str, from: &str, message: &str) -> Result<()> {
    if !merge_is_clean(root, into, from)? {
        return Err(CoreError::Git(format!(
            "{from} does not merge cleanly into {into}"
        )));
    }
    if let Some(worktree) = checked_out_in(root, into) {
        return git(&worktree, &["merge", "--no-ff", "-m", message, from]).map(|_| ());
    }
    let tree = git(root, &["merge-tree", "--write-tree", into, from])?;
    let tree = tree.lines().next().unwrap_or_default().to_string();
    let old = rev_parse(root, into)?;
    let theirs = rev_parse(root, from)?;
    let commit = git(
        root,
        &[
            "commit-tree",
            &tree,
            "-p",
            &old,
            "-p",
            &theirs,
            "-m",
            message,
        ],
    )?;
    git(
        root,
        &["update-ref", &format!("refs/heads/{into}"), &commit, &old],
    )
    .map(|_| ())
}

/// Fast-forward `branch` to `to`, returning the commit it was on so a later failure can put it
/// back. Refused unless `branch` is an ancestor of `to`: this never makes a merge commit and never
/// drops one.
pub fn fast_forward(root: &Path, branch: &str, to: &str) -> Result<String> {
    let previous = rev_parse(root, branch)?;
    let target = rev_parse(root, to)?;
    if !is_ancestor(root, &previous, &target) {
        return Err(CoreError::Git(format!(
            "{branch} cannot fast-forward to {to}: it has commits {to} lacks"
        )));
    }
    match checked_out_in(root, branch) {
        Some(worktree) => git(&worktree, &["merge", "--ff-only", &target])?,
        None => git(
            root,
            &[
                "update-ref",
                &format!("refs/heads/{branch}"),
                &target,
                &previous,
            ],
        )?,
    };
    Ok(previous)
}

/// Put `branch` back on `commit` — the undo of [`fast_forward`]. Where the branch is checked out,
/// `reset --keep` moves it and refuses rather than discard a local change.
pub fn reset_branch_to(root: &Path, branch: &str, commit: &str) -> Result<()> {
    match checked_out_in(root, branch) {
        Some(worktree) => git(&worktree, &["reset", "--keep", commit]).map(|_| ()),
        None => git(
            root,
            &["update-ref", &format!("refs/heads/{branch}"), commit],
        )
        .map(|_| ()),
    }
}

/// The commits `to` has that `from` lacks, newest first, one line each.
pub fn log_between(root: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    Ok(git(root, &["log", "--oneline", &format!("{from}..{to}")])?
        .lines()
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch repository on `main` with one commit and an identity of its own.
    pub(crate) fn scratch_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-coderepo-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            &["init", "-q", "--initial-branch=main"][..],
            &["config", "user.email", "t@example.com"],
            &["config", "user.name", "T"],
            &["config", "commit.gpgsign", "false"],
            &["config", "tag.gpgsign", "false"],
        ] {
            git(&dir, args).unwrap();
        }
        commit_file(&dir, "README", "base\n", "base");
        dir
    }

    pub(crate) fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(repo.join(file), content).unwrap();
        git(repo, &["add", file]).unwrap();
        git(repo, &["commit", "-q", "-m", message]).unwrap();
    }

    #[test]
    fn branches_and_tags_are_made_and_found() {
        let repo = scratch_repo("refs");
        assert_eq!(default_branch(&repo), "main");
        assert!(branch_exists(&repo, "main"));
        assert!(!branch_exists(&repo, "release/v1"));
        create_branch_from(&repo, "release/v1", "main").unwrap();
        assert!(branch_exists(&repo, "release/v1"));
        // Making the branch does not check it out.
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        assert!(create_branch_from(&repo, "release/v1", "main").is_err());

        assert!(!tag_exists(&repo, "v1"));
        create_tag(&repo, "v1", "release/v1", "Release v1").unwrap();
        assert!(tag_exists(&repo, "v1"));
        delete_tag(&repo, "v1").unwrap();
        assert!(!tag_exists(&repo, "v1"));
        assert_eq!(
            repo_root_in(&repo).map(|p| p.ends_with(repo.file_name().unwrap())),
            Some(true)
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_changed_tracked_file_is_unclean_an_untracked_one_is_not() {
        let repo = scratch_repo("clean");
        assert!(is_clean(&repo));
        std::fs::write(repo.join("new"), "x").unwrap();
        assert!(is_clean(&repo), "untracked files do not count");
        std::fs::write(repo.join("README"), "changed\n").unwrap();
        assert!(!is_clean(&repo));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn merge_is_clean_on_clean_and_conflicting_repos() {
        let repo = scratch_repo("mergeclean");
        create_branch_from(&repo, "release/v1", "main").unwrap();
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "a", "release\n", "on release");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        commit_file(&repo, "b", "main\n", "on main");
        assert!(merge_is_clean(&repo, "main", "release/v1").unwrap());

        // The same file changed on both sides is a conflict.
        commit_file(&repo, "README", "main's\n", "main edits");
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "README", "release's\n", "release edits");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        assert!(!merge_is_clean(&repo, "main", "release/v1").unwrap());
        assert!(merge_no_ff(&repo, "main", "release/v1", "merge").is_err());
        assert!(is_clean(&repo), "a refused merge leaves nothing behind");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn merge_no_ff_makes_a_merge_commit_checked_out_or_not() {
        let repo = scratch_repo("noff");
        create_branch_from(&repo, "release/v1", "main").unwrap();
        create_branch_from(&repo, "other", "main").unwrap();
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "a", "release\n", "on release");
        // `main` is not checked out: the merge is made without a working tree.
        merge_no_ff(&repo, "main", "release/v1", "merge release").unwrap();
        let parents = git(&repo, &["rev-list", "--parents", "-n", "1", "main"]).unwrap();
        assert_eq!(parents.split_whitespace().count(), 3, "{parents}");
        // `other` checked out: git's own merge runs in it.
        git(&repo, &["checkout", "-q", "other"]).unwrap();
        merge_no_ff(&repo, "other", "release/v1", "merge into other").unwrap();
        assert!(repo.join("a").exists());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn is_ancestor_and_fast_forward() {
        let repo = scratch_repo("ff");
        create_branch_from(&repo, "release/v1", "main").unwrap();
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "a", "release\n", "on release");
        assert!(is_ancestor(&repo, "main", "release/v1"));
        assert!(!is_ancestor(&repo, "release/v1", "main"));
        assert_eq!(log_between(&repo, "main", "release/v1").unwrap().len(), 1);

        // Not checked out: the ref moves.
        let before = rev_parse(&repo, "main").unwrap();
        let previous = fast_forward(&repo, "main", "release/v1").unwrap();
        assert_eq!(previous, before);
        assert_eq!(
            rev_parse(&repo, "main").unwrap(),
            rev_parse(&repo, "release/v1").unwrap()
        );

        // Checked out: the working tree moves with it.
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "c", "more\n", "more");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        fast_forward(&repo, "main", "release/v1").unwrap();
        assert!(repo.join("c").exists());
        assert!(is_clean(&repo));

        // A branch with commits the target lacks is never fast-forwarded.
        commit_file(&repo, "d", "main only\n", "main only");
        assert!(fast_forward(&repo, "main", "release/v1").is_err());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn reset_branch_to_undoes_a_fast_forward() {
        let repo = scratch_repo("rollback");
        create_branch_from(&repo, "release/v1", "main").unwrap();
        git(&repo, &["checkout", "-q", "release/v1"]).unwrap();
        commit_file(&repo, "a", "release\n", "on release");
        let previous = fast_forward(&repo, "main", "release/v1").unwrap();
        reset_branch_to(&repo, "main", &previous).unwrap();
        assert_eq!(rev_parse(&repo, "main").unwrap(), previous);

        // And where it is checked out, the files go back too.
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        let previous = fast_forward(&repo, "main", "release/v1").unwrap();
        assert!(repo.join("a").exists());
        reset_branch_to(&repo, "main", &previous).unwrap();
        assert!(!repo.join("a").exists());
        assert!(is_clean(&repo));
        let _ = std::fs::remove_dir_all(&repo);
    }
}
