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
    /// Lowercased, each `name:port` (plus the bare name of an allowed host). The loopback names
    /// are not here: they are answered on any port (`loopback_name`).
    hosts: Vec<String>,
    /// Origins a write may come from besides the monitor's own `http://` one (from
    /// `parse_origins`, so lowercased).
    origins: Vec<String>,
    token: String,
}

impl Guard {
    /// Loopback names (`localhost`, `127.0.0.1`, `[::1]`) on any port, the bound address itself,
    /// and each host in `allowed` (`name[:port]`, for a reverse proxy).
    pub fn new(bind: &str, allowed: &[String]) -> Guard {
        let bind = bind.trim().to_ascii_lowercase();
        let port = bind
            .rsplit_once(':')
            .map(|(_, p)| p.to_string())
            .unwrap_or_default();
        let mut hosts = Vec::new();
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
            origins: Vec::new(),
            token: new_token(),
        }
    }

    /// Also accept a write whose Origin is one of `origins` (already from `parse_origins`).
    pub fn with_origins(mut self, origins: Vec<String>) -> Guard {
        self.origins = origins;
        self
    }

    /// The origins the person named, as the start line shows them.
    pub fn origins(&self) -> &[String] {
        &self.origins
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The hosts this monitor answers for, as the start line and the refusal name them.
    pub fn hosts(&self) -> Vec<String> {
        let mut shown = vec!["localhost, 127.0.0.1 and [::1] on any port".to_string()];
        shown.extend(self.hosts.iter().filter(|h| !loopback_name(h)).cloned());
        shown
    }

    pub fn host_ok(&self, host: Option<&str>) -> bool {
        match host {
            Some(h) => {
                let h = h.trim().to_ascii_lowercase();
                loopback_name(&h) || self.hosts.contains(&h)
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
            Some(o) => {
                let o = o.to_ascii_lowercase();
                o == format!("http://{}", host.to_ascii_lowercase()) || self.origins.contains(&o)
            }
            None => true,
        }
    }
}

/// Whether `bind` (like `127.0.0.1:8080`, `[::1]:8080`, `localhost:8080`) names this machine only.
/// `Ok(true)` for 127.0.0.0/8, ::1 (also written as an IPv4-mapped address) and `localhost`
/// (any case); `Ok(false)` for every other address or name (names are never resolved: a name that is
/// not `localhost` is treated as reachable); `Err(why)` when it has no readable host:port.
pub fn bind_is_loopback(bind: &str) -> Result<bool, String> {
    let unreadable = || format!("the bind address \"{bind}\" is not host:port");
    let (host, port) = bind.trim().rsplit_once(':').ok_or_else(unreadable)?;
    if port.parse::<u16>().is_err() {
        return Err(unreadable());
    }
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(true);
    }
    Ok(match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(a)) => a.is_loopback(),
        Ok(std::net::IpAddr::V6(a)) => {
            a.is_loopback() || a.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        Err(_) => false,
    })
}

/// The refusal for writes on a reachable bind, or `None` when the monitor may start.
/// `writes`: write routes are on. `accepted`: the person accepted a reachable bind.
pub fn bind_refusal(bind: &str, writes: bool, accepted: bool) -> Result<Option<String>, String> {
    if !writes {
        return Ok(None);
    }
    if bind_is_loopback(bind)? || accepted {
        return Ok(None);
    }
    Ok(Some(format!(
        "refusing to start: writes are on and {bind} is an address other machines can reach. \
The monitor gives its token to any page it serves, so anyone who can open the page can record an \
approval, a sign-off or a release. Bind 127.0.0.1 and put a reverse proxy that authenticates in \
front of it (see \"Exposing the monitor beyond localhost\" in the monitor docs), or, if only people \
you trust can reach {bind}, accept it: pass --allow-remote-writes to serve, or set \
KANBANR_ALLOW_REMOTE_WRITES=1. Name the host people will use in KANBANR_ALLOWED_HOSTS as well."
    )))
}

/// `KANBANR_ALLOW_REMOTE_WRITES`: "1", "true", "yes", "on" are true; unset, "", "0", "false", "no",
/// "off" are false (any case); anything else is an error.
pub fn accepted_from_env(value: Option<&str>) -> Result<bool, String> {
    let Some(v) = value else { return Ok(false) };
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "KANBANR_ALLOW_REMOTE_WRITES is \"{v}\"; use 1 or 0"
        )),
    }
}

/// `KANBANR_ALLOWED_ORIGINS`: comma-separated origins, each `http://` or `https://`, a host (name,
/// IPv4 or `[IPv6]`), an optional port, nothing else. Returned lowercased, with the scheme's
/// default port dropped, which is how a browser writes the Origin header. The first bad entry is
/// named in the error.
pub fn parse_origins(list: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let bad = || {
            format!(
                "KANBANR_ALLOWED_ORIGINS entry \"{entry}\" is not an origin like https://board.example.com"
            )
        };
        let lower = entry.to_ascii_lowercase();
        let (scheme, rest, default_port) = if let Some(r) = lower.strip_prefix("https://") {
            ("https", r, "443")
        } else if let Some(r) = lower.strip_prefix("http://") {
            ("http", r, "80")
        } else {
            return Err(bad());
        };
        let (host, port) = if rest.starts_with('[') {
            let end = rest.find(']').ok_or_else(bad)?;
            let host = &rest[..=end];
            if host[1..end].parse::<std::net::Ipv6Addr>().is_err() {
                return Err(bad());
            }
            match &rest[end + 1..] {
                "" => (host, None),
                tail => (host, Some(tail.strip_prefix(':').ok_or_else(bad)?)),
            }
        } else {
            match rest.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (rest, None),
            }
        };
        let host_ok = !host.is_empty()
            && (host.starts_with('[')
                || host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_')));
        if !host_ok {
            return Err(bad());
        }
        let port = match port {
            Some(p) if p.parse::<u16>().is_ok() => Some(p.parse::<u16>().unwrap().to_string()),
            Some(_) => return Err(bad()),
            None => None,
        };
        out.push(match port {
            Some(p) if p != default_port => format!("{scheme}://{host}:{p}"),
            _ => format!("{scheme}://{host}"),
        });
    }
    Ok(out)
}

/// Whether a Host names this machine's loopback, on any port or none. A foreign page can't be
/// served from such a name, so the host check (which stops DNS rebinding) lets it through.
fn loopback_name(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        // `[::1]` ends in a bracket, so its colons are not a port separator.
        _ => host,
    };
    matches!(name, "localhost" | "127.0.0.1" | "[::1]")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_binds_are_local() {
        for b in [
            "127.0.0.1:8080",
            "127.3.4.5:1",
            "[::1]:8080",
            "[::ffff:127.0.0.1]:80",
            "localhost:8080",
            "LOCALHOST:1",
        ] {
            assert_eq!(bind_is_loopback(b), Ok(true), "{b}");
        }
        for b in [
            "0.0.0.0:8080",
            "[::]:8080",
            "192.168.1.5:80",
            "10.0.0.2:80",
            "myhost:8080",
            "board.example.com:80",
            ":8080",
        ] {
            assert_eq!(bind_is_loopback(b), Ok(false), "{b}");
        }
        for b in ["8080", "127.0.0.1", "127.0.0.1:x", ""] {
            assert!(bind_is_loopback(b).is_err(), "{b}");
        }
    }

    #[test]
    fn writes_on_a_reachable_bind_are_refused_with_the_reason() {
        let t = bind_refusal("0.0.0.0:8080", true, false)
            .unwrap()
            .expect("refused");
        for want in [
            "0.0.0.0:8080",
            "--allow-remote-writes",
            "KANBANR_ALLOW_REMOTE_WRITES",
            "KANBANR_ALLOWED_HOSTS",
        ] {
            assert!(t.contains(want), "{want} missing from {t}");
        }
        assert_eq!(bind_refusal("0.0.0.0:8080", true, true), Ok(None));
        assert_eq!(bind_refusal("0.0.0.0:8080", false, false), Ok(None));
        assert_eq!(bind_refusal("127.0.0.1:8080", true, false), Ok(None));
    }

    #[test]
    fn the_acceptance_is_read_from_the_flag_or_the_variable() {
        for v in [None, Some(""), Some("0"), Some("false")] {
            assert_eq!(accepted_from_env(v), Ok(false), "{v:?}");
        }
        for v in [Some("1"), Some("TRUE"), Some("yes"), Some("on")] {
            assert_eq!(accepted_from_env(v), Ok(true), "{v:?}");
        }
    }

    #[test]
    fn an_unreadable_acceptance_value_is_refused() {
        let e = accepted_from_env(Some("maybe")).unwrap_err();
        assert!(
            e.contains("maybe") && e.contains("KANBANR_ALLOW_REMOTE_WRITES"),
            "{e}"
        );
    }

    #[test]
    fn an_unreadable_bind_is_refused_when_writes_are_on() {
        assert!(bind_refusal("nonsense", true, true).is_err());
        assert_eq!(bind_refusal("nonsense", false, false), Ok(None));
    }

    #[test]
    fn a_malformed_origin_is_refused_at_start() {
        for bad in [
            "board.example.com",
            "ftp://x",
            "https://x/",
            "https://x/path",
            "https://user@x",
            "https://x:notaport",
            "https://",
        ] {
            let e = parse_origins(bad).unwrap_err();
            assert!(e.contains(bad), "{e}");
        }
        assert_eq!(parse_origins(""), Ok(vec![]));
    }

    #[test]
    fn a_named_origin_matches_as_a_browser_writes_it() {
        let o = parse_origins("HTTPS://Board.Example.com:443, http://[::1]:9000").unwrap();
        assert_eq!(o, ["https://board.example.com", "http://[::1]:9000"]);
        let g = Guard::new("127.0.0.1:8080", &[]).with_origins(o);
        let t = g.token().to_string();
        let post = Method::POST;
        let ok = |origin: Option<&str>, token: Option<&str>| {
            g.write_ok(&post, origin, "127.0.0.1:8080", token)
        };
        assert!(ok(Some("https://board.example.com"), Some(&t)));
        assert!(!ok(Some("https://other.example.com"), Some(&t)));
        assert!(!ok(Some("null"), Some(&t)));
        assert!(!ok(Some("https://board.example.com"), None));
    }

    #[test]
    fn loopback_names_are_answered_on_any_port() {
        let g = Guard::new("0.0.0.0:8080", &[]);
        for h in [
            "localhost",
            "localhost:18080",
            "127.0.0.1:1",
            "127.0.0.1",
            "[::1]:18080",
            "[::1]",
            "LocalHost:9000",
        ] {
            assert!(g.host_ok(Some(h)), "{h}");
        }
    }

    #[test]
    fn a_foreign_name_is_still_refused_on_the_bound_port() {
        let g = Guard::new("127.0.0.1:8080", &[]);
        for h in [
            "attacker.example:8080",
            "attacker.example",
            "localhost.attacker.example:8080",
            "127.0.0.1.example:8080",
            "localhost:",
            "localhost:80a",
            "[::2]:8080",
        ] {
            assert!(!g.host_ok(Some(h)), "{h}");
        }
        assert!(!g.host_ok(None));
    }
}
