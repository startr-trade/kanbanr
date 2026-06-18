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
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("view-{}", std::process::id()));
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
        assert!(o.status.success(), "cmd {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };

    // Populate the folder locally (the CLI is the writer).
    run(&["identity", "--name", "Tester", "--email", "t@example.com"]);
    run(&["--project", "demo", "project", "init", "demo", "--description", "views"]);
    run(&["--project", "demo", "milestone", "add", "--name", "M", "--code", "MS-1"]);
    run(&["--project", "demo", "feature", "add", "--title", "Login", "--milestone", "MS-1"]);

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
    let projects = ureq::get(&format!("{url}/api/projects")).call().unwrap().into_string().unwrap();
    assert!(projects.contains("demo"), "projects: {projects}");
    let project = ureq::get(&format!("{url}/api/projects/demo")).call().unwrap().into_string().unwrap();
    assert!(project.contains("Login"), "project: {project}");

    // The activity changelog was populated by the local writes and is served.
    let activity = ureq::get(&format!("{url}/api/projects/demo/activity")).call().unwrap().into_string().unwrap();
    assert!(activity.contains("Tester"), "activity should record the actor: {activity}");

    let _ = std::fs::remove_dir_all(&base);
}
