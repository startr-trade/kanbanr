//! Layer-2 packaging smoke test: confirm the real Docker image boots and serves the view daemon.
//!
//! Functional coverage lives in the Docker-less tests (`local_mode.rs`, `view_daemon.rs`). Here we
//! only validate the *packaged image*: it scaffolds a project and serves it read-only (no auth) —
//! so the container has no surprises beyond the binary already tested.
//!
//! `#[ignore]`d because it needs Docker and a built image tagged `kanbanr:itest`:
//!     make itest
//!     # or: docker build -f docker/Dockerfile -t kanbanr:itest . && cargo test -p kanbanr-cli -- --ignored

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::SyncRunner;
use testcontainers::{GenericImage, ImageExt};

/// Boot the image: scaffold a demo project, then run the view daemon over /data.
fn start() -> (testcontainers::Container<GenericImage>, String) {
    let cmd = "kanbanr init demo --author CI --email ci@kanbanr.local >/dev/null && \
               exec kanbanr serve --bind 0.0.0.0:8080 --ui-dir /app/web";
    let container = GenericImage::new("kanbanr", "itest")
        .with_exposed_port(8080.tcp())
        .with_wait_for(WaitFor::message_on_stderr("view daemon"))
        .with_cmd(vec!["sh".to_string(), "-c".to_string(), cmd.to_string()])
        .start()
        .expect("start kanbanr container (build it first: docker build -f docker/Dockerfile -t kanbanr:itest .)");
    let port = container
        .get_host_port_ipv4(8080.tcp())
        .expect("mapped port");
    (container, format!("http://127.0.0.1:{port}"))
}

#[test]
#[ignore = "requires Docker + `kanbanr:itest` image (run via `make itest`)"]
fn image_boots_and_serves_the_view() {
    let (_container, base) = start();

    // The view daemon is read-only and unauthenticated; the scaffolded project is visible.
    let resp = ureq::get(&format!("{base}/api/projects"))
        .call()
        .expect("read projects");
    assert_eq!(resp.status(), 200, "image serves the read API");
    let body = resp.into_string().unwrap();
    assert!(
        body.contains("demo"),
        "scaffolded project is served: {body}"
    );

    // The SPA is served at the root.
    let root = ureq::get(&base).call().expect("read SPA");
    assert_eq!(root.status(), 200, "image serves the SPA");
}
