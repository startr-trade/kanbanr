//! The view daemon (`kanbanr serve`) serves a local data folder read-only over HTTP, with no auth.
//! We populate a data dir with the CLI (local writes), run the daemon over it, and hit the reads.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn view_daemon_serves_local_data_without_auth() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("view-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let run = |args: &[&str]| {
        let o = Command::new(cli())
            .args(args)
            .env("HOME", &home)
            .env("KANBANR_DATA_DIR", &data)
            .env_remove("KANBANR_SERVER_URL")
            .output()
            .expect("run cli");
        assert!(
            o.status.success(),
            "cmd {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };

    // Populate the folder locally (the CLI is the writer).
    run(&["identity", "--name", "Tester", "--email", "t@example.com"]);
    run(&[
        "--project",
        "demo",
        "project",
        "init",
        "demo",
        "--description",
        "views",
    ]);
    run(&[
        "--project",
        "demo",
        "milestone",
        "add",
        "--name",
        "M",
        "--code",
        "MS-1",
    ]);
    run(&[
        "--project",
        "demo",
        "feature",
        "add",
        "--title",
        "Login",
        "--milestone",
        "MS-1",
    ]);

    // Run the daemon over the same folder (same single binary, `serve` mode).
    let port = free_port();
    let child = Command::new(cli())
        .args(["serve", "--bind", &format!("127.0.0.1:{port}")])
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .spawn()
        .expect("spawn kanbanr serve");
    let _daemon = Daemon(child);
    let url = format!("http://127.0.0.1:{port}");

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            panic!("view daemon did not become ready");
        }
        match ureq::get(&format!("{url}/api/projects")).call() {
            Ok(_) | Err(ureq::Error::Status(_, _)) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    // No auth: reads just work.
    let projects = ureq::get(&format!("{url}/api/projects"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(projects.contains("demo"), "projects: {projects}");
    let project = ureq::get(&format!("{url}/api/projects/demo"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(project.contains("Login"), "project: {project}");

    // The activity changelog was populated by the local writes and is served.
    let activity = ureq::get(&format!("{url}/api/projects/demo/activity"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        activity.contains("Tester"),
        "activity should record the actor: {activity}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// Reviewing and approving through the daemon (FEAT-067). The approval gate's premise is that
/// agreeing must be cheap, so the monitor needs the same action the CLI has — through the same
/// route, with the same meaning, and only when the daemon was told to accept writes.
/// Reviewing and approving through the daemon (FEAT-067).
#[test]
fn review_and_approve_through_the_daemon() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("review-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let run = |args: &[&str]| {
        let o = Command::new(cli())
            .args(args)
            .env("HOME", &home)
            .env("KANBANR_DATA_DIR", &data)
            .env_remove("KANBANR_SERVER_URL")
            .output()
            .expect("run cli");
        assert!(
            o.status.success(),
            "cmd {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    let p = ["--project", "demo"];

    run(&["identity", "--name", "Tester", "--email", "t@example.com"]);
    run(&[&p[..], &["project", "init", "demo"][..]].concat());
    run(&[
        &p[..],
        &["milestone", "add", "--name", "M", "--code", "MS-1"][..],
    ]
    .concat());
    run(&[
        &p[..],
        &["feature", "add", "--title", "Cart", "--milestone", "MS-1"][..],
    ]
    .concat());
    let def = base.join("def.yaml");
    std::fs::write(
        &def,
        "statement: Keep a cart for 7 days\n\
         requirements:\n  - kind: functional\n    text: \"THE SYSTEM SHALL retain the cart.\"\n\
         \x20   tests:\n      - name: cart::retains\n        kind: unit\n        state: planned\n",
    )
    .unwrap();
    run(&[
        &p[..],
        &[
            "feature",
            "define",
            "FEAT-001",
            "--file",
            def.to_str().unwrap(),
        ][..],
    ]
    .concat());

    let start = |allow_writes: bool| -> (Daemon, String) {
        let port = free_port();
        let mut args = vec![
            "serve".to_string(),
            "--bind".to_string(),
            format!("127.0.0.1:{port}"),
        ];
        if allow_writes {
            args.push("--allow-writes".to_string());
        }
        let child = Command::new(cli())
            .args(&args)
            .env("HOME", &home)
            .env("KANBANR_DATA_DIR", &data)
            .spawn()
            .expect("spawn kanbanr serve");
        let daemon = Daemon(child);
        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if Instant::now() > deadline {
                panic!("daemon did not become ready");
            }
            match ureq::get(&format!("{url}/api/projects")).call() {
                Ok(_) | Err(ureq::Error::Status(_, _)) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        (daemon, url)
    };
    let get = |url: &str| -> String { ureq::get(url).call().unwrap().into_string().unwrap() };

    // A read-only monitor says so, and exposes no write route at all.
    {
        let (_daemon, url) = start(false);
        assert!(
            get(&format!("{url}/api/meta")).contains("\"writes\":false"),
            "a read-only daemon must say it cannot write"
        );
        let pending = get(&format!("{url}/api/projects/demo/review"));
        assert!(
            pending.contains("FEAT-001"),
            "the queue is readable either way: {pending}"
        );
        assert!(pending.contains("\"approval\":\"missing\""), "{pending}");

        // The route is not there, so the request lands on the read-only fallback. Which status it
        // gets (404 for an unknown path, 405 for a non-GET on the fallback) matters less than the
        // thing being asserted: nothing was written.
        let refused = ureq::post(&format!(
            "{url}/api/write/projects/demo/features/FEAT-001/approve"
        ))
        .set("content-type", "application/json")
        .send_string(r#"{"by": "someone"}"#);
        match refused {
            Err(ureq::Error::Status(404 | 405, _)) => {}
            other => panic!("a monitor without --allow-writes must refuse a write: {other:?}"),
        }
        let after = get(&format!("{url}/api/projects/demo/review"));
        assert!(
            after.contains("\"approval\":\"missing\""),
            "the refusal left the board untouched: {after}"
        );
    }

    // With writes enabled, approving through the daemon is the same approval the CLI records.
    let (_daemon, url) = start(true);
    assert!(get(&format!("{url}/api/meta")).contains("\"writes\":true"));
    ureq::post(&format!(
        "{url}/api/write/projects/demo/features/FEAT-001/approve"
    ))
    .set("content-type", "application/json")
    .send_string(r#"{"by": "reviewed in the monitor"}"#)
    .expect("approve through the daemon");

    // FEAT-059 applies to this path too: the event is in the commit, and the board repo is clean
    // after the write. The daemon used to leave the log for some later write to sweep up.
    let board_git = |args: &[&str]| -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&data)
            .output()
            .expect("run git");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    assert_eq!(
        board_git(&["status", "--porcelain"]).trim(),
        "",
        "the daemon leaves the board repo clean, like the CLI does"
    );
    // An approval produces no event — only adds and moves do — so the ordering is exercised with a
    // move, which is the operation that writes the log.
    ureq::post(&format!(
        "{url}/api/write/projects/demo/features/FEAT-001/move"
    ))
    .set("content-type", "application/json")
    .send_string(r#"{"to": "Scheduled"}"#)
    .expect("move through the daemon");
    assert_eq!(
        board_git(&["status", "--porcelain"]).trim(),
        "",
        "still clean after a write that does produce an event"
    );
    assert!(
        board_git(&["show", "--name-only", "--format=", "HEAD"]).contains("projects/demo/events/"),
        "the event is part of the commit the daemon made, not left for a later one"
    );

    // Gone from the queue, and the CLI agrees it is approved.
    assert_eq!(get(&format!("{url}/api/projects/demo/review")).trim(), "[]");
    let check = Command::new(cli())
        .args([&p[..], &["check", "FEAT-001"][..]].concat())
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .output()
        .expect("run check");
    let shown = String::from_utf8_lossy(&check.stdout);
    assert!(!shown.contains("not approved"), "{shown}");
    // And the recorded approval names who gave it.
    let feature = get(&format!("{url}/api/projects/demo"));
    assert!(feature.contains("reviewed in the monitor"), "{feature}");

    let _ = std::fs::remove_dir_all(&base);
}

/// A rebuilt UI must be visible to a running monitor (FEAT-070). The daemon used to read
/// `index.html` once at startup, so after a rebuild it served an index pointing at a bundle that
/// had been deleted — which looks like a broken feature, not a stale process. That is exactly how
/// it was reported: "the diagrams are not rendering".
#[test]
fn a_rebuilt_ui_is_served_without_restarting_the_daemon() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let data = base.join("data");
    let home = base.join("home");
    let ui = base.join("dist");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    let index = ui.join("index.html");
    std::fs::write(
        &index,
        "<html><script src=/assets/first.js></script></html>",
    )
    .unwrap();

    let out = Command::new(cli())
        .args(["identity", "--name", "T", "--email", "t@x"])
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .output()
        .expect("identity");
    assert!(out.status.success());

    let port = free_port();
    let child = Command::new(cli())
        .args([
            "serve",
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--ui-dir",
            ui.to_str().unwrap(),
        ])
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .spawn()
        .expect("spawn kanbanr serve");
    let _daemon = Daemon(child);
    let url = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            panic!("daemon did not become ready");
        }
        match ureq::get(&format!("{url}/api/projects")).call() {
            Ok(_) | Err(ureq::Error::Status(_, _)) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let page = |url: &str| ureq::get(url).call().unwrap().into_string().unwrap();
    assert!(page(&format!("{url}/p/demo")).contains("first.js"));

    // Rebuild under the running daemon: a new hashed bundle, as vite would write.
    std::fs::write(
        &index,
        "<html><script src=/assets/second.js></script></html>",
    )
    .unwrap();
    let served = page(&format!("{url}/p/demo"));
    assert!(
        served.contains("second.js"),
        "a rebuilt index must be served, not the one read at startup: {served}"
    );

    // Mid-rebuild, with the index momentarily gone, the last good copy is served rather than a
    // blank page — an empty document is a worse answer than a slightly old one.
    std::fs::remove_file(&index).unwrap();
    let during = page(&format!("{url}/p/demo"));
    assert!(
        during.contains("second.js"),
        "expected the last good copy: {during}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-072: the daemon reports the schema it understands, so a monitor that receives an empty board
/// can compare it with the board's own version and say "upgrade" — rather than leaving the reader to
/// conclude their data is gone, which is what happened.
#[test]
fn daemon_reports_the_schema_version_it_understands() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let port = free_port();
    let child = Command::new(cli())
        .args(["serve", "--bind", &format!("127.0.0.1:{port}")])
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .spawn()
        .expect("spawn kanbanr serve");
    let _daemon = Daemon(child);
    let url = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            panic!("daemon did not become ready");
        }
        match ureq::get(&format!("{url}/api/meta")).call() {
            Ok(_) | Err(ureq::Error::Status(_, _)) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    let meta = ureq::get(&format!("{url}/api/meta"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&meta).unwrap();
    let reported = parsed["schema_version"].as_u64().expect("a schema version");
    assert!(reported >= 2, "the layout changes bumped it past 1: {meta}");
    // It must be what this build actually understands, not a literal that can drift from it.
    assert_eq!(
        reported,
        u64::from(kanbanr_core::config::CURRENT_SCHEMA_VERSION),
        "the daemon reports the constant, so the two cannot disagree"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-084 R-1/R-2: the monitor is in the binary, and an explicit `--ui-dir` still wins.
///
/// The constraint says one binary installable without a toolchain, and it was only ever true of the
/// CLI: a downloaded release binary served an API with no web view, because `web/dist` had to exist
/// beside it. These two assertions are what "built in" means — the SPA comes back with no
/// `--ui-dir` at all, and pointing at a directory still overrides it so developing is unchanged.
#[test]
fn the_monitor_is_served_from_the_binary_with_no_ui_dir() {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("embed-{}", std::process::id()));
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    Command::new(cli())
        .args(["--project", "demo", "project", "init", "demo"])
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &data)
        .output()
        .expect("init");

    let serve = |ui: Option<&std::path::Path>| {
        let port = free_port();
        let mut cmd = Command::new(cli());
        cmd.args(["serve", "--bind", &format!("127.0.0.1:{port}")]);
        if let Some(dir) = ui {
            cmd.args(["--ui-dir", dir.to_str().unwrap()]);
        }
        let child = cmd
            .env("HOME", &home)
            .env("KANBANR_DATA_DIR", &data)
            .spawn()
            .expect("spawn serve");
        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if Instant::now() > deadline {
                panic!("daemon did not become ready");
            }
            match ureq::get(&format!("{url}/api/projects")).call() {
                Ok(_) | Err(ureq::Error::Status(_, _)) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        (Daemon(child), url)
    };

    // R-1: no --ui-dir anywhere, and the monitor is still served.
    let (_d, url) = serve(None);
    let index = ureq::get(&url).call().unwrap().into_string().unwrap();
    assert!(
        index.contains("<!doctype html") || index.contains("<!DOCTYPE html"),
        "expected the SPA's index, got: {}",
        &index[..index.len().min(200)]
    );
    assert!(
        index.contains("/assets/"),
        "the index must reference its hashed bundle: {index}"
    );
    // The bundle it names is served too — an index alone is a blank page.
    let asset = index
        .split("/assets/")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a hashed asset in the index");
    let resp = ureq::get(&format!("{url}/assets/{asset}")).call().unwrap();
    assert_eq!(resp.status(), 200, "the hashed bundle must be served");
    assert!(
        resp.header("content-type")
            .unwrap_or("")
            .contains("javascript"),
        "served as the wrong type: {:?}",
        resp.header("content-type")
    );
    // A client-side route is the app's to interpret, so it gets the index rather than a 404.
    let route = ureq::get(&format!("{url}/p/demo/review"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(route.contains("/assets/"), "a deep link must load the app");
    drop(_d);

    // R-2: an explicit --ui-dir overrides the embedded copy, so a live build still wins.
    let dir = base.join("custom-ui");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html>OVERRIDDEN").unwrap();
    let (_d2, url2) = serve(Some(&dir));
    let served = ureq::get(&url2).call().unwrap().into_string().unwrap();
    assert!(
        served.contains("OVERRIDDEN"),
        "--ui-dir must win over the embedded monitor, got: {served}"
    );
}
