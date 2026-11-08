//! Layer-2 packaging smoke test: confirm the real Docker image boots and serves the view daemon.
//!
//! Functional coverage lives in the Docker-less tests (`local_mode.rs`, `view_daemon.rs`). Here we
//! only validate the *packaged image*: it scaffolds a project and serves it read-only (no auth) —
//! so the container has no surprises beyond the binary already tested.
//!
//! `#[ignore]`d because it needs Docker and a built image tagged `kanbanr:itest`:
//!     make itest
//!     # or: docker build -f docker/Dockerfile -t kanbanr:itest . && cargo test -p kanbanr-cli -- --ignored
//!
//! It drives the `docker` CLI directly (no test-container crate, MNT-004). The image is
//! `KANBANR_ITEST_IMAGE`, or `kanbanr:itest`; the container's port is published on loopback only.

use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// A running container, removed when dropped (errors ignored: the test is over).
struct Container {
    id: String,
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

/// Run `docker <args>` and return its stdout; panics with the command and stderr on failure.
fn docker(args: &[&str]) -> String {
    let out = Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run `docker {}`: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`docker {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The host port from `docker port <id> 8080/tcp`, whose first line reads `127.0.0.1:49153`.
fn parse_host_port(out: &str) -> Result<u16, String> {
    let line = out
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .ok_or_else(|| format!("`docker port` printed nothing: {out:?}"))?;
    let (_, port) = line
        .rsplit_once(':')
        .ok_or_else(|| format!("no host:port in `docker port` line {line:?}"))?;
    port.parse()
        .map_err(|_| format!("no port number in `docker port` line {line:?}"))
}

/// Boot the image: scaffold a demo project, then run the view daemon over /data.
fn start() -> (Container, String) {
    let image = std::env::var("KANBANR_ITEST_IMAGE").unwrap_or_else(|_| "kanbanr:itest".into());
    let cmd = "kanbanr init demo --author CI --email ci@kanbanr.local >/dev/null && \
               exec kanbanr serve --bind 0.0.0.0:8080 --ui-dir /app/web";
    let id = docker(&[
        "run",
        "-d",
        "-p",
        "127.0.0.1::8080",
        &image,
        "sh",
        "-c",
        cmd,
    ])
    .trim()
    .to_string();
    let container = Container { id };

    // Wait for the daemon to say it is up (it logs to stderr; `docker logs` returns both streams).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let logs = Command::new("docker")
            .args(["logs", &container.id])
            .output()
            .expect("run `docker logs`");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&logs.stdout),
            String::from_utf8_lossy(&logs.stderr)
        );
        if text.contains("view daemon") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the view daemon did not start within 60s (build the image first: docker build -f docker/Dockerfile -t kanbanr:itest .). Logs:\n{text}"
        );
        sleep(Duration::from_millis(250));
    }

    let port = parse_host_port(&docker(&["port", &container.id, "8080/tcp"]))
        .unwrap_or_else(|e| panic!("{e}"));
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

#[cfg(test)]
mod tests {
    use super::parse_host_port;

    #[test]
    fn host_port_is_read_from_docker_port_output() {
        assert_eq!(parse_host_port("127.0.0.1:49153\n"), Ok(49153));
        assert_eq!(parse_host_port("0.0.0.0:80\n[::]:80\n"), Ok(80));
    }

    #[test]
    fn an_unreadable_docker_port_line_is_an_error() {
        assert!(parse_host_port("").is_err());
        assert!(parse_host_port("nonsense").is_err());
        assert!(
            parse_host_port("127.0.0.1:http")
                .unwrap_err()
                .contains("127.0.0.1:http")
        );
    }
}
