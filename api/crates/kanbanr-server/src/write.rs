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

use kanbanr_core::{Store, activity, dispatch, eventing, git};
use serde_json::Value;
use std::path::Path;

pub use kanbanr_core::git::PushPolicy;

/// Outcome of a daemon write: the response body, plus any push warnings to surface to the caller.
pub struct WriteOutcome {
    pub body: String,
    pub warnings: Vec<String>,
}

/// Hold the cross-process advisory write lock on `data_dir` while running `f`, identical to the
/// CLI's `with_write_lock` (FEAT-029), so the daemon and a direct CLI writer can't interleave.
fn with_write_lock<T>(data_dir: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
    use fs2::FileExt;
    let lock_path = data_dir.join(git::WRITE_LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    file.lock_exclusive()?;
    let result = f();
    let _ = file.unlock();
    Ok(result)
}

/// The data-route write recipe: dispatch + activity + local commit + debounced push, serialized by
/// the advisory write lock. `method` is one of POST/PUT/PATCH/DELETE. This is the daemon's only
/// mutation entry point and it reuses `dispatch::dispatch` exactly like the CLI. Returns the
/// dispatcher's error string on failure (the caller maps it to an HTTP status).
pub fn write(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<WriteOutcome, String> {
    with_write_lock(data_dir, || {
        git::ensure_repo(data_dir);
        // Refused before anything changes on disk when nobody can author the commit (FEAT-128).
        if dispatch::is_mutation(method) && !dispatch::is_dry_run(body) {
            git::require_identity(data_dir)?;
        }
        let out = dispatch::dispatch(store, method, path, body).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        if dispatch::is_mutation(method) && !dispatch::is_dry_run(body) {
            let msg = dispatch::commit_message(method, path, body);
            if let Some(project) = project_of(path) {
                let actor = git::identity(data_dir)
                    .map(|(n, _)| n)
                    .unwrap_or_else(|| "kanbanr".into());
                activity::append(data_dir, &project, &actor, &msg, item_of(path).as_deref());
            }
            // Eventing (FEAT-036): additive and best-effort. The LOG is written before the commit
            // so the entry lands in the same commit as the change it describes (FEAT-059) — the CLI
            // path does the same, and the daemon must not be the one that leaves the board dirty.
            // The daemon has no HTTP client, so it records only; webhook *delivery* stays the CLI's
            // responsibility for now, and a daemon-side sender slots in after the commit.
            let events = eventing::record(store, data_dir, method, path, &out);
            if git::commit_local(data_dir, &msg) {
                // The board's policy, as the CLI applies it (FEAT-142): one rule, in core.
                if let Some(outcome) = git::after_commit(data_dir, policy) {
                    warnings = outcome.failures;
                }
            }
            eventing::deliver_all(data_dir, &events, &eventing::NullSender);
        }
        Ok(WriteOutcome {
            body: out,
            warnings,
        })
    })
    .map_err(|e| format!("write lock: {e}"))?
}

/// Explicit sync: push pending local commits regardless of policy (the daemon's `/sync` route).
pub fn sync(data_dir: &Path) -> git::SyncOutcome {
    with_write_lock(data_dir, || git::push_pending(data_dir)).unwrap_or_default()
}

/// The project id from `/projects/<id>/...` (mirrors the CLI helper).
fn project_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// The feature code from `/projects/<id>/features/<CODE>/...` (mirrors the CLI helper).
fn item_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next()?;
    if segs.next()? != "features" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
