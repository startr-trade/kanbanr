//! `kanbanr build queue` (FTR-062): branches integrate into their target one checked batch at a
//! time, so parallel work does not break the line.
//!
//! A queued branch waits in `<git-common-dir>/kanbanr/queue.yaml`. `queue run` takes the next
//! batch ([`buildplan::next_batch`]: one target, up to `queue_batch` branches, by priority), merges
//! each branch onto the target's tip in turn — each onto the last, without touching any branch —
//! checks the result in its own detached worktree (`kanbanr build check`, the line tier for a line,
//! the merge tier for an item branch), and moves the target only when the check is green and the
//! target has not moved meanwhile. A conflicting branch is refused at once; a red batch is split
//! and each branch checked alone, so only the red one is refused. The agent's branch keeps its
//! commits; the queue never pushes, and its worktree is removed on every path.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;
use kanbanr_core::Store;
use kanbanr_core::buildplan::{self, Priority, QueueEntry, QueueState};
use kanbanr_core::coderepo;
use kanbanr_core::config::{ProjectConfig, Role};

use crate::buildrun;

const QUEUE_FILE: &str = "queue.yaml";
const RUN_LOCK: &str = "queue.lock";
const LOGS: &str = "queue-logs";

/// The repository's build folder (`<git-common-dir>/kanbanr`) for the checkout at `repo`.
fn repo_dir(repo: &Path) -> anyhow::Result<PathBuf> {
    buildrun::git_common_dir(repo)
        .map(|c| buildplan::repo_dir(&c))
        .ok_or_else(|| {
            anyhow::anyhow!("not in a git repository: the merge queue integrates git branches")
        })
}

/// The main checkout of the repository at `repo`: branches are made and merged from there.
fn main_root(repo: &Path) -> PathBuf {
    coderepo::worktree_facts(repo)
        .map(|f| f.main_root)
        .unwrap_or_else(|| repo.to_path_buf())
}

/// Read the queue under its lock, let `f` change it, and write it back while still holding the
/// lock, so two agents queueing at once never lose an entry. The file is locked and rewritten in
/// place (never renamed over), so the lock is the file's own.
fn with_queue<T>(
    dir: &Path,
    f: impl FnOnce(&mut Vec<QueueEntry>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(QUEUE_FILE);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    file.lock_exclusive()?;
    let mut entries = read_entries(&mut file, &path)?;
    let before = entries.clone();
    let out = f(&mut entries)?;
    if entries != before {
        let text = kanbanr_core::yaml::to_string(&entries)?;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    Ok(out)
}

fn read_entries(file: &mut File, path: &Path) -> anyhow::Result<Vec<QueueEntry>> {
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    kanbanr_core::yaml::from_str(&text)
        .with_context(|| format!("{} cannot be read", path.display()))
}

/// The queue's entries, in the order they were added.
pub fn list(repo: &Path) -> Vec<QueueEntry> {
    let Ok(dir) = repo_dir(repo) else {
        return Vec::new();
    };
    let path = dir.join(QUEUE_FILE);
    let Ok(mut file) = File::open(&path) else {
        return Vec::new();
    };
    if file.lock_shared().is_err() {
        return Vec::new();
    }
    read_entries(&mut file, &path).unwrap_or_default()
}

/// Is `branch` a local branch of the repository at `root`?
fn branch_exists(root: &Path, branch: &str) -> bool {
    coderepo::rev_parse(root, &format!("refs/heads/{branch}")).is_ok()
}

/// The item branch of `code` among the repository's local branches: the one the item's branch
/// pattern names that is not a task branch.
fn item_branch(root: &Path, config: &ProjectConfig, code: &str) -> anyhow::Result<String> {
    let branches = coderepo::local_branches(root)?;
    let found: Vec<&String> = branches
        .keys()
        .filter(|b| kanbanr_core::scm::task_of_branch(b).is_none())
        .filter(|b| {
            kanbanr_core::scm::item_code_from_branch(config.branch_pattern(), b).as_deref()
                == Some(code)
        })
        .collect();
    match found.as_slice() {
        [] => anyhow::bail!("{code} has no branch in {}", root.display()),
        [one] => Ok((*one).clone()),
        many => anyhow::bail!(
            "{code} has more than one branch ({}): queue the one to integrate by name",
            many.iter()
                .map(|b| b.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The line an item integrates into: its release's branch or the current line, else main. With
/// no item, main.
fn line_of(
    store: &Store,
    project: &str,
    root: &Path,
    config: &ProjectConfig,
    code: &str,
) -> String {
    let main = kanbanr_core::gitmodel::main_of(config, root);
    if code.is_empty() {
        return main;
    }
    let line = || -> Option<String> {
        let (model, _) = kanbanr_core::gitmodel::load(store, project, root).ok()?;
        let item = store.load_feature(project, code).ok()?;
        let releases = kanbanr_core::releases::load(store, project).unwrap_or_default();
        kanbanr_core::gitmodel::line_for(&model, &item, &releases)
    };
    line().unwrap_or(main)
}

/// The critical path's tasks, `CODE/TL-001/T3`; empty when the graph cannot be built.
fn critical_tasks(store: &Store, project: &str) -> Vec<String> {
    kanbanr_core::taskgraph::TaskGraph::build(store, project, None)
        .map(|g| g.critical_path(project).0)
        .unwrap_or_default()
}

/// Queue `target` for integration: `CODE/TL/T` (the task branch, into its item branch), `CODE`
/// (the item branch, into its line) or a branch name (its item and task read from the name).
/// Priority 0 when the task, or an open task of the item, is on the critical path. Refused when
/// the branch does not exist or is already waiting.
pub fn add(store: &Store, project: &str, repo: &Path, target: &str) -> anyhow::Result<QueueEntry> {
    let root = main_root(repo);
    let config = store
        .load_meta(project)
        .with_context(|| format!("the merge queue needs the project's board ({project})"))?
        .config;
    let task_ref = kanbanr_core::taskgraph::parse_task_ref(target, project, "", "")
        .filter(|(p, _, list, _)| list.is_some() && (p == project || project.is_empty()));
    let is_item = !target.contains('/') && store.load_feature(project, target).is_ok();
    let (branch, into, item, task) = if let Some((_, code, Some(list), key)) = task_ref {
        let item_b = item_branch(&root, &config, &code)?;
        let branch = kanbanr_core::scm::task_branch_for(&item_b, &list, &key);
        (
            branch,
            item_b,
            code.clone(),
            Some(format!("{code}/{list}/{key}")),
        )
    } else if is_item {
        let branch = item_branch(&root, &config, target)?;
        let into = line_of(store, project, &root, &config, target);
        (branch, into, target.to_string(), None)
    } else if let Some((list, key)) = kanbanr_core::scm::task_of_branch(target) {
        let (item_b, _) = target.rsplit_once("--").unwrap_or((target, ""));
        let code = kanbanr_core::scm::item_code_from_branch(config.branch_pattern(), item_b)
            .unwrap_or_default();
        let task = (!code.is_empty()).then(|| format!("{code}/{list}/{key}"));
        (target.to_string(), item_b.to_string(), code, task)
    } else {
        let code = kanbanr_core::scm::item_code_from_branch(config.branch_pattern(), target)
            .unwrap_or_default();
        let into = line_of(store, project, &root, &config, &code);
        (target.to_string(), into, code, None)
    };
    if !branch_exists(&root, &branch) {
        anyhow::bail!("no branch {branch} in {}: nothing to queue", root.display());
    }
    if !branch_exists(&root, &into) {
        anyhow::bail!(
            "{branch} would integrate into {into}, which does not exist in {}",
            root.display()
        );
    }
    let critical = critical_tasks(store, project);
    let priority = match (&task, item.is_empty()) {
        (Some(t), _) if critical.iter().any(|c| c == t) => Priority::Critical,
        (None, false) if critical.iter().any(|c| c.starts_with(&format!("{item}/"))) => {
            Priority::Critical
        }
        (_, false) => Priority::Task,
        (_, true) => Priority::None,
    };
    let dir = repo_dir(repo)?;
    with_queue(&dir, |entries| {
        if let Some(e) = entries.iter().find(|e| {
            e.branch == branch && matches!(e.state, QueueState::Queued | QueueState::Checking)
        }) {
            anyhow::bail!("{branch} is already queued as {}", e.id);
        }
        let n = entries
            .iter()
            .filter_map(|e| e.id.strip_prefix("MQ-").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0)
            + 1;
        let entry = QueueEntry {
            id: format!("MQ-{n}"),
            branch: branch.clone(),
            into: into.clone(),
            item: item.clone(),
            task: task.clone(),
            priority: priority as u8,
            added_at: kanbanr_core::now_rfc3339(),
            state: QueueState::Queued,
            ..Default::default()
        };
        entries.push(entry.clone());
        Ok(entry)
    })
}

/// Take entry `id` out of the queue. One being checked, or already merged, stays.
pub fn remove(repo: &Path, id: &str) -> anyhow::Result<()> {
    let dir = repo_dir(repo)?;
    with_queue(&dir, |entries| {
        let e = entries
            .iter_mut()
            .find(|e| e.id.eq_ignore_ascii_case(id))
            .ok_or_else(|| anyhow::anyhow!("no queue entry {id}"))?;
        match e.state {
            QueueState::Checking => {
                anyhow::bail!("{} is being checked: remove it once its run ends", e.id)
            }
            QueueState::Merged => anyhow::bail!("{} is already merged", e.id),
            _ => {
                e.state = QueueState::Removed;
                e.reason = "removed from the queue".into();
                Ok(())
            }
        }
    })
}

/// Is `into` one of the lines (main, the integration branch, a sprint or release branch), as
/// opposed to an item branch?
fn is_line(config: &ProjectConfig, root: &Path, into: &str) -> bool {
    let main = kanbanr_core::gitmodel::main_of(config, root);
    let one = std::collections::BTreeMap::from([(into.to_string(), String::new())]);
    !kanbanr_core::gitmodel::line_branches(config, &main, &one).is_empty()
}

/// The queue's worktree: removed (forced) when dropped, whatever happened in it.
struct QueueWorktree {
    root: PathBuf,
    path: PathBuf,
}

impl QueueWorktree {
    fn add(root: &Path, id: &str, commit: &str) -> anyhow::Result<QueueWorktree> {
        let path = root
            .join(".claude")
            .join("worktrees")
            .join(id.to_ascii_lowercase());
        // One left by a run that was killed goes first.
        let _ = coderepo::git(
            root,
            &["worktree", "remove", "--force", &path.to_string_lossy()],
        );
        let _ = std::fs::remove_dir_all(&path);
        let _ = coderepo::git(root, &["worktree", "prune"]);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        coderepo::git(
            root,
            &[
                "worktree",
                "add",
                "--detach",
                &path.to_string_lossy(),
                commit,
            ],
        )?;
        Ok(QueueWorktree {
            root: root.to_path_buf(),
            path,
        })
    }
}

impl Drop for QueueWorktree {
    fn drop(&mut self) {
        let _ = coderepo::git(
            &self.root,
            &[
                "worktree",
                "remove",
                "--force",
                &self.path.to_string_lossy(),
            ],
        );
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = coderepo::git(&self.root, &["worktree", "prune"]);
    }
}

/// `kanbanr` itself with the board and project this run uses, for the steps the queue runs as
/// commands of their own (the check, the move).
fn kanbanr(store: &Store, project: &str) -> anyhow::Result<std::process::Command> {
    let exe = std::env::current_exe().context("cannot find the kanbanr binary")?;
    let mut c = std::process::Command::new(exe);
    c.arg("--data-dir").arg(store.data_dir());
    if !project.is_empty() {
        c.args(["--project", project]);
    }
    Ok(c)
}

/// The checks a check run's output names as failed: its `FAIL (exit n) <check>` lines.
fn failed_checks(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|l| l.strip_prefix("FAIL (exit "))
        .map(|l| {
            let l = l.split_once(") ").map_or(l, |(_, rest)| rest);
            // Drop the timing, `  [1.2 s]`.
            l.rsplit_once("  [")
                .map_or(l, |(c, _)| c)
                .trim()
                .to_string()
        })
        .collect()
}

/// Merge `branch` onto `cur` without touching any branch or worktree: the merge commit, or the
/// paths that conflict.
fn merge_onto(
    root: &Path,
    cur: &str,
    branch: &str,
    into: &str,
) -> anyhow::Result<Result<String, Vec<String>>> {
    let preview = coderepo::merge_preview(root, cur, branch)?;
    if !preview.clean {
        return Ok(Err(preview.conflicts));
    }
    let out = coderepo::git(root, &["merge-tree", "--write-tree", cur, branch])?;
    let tree = out.lines().next().unwrap_or_default().trim().to_string();
    let commit = coderepo::git(
        root,
        &[
            "commit-tree",
            &tree,
            "-p",
            cur,
            "-p",
            branch,
            "-m",
            &format!("Merge {branch} into {into}"),
        ],
    )?;
    Ok(Ok(commit))
}

/// Set fields of the entries named in `ids` on the queue file as it is now (others may have been
/// queued meanwhile).
fn update(dir: &Path, ids: &[String], f: impl Fn(&mut QueueEntry)) -> anyhow::Result<()> {
    with_queue(dir, |entries| {
        for e in entries.iter_mut().filter(|e| ids.contains(&e.id)) {
            f(e);
        }
        Ok(())
    })
}

/// Integrate the queue, one batch at a time (only one batch with `once`), holding the queue's run
/// lock throughout: a second runner stops at once, saying one is running.
pub fn run(store: &Store, project: &str, repo: &Path, once: bool) -> anyhow::Result<()> {
    let dir = repo_dir(repo)?;
    std::fs::create_dir_all(&dir)?;
    let lock_path = dir.join(RUN_LOCK);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    if lock.try_lock_exclusive().is_err() {
        anyhow::bail!(
            "a merge queue run is already running here ({} is held)",
            lock_path.display()
        );
    }
    let root = main_root(repo);
    let config = store
        .load_meta(project)
        .with_context(|| format!("the merge queue needs the project's board ({project})"))?
        .config;
    let effective = config.build.effective(buildrun::cpus(), &root);
    let max = usize::try_from(effective.queue_batch).unwrap_or(1);
    let mut batches = 0;
    loop {
        let entries = with_queue(&dir, |e| Ok(e.clone()))?;
        let batch: Vec<QueueEntry> = buildplan::next_batch(&entries, max)
            .into_iter()
            .map(|i| entries[i].clone())
            .collect();
        if batch.is_empty() {
            if batches == 0 {
                println!("nothing queued");
            }
            break;
        }
        batches += 1;
        run_batch(store, project, &root, &dir, &config, &batch)?;
        if once {
            break;
        }
    }
    drop(lock);
    Ok(())
}

/// One batch: merge, check, integrate or refuse.
fn run_batch(
    store: &Store,
    project: &str,
    root: &Path,
    dir: &Path,
    config: &ProjectConfig,
    batch: &[QueueEntry],
) -> anyhow::Result<()> {
    let into = batch[0].into.clone();
    let all: Vec<String> = batch.iter().map(|e| e.id.clone()).collect();
    let Ok(tip) = coderepo::rev_parse(root, &format!("refs/heads/{into}")) else {
        update(dir, &all, |e| {
            e.state = QueueState::Refused;
            e.reason = format!("{into} does not exist");
        })?;
        println!("{}: refused, {into} does not exist", all.join(", "));
        return Ok(());
    };

    // Each branch merged onto the last; a conflicting one refused and left out.
    let mut cur = tip.clone();
    let mut checking: Vec<QueueEntry> = Vec::new();
    for e in batch {
        let refuse = |reason: String| -> anyhow::Result<()> {
            println!("{} {}: refused, {reason}", e.id, e.branch);
            update(dir, std::slice::from_ref(&e.id), |x| {
                x.state = QueueState::Refused;
                x.reason = reason.clone();
            })
        };
        if !branch_exists(root, &e.branch) {
            refuse(format!("the branch {} is gone", e.branch))?;
            continue;
        }
        match merge_onto(root, &cur, &e.branch, &into)? {
            Ok(commit) => {
                cur = commit;
                checking.push(e.clone());
            }
            Err(files) => refuse(format!("conflicts with {into} in: {}", files.join(", ")))?,
        }
    }
    if checking.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = checking.iter().map(|e| e.id.clone()).collect();
    let log_path = dir.join(LOGS).join(format!("{}.log", checking[0].id));
    let log_text = log_path.display().to_string();
    update(dir, &ids, |e| {
        e.state = QueueState::Checking;
        e.log = log_text.clone();
        e.reason.clear();
    })?;

    // The check, in the queue's own worktree at the merged commit.
    let tier = if is_line(config, root, &into) {
        "line"
    } else {
        "merge"
    };
    println!(
        "{}: checking {} merged onto {into} ({tier} tier)",
        ids.join(", "),
        checking
            .iter()
            .map(|e| e.branch.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let green = {
        let wt = QueueWorktree::add(root, &checking[0].id, &cur);
        match wt {
            Ok(wt) => check_in(store, project, &wt.path, tier, &tip, &log_path),
            Err(e) => Err(e),
        }
    };
    let green = match green {
        Ok(g) => g,
        Err(e) => {
            // The run could not check: the entries wait for the next run, nothing moved.
            update(dir, &ids, |x| {
                x.state = QueueState::Queued;
                x.reason = format!("not checked: {e}");
            })?;
            return Err(e);
        }
    };

    if !green {
        if checking.len() == 1 {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            let failed = failed_checks(&log);
            let reason = if failed.is_empty() {
                "check red: see its log".to_string()
            } else {
                format!("check red: {}", failed.join("; "))
            };
            println!(
                "{} {}: refused, {reason}",
                checking[0].id, checking[0].branch
            );
            update(dir, &ids, |e| {
                e.state = QueueState::Refused;
                e.reason = reason.clone();
            })?;
        } else {
            println!(
                "{}: red together; each is checked alone next",
                ids.join(", ")
            );
            with_queue(dir, |entries| {
                let at: Vec<usize> = (0..entries.len())
                    .filter(|&i| ids.contains(&entries[i].id))
                    .collect();
                buildplan::split_red(entries, &at);
                Ok(())
            })?;
        }
        return Ok(());
    }

    if coderepo::rev_parse(root, &format!("refs/heads/{into}"))
        .ok()
        .as_deref()
        != Some(tip.as_str())
    {
        println!(
            "{}: green, but {into} moved during the check; queued again",
            ids.join(", ")
        );
        update(dir, &ids, |e| e.state = QueueState::Queued)?;
        return Ok(());
    }
    integrate(store, project, root, dir, config, &into, &cur, &checking)
}

/// Run `kanbanr build check --tier <tier> --base <tip>` in `worktree`, its output to `log`.
/// Whether it was green.
fn check_in(
    store: &Store,
    project: &str,
    worktree: &Path,
    tier: &str,
    tip: &str,
    log: &Path,
) -> anyhow::Result<bool> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out = File::create(log).with_context(|| format!("cannot write {}", log.display()))?;
    let err = out.try_clone()?;
    let status = kanbanr(store, project)?
        .args(["build", "check", "--tier", tier, "--base", tip])
        .current_dir(worktree)
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .status()
        .context("cannot run the check")?;
    Ok(status.success())
}

/// Move `into` to the checked commit. With the git model on and every entry an item branch bound
/// for its item's line, each item is moved into the integrate stage in order (`kanbanr move`, the
/// board's own merge, giving the same tree); otherwise `into` is fast-forwarded to `cur`.
#[allow(clippy::too_many_arguments)]
fn integrate(
    store: &Store,
    project: &str,
    root: &Path,
    dir: &Path,
    config: &ProjectConfig,
    into: &str,
    cur: &str,
    entries: &[QueueEntry],
) -> anyhow::Result<()> {
    let integrate_status = config.roles().status(Role::Integrate).map(str::to_string);
    let by_move = config.cadence.git_model
        && integrate_status.is_some()
        && entries.iter().all(|e| {
            !e.item.is_empty()
                && e.task.is_none()
                && line_of(store, project, root, config, &e.item) == into
        });
    if let (true, Some(status)) = (by_move, integrate_status) {
        for e in entries {
            let out = kanbanr(store, project)?
                .args(["move", &e.item, &status])
                .current_dir(root)
                .stdin(std::process::Stdio::null())
                .output()
                .context("cannot run kanbanr move")?;
            if out.status.success() {
                let sha =
                    coderepo::rev_parse(root, &format!("refs/heads/{into}")).unwrap_or_default();
                println!(
                    "{} {}: merged into {into} by moving {} to {status}",
                    e.id, e.branch, e.item
                );
                update(dir, std::slice::from_ref(&e.id), |x| {
                    x.state = QueueState::Merged;
                    x.merged = sha.clone();
                })?;
            } else {
                let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
                println!(
                    "{} {}: refused, the move was refused: {why}",
                    e.id, e.branch
                );
                update(dir, std::slice::from_ref(&e.id), |x| {
                    x.state = QueueState::Refused;
                    x.reason = format!("the move into {status} was refused: {why}");
                })?;
            }
        }
        return Ok(());
    }
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    match coderepo::fast_forward(root, into, cur) {
        Ok(_) => {
            println!(
                "{}: merged, {into} is at {}",
                ids.join(", "),
                &cur[..cur.len().min(12)]
            );
            update(dir, &ids, |e| {
                e.state = QueueState::Merged;
                e.merged = cur.to_string();
            })
        }
        Err(err) => {
            // Moved since it was read, or checked out with changes: the entries wait, and the run
            // stops rather than check them again and again against a target it cannot move.
            update(dir, &ids, |e| {
                e.state = QueueState::Queued;
                e.reason = format!("not integrated: {err}");
            })?;
            anyhow::bail!(
                "{}: green, but {into} could not be moved ({err}); they stay queued",
                ids.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failed_checks_are_read_from_a_check_runs_summary() {
        let log = "line checks against abc: the merge into the line: the full suite\n\n\
                   ok   fmt cargo fmt --all --check  [0.1 s]\n\
                   FAIL (exit 1) test sh check.sh  [0.0 s]\n\
                   2 of 3 checks passed\n";
        assert_eq!(failed_checks(log), ["test sh check.sh"]);
    }
}
