//! What the server's integration tests share: a scratch folder, a board under git, the monitor
//! over it with writes on, and the token its page carries.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A scratch folder under the system temporary directory, removed afterwards.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-server-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A board that is a git repo with one commit, so a write adds a second.
pub fn board(scratch: &Scratch) -> PathBuf {
    let data = scratch.0.join("data");
    std::fs::create_dir_all(data.join("projects")).unwrap();
    git(&data, &["init", "-q"]);
    git(&data, &["config", "user.name", "Tester"]);
    git(&data, &["config", "user.email", "t@example.com"]);
    git(&data, &["commit", "-q", "--allow-empty", "-m", "start"]);
    data
}

/// The monitor over `data` with a page that has a head, on a free port. Returns its base URL.
pub fn serve(scratch: &Scratch, data: &Path) -> String {
    let ui = scratch.0.join("dist");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><html><head><title>t</title></head><body></body></html>",
    )
    .unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bind = format!("127.0.0.1:{port}");
    let (dir, addr, ui) = (
        data.to_path_buf(),
        bind.clone(),
        ui.to_str().unwrap().to_string(),
    );
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(kanbanr_server::run(dir, addr, Some(ui), true, None))
            .unwrap();
    });
    let url = format!("http://{bind}");
    for _ in 0..200 {
        if ureq::get(&format!("{url}/healthz")).call().is_ok() {
            return url;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the monitor did not come up on {bind}");
}

/// The token in the page the monitor serves.
pub fn page_token(url: &str) -> String {
    let html = ureq::get(&format!("{url}/"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let marker = "<meta name=\"kanbanr-token\" content=\"";
    let start = html.find(marker).expect("the page carries a token") + marker.len();
    let end = start + html[start..].find('"').unwrap();
    html[start..end].to_string()
}

pub fn status(r: Result<ureq::Response, ureq::Error>) -> u16 {
    match r {
        Ok(r) => r.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(e) => panic!("request failed: {e}"),
    }
}
