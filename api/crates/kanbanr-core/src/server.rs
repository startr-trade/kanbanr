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
    /// The kanbanr version this daemon runs (FTR-036).
    pub version: String,
    /// The commit this daemon was built from, or `unknown`.
    pub commit: String,
    /// Whether this daemon was built from a modified tree.
    pub dirty: bool,
    /// The newest kanbanr that wrote the board, from its `writer.yaml`; null when none is recorded.
    pub board_writer: Option<BoardWriter>,
    /// Whether that writer is newer than this daemon: the monitor then warns.
    pub writer_newer: bool,
}

/// The board's writer record, as the monitor shows it (FTR-036).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoardWriter {
    pub version: String,
    pub commit: String,
    pub at: String,
    /// The writer was built from a modified tree.
    pub dirty: bool,
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
    ///
    /// `code_repo` is the project's code repository, which lets a same-version writer be told
    /// apart by commit ancestry (FTR-036); without it only a higher version counts as newer.
    pub fn for_board(
        data_dir: &std::path::Path,
        writes: bool,
        code_repo: Option<&std::path::Path>,
    ) -> Self {
        let running = crate::writer::running();
        let recorded = crate::writer::recorded(data_dir);
        let writer_newer = recorded.as_ref().is_some_and(|w| {
            let descends = code_repo.and_then(|r| {
                crate::writer::descends(
                    r,
                    crate::writer::base_commit(&running.commit),
                    crate::writer::base_commit(&w.commit),
                )
            });
            crate::writer::newer(&running, w, descends)
        });
        Meta {
            writes,
            identity: crate::git::identity(data_dir).map(|(name, email)| Identity { name, email }),
            schema_version: crate::config::CURRENT_SCHEMA_VERSION,
            version: running.version,
            dirty: crate::writer::is_dirty(&running.commit),
            commit: running.commit,
            board_writer: recorded.map(|w| BoardWriter {
                version: w.version,
                commit: w.commit,
                at: w.at,
                dirty: w.dirty,
            }),
            writer_newer,
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
