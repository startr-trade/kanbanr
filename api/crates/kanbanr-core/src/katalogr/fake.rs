//! A fake `katalogr api` for kanbanr's own tests: a loopback HTTP/1.1 server on a `TcpListener`
//! thread, `Content-Length` bodies, `Connection: close`. Never shipped behaviour (hidden from the
//! docs); the production code never starts it. FTR-016.

use super::query::{OPS, SPECS};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The position the fake serves when a request names none, and offers when one is missing.
pub const DEFAULT_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
/// The view ID of the fake's exact answers (the shape of `views-v1.json`'s sample).
pub const VIEW_ID: &str = "v1-0123456789abcdef0123456789abcdef";

const COMMON: [&str; 5] = ["context", "at", "consistency", "timeout", "no-peers"];

/// A running fake. It stops when dropped.
pub struct FakeKatalogr {
    pub url: String,
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeKatalogr {
    /// The request targets seen so far (path and query), oldest first.
    pub fn requests(&self) -> Vec<String> {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for FakeKatalogr {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so the thread ends.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
    }
}

/// A loopback server answering `GET /api/v1/meta` with `{..., "capabilities": caps}` and
/// `GET /api/v1/views/<op>` with an exact answer shaped like `views-v1.json`'s (its `positions`
/// and `query` replaced by the request's `at` and args), 409 `position-unavailable` for an `at`
/// in `missing`, 404 for an op not in `OPS`, 400 for an unknown parameter. `delay` sleeps before
/// each answer.
pub fn serve(caps: &[&str], missing: &[&str], delay: Duration) -> FakeKatalogr {
    let caps: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    let missing: Vec<String> = missing.iter().map(|c| c.to_string()).collect();
    start(delay, move |target| answer(target, &caps, &missing))
}

/// A loopback server that answers every request with `status` and `body`, for the shapes
/// `serve` does not make: a 404 where `/api/v1/meta` should be, a body that is not JSON, one over
/// the cap.
pub fn serve_raw(status: u16, body: &str) -> FakeKatalogr {
    let body = body.to_string();
    start(Duration::ZERO, move |_| (status, body.clone()))
}

/// A port nothing listens on, for the Absent case.
pub fn closed_port_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = l.local_addr().expect("local addr").port();
    drop(l);
    format!("http://127.0.0.1:{port}")
}

fn start(
    delay: Duration,
    handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> FakeKatalogr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let addr = listener.local_addr().expect("local addr");
    let stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(handler);
    let (stop2, seen2) = (stop.clone(), seen.clone());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let (handler, seen) = (handler.clone(), seen2.clone());
            std::thread::spawn(move || handle(conn, delay, &*handler, &seen));
        }
    });
    FakeKatalogr {
        url: format!("http://{addr}"),
        stop,
        addr,
        seen,
    }
}

fn handle(
    mut conn: TcpStream,
    delay: Duration,
    handler: &(dyn Fn(&str) -> (u16, String) + Send + Sync),
    seen: &Mutex<Vec<String>>,
) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut first = text.lines().next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let (status, body) = if method == "GET" {
        if let Ok(mut s) = seen.lock() {
            s.push(target.to_string());
        }
        handler(target)
    } else {
        (405, error_body("method-not-allowed", "only GET"))
    };
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Status",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = conn.write_all(head.as_bytes());
    let _ = conn.write_all(body.as_bytes());
    let _ = conn.flush();
}

fn error_body(kind: &str, message: &str) -> String {
    json!({"schema": 1, "error": kind, "message": message}).to_string()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn answer(target: &str, caps: &[String], missing: &[String]) -> (u16, String) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == "/api/v1/meta" {
        let mut sorted = caps.to_vec();
        sorted.sort();
        return (
            200,
            json!({
                "api": 1, "contract": 99, "schema": 1, "position_format": 1, "view_format": 1,
                "katalogr": "0.2.0-fake", "capabilities": sorted, "unknown_field": true
            })
            .to_string(),
        );
    }
    let Some(op) = path.strip_prefix("/api/v1/views/") else {
        return (404, error_body("not-found", "no such route"));
    };
    let Some(spec) = SPECS.iter().find(|s| s.op == op && OPS.contains(&s.op)) else {
        return (
            404,
            error_body("unknown-view", &format!("unknown view `{op}`")),
        );
    };
    let mut args = BTreeMap::new();
    let mut at = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (decode(k), decode(v));
        let known = COMMON.contains(&k.as_str())
            || spec.positionals.contains(&k.as_str())
            || spec.flags.iter().any(|(n, _)| *n == k);
        if !known {
            return (
                400,
                error_body("bad-parameter", &format!("unknown parameter `{k}`")),
            );
        }
        if k == "at" {
            at = Some(v);
        } else if !COMMON.contains(&k.as_str()) {
            args.insert(k, v);
        }
    }
    if let Some(at) = &at
        && missing.iter().any(|m| m == at)
    {
        return (
            409,
            json!({
                "schema": 1, "error": "position-unavailable",
                "message": "no context holds the wanted position",
                "wanted": at,
                "available": [{"repo": "app", "commit": DEFAULT_COMMIT, "dirty": null}]
            })
            .to_string(),
        );
    }
    let position = match at.as_deref() {
        Some(a) => {
            crate::position::PositionRef::parse("app", a).unwrap_or(crate::position::PositionRef {
                repo: "app".into(),
                commit: Some(DEFAULT_COMMIT.into()),
                dirty: None,
            })
        }
        None => crate::position::PositionRef {
            repo: "app".into(),
            commit: Some(DEFAULT_COMMIT.into()),
            dirty: None,
        },
    };
    let body: Value = json!({
        "schema": 1,
        "view_format": 1,
        "view_id": VIEW_ID,
        "exact": true,
        "positions": [position],
        "query": {"op": op, "args": args},
        "mermaid": "flowchart LR\n  n1[\"app:api.tx\"]\n  n2[\"app:lib.tx\"]\n  n1 -->|\"calls\"| n2\n",
        "graph": {"edges": [], "nodes": [], "shown": 2, "title": op, "total": 2},
        "pele": "1.7.0"
    });
    (200, body.to_string())
}
