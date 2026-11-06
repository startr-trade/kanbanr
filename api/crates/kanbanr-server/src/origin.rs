//! Who the monitor answers (DEF-024). The monitor's page is served by the monitor itself, so
//! nothing needs cross-origin access: every request must name a host the monitor answers for, and
//! a write must carry the token its own page was given at this start (and, when a browser sends
//! one, the monitor's own Origin). Reads by the page, the VS Code extension's view of it and
//! `curl` on the same machine work as before; the CLI writes the board directly and never comes
//! through here.

use axum::extract::{Request, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// The header the page sends its token in.
pub const TOKEN_HEADER: &str = "x-kanbanr-token";

pub struct Guard {
    /// Lowercased, each `name:port` (plus the bare name of an allowed host).
    hosts: Vec<String>,
    token: String,
}

impl Guard {
    /// Loopback names (`localhost`, `127.0.0.1`, `[::1]`) with the bound port, the bound address
    /// itself, and each host in `allowed` (`name[:port]`, for a reverse proxy).
    pub fn new(bind: &str, allowed: &[String]) -> Guard {
        let bind = bind.trim().to_ascii_lowercase();
        let port = bind
            .rsplit_once(':')
            .map(|(_, p)| p.to_string())
            .unwrap_or_default();
        let mut hosts = Vec::new();
        for name in ["localhost", "127.0.0.1", "[::1]"] {
            hosts.push(format!("{name}:{port}"));
        }
        hosts.push(bind.clone());
        for host in allowed {
            let host = host.trim().to_ascii_lowercase();
            if host.is_empty() {
                continue;
            }
            // A name without a port is answered on the bound port as well as bare, which is how
            // a reverse proxy on 80 or 443 names it.
            if !host.contains(':') || host.ends_with(']') {
                hosts.push(format!("{host}:{port}"));
            }
            hosts.push(host);
        }
        hosts.sort();
        hosts.dedup();
        Guard {
            hosts,
            token: new_token(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The hosts this monitor answers for, as the start line and the refusal name them.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub fn host_ok(&self, host: Option<&str>) -> bool {
        match host {
            Some(h) => {
                let h = h.trim().to_ascii_lowercase();
                self.hosts.contains(&h)
            }
            None => false,
        }
    }

    /// Whether a request may proceed. Reads always may (the host is checked separately); any other
    /// method needs the token, and an Origin, when there is one, must be the monitor's own.
    pub fn write_ok(
        &self,
        method: &Method,
        origin: Option<&str>,
        host: &str,
        token: Option<&str>,
    ) -> bool {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return true;
        }
        let Some(token) = token else { return false };
        if !same(token.as_bytes(), self.token.as_bytes()) {
            return false;
        }
        match origin {
            Some(o) => o.eq_ignore_ascii_case(&format!("http://{host}")),
            None => true,
        }
    }
}

/// 32 random bytes as hex.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system has no random bytes to give");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Equality that takes the same time wherever two values differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A refusal, logged like any request (the token is never in it). The guard wraps the access log,
/// so it writes its own line.
fn refuse(req: &Request, message: String) -> Response {
    eprintln!("{} {} -> 403 ({message})", req.method(), req.uri().path());
    (StatusCode::FORBIDDEN, message).into_response()
}

/// The layer: nothing reaches a handler on a refusal.
pub async fn check(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if !g.host_ok(host.as_deref()) {
        return refuse(
            &req,
            format!(
                "this monitor answers for {}; set KANBANR_ALLOWED_HOSTS to add one",
                g.hosts().join(", ")
            ),
        );
    }
    if req.uri().path().starts_with("/api/") {
        let origin = req
            .headers()
            .get(ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let token = req
            .headers()
            .get(TOKEN_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if !g.write_ok(
            req.method(),
            origin.as_deref(),
            host.as_deref().unwrap_or(""),
            token.as_deref(),
        ) {
            return refuse(
                &req,
                "writes are accepted only from this monitor's own page".to_string(),
            );
        }
    }
    next.run(req).await
}
