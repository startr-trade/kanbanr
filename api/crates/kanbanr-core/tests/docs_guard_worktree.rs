//! DEF-065: the Claude docs guard judges a file by the git top level of the folder it is in, so a
//! linked worktree's book, changelog, skill and fixtures are deliverables there, as they are in the
//! main checkout. Real `git worktree add` under the temporary folder (L-41); no CLI, no board.

use kanbanr_core::claude::{docs_guard_reason, docs_guard_suggestion};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// DEF-065 R-1, R-2, R-3: with the main checkout as the working directory and the fallback root,
/// a worktree under `.claude/worktrees/` and one beside the checkout are each judged by their own
/// root: the book, the changelog, the skill and a fixture pass, a stray `notes.md` is refused with
/// the suggestion `notes` and the refusal message as before.
#[test]
fn a_linked_worktree_is_judged_by_its_own_root() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("git is not on PATH: skipped");
        return;
    }
    let base = std::env::temp_dir().join(format!("kanbanr-guard-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let main = base.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(&main, &["config", "user.name", "T"]);
    git(&main, &["config", "user.email", "t@x"]);
    git(&main, &["config", "commit.gpgsign", "false"]);
    std::fs::write(main.join("empty"), "").unwrap();
    git(&main, &["add", "empty"]);
    git(&main, &["commit", "-q", "--no-verify", "-m", "initial"]);
    let under = main.join(".claude/worktrees/ftr");
    let beside = base.join("beside");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            under.to_str().unwrap(),
            "-b",
            "ftr",
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            beside.to_str().unwrap(),
            "-b",
            "beside",
        ],
    );

    for worktree in [&under, &beside] {
        for file in [
            "docs/src/book.md",
            "CHANGELOG.md",
            "skill/kanbanr/SKILL.md",
            "api/tests/fixtures/board.md",
        ] {
            let path = worktree.join(file);
            assert_eq!(
                docs_guard_suggestion(path.to_str().unwrap(), &main, &main),
                None,
                "{} should be allowed",
                path.display()
            );
        }
        let notes = worktree.join("notes.md");
        let suggestion = docs_guard_suggestion(notes.to_str().unwrap(), &main, &main);
        assert_eq!(
            suggestion.as_deref(),
            Some("notes"),
            "{} should be refused",
            notes.display()
        );
        assert!(
            docs_guard_reason(suggestion.as_deref().unwrap())
                .starts_with("Documentation for this project belongs on its board"),
            "the refusal message is unchanged"
        );
    }

    for worktree in [&under, &beside] {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force", worktree.to_str().unwrap()])
            .current_dir(&main)
            .output();
    }
    let _ = std::fs::remove_dir_all(&base);
}
