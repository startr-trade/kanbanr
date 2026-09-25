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
    for name in ["session-start", "stop-check"] {
        std::fs::write(scripts.join(format!("{name}.sh")), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(scripts.join(format!("{name}.ps1")), "exit 0\n").unwrap();
    }
    let settings = home.join(".claude/settings.json");
    std::fs::write(&settings, r#"{"theme": "dark"}"#).unwrap();

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
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(v["theme"], "dark");
    assert!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("session-start")
    );
    assert!(
        v["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("stop-check")
    );

    // A second project: already installed, nothing duplicated.
    let before = std::fs::read_to_string(&settings).unwrap();
    let out = run(
        &base.join("code/web"),
        &["init", "web", "--author", "A", "--email", "a@x"],
    );
    assert!(out.contains("hooks already installed"), "{out}");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);
    assert!(run(&base.join("code/web"), &["hooks", "status"]).contains("SessionStart: ✓"));

    // --no-hooks leaves the settings alone even when hooks are missing.
    run(&base.join("code/web"), &["hooks", "uninstall"]);
    let before = std::fs::read_to_string(&settings).unwrap();
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
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);

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
        after.contains("cycle time (days):"),
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
