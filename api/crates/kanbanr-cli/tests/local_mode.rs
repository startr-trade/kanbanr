//! Local (serverless) mode: the real CLI binary operates on the data folder directly with NO
//! server running. Runs in plain `cargo test` against an ephemeral data dir under the build output.

use std::path::PathBuf;
use std::process::{Command, Output};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

#[test]
fn cli_local_mode_without_a_server() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("local-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let run = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR") // isolate ~/.kanbanr -> no login profile
            .env("KANBANR_DATA_DIR", &data)
            .env_remove("KANBANR_SERVER_URL")
            .env_remove("KANBANR_LOCAL")
            .output()
            .expect("run kanbanr CLI")
    };
    let ok = |args: &[&str]| -> Output {
        let o = run(args);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        o
    };

    // Identity + a full create/move/todo/task flow, all with --local and no server.
    ok(&["identity", "--name", "Tester", "--email", "t@example.com"]);
    let who = ok(&["whoami"]);
    assert!(String::from_utf8_lossy(&who.stdout).contains("Tester"));

    ok(&[
        "--project",
        "demo",
        "project",
        "init",
        "demo",
        "--description",
        "local",
    ]);
    ok(&[
        "--project",
        "demo",
        "milestone",
        "add",
        "--name",
        "M",
        "--code",
        "MS-1",
    ]);
    ok(&[
        "--project",
        "demo",
        "feature",
        "add",
        "--title",
        "Login",
        "--milestone",
        "MS-1",
    ]);
    ok(&["--project", "demo", "move", "FEAT-001", "Scheduled"]);
    ok(&[
        "--project",
        "demo",
        "todo",
        "add",
        "FEAT-001",
        "--description",
        "s1",
    ]);
    ok(&[
        "--project",
        "demo",
        "task",
        "add",
        "FEAT-001",
        "TL-001",
        "--text",
        "do it",
    ]);
    ok(&[
        "--project",
        "demo",
        "task",
        "state",
        "FEAT-001",
        "TL-001",
        "T1",
        "Completed",
    ]);

    // All tasks done -> the feature auto-completed (verified through the shared dispatcher).
    let board = ok(&["--project", "demo", "board"]);
    let out = String::from_utf8_lossy(&board.stdout);
    assert!(out.contains("FEAT-001"), "board missing feature: {out}");
    assert!(
        out.contains("Completed (1)"),
        "feature should auto-complete: {out}"
    );

    // A feature without a milestone is rejected.
    assert!(
        !run(&[
            "--project",
            "demo",
            "feature",
            "add",
            "--title",
            "Orphan",
            "--milestone",
            ""
        ])
        .status
        .success(),
        "feature without a milestone must fail"
    );

    // The data dir is a git repo with commits authored by the configured identity, and the secret
    // file is never tracked.
    assert!(
        data.join(".git").is_dir(),
        "local mode initializes a git repo"
    );
    let log = Command::new("git")
        .args(["-C", data.to_str().unwrap(), "log", "--pretty=%an <%ae>"])
        .output()
        .unwrap();
    let authors = String::from_utf8_lossy(&log.stdout);
    assert!(
        authors.contains("Tester <t@example.com>"),
        "commits authored by identity: {authors}"
    );

    // Auto-detect: WITHOUT --local, with no server configured but a data dir present, it still
    // works locally.
    let auto = run(&["--project", "demo", "board"]);
    assert!(
        auto.status.success(),
        "auto-local failed: {}",
        String::from_utf8_lossy(&auto.stderr)
    );
    assert!(String::from_utf8_lossy(&auto.stdout).contains("FEAT-001"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_init_scaffolds_a_local_project() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("init-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    let work = base.join("work"); // cwd for the .kanbanr marker
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let out = Command::new(cli())
        .args([
            "init",
            "app",
            "--author",
            "Ada",
            "--email",
            "ada@example.com",
        ])
        .current_dir(&work)
        .env("HOME", &home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("KANBANR_DATA_DIR", &data)
        .env_remove("KANBANR_SERVER_URL")
        .output()
        .expect("run init");
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // It scaffolded the project, set the identity, and selected the project here.
    assert!(
        data.join("projects")
            .join("app")
            .join("config.yaml")
            .is_file(),
        "project scaffolded"
    );
    assert_eq!(
        std::fs::read_to_string(work.join(".kanbanr"))
            .unwrap()
            .trim(),
        "app"
    );
    let id = Command::new("git")
        .args(["-C", data.to_str().unwrap(), "config", "user.email"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&id.stdout).trim(),
        "ada@example.com"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_init_puts_the_board_next_to_the_git_repo_and_finds_it_from_subfolders() {
    // Outside the build dir (which lives in kanbanr's own repo) so nothing here is nested.
    let base = std::env::temp_dir().join(format!("kanbanr-sibling-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let repo = base.join("code").join("app");
    let sub = repo.join("src").join("deep");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    let git_init = Command::new("git")
        .args(["init", "-q", repo.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(git_init.status.success());

    let run = |cwd: &std::path::Path, args: &[&str]| -> Output {
        let o = Command::new(cli())
            .args(args)
            .current_dir(cwd)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("KANBANR_SERVER_URL")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        o
    };

    // Non-interactive init takes the recommended sibling of the git root.
    let out = run(
        &repo,
        &[
            "init",
            "app",
            "--author",
            "Ada",
            "--email",
            "ada@example.com",
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains('⚠'),
        "unexpected nesting warning: {stdout}"
    );
    let board = std::fs::canonicalize(base.join("code").join("app.kanbanr")).unwrap();
    assert!(board.join("projects/app/config.yaml").is_file());
    assert!(board.join(".git").is_dir(), "board is its own git repo");
    assert!(!repo.join("data").exists(), "no board inside the project");

    let marker = std::fs::read_to_string(repo.join(".kanbanr")).unwrap();
    assert!(marker.contains("project: app"), "{marker}");
    assert!(marker.contains("data_dir: ../app.kanbanr"), "{marker}");

    // From a subfolder, the marker is found and the board resolves.
    let w = run(&sub, &["where"]);
    assert_eq!(
        String::from_utf8_lossy(&w.stdout).trim(),
        board.display().to_string()
    );
    run(&sub, &["milestone", "add", "--name", "M1"]);
    let ms = run(&sub, &["milestone", "list"]);
    assert!(String::from_utf8_lossy(&ms.stdout).contains("M1"));

    let j: serde_json::Value =
        serde_json::from_slice(&run(&sub, &["where", "--json"]).stdout).unwrap();
    assert_eq!(j["source"], "marker");
    assert_eq!(j["project"], "app");
    assert_eq!(j["exists"], true);
    assert!(j["inside_git_repo"].is_null());

    // `project use` keeps the recorded data folder.
    run(&repo, &["project", "use", "app"]);
    let marker = std::fs::read_to_string(repo.join(".kanbanr")).unwrap();
    assert!(marker.contains("data_dir: ../app.kanbanr"), "{marker}");

    // The project repo only gains the marker.
    let status = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&status.stdout).trim(),
        "?? .kanbanr"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_import_preview_apply_reimport_and_missing_sources() {
    let base = std::env::temp_dir().join(format!("kanbanr-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let repo = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("TODO.md"), "- [ ] Add cart page\n").unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@x",
        "commit",
        "-qm",
        "init",
    ]);
    let head = git(&["rev-parse", "HEAD"]);

    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&repo)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    run(&["init", "shop", "--author", "Ada", "--email", "a@x"]);
    let board = base.join("code").join("shop.kanbanr");
    let commits = || {
        let o = Command::new("git")
            .args(["-C", board.to_str().unwrap(), "rev-list", "--count", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };

    let bundle = base.join("bundle.json");
    std::fs::write(
        &bundle,
        r#"{"operations":[
          {"op":"milestone.add","ref":"m","name":"Imported"},
          {"op":"feature.add","ref":"a","title":"Add cart page","milestone":"m",
           "source":{"system":"file","ref":"TODO.md:1"},"original":"- [ ] Add cart page"},
          {"op":"todo.add","ref":"t","feature":"a","description":"imported"},
          {"op":"task.add","feature":"a","todo":"t","text":"cart UI"}
        ]}"#,
    )
    .unwrap();
    let file = bundle.to_str().unwrap();

    // Preview writes nothing: no new commit, no feature.
    let before = commits();
    let preview = run(&["batch", "--dry-run", "--file", file]);
    assert!(preview.contains("FEAT-001 Add cart page"), "{preview}");
    assert!(preview.contains("nothing written"), "{preview}");
    assert_eq!(commits(), before, "dry run must not commit");
    assert!(!run(&["feature", "list"]).contains("FEAT-001"));

    // Apply: one commit; provenance carries the project commit.
    run(&["batch", "--file", file]);
    assert_eq!(
        commits().parse::<u32>().unwrap(),
        before.parse::<u32>().unwrap() + 1
    );
    let shown = run(&["export", "FEAT-001", "--format", "json"]);
    let f: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(f["source"]["revision"], head[..12]);
    assert!(
        f["specification"]
            .as_str()
            .unwrap()
            .contains("## Imported from")
    );

    // Re-import: everything skipped, nothing duplicated.
    let again = run(&["batch", "--file", file]);
    assert!(again.contains("skipped 4"), "{again}");

    // The source file goes away (and its history with it): the item is still intact and flagged.
    std::fs::remove_file(repo.join("TODO.md")).unwrap();
    let listed = run(&["sources"]);
    assert!(listed.contains("MISSING"), "{listed}");
    run(&["sources", "--write"]);
    let rows: serde_json::Value = serde_json::from_str(&run(&["--json", "sources"])).unwrap();
    assert_eq!(rows[0]["present"], false);
    assert!(rows[0]["missing_since"].is_string());

    let _ = std::fs::remove_dir_all(&base);
}

/// The GitHub mirror end to end through the real binary, against a fake `gh` that records the
/// REST calls it receives (never the network).
#[cfg(unix)]
#[test]
fn cli_github_mirror_with_a_fake_gh() {
    use std::os::unix::fs::PermissionsExt;

    let base = std::env::temp_dir().join(format!("kanbanr-mirror-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    let fake = base.join("fake-gh");
    for d in [&home, &work, &fake] {
        std::fs::create_dir_all(d).unwrap();
    }
    let gh = base.join("gh");
    std::fs::write(
        &gh,
        r#"#!/usr/bin/env bash
dir="$FAKE_GH_DIR"
echo "$*" >> "$dir/calls.log"
if [ "$1 $2" = "auth status" ]; then exit 0; fi
if [ "$1" != "api" ]; then echo "unexpected gh args: $*" >&2; exit 1; fi
method="$3"; path="$4"; input=""
if [ "$5" = "--input" ]; then input="$(cat)"; fi
case "$method $path" in
  "GET repos/acme/shop") echo '{"private": true}' ;;
  "GET repos/acme/open") echo '{"private": false}' ;;
  "POST repos/acme/shop/issues")
    n=$(( $(ls "$dir" | grep -c '^issue-') + 1 ))
    printf '%s' "$input" > "$dir/issue-$n.json"
    echo "{\"number\": $n, \"html_url\": \"https://github.com/acme/shop/issues/$n\"}" ;;
  "PATCH repos/acme/shop/issues/"*)
    k=$(( $(ls "$dir" | grep -c '^patch-') + 1 ))
    printf '%s' "$input" > "$dir/patch-$k.json"
    echo '{}' ;;
  *) echo "unexpected: $method $path" >&2; exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let run_env = |args: &[&str], extra: &[(&str, &str)]| -> (bool, String, String) {
        let mut cmd = Command::new(cli());
        cmd.args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("KANBANR_GH", &gh)
            .env("FAKE_GH_DIR", &fake)
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("KANBANR_MIRROR")
            .stdin(std::process::Stdio::null());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let o = cmd.output().expect("run kanbanr CLI");
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        )
    };
    let run = |args: &[&str]| -> (String, String) {
        let (ok, out, err) = run_env(args, &[]);
        assert!(ok, "cmd {args:?} failed: {err}");
        (out, err)
    };
    let calls = || {
        std::fs::read_to_string(fake.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let read = |name: &str| std::fs::read_to_string(fake.join(name)).unwrap_or_default();

    run(&["init", "shop", "--author", "Ada", "--email", "a@x"]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);

    // A public repo is refused without --allow-public.
    let (ok, _, err) = run_env(&["mirror", "enable", "--repo", "acme/open"], &[]);
    assert!(!ok && err.contains("is public"), "{err}");
    run(&["mirror", "enable", "--repo", "acme/shop"]);

    // A new feature is mirrored right after the write.
    let (_, err) = run(&[
        "feature",
        "add",
        "--title",
        "Cart page",
        "--milestone",
        "MS-001",
    ]);
    assert!(
        err.contains("mirrored FEAT-001 to acme/shop#1 (created)"),
        "{err}"
    );
    let created = read("issue-1.json");
    assert!(created.contains("\"title\":\"Cart page\""), "{created}");
    assert!(created.contains("shop:FEAT-001"), "{created}");
    let (exported, _) = run(&["export", "FEAT-001", "--format", "json"]);
    let f: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(f["issue"]["number"], 1);
    assert!(f["issue"]["synced_hash"].is_string());
    let (status, _) = run(&["mirror", "status"]);
    assert!(status.contains("0 pending"), "{status}");

    // A change that doesn't affect the issue makes no GitHub calls.
    let before = calls();
    run(&["feature", "edit", "FEAT-001", "--priority", "high"]);
    assert_eq!(calls(), before, "no calls for an unchanged issue");

    // Completing the feature closes the issue.
    run(&["move", "FEAT-001", "Scheduled"]);
    run(&["move", "FEAT-001", "Completed"]);
    let closed = read(&format!(
        "patch-{}.json",
        std::fs::read_dir(&fake)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("patch-"))
            .count()
    ));
    assert!(closed.contains("\"state\":\"closed\""), "{closed}");
    assert!(
        closed.contains("\"state_reason\":\"completed\""),
        "{closed}"
    );

    // KANBANR_MIRROR=off defers; `mirror sync` catches up.
    let before = calls();
    let (ok, _, err) = run_env(
        &["feature", "edit", "FEAT-001", "--title", "Cart v2"],
        &[("KANBANR_MIRROR", "off")],
    );
    assert!(ok, "{err}");
    assert_eq!(calls(), before);
    let (synced, _) = run(&["mirror", "sync"]);
    assert!(synced.contains("updated 1"), "{synced}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_init_registers_claude_code_hooks_once_and_respects_no_hooks() {
    let base = std::env::temp_dir().join(format!("kanbanr-hooks-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let scripts = home.join(".claude/skills/kanbanr/hooks");
    std::fs::create_dir_all(&scripts).unwrap();
    for name in ["session-start", "stop-check", "session-summary"] {
        std::fs::write(scripts.join(format!("{name}.sh")), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(scripts.join(format!("{name}.ps1")), "exit 0\n").unwrap();
    }
    // The machine's settings exist and must stay untouched: hooks belong to the project that has
    // a board, so a checkout nobody tracks with kanbanr carries none of them (FEAT-065).
    let machine = home.join(".claude/settings.json");
    std::fs::write(&machine, r#"{"theme": "dark"}"#).unwrap();

    let run = |dir: &std::path::Path, args: &[&str]| -> String {
        std::fs::create_dir_all(dir).unwrap();
        let o = Command::new(cli())
            .args(args)
            .current_dir(dir)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    let out = run(
        &base.join("code/app"),
        &["init", "app", "--author", "A", "--email", "a@x"],
    );
    assert!(out.contains("Claude Code hooks added"), "{out}");
    // Written to THIS project, pointing at the machine's scripts.
    let project_settings = base.join("code/app/.claude/settings.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project_settings).unwrap()).unwrap();
    assert!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .starts_with(home.to_str().unwrap()),
        "the project's settings point at the machine's scripts, not copies"
    );
    assert!(
        v["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("stop-check")
    );
    // The machine's settings are untouched.
    assert_eq!(
        std::fs::read_to_string(&machine).unwrap(),
        r#"{"theme": "dark"}"#
    );

    // A second project gets its own registration rather than inheriting the first one's.
    let out = run(
        &base.join("code/web"),
        &["init", "web", "--author", "A", "--email", "a@x"],
    );
    assert!(out.contains("Claude Code hooks added"), "{out}");
    assert!(base.join("code/web/.claude/settings.json").exists());
    assert!(run(&base.join("code/web"), &["hooks", "status"]).contains("SessionStart: ✓"));
    // ...and it is idempotent within that project.
    assert!(
        run(&base.join("code/web"), &["hooks", "install"]).contains("already installed"),
        "installing twice in one project changes nothing"
    );

    // --global is the opt-in for someone who wants them everywhere.
    run(&base.join("code/web"), &["hooks", "install", "--global"]);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&machine).unwrap()).unwrap();
    assert_eq!(v["theme"], "dark", "other keys survive");
    assert!(
        v["hooks"]["SessionStart"].is_array(),
        "now registered machine-wide too"
    );

    // --no-hooks writes no project settings at all.
    let out = run(
        &base.join("code/api"),
        &[
            "init",
            "api",
            "--author",
            "A",
            "--email",
            "a@x",
            "--no-hooks",
        ],
    );
    assert!(!out.contains("Claude Code hooks"), "{out}");
    assert!(!base.join("code/api/.claude/settings.json").exists());

    let _ = std::fs::remove_dir_all(&base);
}

/// The project charter end to end (FEAT-046): an absent charter is a warning, a written one is
/// round-tripped with goal ids assigned, and clearing it removes the file.
#[test]
fn cli_charter_round_trips_and_doctor_reports_its_absence() {
    let base = std::env::temp_dir().join(format!("kanbanr-charter-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let run = |args: &[&str], stdin: Option<&str>| -> String {
        let mut cmd = Command::new(cli());
        cmd.args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT");
        let o = match stdin {
            None => cmd.stdin(std::process::Stdio::null()).output(),
            Some(text) => {
                cmd.stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = cmd.spawn().expect("spawn kanbanr");
                use std::io::Write;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(text.as_bytes())
                    .unwrap();
                child.wait_with_output()
            }
        }
        .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(
        &[
            "init",
            "shop",
            "--author",
            "A",
            "--email",
            "a@x",
            "--no-hooks",
        ],
        None,
    );

    // No charter: doctor says so, in the words that tell you how to fix it.
    let before = run(&["doctor"], None);
    assert!(before.contains("no charter purpose"), "{before}");

    // Written from stdin as YAML; the goal without an id gets one.
    let yaml = "purpose: Carts vanish on mobile.\ngoals:\n  - statement: A cart survives 7 days\n    measure: recovery above 90%\nnon_goals:\n  - Payment rewrite\n";
    let set = run(&["charter", "set"], Some(yaml));
    assert!(set.contains("1 goal(s) G-1"), "{set}");

    let shown = run(&["charter", "show"], None);
    assert!(shown.contains("Carts vanish on mobile."), "{shown}");
    assert!(
        shown.contains("- **G-1** A cart survives 7 days"),
        "{shown}"
    );
    assert!(shown.contains("_Measure:_ recovery above 90%"), "{shown}");
    assert!(shown.contains("## Non-goals"), "{shown}");

    let json: serde_json::Value =
        serde_json::from_str(&run(&["--json", "charter", "show"], None)).unwrap();
    assert_eq!(json["goals"][0]["id"], "G-1");
    assert!(json["adopted_at"].as_str().is_some_and(|s| !s.is_empty()));

    // With a purpose and a goal, the "no charter" warning is gone. What remains is the honest
    // observation that nothing is linked to the goal yet (FEAT-049).
    let after = run(&["doctor"], None);
    assert!(!after.contains("no charter purpose"), "{after}");
    assert!(after.contains("G-1 has no work linked to it"), "{after}");

    // An empty charter clears the file.
    let cleared = run(&["charter", "set"], Some("{}\n"));
    assert!(cleared.contains("charter cleared"), "{cleared}");
    assert!(
        !base
            .join("code/shop.kanbanr/projects/shop/charter.yaml")
            .exists()
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// `kanbanr feature define` end to end (FEAT-047): a per-kind template, a definition written from
/// a file with ids assigned and gaps reported, and clearing it again.
#[test]
fn cli_feature_define_templates_writes_and_clears() {
    let base = std::env::temp_dir().join(format!("kanbanr-define-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart recovery",
        "--milestone",
        "MS-001",
    ]);

    // The template is shaped to the kind: a chore asserts an invariant, not new behaviour.
    let chore = run(&[
        "feature",
        "define",
        "FEAT-001",
        "--template",
        "--kind",
        "chore",
    ]);
    assert!(chore.contains("THE SYSTEM SHALL continue to"), "{chore}");
    let defect = run(&[
        "feature",
        "define",
        "FEAT-001",
        "--template",
        "--kind",
        "defect",
    ]);
    assert!(defect.contains("violates:"), "{defect}");
    assert!(defect.contains("state: red"), "{defect}");

    // A definition with a deliberate gap: `how` is left blank rather than invented.
    let def = base.join("def.yaml");
    std::fs::write(
        &def,
        "statement: Keep a cart for 7 days so a returning shopper resumes\n\
         goals: [G-1]\n\
         zachman:\n  what: cart persistence\n  how: \"\"\n  where: checkout service\n\
         \x20 when: on every cart mutation\n  who: returning shoppers\n  why: carts vanish overnight\n\
         requirements:\n  - kind: functional\n    text: \"WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.\"\n\
         \x20   tests:\n      - name: cart::retains_for_seven_days\n        kind: unit\n        state: planned\n",
    )
    .unwrap();
    let out = run(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        def.to_str().unwrap(),
    ]);
    assert!(out.contains("1 requirement(s)"), "{out}");
    assert!(
        out.contains("[MISSING: How]"),
        "gaps are reported, not invented: {out}"
    );

    let exported: serde_json::Value =
        serde_json::from_str(&run(&["export", "FEAT-001", "--format", "json"])).unwrap();
    assert_eq!(exported["definition"]["requirements"][0]["id"], "R-1");
    assert_eq!(
        exported["definition"]["requirements"][0]["tests"][0]["state"],
        "planned"
    );
    assert_eq!(exported["definition"]["goals"][0], "G-1");

    let cleared = run(&["feature", "define", "FEAT-001", "--clear"]);
    assert!(cleared.contains("cleared the definition"), "{cleared}");
    let exported: serde_json::Value =
        serde_json::from_str(&run(&["export", "FEAT-001", "--format", "json"])).unwrap();
    assert!(exported.get("definition").is_none(), "absent, not null");

    let _ = std::fs::remove_dir_all(&base);
}

/// Surfacing and search (FEAT-050): the definition reaches `feature show`, and the gap filters
/// answer the troubleshooting questions.
#[test]
fn cli_feature_show_renders_the_definition_and_query_finds_gaps() {
    let base = std::env::temp_dir().join(format!("kanbanr-surface-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart recovery",
        "--milestone",
        "MS-001",
    ]);
    run(&[
        "feature",
        "add",
        "--title",
        "Undefined work",
        "--milestone",
        "MS-001",
    ]);

    let def = base.join("def.yaml");
    std::fs::write(
        &def,
        "statement: Keep a cart for 7 days\ngoals: [G-1]\n\
         zachman: {what: persistence, how: server store, where: checkout, when: on mutation, who: shoppers, why: carts vanish}\n\
         requirements:\n  - kind: functional\n    text: \"WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.\"\n\
         \x20   tests: [{name: cart::retains, kind: unit, state: green}]\n",
    )
    .unwrap();
    run(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        def.to_str().unwrap(),
    ]);

    // `feature show` IS what Claude reads, so the definition must appear there.
    let shown = run(&["feature", "show", "FEAT-001"]);
    assert!(shown.contains("## Definition"), "{shown}");
    assert!(shown.contains("**Serves:** G-1"), "{shown}");
    assert!(shown.contains("| Where | checkout |"), "{shown}");
    assert!(
        shown.contains("### R-1 · functional · _event_"),
        "derives the EARS pattern: {shown}"
    );
    assert!(shown.contains("- [x] `cart::retains` (unit)"), "{shown}");
    assert!(shown.contains("**Not approved**"), "{shown}");

    // An item with no definition renders exactly as before — no empty section.
    let bare = run(&["feature", "show", "FEAT-002"]);
    assert!(!bare.contains("## Definition"), "{bare}");

    // Search: by goal, and by the kind of gap being chased.
    let by_goal = run(&["query", "--goal", "G-1", "--json"]);
    assert!(
        by_goal.contains("FEAT-001") && !by_goal.contains("FEAT-002"),
        "{by_goal}"
    );
    let why_gaps = run(&["query", "--gap", "why", "--json"]);
    assert!(
        why_gaps.contains("FEAT-002") && !why_gaps.contains("FEAT-001"),
        "{why_gaps}"
    );
    let approval_gaps = run(&["query", "--gap", "approval", "--json"]);
    assert!(
        approval_gaps.contains("FEAT-001"),
        "unapproved counts as a gap: {approval_gaps}"
    );
    // An unknown gap name matches nothing rather than everything.
    assert_eq!(run(&["query", "--gap", "nonsense", "--json"]).trim(), "[]");
    // Requirement text is searchable without reading spec files.
    let hit = run(&["query", "--text", "abandoned", "--json"]);
    assert!(hit.contains("definition"), "match field reported: {hit}");

    let _ = std::fs::remove_dir_all(&base);
}

/// Measurement end to end (FEAT-053): a real test run flips the evidence through the capture hook,
/// `report` derives the numbers from history rather than from anyone's claim, and a defect knows
/// whether it escaped. Nothing here is typed in by hand except the test output itself.
#[test]
fn cli_capture_report_and_defect_escape() {
    let base = std::env::temp_dir().join(format!("kanbanr-measure-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let exec = |args: &[&str], stdin: Option<&str>| -> Output {
        let mut cmd = Command::new(cli());
        cmd.args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT");
        match stdin {
            None => {
                cmd.stdin(std::process::Stdio::null());
                cmd.output().expect("run kanbanr CLI")
            }
            Some(text) => {
                use std::io::Write;
                cmd.stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = cmd.spawn().expect("spawn kanbanr CLI");
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(text.as_bytes())
                    .unwrap();
                child.wait_with_output().expect("run kanbanr CLI")
            }
        }
    };
    let run = |args: &[&str]| -> String {
        let o = exec(args, None);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart recovery",
        "--milestone",
        "MS-001",
    ]);
    let def = base.join("def.yaml");
    std::fs::write(
        &def,
        "statement: Keep a cart for 7 days so a returning shopper resumes\n\
         requirements:\n  - kind: functional\n    text: \"WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.\"\n\
         \x20   tests:\n      - name: cart::retains_for_seven_days\n        kind: unit\n        state: planned\n",
    )
    .unwrap();
    run(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        def.to_str().unwrap(),
    ]);

    // Nothing is proven yet: one requirement, no green.
    let before = run(&["report"]);
    assert!(
        before.contains("requirements proven by a green test: 0/1"),
        "{before}"
    );

    // A real run goes past: the hook reads the output, not a claim about it.
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "tool_response": {"stdout": "test cart::retains_for_seven_days ... ok\n"},
    })
    .to_string();
    let captured = exec(&["capture"], Some(&payload));
    assert!(
        captured.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&captured.stderr)
    );
    assert!(
        String::from_utf8_lossy(&captured.stderr).contains("recorded 1 test result"),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let exported: serde_json::Value =
        serde_json::from_str(&run(&["export", "FEAT-001", "--format", "json"])).unwrap();
    assert_eq!(
        exported["definition"]["requirements"][0]["tests"][0]["state"], "green",
        "the run itself moved the evidence"
    );
    // A second identical run has nothing new to say.
    let again = exec(&["capture"], Some(&payload));
    assert!(
        !String::from_utf8_lossy(&again.stderr).contains("recorded"),
        "already recorded at this revision: {}",
        String::from_utf8_lossy(&again.stderr)
    );

    // Call the work done, then find a defect in it: that is an escape, and nobody had to say so.
    run(&["move", "FEAT-001", "Scheduled"]);
    run(&["move", "FEAT-001", "Completed"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart empties on sign-out",
        "--milestone",
        "MS-001",
        "--kind",
        "defect",
    ]);
    let recorded = run(&[
        "defect",
        "FEAT-002",
        "--severity",
        "high",
        "--introduced-by",
        "FEAT-001",
        "--found-in",
        "production",
    ]);
    assert!(recorded.contains("escaped"), "{recorded}");

    let after = run(&["report"]);
    assert!(after.contains("completed: 1"), "{after}");
    assert!(after.contains("defects: 1   escaped: 1"), "{after}");
    assert!(
        after.contains("requirements proven by a green test: 1/1 (100%)"),
        "{after}"
    );
    // Cycle time comes from the transition history, so it exists at all only because moves are
    // recorded; the same board with no history reports no number rather than a guess.
    assert!(
        after.contains("cycle time: p50 "),
        "history drives cycle time: {after}"
    );

    // While the test exists in the project, the recorded green stands.
    std::fs::create_dir_all(work.join("src")).unwrap();
    let source = work.join("src").join("cart.rs");
    std::fs::write(&source, "fn cart_retains_for_seven_days() {}\n").unwrap();
    let tracked = run(&["tests"]);
    assert!(tracked.contains("found"), "{tracked}");
    assert!(!tracked.contains("NOT IN THE REPO"), "{tracked}");

    // Delete the test and the evidence goes with it: a green nobody can re-run is not evidence.
    std::fs::remove_file(&source).unwrap();
    let gone = run(&["tests"]);
    assert!(gone.contains("NOT IN THE REPO"), "{gone}");
    assert!(gone.contains("`kanbanr tests --write`"), "{gone}");
    let rewritten = run(&["tests", "--write"]);
    assert!(
        rewritten.contains("returned 1 result(s) to planned"),
        "{rewritten}"
    );
    let exported: serde_json::Value =
        serde_json::from_str(&run(&["export", "FEAT-001", "--format", "json"])).unwrap();
    assert_eq!(
        exported["definition"]["requirements"][0]["tests"][0]["state"],
        "planned"
    );
    let unproven = run(&["report"]);
    assert!(
        unproven.contains("requirements proven by a green test: 0/1"),
        "{unproven}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// The git guardrails end to end (FEAT-056), in a scratch repo: a commit with no reference is
/// refused, one naming something that does not exist is refused, the recorded escape passes, the
/// default branch is refused, and `kanbanr start` puts you where the rules expect you to be.
#[test]
fn cli_git_guardrails_in_a_scratch_repo() {
    let base = std::env::temp_dir().join(format!("kanbanr-scm-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let git = |args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("run git")
    };
    let exec = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI")
    };
    let run = |args: &[&str]| -> String {
        let o = exec(args);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    assert!(git(&["init", "--initial-branch=main"]).status.success());
    // An established repository: a fresh one's first commit is a separate case (FEAT-105).
    assert!(
        git(&[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "[no-ref] initial commit"
        ])
        .status
        .success()
    );
    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart recovery",
        "--milestone",
        "MS-001",
    ]);

    // The message check is what the commit-msg hook runs.
    let msg = base.join("msg.txt");
    let check = |text: &str| -> Output {
        std::fs::write(&msg, text).unwrap();
        exec(&["git", "check-msg", msg.to_str().unwrap()])
    };
    let bare = check("feat: do a thing\n");
    assert!(
        !bare.status.success(),
        "a commit with no reference is refused"
    );
    assert!(
        String::from_utf8_lossy(&bare.stderr).contains("which item it serves"),
        "{}",
        String::from_utf8_lossy(&bare.stderr)
    );
    assert!(
        check("feat: do a thing\n\nRefs: kanbanr:FEAT-001\n")
            .status
            .success()
    );
    let unknown = check("feat: do a thing\n\nRefs: kanbanr:FEAT-999\n");
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("not an item on this board"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    // A requirement that the item does not have is just as broken as a missing item.
    let bad_req = check("feat: x\n\nRefs: kanbanr:FEAT-001/R-9\n");
    assert!(!bad_req.status.success());
    assert!(
        String::from_utf8_lossy(&bad_req.stderr).contains("not a requirement"),
        "{}",
        String::from_utf8_lossy(&bad_req.stderr)
    );
    // Merges, and escapes that say why, pass. An escape with no reason does not.
    assert!(
        check("Merge branch 'feat/FEAT-001-cart'\n")
            .status
            .success()
    );
    assert!(
        check("chore: rotate a key\n\n[no-ref] incident response, item filed after\n")
            .status
            .success()
    );
    assert!(!check("chore: rotate a key\n\n[no-ref]\n").status.success());
    // Explaining the escape is not taking it: this commit has a reference and keeps it.
    assert!(
        check("docs: describe `[no-ref] <why>`\n\nRefs: kanbanr:FEAT-001\n")
            .status
            .success(),
        "a referenced commit that mentions the escape is still a referenced commit"
    );
    assert!(
        !check("docs: describe `[no-ref] <why>` in the guide\n")
            .status
            .success(),
        "...and mentioning it does not excuse having no reference"
    );

    // The branch check refuses the default branch, and `start` moves you off it.
    let on_main = exec(&["git", "check-branch"]);
    assert!(!on_main.status.success());
    assert!(
        String::from_utf8_lossy(&on_main.stderr).contains("default branch"),
        "{}",
        String::from_utf8_lossy(&on_main.stderr)
    );
    let started = run(&["start", "FEAT-001"]);
    assert!(started.contains("feat/FEAT-001-cart-recovery"), "{started}");
    assert!(
        exec(&["git", "check-branch"]).status.success(),
        "a branch that names an item is fine"
    );
    let status = run(&["git", "status"]);
    assert!(status.contains("item:    FEAT-001"), "{status}");

    // A branch that belongs to nothing is refused; a spike is allowed to exist.
    assert!(
        git(&["checkout", "-q", "-b", "random-work"])
            .status
            .success()
    );
    assert!(!exec(&["git", "check-branch"]).status.success());
    assert!(
        git(&["checkout", "-q", "-b", "spike/try-it"])
            .status
            .success()
    );
    assert!(exec(&["git", "check-branch"]).status.success());
    // ...but a spike cannot be finished: its output is a definition change, not merged code.
    let spike = exec(&["finish"]);
    assert!(!spike.status.success());
    assert!(
        String::from_utf8_lossy(&spike.stderr).contains("definition"),
        "{}",
        String::from_utf8_lossy(&spike.stderr)
    );

    // Installing hooks is idempotent and keeps a hook that was already there.
    let hooks = work.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 0\n").unwrap();
    let refused = exec(&["git", "install-hooks"]);
    assert!(
        !refused.status.success(),
        "someone else's hook is not clobbered"
    );
    assert!(run(&["git", "install-hooks", "--force"]).contains("commit-msg"));
    assert!(hooks.join("pre-commit.pre-kanbanr").exists());
    assert!(run(&["git", "install-hooks"]).contains("installed"));
    assert!(run(&["git", "uninstall-hooks"]).contains("removed 2"));

    let _ = std::fs::remove_dir_all(&base);
}

/// A wave retrospective end to end (FEAT-054): the facts come out of the board, growth is
/// attributed only to what items record, and the written document keeps the narrative separate.
#[test]
fn cli_retro_reports_facts_and_writes_a_document() {
    let base = std::env::temp_dir().join(format!("kanbanr-retro-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "Wave", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "The work",
        "--milestone",
        "MS-001",
    ]);
    run(&["move", "FEAT-001", "Scheduled"]);
    // Three items join after the wave began, each for a different recorded reason.
    run(&[
        "feature",
        "add",
        "--title",
        "It breaks",
        "--milestone",
        "MS-001",
        "--kind",
        "defect",
    ]);
    run(&[
        "defect",
        "FEAT-002",
        "--introduced-by",
        "FEAT-001",
        "--found-in",
        "review",
    ]);
    run(&[
        "feature",
        "add",
        "--title",
        "Second half",
        "--milestone",
        "MS-001",
    ]);
    run(&["split-from", "FEAT-003", "FEAT-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Something else",
        "--milestone",
        "MS-001",
    ]);

    let retro = run(&["retro", "MS-001"]);
    assert!(
        retro.contains("items: 4 (0 finished, 4 still open)"),
        "{retro}"
    );
    assert!(
        retro.contains("1 to begin with, 3 added (1 defect(s), 1 split, 1 unaccounted for)"),
        "{retro}"
    );
    assert!(
        retro.contains("caused by work in this same wave: FEAT-002 ← FEAT-001"),
        "{retro}"
    );
    assert!(
        retro.contains("cycle time: nothing finished with a recorded history"),
        "no invented number when nothing has finished: {retro}"
    );

    // Nothing is due while the wave is open; finishing every item makes it due.
    assert!(run(&["retro", "--due"]).contains("no retro is due"));
    for code in ["FEAT-001", "FEAT-002", "FEAT-003", "FEAT-004"] {
        if code != "FEAT-001" {
            run(&["move", code, "Scheduled"]);
        }
        run(&["move", code, "Completed"]);
    }
    let due = run(&["retro", "--due"]);
    assert!(due.contains("MS-001 is finished and has no retro"), "{due}");

    // What the wave taught belongs in the wave's own write-up.
    run(&[
        "lesson",
        "add",
        "Record where an item came from while the wave runs, not afterwards",
        "--kind",
        "practice",
        "--from",
        "FEAT-003",
        "--evidence",
        "the unattributed item took longer to explain than to record",
    ]);
    let with_lesson = run(&["retro", "MS-001"]);
    assert!(
        with_lesson.contains("## What this wave taught"),
        "{with_lesson}"
    );
    assert!(
        with_lesson.contains("Record where an item came from"),
        "{with_lesson}"
    );

    let written = run(&["retro", "MS-001", "--write"]);
    assert!(written.contains("written to retros/MS-001-"), "{written}");
    let path = written
        .lines()
        .find_map(|l| l.strip_prefix("written to "))
        .unwrap()
        .trim()
        .to_string();
    let doc = run(&["doc", "show", &path]);
    assert!(doc.contains("## What the board recorded"), "{doc}");
    assert!(
        doc.contains("## What we make of it"),
        "the narrative has its own section, so a reader can tell them apart: {doc}"
    );
    assert!(doc.contains("cycle time: p50 "), "{doc}");
    assert!(doc.contains("## What this wave taught"), "{doc}");
    // The rows stay canonical; the document says so rather than pretending to be the record.
    assert!(doc.contains("Live state is `kanbanr lessons`"), "{doc}");
    // Written up, so no longer due.
    assert!(run(&["retro", "--due"]).contains("no retro is due"));

    let _ = std::fs::remove_dir_all(&base);
}

/// Lessons end to end (FEAT-055): recorded with where they came from, matched to the item about to
/// be worked on, affirmed when they hold, and retired — not deleted — when they turn out wrong.
#[test]
fn cli_lessons_are_captured_matched_and_can_be_contradicted() {
    let base = std::env::temp_dir().join(format!("kanbanr-lessons-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "Mirror sync",
        "--milestone",
        "MS-001",
        "--labels",
        "mirror",
    ]);
    run(&[
        "feature",
        "add",
        "--title",
        "Docs pass",
        "--milestone",
        "MS-001",
        "--labels",
        "docs",
    ]);

    assert!(run(&["lessons"]).contains("nothing learned here yet"));
    let added = run(&[
        "lesson",
        "add",
        "Auto-sync re-pushes every linked issue on any write",
        "--kind",
        "pitfall",
        "--from",
        "FEAT-001",
        "--evidence",
        "it re-notified 40 issues during an unrelated task update",
        "--tags",
        "mirror",
    ]);
    assert!(added.contains("L-1"), "{added}");
    assert!(
        added.contains("60%"),
        "a new lesson is believed, not proven: {added}"
    );

    // Said again in different words: the same lesson, affirmed rather than duplicated.
    let again = run(&[
        "lesson",
        "add",
        "auto-sync RE-PUSHES every linked issue on any write!",
    ]);
    assert!(again.contains("L-1"), "{again}");
    assert!(again.contains("75%"), "repetition is evidence: {again}");
    assert_eq!(
        run(&["lessons"]).matches("L-").count(),
        1,
        "one lesson, not two"
    );

    // Matched to the work about to start, and not offered to unrelated work.
    let matched = run(&["lessons", "--for", "FEAT-001"]);
    assert!(matched.contains("Auto-sync re-pushes"), "{matched}");
    assert!(
        run(&["lessons", "--for", "FEAT-002"]).contains("nothing learned here yet"),
        "a docs item is not told about mirrors"
    );

    // Contradiction costs more than affirmation: one wipes out the gain and then some.
    let out = run(&[
        "lesson",
        "contradict",
        "L-1",
        "--note",
        "the re-push was a config error, not the sync",
    ]);
    assert!(
        out.contains("30%"),
        "75% affirmed, then contradicted: {out}"
    );
    assert!(
        !out.contains("retired"),
        "once is a doubt, not a refutation: {out}"
    );
    // Twice puts it below the threshold, and it stops being offered.
    let out = run(&[
        "lesson",
        "contradict",
        "L-1",
        "--note",
        "again, not the sync",
    ]);
    assert!(out.contains("retired"), "{out}");
    assert!(run(&["lessons"]).contains("nothing learned here yet"));
    // Kept as a record, not deleted: being wrong later is part of the history.
    let all = run(&["lessons", "--all"]);
    assert!(all.contains("L-1"), "{all}");

    let _ = std::fs::remove_dir_all(&base);
}

/// Traceability end to end (FEAT-057): down from a goal to its evidence, up from a line of code to
/// the reason it exists, decisions joining the graph as documents, and the trailer levels that
/// point at them being checked like any other reference.
#[test]
fn cli_trace_and_why_follow_the_chain_in_both_directions() {
    let base = std::env::temp_dir().join(format!("kanbanr-trace-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let git = |args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("run git")
    };
    let exec = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI")
    };
    let run = |args: &[&str]| -> String {
        let o = exec(args);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    assert!(git(&["init", "--initial-branch=main"]).status.success());
    // An established repository: a fresh one's first commit is a separate case (FEAT-105).
    assert!(
        git(&[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "[no-ref] initial commit"
        ])
        .status
        .success()
    );
    // `kanbanr commit` runs the real git, which needs an identity in this scratch repo.
    assert!(git(&["config", "user.name", "T"]).status.success());
    assert!(git(&["config", "user.email", "t@x"]).status.success());
    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    let charter = base.join("charter.yaml");
    std::fs::write(
        &charter,
        "purpose: A cart that survives a closed tab\ngoals:\n  - id: G-1\n    statement: A returning shopper resumes where they left off\n",
    )
    .unwrap();
    run(&["charter", "set", "--file", charter.to_str().unwrap()]);
    run(&[
        "feature",
        "add",
        "--title",
        "Cart recovery",
        "--milestone",
        "MS-001",
    ]);
    let def = base.join("def.yaml");
    std::fs::write(
        &def,
        "statement: Keep a cart for 7 days\ngoals: [G-1]\n\
         requirements:\n  - kind: functional\n    text: \"WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.\"\n\
         \x20   tests:\n      - name: cart::retains\n        kind: unit\n        state: planned\n",
    )
    .unwrap();
    run(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        def.to_str().unwrap(),
    ]);

    // Downward: the chain, and the gap in it.
    let down = run(&["trace", "G-1"]);
    assert!(down.contains("FEAT-001"), "{down}");
    assert!(down.contains("cart::retains"), "{down}");
    assert!(
        down.contains("R-1 has tests but none is green"),
        "the gap is the output: {down}"
    );

    // A decision joins the graph as a document, and shows up against the item it affects.
    let adr = run(&[
        "adr",
        "new",
        "Store carts server-side",
        "--affects",
        "FEAT-001",
        "--status",
        "accepted",
        "--zachman",
        "How",
    ]);
    assert!(adr.contains("ADR-0001"), "{adr}");
    let listed = run(&["adr", "list", "--for", "FEAT-001"]);
    assert!(listed.contains("Store carts server-side"), "{listed}");
    assert!(
        listed.contains("unwritten: Context"),
        "a scaffold is not a written decision: {listed}"
    );
    assert!(run(&["trace", "FEAT-001"]).contains("ADR-0001"));

    // Upward: a line of code, through the commit that wrote it, to the reason it exists.
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(
        work.join("src").join("cart.rs"),
        "// Carts outlive the tab that made them (FEAT-001 R-1).\nfn retain() {}\n",
    )
    .unwrap();
    // The gate is real: starting requires the definition to have been agreed.
    let refused = exec(&["start", "FEAT-001"]);
    assert!(!refused.status.success(), "unapproved work does not start");
    run(&["approve", "FEAT-001"]);
    run(&["start", "FEAT-001"]);
    assert!(git(&["add", "-A"]).status.success());
    run(&[
        "commit",
        "-m",
        "feat(cart): keep the cart for a week",
        "--ref",
        "R-1",
    ]);

    let why = run(&["why", "src/cart.rs:2"]);
    assert!(why.contains("FEAT-001/R-1"), "{why}");
    assert!(
        why.contains("THE SYSTEM SHALL retain it for 7 days"),
        "{why}"
    );
    assert!(why.contains("G-1"), "{why}");
    assert!(why.contains("A cart that survives a closed tab"), "{why}");
    // The annotation is preferred over blame, and is what survives a refactor.
    assert!(why.contains("from the annotation on the code"), "{why}");

    // A line nobody claimed says so, rather than borrowing the file's other references.
    std::fs::write(work.join("src").join("stray.rs"), "fn nobody_asked() {}\n").unwrap();
    let orphan = run(&["why", "src/stray.rs:1"]);
    assert!(orphan.contains("carries no reference"), "{orphan}");

    // The new trailer levels are checked like any other reference.
    let msg = base.join("msg.txt");
    let check = |text: &str| -> Output {
        std::fs::write(&msg, text).unwrap();
        exec(&["git", "check-msg", msg.to_str().unwrap()])
    };
    assert!(
        check("docs: write it up\n\nRefs: kanbanr:FEAT-001\nADR: ADR-0001\n")
            .status
            .success()
    );
    let bad_adr = check("docs: x\n\nRefs: kanbanr:FEAT-001\nADR: ADR-0404\n");
    assert!(!bad_adr.status.success());
    assert!(
        String::from_utf8_lossy(&bad_adr.stderr).contains("ADR-0404 is not a decision"),
        "{}",
        String::from_utf8_lossy(&bad_adr.stderr)
    );
    let bad_doc = check("docs: x\n\nRefs: kanbanr:FEAT-001\nDocs: design/nope.md\n");
    assert!(!bad_doc.status.success());
    assert!(
        String::from_utf8_lossy(&bad_doc.stderr).contains("design/nope.md is not a document"),
        "{}",
        String::from_utf8_lossy(&bad_doc.stderr)
    );

    // Superseding names what was resting on the old decision.
    run(&[
        "adr",
        "new",
        "Keep carts in the browser",
        "--affects",
        "FEAT-001",
        "--status",
        "accepted",
    ]);
    let out = run(&["adr", "supersede", "ADR-0002", "--replaces", "ADR-0001"]);
    assert!(out.contains("now stand on an overturned decision"), "{out}");
    assert!(out.contains("FEAT-001"), "{out}");
    let history = run(&["adr", "history", "ADR-0002"]);
    assert!(
        history.contains("ADR-0001") && history.contains("ADR-0002"),
        "{history}"
    );
    assert!(
        run(&["trace", "FEAT-001"]).contains("ADR-0001 has been superseded"),
        "work resting on an overturned decision is a gap"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// The board governing the agent, end to end (FEAT-065): the generated CLAUDE.md block refreshes
/// without touching what the author wrote, the hooks register for this project rather than the
/// machine, and the docs guard denies a loose note while letting a deliverable through.
#[test]
fn cli_claude_sync_and_the_docs_guard() {
    let base = std::env::temp_dir().join(format!("kanbanr-claude-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let exec = |args: &[&str], stdin: Option<&str>| -> Output {
        let mut cmd = Command::new(cli());
        cmd.args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT");
        match stdin {
            None => {
                cmd.stdin(std::process::Stdio::null());
                cmd.output().expect("run kanbanr CLI")
            }
            Some(text) => {
                use std::io::Write;
                cmd.stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = cmd.spawn().expect("spawn kanbanr CLI");
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(text.as_bytes())
                    .unwrap();
                child.wait_with_output().expect("run kanbanr CLI")
            }
        }
    };
    let run = |args: &[&str]| -> String {
        let o = exec(args, None);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    // The skill is installed once per machine; a project's settings point at these scripts.
    let scripts = home.join(".claude/skills/kanbanr/hooks");
    std::fs::create_dir_all(&scripts).unwrap();
    for name in ["session-start", "stop-check", "session-summary"] {
        for ext in ["sh", "ps1"] {
            std::fs::write(scripts.join(format!("{name}.{ext}")), "#!/bin/sh\nexit 0\n").unwrap();
        }
    }

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);

    // No charter: nothing to put in front of anyone, and it says so rather than writing a husk.
    let refused = exec(&["claude", "sync"], None);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("no charter"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let charter = base.join("charter.yaml");
    std::fs::write(
        &charter,
        "purpose: A cart that survives a closed tab\n\
         goals:\n  - id: G-1\n    statement: A returning shopper resumes\n\
         non_goals:\n  - A checkout redesign\n\
         constraints:\n  - One developer, one machine\n",
    )
    .unwrap();
    run(&["charter", "set", "--file", charter.to_str().unwrap()]);

    // The author's own instructions come first and survive; the block is appended.
    let claude_md = work.join("CLAUDE.md");
    std::fs::write(&claude_md, "# shop\n\nRun the linter before committing.\n").unwrap();
    let out = run(&["claude", "sync"]);
    assert!(out.contains("CLAUDE.md"), "{out}");
    let written = std::fs::read_to_string(&claude_md).unwrap();
    assert!(written.starts_with("# shop"), "{written}");
    assert!(written.contains("Run the linter before committing."));
    assert!(written.contains("A cart that survives a closed tab"));
    assert!(written.contains("`G-1` A returning shopper resumes"));
    assert!(
        written.contains("A checkout redesign"),
        "non-goals are the point"
    );
    assert!(written.contains("One developer, one machine"));
    // Running it again changes nothing.
    assert!(run(&["claude", "sync"]).contains("already current"));

    // A changed charter refreshes the block in place, leaving one copy and the author's text.
    std::fs::write(
        &charter,
        "purpose: A cart that survives a closed tab\n\
         goals:\n  - id: G-1\n    statement: A returning shopper resumes in one click\n",
    )
    .unwrap();
    run(&["charter", "set", "--file", charter.to_str().unwrap()]);
    run(&["claude", "sync"]);
    let refreshed = std::fs::read_to_string(&claude_md).unwrap();
    assert!(refreshed.contains("resumes in one click"), "{refreshed}");
    assert!(
        !refreshed.contains("A returning shopper resumes\n"),
        "the stale copy is gone"
    );
    assert!(refreshed.contains("Run the linter before committing."));
    assert_eq!(
        refreshed.matches("kanbanr:begin").count(),
        1,
        "exactly one block"
    );

    // Hooks register for THIS project, not the machine.
    let installed = run(&["hooks", "install"]);
    assert!(installed.contains(".claude/settings.json"), "{installed}");
    assert!(
        work.join(".claude").join("settings.json").exists(),
        "the project's settings file is the one that was written"
    );
    assert!(
        !home.join(".claude").join("settings.json").exists(),
        "nothing was written to the machine's settings"
    );

    // The docs guard: a loose note goes to the board, a deliverable does not.
    let guard = |path: &str| -> String {
        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": {"file_path": work.join(path).to_string_lossy()},
        })
        .to_string();
        let o = exec(&["claude", "guard"], Some(&payload));
        assert!(o.status.success(), "the guard must never fail a write");
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    let denied = guard("RETROSPECTIVE-NOTES.md");
    assert!(
        denied.contains("\"permissionDecision\":\"deny\""),
        "{denied}"
    );
    assert!(denied.contains("kanbanr doc add"), "{denied}");
    for allowed in [
        "README.md",
        "CHANGELOG.md",
        "docs/USER_GUIDE.md",
        "src/cart.rs",
    ] {
        assert!(
            guard(allowed).trim().is_empty(),
            "{allowed} is a deliverable and must pass silently"
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}

/// The event log belongs in the commit that caused it (FEAT-059). Eventing used to run entirely
/// after the commit, so a session's last events stayed uncommitted until some later write swept
/// them up — and the board repo was dirty after every session's final operation.
#[test]
fn cli_event_log_is_committed_with_the_change() {
    let base = std::env::temp_dir().join(format!("kanbanr-events-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let run = |args: &[&str]| -> String {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    let board = run(&["where"]).trim().to_string();
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&["feature", "add", "--title", "Cart", "--milestone", "MS-001"]);
    // A status move is the op that produces an event.
    run(&["move", "FEAT-001", "Scheduled"]);

    let git = |args: &[&str]| -> String {
        let o = Command::new("git")
            .args(args)
            .current_dir(&board)
            .output()
            .expect("run git");
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    // The whole point: nothing is left over for the next write to sweep up.
    assert_eq!(
        git(&["status", "--porcelain"]).trim(),
        "",
        "the board repo is clean after the last write"
    );
    // And the event landed in the commit for that very move, not a later one.
    let files = git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("projects/shop/events/"),
        "the event log is part of the commit that caused it: {files}"
    );
    // One file per day (FEAT-066), so read whichever day the write landed on.
    let events: String =
        std::fs::read_dir(std::path::Path::new(&board).join("projects/shop/events"))
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect();
    assert!(events.contains("FEAT-001"), "{events}");

    let _ = std::fs::remove_dir_all(&base);
}

/// The bar applied without a board (FEAT-052): a contributor has no board access, so their
/// definition travels with the pull request and CI checks it on its own. Same rules, no project.
#[test]
fn cli_check_file_holds_a_contribution_to_the_same_bar() {
    let base = std::env::temp_dir().join(format!("kanbanr-checkfile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    // Deliberately NOT a kanbanr project: no board, no marker, no data dir.
    let check = |body: &str| -> Output {
        let file = base.join("definition.yaml");
        std::fs::write(&file, body).unwrap();
        Command::new(cli())
            .args(["check", "--file", file.to_str().unwrap()])
            .current_dir(&base)
            .env("HOME", base.join("home"))
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI")
    };

    let good = "\
statement: Keep a cart for 7 days so a returning shopper resumes
zachman:
  what: cart persistence
  how: server-side, keyed by session
  where: the checkout service
  when: on every cart mutation
  who: returning shoppers
  why: carts vanish overnight and the sale is lost
requirements:
  - kind: functional
    text: \"WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.\"
    tests:
      - name: cart::retains_for_seven_days
        kind: unit
        state: green
";
    let out = check(good);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("meets the bar"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // The failures a reviewer would otherwise have to catch by reading.
    let bad = "\
statement: \"\"
requirements:
  - kind: nfr
    text: It should be fast.
    iso25010: [Speediness]
";
    let out = check(bad);
    assert!(
        !out.status.success(),
        "a non-zero exit is what makes this useful in CI"
    );
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("[MISSING: statement]"), "{shown}");
    assert!(shown.contains("not in EARS form"), "{shown}");
    assert!(shown.contains("has no test"), "{shown}");
    assert!(
        shown.contains("not an ISO/IEC 25010 characteristic"),
        "{shown}"
    );
    assert!(shown.contains("no scenario"), "{shown}");

    // A green test is the requirement: intent alone does not pass.
    let planned = good.replace("state: green", "state: planned");
    let out = check(&planned);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("none is green"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-085: `kanbanr init` overwrote an existing `.kanbanr` marker without a word, repointing a
/// folder from its real board to an empty one. The board survived on disk and the tool simply
/// stopped being able to find it — reported, as ever, as "nothing here". It happened to this
/// repository, and nothing was lost only because FEAT-073 had made the marker a tracked file.
///
/// The property under test is the one that matters: init does not repoint a folder that already
/// names a board. Asserted against the real binary, because the defect was the call site writing
/// unconditionally, not a helper returning the wrong answer.
#[test]
fn init_refuses_to_repoint_a_folder_that_already_has_a_board() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("repoint-{}", std::process::id()));
    let home = base.join("home");
    let work = base.join("work");
    let first = base.join("first-board");
    let second = base.join("second-board");
    for d in [&home, &work] {
        std::fs::create_dir_all(d).unwrap();
    }

    let run = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("KANBANR_SERVER_URL")
            .output()
            .expect("run cli")
    };

    let init = |name: &str, dir: &std::path::Path, extra: &[&str]| -> Output {
        let mut args = vec![
            "init",
            name,
            "--data-dir",
            dir.to_str().unwrap(),
            "--no-hooks",
            "--author",
            "CI",
            "--email",
            "ci@kanbanr.local",
        ];
        args.extend_from_slice(extra);
        run(&args)
    };

    // A folder with a board.
    assert!(init("alpha", &first, &[]).status.success(), "first init");
    let marker = work.join(".kanbanr");
    let original = std::fs::read_to_string(&marker).unwrap();
    assert!(original.contains("alpha"), "marker: {original}");

    // R-1: initialising a DIFFERENT board here is refused, and says what both pointers are.
    let out = init("beta", &second, &[]);
    assert!(
        !out.status.success(),
        "init must refuse to repoint: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("already names a board"), "unhelpful: {err}");
    assert!(
        err.contains("alpha"),
        "must name the current pointer: {err}"
    );
    assert!(err.contains("beta"), "must name the proposed one: {err}");
    assert!(
        err.contains("--force"),
        "a refusal must name the way through: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        original,
        "the marker must be byte-identical after a refusal"
    );

    // R-2: re-running the SAME init is idempotent, not an error.
    assert!(
        init("alpha", &first, &[]).status.success(),
        "re-initialising the same board must still work"
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), original);

    // R-3: --force repoints, and says what it replaced rather than leaving it to be inferred.
    let out = init("beta", &second, &["--force"]);
    assert!(out.status.success(), "--force must repoint");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("replaced") && said.contains("alpha"),
        "--force must report what it replaced: {said}"
    );
    assert!(std::fs::read_to_string(&marker).unwrap().contains("beta"));
}

/// FEAT-105 R-2/R-3: a repository with no commits. `git init` made `master`; the rules must call
/// that the default (not an assumed `main`), let the root commit land there, and have `start`
/// refuse to branch from nothing — then behave exactly as usual once the first commit exists.
#[test]
fn cli_the_first_commit_of_a_new_repo_is_allowed() {
    let base = std::env::temp_dir().join(format!("kanbanr-unborn-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("fresh");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let git = |args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("run git")
    };
    let exec = |args: &[&str]| -> Output {
        Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI")
    };
    let run = |args: &[&str]| -> String {
        let o = exec(args);
        assert!(
            o.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };

    assert!(git(&["init", "--initial-branch=master"]).status.success());
    run(&[
        "init",
        "fresh",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    run(&["milestone", "add", "--name", "M", "--code", "MS-001"]);
    run(&[
        "feature",
        "add",
        "--title",
        "First",
        "--milestone",
        "MS-001",
    ]);

    let status = run(&["git", "status"]);
    assert!(status.contains("default: master"), "{status}");

    // R-3: nothing to branch from yet, so start says what to do instead of making an orphan.
    let early = exec(&["start", "FEAT-001", "--unapproved", "test"]);
    assert!(!early.status.success());
    let why = String::from_utf8_lossy(&early.stderr);
    assert!(
        why.contains("no commits yet") && why.contains("master"),
        "{why}"
    );

    // R-2: the root commit may land on the default branch.
    assert!(exec(&["git", "check-branch"]).status.success());
    assert!(
        git(&[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "[no-ref] initial commit"
        ])
        .status
        .success()
    );

    // With a commit in place the ordinary rules are back: the default branch is refused again,
    // and start branches from it.
    assert!(!exec(&["git", "check-branch"]).status.success());
    let started = run(&["start", "FEAT-001", "--unapproved", "test"]);
    assert!(started.contains("from master"), "{started}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-103: in a folder nothing has set up, looking must not create a board. Every read used to
/// leave `./data/projects` behind — one did so in a project during its read-only setup interview.
/// Hook commands stay silent there, and an existing legacy `./data` board keeps working.
#[test]
fn cli_reads_in_an_untracked_folder_create_nothing() {
    let base = std::env::temp_dir().join(format!("kanbanr-noboard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("plain");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let exec = |args: &[&str], stdin: &str| -> Output {
        use std::io::Write;
        let mut child = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run kanbanr CLI");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let listing = || -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&work)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };

    // R-1: reads refuse, say how to get a board, and leave the folder as they found it.
    for args in [
        &["whoami"][..],
        &["board"],
        &["feature", "list"],
        &["charter", "show"],
        &["doctor"],
    ] {
        let out = exec(args, "");
        assert!(!out.status.success(), "{args:?} should report no board");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("no kanbanr board here") && err.contains("kanbanr init"),
            "{args:?}: {err}"
        );
        assert!(listing().is_empty(), "{args:?} created {:?}", listing());
    }
    // Hooks run in every folder: silent, successful, and still writing nothing.
    for args in [&["capture"][..], &["git", "guard"], &["claude", "guard"]] {
        let out = exec(args, "{}");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(listing().is_empty(), "{args:?} created {:?}", listing());
    }

    // R-2: a legacy ./data board that already exists is still read.
    let legacy = Command::new(cli())
        .args(["--data-dir", "data", "project", "init", "plain"])
        .current_dir(&work)
        .env("HOME", &home)
        .env_remove("KANBANR_DATA_DIR")
        .env_remove("KANBANR_PROJECT")
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    let out = exec(&["board"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-110: output cut short by its reader (`kanbanr … | head`) ends quietly, as git's does, not
/// with a panic and a stack trace after a command that had succeeded. The read end is closed before
/// the CLI starts, so its first write is guaranteed to meet a broken pipe — no timing involved.
#[test]
fn cli_a_closed_pipe_is_not_a_panic() {
    let dir = std::env::temp_dir().join(format!("kanbanr-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = Command::new(cli())
        .args(["where"])
        .current_dir(&dir)
        .env_remove("KANBANR_DATA_DIR")
        .env_remove("KANBANR_PROJECT")
        .stdout(writer)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run kanbanr CLI");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.trim().is_empty(), "nothing on stderr either: {err}");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(
            out.status.code() == Some(141) || out.status.signal() == Some(13),
            "the status a broken pipe gives: {:?}",
            out.status
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scratch repo whose board uses the TOGAF phases, with a branch made at Implementation and the
/// charter adopted — the shape FEAT-115 is about. Returns (base, work, exec).
fn togaf_scratch(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("kanbanr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(base.join("home")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    (base, work)
}

fn run_in(base: &std::path::Path, work: &std::path::Path, args: &[&str]) -> Output {
    Command::new(cli())
        .args(args)
        .current_dir(work)
        .env("HOME", base.join("home"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("KANBANR_DATA_DIR")
        .env_remove("KANBANR_PROJECT")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run kanbanr CLI")
}

fn git_in(base: &std::path::Path, work: &std::path::Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(work)
        .env("HOME", base.join("home"))
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .output()
        .expect("run git")
}

/// Set up: git repo with a first commit, a TOGAF board with its charter adopted, gates that ask
/// only for approval to enter each phase and make the branch at Implementation, and one approved
/// item at Vision.
fn togaf_board(base: &std::path::Path, work: &std::path::Path) {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(base, work, &["init", "--initial-branch=main"]));
    ok(git_in(
        base,
        work,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "[no-ref] initial commit",
        ],
    ));
    ok(run_in(
        base,
        work,
        &[
            "init",
            "shop",
            "--author",
            "A",
            "--email",
            "a@x",
            "--no-hooks",
        ],
    ));
    ok(run_in(base, work, &["config", "workflow", "--togaf"]));
    let charter = base.join("charter.yaml");
    std::fs::write(
        &charter,
        "purpose: p\ngoals:\n  - statement: g\n    measure: m\n",
    )
    .unwrap();
    ok(run_in(
        base,
        work,
        &["charter", "set", "--file", charter.to_str().unwrap()],
    ));
    let config = base.join("code/shop.kanbanr/projects/shop/config.yaml");
    let mut yaml = std::fs::read_to_string(&config).unwrap();
    yaml.push_str(
        "gates:\n  Business Arch:\n    requires: [definition, approved]\n  System Design:\n    requires: [approved]\n  Implementation:\n    requires: [approved]\n    on_enter: [branch]\n  Migration:\n    requires: [approved]\n",
    );
    std::fs::write(&config, yaml).unwrap();
    ok(run_in(
        base,
        work,
        &["milestone", "add", "--name", "M", "--code", "MS-001"],
    ));
    ok(run_in(
        base,
        work,
        &["feature", "add", "--title", "Cart", "--milestone", "MS-001"],
    ));
    let def = base.join("def.yaml");
    std::fs::write(&def, "statement: Keep a cart\n").unwrap();
    ok(run_in(
        base,
        work,
        &[
            "feature",
            "define",
            "FEAT-001",
            "--file",
            def.to_str().unwrap(),
        ],
    ));
    ok(run_in(base, work, &["approve", "FEAT-001"]));
}

/// FEAT-115 R-1: `start` goes where the workflow makes the branch — Implementation under TOGAF,
/// not the phase after Vision — and refuses to leap there, naming the stages in between.
#[test]
fn start_branches_where_the_gate_says() {
    let (base, work) = togaf_scratch("start-gate");
    togaf_board(&base, &work);
    let early = run_in(&base, &work, &["start", "FEAT-001"]);
    assert!(
        !early.status.success(),
        "a leap from Vision to Implementation is refused"
    );
    let why = String::from_utf8_lossy(&early.stderr);
    assert!(why.contains("Business Arch → System Design"), "{why}");
    // Nothing was branched for a start that did not happen.
    let branches = String::from_utf8_lossy(&git_in(&base, &work, &["branch"]).stdout).to_string();
    assert!(!branches.contains("feat/"), "{branches}");

    for phase in ["Business Arch", "System Design"] {
        let o = run_in(&base, &work, &["move", "FEAT-001", phase]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let started = run_in(&base, &work, &["start", "FEAT-001"]);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let said = String::from_utf8_lossy(&started.stdout);
    assert!(said.contains("created feat/FEAT-001"), "{said}");
    assert!(said.contains("FEAT-001 -> Implementation"), "{said}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-115 R-2: `finish` ends at a terminal the workflow allows from where the item is, and
/// otherwise names the path — it no longer jumps to the first terminal regardless.
#[test]
fn finish_reaches_a_terminal_through_an_allowed_edge() {
    let (base, work) = togaf_scratch("finish-edge");
    togaf_board(&base, &work);
    for phase in ["Business Arch", "System Design", "Implementation"] {
        let o = run_in(&base, &work, &["move", "FEAT-001", phase]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    // Implementation → Operations is not an edge in this workflow: finish says how to get there.
    let refused = run_in(&base, &work, &["finish", "FEAT-001"]);
    assert!(!refused.status.success());
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(why.contains("through Migration"), "{why}");
    // From Migration the end is one step away, and finish takes it.
    assert!(
        run_in(&base, &work, &["move", "FEAT-001", "Migration"])
            .status
            .success()
    );
    let done = run_in(&base, &work, &["finish", "FEAT-001"]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert!(String::from_utf8_lossy(&done.stdout).contains("FEAT-001 -> Operations"));
    let _ = std::fs::remove_dir_all(&base);
}
