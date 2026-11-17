//! The HTTP client for `katalogr api` (loopback, ADR-0014). It judges katalogr by the capability
//! names `GET /api/v1/meta` lists, never by a version number, and never errors for an absent or
//! older katalogr: that is a state to report. FTR-016 R-4, R-5, R-10, R-11.

use super::query::QuerySpec;
use super::{KatalogrState, Settings};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

pub use crate::position::PositionRef;
pub use crate::views::ViewQueryJson;

/// How long to wait for a connection to a loopback service. An absent katalogr is reported
/// within this (R-11 asks for 300 ms).
const CONNECT: Duration = Duration::from_millis(250);
/// How long `GET /api/v1/meta` may take to answer.
const META_READ: Duration = Duration::from_secs(1);
/// The most bytes of any answer that are read.
pub const MAX_BODY: u64 = 1024 * 1024;

pub struct Client {
    settings: Settings,
    meta_agent: ureq::Agent,
    view_agent: ureq::Agent,
}

/// What the daemon keeps of katalogr's view answer; unknown fields are dropped on read. `graph`
/// is kept as a `Value` and never sent to the monitor in v1. Named `KatalogrView`, not
/// `ViewAnswer`: kanbanr's own answer is `views::ViewAnswer` (FTR-003), with a typed graph; this
/// is the remote one as received.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KatalogrView {
    pub schema: u32,
    pub view_format: u32,
    pub exact: bool,
    #[serde(default)]
    pub view_id: Option<String>,
    pub positions: Vec<PositionRef>,
    pub query: ViewQueryJson,
    pub mermaid: String,
    #[serde(default)]
    pub graph: serde_json::Value,
    pub pele: String,
    #[serde(default)]
    pub stale: Option<bool>,
    #[serde(default)]
    pub pending: Option<u64>,
}

/// Why a view is not there.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ViewError {
    /// 409 `position-unavailable`: "no katalogr context holds <wanted>; it holds <available…>".
    PositionUnavailable {
        wanted: String,
        available: Vec<PositionRef>,
    },
    /// 404 unknown op or 400 unknown parameter: katalogr's message, as it said it.
    Refused { status: u16, said: String },
    /// Connection lost mid-request, a body over 1 MiB, or JSON that is not a view.
    Failed { said: String },
}

impl ViewError {
    /// One plain sentence, with no path in it (katalogr redacts its own messages).
    pub fn sentence(&self) -> String {
        match self {
            ViewError::PositionUnavailable { wanted, available } => {
                let held = if available.is_empty() {
                    "none".to_string()
                } else {
                    available
                        .iter()
                        .map(PositionRef::text)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!("no katalogr context holds {wanted}; it holds {held}")
            }
            ViewError::Refused { status, said } => {
                format!("katalogr refused the query ({status}): {said}")
            }
            ViewError::Failed { said } => said.clone(),
        }
    }
}

enum Got {
    Body(u16, Vec<u8>),
    Over,
    Down(String),
}

impl Client {
    pub fn new(settings: Settings) -> Client {
        let agent = |read: Duration| {
            ureq::AgentBuilder::new()
                .timeout_connect(CONNECT)
                .timeout_read(read)
                .redirects(0)
                .build()
        };
        let view_read = Duration::from_secs_f64(settings.timeout_secs + 2.0);
        Client {
            meta_agent: agent(META_READ),
            view_agent: agent(view_read),
            settings,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn get(&self, agent: &ureq::Agent, target: &str) -> Got {
        let url = format!("{}{target}", self.settings.url);
        let resp = match agent.get(&url).set("Accept", "application/json").call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Got::Down(t.kind().to_string()),
        };
        let status = resp.status();
        let mut body = Vec::new();
        if resp
            .into_reader()
            .take(MAX_BODY + 1)
            .read_to_end(&mut body)
            .is_err()
        {
            return Got::Down("the connection was lost while reading the answer".to_string());
        }
        if body.len() as u64 > MAX_BODY {
            return Got::Over;
        }
        Got::Body(status, body)
    }

    /// `GET /api/v1/meta`, judged by capability names. Never errors: every failure is a state.
    pub fn state(&self) -> KatalogrState {
        let url = self.settings.url.clone();
        let (status, body) = match self.get(&self.meta_agent, "/api/v1/meta") {
            Got::Body(s, b) => (s, b),
            Got::Down(said) => return KatalogrState::Absent { url, said },
            Got::Over => {
                return too_old(url, "its /api/v1/meta answer is far larger than katalogr's");
            }
        };
        if status != 200 {
            return too_old(url, &format!("it answers {status} at /api/v1/meta"));
        }
        let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return too_old(url, "its /api/v1/meta answer is not JSON");
        };
        let capabilities: Vec<String> = meta
            .get("capabilities")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let katalogr = meta
            .get("katalogr")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_string();
        let contract = meta
            .get("contract")
            .and_then(|c| c.as_u64())
            .map_or(0, |c| u32::try_from(c).unwrap_or(u32::MAX));
        if !capabilities.iter().any(|c| c == "views") {
            return KatalogrState::TooOld {
                url,
                katalogr,
                said: "it has no `views` capability".to_string(),
            };
        }
        KatalogrState::Ready {
            url,
            katalogr,
            contract,
            capabilities,
        }
    }

    /// `GET /api/v1/views/<op>?<args>[&at=<position>]&consistency=wait&timeout=<timeout_secs>`.
    pub fn view(&self, spec: &QuerySpec, at: Option<&str>) -> Result<KatalogrView, ViewError> {
        if !super::query::OPS.contains(&spec.op.as_str()) {
            return Err(ViewError::Refused {
                status: 404,
                said: format!("`{}` is not an operation", spec.op),
            });
        }
        let mut query = spec.query_string();
        let mut add = |pair: String| {
            if !query.is_empty() {
                query.push('&');
            }
            query.push_str(&pair);
        };
        if let Some(at) = at.filter(|a| !a.is_empty()) {
            add(format!("at={}", super::query::encode(at)));
        }
        add("consistency=wait".to_string());
        add(format!("timeout={}", self.settings.timeout_secs));
        let target = format!("/api/v1/views/{}?{query}", spec.op);
        let (status, body) = match self.get(&self.view_agent, &target) {
            Got::Body(s, b) => (s, b),
            Got::Down(why) => {
                return Err(ViewError::Failed {
                    said: format!("katalogr did not answer at {}: {why}", self.settings.url),
                });
            }
            Got::Over => {
                return Err(ViewError::Failed {
                    said: "katalogr's answer is over 1 MiB and was not read".to_string(),
                });
            }
        };
        match status {
            200 => {
                let view: KatalogrView =
                    serde_json::from_slice(&body).map_err(|_| ViewError::Failed {
                        said: "katalogr answered something that is not a view".to_string(),
                    })?;
                match (&view.view_id, view.exact) {
                    (Some(id), true) if !crate::position::is_view_id(id) => {
                        Err(ViewError::Failed {
                            said: "katalogr's view ID is not the contract's shape".to_string(),
                        })
                    }
                    (None, true) => Err(ViewError::Failed {
                        said: "katalogr's exact answer has no view ID".to_string(),
                    }),
                    _ => Ok(view),
                }
            }
            409 => {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                if v.get("error").and_then(|e| e.as_str()) == Some("position-unavailable") {
                    let wanted = v
                        .get("wanted")
                        .map(|w| match w.as_str() {
                            Some(s) => s.to_string(),
                            None => serde_json::from_value::<PositionRef>(w.clone())
                                .map_or_else(|_| w.to_string(), |p| p.text()),
                        })
                        .or_else(|| at.map(str::to_string))
                        .unwrap_or_default();
                    let available = v
                        .get("available")
                        .and_then(|a| serde_json::from_value(a.clone()).ok())
                        .unwrap_or_default();
                    Err(ViewError::PositionUnavailable { wanted, available })
                } else {
                    Err(ViewError::Failed {
                        said: "katalogr answered 409".to_string(),
                    })
                }
            }
            400 | 404 => {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let said = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map_or_else(|| format!("katalogr answered {status}"), str::to_string);
                Err(ViewError::Refused { status, said })
            }
            other => Err(ViewError::Failed {
                said: format!("katalogr answered {other}"),
            }),
        }
    }
}

fn too_old(url: String, said: &str) -> KatalogrState {
    KatalogrState::TooOld {
        url,
        katalogr: String::new(),
        said: said.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake;
    use super::super::query::{Kind, OPS, SPECS};
    use super::*;
    use std::path::Path;

    fn client(url: &str) -> Client {
        Client::new(Settings::from_env().with_url(url))
    }

    fn spec(line: &str) -> QuerySpec {
        QuerySpec::parse(line).unwrap()
    }

    fn no_path(sentence: &str) {
        for bad in ["/home", "/tmp", "/Users", "/var", "C:\\", "\\\\"] {
            assert!(!sentence.contains(bad), "{sentence:?} names a path");
        }
    }

    /// FTR-016 R-4: katalogr is judged by capability names; a high contract number and unknown
    /// fields change nothing.
    #[test]
    fn capabilities_are_read_by_name_and_unknown_fields_are_ignored() {
        let fake = fake::serve(&["views", "web-view", "future-thing"], &[], Duration::ZERO);
        let state = client(&fake.url).state();
        match &state {
            KatalogrState::Ready {
                contract,
                katalogr,
                capabilities,
                ..
            } => {
                assert_eq!(*contract, 99);
                assert_eq!(katalogr, "0.2.0-fake");
                assert!(capabilities.iter().any(|c| c == "future-thing"));
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(state.has("views") && state.has("web-view") && !state.has("impact"));

        // A higher contract but no `views`: too old, whatever the number says.
        let meta = r#"{"api":1,"contract":500,"schema":1,"katalogr":"9.9.9","capabilities":["impact","position"]}"#;
        let high = fake::serve_raw(200, meta);
        match client(&high.url).state() {
            KatalogrState::TooOld { katalogr, said, .. } => {
                assert_eq!(katalogr, "9.9.9");
                assert!(said.contains("views"), "{said}");
            }
            other => panic!("expected TooOld, got {other:?}"),
        }

        // A view body with fields kanbanr does not know parses.
        let body = r#"{"schema":1,"view_format":1,"view_id":"v1-0123456789abcdef0123456789abcdef",
            "exact":true,"positions":[{"repo":"app","commit":null,"dirty":"aa","extra":1}],
            "query":{"op":"structure","args":{}},"mermaid":"flowchart LR\n","graph":{},"pele":"1.7.0",
            "peers":[{"x":1}],"future":"field"}"#;
        let raw = fake::serve_raw(200, body);
        let view = client(&raw.url).view(&spec("structure"), None).unwrap();
        assert!(view.exact);
        assert_eq!(view.positions[0].repo, "app");
    }

    /// FTR-016 R-5: an absent or older katalogr is one plain sentence, never an error.
    #[test]
    fn an_absent_or_older_katalogr_is_reported_plainly() {
        let url = fake::closed_port_url();
        let state = client(&url).state();
        assert!(matches!(state, KatalogrState::Absent { .. }), "{state:?}");
        assert_eq!(
            state.sentence(),
            format!("katalogr is not running at {url}")
        );
        assert!(!state.has("views"));

        let no_meta = fake::serve_raw(404, r#"{"schema":1,"error":"not-found","message":"no"}"#);
        let state = client(&no_meta.url).state();
        assert!(matches!(state, KatalogrState::TooOld { .. }), "{state:?}");
        assert!(state.sentence().contains("too old"));
        assert!(state.sentence().contains(&no_meta.url));
        no_path(&state.sentence());

        let not_json = fake::serve_raw(200, "<html>hello</html>");
        assert!(matches!(
            client(&not_json.url).state(),
            KatalogrState::TooOld { .. }
        ));

        let no_views = fake::serve(&["impact", "position"], &[], Duration::ZERO);
        let state = client(&no_views.url).state();
        assert!(matches!(state, KatalogrState::TooOld { .. }), "{state:?}");
        assert!(state.sentence().contains("`views`"));
        no_path(&state.sentence());

        // Asking a view of an absent katalogr is a Failed sentence, not a panic.
        let err = client(&url).view(&spec("structure"), None).unwrap_err();
        assert!(matches!(err, ViewError::Failed { .. }), "{err:?}");
        no_path(&err.sentence());
    }

    /// FTR-016 R-3, R-5: the request carries the position, `consistency=wait`, the timeout and
    /// the arguments sorted; refusals keep katalogr's words.
    #[test]
    fn a_view_is_asked_with_its_position_and_refusals_keep_katalogrs_words() {
        let missing = "f".repeat(40);
        let fake = fake::serve(&["views"], &[&missing], Duration::ZERO);
        let c = client(&fake.url);
        let at = "a".repeat(40);
        let v = c
            .view(&spec("callers \"a b\" --max-nodes 5 --depth 2"), Some(&at))
            .unwrap();
        assert!(v.exact);
        assert_eq!(v.view_id.as_deref(), Some(fake::VIEW_ID));
        assert_eq!(v.positions[0].text(), at);
        assert_eq!(v.query.args["symbol"], "a b");
        let seen = fake.requests();
        assert_eq!(
            seen[0],
            format!(
                "/api/v1/views/callers?depth=2&max-nodes=5&symbol=a%20b&at={at}&consistency=wait&timeout=3"
            )
        );

        match c.view(&spec("structure"), Some(&missing)).unwrap_err() {
            ViewError::PositionUnavailable { wanted, available } => {
                assert_eq!(wanted, missing);
                assert_eq!(available.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        let err = c.view(&spec("structure"), Some(&missing)).unwrap_err();
        assert!(
            err.sentence()
                .starts_with(&format!("no katalogr context holds {missing}; it holds "))
        );

        // A katalogr that does not know an op says so.
        let old = fake::serve_raw(
            404,
            r#"{"schema":1,"error":"unknown-view","message":"unknown view `flow`"}"#,
        );
        let err = client(&old.url).view(&spec("flow f"), None).unwrap_err();
        assert_eq!(
            err,
            ViewError::Refused {
                status: 404,
                said: "unknown view `flow`".into()
            }
        );
        assert!(err.sentence().contains("unknown view `flow`"));

        // A body over the cap is not read; JSON that is not a view is Failed.
        let big = fake::serve_raw(200, &" ".repeat(MAX_BODY as usize + 10));
        let err = client(&big.url).view(&spec("structure"), None).unwrap_err();
        assert!(err.sentence().contains("1 MiB"), "{err:?}");
        let odd = fake::serve_raw(200, r#"{"hello":1}"#);
        assert!(matches!(
            client(&odd.url).view(&spec("structure"), None),
            Err(ViewError::Failed { .. })
        ));
        // An exact answer whose ID is not the contract's shape is refused.
        let bad_id = r#"{"schema":1,"view_format":1,"view_id":"nope","exact":true,"positions":[],
            "query":{"op":"structure","args":{}},"mermaid":"","pele":"1.7.0"}"#;
        let bad = fake::serve_raw(200, bad_id);
        assert!(matches!(
            client(&bad.url).view(&spec("structure"), None),
            Err(ViewError::Failed { .. })
        ));
    }

    fn shared() -> serde_json::Value {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/contract/views-v1.json");
        serde_json::from_slice(&std::fs::read(path).expect("read views-v1.json")).unwrap()
    }

    /// FTR-016 R-10: the shared file's two bodies read as a view answer, its `required` lists hold,
    /// and its catalogue is `OPS` with the grammar's tables.
    #[test]
    fn the_shared_views_file_parses_as_a_view_answer() {
        let file = shared();
        let keys = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap().to_string())
                .collect()
        };
        for (name, exact) in [("exact", true), ("not_exact", false)] {
            let body = &file[name];
            let view: KatalogrView = serde_json::from_value(body.clone())
                .unwrap_or_else(|e| panic!("{name} is not a view answer: {e}"));
            assert_eq!(view.exact, exact);
            assert_eq!(view.pele, "1.7.0");
            let obj = body.as_object().unwrap();
            for k in keys(&file["required"]["always"]) {
                assert!(obj.contains_key(&k), "{name} lacks {k}");
            }
            let extra = if exact {
                "when_exact"
            } else {
                "when_not_exact"
            };
            for k in keys(&file["required"][extra]) {
                assert!(obj.contains_key(&k), "{name} lacks {k}");
            }
            if exact {
                assert!(crate::position::is_view_id(
                    view.view_id.as_deref().unwrap()
                ));
            } else {
                assert_eq!(view.stale, Some(true));
                assert!(view.pending.is_some());
            }
        }

        // The catalogue is OPS, and each op's parameters are the grammar's tables.
        let views = file["catalogue"]["views"].as_array().unwrap();
        let mut ops: Vec<&str> = views.iter().map(|v| v["op"].as_str().unwrap()).collect();
        ops.sort_unstable();
        let mut ours = OPS.to_vec();
        ours.sort_unstable();
        assert_eq!(ops, ours, "the catalogue's ops and OPS differ");
        for v in views {
            let op = v["op"].as_str().unwrap();
            let spec = SPECS.iter().find(|s| s.op == op).unwrap();
            let params = v["params"].as_array().unwrap();
            let positionals: Vec<&str> = params
                .iter()
                .filter(|p| p["required"] == true)
                .map(|p| p["name"].as_str().unwrap())
                .collect();
            assert_eq!(positionals, spec.positionals, "{op} positionals");
            let flags: Vec<(&str, Kind)> = params
                .iter()
                .filter(|p| p["required"] != true)
                .map(|p| {
                    let kind = match p["type"].as_str().unwrap() {
                        "integer" => Kind::Integer,
                        "boolean" => Kind::Boolean,
                        "string" => Kind::Text,
                        "enum" => {
                            let values: Vec<&str> = p["values"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_str().unwrap())
                                .collect();
                            let ours = SPECS
                                .iter()
                                .find(|s| s.op == op)
                                .and_then(|s| {
                                    s.flags
                                        .iter()
                                        .find(|(n, _)| *n == p["name"].as_str().unwrap())
                                })
                                .map(|(_, k)| *k);
                            match ours {
                                Some(Kind::Enum(list)) => {
                                    assert_eq!(list, values.as_slice(), "{op} enum values");
                                    Kind::Enum(list)
                                }
                                other => panic!("{op}: {other:?} is not an enum"),
                            }
                        }
                        other => panic!("a parameter type kanbanr does not know: {other}"),
                    };
                    (p["name"].as_str().unwrap(), kind)
                })
                .collect();
            let mut want: Vec<(&str, Kind)> = spec.flags.to_vec();
            let mut got = flags;
            want.sort_by_key(|f| f.0);
            got.sort_by_key(|f| f.0);
            assert_eq!(got, want, "{op} flags");
        }
    }

    /// FTR-016 R-11: an absent katalogr is reported within the connect budget.
    #[test]
    fn an_absent_katalogr_is_detected_within_the_connect_budget() {
        let c = client(&fake::closed_port_url());
        let started = std::time::Instant::now();
        let state = c.state();
        let took = started.elapsed();
        assert!(matches!(state, KatalogrState::Absent { .. }));
        assert!(took < Duration::from_millis(300), "took {took:?}");
    }
}
