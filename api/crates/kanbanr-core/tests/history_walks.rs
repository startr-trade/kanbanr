//! MNT-014: a whole read of the board asks its history once.
//!
//! Each test builds its own board and code repository under the system temporary folder (L-41),
//! reads through the public entry points, and counts the full walks of that board's repository
//! (`git::path_history_walks_of`), so tests running side by side do not see each other's walks.

use std::path::{Path, PathBuf};
use std::process::Command;

use kanbanr_core::Store;
use kanbanr_core::config::{Gate, ProjectConfig};
use kanbanr_core::git::{commit, ensure_repo_as, path_history_walks_of};
use kanbanr_core::models::FeatureDefinition;

fn scratch(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kanbanr-history-walks-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(dir.starts_with(std::env::temp_dir()));
    dir
}

fn git(dir: &Path, args: &[&str], date: Option<&str>) {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    if let Some(d) = date {
        cmd.env("GIT_COMMITTER_DATE", d).env("GIT_AUTHOR_DATE", d);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fixture {
    store: Store,
    dir: PathBuf,
    repo: PathBuf,
    codes: Vec<String>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.repo);
    }
}

/// A board of `items` items waiting on `docs-folded`, each linked from `texts` docs that name a
/// flag the item changed in the code repository, committed in the board's own repository.
fn docs_folded_board(items: usize, texts: usize) -> Fixture {
    let dir = scratch("board");
    ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
    let store = Store::new(dir.clone());
    store
        .init_project("demo", ProjectConfig::default_for("demo"))
        .unwrap();
    store
        .add_milestone("demo", "M", "", vec![], Some("M".into()))
        .unwrap();
    kanbanr_core::charter::save(
        &store,
        "demo",
        &kanbanr_core::charter::Charter {
            purpose: "p".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut gates = std::collections::BTreeMap::new();
    gates.insert(
        "Completed".to_string(),
        Gate {
            signoffs: vec!["docs-folded".into()],
            ..Default::default()
        },
    );
    store.set_gates("demo", gates).unwrap();
    let mut codes = Vec::new();
    for i in 0..items {
        let f = store
            .add_feature("demo", &format!("Item {i}"), "# x", "M", None)
            .unwrap();
        let def: FeatureDefinition = kanbanr_core::yaml::from_str(
            "statement: s\nrequirements:\n  - kind: functional\n    text: THE SYSTEM SHALL x.\n    tests:\n      - name: t::x\n        kind: unit\n        state: planned\n",
        )
        .unwrap();
        store
            .set_feature_definition("demo", &f.code, Some(def))
            .unwrap();
        store.approve_feature("demo", &f.code, "Ada").unwrap();
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        let list = store.add_todo_list("demo", &f.code, "work", None).unwrap();
        store
            .add_task(
                "demo",
                &f.code,
                &list.code,
                "one",
                None,
                &kanbanr_core::store::TaskFields::default(),
            )
            .unwrap();
        store
            .set_task_state(
                "demo",
                &f.code,
                &list.code,
                "T1",
                kanbanr_core::models::TaskState::Completed,
            )
            .unwrap();
        // Finishing the work moved it on; it waits at Scheduled for its sign-offs.
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        for j in 0..texts {
            store
                .write_doc(
                    "demo",
                    &format!("design/d{i}-{j}.md"),
                    &format!(
                        "---\nitems: [{}]\n---\n\n# D\n\nRun with --flag-{i}-{j}.\n",
                        f.code
                    ),
                )
                .unwrap();
        }
        codes.push(f.code);
    }
    assert!(commit(&dir, "Ada", "ada@example.com", "board"));
    let repo = scratch("code");
    git(&repo, &["init", "-q", "-b", "main"], None);
    for (i, code) in codes.iter().enumerate() {
        let body: String = (0..texts)
            .map(|j| format!("let a{j} = \"--flag-{i}-{j}\";\n"))
            .collect();
        let file = repo.join(format!("src/f{i}.rs"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, body).unwrap();
        git(&repo, &["add", "-A"], None);
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-m",
                &format!("add\n\nRefs: kanbanr:{code}"),
            ],
            Some("2099-01-01T00:00:00+00:00"),
        );
    }
    Fixture {
        store,
        dir,
        repo,
        codes,
    }
}

/// MNT-014 R-1: `kanbanr check` over four items, three of them at `docs-folded` and each linking
/// four texts that name something it changed, walks the board's history once.
#[test]
fn check_walks_board_history_once_for_every_item() {
    let fx = docs_folded_board(3, 4);
    let extra = fx
        .store
        .add_feature("demo", "Plain", "# x", "M", None)
        .unwrap();
    assert!(commit(&fx.dir, "Ada", "ada@example.com", "plain"));
    let before = path_history_walks_of(&fx.dir);
    let report =
        kanbanr_core::readiness::check_board(&fx.store, "demo", None, Some(&fx.repo)).unwrap();
    assert_eq!(path_history_walks_of(&fx.dir) - before, 1);
    assert_eq!(report.items.len(), 4);
    for code in &fx.codes {
        let item = report.items.iter().find(|i| &i.code == code).unwrap();
        assert_eq!(item.texts_to_settle.len(), 4, "{code}: {item:?}");
    }
    let plain = report.items.iter().find(|i| i.code == extra.code).unwrap();
    assert!(plain.texts_to_settle.is_empty());
}

/// MNT-014 R-1: settling one text (`kanbanr prepare text`) walks the board's history once.
#[test]
fn prepare_text_walks_history_once() {
    let fx = docs_folded_board(1, 4);
    let code = fx.codes[0].clone();
    let before = path_history_walks_of(&fx.dir);
    let made = kanbanr_core::prepared::prepare(
        &fx.store,
        "demo",
        &serde_json::json!({
            "kind": "text-check",
            "subject": {"item": code, "target": "doc:design/d0-0.md"},
            "fields": {"outcome": "confirmed", "why": "--flag-0-0 is still a flag"},
            "origin": "conversation",
        }),
        Some(&fx.repo),
    );
    made.unwrap();
    assert_eq!(path_history_walks_of(&fx.dir) - before, 1);
}

/// MNT-014 R-1: the doctor walks the board's history once, however many items' docs it judges,
/// and a second run at the same revision walks none.
#[test]
fn doctor_walks_board_history_once() {
    let fx = docs_folded_board(3, 4);
    let before = path_history_walks_of(&fx.dir);
    kanbanr_core::doctor::run_project_in(&fx.store, "demo", Some(&fx.repo)).unwrap();
    assert!(
        path_history_walks_of(&fx.dir) - before <= 1,
        "walks: {}",
        path_history_walks_of(&fx.dir) - before
    );
    let again = path_history_walks_of(&fx.dir);
    kanbanr_core::doctor::run_project_in(&fx.store, "demo", Some(&fx.repo)).unwrap();
    assert_eq!(path_history_walks_of(&fx.dir), again);
}

/// MNT-014 R-2: the one-context check reports what asking each item alone reported.
#[test]
fn check_answers_as_before() {
    let fx = docs_folded_board(3, 4);
    let report =
        kanbanr_core::readiness::check_board(&fx.store, "demo", None, Some(&fx.repo)).unwrap();
    assert_eq!(report.items.len(), 3);
    for item in &report.items {
        let alone = kanbanr_core::docdrift::flags_for_item(&fx.store, "demo", &item.code, &fx.repo)
            .map(|flags| {
                kanbanr_core::review::split_settled(&fx.store, "demo", &item.code, flags).0
            })
            .unwrap_or_default()
            .iter()
            .map(|f| f.target.key())
            .collect::<Vec<_>>();
        assert!(!alone.is_empty(), "{}", item.code);
        assert_eq!(item.texts_to_settle, alone, "{}", item.code);
    }
    // One scoped item reads the same as it did in the whole run.
    let one =
        kanbanr_core::readiness::check_board(&fx.store, "demo", Some(&fx.codes[1]), Some(&fx.repo))
            .unwrap();
    let whole = report.items.iter().find(|i| i.code == fx.codes[1]).unwrap();
    assert_eq!(one.items[0].texts_to_settle, whole.texts_to_settle);
}
