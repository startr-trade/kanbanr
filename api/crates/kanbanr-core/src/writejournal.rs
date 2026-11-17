//! The write journal (DEF-102). A recording write notes, in the git directory, what the board
//! looked like before it began. If the process dies before its commit lands, the next write
//! (which holds the write lock) reads the note and either finds the commit made, or puts the
//! dead write's changes back. The journal is machine-local, lives in the git directory, and is
//! never part of the tree.

use crate::git;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The journal's file name inside the git directory.
const JOURNAL_FILE: &str = "kanbanr-write.json";
/// How deep recovery looks for the atomic write's leftover temporary files.
/// The op an unreadable journal is reported with.
const UNREADABLE: &str = "(unreadable journal)";
const LEFTOVER_DEPTH: usize = 8;

/// What a write in flight records about itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Journal {
    pub version: u32,
    pub pid: u32,
    pub started_at: String,
    /// HEAD before the write, 40 hex; empty on an unborn branch.
    pub head: String,
    /// The method and path only, never the body.
    pub op: String,
    /// The paths that were already changed when the write began.
    pub dirty_before: Vec<String>,
}

/// What `recover` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    None,
    /// The dead write had committed; only its journal was left.
    Committed {
        op: String,
    },
    /// The dead write had not committed; these paths were put back.
    RolledBack {
        op: String,
        paths: Vec<String>,
    },
}

/// Where the journal of the board at `data_dir` lives: `<git dir>/kanbanr-write.json`.
#[must_use]
pub fn path(data_dir: &Path) -> PathBuf {
    git::git_dir(data_dir).join(JOURNAL_FILE)
}

/// Write the journal, by rename.
pub fn begin(data_dir: &Path, j: &Journal) -> std::io::Result<()> {
    let text = serde_json::to_vec(j).map_err(std::io::Error::other)?;
    crate::atomic::write(path(data_dir), text)
}

/// Remove the journal; a missing one is fine.
pub fn end(data_dir: &Path) {
    let _ = std::fs::remove_file(path(data_dir));
}

/// The journal a dead write left, if any. An unreadable file comes back as a journal whose op is
/// "(unreadable journal)", so that recovery still runs.
#[must_use]
pub fn pending(data_dir: &Path) -> Option<Journal> {
    let text = match std::fs::read(path(data_dir)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(unreadable()),
    };
    Some(serde_json::from_slice(&text).unwrap_or_else(|_| unreadable()))
}

fn unreadable() -> Journal {
    Journal {
        version: 1,
        pid: 0,
        started_at: String::new(),
        head: String::new(),
        op: UNREADABLE.to_string(),
        dirty_before: Vec::new(),
    }
}

/// Finish a dead write's business. Call it holding the write lock.
///
/// No journal: `None`. HEAD moved since the journal: the dead write committed and died before it
/// ended; the journal is removed. Otherwise the write never committed: what it changed (the
/// changed paths less those that were changed before it began) is put back to HEAD, its
/// temporary files are removed and the journal ends.
pub fn recover(data_dir: &Path) -> std::io::Result<Recovery> {
    let Some(j) = pending(data_dir) else {
        return Ok(Recovery::None);
    };
    let head = git::head_commit(data_dir).unwrap_or_default();
    // An unreadable journal names no HEAD, so it is treated as a write that never committed.
    if j.op != UNREADABLE && head != j.head {
        end(data_dir);
        return Ok(Recovery::Committed { op: j.op });
    }
    let paths: Vec<String> = git::changed_paths(data_dir)
        .into_iter()
        .filter(|p| !j.dirty_before.contains(p))
        .collect();
    // Temporary files first, so that a folder holding only one is empty when its file goes.
    remove_leftovers(data_dir, 0);
    git::restore_to_head(data_dir, &paths).map_err(std::io::Error::other)?;
    end(data_dir);
    Ok(Recovery::RolledBack { op: j.op, paths })
}

/// Remove the atomic write's temporary files below `dir`, apart from the git directory.
fn remove_leftovers(dir: &Path, depth: usize) {
    for f in crate::atomic::leftovers(dir, std::time::Duration::ZERO) {
        let _ = std::fs::remove_file(f);
    }
    if depth >= LEFTOVER_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        if name == ".git" || name == ".sessions" {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_leftovers(&e.path(), depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanbanr-wj-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git::ensure_repo_as(&dir, Some(("Ada Lovelace", "ada@example.com")));
        let repo = git2::Repository::open(&dir).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Ada Lovelace").unwrap();
        cfg.set_str("user.email", "ada@example.com").unwrap();
        std::fs::write(dir.join("item.yaml"), "status: a\n").unwrap();
        assert!(git::commit_local(&dir, "seed"));
        dir
    }

    fn journal(dir: &Path, dirty_before: Vec<String>) -> Journal {
        Journal {
            version: 1,
            pid: std::process::id(),
            started_at: "2026-11-17T00:00:00Z".to_string(),
            head: git::head_commit(dir).unwrap_or_default(),
            op: "PUT /projects/p/features/X-1/tasks/T1".to_string(),
            dirty_before,
        }
    }

    /// DEF-102 R-5: a write that committed and died before it ended leaves only its journal; the
    /// commit stays and nothing is put back.
    #[test]
    fn recover_after_a_commit_only_clears_the_journal() {
        let dir = board("committed");
        begin(&dir, &journal(&dir, vec![])).unwrap();
        std::fs::write(dir.join("item.yaml"), "status: b\n").unwrap();
        assert!(git::commit_local(&dir, "the dead write's commit"));
        let head = git::head_commit(&dir);
        assert_eq!(
            recover(&dir).unwrap(),
            Recovery::Committed {
                op: "PUT /projects/p/features/X-1/tasks/T1".into()
            }
        );
        assert_eq!(git::head_commit(&dir), head);
        assert_eq!(
            std::fs::read_to_string(dir.join("item.yaml")).unwrap(),
            "status: b\n"
        );
        assert!(pending(&dir).is_none());
        assert_eq!(recover(&dir).unwrap(), Recovery::None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-5: a write that never committed has only its own changes put back; a path that
    /// was already changed before it began is left as it was.
    #[test]
    fn recover_restores_only_what_the_dead_write_changed() {
        let dir = board("rolled");
        std::fs::write(dir.join("other.yaml"), "hand edit\n").unwrap();
        begin(&dir, &journal(&dir, vec!["other.yaml".to_string()])).unwrap();
        std::fs::write(dir.join("item.yaml"), "status: dead\n").unwrap();
        std::fs::create_dir_all(dir.join("activity")).unwrap();
        std::fs::write(dir.join("activity").join("day.yaml"), "x\n").unwrap();
        std::fs::write(dir.join("activity").join(".day.yaml.1.2.tmp"), "half").unwrap();
        let got = recover(&dir).unwrap();
        let Recovery::RolledBack { op, paths } = got else {
            panic!("expected RolledBack, got {got:?}");
        };
        assert!(op.starts_with("PUT "));
        assert_eq!(paths, ["activity/day.yaml", "item.yaml"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("item.yaml")).unwrap(),
            "status: a\n"
        );
        assert!(
            !dir.join("activity").exists(),
            "the folder it made goes too"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("other.yaml")).unwrap(),
            "hand edit\n",
            "a change made before the write stays"
        );
        assert!(pending(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-102 R-5: the journal sits in the git directory (a linked worktree's too) and git never
    /// lists it.
    #[test]
    fn the_journal_lives_in_the_git_dir_and_is_never_tracked() {
        let dir = board("place");
        begin(&dir, &journal(&dir, vec![])).unwrap();
        assert_eq!(path(&dir), dir.join(".git").join("kanbanr-write.json"));
        assert!(path(&dir).is_file());
        assert!(git::changed_paths(&dir).is_empty());
        assert_eq!(
            pending(&dir).unwrap().op,
            "PUT /projects/p/features/X-1/tasks/T1"
        );
        end(&dir);
        end(&dir);
        assert!(pending(&dir).is_none());

        // A linked worktree keeps its journal in its own git directory.
        let linked = std::env::temp_dir().join(format!("kanbanr-wj-linked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&linked);
        let repo = git2::Repository::open(&dir).unwrap();
        repo.worktree("wt", &linked, None).unwrap();
        let gd = git::git_dir(&linked);
        assert!(gd.starts_with(dir.join(".git").join("worktrees")), "{gd:?}");
        begin(&linked, &journal(&linked, vec![])).unwrap();
        assert_eq!(path(&linked), gd.join("kanbanr-write.json"));
        assert!(path(&linked).is_file());
        assert!(!path(&dir).exists());
        assert!(git::changed_paths(&linked).is_empty());
        let _ = std::fs::remove_dir_all(&linked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unreadable journal still recovers.
    #[test]
    fn an_unreadable_journal_still_recovers() {
        let dir = board("unreadable");
        std::fs::write(path(&dir), "{ not json").unwrap();
        std::fs::write(dir.join("item.yaml"), "status: dead\n").unwrap();
        let got = recover(&dir).unwrap();
        assert!(
            matches!(&got, Recovery::RolledBack { op, .. } if op == "(unreadable journal)"),
            "{got:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("item.yaml")).unwrap(),
            "status: a\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
