//! The optional link to katalogr (FTR-016, ADR-0014).
//!
//! kanbanr works fully without katalogr. This module holds everything that knows it: the shared
//! query-block grammar (`query`), where katalogr is and how long to wait (`Settings`) and what a
//! client found (`KatalogrState`). kanbanr checks capability names, never version numbers.

pub mod client;
pub mod docviews;
#[doc(hidden)]
pub mod fake;
pub mod query;

use serde::Serialize;

const DEFAULT_URL: &str = "http://127.0.0.1:7878";
const DEFAULT_TIMEOUT_SECS: f64 = 3.0;
const DEFAULT_BIN: &str = "katalogr";
const DEFAULT_SNAPSHOT_SECS: u64 = 120;

/// Where katalogr is asked: `KANBANR_KATALOGR_URL` (default `http://127.0.0.1:7878`),
/// `KANBANR_KATALOGR_TIMEOUT_SECS` (default 3), `KANBANR_KATALOGR_BIN` (default `katalogr`, looked
/// up on PATH), `KANBANR_KATALOGR_SNAPSHOT_SECS` (default 120). A value that does not parse, or
/// is not above zero, is ignored for its default.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub url: String,
    pub timeout_secs: f64,
    pub bin: String,
    pub snapshot_secs: u64,
}

impl Settings {
    /// The settings the environment names. Read when called (not cached), so a test that sets a
    /// variable sees it.
    pub fn from_env() -> Settings {
        Settings::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Settings {
        let text = |name: &str, default: &str| {
            get(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        let timeout_secs = get("KANBANR_KATALOGR_TIMEOUT_SECS")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let snapshot_secs = get("KANBANR_KATALOGR_SNAPSHOT_SECS")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_SNAPSHOT_SECS);
        Settings {
            url: text("KANBANR_KATALOGR_URL", DEFAULT_URL),
            timeout_secs,
            bin: text("KANBANR_KATALOGR_BIN", DEFAULT_BIN),
            snapshot_secs,
        }
    }

    /// The same settings asking at `url` (a trailing `/` is dropped).
    pub fn with_url(mut self, url: &str) -> Settings {
        self.url = url.trim().trim_end_matches('/').to_string();
        self
    }
}

/// What a client found, as one sentence a page or a command can print as it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum KatalogrState {
    /// `GET /api/v1/meta` answered and `views` is among the capabilities.
    Ready {
        url: String,
        katalogr: String,
        contract: u32,
        capabilities: Vec<String>,
    },
    /// Connection refused or timed out: "katalogr is not running at <url>".
    Absent { url: String, said: String },
    /// `/api/v1/meta` missing, unparsable, or no `views`: "the katalogr at <url> is too old (…)".
    TooOld {
        url: String,
        katalogr: String,
        said: String,
    },
}

impl KatalogrState {
    /// Whether katalogr is ready and lists `capability` (a name, never a version).
    pub fn has(&self, capability: &str) -> bool {
        match self {
            KatalogrState::Ready { capabilities, .. } => {
                capabilities.iter().any(|c| c == capability)
            }
            _ => false,
        }
    }

    /// One plain sentence, with no path in it.
    pub fn sentence(&self) -> String {
        match self {
            KatalogrState::Ready {
                url,
                katalogr,
                contract,
                ..
            } => {
                let who = if katalogr.is_empty() {
                    "katalogr".to_string()
                } else {
                    format!("katalogr {katalogr}")
                };
                format!("{who} is running at {url} (contract {contract})")
            }
            KatalogrState::Absent { url, .. } => format!("katalogr is not running at {url}"),
            KatalogrState::TooOld { url, said, .. } => {
                format!("the katalogr at {url} is too old: {said}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FTR-016 R-12: the link is outside the 1.0 promise (ADR-0014): no public route names it and
    /// the stability page lists it under "Not covered". Passes once FTR-016 T9 edits that page.
    #[cfg(feature = "schema")]
    #[test]
    #[ignore = "passes once FTR-016 T9 edits docs/guide/project/stability.md; T9 removes this ignore"]
    fn the_link_is_outside_the_stability_promise() {
        for route in crate::openapi::public_routes() {
            assert!(
                !route.path.contains("katalogr") && !route.path.contains("docs/views"),
                "{} is a public route",
                route.path
            );
        }
        let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/guide/project/stability.md");
        let text = std::fs::read_to_string(page).expect("read stability.md");
        let not_covered = text
            .split("\n## ")
            .find(|s| s.starts_with("Not covered"))
            .expect("a Not covered section");
        for needle in [
            "The katalogr link",
            "/docs/views",
            "kanbanr katalogr status",
        ] {
            assert!(
                not_covered.contains(needle),
                "stability.md: Not covered does not name {needle:?}"
            );
        }
    }

    #[test]
    fn settings_come_from_the_environment_with_defaults() {
        let none = Settings::from_lookup(|_| None);
        assert_eq!(
            none,
            Settings {
                url: "http://127.0.0.1:7878".into(),
                timeout_secs: 3.0,
                bin: "katalogr".into(),
                snapshot_secs: 120
            }
        );
        let set = Settings::from_lookup(|n| match n {
            "KANBANR_KATALOGR_URL" => Some("http://127.0.0.1:9/".into()),
            "KANBANR_KATALOGR_TIMEOUT_SECS" => Some("0.5".into()),
            "KANBANR_KATALOGR_BIN" => Some("/opt/k".into()),
            "KANBANR_KATALOGR_SNAPSHOT_SECS" => Some("0".into()),
            _ => None,
        });
        assert_eq!(set.timeout_secs, 0.5);
        assert_eq!(set.bin, "/opt/k");
        assert_eq!(set.snapshot_secs, 120, "zero falls back to the default");
        assert_eq!(set.with_url("http://h:1/").url, "http://h:1");
    }

    #[test]
    fn states_read_as_one_sentence_and_answer_by_capability_name() {
        let ready = KatalogrState::Ready {
            url: "http://x:1".into(),
            katalogr: "0.2.0".into(),
            contract: 99,
            capabilities: vec!["views".into(), "web-view".into()],
        };
        assert!(ready.has("views") && ready.has("web-view") && !ready.has("impact"));
        let absent = KatalogrState::Absent {
            url: "http://x:1".into(),
            said: "refused".into(),
        };
        assert_eq!(absent.sentence(), "katalogr is not running at http://x:1");
        assert!(!absent.has("views"));
        let old = KatalogrState::TooOld {
            url: "http://x:1".into(),
            katalogr: String::new(),
            said: "it has no `views` capability".into(),
        };
        assert_eq!(
            old.sentence(),
            "the katalogr at http://x:1 is too old: it has no `views` capability"
        );
    }
}
