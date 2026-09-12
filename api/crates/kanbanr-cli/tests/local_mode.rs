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
            .env("HOME", &home) // isolate ~/.kanbanr -> no login profile
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
    assert!(f["specification"]
        .as_str()
        .unwrap()
        .contains("## Imported from"));

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
