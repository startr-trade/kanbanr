//! Repository docs read at a position (FTR-004): a doc's text, its blob and the tree under
//! `docs/`, at a [`Picked`] position, with nothing checked out.
//!
//! Every read goes through FTR-003's [`position::read_at`] and [`position::tree_at`], so the bytes
//! shown, the files listed and the blob a sign-off pins are the bytes the position hashes, on every
//! checkout, a CRLF one included (DEF-066). Nothing here writes or checks out.

use crate::error::{CoreError, Result};
use crate::picks::Picked;
use crate::position::{self, WorkingState};
use serde::Serialize;
use std::path::Path;

/// A repository doc at a position: its text, or why it is not there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RepoDoc {
    #[serde(flatten)]
    pub at: Picked,
    pub path: String,
    /// The doc's text (lossy UTF-8); None when it is missing at the position.
    pub text: Option<String>,
    /// The blob id of the bytes read: the id the position recorded, from git or disk.
    pub blob: Option<String>,
    pub missing: bool,
    /// Why there is no text; empty when there is.
    pub why: String,
    /// For a missing doc, whether main has it; None otherwise.
    pub at_main: Option<bool>,
}

/// The `.md` files under a root at a position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RepoTree {
    #[serde(flatten)]
    pub at: Picked,
    pub root: String,
    pub files: Vec<String>,
}

/// [`position::read_at`], with one more way git says a path is absent at a commit: when the
/// checkout has the file, `cat-file` answers "exists on disk, but not in '<commit>'", which
/// `read_at` passes through as an error. Here it is a missing doc, as any other absence is.
fn read_at(repo: &Path, state: &WorkingState, path: &str) -> Result<Option<Vec<u8>>> {
    match position::read_at(repo, state, path) {
        Err(CoreError::Git(why)) if why.contains("exists on disk, but not in") => Ok(None),
        other => other,
    }
}

fn checked(path: &str) -> Result<&str> {
    crate::releaseprep::check_repo_path(path).map_err(CoreError::Unsupported)?;
    Ok(path.trim())
}

/// The doc `path` of the repository `repo` at `at`. A doc missing there says so, naming the
/// position, and whether `main` (main's position) has it. A path outside the repository is refused
/// before anything is read; a file that changed after the position was read is "position moved".
pub fn read(repo: &Path, at: &Picked, path: &str, main: &WorkingState) -> Result<RepoDoc> {
    let path = checked(path)?;
    let (text, blob, missing, why, at_main) = match read_at(repo, &at.state, path)? {
        Some(bytes) => (
            Some(String::from_utf8_lossy(&bytes).into_owned()),
            Some(position::blob_id(&bytes)),
            false,
            String::new(),
            None,
        ),
        None => (
            None,
            None,
            true,
            format!("{path} is not in the repository at {}", at.label),
            Some(read_at(repo, main, path)?.is_some()),
        ),
    };
    Ok(RepoDoc {
        at: at.clone(),
        path: path.to_string(),
        text,
        blob,
        missing,
        why,
        at_main,
    })
}

/// The bytes of `path` at `at`, for images; None when it is not there.
pub fn read_bytes(repo: &Path, at: &Picked, path: &str) -> Result<Option<Vec<u8>>> {
    let path = checked(path)?;
    read_at(repo, &at.state, path)
}

/// The `.md` files under `root` (`docs/`) at `at`, sorted.
pub fn tree(repo: &Path, at: &Picked, root: &str) -> Result<RepoTree> {
    if !root.trim_matches('/').is_empty() {
        checked(root)?;
    }
    let files = position::tree_at(repo, &at.state, root)?
        .into_iter()
        .map(|(p, _)| p)
        .filter(|p| p.ends_with(".md"))
        .collect();
    Ok(RepoTree {
        at: at.clone(),
        root: root.to_string(),
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coderepo::tests::{commit_file, scratch_repo};
    use crate::coderepo::{git, rev_parse};
    use crate::picks::Source;
    use crate::position::PositionRef;
    use std::path::PathBuf;

    fn at_commit(commit: &str, label: &str) -> Picked {
        Picked {
            pick: commit.to_string(),
            kind: "commit",
            state: WorkingState {
                position: PositionRef::at_commit("demo", commit),
                changed: vec![],
            },
            source: Source::Git,
            label: label.to_string(),
            resolved_from: "commit".into(),
        }
    }

    fn at_worktree(repo: &Path) -> Picked {
        let state = position::read_position(repo, "demo").unwrap();
        Picked {
            pick: "worktree".into(),
            kind: "worktree",
            label: format!("working tree @ {}", state.position.short()),
            state,
            source: Source::Disk,
            resolved_from: "HEAD and the uncommitted changes".into(),
        }
    }

    fn main_state(repo: &Path) -> WorkingState {
        at_commit(&rev_parse(repo, "main").unwrap(), "main").state
    }

    /// What a read must leave unchanged: every ref and the checkout's status.
    fn snapshot(repo: &Path) -> (String, String) {
        (
            git(repo, &["for-each-ref"]).unwrap(),
            git(repo, &["status", "--porcelain"]).unwrap(),
        )
    }

    fn repo_with_doc(tag: &str) -> PathBuf {
        let repo = scratch_repo(&format!("repodocs-{tag}"));
        assert!(repo.starts_with(std::env::temp_dir()));
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        commit_file(&repo, "docs/guide/x.md", "on main\n", "doc");
        repo
    }

    /// FTR-004 R-2: a doc at a commit is the commit's text, not the checkout's; nothing changes.
    #[test]
    fn a_doc_is_read_from_git_at_a_commit_with_nothing_checked_out() {
        let repo = repo_with_doc("git");
        let main = rev_parse(&repo, "main").unwrap();
        git(&repo, &["checkout", "-q", "-b", "other"]).unwrap();
        commit_file(&repo, "docs/guide/x.md", "on other\n", "other");
        std::fs::write(repo.join("docs/guide/x.md"), "uncommitted\n").unwrap();
        let before = snapshot(&repo);
        let at = at_commit(&main, &format!("main @ {}", &main[..7]));
        let doc = read(&repo, &at, "docs/guide/x.md", &main_state(&repo)).unwrap();
        assert_eq!(doc.text.as_deref(), Some("on main\n"));
        assert_eq!(doc.blob, Some(position::blob_id(b"on main\n")));
        assert!(!doc.missing && doc.why.is_empty() && doc.at_main.is_none());
        assert_eq!(doc.at.source, Source::Git);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["source"], "git");
        assert_eq!(json["position"]["commit"], serde_json::json!(main));
        assert_eq!(json["path"], "docs/guide/x.md");
        assert_eq!(
            read_bytes(&repo, &at, "docs/guide/x.md").unwrap().unwrap(),
            b"on main\n"
        );
        let t = tree(&repo, &at, "docs/").unwrap();
        assert_eq!(t.files, ["docs/guide/x.md"]);
        assert_eq!(snapshot(&repo), before, "a read changes nothing");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-2: the working tree is read from disk, in its repository form, and says so: on a
    /// CRLF checkout the blob is the id the position recorded.
    #[test]
    fn a_doc_at_the_working_tree_is_read_from_disk_and_says_so() {
        let repo = repo_with_doc("disk");
        git(&repo, &["config", "core.autocrlf", "true"]).unwrap();
        std::fs::write(repo.join("docs/guide/x.md"), "edited\r\non disk\r\n").unwrap();
        std::fs::write(repo.join("docs/new.md"), "new\r\n").unwrap();
        let before = snapshot(&repo);
        let at = at_worktree(&repo);
        assert_eq!(at.source, Source::Disk);
        assert!(at.state.position.dirty.is_some());
        let recorded = |p: &str| {
            at.state
                .changed
                .iter()
                .find(|(c, _)| c == p)
                .and_then(|(_, id)| id.clone())
                .unwrap()
        };
        let main = main_state(&repo);
        let doc = read(&repo, &at, "docs/guide/x.md", &main).unwrap();
        assert_eq!(doc.text.as_deref(), Some("edited\non disk\n"));
        assert_eq!(
            doc.blob.as_deref(),
            Some(recorded("docs/guide/x.md").as_str())
        );
        assert_eq!(serde_json::to_value(&doc).unwrap()["source"], "disk");
        let new = read(&repo, &at, "docs/new.md", &main).unwrap();
        assert_eq!(new.blob.as_deref(), Some(recorded("docs/new.md").as_str()));
        let t = tree(&repo, &at, "docs/").unwrap();
        assert_eq!(t.files, ["docs/guide/x.md", "docs/new.md"]);
        assert_eq!(snapshot(&repo), before);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-2: a path that leaves the repository is refused before anything is read.
    #[test]
    fn a_path_outside_the_repository_is_refused() {
        let repo = repo_with_doc("outside");
        let at = at_commit(&rev_parse(&repo, "main").unwrap(), "main");
        let main = main_state(&repo);
        for bad in [
            "../../etc/passwd",
            "docs/../../x.md",
            "/etc/passwd",
            "C:/x.md",
            ".git/config",
            "",
        ] {
            assert!(
                matches!(read(&repo, &at, bad, &main), Err(CoreError::Unsupported(_))),
                "{bad}"
            );
            assert!(
                matches!(read_bytes(&repo, &at, bad), Err(CoreError::Unsupported(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            tree(&repo, &at, "../"),
            Err(CoreError::Unsupported(_))
        ));
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-4: a doc missing at a position says so, naming the position, and whether main has
    /// it.
    #[test]
    fn a_doc_missing_at_a_position_says_so_and_whether_main_has_it() {
        let repo = repo_with_doc("missing");
        let old = rev_parse(&repo, "main~1").unwrap();
        let at = at_commit(&old, "v0.6.0 · tag v0.6.0 @ 9fe1d20");
        let main = main_state(&repo);
        let doc = read(&repo, &at, "docs/guide/x.md", &main).unwrap();
        assert!(doc.missing);
        assert!(doc.text.is_none() && doc.blob.is_none());
        assert_eq!(
            doc.why,
            "docs/guide/x.md is not in the repository at v0.6.0 · tag v0.6.0 @ 9fe1d20"
        );
        assert_eq!(doc.at_main, Some(true));
        let doc = read(&repo, &at, "docs/system/store.md", &main).unwrap();
        assert!(doc.missing);
        assert_eq!(doc.at_main, Some(false));
        assert!(read_bytes(&repo, &at, "docs/guide/x.md").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-9: a read that fails is an answer naming the reason, and changes nothing.
    #[test]
    fn a_failed_read_changes_nothing_and_names_the_reason() {
        let repo = repo_with_doc("failed");
        std::fs::write(repo.join("docs/guide/x.md"), "first edit\n").unwrap();
        let at = at_worktree(&repo);
        std::fs::write(repo.join("docs/guide/x.md"), "second edit\n").unwrap();
        let main = main_state(&repo);
        let before = snapshot(&repo);
        let e = read(&repo, &at, "docs/guide/x.md", &main)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("position moved") && e.contains("docs/guide/x.md"),
            "{e}"
        );
        assert_eq!(snapshot(&repo), before);
        // A commit the repository does not hold: git's reason, nothing changed.
        let gone = at_commit("0123456789abcdef0123456789abcdef01234567", "gone");
        let e = tree(&repo, &gone, "docs/").unwrap_err().to_string();
        assert!(!e.is_empty());
        assert_eq!(snapshot(&repo), before);
        // A refused path: the reason names it.
        let e = read(&repo, &gone, "../x.md", &main)
            .unwrap_err()
            .to_string();
        assert!(e.contains("../x.md"), "{e}");
        assert_eq!(snapshot(&repo), before);
        assert_eq!(
            std::fs::read_to_string(repo.join("docs/guide/x.md")).unwrap(),
            "second edit\n"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }
}
