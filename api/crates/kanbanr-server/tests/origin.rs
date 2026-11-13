//! The monitor answers only its own page and hosts (DEF-024): another Host is refused, another
//! origin gets no cross-origin headers, and a write needs the token its own page was given at this
//! start and its own origin. Each test starts the library's `run` on a free port over a temporary
//! board, with writes on.

mod common;

use common::{Scratch, board, git, page_token, serve, status};
use std::path::Path;
use std::time::Duration;

/// The commits on the board.
fn commits(data: &Path) -> String {
    git(data, &["rev-list", "--count", "HEAD"])
}

fn create_project(url: &str, headers: &[(&str, &str)]) -> u16 {
    let mut req = ureq::post(&format!("{url}/api/write/projects"));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    status(
        req.set("content-type", "application/json")
            .send_string(r#"{"name": "demo"}"#),
    )
}

#[test]
fn a_request_for_another_host_is_refused() {
    let s = Scratch::new("host");
    let data = board(&s);
    let url = serve(&s, &data);
    let before = commits(&data);
    for path in ["/api/projects", "/healthz", "/"] {
        let r = ureq::get(&format!("{url}{path}"))
            .set("Host", "attacker.example")
            .call();
        assert_eq!(status(r), 403, "{path} for another host");
    }
    // A write for another host is refused too, even with the right token.
    let token = page_token(&url);
    let r = create_project(
        &url,
        &[("Host", "attacker.example"), ("x-kanbanr-token", &token)],
    );
    assert_eq!(r, 403);
    assert_eq!(commits(&data), before, "nothing was written");
    // The names the monitor answers for still work, and the refusal names them.
    let port = url.rsplit(':').next().unwrap();
    for host in [
        format!("localhost:{port}"),
        format!("127.0.0.1:{port}"),
        format!("[::1]:{port}"),
    ] {
        let r = ureq::get(&format!("{url}/api/projects"))
            .set("Host", &host)
            .call();
        assert_eq!(status(r), 200, "{host}");
    }
    match ureq::get(&format!("{url}/api/projects"))
        .set("Host", "attacker.example")
        .call()
    {
        Err(ureq::Error::Status(403, r)) => {
            let body = r.into_string().unwrap();
            assert!(body.contains("KANBANR_ALLOWED_HOSTS"), "{body}");
        }
        other => panic!("expected a refusal: {other:?}"),
    }
}

#[test]
fn a_published_port_serves_the_page() {
    // A container binds one port and publishes another: the browser names the published one.
    let s = Scratch::new("published");
    let data = board(&s);
    let url = serve(&s, &data);
    let bound: u16 = url.rsplit(':').next().unwrap().parse().unwrap();
    let other = if bound == 18080 { 18081 } else { 18080 };
    for host in [
        format!("localhost:{other}"),
        format!("127.0.0.1:{other}"),
        format!("[::1]:{other}"),
        "localhost".to_string(),
    ] {
        for path in ["/", "/api/projects"] {
            let r = ureq::get(&format!("{url}{path}")).set("Host", &host).call();
            assert_eq!(status(r), 200, "{path} for {host}");
        }
    }
}

#[test]
fn another_origin_gets_no_cross_origin_headers() {
    let s = Scratch::new("cors");
    let data = board(&s);
    let url = serve(&s, &data);
    let resp = ureq::get(&format!("{url}/api/projects"))
        .set("Origin", "http://attacker.example")
        .call()
        .unwrap();
    assert!(
        resp.header("access-control-allow-origin").is_none(),
        "no header lets another origin read the response"
    );
    // A preflight is not answered with permission either.
    let resp = match ureq::request("OPTIONS", &format!("{url}/api/write/projects"))
        .set("Origin", "http://attacker.example")
        .set("Access-Control-Request-Method", "POST")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{e}"),
    };
    assert!(resp.header("access-control-allow-origin").is_none());
    assert!(resp.header("access-control-allow-methods").is_none());
}

#[test]
fn a_write_without_the_token_is_refused() {
    let s = Scratch::new("notoken");
    let data = board(&s);
    let url = serve(&s, &data);
    let before = commits(&data);
    assert_eq!(create_project(&url, &[]), 403);
    assert_eq!(
        create_project(&url, &[("x-kanbanr-token", "not-the-token")]),
        403
    );
    assert_eq!(commits(&data), before, "nothing was written");
    assert!(!data.join("projects/demo").exists());
}

#[test]
fn a_write_from_another_origin_is_refused() {
    let s = Scratch::new("badorigin");
    let data = board(&s);
    let url = serve(&s, &data);
    let token = page_token(&url);
    let before = commits(&data);
    let r = create_project(
        &url,
        &[
            ("x-kanbanr-token", &token),
            ("Origin", "http://attacker.example"),
        ],
    );
    assert_eq!(r, 403, "even with the token, another origin is refused");
    assert_eq!(commits(&data), before, "nothing was written");
}

#[test]
fn the_monitors_own_page_writes_as_before() {
    let s = Scratch::new("own");
    let data = board(&s);
    let url = serve(&s, &data);
    let token = page_token(&url);
    let before: u32 = commits(&data).parse().unwrap();
    // A browser sends the token and its own origin.
    let r = create_project(&url, &[("x-kanbanr-token", &token), ("Origin", &url)]);
    assert_eq!(r, 200);
    assert_eq!(commits(&data).parse::<u32>().unwrap(), before + 1);
    assert!(data.join("projects/demo").exists());
}

#[test]
fn each_start_gets_a_new_token_in_the_page() {
    let (a, b) = (Scratch::new("tok-a"), Scratch::new("tok-b"));
    let (da, db) = (board(&a), board(&b));
    let (ua, ub) = (serve(&a, &da), serve(&b, &db));
    let (ta, tb) = (page_token(&ua), page_token(&ub));
    assert_eq!(ta.len(), 64, "32 random bytes as hex");
    assert!(ta.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(ta, tb, "two starts, two tokens");
    // The token of one start does not write on the other.
    assert_eq!(create_project(&ub, &[("x-kanbanr-token", &ta)]), 403);
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn writes_on_a_non_local_bind_refuse_to_start() {
    // The test never sets or clears variables: it needs the acceptance unset.
    assert!(
        std::env::var("KANBANR_ALLOW_REMOTE_WRITES").is_err(),
        "unset KANBANR_ALLOW_REMOTE_WRITES to run this test"
    );
    let s = Scratch::new("refuse");
    let data = board(&s);
    let before = commits(&data);
    let port = free_port();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let r = rt.block_on(kanbanr_server::run_with(
        data.clone(),
        format!("0.0.0.0:{port}"),
        None,
        true,
        None,
        false,
    ));
    let text = format!(
        "{:#}",
        r.expect_err("a reachable bind with writes is refused")
    );
    assert!(text.contains("--allow-remote-writes"), "{text}");
    assert!(
        std::net::TcpListener::bind(format!("0.0.0.0:{port}")).is_ok(),
        "nothing listens on the port"
    );
    assert_eq!(commits(&data), before);
}

#[test]
fn a_read_only_monitor_binds_anywhere() {
    let s = Scratch::new("readonly");
    let data = board(&s);
    let port = free_port();
    let dir = data.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(kanbanr_server::run_with(
            dir,
            format!("0.0.0.0:{port}"),
            None,
            false,
            None,
            false,
        ))
        .unwrap();
    });
    let url = format!("http://127.0.0.1:{port}/healthz");
    for _ in 0..200 {
        if ureq::get(&url).call().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the read-only monitor did not come up on 0.0.0.0:{port}");
}
