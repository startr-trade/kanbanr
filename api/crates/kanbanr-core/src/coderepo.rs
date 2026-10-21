//! The project's **code** repository: what the commit guards, `start` and release promotion read
//! from it and do to it (FEAT-056, FEAT-183).
//!
//! Not the board's data folder (that is `git.rs`, through libgit2): this is the developer's own
//! repository, so everything here shells out to `git` to behave exactly like their git — their
//! hooks, their signing key, their config. It lives in core so the CLI and the daemon read and
//! move a repository the same way (FEAT-176); the CLI keeps only installing its hook files.
//!
//! Nothing here pushes. A tag or a moved branch stays local; publishing it is whoever pushes.

use crate::error::{CoreError, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    run_with(root, args, None, &[])
}

/// Run git with bytes on its stdin and extra environment (FEAT-212: plumbing that writes objects
/// and a temporary index). Still never a push.
fn run_with(
    root: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: &[(&str, &str)],
) -> Result<Output> {
    // kanbanr never publishes anything (FEAT-189): pushing is whoever reviewed it, by hand.
    if args.first() == Some(&"push") {
        return Err(CoreError::Git(
            "kanbanr never pushes: push it yourself once you have checked it".into(),
        ));
    }
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let fail = |e: std::io::Error| CoreError::Git(format!("could not run git: {e}"));
    let Some(input) = stdin else {
        return cmd.output().map_err(fail);
    };
    use std::io::Write;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(fail)?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(input).map_err(fail)?;
    }
    child.wait_with_output().map_err(fail)
}

/// [`git`] with stdin and environment.
fn git_with(
    root: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: &[(&str, &str)],
) -> Result<String> {
    let out = run_with(root, args, stdin, env)?;
    if !out.status.success() {
        return Err(CoreError::Git(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
    succeeds(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
}

/// Does the repository have a commit yet? A fresh `git init` has an unborn HEAD: a branch name
/// with nothing behind it, from which no item branch can be made.
pub fn has_commits(root: &Path) -> bool {
    succeeds(root, &["rev-parse", "--verify", "--quiet", "HEAD"])
}

/// The branch work is *not* supposed to land on directly (FEAT-105): in a repository with no
/// commits, the branch HEAD is waiting on — whatever `git init` chose, which is what the first
/// commit will create; else the remote's head; else the first of the configured default, main and
/// master that exists here.
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
        if branch_exists(root, name) {
            return name.to_string();
        }
    }
    configured.unwrap_or_else(|| "main".to_string())
}

/// What the commit and branch rules read from the repository at `root`, with the commit's
/// `message` where the caller knows it (FEAT-176): the facts `scm`'s verdicts decide on.
pub fn repo_facts(root: &Path, message: Option<String>) -> crate::scm::RepoFacts {
    crate::scm::RepoFacts {
        branch: current_branch(root),
        default_branch: default_branch(root),
        merging: merge_in_progress(root),
        has_commits: has_commits(root),
        message,
    }
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

/// Take away a branch this run made, when the board step after it failed (FEAT-189). Never used
/// on a branch someone else made.
pub fn delete_branch(root: &Path, name: &str) -> Result<()> {
    git(root, &["branch", "-D", name]).map(|_| ())
}

/// The local branches with their tips, and which of them `main` already contains (FEAT-189): one
/// `for-each-ref` and one `branch --merged`.
pub fn branch_facts(root: &Path, main: &str) -> Result<crate::gitmodel::BranchFacts> {
    let mut facts = crate::gitmodel::BranchFacts {
        main: main.to_string(),
        ..Default::default()
    };
    let refs = git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads",
        ],
    )?;
    for line in refs.lines() {
        if let Some((name, sha)) = line.rsplit_once(' ') {
            facts.branches.insert(name.to_string(), sha.to_string());
        }
    }
    if facts.branches.contains_key(main) {
        let merged = git(
            root,
            &["branch", "--merged", main, "--format=%(refname:short)"],
        )?;
        facts.merged_into_main = merged.lines().map(|l| l.trim().to_string()).collect();
    }
    Ok(facts)
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

/// Whether `from` merges cleanly into `into`, and if not, the paths that conflict (FEAT-190).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MergePreview {
    pub clean: bool,
    pub conflicts: Vec<String>,
}

/// Preview merging `from` into `into` without touching any working tree or branch.
pub fn merge_preview(root: &Path, into: &str, from: &str) -> Result<MergePreview> {
    let out = run(
        root,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            into,
            from,
        ],
    )?;
    match out.status.code() {
        Some(0) => Ok(MergePreview {
            clean: true,
            conflicts: Vec::new(),
        }),
        Some(1) => {
            // The first line is the tree; the conflicted paths follow.
            let text = String::from_utf8_lossy(&out.stdout);
            let mut conflicts: Vec<String> = text
                .lines()
                .skip(1)
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            conflicts.dedup();
            Ok(MergePreview {
                clean: false,
                conflicts,
            })
        }
        _ => Err(CoreError::Git(format!(
            "git merge-tree {into} {from}: {} (needs git 2.38 or later)",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

/// A commit, for reading its trailers (FEAT-190).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub message: String,
}

/// The commits `head` has that `base` lacks, newest first.
pub fn commits_between(root: &Path, base: &str, head: &str) -> Result<Vec<Commit>> {
    let out = git(
        root,
        &[
            "log",
            "--format=%H%x1f%s%x1f%B%x1e",
            &format!("{base}..{head}"),
        ],
    )?;
    Ok(out
        .split('\u{1e}')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let mut parts = r.splitn(3, '\u{1f}');
            Some(Commit {
                sha: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
                message: parts.next().unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// At most `limit` commits of `range` — `base..head`, or a single ref for its own history —
/// newest first (FEAT-191: a doctor reads a window, never a whole history).
pub fn commits_limited(root: &Path, range: &str, limit: usize) -> Result<Vec<Commit>> {
    let out = git(
        root,
        &[
            "log",
            "-n",
            &limit.to_string(),
            "--format=%H%x1f%s%x1f%B%x1e",
            range,
        ],
    )?;
    Ok(out
        .split('\u{1e}')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let mut parts = r.splitn(3, '\u{1f}');
            Some(Commit {
                sha: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
                message: parts.next().unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// The local branches `target` contains (FEAT-191): one `git branch --merged`.
pub fn branches_merged_into(
    root: &Path,
    target: &str,
) -> Result<std::collections::BTreeSet<String>> {
    Ok(git(
        root,
        &["branch", "--merged", target, "--format=%(refname:short)"],
    )?
    .lines()
    .map(|l| l.trim().to_string())
    .filter(|l| !l.is_empty())
    .collect())
}

/// Does the worktree at `root` have uncommitted changes to tracked files?
pub fn worktree_dirty(root: &Path) -> bool {
    !is_clean(root)
}

/// The worktree `branch` is checked out in, if any (FEAT-190: a line checked out must be clean
/// before a merge moves it).
pub fn worktree_of(root: &Path, branch: &str) -> Option<PathBuf> {
    checked_out_in(root, branch)
}

/// What agent scope resolution reads about the working tree `cwd` is in (FEAT-214): its root, its
/// own git directory, the main worktree's root, whether it is a linked worktree and its branch.
/// `None` outside a work tree. Paths are resolved on disk, so they compare as the board stores them.
pub fn worktree_facts(cwd: &Path) -> Option<crate::agentscope::WorktreeFacts> {
    let out = git(
        cwd,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .ok()?;
    let mut lines = out
        .lines()
        .map(|l| crate::project::normalize(Path::new(l.trim())));
    let (root, git_dir, common) = (lines.next()?, lines.next()?, lines.next()?);
    let linked = git_dir != common;
    // The main worktree is the one whose git directory is the common one: `<main>/.git`.
    let main_root = if common.file_name().is_some_and(|n| n == ".git") {
        common.parent()?.to_path_buf()
    } else {
        root.clone()
    };
    Some(crate::agentscope::WorktreeFacts {
        branch: current_branch(&root),
        root,
        git_dir,
        main_root,
        linked,
    })
}

/// The commits of `range` committed after `since` (an RFC3339 instant), newest first (FEAT-214:
/// agent drift). Compared as instants here rather than by `git log --since`, which reads only to
/// the second and parses fractions loosely.
pub fn commits_since(root: &Path, range: &str, since: &str) -> Result<Vec<Commit>> {
    let out = git(root, &["log", "--format=%H%x1f%cI%x1f%s%x1f%B%x1e", range])?;
    Ok(out
        .split('\u{1e}')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let mut parts = r.splitn(4, '\u{1f}');
            let sha = parts.next()?.to_string();
            let at = parts.next()?.to_string();
            let commit = Commit {
                sha,
                subject: parts.next()?.to_string(),
                message: parts.next().unwrap_or_default().to_string(),
            };
            crate::before(since, &at).then_some(commit)
        })
        .collect())
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
/// Returns the tips of `into` before and after, so a later failure can put it back (FEAT-190).
pub fn merge_no_ff(root: &Path, into: &str, from: &str, message: &str) -> Result<(String, String)> {
    if !merge_is_clean(root, into, from)? {
        return Err(CoreError::Git(format!(
            "{from} does not merge cleanly into {into}"
        )));
    }
    if let Some(worktree) = checked_out_in(root, into) {
        if !is_clean(&worktree) {
            return Err(CoreError::Git(format!(
                "{into} is checked out in {} with uncommitted changes: commit or stash them first",
                worktree.display()
            )));
        }
        let old = rev_parse(root, into)?;
        git(&worktree, &["merge", "--no-ff", "-m", message, from])?;
        return Ok((old, rev_parse(root, into)?));
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
    )?;
    Ok((old, commit))
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

/// A file as it is at `rev`: its mode and its text, or `None` when `rev` has no such file
/// (FEAT-212). Read from git's objects, so no working tree is involved.
pub fn read_blob(root: &Path, rev: &str, path: &str) -> Result<Option<(String, String)>> {
    let listed = git(root, &["ls-tree", rev, "--", path])?;
    let Some(line) = listed.lines().next() else {
        return Ok(None);
    };
    // `<mode> <type> <object>\t<path>`
    let meta = line.split('\t').next().unwrap_or_default();
    let mut parts = meta.split_whitespace();
    let (Some(mode), Some(kind), Some(object)) = (parts.next(), parts.next(), parts.next()) else {
        return Ok(None);
    };
    if kind != "blob" {
        return Err(CoreError::Git(format!(
            "{path} at {rev} is a {kind}, not a file"
        )));
    }
    let out = run(root, &["cat-file", "blob", object])?;
    if !out.status.success() {
        return Err(CoreError::Git(format!(
            "git cat-file blob {object}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let text = String::from_utf8(out.stdout)
        .map_err(|_| CoreError::Git(format!("{path} at {rev} is not UTF-8 text")))?;
    Ok(Some((mode.to_string(), text)))
}

/// One commit on `branch` holding `files` (path, new content) on top of its tip, made with git's
/// plumbing so nothing is checked out and no hook runs (FEAT-212): kanbanr's own release
/// preparation commit. Refused when the branch is checked out in a worktree, whose files would
/// then no longer match it. Returns the tips before and after, for [`undo_commit`].
pub fn commit_files(
    root: &Path,
    branch: &str,
    files: &[(String, String)],
    message: &str,
) -> Result<(String, String)> {
    if let Some(worktree) = checked_out_in(root, branch) {
        return Err(CoreError::Git(format!(
            "{branch} is checked out in {}: kanbanr commits the release preparation without \
             checking anything out, so switch that worktree to another branch first",
            worktree.display()
        )));
    }
    let tip = rev_parse(root, branch)?;
    let index = PathBuf::from(git(
        root,
        &["rev-parse", "--git-path", "kanbanr-prepare.index"],
    )?);
    let index = if index.is_absolute() {
        index
    } else {
        root.join(index)
    };
    let index_env = index.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", index_env.as_str())];
    let result = (|| {
        git_with(root, &["read-tree", &tip], None, &env)?;
        for (path, content) in files {
            let mode = read_blob(root, &tip, path)?
                .map(|(m, _)| m)
                .unwrap_or_else(|| "100644".to_string());
            let blob = git_with(
                root,
                &["hash-object", "-w", "--stdin"],
                Some(content.as_bytes()),
                &[],
            )?;
            git_with(
                root,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("{mode},{blob},{path}"),
                ],
                None,
                &env,
            )?;
        }
        let tree = git_with(root, &["write-tree"], None, &env)?;
        let commit = git(root, &["commit-tree", &tree, "-p", &tip, "-m", message])?;
        git(
            root,
            &["update-ref", &format!("refs/heads/{branch}"), &commit, &tip],
        )?;
        Ok(commit)
    })();
    let _ = std::fs::remove_file(&index);
    result.map(|commit| (tip, commit))
}

/// Put `branch` back from `new` to `old` — the undo of [`commit_files`], refused if the branch
/// has moved on since.
pub fn undo_commit(root: &Path, branch: &str, old: &str, new: &str) -> Result<()> {
    git(
        root,
        &["update-ref", &format!("refs/heads/{branch}"), old, new],
    )
    .map(|_| ())
}

/// The remote-tracking branch `branch` is judged pushed against: its upstream, else
/// `origin/<branch>` when that exists.
pub fn upstream_of(root: &Path, branch: &str) -> Option<String> {
    if let Ok(up) = git(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            &format!("{branch}@{{upstream}}"),
        ],
    ) && !up.is_empty()
    {
        return Some(up);
    }
    let origin = format!("origin/{branch}");
    succeeds(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{origin}"),
        ],
    )
    .then_some(origin)
}

/// The remote `branch` pushes to: its configured remote, else `origin`.
pub fn remote_of(root: &Path, branch: &str) -> String {
    git(
        root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )
    .ok()
    .filter(|r| !r.is_empty() && r != ".")
    .unwrap_or_else(|| "origin".to_string())
}

/// Bring `remote`'s `branch` into its remote-tracking ref (FEAT-212). Non-interactive and bounded:
/// no prompt for credentials, a slow transfer is abandoned, ssh gives up rather than asks.
pub fn fetch_branch(root: &Path, remote: &str, branch: &str) -> Result<()> {
    let mut env = vec![
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_HTTP_LOW_SPEED_LIMIT", "1000"),
        ("GIT_HTTP_LOW_SPEED_TIME", "10"),
    ];
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        env.push((
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=10",
        ));
    }
    git_with(
        root,
        &["fetch", "--quiet", "--no-tags", remote, branch],
        None,
        &env,
    )
    .map(|_| ())
}

/// The remote-tracking branches that contain `commit`.
pub fn remote_branches_containing(root: &Path, commit: &str) -> Result<Vec<String>> {
    Ok(git(
        root,
        &[
            "branch",
            "-r",
            "--contains",
            commit,
            "--format=%(refname:short)",
        ],
    )?
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
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

    /// FEAT-190: merging into a branch that is not checked out makes the merge commit without
    /// touching the working tree, and says where the branch was.
    #[test]
    fn merge_no_ff_leaves_worktree_untouched() {
        let repo = scratch_repo("merge-untouched");
        create_branch_from(&repo, "line", "main").unwrap();
        git(&repo, &["checkout", "-q", "-b", "feat/x"]).unwrap();
        commit_file(&repo, "x.txt", "x\n", "x");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        std::fs::write(repo.join("README"), "edited, not committed\n").unwrap();
        let before = rev_parse(&repo, "line").unwrap();
        let (old, new) = merge_no_ff(
            &repo,
            "line",
            "feat/x",
            "Merge feat/x\n\nRefs: kanbanr:FEAT-1",
        )
        .unwrap();
        assert_eq!(old, before);
        assert_eq!(rev_parse(&repo, "line").unwrap(), new);
        assert_eq!(
            git(&repo, &["rev-list", "--parents", "-n1", "line"])
                .unwrap()
                .split(' ')
                .count(),
            3,
            "a merge commit"
        );
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(
            std::fs::read_to_string(repo.join("README")).unwrap(),
            "edited, not committed\n"
        );
        assert!(!repo.join("x.txt").exists(), "main's tree is as it was");
    }

    #[test]
    fn fast_forward_refuses_divergence() {
        let repo = scratch_repo("ff-diverge");
        create_branch_from(&repo, "sprint", "main").unwrap();
        git(&repo, &["checkout", "-q", "sprint"]).unwrap();
        commit_file(&repo, "a.txt", "a\n", "a");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        commit_file(&repo, "b.txt", "b\n", "b");
        let main = rev_parse(&repo, "main").unwrap();
        let err = fast_forward(&repo, "main", "sprint")
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot fast-forward"), "{err}");
        assert_eq!(rev_parse(&repo, "main").unwrap(), main, "main did not move");
    }

    #[test]
    fn merge_preview_names_conflicts() {
        let repo = scratch_repo("preview");
        create_branch_from(&repo, "feat/x", "main").unwrap();
        commit_file(&repo, "README", "main's\n", "main edit");
        git(&repo, &["checkout", "-q", "feat/x"]).unwrap();
        commit_file(&repo, "README", "the item's\n", "item edit");
        commit_file(&repo, "new.txt", "n\n", "new");
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        let p = merge_preview(&repo, "main", "feat/x").unwrap();
        assert!(!p.clean);
        assert_eq!(p.conflicts, vec!["README".to_string()]);
        create_branch_from(&repo, "feat/y", "main").unwrap();
        assert!(merge_preview(&repo, "main", "feat/y").unwrap().clean);
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

    /// FEAT-212 R-8: the preparation commit lands on the branch with the files given, keeping a
    /// file's mode, and nothing is checked out: the working tree, the index and HEAD stay as they
    /// were. A branch checked out somewhere is refused; the undo puts the branch back.
    #[test]
    fn commit_files_moves_branch_without_checkout() {
        let repo = scratch_repo("commit-files");
        std::fs::write(repo.join("run.sh"), "echo 1\n").unwrap();
        git(&repo, &["add", "run.sh"]).unwrap();
        git(&repo, &["update-index", "--chmod=+x", "run.sh"]).unwrap();
        git(&repo, &["commit", "-q", "-m", "script"]).unwrap();
        create_branch_from(&repo, "release/v1", "main").unwrap();
        std::fs::write(repo.join("README"), "edited, not committed\n").unwrap();
        let head_before = rev_parse(&repo, "HEAD").unwrap();
        let tip = rev_parse(&repo, "release/v1").unwrap();
        let files = vec![
            ("README".to_string(), "v1\n".to_string()),
            ("run.sh".to_string(), "echo 2\n".to_string()),
            ("docs/new.md".to_string(), "# v1\n".to_string()),
        ];
        let (old, new) = commit_files(
            &repo,
            "release/v1",
            &files,
            "Release v1\n\n[no-ref] release preparation, made by kanbanr",
        )
        .unwrap();
        assert_eq!(
            (old.as_str(), rev_parse(&repo, "release/v1").unwrap()),
            (tip.as_str(), new.clone())
        );
        assert_eq!(
            read_blob(&repo, "release/v1", "README").unwrap().unwrap().1,
            "v1\n"
        );
        assert_eq!(
            read_blob(&repo, "release/v1", "run.sh").unwrap().unwrap().0,
            "100755",
            "the mode is kept"
        );
        assert_eq!(
            read_blob(&repo, "release/v1", "docs/new.md")
                .unwrap()
                .unwrap(),
            ("100644".to_string(), "# v1\n".to_string())
        );
        assert_eq!(read_blob(&repo, "release/v1", "absent").unwrap(), None);
        assert_eq!(
            git(
                &repo,
                &["rev-list", "--count", &format!("{tip}..release/v1")]
            )
            .unwrap(),
            "1",
            "one commit"
        );
        // Nothing checked out moved.
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(rev_parse(&repo, "HEAD").unwrap(), head_before);
        assert_eq!(
            std::fs::read_to_string(repo.join("README")).unwrap(),
            "edited, not committed\n"
        );
        assert!(!repo.join("docs").exists());
        assert_eq!(
            git(&repo, &["diff", "--cached", "--name-only"]).unwrap(),
            "",
            "the real index is untouched"
        );
        let index = git(&repo, &["rev-parse", "--git-path", "kanbanr-prepare.index"]).unwrap();
        assert!(!repo.join(index).exists(), "the temporary index is removed");

        undo_commit(&repo, "release/v1", &old, &new).unwrap();
        assert_eq!(rev_parse(&repo, "release/v1").unwrap(), tip);
        // Checked out: refused, nothing made.
        let err = commit_files(&repo, "main", &files, "x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("is checked out in"), "{err}");
        assert_eq!(rev_parse(&repo, "main").unwrap(), head_before);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FEAT-212 R-7: `pushed` reads main's remote-tracking branch — its upstream, else
    /// `origin/<main>` — never the remote itself; fetching updates it, and nothing here can push.
    #[test]
    fn pushed_condition_reads_remote_tracking_ref() {
        let repo = scratch_repo("pushed");
        let bare = repo.with_extension("remote.git");
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]).unwrap();
        assert_eq!(upstream_of(&repo, "main"), None, "no remote yet");
        assert_eq!(remote_of(&repo, "main"), "origin");
        git(&repo, &["remote", "add", "origin", &bare.to_string_lossy()]).unwrap();
        // The test pushes with plain git: kanbanr's own runner refuses.
        assert!(run(&repo, &["push", "origin", "main"]).is_err());
        assert!(
            Command::new("git")
                .args(["push", "-q", "origin", "main"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(upstream_of(&repo, "main").as_deref(), Some("origin/main"));
        let pushed = rev_parse(&repo, "main").unwrap();
        assert!(is_ancestor(&repo, &pushed, "origin/main"));
        commit_file(&repo, "a", "a\n", "not pushed");
        let local = rev_parse(&repo, "main").unwrap();
        assert!(!is_ancestor(&repo, &local, "origin/main"));
        assert!(
            remote_branches_containing(&repo, &local)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            remote_branches_containing(&repo, &pushed).unwrap(),
            ["origin/main"]
        );
        // The remote moves (someone pushed from elsewhere): only a fetch shows it.
        let other = repo.with_extension("other");
        let _ = std::fs::remove_dir_all(&other);
        assert!(
            Command::new("git")
                .args([
                    "clone",
                    "-b",
                    "main",
                    "-q",
                    &bare.to_string_lossy(),
                    &other.to_string_lossy()
                ])
                .status()
                .unwrap()
                .success()
        );
        git(&repo, &["checkout", "-q", "-b", "side"]).unwrap();
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        git(&repo, &["push", "--dry-run"]).unwrap_err();
        let before = rev_parse(&repo, "origin/main").unwrap();
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@x",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    "elsewhere",
                ])
                .current_dir(&other)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["push", "-q", "origin", "main"])
                .current_dir(&other)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(rev_parse(&repo, "origin/main").unwrap(), before);
        fetch_branch(&repo, "origin", "main").unwrap();
        assert_ne!(rev_parse(&repo, "origin/main").unwrap(), before);
        // An upstream set explicitly wins over origin/<branch>.
        git(&repo, &["branch", "--set-upstream-to=origin/main", "side"]).unwrap();
        assert_eq!(upstream_of(&repo, "side").as_deref(), Some("origin/main"));
        // A remote that cannot be reached is an error, not a hang.
        git(
            &repo,
            &["remote", "add", "gone", "/nonexistent/kanbanr-remote.git"],
        )
        .unwrap();
        assert!(fetch_branch(&repo, "gone", "main").is_err());
        for d in [&repo, &bare, &other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// A scratch repository with no commit yet, on `main`.
    fn unborn_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-coderepo-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "--initial-branch=main"]).unwrap();
        dir
    }

    /// FEAT-094: finishing an interrupted merge was refused as "a commit straight to main".
    #[test]
    fn the_guard_lets_a_merge_be_completed() {
        let repo = unborn_repo("merge");
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
        let repo =
            std::env::temp_dir().join(format!("kanbanr-coderepo-unborn-{}", std::process::id()));
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
        let repo = unborn_repo("branch");
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
