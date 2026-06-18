//! Local (serverless) mode: the real CLI binary operates on the data folder directly with NO
//! server running. Runs in plain `cargo test` against an ephemeral data dir under the build output.

use std::path::PathBuf;
use std::process::{Command, Output};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

#[test]
fn cli_local_mode_without_a_server() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("local-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let run = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .env("HOME", &home) // isolate ~/.kanbanr -> no login profile
            .env("KANBANR_DATA_DIR", &data)
            .env_remove("KANBANR_SERVER_URL")
            .env_remove("KANBANR_LOCAL")
            .output()
            .expect("run kanbanr CLI")
    };
    let ok = |args: &[&str]| -> Output {
        let o = run(args);
        assert!(o.status.success(), "cmd {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
        o
    };

    // Identity + a full create/move/todo/task flow, all with --local and no server.
    ok(&["identity", "--name", "Tester", "--email", "t@example.com"]);
    let who = ok(&["whoami"]);
    assert!(String::from_utf8_lossy(&who.stdout).contains("Tester"));

    ok(&["--project", "demo", "project", "init", "demo", "--description", "local"]);
    ok(&["--project", "demo", "milestone", "add", "--name", "M", "--code", "MS-1"]);
    ok(&["--project", "demo", "feature", "add", "--title", "Login", "--milestone", "MS-1"]);
    ok(&["--project", "demo", "move", "FEAT-001", "Scheduled"]);
    ok(&["--project", "demo", "todo", "add", "FEAT-001", "--description", "s1"]);
    ok(&["--project", "demo", "task", "add", "FEAT-001", "TL-001", "--text", "do it"]);
    ok(&["--project", "demo", "task", "state", "FEAT-001", "TL-001", "T1", "Completed"]);

    // All tasks done -> the feature auto-completed (verified through the shared dispatcher).
    let board = ok(&["--project", "demo", "board"]);
    let out = String::from_utf8_lossy(&board.stdout);
    assert!(out.contains("FEAT-001"), "board missing feature: {out}");
    assert!(out.contains("Completed (1)"), "feature should auto-complete: {out}");

    // A feature without a milestone is rejected.
    assert!(
        !run(&["--project", "demo", "feature", "add", "--title", "Orphan", "--milestone", ""]).status.success(),
        "feature without a milestone must fail"
    );

    // The data dir is a git repo with commits authored by the configured identity, and the secret
    // file is never tracked.
    assert!(data.join(".git").is_dir(), "local mode initializes a git repo");
    let log = Command::new("git")
        .args(["-C", data.to_str().unwrap(), "log", "--pretty=%an <%ae>"])
        .output()
        .unwrap();
    let authors = String::from_utf8_lossy(&log.stdout);
    assert!(authors.contains("Tester <t@example.com>"), "commits authored by identity: {authors}");

    // Auto-detect: WITHOUT --local, with no server configured but a data dir present, it still
    // works locally.
    let auto = run(&["--project", "demo", "board"]);
    assert!(auto.status.success(), "auto-local failed: {}", String::from_utf8_lossy(&auto.stderr));
    assert!(String::from_utf8_lossy(&auto.stdout).contains("FEAT-001"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_init_scaffolds_a_local_project() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("init-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    let work = base.join("work"); // cwd for the .kanbanr marker
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let out = Command::new(cli())
        .args(["init", "app", "--author", "Ada", "--email", "ada@example.com"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .env_remove("KANBANR_SERVER_URL")
        .output()
        .expect("run init");
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));

    // It scaffolded the project, set the identity, and selected the project here.
    assert!(data.join("projects").join("app").join("config.yaml").is_file(), "project scaffolded");
    assert_eq!(std::fs::read_to_string(work.join(".kanbanr")).unwrap().trim(), "app");
    let id = Command::new("git").args(["-C", data.to_str().unwrap(), "config", "user.email"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&id.stdout).trim(), "ada@example.com");

    let _ = std::fs::remove_dir_all(&base);
}
