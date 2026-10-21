//! Local (serverless) mode: the real CLI binary operates on the data folder directly with NO
//! server running. Runs in plain `cargo test` against an ephemeral data dir under the build output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

#[test]
fn cli_local_mode_without_a_server() {
    let base = std::env::temp_dir().join(format!("kanbanr-local-{}", std::process::id()));
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
    ok(&["--project", "demo", "move", "FEAT-001", "In Progress"]);
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
    let base = std::env::temp_dir().join(format!("kanbanr-init-{}", std::process::id()));
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
    // Resolved the way the CLI resolves paths: on Windows, without canonicalize's `\\?\` prefix.
    let board = kanbanr_core::project::normalize(&base.join("code").join("app.kanbanr"));
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
    run(&["move", "FEAT-001", "In Progress"]);
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
            // On Windows the command is `powershell … -File "<script>"`, so the path is inside it.
            .contains(home.to_str().unwrap()),
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
    assert!(chore.contains("affects_docs:"), "FEAT-180: {chore}");

    // The docs an item declares it changes (FEAT-180): stored as board paths, shown on the item,
    // and nothing outside the doc tree accepted.
    let docs_def = base.join("docs-def.yaml");
    std::fs::write(
        &docs_def,
        "statement: Keep a cart\naffects_docs: [architecture/data]\n",
    )
    .unwrap();
    run(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        docs_def.to_str().unwrap(),
    ]);
    let shown = run(&["feature", "show", "FEAT-001"]);
    assert!(
        shown.contains("**Affects docs:** `architecture/data.md`"),
        "{shown}"
    );
    std::fs::write(
        &docs_def,
        "statement: Keep a cart\naffects_docs: [../secrets]\n",
    )
    .unwrap();
    let refused = exec(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        docs_def.to_str().unwrap(),
    ]);
    assert!(
        !refused.status.success(),
        "a path out of the doc tree is refused"
    );

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
    run(&["move", "FEAT-001", "In Progress"]);
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
    run(&["move", "FEAT-001", "In Progress"]);
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
            run(&["move", code, "In Progress"]);
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
    // The trace is the subject here, not the method (FEAT-193).
    run(&["config", "method", "--off"]);
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

    // A doc that names the item in its text is an edge too (FEAT-181), followed both ways.
    run(&[
        "doc",
        "add",
        "design/cart.md",
        "--content",
        "# Cart\n\nHow FEAT-001 keeps a cart.\n",
    ]);
    let doc = run(&["trace", "design/cart.md"]);
    assert!(doc.contains("FEAT-001"), "{doc}");

    let why = run(&["why", "src/cart.rs:2"]);
    assert!(why.contains("Documented in: design/cart.md"), "{why}");
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

    // FEAT-153: the user's verdict on a proposed decision, from the CLI, on the record — and in
    // the project's activity like every other write.
    run(&["adr", "new", "Cache carts on the server"]);
    let refused = exec(&["adr", "accept", "ADR-0003"]);
    assert!(
        !refused.status.success(),
        "a scaffold has unanswered sections"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("Consequences"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let out = run(&[
        "adr",
        "reject",
        "ADR-0003",
        "--reason",
        "the browser is enough",
    ]);
    assert!(out.contains("ADR-0003 rejected by A"), "{out}");
    assert!(run(&["adr", "list"]).contains("rejected"));
    let activity = run(&["activity"]);
    assert!(activity.contains("reject decision ADR-0003"), "{activity}");

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
    // Named the platform's way: `.claude\settings.json` on Windows.
    let settings_shown = Path::new(".claude").join("settings.json");
    assert!(
        installed.contains(&*settings_shown.to_string_lossy()),
        "{installed}"
    );
    assert!(
        !installed.contains(r"\\?\"),
        "no verbatim prefix: {installed}"
    );
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
    run(&["move", "FEAT-001", "In Progress"]);

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
    let base = std::env::temp_dir().join(format!("kanbanr-repoint-{}", std::process::id()));
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

    // R-2: a legacy ./data board that already exists is still read. Creating it commits, which
    // needs someone to author it (FEAT-128): here, the user's own git identity.
    std::fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = Tester\n\temail = t@example.com\n",
    )
    .unwrap();
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
        .env_remove("KANBANR_PROCESSES_DIR")
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
    // The TOGAF phases, loaded from a process file (FEAT-116), with gates that ask only for
    // agreement and make the branch at Implementation, and no Implementation → Operations edge.
    let process = base.join("process.yaml");
    std::fs::write(
        &process,
        r#"statuses: [Vision, Business Arch, System Design, Implementation, Migration, Operations]
default_state: Vision
displayed_states: [Vision, Business Arch, System Design, Implementation, Migration, Operations]
terminal_states: [Operations]
transitions:
  Vision: [Business Arch]
  Business Arch: [System Design, Vision]
  System Design: [Implementation, Business Arch]
  Implementation: [Migration, System Design]
  Migration: [Operations, Implementation]
  Operations: [Migration]
gates:
  Business Arch:
    requires: [definition, approved]
  System Design:
    requires: [approved]
  Implementation:
    requires: [approved]
    on_enter: [branch]
  Migration:
    requires: [approved]
"#,
    )
    .unwrap();
    ok(run_in(
        base,
        work,
        &[
            "config",
            "workflow",
            "--from-file",
            process.to_str().unwrap(),
        ],
    ));
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

/// FEAT-117 R-1: `check` names what the next stage needs, stage by stage, under a phased process.
#[test]
fn check_names_what_the_next_stage_needs() {
    let (base, work) = togaf_scratch("check-next");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(
        &base,
        &work,
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
    ok(run_in(
        &base,
        &work,
        &["config", "workflow", "--preset", "togaf"],
    ));
    ok(run_in(
        &base,
        &work,
        &["milestone", "add", "--name", "M", "--code", "MS-001"],
    ));
    ok(run_in(
        &base,
        &work,
        &["feature", "add", "--title", "Cart", "--milestone", "MS-001"],
    ));
    let def = base.join("def.yaml");
    std::fs::write(&def, "statement: Keep a cart\n").unwrap();
    ok(run_in(
        &base,
        &work,
        &[
            "feature",
            "define",
            "FEAT-001",
            "--file",
            def.to_str().unwrap(),
        ],
    ));
    let out = ok(run_in(&base, &work, &["check", "FEAT-001"]));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("to move to Business Arch"), "{said}");
    assert!(
        said.contains("Who it is for"),
        "the stage's purpose is said: {said}"
    );
    assert!(said.contains("[MISSING: Who]"), "{said}");
    // The JSON carries it too, for Claude and scripts.
    let json = ok(run_in(&base, &work, &["check", "FEAT-001", "--json"]));
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v[0]["next"][0]["status"], "Business Arch");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-129: Windows gives a program's main thread 1 MiB of stack, and every command — even
/// `--version` — overflowed it in a debug build, so the whole suite failed on Windows while Linux,
/// with 8 MiB, never noticed. A 1 MiB stack limit reproduces Windows' main thread here.
#[cfg(unix)]
#[test]
fn the_cli_runs_within_a_one_mebibyte_main_stack() {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s 1024 && exec '{}' --version",
            cli().display()
        ))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("kanbanr "));
}

/// FEAT-168 R-1: `process check` reports every problem the board would refuse a process for —
/// including statuses it would empty of this project's items — and changes nothing, board or not.
#[test]
fn process_check_reports_problems_and_changes_nothing() {
    let (base, work) = togaf_scratch("process-check");
    togaf_board(&base, &work);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    let board = String::from_utf8(ok(run_in(&base, &work, &["where"])).stdout).unwrap();
    let board = std::path::PathBuf::from(board.trim());
    let state = || {
        let head = git_in(&base, &board, &["rev-parse", "HEAD"]).stdout;
        let dirty = git_in(&base, &board, &["status", "--porcelain"]).stdout;
        (head, dirty)
    };
    let before = state();

    // A file with four problems: all four are named, and it fails.
    let bad = base.join("bad.yaml");
    std::fs::write(
        &bad,
        "statuses: [Vision, Build]\ndefault_state: Vision\ndisplayed_states: [Vision, Build]\n\
         transitions: {Vision: [Ship]}\ngates:\n  Build: {requires: [approved, {zachman: [colour]}], \
         signoffs: [\"\"]}\n  QA: {}\n",
    )
    .unwrap();
    let out = run_in(&base, &work, &["process", "check", bad.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a bad process must fail");
    for want in [
        "'Ship'",
        "no name",
        "'colour'",
        "'QA'",
        "nothing was changed",
    ] {
        assert!(err.contains(want), "missing {want}: {err}");
    }

    // A good file that drops the status the project's item sits in: refused for this project.
    let out = run_in(&base, &work, &["process", "check", "pdca"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        err.contains("'Vision'") && err.contains("rename-status"),
        "{err}"
    );

    // The project's own workflow, exported, checks clean, with its agreement and diagram.
    let mine = base.join("mine.yaml");
    let export = ok(run_in(&base, &work, &["config", "workflow", "--export"])).stdout;
    std::fs::write(&mine, export).unwrap();
    let out = ok(run_in(
        &base,
        &work,
        &["process", "check", mine.to_str().unwrap()],
    ));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("no problems") && text.contains("checked against project shop"));
    assert!(text.contains("# Working agreement") && text.contains("stateDiagram-v2"));

    assert_eq!(state(), before, "the board was not touched");

    // No board at all: still checks, and makes none.
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = ok(run_in(&base, &empty, &["process", "check", "pdca"]));
    assert!(String::from_utf8_lossy(&out.stdout).contains("pdca: no problems"));
    assert_eq!(
        std::fs::read_dir(&empty).unwrap().count(),
        0,
        "no board made"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-169 R-3: a process saved on the board reaches a teammate through the board's remote — their
/// clone lists it and applies it, with no copy in anyone's home folder.
#[test]
fn a_teammate_clone_applies_the_boards_process() {
    let (base, work) = togaf_scratch("process-clone");
    togaf_board(&base, &work);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(
        &base,
        &work,
        &["process", "save", "ours", "--description", "Our TOGAF"],
    ));
    let board = String::from_utf8(ok(run_in(&base, &work, &["where"])).stdout).unwrap();
    let board = board.trim();

    // The teammate: another home, a clone of the board, a folder of their own.
    let mate = base.join("mate");
    std::fs::create_dir_all(mate.join("home")).unwrap();
    let clone = mate.join("board");
    ok(git_in(
        &base,
        &base,
        &["clone", "-q", board, clone.to_str().unwrap()],
    ));
    let theirs = |args: &[&str]| {
        let mut all = vec!["--data-dir", clone.to_str().unwrap()];
        all.extend_from_slice(args);
        run_in(&mate, &mate, &all)
    };
    ok(theirs(&["identity", "--name", "Mate", "--email", "m@x"]));
    let list = String::from_utf8(ok(theirs(&["process", "list"])).stdout).unwrap();
    assert!(
        list.lines()
            .any(|l| l.starts_with("ours") && l.contains("board") && l.contains("v1")),
        "{list}"
    );
    ok(theirs(&[
        "project",
        "init",
        "storefront",
        "--workflow",
        "ours",
    ]));
    let config = std::fs::read_to_string(clone.join("projects/storefront/config.yaml")).unwrap();
    assert!(config.contains("Business Arch"), "{config}");
    assert!(
        config.contains("name: ours") && config.contains("library: board"),
        "{config}"
    );
    assert!(
        !mate.join("home/.kanbanr").exists(),
        "nothing personal was needed"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-169 R-5: `~/.kanbanr/processes/` is a folder, never a project's marker — a folder under the
/// home folder with no board still has no board, and the personal process is still found.
#[test]
fn the_personal_folder_is_never_a_marker() {
    let (base, _work) = togaf_scratch("process-marker");
    let home = base.join("home");
    let inside = home.join("code").join("loose");
    std::fs::create_dir_all(&inside).unwrap();
    let file = base.join("p.yaml");
    std::fs::write(
        &file,
        "statuses: [Plan, Done]\ndefault_state: Plan\ndisplayed_states: [Plan, Done]\n\
         terminal_states: [Done]\ntransitions: {Plan: [Done]}\n",
    )
    .unwrap();
    let out = run_in(
        &base,
        &inside,
        &[
            "process",
            "save",
            "mine",
            "--personal",
            "--from-file",
            file.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join(".kanbanr/processes/mine.yaml").is_file());

    for dir in [&home, &inside] {
        let out = run_in(&base, dir, &["where", "--json"]);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_ne!(v["source"], "marker", "{dir:?} found a marker: {v}");
        // A command that needs a board says there is none, rather than using ~/.kanbanr.
        let out = run_in(&base, dir, &["board"]);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("no kanbanr board here"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let list = run_in(&base, &inside, &["process", "list"]);
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(
        list.lines()
            .any(|l| l.starts_with("mine") && l.contains("personal")),
        "{list}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// Save the shop's workflow as `ours`, then a version with one gate changed, as v2.
fn shop_with_a_newer_process(base: &std::path::Path, work: &std::path::Path) -> String {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(base, work, &["process", "save", "ours"]));
    // `ours` was saved from the project, which now records it as its source.
    ok(run_in(
        base,
        work,
        &["config", "workflow", "--preset", "ours"],
    ));
    let v1 = String::from_utf8(ok(run_in(base, work, &["config", "workflow", "--export"])).stdout)
        .unwrap();
    let v2 = v1.replace(
        "  Migration:\n    requires:\n    - approved\n",
        "  Migration:\n    requires:\n    - approved\n    - tests_green\n",
    );
    assert_ne!(v1, v2, "the export changed shape: {v1}");
    let file = base.join("v2.yaml");
    std::fs::write(&file, &v2).unwrap();
    ok(run_in(
        base,
        work,
        &[
            "process",
            "save",
            "ours",
            "--from-file",
            file.to_str().unwrap(),
        ],
    ));
    v1
}

/// FEAT-170 R-2: `process diff` names what changed between the project's workflow and the saved
/// version — and changes nothing.
#[test]
fn process_diff_names_what_changed() {
    let (base, work) = togaf_scratch("process-diff");
    togaf_board(&base, &work);
    let v1 = shop_with_a_newer_process(&base, &work);
    let out = run_in(&base, &work, &["process", "diff"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("ours v2"), "{text}");
    assert!(
        text.contains("gate 'Migration' requires") && text.contains("tests_green"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 2, "only the one change: {text}");
    let now = String::from_utf8(run_in(&base, &work, &["config", "workflow", "--export"]).stdout)
        .unwrap();
    assert_eq!(now, v1, "diff changed the project");
    let status = run_in(&base, &work, &["process", "status", "--if-changed"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("v2 is saved now"));
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-170 R-3: `process update` applies the saved version only when run, and is refused when a
/// status that still holds items would go, naming `config rename-status`.
#[test]
fn update_applies_on_request_and_keeps_occupied_statuses() {
    let (base, work) = togaf_scratch("process-update");
    togaf_board(&base, &work);
    shop_with_a_newer_process(&base, &work);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    // Applied on request.
    ok(run_in(&base, &work, &["process", "update"]));
    let now =
        String::from_utf8(ok(run_in(&base, &work, &["config", "workflow", "--export"])).stdout)
            .unwrap();
    assert!(now.contains("    - tests_green\n"), "{now}");
    let status =
        String::from_utf8(ok(run_in(&base, &work, &["process", "status"])).stdout).unwrap();
    assert!(status.contains("ours v2 (board), current"), "{status}");

    // v3 drops Vision, where the shop's item sits: refused, nothing changed.
    let mut v3: kanbanr_core::yaml::Value = kanbanr_core::yaml::from_str(&now).unwrap();
    let statuses = v3["statuses"].as_array_mut().unwrap();
    statuses.retain(|s| s.as_str() != Some("Vision"));
    v3["default_state"] = "Business Arch".into();
    for key in ["displayed_states", "terminal_states", "no_op_states"] {
        if let Some(list) = v3[key].as_array_mut() {
            list.retain(|s| s.as_str() != Some("Vision"));
        }
    }
    let transitions = v3["transitions"].as_object_mut().unwrap();
    transitions.remove("Vision");
    for (_, tos) in transitions.iter_mut() {
        tos.as_array_mut()
            .unwrap()
            .retain(|s| s.as_str() != Some("Vision"));
    }
    let v3 = kanbanr_core::yaml::to_string(&v3).unwrap();
    let file = base.join("v3.yaml");
    std::fs::write(&file, v3).unwrap();
    let saved = run_in(
        &base,
        &work,
        &[
            "process",
            "save",
            "ours",
            "--from-file",
            file.to_str().unwrap(),
        ],
    );
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    // The save names the projects it leaves behind, so each is asked about (FEAT-171).
    let said = String::from_utf8_lossy(&saved.stdout);
    assert!(
        said.contains("on an earlier version on this board: shop (v2)")
            && said.contains("Update or Not now"),
        "{said}"
    );
    let out = run_in(&base, &work, &["process", "update"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("'Vision'") && err.contains("rename-status"),
        "{err}"
    );
    let after =
        String::from_utf8(ok(run_in(&base, &work, &["config", "workflow", "--export"])).stdout)
            .unwrap();
    assert_eq!(after, now, "a refused update changed nothing");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-171 R-5: a draft is checked from stdin, so Claude can check it in plan mode, where it may
/// not write a file; and saved from stdin, with its description, once the plan is accepted.
#[test]
fn process_check_reads_a_draft_from_stdin() {
    use std::io::Write;
    let (base, _work) = togaf_scratch("process-stdin");
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let pipe = |args: &[&str], input: &str| {
        let mut child = Command::new(cli())
            .args(args)
            .current_dir(&empty)
            .env("HOME", base.join("home"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("KANBANR_PROCESSES_DIR")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let draft = "statuses: [Idea, Build, Done]\ndefault_state: Idea\n\
                 displayed_states: [Idea, Build, Done]\nterminal_states: [Done]\n\
                 transitions: {Idea: [Build], Build: [Done]}\n\
                 gates: {Build: {requires: [approved], signoffs: [design-review]}}\n";
    let out = pipe(&["process", "check", "-"], draft);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("the draft: no problems"), "{text}");
    assert!(text.contains("# Working agreement") && text.contains("stateDiagram-v2"));
    assert!(
        text.contains("design-review"),
        "the sign-off is in the agreement: {text}"
    );

    let bad = pipe(
        &["process", "check", "-"],
        &draft.replace("[Build]", "[Ship]"),
    );
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("'Ship'"));

    let saved = pipe(
        &[
            "process",
            "save",
            "ours",
            "--personal",
            "--description",
            "Idea to done, design reviewed",
            "--from-file",
            "-",
        ],
        draft,
    );
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let file = std::fs::read_to_string(base.join("home/.kanbanr/processes/ours.yaml")).unwrap();
    assert!(
        file.contains("description: Idea to done, design reviewed"),
        "{file}"
    );
    assert_eq!(
        std::fs::read_dir(&empty).unwrap().count(),
        0,
        "no board, no file here"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-171 / FEAT-173: `where --json` names the questions a setup must put to the user — the
/// board's place and the process always, the repository's git identity when git has none — and
/// none once the folder is set up.
#[test]
fn where_names_the_questions_setup_must_ask() {
    let (base, work) = togaf_scratch("where-ask");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(&base, &work, &["init", "-q", "-b", "main"]));
    // run_in's HOME has no .gitconfig, so git has no identity here.
    let ask = |dir: &std::path::Path| -> (serde_json::Value, Vec<String>) {
        let v: serde_json::Value =
            serde_json::from_slice(&ok(run_in(&base, dir, &["where", "--json"])).stdout).unwrap();
        let ask = v["ask_the_user"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q.as_str().unwrap().to_string())
            .collect();
        (v, ask)
    };
    let (v, questions) = ask(&work);
    assert_eq!(v["git_identity"]["name"], serde_json::Value::Null);
    assert_eq!(questions.len(), 4, "{questions:?}");
    assert!(questions[0].starts_with("Where the board lives"));
    assert!(
        questions[1].starts_with("Which process")
            && questions[1].contains("saved processes first")
            && questions[1].contains("after the board is chosen")
            && questions[3].contains("after the board's commit identity")
    );
    assert!(questions[3].contains("git identity") && questions[3].contains("never --global"));

    // With a repository identity, that question goes.
    ok(git_in(&base, &work, &["config", "user.name", "T"]));
    ok(git_in(&base, &work, &["config", "user.email", "t@x"]));
    let (v, questions) = ask(&work);
    assert_eq!(v["git_identity"]["name"], "T");
    assert_eq!(questions.len(), 3, "{questions:?}");

    // Set up: nothing left to ask.
    ok(run_in(
        &base,
        &work,
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
    assert!(ask(&work).1.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-194 R-2: setup asks the cadence third, after the process, and names the preview; the
/// preview runs before there is a board, each cadence gives its own model, and the model the board
/// then follows is the one previewed.
#[test]
fn setup_asks_cadence_and_shows_git_model() {
    let (base, work) = togaf_scratch("cadence-ask");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(&base, &work, &["init", "-q", "-b", "main"]));
    ok(git_in(
        &base,
        &work,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "[no-ref] initial commit",
        ],
    ));
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_slice(&ok(run_in(&base, &work, args)).stdout).unwrap()
    };

    let asked = json(&["where", "--json"]);
    let questions = asked["ask_the_user"].as_array().unwrap();
    assert_eq!(questions.len(), 4, "{questions:?}");
    let cadence = questions[2].as_str().unwrap();
    assert!(
        cadence.contains("sprints and releases")
            && cadence.contains("after the process")
            && cadence.contains("kanbanr git model"),
        "{cadence}"
    );

    // No board yet: the preview still answers.
    let preview = json(&[
        "git",
        "model",
        "--sprints",
        "on",
        "--releases",
        "off",
        "--json",
    ]);
    assert_eq!(preview["line"], "sprint/<SP>");
    assert_eq!(preview["branch_from"], "sprint/<SP>");
    assert_eq!(preview["production_at"], "sprint_close");
    assert_eq!(preview["pushes"], "you");
    assert!(
        !work.join("data").exists(),
        "a preview must not make a board"
    );

    let mut seen = std::collections::BTreeSet::new();
    for s in ["on", "off"] {
        for r in ["on", "off"] {
            let v = json(&["git", "model", "--sprints", s, "--releases", r, "--json"]);
            seen.insert(v.to_string());
        }
    }
    assert_eq!(seen.len(), 4, "each cadence gives its own model");

    // Applied: the board's model is the one previewed.
    ok(run_in(
        &base,
        &work,
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
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--sprints", "on"],
    ));
    let applied = json(&["git", "model", "--json"]);
    for k in [
        "topology",
        "line",
        "branch_from",
        "production_at",
        "kanbanr_merges",
        "automated",
        "pushes",
    ] {
        assert_eq!(applied[k], preview[k], "{k}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-182: a sign-off can be withdrawn from the CLI with a reason, never without one, and the
/// brief then says it was withdrawn and why.
#[test]
fn cli_signoff_withdraw_round_trip() {
    let (base, work) = togaf_scratch("signoff-withdraw");
    togaf_board(&base, &work);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(
        &base,
        &work,
        &["signoff", "FEAT-001", "design-review"],
    ));
    let refused = run_in(
        &base,
        &work,
        &["signoff", "FEAT-001", "design-review", "--withdraw"],
    );
    assert!(!refused.status.success(), "a withdrawal needs --reason");
    let said = String::from_utf8(
        ok(run_in(
            &base,
            &work,
            &[
                "signoff",
                "FEAT-001",
                "design-review",
                "--withdraw",
                "--reason",
                "given for the old design",
            ],
        ))
        .stdout,
    )
    .unwrap();
    assert!(
        said.contains("sign-off 'design-review' withdrawn"),
        "{said}"
    );
    let shown =
        String::from_utf8(ok(run_in(&base, &work, &["feature", "show", "FEAT-001"])).stdout)
            .unwrap();
    assert!(
        shown.contains("Sign-off `design-review` withdrawn")
            && shown.contains("given for the old design"),
        "{shown}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-192 R-1: inside a linked git worktree (`.claude/worktrees/w1`), the committed marker's
/// relative board path is resolved from the main worktree, so commands find the board with no
/// extra environment variables.
#[test]
fn marker_resolves_from_linked_worktree() {
    let (base, work) = togaf_scratch("worktree");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(&base, &work, &["init", "-q", "-b", "main"]));
    ok(run_in(
        &base,
        &work,
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
    // The marker is committed, as in a real repository, so the worktree checks out its own copy.
    ok(git_in(&base, &work, &["add", ".kanbanr"]));
    ok(git_in(
        &base,
        &work,
        &["commit", "-q", "-m", "[no-ref] marker"],
    ));
    let main_board = String::from_utf8(ok(run_in(&base, &work, &["where"])).stdout).unwrap();
    ok(git_in(
        &base,
        &work,
        &["worktree", "add", "-q", ".claude/worktrees/w1", "-b", "w1"],
    ));
    let wt = work.join(".claude").join("worktrees").join("w1");
    assert!(
        wt.join(".kanbanr").is_file(),
        "the worktree has its own marker"
    );

    let from_wt = String::from_utf8(ok(run_in(&base, &wt, &["where"])).stdout).unwrap();
    assert_eq!(
        from_wt.trim(),
        main_board.trim(),
        "the main board, found from the worktree"
    );
    let board = String::from_utf8(ok(run_in(&base, &wt, &["board"])).stdout).unwrap();
    assert!(
        board.contains("# shop board") || board.contains("board"),
        "{board}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-188: `process checks` lists the stage roles a gate can hold or be addressed by, in text and
/// JSON, without a board.
#[test]
fn process_checks_lists_roles() {
    let (base, _work) = togaf_scratch("checks-roles");
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = run_in(&base, &empty, &["process", "checks"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for role in ["design", "build", "integrate", "release"] {
        assert!(text.contains(&format!("\n{role}")), "{role}: {text}");
    }
    let json = run_in(&base, &empty, &["process", "checks", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["roles"].as_array().unwrap().len(), 4);
    assert_eq!(v["roles"][1]["role"], "build");
    let _ = std::fs::remove_dir_all(&base);
}

/// A code repository on `main` with one commit and its own commit identity, beside a board with
/// releases on and `v1.0.0` added — the shape FEAT-183's promotions act on.
fn release_scratch(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let (base, work) = togaf_scratch(tag);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(&base, &work, &["init", "-q", "-b", "main"]));
    // The tag is made by the person's own git, so the repository needs someone to author it.
    ok(git_in(&base, &work, &["config", "user.name", "T"]));
    ok(git_in(&base, &work, &["config", "user.email", "t@x"]));
    // The tests read files back byte for byte; Windows's git would check them out with CRLF
    // (FEAT-208). The setting is the repository's, so kanbanr's own git honours it too.
    ok(git_in(&base, &work, &["config", "core.autocrlf", "false"]));
    std::fs::write(work.join("app.txt"), "v0\n").unwrap();
    ok(git_in(&base, &work, &["add", "app.txt"]));
    ok(git_in(
        &base,
        &work,
        &["commit", "-q", "-m", "[no-ref] base"],
    ));
    ok(run_in(
        &base,
        &work,
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
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--releases", "on"],
    ));
    ok(run_in(&base, &work, &["release", "add", "v1.0.0"]));
    (base, work)
}

/// The commit a name points at in the scratch repository, or "" when it names none.
fn rev(base: &std::path::Path, work: &std::path::Path, name: &str) -> String {
    let o = git_in(base, work, &["rev-parse", "--verify", "--quiet", name]);
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Commit a change to `app.txt` on `branch`, leaving `main` checked out afterwards.
fn commit_on(base: &std::path::Path, work: &std::path::Path, branch: &str, content: &str) {
    assert!(
        git_in(base, work, &["checkout", "-q", branch])
            .status
            .success()
    );
    std::fs::write(work.join("app.txt"), content).unwrap();
    assert!(
        git_in(base, work, &["commit", "-q", "-am", "[no-ref] change"])
            .status
            .success()
    );
    assert!(
        git_in(base, work, &["checkout", "-q", "main"])
            .status
            .success()
    );
}

fn release_json(
    base: &std::path::Path,
    work: &std::path::Path,
    version: &str,
) -> serde_json::Value {
    let o = run_in(base, work, &["--json", "release", "list"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let all: Vec<serde_json::Value> = serde_json::from_slice(&o.stdout).unwrap();
    all.into_iter()
        .find(|r| r["version"] == version)
        .expect("the release is listed")
}

/// FEAT-183 R-2: promoting a release to candidate makes `release/<VERSION>` from the integration
/// branch — the repository's default when the project names none — without checking it out.
#[test]
fn promote_candidate_creates_branch() {
    let (base, work) = release_scratch("promote-candidate");
    let o = run_in(&base, &work, &["release", "promote", "v1.0.0"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(said.contains("v1.0.0 -> candidate"), "{said}");
    assert!(said.contains("created release/v1.0.0"), "{said}");
    assert_eq!(
        rev(&base, &work, "release/v1.0.0"),
        rev(&base, &work, "main")
    );
    let head = git_in(&base, &work, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");
    let r = release_json(&base, &work, "v1.0.0");
    assert_eq!(r["stage"], "candidate");
    assert_eq!(r["branch"], "release/v1.0.0");
    assert_eq!(
        r["stages"][0]["by"], "A",
        "the board's commit identity moved it"
    );

    // A named integration branch is where the next one comes from.
    assert!(
        git_in(&base, &work, &["branch", "develop"])
            .status
            .success()
    );
    commit_on(&base, &work, "develop", "on develop\n");
    let o = run_in(
        &base,
        &work,
        &["config", "cadence", "--integration-branch", "develop"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = run_in(&base, &work, &["release", "add", "v1.1.0", "--branch"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        rev(&base, &work, "release/v1.1.0"),
        rev(&base, &work, "develop")
    );

    // An existing branch of that name is refused, and the board does not move.
    assert!(
        git_in(&base, &work, &["branch", "release/v2.0.0"])
            .status
            .success()
    );
    assert!(
        run_in(&base, &work, &["release", "add", "v2.0.0"])
            .status
            .success()
    );
    let o = run_in(&base, &work, &["release", "promote", "v2.0.0", "candidate"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("already exists"));
    assert!(release_json(&base, &work, "v2.0.0").get("stage").is_none());

    // Outside any code repository, the stage that needs one is refused, saying why.
    let board = String::from_utf8(run_in(&base, &work, &["where"]).stdout).unwrap();
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join(".kanbanr"),
        format!("project: shop\ndata_dir: {}\n", board.trim()),
    )
    .unwrap();
    let o = run_in(&base, &elsewhere, &["release", "add", "v3.0.0", "--branch"]);
    assert!(!o.status.success());
    let why = String::from_utf8_lossy(&o.stderr);
    assert!(why.contains("needs the code repository"), "{why}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-183 R-3: promoting to production cuts the release, tags its tip with the version and
/// brings main to exactly that commit; `cut --tag` does the same.
#[test]
fn promote_production_cuts_tags_merges() {
    let (base, work) = release_scratch("promote-production");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(&base, &work, &["release", "promote", "v1.0.0"]));
    commit_on(&base, &work, "release/v1.0.0", "fixed on the release\n");
    ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.0", "staging"],
    ));
    let o = ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.0", "production"],
    ));
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(
        said.contains("tagged v1.0.0, main fast-forwarded to it"),
        "{said}"
    );
    let tip = rev(&base, &work, "release/v1.0.0");
    assert_eq!(rev(&base, &work, "v1.0.0^{commit}"), tip);
    assert_eq!(rev(&base, &work, "main"), tip);
    assert_eq!(
        std::fs::read_to_string(work.join("app.txt")).unwrap(),
        "fixed on the release\n",
        "the checked-out main moved with its files"
    );
    let r = release_json(&base, &work, "v1.0.0");
    assert_eq!(
        (r["state"].as_str(), r["stage"].as_str()),
        (Some("shipped"), Some("production"))
    );
    assert_eq!(r["notes_doc"], "releases/v1.0.0.md");
    let docs = ok(run_in(&base, &work, &["doc", "list"]));
    assert!(String::from_utf8_lossy(&docs.stdout).contains("releases/v1.0.0.md"));
    let moves: Vec<&str> = r["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["stage"].as_str())
        .collect();
    assert_eq!(moves, ["candidate", "staging", "production"]);

    // `cut --tag` is the same promotion.
    ok(run_in(
        &base,
        &work,
        &["release", "add", "v1.1.0", "--branch"],
    ));
    commit_on(&base, &work, "release/v1.1.0", "v1.1\n");
    ok(run_in(&base, &work, &["release", "cut", "v1.1.0", "--tag"]));
    assert_eq!(
        rev(&base, &work, "main"),
        rev(&base, &work, "release/v1.1.0")
    );
    assert_eq!(
        rev(&base, &work, "v1.1.0^{commit}"),
        rev(&base, &work, "main")
    );
    assert_eq!(release_json(&base, &work, "v1.1.0")["state"], "shipped");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-183 R-4: when main has commits the release lacks — here, a conflicting change to the same
/// file — production is refused and nothing changes: not the board, not a tag, not main.
#[test]
fn promote_production_refuses_unclean_merge() {
    let (base, work) = release_scratch("promote-unclean");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(&base, &work, &["release", "promote", "v1.0.0"]));
    commit_on(&base, &work, "release/v1.0.0", "the release's line\n");
    commit_on(&base, &work, "main", "main's line\n");
    let main_before = rev(&base, &work, "main");
    let board = std::path::PathBuf::from(
        String::from_utf8(ok(run_in(&base, &work, &["where"])).stdout)
            .unwrap()
            .trim(),
    );
    let shop = board.join("projects").join("shop");
    let releases = || std::fs::read_to_string(shop.join("releases.yaml")).unwrap();
    let before = releases();

    let o = run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.0", "production"],
    );
    assert!(
        !o.status.success(),
        "a release missing main's commits is refused"
    );
    let why = String::from_utf8_lossy(&o.stderr);
    assert!(
        why.contains("main has commits release/v1.0.0 lacks"),
        "{why}"
    );
    assert!(why.contains("nothing was changed"), "{why}");
    assert_eq!(rev(&base, &work, "main"), main_before, "main did not move");
    assert_eq!(rev(&base, &work, "v1.0.0"), "", "no tag was made");
    assert_eq!(releases(), before, "the board's releases are untouched");
    let r = release_json(&base, &work, "v1.0.0");
    assert_eq!(
        (r["state"].as_str(), r["stage"].as_str()),
        (Some("planned"), Some("candidate"))
    );
    let notes = ok(run_in(&base, &work, &["doc", "list"]));
    assert!(
        !String::from_utf8_lossy(&notes.stdout).contains("v1.0.0"),
        "no notes were written"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// Every file under `dir`, with its bytes, to show nothing was written.
fn snapshot(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// FEAT-195 R-1: under the fence, a board outside the temporary directory is refused before it is
/// read or written. The board here is made with plain file writes, never through kanbanr, in the
/// build tree — exactly where the test that reached the real board kept its scratch folder.
#[test]
fn board_outside_the_fence_is_refused() {
    let outside = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("fenced-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let data = outside.join("data");
    std::fs::create_dir_all(data.join("projects").join("shop")).unwrap();
    std::fs::write(
        data.join("projects").join("shop").join("config.yaml"),
        "name: shop\n",
    )
    .unwrap();
    let before = snapshot(&outside);

    for args in [
        vec!["board", "--project", "shop"],
        vec![
            "init",
            "shop",
            "--author",
            "A",
            "--email",
            "a@x",
            "--no-hooks",
        ],
        vec!["identity", "--name", "A", "--email", "a@x"],
    ] {
        let mut full: Vec<&str> = args.clone();
        full.extend(["--data-dir", data.to_str().unwrap()]);
        let out = Command::new(cli())
            .args(&full)
            .current_dir(&outside)
            .env("KANBANR_BOARD_FENCE", "temp")
            .env_remove("KANBANR_DATA_DIR")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run kanbanr CLI");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} ran outside the fence");
        assert!(
            err.contains("outside KANBANR_BOARD_FENCE"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(
        snapshot(&outside),
        before,
        "nothing outside the fence was touched"
    );
    let _ = std::fs::remove_dir_all(&outside);
}

/// FEAT-195 R-2: every test runs fenced to the temporary directory (api/.cargo/config.toml), and
/// no test keeps its scratch folder in the build tree, which sits under this repository's marker.
#[test]
fn tests_run_inside_the_fence() {
    assert_eq!(
        std::env::var("KANBANR_BOARD_FENCE").as_deref(),
        Ok("temp"),
        "cargo sets the fence for every test"
    );
    let (base, work) = togaf_scratch("fence-check");
    assert!(work.starts_with(std::env::temp_dir()), "{}", work.display());
    let _ = std::fs::remove_dir_all(&base);

    // Only the refusal test above may name the build tree, and only to stand outside the fence.
    let in_tree = concat!("env!(\"CARGO_TARGET", "_TMPDIR\")");
    for (name, source) in [
        ("local_mode.rs", include_str!("local_mode.rs")),
        ("view_daemon.rs", include_str!("view_daemon.rs")),
        ("integration.rs", include_str!("integration.rs")),
    ] {
        let uses = source.matches(in_tree).count();
        let allowed = usize::from(name == "local_mode.rs");
        assert_eq!(
            uses, allowed,
            "{name} keeps a scratch folder in the build tree"
        );
    }
}

/// FEAT-193 R-2: the method is on in a built-in process and can be turned off, all of it or part
/// by part. Off, an item with no Design section enters the build stage; the project's record of
/// its process stays current, because opting out is a choice, not an edit; and turning one part
/// back on brings back that part alone.
#[test]
fn method_gates_opt_out() {
    let (base, work) = togaf_scratch("method-opt-out");
    togaf_board(&base, &work);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(
        &base,
        &work,
        &["config", "workflow", "--preset", "togaf"],
    ));
    let shown = ok(run_in(&base, &work, &["config", "method"]));
    assert!(
        shown.contains("design        on: Implementation"),
        "{shown}"
    );
    assert!(shown.contains("docs          on: Migration"), "{shown}");
    for phase in ["Business Arch", "System Design"] {
        ok(run_in(
            &base,
            &work,
            &[
                "move",
                "FEAT-001",
                phase,
                "--override",
                "not what this test is about",
            ],
        ));
    }
    let refused = run_in(&base, &work, &["move", "FEAT-001", "Implementation"]);
    assert!(!refused.status.success());
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(
        why.contains("no 'Design' section") && why.contains("design-review"),
        "{why}"
    );

    ok(run_in(&base, &work, &["config", "method", "--off"]));
    let parts: serde_json::Value =
        serde_json::from_str(&ok(run_in(&base, &work, &["--json", "config", "method"]))).unwrap();
    let parts = parts["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 5);
    assert!(parts.iter().all(|p| p["on"] == false), "{parts:?}");
    let status = ok(run_in(&base, &work, &["process", "status"]));
    assert!(
        !status.contains("edited"),
        "an opt-out is not an edit: {status}"
    );
    ok(run_in(
        &base,
        &work,
        &["move", "FEAT-001", "Implementation"],
    ));

    ok(run_in(
        &base,
        &work,
        &["config", "method", "--on", "design"],
    ));
    let shown = ok(run_in(&base, &work, &["config", "method"]));
    assert!(
        shown.contains("design        on: Implementation"),
        "{shown}"
    );
    assert!(shown.contains("design-review off"), "{shown}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-189: a code repository with a first commit, a default-process board with its charter
/// adopted, the cadence given, the git model on and the hooks installed, and one approved item.
fn git_model_board(base: &std::path::Path, work: &std::path::Path, sprints: bool, releases: bool) {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(base, work, &["init", "--initial-branch=main"]));
    ok(git_in(base, work, &["config", "user.name", "T"]));
    ok(git_in(base, work, &["config", "user.email", "t@x"]));
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
    // The method is not what these tests are about (FEAT-193).
    ok(run_in(base, work, &["config", "method", "--off"]));
    let on_off = |b: bool| if b { "on" } else { "off" };
    ok(run_in(
        base,
        work,
        &[
            "config",
            "cadence",
            "--sprints",
            on_off(sprints),
            "--releases",
            on_off(releases),
            "--git-model",
            "on",
        ],
    ));
    ok(run_in(base, work, &["git", "install-hooks"]));
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

fn branches(base: &std::path::Path, work: &std::path::Path) -> String {
    String::from_utf8_lossy(&git_in(base, work, &["branch", "--format=%(refname:short)"]).stdout)
        .to_string()
}

/// FEAT-189 R-2: with releases on, `start` branches from the integration line, making it from
/// main the first time.
#[test]
fn start_branches_from_integration_line() {
    let (base, work) = togaf_scratch("gm-start");
    git_model_board(&base, &work, false, true);
    let model = run_in(&base, &work, &["git", "model"]);
    let said = String::from_utf8_lossy(&model.stdout);
    assert!(said.contains("line:       integration"), "{said}");
    assert!(said.contains("pushing:    never automatic"), "{said}");

    let started = run_in(&base, &work, &["start", "FEAT-001"]);
    let out = String::from_utf8_lossy(&started.stdout);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    assert!(out.contains("created integration from main"), "{out}");
    assert!(out.contains("from integration"), "{out}");
    let merge_base = git_in(
        &base,
        &work,
        &["merge-base", "--is-ancestor", "integration", "HEAD"],
    );
    assert!(
        merge_base.status.success(),
        "the item branch grows from integration"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-189 R-3: starting a sprint makes its branch — the first from main, the next from the
/// sprint that has not shipped, and from main again once main holds it.
#[test]
fn sprint_start_creates_branch() {
    let (base, work) = togaf_scratch("gm-sprint");
    // Sprints and releases: closing a sprint leaves main alone (production is the release cut), so
    // the next sprint has an unshipped one to build on. Sprints alone, the close moves main
    // (FEAT-190).
    git_model_board(&base, &work, true, true);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    for start in ["2026-10-05", "2026-10-19", "2026-11-02"] {
        ok(run_in(&base, &work, &["sprint", "add", "--start", start]));
    }
    let said = ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    assert!(said.contains("created sprint/SP-001 from main"), "{said}");
    // Work lands on the sprint, so the next sprint has something main does not.
    ok(git_in(&base, &work, &["checkout", "-q", "sprint/SP-001"]));
    ok(git_in(
        &base,
        &work,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--allow-unrelated-histories",
            "main",
            "-m",
            "x\n\nRefs: kanbanr:FEAT-001",
        ],
    ));
    ok(git_in(
        &base,
        &work,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "--no-verify",
            "-m",
            "sprint work",
        ],
    ));
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));
    ok(run_in(&base, &work, &["sprint", "close", "SP-001"]));
    let said = ok(run_in(&base, &work, &["sprint", "start", "SP-002"]));
    assert!(
        said.contains("created sprint/SP-002 from sprint/SP-001"),
        "{said}"
    );

    // Once main holds both, the next sprint starts from main.
    ok(git_in(
        &base,
        &work,
        &["merge", "-q", "--ff-only", "sprint/SP-002"],
    ));
    ok(run_in(&base, &work, &["sprint", "close", "SP-002"]));
    let said = ok(run_in(&base, &work, &["sprint", "start", "SP-003"]));
    assert!(said.contains("created sprint/SP-003 from main"), "{said}");
    assert!(branches(&base, &work).contains("sprint/SP-003"));
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-189 R-4: on a line of integration a plain commit is refused, and a merge must say which
/// items it brings; item, fix and spike branches commit as before.
#[test]
fn hook_guards_integration_lines() {
    let (base, work) = togaf_scratch("gm-hooks");
    git_model_board(&base, &work, true, false);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(run_in(
        &base,
        &work,
        &["sprint", "add", "--start", "2026-10-05"],
    ));
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    // Real commits through the real hooks: the hooks run `kanbanr`, so put this build on PATH.
    let path = format!(
        "{}:{}",
        cli().parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let git_in = |base: &std::path::Path, work: &std::path::Path, args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(work)
            .env("HOME", base.join("home"))
            .env("PATH", &path)
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("run git")
    };
    let commit = |msg: &str| git_in(&base, &work, &["commit", "-q", "--allow-empty", "-m", msg]);

    for line in ["main", "sprint/SP-001"] {
        ok(git_in(&base, &work, &["checkout", "-q", line]));
        let refused = commit("work\n\nRefs: kanbanr:FEAT-001");
        assert!(
            !refused.status.success(),
            "a plain commit on {line} is refused"
        );
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("line of integration"),
            "{}",
            String::from_utf8_lossy(&refused.stderr)
        );
    }

    // An item branch takes work; a merge into the line must name the item.
    ok(git_in(
        &base,
        &work,
        &[
            "checkout",
            "-q",
            "-b",
            "feat/FEAT-001-cart",
            "sprint/SP-001",
        ],
    ));
    ok(commit("feat: cart\n\nRefs: kanbanr:FEAT-001"));
    ok(git_in(&base, &work, &["checkout", "-q", "sprint/SP-001"]));
    let generated = git_in(
        &base,
        &work,
        &["merge", "--no-ff", "--no-edit", "feat/FEAT-001-cart"],
    );
    assert!(
        !generated.status.success(),
        "a generated merge message names no item"
    );
    let _ = git_in(&base, &work, &["merge", "--abort"]);
    ok(git_in(
        &base,
        &work,
        &[
            "merge",
            "--no-ff",
            "feat/FEAT-001-cart",
            "-m",
            "Merge FEAT-001\n\nRefs: kanbanr:FEAT-001",
        ],
    ));

    // fix/ and spike/ branches commit as item branches do.
    ok(git_in(
        &base,
        &work,
        &[
            "checkout",
            "-q",
            "-b",
            "fix/FEAT-001-crash",
            "sprint/SP-001",
        ],
    ));
    ok(commit("fix: crash\n\nRefs: kanbanr:FEAT-001"));
    ok(git_in(
        &base,
        &work,
        &["checkout", "-q", "-b", "spike/try", "sprint/SP-001"],
    ));
    ok(commit("spike\n\n[no-ref] exploring"));
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-190: add a defined, approved item — a defect when `kind` says so.
fn approved_item(
    base: &std::path::Path,
    work: &std::path::Path,
    code: &str,
    title: &str,
    kind: Option<&str>,
) {
    let ok = |o: Output| assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let mut add = vec!["feature", "add", "--title", title, "--milestone", "MS-001"];
    if let Some(k) = kind {
        add.extend(["--kind", k]);
    }
    ok(run_in(base, work, &add));
    let def = base.join(format!("{code}.yaml"));
    std::fs::write(&def, format!("statement: {title}\n")).unwrap();
    ok(run_in(
        base,
        work,
        &["feature", "define", code, "--file", def.to_str().unwrap()],
    ));
    ok(run_in(base, work, &["approve", code]));
}

/// Commit a file on whatever is checked out, past the hooks (they are not what these test).
fn commit_file_in(
    base: &std::path::Path,
    work: &std::path::Path,
    file: &str,
    content: &str,
    msg: &str,
) {
    std::fs::write(work.join(file), content).unwrap();
    for args in [
        &["add", file][..],
        &["commit", "-q", "--no-verify", "-m", msg],
    ] {
        let o = git_in(base, work, args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

fn tip(base: &std::path::Path, work: &std::path::Path, rev: &str) -> String {
    String::from_utf8_lossy(&git_in(base, work, &["rev-parse", rev]).stdout)
        .trim()
        .to_string()
}

/// FEAT-190 R-1: entering the integrate stage merges the item's branch into its line, with a merge
/// commit that names the item — main in trunk, the sprint's branch with sprints.
#[test]
fn integrate_merges_item_branch() {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    // Trunk: into main.
    let (base, work) = togaf_scratch("gm-integrate-trunk");
    git_model_board(&base, &work, false, false);
    ok(run_in(&base, &work, &["start", "FEAT-001"]));
    commit_file_in(
        &base,
        &work,
        "cart.txt",
        "cart\n",
        "feat: cart\n\nRefs: kanbanr:FEAT-001",
    );
    let said = ok(run_in(&base, &work, &["move", "FEAT-001", "Completed"]));
    assert!(
        said.contains("merged feat/FEAT-001-cart into main"),
        "{said}"
    );
    let message = String::from_utf8_lossy(
        &git_in(&base, &work, &["log", "-1", "--format=%B", "main"]).stdout,
    )
    .to_string();
    assert!(message.contains("Refs: kanbanr:FEAT-001"), "{message}");
    let parents = String::from_utf8_lossy(
        &git_in(&base, &work, &["rev-list", "--parents", "-n1", "main"]).stdout,
    )
    .to_string();
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "a merge commit: {parents}"
    );
    assert!(
        branches(&base, &work).contains("feat/FEAT-001-cart"),
        "the branch is kept (Q3)"
    );
    let _ = std::fs::remove_dir_all(&base);

    // Sprints: into the running sprint's branch, and main is untouched until the close.
    let (base, work) = togaf_scratch("gm-integrate-sprint");
    git_model_board(&base, &work, true, false);
    ok(run_in(
        &base,
        &work,
        &["sprint", "add", "--start", "2026-10-05"],
    ));
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    let main = tip(&base, &work, "main");
    ok(run_in(&base, &work, &["start", "FEAT-001"]));
    commit_file_in(
        &base,
        &work,
        "cart.txt",
        "cart\n",
        "feat: cart\n\nRefs: kanbanr:FEAT-001",
    );
    let said = ok(run_in(&base, &work, &["move", "FEAT-001", "Completed"]));
    assert!(said.contains("into sprint/SP-001"), "{said}");
    assert_eq!(tip(&base, &work, "main"), main);
    let in_sprint = git_in(
        &base,
        &work,
        &[
            "merge-base",
            "--is-ancestor",
            "feat/FEAT-001-cart",
            "sprint/SP-001",
        ],
    );
    assert!(in_sprint.status.success());
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-190 R-2: a merge that would conflict refuses the move, names the paths, and changes
/// nothing — the item's status, the line's tip and the working tree.
#[test]
fn integrate_merge_conflict_refused() {
    let (base, work) = togaf_scratch("gm-conflict");
    git_model_board(&base, &work, false, false);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(&base, &work, &["start", "FEAT-001"]));
    commit_file_in(
        &base,
        &work,
        "a.txt",
        "the item's\n",
        "feat: a\n\nRefs: kanbanr:FEAT-001",
    );
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));
    commit_file_in(
        &base,
        &work,
        "a.txt",
        "main's\n",
        "x\n\n[no-ref] a test of a conflict",
    );
    ok(git_in(
        &base,
        &work,
        &["checkout", "-q", "feat/FEAT-001-cart"],
    ));
    let main = tip(&base, &work, "main");

    let refused = run_in(&base, &work, &["move", "FEAT-001", "Completed"]);
    assert!(!refused.status.success());
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(why.contains("conflicts in: a.txt"), "{why}");
    assert_eq!(tip(&base, &work, "main"), main, "the line did not move");
    let shown = ok(run_in(&base, &work, &["feature", "show", "FEAT-001"]));
    assert!(shown.contains("**Status:** In Progress"), "{shown}");
    let status = ok(git_in(&base, &work, &["status", "--porcelain"]));
    assert!(
        status.trim().is_empty() || !status.contains("a.txt"),
        "{status}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-190: in the sprints topology a sprint's close is production — main fast-forwards to the
/// sprint and is tagged with its code — and it refuses, changing nothing, when main has moved on.
#[test]
fn sprints_close_fast_forwards_and_tags() {
    let (base, work) = togaf_scratch("gm-close");
    git_model_board(&base, &work, true, false);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    for start in ["2026-10-05", "2026-10-19"] {
        ok(run_in(&base, &work, &["sprint", "add", "--start", start]));
    }
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    ok(git_in(&base, &work, &["checkout", "-q", "sprint/SP-001"]));
    commit_file_in(&base, &work, "s.txt", "s\n", "x\n\n[no-ref] sprint work");
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));
    let said = ok(run_in(&base, &work, &["sprint", "close", "SP-001"]));
    assert!(said.contains("tagged SP-001"), "{said}");
    assert_eq!(
        tip(&base, &work, "main"),
        tip(&base, &work, "sprint/SP-001")
    );
    assert_eq!(
        tip(&base, &work, "SP-001^{commit}"),
        tip(&base, &work, "main")
    );

    // Main moves on by itself: the next close is not a fast-forward, and nothing changes.
    ok(run_in(&base, &work, &["sprint", "start", "SP-002"]));
    commit_file_in(
        &base,
        &work,
        "hotfix.txt",
        "h\n",
        "x\n\n[no-ref] a hotfix on main",
    );
    let main = tip(&base, &work, "main");
    let refused = run_in(&base, &work, &["sprint", "close", "SP-002"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("cannot fast-forward"));
    assert_eq!(tip(&base, &work, "main"), main);
    let tags = ok(git_in(&base, &work, &["tag"]));
    assert!(!tags.contains("SP-002"), "{tags}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-190 R-3: items carried into the next sprint are brought up to date with its branch by a
/// merge into theirs — never a rebase, the branch kept — and one that would conflict is reported
/// and left alone.
#[test]
fn carry_over_catch_up_merge() {
    let (base, work) = togaf_scratch("gm-carry");
    git_model_board(&base, &work, true, false);
    approved_item(&base, &work, "FEAT-002", "Checkout", None);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    for start in ["2026-10-05", "2026-10-19"] {
        ok(run_in(&base, &work, &["sprint", "add", "--start", start]));
    }
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    ok(run_in(
        &base,
        &work,
        &["sprint", "plan", "SP-001", "FEAT-001", "FEAT-002"],
    ));
    ok(run_in(&base, &work, &["start", "FEAT-001"]));
    commit_file_in(
        &base,
        &work,
        "cart.txt",
        "cart\n",
        "wip\n\nRefs: kanbanr:FEAT-001",
    );
    ok(run_in(&base, &work, &["start", "FEAT-002"]));
    commit_file_in(
        &base,
        &work,
        "shared.txt",
        "the item's\n",
        "wip\n\nRefs: kanbanr:FEAT-002",
    );
    let feat2 = tip(&base, &work, "feat/FEAT-002-checkout");
    // The sprint moves on underneath both.
    ok(git_in(&base, &work, &["checkout", "-q", "sprint/SP-001"]));
    commit_file_in(
        &base,
        &work,
        "shared.txt",
        "the sprint's\n",
        "x\n\n[no-ref] other work",
    );
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));

    ok(run_in(
        &base,
        &work,
        &["sprint", "close", "SP-001", "--carry-to", "SP-002"],
    ));
    let said = ok(run_in(&base, &work, &["sprint", "start", "SP-002"]));
    assert!(said.contains("caught up feat/FEAT-001-cart"), "{said}");
    assert!(
        said.contains("feat/FEAT-002-checkout (FEAT-002) conflicts in shared.txt"),
        "{said}"
    );
    let caught = git_in(
        &base,
        &work,
        &[
            "merge-base",
            "--is-ancestor",
            "sprint/SP-002",
            "feat/FEAT-001-cart",
        ],
    );
    assert!(
        caught.status.success(),
        "the sprint is in the carried branch"
    );
    assert_eq!(
        tip(&base, &work, "feat/FEAT-002-checkout"),
        feat2,
        "the conflicting one is untouched"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-190 R-4: a fix planned into a release under test integrates into the release's branch, the
/// move says the line lacks it, and `git forward` merges it there naming the fix — once.
#[test]
fn fixes_forward_merged() {
    let (base, work) = togaf_scratch("gm-forward");
    git_model_board(&base, &work, false, true);
    approved_item(&base, &work, "FEAT-002", "Cart loses items", Some("bug"));
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(&base, &work, &["release", "add", "v1"]));
    ok(run_in(&base, &work, &["release", "promote", "v1"]));
    assert!(branches(&base, &work).contains("release/v1"));
    ok(run_in(&base, &work, &["release", "plan", "v1", "FEAT-002"]));
    let started = ok(run_in(&base, &work, &["start", "FEAT-002"]));
    assert!(
        started.contains("fix/FEAT-002-"),
        "a bug gets the fix/ twin: {started}"
    );
    assert!(started.contains("from release/v1"), "{started}");
    commit_file_in(
        &base,
        &work,
        "fix.txt",
        "fixed\n",
        "fix: cart\n\nRefs: kanbanr:FEAT-002",
    );
    let said = ok(run_in(&base, &work, &["move", "FEAT-002", "Completed"]));
    assert!(said.contains("into release/v1"), "{said}");
    assert!(said.contains("kanbanr git forward v1"), "{said}");

    let forwarded = ok(run_in(&base, &work, &["git", "forward", "v1"]));
    assert!(
        forwarded.contains("merged release/v1 forward into integration"),
        "{forwarded}"
    );
    let message = ok(git_in(
        &base,
        &work,
        &["log", "-1", "--format=%B", "integration"],
    ));
    assert!(message.contains("Refs: kanbanr:FEAT-002"), "{message}");
    let again = ok(run_in(&base, &work, &["git", "forward", "v1"]));
    assert!(again.contains("nothing to forward"), "{again}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-196 R-1: deleting a project leaves no folder for it, and nothing for the board to commit
/// afterwards — the delete's own activity entry used to bring the folder back.
#[test]
fn project_delete_leaves_no_folder() {
    let (base, work) = togaf_scratch("delete-project");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(
        &base,
        &work,
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
    let where_json: serde_json::Value =
        serde_json::from_str(&ok(run_in(&base, &work, &["--json", "where"]))).unwrap();
    let board = std::path::PathBuf::from(where_json["data_dir"].as_str().unwrap());
    assert!(board.join("projects").join("shop").is_dir());
    ok(run_in(&base, &work, &["project", "delete", "shop"]));
    assert!(
        !board.join("projects").join("shop").exists(),
        "the project's folder stays gone"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&board)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).trim().is_empty(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-184: cut v1.0.0 through its stages, so there is a tagged release to patch.
fn cut_first_release(base: &std::path::Path, work: &std::path::Path) {
    for args in [
        &["release", "promote", "v1.0.0"][..],
        &["release", "promote", "v1.0.0", "production"],
    ] {
        let o = run_in(base, work, args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

/// FEAT-184 R-1: a patch names a cut release, is branched from its tag — not from where main has
/// since gone — and starts at the first stage; a release that was never cut cannot be patched.
#[test]
fn patch_release_branches_from_parent_tag() {
    let (base, work) = release_scratch("patch-branch");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    let refused = run_in(
        &base,
        &work,
        &["release", "add", "v1.0.1", "--patch-of", "v1.0.0"],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("has not been cut"));

    cut_first_release(&base, &work);
    commit_on(&base, &work, "main", "work for the next release\n");
    let said = ok(run_in(
        &base,
        &work,
        &["release", "add", "v1.0.1", "--patch-of", "v1.0.0"],
    ));
    assert!(said.contains("a patch of v1.0.0"), "{said}");
    assert_eq!(
        rev(&base, &work, "release/v1.0.1"),
        rev(&base, &work, "v1.0.0^{commit}")
    );
    assert_ne!(
        rev(&base, &work, "release/v1.0.1"),
        rev(&base, &work, "main")
    );
    let r = release_json(&base, &work, "v1.0.1");
    assert_eq!(r["parent"], "v1.0.0");
    assert_eq!(r["stage"], "candidate");
    assert_eq!(r["branch"], "release/v1.0.1");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-184 R-5: a patch of a release main has moved past is tagged at production and leaves main
/// where it is; an ordinary release in that state is still refused.
#[test]
fn patch_production_tags_without_moving_main() {
    let (base, work) = release_scratch("patch-production");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    cut_first_release(&base, &work);
    commit_on(&base, &work, "main", "work for the next release\n");
    let main = rev(&base, &work, "main");
    ok(run_in(
        &base,
        &work,
        &["release", "add", "v1.0.1", "--patch-of", "v1.0.0"],
    ));
    commit_on(&base, &work, "release/v1.0.1", "the fix\n");
    let said = ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.1", "production"],
    ));
    assert!(said.contains("main has moved on"), "{said}");
    assert_eq!(
        rev(&base, &work, "v1.0.1^{commit}"),
        rev(&base, &work, "release/v1.0.1")
    );
    assert_eq!(rev(&base, &work, "main"), main, "main is left where it was");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-184 R-3: at production a patch lists its commits the integration branch lacks; with
/// main as the integration branch and main moving to the patch, there is nothing to list.
#[test]
fn patch_release_reports_forward_merge() {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    let (base, work) = release_scratch("patch-forward");
    cut_first_release(&base, &work);
    ok(git_in(&base, &work, &["branch", "develop", "main"]));
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--integration-branch", "develop"],
    ));
    ok(run_in(
        &base,
        &work,
        &["release", "add", "v1.0.1", "--patch-of", "v1.0.0"],
    ));
    commit_on(&base, &work, "release/v1.0.1", "the fix\n");
    let said = ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.1", "production"],
    ));
    assert!(said.contains("merge these forward into develop"), "{said}");
    assert!(said.contains("[no-ref] change"), "{said}");
    let notes = ok(run_in(&base, &work, &["doc", "show", "releases/v1.0.1.md"]));
    assert!(notes.contains("To merge forward into develop"), "{notes}");
    let _ = std::fs::remove_dir_all(&base);

    let (base, work) = release_scratch("patch-forward-main");
    cut_first_release(&base, &work);
    ok(run_in(
        &base,
        &work,
        &["release", "add", "v1.0.1", "--patch-of", "v1.0.0"],
    ));
    commit_on(&base, &work, "release/v1.0.1", "the fix\n");
    let said = ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.1", "production"],
    ));
    assert!(!said.contains("merge these forward"), "{said}");
    assert_eq!(
        rev(&base, &work, "main"),
        rev(&base, &work, "release/v1.0.1")
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-184 R-4: a release with no branch gets one before any promotion — in a process whose one
/// stage is the cut, and when the first stage is skipped.
#[test]
fn promote_without_branch_makes_it_first() {
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    let (base, work) = release_scratch("one-stage");
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--release-stages", "live"],
    ));
    let said = ok(run_in(&base, &work, &["release", "promote", "v1.0.0"]));
    assert!(said.contains("created release/v1.0.0"), "{said}");
    assert!(said.contains("tagged v1.0.0"), "{said}");
    assert_eq!(release_json(&base, &work, "v1.0.0")["state"], "shipped");
    let _ = std::fs::remove_dir_all(&base);

    let (base, work) = release_scratch("skip-first");
    let said = ok(run_in(
        &base,
        &work,
        &["release", "promote", "v1.0.0", "staging"],
    ));
    assert!(said.contains("created release/v1.0.0"), "{said}");
    assert_eq!(
        release_json(&base, &work, "v1.0.0")["branch"],
        "release/v1.0.0"
    );
    assert!(!rev(&base, &work, "release/v1.0.0").is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-191 R-1: an integrated item whose branch never reached its line is an error in `git
/// doctor` and in `doctor` with the git model on; once merged, it is a reminder to delete the
/// branch, not an error. With the model off, `doctor` leaves branches alone.
#[test]
fn git_doctor_unmerged_integrated_items() {
    let (base, work) = togaf_scratch("gd-unmerged");
    git_model_board(&base, &work, false, false);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    // Completed with the model off: nothing merged it.
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--git-model", "off"],
    ));
    ok(run_in(&base, &work, &["start", "FEAT-001"]));
    commit_file_in(
        &base,
        &work,
        "cart.txt",
        "cart\n",
        "feat: cart\n\nRefs: kanbanr:FEAT-001",
    );
    ok(run_in(&base, &work, &["move", "FEAT-001", "Completed"]));
    let doctor_off = ok(run_in(&base, &work, &["doctor"]));
    assert!(!doctor_off.contains("not merged into"), "{doctor_off}");
    ok(run_in(
        &base,
        &work,
        &["config", "cadence", "--git-model", "on"],
    ));

    let found = run_in(&base, &work, &["git", "doctor"]);
    assert_eq!(found.status.code(), Some(1), "an error exits 1");
    let said = String::from_utf8_lossy(&found.stdout);
    assert!(
        said.contains("FEAT-001: FEAT-001 is Completed (integrated) but feat/FEAT-001-cart is not merged into main"),
        "{said}"
    );
    let doctor = ok(run_in(&base, &work, &["doctor"]));
    assert!(doctor.contains("is not merged into main"), "{doctor}");

    ok(git_in(&base, &work, &["checkout", "-q", "main"]));
    ok(git_in(
        &base,
        &work,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--no-verify",
            "feat/FEAT-001-cart",
            "-m",
            "Merge FEAT-001\n\nRefs: kanbanr:FEAT-001",
        ],
    ));
    let clean = ok(run_in(&base, &work, &["git", "doctor"]));
    assert!(
        clean.contains("FEAT-001 is finished and feat/FEAT-001-cart is merged"),
        "{clean}"
    );
    assert!(!clean.contains("Errors"), "{clean}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-191 R-2: commits on a line for work that is not integrated are errors, counted with the
/// newest named; a commit that also serves an integrated item is justified.
#[test]
fn git_doctor_unfinished_commits_on_line() {
    let (base, work) = togaf_scratch("gd-line");
    git_model_board(&base, &work, true, false);
    approved_item(&base, &work, "FEAT-002", "Checkout", None);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(
        &base,
        &work,
        &["sprint", "add", "--start", "2026-10-05"],
    ));
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    // FEAT-001 is integrated (it has no branch, so nothing to merge).
    ok(run_in(&base, &work, &["move", "FEAT-001", "In Progress"]));
    ok(run_in(&base, &work, &["move", "FEAT-001", "Completed"]));
    ok(run_in(&base, &work, &["start", "FEAT-002"]));
    commit_file_in(
        &base,
        &work,
        "pay.txt",
        "pay\n",
        "wip\n\nRefs: kanbanr:FEAT-002",
    );
    // Merged into the sprint by hand while FEAT-002 is still In Progress.
    ok(git_in(&base, &work, &["checkout", "-q", "sprint/SP-001"]));
    ok(git_in(
        &base,
        &work,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--no-verify",
            "feat/FEAT-002-checkout",
            "-m",
            "Merge FEAT-002\n\nRefs: kanbanr:FEAT-002",
        ],
    ));
    commit_file_in(
        &base,
        &work,
        "both.txt",
        "b\n",
        "shared\n\nRefs: kanbanr:FEAT-001, kanbanr:FEAT-002",
    );
    let newest_for_2 = String::from_utf8_lossy(
        &git_in(&base, &work, &["rev-parse", "--short=7", "sprint/SP-001^"]).stdout,
    )
    .trim()
    .to_string();
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));

    let found = run_in(&base, &work, &["git", "doctor"]);
    assert_eq!(found.status.code(), Some(1));
    let said = String::from_utf8_lossy(&found.stdout);
    assert!(
        said.contains(&format!(
            "sprint/SP-001 has 2 commits serving FEAT-002 (newest {newest_for_2}), which is In Progress, not integrated"
        )),
        "{said}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-191 R-3: branches that name nothing on the board, and a release fix the line lacks, are
/// warnings; a forward merge clears the latter; spikes are never reported.
#[test]
fn git_doctor_orphans_and_forward_merges() {
    let (base, work) = togaf_scratch("gd-orphans");
    git_model_board(&base, &work, false, true);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    for b in [
        "wip",
        "feat/FEAT-999-x",
        "sprint/SP-099",
        "release/v9",
        "spike/x",
    ] {
        ok(git_in(&base, &work, &["branch", b, "main"]));
    }
    ok(run_in(&base, &work, &["release", "add", "v1"]));
    ok(run_in(&base, &work, &["release", "promote", "v1"]));
    ok(git_in(&base, &work, &["checkout", "-q", "release/v1"]));
    commit_file_in(
        &base,
        &work,
        "fix.txt",
        "fixed\n",
        "fix: on the release\n\n[no-ref] a test fix",
    );
    ok(git_in(&base, &work, &["checkout", "-q", "main"]));

    let said = ok(run_in(&base, &work, &["git", "doctor"]));
    for want in [
        "wip names no item",
        "feat/FEAT-999-x names FEAT-999, which is not on the board",
        "sprint/SP-099 names no sprint",
        "release/v9 names no release",
        "release/v1 has 1 commit integration lacks",
    ] {
        assert!(said.contains(want), "missing {want:?} in {said}");
    }
    assert!(!said.contains("spike/x"), "{said}");
    ok(run_in(&base, &work, &["git", "forward", "v1"]));
    let after = ok(run_in(&base, &work, &["git", "doctor"]));
    assert!(!after.contains("release/v1 has"), "{after}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-191: `git doctor` only reads — the repository's refs and the board's files are the same
/// after it as before.
#[test]
fn git_doctor_is_read_only() {
    let (base, work) = togaf_scratch("gd-readonly");
    git_model_board(&base, &work, true, true);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(
        &base,
        &work,
        &["sprint", "add", "--start", "2026-10-05"],
    ));
    ok(run_in(&base, &work, &["sprint", "start", "SP-001"]));
    ok(git_in(&base, &work, &["branch", "wip", "main"]));
    let where_json: serde_json::Value =
        serde_json::from_str(&ok(run_in(&base, &work, &["--json", "where"]))).unwrap();
    let board = std::path::PathBuf::from(where_json["data_dir"].as_str().unwrap());
    let refs = || ok(git_in(&base, &work, &["for-each-ref"]));
    let (refs_before, board_before) = (refs(), snapshot(&board));
    let _ = run_in(&base, &work, &["git", "doctor"]);
    let _ = run_in(&base, &work, &["doctor"]);
    assert_eq!(refs(), refs_before, "no ref moved");
    assert_eq!(snapshot(&board), board_before, "no board file changed");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-201 R-3: a new board's README is committed with its first project and follows its
/// projects; a board without one is told how to get it, and `kanbanr readme` writes it.
#[test]
fn new_board_has_a_readme() {
    let (base, work) = togaf_scratch("board-readme");
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    };
    ok(run_in(
        &base,
        &work,
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
    let where_json: serde_json::Value =
        serde_json::from_str(&ok(run_in(&base, &work, &["--json", "where"]))).unwrap();
    let board = std::path::PathBuf::from(where_json["data_dir"].as_str().unwrap());
    let board_git = |args: &[&str]| -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .args(args)
                .current_dir(&board)
                .output()
                .unwrap()
                .stdout,
        )
        .to_string()
    };
    assert_eq!(
        board_git(&["ls-files", "README.md"]).trim(),
        "README.md",
        "committed"
    );
    assert!(board_git(&["status", "--porcelain"]).trim().is_empty());
    let readme = std::fs::read_to_string(board.join("README.md")).unwrap();
    assert!(
        readme.contains("`shop`") && readme.contains("kanbanr serve --data-dir"),
        "{readme}"
    );

    ok(run_in(&base, &work, &["project", "init", "ops"]));
    let readme = std::fs::read_to_string(board.join("README.md")).unwrap();
    assert!(
        readme.contains("`ops`"),
        "a new project joins the list:\n{readme}"
    );

    // A board from before: no README. Doctor says how to get one, and `kanbanr readme` writes it.
    std::fs::remove_file(board.join("README.md")).unwrap();
    board_git(&["commit", "-qam", "an older board"]);
    let doctor = ok(run_in(&base, &work, &["doctor"]));
    assert!(doctor.contains("`kanbanr readme` writes one"), "{doctor}");
    let said = ok(run_in(&base, &work, &["readme"]));
    assert!(said.contains("wrote the board README"), "{said}");
    assert_eq!(board_git(&["ls-files", "README.md"]).trim(), "README.md");
    let doctor = ok(run_in(&base, &work, &["doctor"]));
    assert!(!doctor.contains("kanbanr readme"), "{doctor}");
    let again = ok(run_in(&base, &work, &["readme"]));
    assert!(again.contains("up to date"), "{again}");
    let _ = std::fs::remove_dir_all(&base);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// FEAT-187: a board kanbanr 0.1.7 made is refused by every command with the remedy, until
/// `kanbanr migrate` brings it up to date in one commit; then it reads as 0.1.7 read it, and a
/// second run has nothing to do.
#[test]
fn cli_migrate_brings_a_v017_board_up_to_date() {
    let (base, work) = togaf_scratch("migrate");
    let board = base.join("board");
    copy_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/v017/board"),
        &board,
    );
    std::fs::rename(board.join("dot-git"), board.join(".git")).unwrap();
    // L-41: the board is this test's own copy, in the temp dir, before the first write.
    assert!(board.starts_with(std::env::temp_dir()));
    let dir = board.to_str().unwrap();
    let run = |args: &[&str]| {
        let mut all = vec!["--data-dir", dir, "--project", "demo"];
        all.extend_from_slice(args);
        run_in(&base, &work, &all)
    };
    let text = |o: &Output| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };

    let refused = run(&["board"]);
    assert!(!refused.status.success());
    assert!(
        text(&refused).contains("run `kanbanr migrate`"),
        "{}",
        text(&refused)
    );

    let migrated = run(&["migrate"]);
    assert!(migrated.status.success(), "{}", text(&migrated));
    let out = text(&migrated);
    assert!(out.contains("migrated 4 project(s) to schema 5"), "{out}");
    assert!(out.contains("committed"), "{out}");
    assert!(!out.contains("could not be verified"), "{out}");

    // What 0.1.7 had awaiting review (`review --pending --json` on the fixture), and nothing else.
    let review = run(&["--json", "review", "--pending"]);
    assert!(review.status.success(), "{}", text(&review));
    let pending: serde_json::Value = serde_json::from_slice(&review.stdout).unwrap();
    let mut got: Vec<String> = pending
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{} {}", e["code"].as_str().unwrap(), e["approval"]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            "FEAT-002 \"lapsed\"",
            "FEAT-003 \"missing\"",
            "FEAT-005 \"lapsed\"",
            "FEAT-009 \"missing\""
        ]
    );

    let again = run(&["migrate"]);
    assert!(again.status.success(), "{}", text(&again));
    assert!(
        text(&again).contains("nothing to migrate"),
        "{}",
        text(&again)
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-202 R-1: the build scripts find the monitor and the skill from where the crate is when the
/// script runs. `env!` would fix the folder the script was compiled in, and a build cache reused one
/// compiled in a deleted worktree: the installed binary had no monitor and no skill.
#[test]
fn build_scripts_read_the_manifest_dir_at_run_time() {
    let compiled_in = concat!("env!(\"CARGO_MANIFEST", "_DIR\")");
    for (name, source) in [
        ("kanbanr-cli/build.rs", include_str!("../build.rs")),
        (
            "kanbanr-server/build.rs",
            include_str!("../../kanbanr-server/build.rs"),
        ),
    ] {
        assert!(
            !source.contains(compiled_in),
            "{name} fixes the folder at compile time"
        );
        assert!(
            source.contains("std::env::var(\"CARGO_MANIFEST_DIR\")"),
            "{name} reads the folder when it runs"
        );
    }
}

/// A scratch code repository with a default-process board `shop` and one milestone `MS-1`
/// (FEAT-200's tests). Returns (base, work); both live under the temp dir.
fn diagram_scratch(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let (base, work) = togaf_scratch(tag);
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(git_in(&base, &work, &["init", "-q", "-b", "main"]));
    ok(run_in(
        &base,
        &work,
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
    ok(run_in(
        &base,
        &work,
        &["milestone", "add", "--name", "M", "--code", "MS-1"],
    ));
    (base, work)
}

/// FEAT-200 R-17: `config diagrams` shows the setting, `--strict on|off` sets it, and off leaves
/// the key absent from config.yaml, as it was before anyone chose.
#[test]
fn cli_config_diagrams_strict_toggles() {
    let (base, work) = diagram_scratch("diagrams-config");
    let ok = |args: &[&str]| {
        let o = run_in(&base, &work, args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    let board = std::path::PathBuf::from(ok(&["where"]).trim());
    let config = || std::fs::read_to_string(board.join("projects/shop/config.yaml")).unwrap();
    assert!(!config().contains("diagrams"), "absent until chosen");
    assert_eq!(ok(&["config", "diagrams"]).trim(), "strict: off");

    assert!(ok(&["config", "diagrams", "--strict", "on"]).contains("diagrams strict: on"));
    assert!(config().contains("strict: true"), "{}", config());
    assert_eq!(ok(&["config", "diagrams"]).trim(), "strict: on");
    let json: serde_json::Value =
        serde_json::from_str(&ok(&["--json", "config", "diagrams"])).unwrap();
    assert_eq!(json["strict"], true);

    assert!(ok(&["config", "diagrams", "--strict", "off"]).contains("diagrams strict: off"));
    assert!(!config().contains("diagrams"), "off is absent again");
    assert_eq!(ok(&["config", "diagrams"]).trim(), "strict: off");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-200 R-12: `process check` accepts `diagrams_valid` in a gate's `requires` and `warns`, a
/// misspelling is refused naming it, and `process checks` lists it with what it means.
#[test]
fn cli_process_check_accepts_diagrams_valid() {
    let (base, _) = togaf_scratch("diagrams-process");
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let file = base.join("mine.yaml");
    let workflow = "statuses: [Planned, Doing, Done]\ndefault_state: Planned\n\
                    displayed_states: [Planned, Doing, Done]\nterminal_states: [Done]\n\
                    transitions: {Planned: [Doing], Doing: [Done]}\n\
                    gates:\n  Doing: {requires: [diagrams_valid]}\n  Done: {warns: [diagrams_valid]}\n";
    std::fs::write(&file, workflow).unwrap();
    let out = run_in(&base, &empty, &["process", "check", file.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("no problems"), "{text}");

    std::fs::write(
        &file,
        workflow.replace("[diagrams_valid]}\n  Done", "[diagrams_vaild]}\n  Done"),
    )
    .unwrap();
    let out = run_in(&base, &empty, &["process", "check", file.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        err.contains("'diagrams_vaild'") && err.contains("diagrams_valid"),
        "{err}"
    );

    let out = run_in(&base, &empty, &["process", "checks"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        text.lines()
            .any(|l| l.contains("diagrams_valid") && l.contains("diagram lint")),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-200 R-2: strict off, `feature add` writes the item and prints each diagram problem on
/// stderr; `--json` carries them as `diagram_warnings` instead.
#[test]
fn cli_feature_add_prints_diagram_warning() {
    let (base, work) = diagram_scratch("diagrams-warn");
    let spec = "# Spec\n\n```mermaid\nsequenceDiagram\n  A->>B: one; two\n```\n";
    let add = [
        "feature",
        "add",
        "--milestone",
        "MS-1",
        "--spec",
        spec,
        "--title",
    ];
    let out = run_in(&base, &work, &[&add[..], &["Broken"]].concat());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("created FEAT-001"),
        "the write succeeds"
    );
    assert!(
        stderr.contains("diagram: FEAT-001 spec, diagram #1, line 5:")
            && stderr.contains("(semicolon_in_text)"),
        "{stderr}"
    );

    let out = run_in(&base, &work, &[&["--json"], &add[..], &["Again"]].concat());
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("diagram:"));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["diagram_warnings"][0]["rule"], "semicolon_in_text");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-197: run a command in a coverage scratch board, returning (success, stdout, stderr).
fn coverage_run(base: &Path, work: &Path, args: &[&str]) -> (bool, String, String) {
    let o = run_in(base, work, args);
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

/// FEAT-197: as [`coverage_run`], failing the test when the command fails.
fn coverage_ok(base: &Path, work: &Path, args: &[&str]) -> (String, String) {
    let (ok, out, err) = coverage_run(base, work, args);
    assert!(ok, "{args:?}: {err}");
    (out, err)
}

/// FEAT-197 R-4: with no coverage rules, writes carry no `coverage_warnings`, print no coverage
/// line, and doctor says nothing about coverage; `config coverage` says there are none.
#[test]
fn coverage_silent_without_rules() {
    let (base, work) = diagram_scratch("coverage-none");
    let ok = |args: &[&str]| coverage_ok(&base, &work, args);
    ok(&[
        "doc",
        "add",
        "architecture/zachman.md",
        "--content",
        "# Zachman\n",
    ]);
    let (out, _) = ok(&[
        "--json",
        "feature",
        "add",
        "--milestone",
        "MS-1",
        "--title",
        "One",
    ]);
    assert!(!out.contains("coverage_warnings"), "{out}");
    for args in [
        &["feature", "edit", "FEAT-001", "--title", "Uno"][..],
        &["move", "FEAT-001", "Deferred"],
        &[
            "doc",
            "add",
            "architecture/catalog.md",
            "--content",
            "# Catalog\n",
        ],
    ] {
        let (_, err) = ok(args);
        assert!(!err.contains("coverage:"), "{args:?}: {err}");
    }
    let (out, _) = ok(&["--json", "move", "FEAT-001", "Planned"]);
    assert!(!out.contains("coverage_warnings"), "{out}");
    let (doctor, _) = ok(&["doctor"]);
    assert!(!doctor.contains("coverage"), "{doctor}");
    let (shown, _) = ok(&["config", "coverage"]);
    assert!(shown.contains("no coverage rules"), "{shown}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-197 R-5: with a rule set from a file, adding, editing, defining and moving an item each
/// warn of the rule it fails — on stderr, and as `coverage_warnings` under `--json` — until the
/// view doc links it.
#[test]
fn coverage_warns_on_item_events() {
    let (base, work) = diagram_scratch("coverage-items");
    let ok = |args: &[&str]| coverage_ok(&base, &work, args);
    ok(&[
        "doc",
        "add",
        "architecture/zachman.md",
        "--content",
        "# Zachman\n",
    ]);
    let rules = base.join("rules.yaml");
    std::fs::write(
        &rules,
        "coverage:\n  - id: zachman\n    description: the matrix lists every item\n    \
         referenced_from: architecture/zachman.md\n",
    )
    .unwrap();
    let (set, _) = ok(&["config", "coverage", "--from-file", rules.to_str().unwrap()]);
    assert!(set.contains("zachman"), "{set}");
    let warning = "coverage: [zachman] FEAT-001 is not referenced from architecture/zachman.md \
                   — add a link to it there";

    let (out, err) = ok(&["feature", "add", "--milestone", "MS-1", "--title", "One"]);
    assert!(out.contains("created FEAT-001"), "the write is kept: {out}");
    assert!(err.contains(warning), "add: {err}");
    let (_, err) = ok(&["feature", "edit", "FEAT-001", "--title", "Uno"]);
    assert!(err.contains(warning), "edit: {err}");
    let def = base.join("def.yaml");
    std::fs::write(&def, "statement: One thing for someone so that something\n").unwrap();
    let (_, err) = ok(&[
        "feature",
        "define",
        "FEAT-001",
        "--file",
        def.to_str().unwrap(),
    ]);
    assert!(err.contains(warning), "define: {err}");
    let (_, err) = ok(&["move", "FEAT-001", "Deferred"]);
    assert!(err.contains(warning), "move: {err}");
    let (json, err) = ok(&["--json", "move", "FEAT-001", "Planned"]);
    assert!(
        !err.contains("coverage:"),
        "--json prints nothing extra: {err}"
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["coverage_warnings"][0]["rule"], "zachman");
    assert_eq!(v["coverage_warnings"][0]["subject"]["id"], "FEAT-001");
    let (shown, _) = ok(&["config", "coverage"]);
    assert!(
        shown.contains("zachman") && shown.contains("1/1 uncovered  architecture/zachman.md"),
        "{shown}"
    );

    // Linked from the view, the item is covered.
    ok(&[
        "doc",
        "add",
        "architecture/zachman.md",
        "--content",
        "# Zachman\n\n| [FEAT-001](/p/shop/feature/FEAT-001) |\n",
    ]);
    let (_, err) = ok(&["feature", "edit", "FEAT-001", "--title", "One"]);
    assert!(!err.contains("coverage:"), "{err}");
    let (shown, _) = ok(&["config", "coverage"]);
    assert!(shown.contains("0/1 uncovered"), "{shown}");
    ok(&["config", "coverage", "--clear"]);
    let (shown, _) = ok(&["config", "coverage"]);
    assert!(shown.contains("no coverage rules"), "{shown}");
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-197 R-6: accepting an ADR warns when no architecture view indexes it; writing a doc under
/// a catalogued folder warns until the catalog links it; deleting the catalog warns that its rule
/// has no source.
#[test]
fn coverage_warns_on_adr_accept_and_doc_write() {
    let (base, work) = diagram_scratch("coverage-adr-doc");
    let ok = |args: &[&str]| coverage_ok(&base, &work, args);
    ok(&[
        "doc",
        "add",
        "architecture/index.md",
        "--content",
        "# Index\n",
    ]);
    ok(&[
        "doc",
        "add",
        "architecture/catalog.md",
        "--content",
        "# Catalog\n",
    ]);
    let rules = base.join("rules.yaml");
    std::fs::write(
        &rules,
        "- id: adr-index\n  every: adr\n  referenced_from: architecture/\n\
         - id: catalog\n  every: doc\n  under: architecture/components/\n  \
         referenced_from: architecture/catalog.md\n",
    )
    .unwrap();
    ok(&["config", "coverage", "--from-file", rules.to_str().unwrap()]);

    let adr = "---\nid: ADR-0001\nstatus: proposed\n---\n\n# Cache carts\n\n## Context\n\nSlow.\n\n\
               ## Decision\n\nCache them.\n\n## Alternatives considered\n\nNone.\n\n\
               ## Consequences\n\nStale reads.\n\n## Compliance\n\nA test.\n";
    // `--content=` because the text starts with `---`, which would read as a flag.
    let content = format!("--content={adr}");
    ok(&["doc", "add", "decisions/0001-cache-carts.md", &content]);
    let (out, err) = ok(&["adr", "accept", "ADR-0001"]);
    assert!(out.contains("ADR-0001 accepted"), "{out}");
    assert!(
        err.contains(
            "coverage: [adr-index] ADR-0001 is not referenced from architecture/ — add a link to \
             it there"
        ),
        "accept: {err}"
    );
    let (_, err) = ok(&[
        "doc",
        "add",
        "architecture/index.md",
        "--content",
        "# Index\n\n- [ADR-0001](../decisions/0001-cache-carts.md)\n",
    ]);
    assert!(
        !err.contains("adr-index"),
        "indexed by a relative link: {err}"
    );

    let (_, err) = ok(&[
        "doc",
        "add",
        "architecture/components/store.md",
        "--content",
        "# Store\n",
    ]);
    assert!(
        err.contains(
            "coverage: [catalog] architecture/components/store.md is not referenced from \
             architecture/catalog.md — add a link to it there"
        ),
        "doc write: {err}"
    );
    let (_, err) = ok(&[
        "doc",
        "add",
        "architecture/catalog.md",
        "--content",
        "# Catalog\n\n- [Store](components/store.md)\n",
    ]);
    assert!(!err.contains("coverage:"), "{err}");
    let (_, err) = ok(&["doc", "rm", "architecture/catalog.md"]);
    assert!(
        err.contains("coverage: [catalog] architecture/catalog.md is not on the board"),
        "doc delete: {err}"
    );
    let (doctor, _) = ok(&["doctor"]);
    assert!(
        doctor.contains("coverage rule 'catalog': architecture/catalog.md is not on the board"),
        "{doctor}"
    );
    let _ = std::fs::remove_dir_all(&base);
}
