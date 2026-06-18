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

use kanbanr_core::{activity, dispatch, git, Store};
use serde_json::Value;
use std::path::Path;

/// Push policy for the daemon, mirroring the CLI's (`KANBANR_PUSH`). Default debounced so the
/// network stays off the per-write hot path; commits are always local-first.
#[derive(Clone, Copy, Debug)]
pub enum PushPolicy {
    Auto,
    Debounce { every: u32 },
    Off,
}

const DEBOUNCE_DEFAULT_EVERY: u32 = 10;

impl PushPolicy {
    pub fn from_env() -> Self {
        match std::env::var("KANBANR_PUSH").ok().as_deref() {
            Some("auto") => PushPolicy::Auto,
            Some("off") => PushPolicy::Off,
            Some(s) if s.starts_with("debounce") => {
                let every = s
                    .split(':')
                    .nth(1)
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n > 0)
                    .unwrap_or(DEBOUNCE_DEFAULT_EVERY);
                PushPolicy::Debounce { every }
            }
            _ => PushPolicy::Debounce {
                every: DEBOUNCE_DEFAULT_EVERY,
            },
        }
    }
}

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

/// Apply the push policy after a successful local commit (mirrors the CLI). The commit is already
/// durable, so deferring a push never loses work. Returns push warnings (empty unless a push ran).
fn after_commit(
    data_dir: &Path,
    policy: PushPolicy,
    pending: &std::sync::atomic::AtomicU32,
) -> Vec<String> {
    use std::sync::atomic::Ordering;
    match policy {
        PushPolicy::Auto => push_now(data_dir, pending),
        PushPolicy::Off => {
            git::mark_unpushed(data_dir);
            Vec::new()
        }
        PushPolicy::Debounce { every } => {
            git::mark_unpushed(data_dir);
            let n = pending.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= every {
                push_now(data_dir, pending)
            } else {
                Vec::new()
            }
        }
    }
}

fn push_now(data_dir: &Path, pending: &std::sync::atomic::AtomicU32) -> Vec<String> {
    let warnings = git::sync_all(data_dir);
    git::clear_unpushed(data_dir);
    pending.store(0, std::sync::atomic::Ordering::SeqCst);
    warnings
}

/// The data-route write recipe: dispatch + activity + local commit + debounced push, serialized by
/// the advisory write lock. `method` is one of POST/PUT/PATCH/DELETE. This is the daemon's only
/// mutation entry point and it reuses `dispatch::dispatch` exactly like the CLI. Returns the
/// dispatcher's error string on failure (the caller maps it to an HTTP status).
pub fn write(
    store: &Store,
    data_dir: &Path,
    pending: &std::sync::atomic::AtomicU32,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<WriteOutcome, String> {
    with_write_lock(data_dir, || {
        git::ensure_repo(data_dir);
        let out = dispatch::dispatch(store, method, path, body).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        if dispatch::is_mutation(method) {
            let msg = dispatch::commit_message(method, path, body);
            if let Some(project) = project_of(path) {
                let actor = git::identity(data_dir)
                    .map(|(n, _)| n)
                    .unwrap_or_else(|| "kanbanr".into());
                activity::append(data_dir, &project, &actor, &msg, item_of(path).as_deref());
            }
            if git::commit_local(data_dir, &msg) {
                warnings = after_commit(data_dir, policy, pending);
            }
        }
        Ok(WriteOutcome {
            body: out,
            warnings,
        })
    })
    .map_err(|e| format!("write lock: {e}"))?
}

/// Explicit sync: push pending local commits regardless of policy (the daemon's `/sync` route).
pub fn sync(data_dir: &Path, pending: &std::sync::atomic::AtomicU32) -> Vec<String> {
    with_write_lock(data_dir, || push_now(data_dir, pending)).unwrap_or_default()
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
    use std::sync::atomic::AtomicU32;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kanbanr-srvwrite-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
        ));
        std::fs::create_dir_all(p.join("projects")).unwrap();
        p
    }

    /// FEAT-034 daemon write mode: a write routed through the server's write recipe applies the
    /// mutation (dispatch), persists it to the store, and commits it locally — all without any
    /// networking. Hermetic: no remote configured, default-debounce policy.
    #[test]
    fn daemon_write_applies_and_commits() {
        let dir = temp_dir("apply");
        let store = Store::new(dir.clone());
        let pending = AtomicU32::new(0);

        let out = write(
            &store,
            &dir,
            &pending,
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
        let pending = AtomicU32::new(0);
        write(
            &store,
            &dir,
            &pending,
            PushPolicy::Off,
            "POST",
            "/projects",
            Some(&serde_json::json!({ "name": "demo" })),
        )
        .unwrap();
        assert!(git::has_unpushed(&dir));
        let _ = sync(&dir, &pending);
        assert!(!git::has_unpushed(&dir), "sync cleared the marker");
        std::fs::remove_dir_all(&dir).ok();
    }
}
