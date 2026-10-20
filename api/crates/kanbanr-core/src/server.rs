//! What the view daemon answers about itself rather than about the board (FEAT-067, FEAT-177):
//! `GET /api/meta` and `GET /healthz`. The daemon builds these; they live here so the published
//! API description ([`crate::openapi`], feature `schema`) can describe them with the board's types.

use serde::Serialize;

/// What this daemon can do: `GET /api/meta`. The monitor asks before offering an action.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Meta {
    /// Whether the write routes are mounted (`serve --allow-writes`, FEAT-034).
    pub writes: bool,
    /// Who an approval recorded here would be attributed to (FEAT-077): the board's git
    /// identity. Null when there is none, and the monitor then refuses to offer the action.
    pub identity: Option<Identity>,
    /// The board schema this build understands (FEAT-072).
    pub schema_version: u32,
}

/// A git identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Meta {
    /// The answer for a daemon over `data_dir`.
    pub fn for_board(data_dir: &std::path::Path, writes: bool) -> Self {
        Meta {
            writes,
            identity: crate::git::identity(data_dir).map(|(name, email)| Identity { name, email }),
            schema_version: crate::config::CURRENT_SCHEMA_VERSION,
        }
    }
}

/// The liveness probe's answer: `GET /healthz`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Health {
    /// Always `ok`.
    pub status: String,
}

impl Health {
    pub fn ok() -> Self {
        Health {
            status: "ok".into(),
        }
    }
}
