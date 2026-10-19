//! Single-writer daemon write path (FEAT-034, opt-in via `serve --allow-writes`).
//!
//! This is the SAME write recipe the CLI's local mode uses (`kanbanr-cli/src/backend.rs`): a write
//! goes through the shared `kanbanr_core::dispatch` (no second dispatch/write path), appends to the
//! project activity changelog, commits locally under the data repo's identity, and then pushes per
//! the debounce policy. The only difference is *who* runs it: when the daemon is write-enabled, all
//! mutations are serialized through this one process — but it still takes the same cross-process
//! advisory lock (`git::WRITE_LOCK_FILE`) so a CLI writing the same folder directly can't interleave
//! with the daemon. Direct CLI writes therefore remain valid whether or not the daemon is running;
//! the daemon is purely additive.
//!
//! Binds localhost, no auth — consistent with the read-only monitor.

use kanbanr_core::{Store, eventing, git};
use serde_json::Value;
use std::path::Path;

pub use kanbanr_core::git::PushPolicy;

/// Outcome of a daemon write: the response body, plus any push warnings to surface to the caller.
pub struct WriteOutcome {
    pub body: String,
    pub warnings: Vec<String>,
}

/// The data-route write: core's one write recipe (FEAT-176), the same the CLI runs. The daemon has
/// no HTTP client, so events are recorded and not delivered; webhook delivery stays the CLI's.
/// Returns the dispatcher's error text on failure (the caller maps it to an HTTP status).
pub fn write(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<WriteOutcome, String> {
    let applied = kanbanr_core::write::apply(
        store,
        data_dir,
        policy,
        method,
        path,
        body,
        &eventing::NullSender,
    )?;
    Ok(WriteOutcome {
        body: applied.body,
        warnings: applied.push_failures,
    })
}

/// Explicit sync: push pending local commits regardless of policy (the daemon's `/sync` route).
pub fn sync(data_dir: &Path) -> git::SyncOutcome {
    kanbanr_core::write::with_lock(data_dir, || git::push_pending(data_dir)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanbanr_core::dispatch;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kanbanr-srvwrite-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
        ));
        std::fs::create_dir_all(p.join("projects")).unwrap();
        // A board commits as someone (FEAT-128).
        git::ensure_repo_as(&p, Some(("Tester", "t@example.com")));
        p
    }

    /// FEAT-034 daemon write mode: a write routed through the server's write recipe applies the
    /// mutation (dispatch), persists it to the store, and commits it locally — all without any
    /// networking. Hermetic: no remote configured, default-debounce policy.
    #[test]
    fn daemon_write_applies_and_commits() {
        let dir = temp_dir("apply");
        let store = Store::new(dir.clone());
        let out = write(
            &store,
            &dir,
            PushPolicy::Debounce { every: 10 },
            "POST",
            "/projects",
            Some(&serde_json::json!({ "name": "demo" })),
        )
        .expect("daemon write should succeed");
        assert!(out.body.contains("demo"), "response echoes the new project");

        // The mutation is visible through the store (it was actually applied).
        let listing = dispatch::dispatch(&store, "GET", "/projects", None).unwrap();
        assert!(listing.contains("demo"), "project persisted to the store");

        // It produced a local commit, and (debounced) is marked unpushed rather than pushed now.
        let repo = git2::Repository::open(&dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        assert!(walk.count() >= 1, "write committed locally");
        assert!(git::has_unpushed(&dir), "debounced: marked unpushed");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// FEAT-034: an explicit daemon `sync` clears the pending marker (no remote -> no-op push).
    #[test]
    fn daemon_sync_clears_pending() {
        let dir = temp_dir("sync");
        let store = Store::new(dir.clone());
        write(
            &store,
            &dir,
            PushPolicy::Off,
            "POST",
            "/projects",
            Some(&serde_json::json!({ "name": "demo" })),
        )
        .unwrap();
        assert!(git::has_unpushed(&dir));
        let _ = sync(&dir);
        assert!(!git::has_unpushed(&dir), "sync cleared the marker");
        std::fs::remove_dir_all(&dir).ok();
    }
}
