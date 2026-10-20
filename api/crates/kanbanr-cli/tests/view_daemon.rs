//! The view daemon (`kanbanr serve`) serves a local data folder read-only over HTTP, with no auth.
//! We populate a data dir with the CLI (local writes), run the daemon over it, and hit the reads.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

/// A port no other test in this binary will be handed (FEAT-165). The tests here run in parallel,
/// each starting a daemon; asking the OS for port 0 and letting go of it let two tests be given the
/// same port. The second daemon then failed to bind, and its test talked to the first test's daemon
/// (one serving a different UI) and saw a 404 for `/`. Ports now come from a per-process counter,
/// starting at a base derived from the process id, skipping any already taken on the machine.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let _ = NEXT.compare_exchange(
        0,
        20_000 + (std::process::id() % 20_000) as u16,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    loop {
        let port = NEXT.fetch_add(1, Ordering::SeqCst);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
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
    let base = std::env::temp_dir().join(format!("kanbanr-view-{}", std::process::id()));
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
    let base = std::env::temp_dir().join(format!("kanbanr-review-{}", std::process::id()));
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
    // Approval is the subject here; the method's sign-offs would join the queue (FEAT-193).
    run(&[&p[..], &["config", "method", "--off"][..]].concat());
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
    .send_string(r#"{"to": "In Progress"}"#)
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

    // FEAT-182: a sign-off withdrawn through the daemon, as the monitor's button does it — with a
    // reason, or not at all.
    ureq::post(&format!(
        "{url}/api/write/projects/demo/features/FEAT-001/signoff/design-review"
    ))
    .set("content-type", "application/json")
    .send_string(r#"{"by": "reviewed in the monitor"}"#)
    .expect("sign off through the daemon");
    let withdraw =
        format!("{url}/api/write/projects/demo/features/FEAT-001/signoff/design-review/withdraw");
    match ureq::post(&withdraw)
        .set("content-type", "application/json")
        .send_string(r#"{"by": "reviewed in the monitor", "reason": " "}"#)
    {
        Err(ureq::Error::Status(400, _)) => {}
        other => panic!("a withdrawal without a reason must be refused: {other:?}"),
    }
    ureq::post(&withdraw)
        .set("content-type", "application/json")
        .send_string(r#"{"by": "reviewed in the monitor", "reason": "signed before the review"}"#)
        .expect("withdraw through the daemon");
    let feature = get(&format!("{url}/api/projects/demo"));
    assert!(
        feature.contains("\"verdict\":\"withdrawn\"")
            && feature.contains("signed before the review"),
        "{feature}"
    );
    assert!(
        board_git(&["log", "-1", "--format=%s"]).contains("withdraw sign-off design-review"),
        "the commit says what happened"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// A rebuilt UI must be visible to a running monitor (FEAT-070). The daemon used to read
/// `index.html` once at startup, so after a rebuild it served an index pointing at a bundle that
/// had been deleted — which looks like a broken feature, not a stale process. That is exactly how
/// it was reported: "the diagrams are not rendering".
#[test]
fn a_rebuilt_ui_is_served_without_restarting_the_daemon() {
    let base = std::env::temp_dir().join(format!("kanbanr-ui-{}", std::process::id()));
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
    let base = std::env::temp_dir().join(format!("kanbanr-schema-{}", std::process::id()));
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
    let base = std::env::temp_dir().join(format!("kanbanr-embed-{}", std::process::id()));
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
        let mut child = cmd
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
        // Whoever answered must be this test's daemon, not another one on the same port (FEAT-165).
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(Some(status)) = child.try_wait() {
            panic!("this test's daemon exited ({status}) — something else answered on port {port}");
        }
        (Daemon(child), url)
    };

    // R-1: no --ui-dir anywhere, and the monitor is still served.
    let (_d, url) = serve(None);
    let index = match ureq::get(&url).call() {
        Ok(resp) => resp.into_string().unwrap(),
        Err(ureq::Error::Status(404, _)) => panic!(
            "no monitor at / — this binary carries none: web/dist was absent when kanbanr-server \
             was compiled (build the monitor first: cd web && npm run build)"
        ),
        Err(e) => panic!("GET / failed: {e}"),
    };
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

/// A scratch folder under the system temp dir, emptied first (FEAT-178's tests keep out of the
/// repository and its target dir).
fn scratch(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("kanbanr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("home")).unwrap();
    std::fs::create_dir_all(base.join("data")).unwrap();
    base
}

/// Start `kanbanr serve` over `base`'s data, with `--ui-dir` when given, and wait until it answers.
fn serve_ui(base: &std::path::Path, ui: Option<&std::path::Path>) -> (Daemon, String) {
    let port = free_port();
    let mut cmd = Command::new(cli());
    cmd.args(["serve", "--bind", &format!("127.0.0.1:{port}")]);
    if let Some(dir) = ui {
        cmd.args(["--ui-dir", dir.to_str().unwrap()]);
    }
    let mut child = cmd
        .env("HOME", base.join("home"))
        .env("KANBANR_DATA_DIR", base.join("data"))
        .spawn()
        .expect("spawn kanbanr serve");
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
    std::thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        panic!("this test's daemon exited ({status}) — something else answered on port {port}");
    }
    (Daemon(child), url)
}

/// FEAT-178: the monitor is built with relative asset paths so one build works from any folder of a
/// static host. Under `--ui-dir`, a deep link must still load them from the root, so the index the
/// daemon serves carries `<base href="/">`.
#[test]
fn a_deep_link_under_ui_dir_loads_relative_assets_from_the_root() {
    let base = scratch("deeplink");
    let ui = base.join("dist");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><html><head><script type=\"module\" src=\"./assets/app.js\"></script></head><body></body></html>",
    )
    .unwrap();
    std::fs::write(ui.join("assets/app.js"), "console.log(1)").unwrap();
    let (d, url) = serve_ui(&base, Some(&ui));

    let page = ureq::get(&format!("{url}/p/demo/feature/FEAT-1"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        page.contains("<head><base href=\"/\">"),
        "a deep link's index must resolve ./assets from the root: {page}"
    );
    assert!(page.contains("./assets/app.js"), "the index itself: {page}");
    // The relative asset, resolved against that base, is served.
    let asset = ureq::get(&format!("{url}/assets/app.js")).call().unwrap();
    assert_eq!(asset.status(), 200);
    // Asked for by name, the index is still the copy with its base.
    let named = ureq::get(&format!("{url}/index.html"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        named.contains("<base href=\"/\">"),
        "index.html by name: {named}"
    );
    drop(d);
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-178: under `--ui-dir` the favicon (and the runtime config.json) used to fall back to
/// index.html, because only `/assets` was served from the dir. Any file the build puts there is
/// served as itself; a path that does not exist, or reaches outside the dir, is still the app's.
#[test]
fn the_favicon_and_config_are_served_under_ui_dir_not_the_index() {
    let base = scratch("favicon");
    let ui = base.join("dist");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><html><head></head>APP</html>",
    )
    .unwrap();
    std::fs::write(
        ui.join("favicon.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    )
    .unwrap();
    std::fs::write(ui.join("config.json"), "{\"apiBase\": \"/api\"}").unwrap();
    std::fs::write(base.join("secret.txt"), "outside").unwrap();
    let (d, url) = serve_ui(&base, Some(&ui));

    let icon = ureq::get(&format!("{url}/favicon.svg")).call().unwrap();
    assert_eq!(icon.header("content-type"), Some("image/svg+xml"));
    let body = icon.into_string().unwrap();
    assert!(
        body.starts_with("<svg"),
        "the favicon, not the index: {body}"
    );

    let config = ureq::get(&format!("{url}/config.json")).call().unwrap();
    assert!(
        config.header("content-type").unwrap_or("").contains("json"),
        "config.json's type: {:?}",
        config.header("content-type")
    );
    assert!(config.into_string().unwrap().contains("apiBase"));

    // Not a file in the dir: the app's route, as before.
    let missing = ureq::get(&format!("{url}/no-such.png"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        missing.contains("APP"),
        "an unknown path is the app's: {missing}"
    );
    let escape = ureq::get(&format!("{url}/%2e%2e/secret.txt"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        !escape.contains("outside"),
        "a path must not leave the ui dir: {escape}"
    );
    drop(d);
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-178: the embedded monitor is the relative build too, so its index also gets the root base,
/// and the shipped config.json (the embedded defaults) is served beside it.
#[test]
fn the_embedded_index_carries_a_root_base_and_its_config() {
    let base = scratch("embedbase");
    let (d, url) = serve_ui(&base, None);
    let page = match ureq::get(&format!("{url}/p/demo/feature/FEAT-1")).call() {
        Ok(resp) => resp.into_string().unwrap(),
        Err(e) => panic!("no embedded monitor (build web/dist first): {e}"),
    };
    assert!(
        page.contains("<base href=\"/\">"),
        "the embedded index must carry a root base: {page}"
    );
    let config: serde_json::Value = serde_json::from_str(
        &ureq::get(&format!("{url}/config.json"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
    )
    .expect("config.json is JSON, not the index");
    assert_eq!(config["apiBase"], "/api", "the embedded defaults: {config}");
    assert_eq!(config["readOnly"], false, "the embedded defaults: {config}");
    drop(d);
    let _ = std::fs::remove_dir_all(&base);
}

/// FEAT-183 (Q1): the daemon promotes with the code repository it was started in, when that
/// folder's marker names the project; started anywhere else, the promotions that need the
/// repository are refused and the board does not move.
#[test]
fn promote_through_the_daemon_uses_the_repository_it_was_started_in() {
    // Outside the build tree, never under CARGO_TARGET_TMPDIR: that is inside this repository,
    // whose own marker `init` and the daemon would find by walking up — and the test would write
    // to the real board.
    let base = std::env::temp_dir().join(format!("kanbanr-promote-daemon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .output()
            .expect("run git");
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let kanbanr = |args: &[&str]| {
        let o = Command::new(cli())
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("KANBANR_SERVER_URL")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run cli");
        assert!(
            o.status.success(),
            "cmd {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "user.email", "t@x"]);
    git(&["commit", "-q", "--allow-empty", "-m", "[no-ref] base"]);
    kanbanr(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    let board = kanbanr(&["where"]);
    assert!(
        std::path::Path::new(&board).starts_with(&base),
        "the scratch board must be the test's own, not {board}"
    );
    kanbanr(&["config", "cadence", "--releases", "on"]);
    kanbanr(&["release", "add", "v1.0.0"]);
    kanbanr(&["release", "add", "v2.0.0"]);

    let start = |dir: &std::path::Path| -> (Daemon, String) {
        let port = free_port();
        let child = Command::new(cli())
            .args([
                "serve",
                "--allow-writes",
                "--bind",
                &format!("127.0.0.1:{port}"),
            ])
            .current_dir(dir)
            .env("HOME", &home)
            .env("KANBANR_DATA_DIR", &board)
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
    // The status and body of a promotion, whichever way it went.
    let promote = |url: &str, version: &str| -> (u16, String) {
        match ureq::post(&format!(
            "{url}/api/write/projects/shop/releases/{version}/promote"
        ))
        .set("content-type", "application/json")
        .send_string(r#"{"stage": "candidate", "by": "A"}"#)
        {
            Ok(r) => (r.status(), r.into_string().unwrap()),
            Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap()),
            Err(e) => panic!("no answer from the daemon: {e}"),
        }
    };

    // Started in the project folder: the branch is made in that repository.
    {
        let (_daemon, url) = start(&work);
        let (status, made) = promote(&url, "v1.0.0");
        assert_eq!(status, 200, "promote through the daemon: {made}");
        assert!(
            made.contains("\"branch_created\":\"release/v1.0.0\""),
            "{made}"
        );
        assert_eq!(
            git(&["rev-parse", "release/v1.0.0"]),
            git(&["rev-parse", "main"])
        );
    }

    // Started elsewhere: refused, saying where to run it, and nothing moves.
    {
        let (_daemon, url) = start(&base);
        let (status, why) = promote(&url, "v2.0.0");
        assert_eq!(
            status, 400,
            "a promotion with no code repository is refused"
        );
        assert!(why.contains("use the CLI from the project folder"), "{why}");
        let releases = ureq::get(&format!("{url}/api/projects/shop/releases"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        let releases: serde_json::Value = serde_json::from_str(&releases).unwrap();
        assert_eq!(releases[1]["version"], "v2.0.0");
        assert!(releases[1].get("stage").is_none(), "{releases}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A scratch code repository with its own board, releases on and `v1.0.0` planned (FEAT-185). Under
/// the system temp folder, never inside this repository. Returns the home, the code folder, and the
/// board folder.
fn release_scratch(base: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
    let _ = std::fs::remove_dir_all(base);
    let home = base.join("home");
    let work = base.join("code").join("shop");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .output()
            .expect("run git");
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "user.email", "t@x"]);
    git(&["commit", "-q", "--allow-empty", "-m", "[no-ref] base"]);
    let run = |args: &[&str]| release_cli(&home, &work, args);
    run(&[
        "init",
        "shop",
        "--author",
        "A",
        "--email",
        "a@x",
        "--no-hooks",
    ]);
    let board = PathBuf::from(run(&["where"]));
    assert!(
        board.starts_with(base),
        "the scratch board must be the test's own, not {}",
        board.display()
    );
    run(&["config", "cadence", "--releases", "on"]);
    run(&["release", "add", "v1.0.0"]);
    (home, work, board)
}

fn release_cli(home: &std::path::Path, work: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new(cli())
        .args(args)
        .current_dir(work)
        .env("HOME", home)
        .env_remove("KANBANR_DATA_DIR")
        .env_remove("KANBANR_PROJECT")
        .env_remove("KANBANR_SERVER_URL")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run cli");
    assert!(
        o.status.success(),
        "cmd {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// A board's releases.yaml, with each stage change's time cleared: two promotions made a moment
/// apart are the same promotion.
fn releases_yaml(board: &std::path::Path) -> serde_yaml::Value {
    let text = std::fs::read_to_string(board.join("projects").join("shop").join("releases.yaml"))
        .expect("releases.yaml");
    let mut v: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    for release in v.as_sequence_mut().unwrap() {
        if let Some(stages) = release.get_mut("stages").and_then(|s| s.as_sequence_mut()) {
            for change in stages {
                change["at"] = serde_yaml::Value::String(String::new());
            }
        }
    }
    v
}

/// FEAT-185 R-2: the monitor's Promote, posted to a writes-enabled daemon started in the code
/// repository, makes the release branch and leaves releases.yaml exactly as `kanbanr release
/// promote` leaves it on a twin board.
#[test]
fn monitor_promote_matches_cli() {
    let root = std::env::temp_dir().join(format!("kanbanr-monitor-promote-{}", std::process::id()));
    let (home, work, board) = release_scratch(&root.join("monitor"));
    let (twin_home, twin_work, twin_board) = release_scratch(&root.join("cli"));

    // The CLI, on the twin.
    release_cli(&twin_home, &twin_work, &["release", "promote", "v1.0.0"]);

    // The monitor: the Promote button posts the next stage, as the board's identity.
    let port = free_port();
    let child = Command::new(cli())
        .args([
            "serve",
            "--allow-writes",
            "--bind",
            &format!("127.0.0.1:{port}"),
        ])
        .current_dir(&work)
        .env("HOME", &home)
        .env("KANBANR_DATA_DIR", &board)
        .spawn()
        .expect("spawn kanbanr serve");
    let _daemon = Daemon(child);
    let url = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "daemon did not become ready");
        match ureq::get(&format!("{url}/api/projects")).call() {
            Ok(_) | Err(ureq::Error::Status(_, _)) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let made = match ureq::post(&format!(
        "{url}/api/write/projects/shop/releases/v1.0.0/promote"
    ))
    .set("content-type", "application/json")
    .send_string(r#"{"stage": "candidate", "by": "A"}"#)
    {
        Ok(r) => r.into_string().unwrap(),
        Err(e) => panic!("promote through the daemon: {e}"),
    };
    assert!(
        made.contains("\"branch_created\":\"release/v1.0.0\""),
        "{made}"
    );
    let branch = Command::new("git")
        .args(["rev-parse", "--verify", "-q", "release/v1.0.0"])
        .current_dir(&work)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        branch.status.success(),
        "the release branch is made in the daemon's repository"
    );

    let monitor = releases_yaml(&board);
    assert_eq!(monitor[0]["stage"], "candidate", "{monitor:?}");
    assert_eq!(
        monitor,
        releases_yaml(&twin_board),
        "the monitor's promotion is the CLI's"
    );
    let _ = std::fs::remove_dir_all(&root);
}
