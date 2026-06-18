//! The CLI is a **local-only writer**: it operates on the data folder directly via `kanbanr-core`
//! (store + dispatch + git), with no server and no network. Sharing/centralization is handled by
//! git remotes; the view daemon (`kanbanr serve`) only reads the same folder.
//!
//! Every command speaks the same `(method, path, body) -> body` shape, so handlers don't care that
//! it's local. Data routes go through the shared `kanbanr_core::dispatch`; each write appends to the
//! project's activity changelog and commits the data repo under the configured identity.

use anyhow::{anyhow, Result};
use kanbanr_core::{activity, dispatch, git, Store};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Mutating HTTP-shaped methods the command handlers use (reads go through `get`).
#[derive(Clone, Copy)]
pub enum Method {
    Post,
    Put,
    Patch,
    Delete,
}

fn method_str(m: Method) -> &'static str {
    match m {
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Patch => "PATCH",
        Method::Delete => "DELETE",
    }
}

pub struct Backend {
    data_dir: PathBuf,
    store: Store,
}

impl Backend {
    pub fn new(data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(data_dir.join("projects"));
        let store = Store::new(data_dir.clone());
        Backend { data_dir, store }
    }

    /// A read.
    pub fn get(&self, path: &str) -> Result<String> {
        let bare = path.split('?').next().unwrap_or(path);
        if bare == "/auth/whoami" {
            return Ok(self.whoami_json());
        }
        if bare == "/remotes" {
            let remotes: Vec<_> = git::list_remotes(&self.data_dir)
                .into_iter()
                .map(|(name, url)| json!({ "name": name, "url": url }))
                .collect();
            return Ok(serde_json::to_string(&remotes)?);
        }
        if let Some(project) = activity_project(bare) {
            return Ok(serde_json::to_string(&activity::read(&self.data_dir, &project, 25))?);
        }
        dispatch::dispatch(&self.store, "GET", path, None).map_err(|e| anyhow!(e.to_string()))
    }

    /// Authenticated read — same as `get` in local mode.
    pub fn get_auth(&self, path: &str) -> Result<String> {
        self.get(path)
    }

    /// Run `f` while holding an exclusive, cross-process advisory lock on the data dir, so two
    /// writers (e.g. two VS Code sessions / agents) can't interleave a mutate→commit→sync sequence
    /// and diverge the git repo (FEAT-029). The lock is advisory between kanbanr processes; the OS
    /// releases it if the process dies. Reads (`get`) are intentionally not locked.
    fn with_write_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        use fs2::FileExt;
        let lock_path = self.data_dir.join(kanbanr_core::git::WRITE_LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            // A lock file's contents are irrelevant; never truncate (keeps lock semantics intact).
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| anyhow!("could not open write lock {}: {e}", lock_path.display()))?;
        file.lock_exclusive()
            .map_err(|e| anyhow!("could not acquire write lock {}: {e}", lock_path.display()))?;
        let result = f();
        let _ = file.unlock();
        result
    }

    /// A write: git remotes act on the local repo; data routes dispatch + log + commit. Serialized
    /// against other writers by an advisory lock on the data dir.
    pub fn write(&self, method: Method, path: &str, body: Option<Value>) -> Result<String> {
        self.with_write_lock(move || self.write_locked(method, path, body))
    }

    fn write_locked(&self, method: Method, path: &str, body: Option<Value>) -> Result<String> {
        let bare = path.split('?').next().unwrap_or(path);
        if bare == "/remotes" || bare.starts_with("/remotes/") {
            if let Some(name) = bare.strip_prefix("/remotes/") {
                git::remove_remote(&self.data_dir, name).map_err(|e| anyhow!(e))?;
                return Ok(String::new());
            }
            let b = body.unwrap_or(Value::Null);
            let name = b.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let url = b.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            git::add_remote(&self.data_dir, name, url).map_err(|e| anyhow!(e))?;
            return Ok(json!({ "name": name, "url": url }).to_string());
        }

        git::ensure_repo(&self.data_dir);
        let m = method_str(method);
        let out = dispatch::dispatch(&self.store, m, path, body.as_ref()).map_err(|e| anyhow!(e.to_string()))?;
        if dispatch::is_mutation(m) {
            let msg = dispatch::commit_message(m, path, body.as_ref());
            if let Some(project) = project_of(path) {
                let actor = git::identity(&self.data_dir).map(|(n, _)| n).unwrap_or_else(|| "kanbanr".into());
                activity::append(&self.data_dir, &project, &actor, &msg, item_of(path).as_deref());
            }
            git::commit_local(&self.data_dir, &msg);
            // Best-effort remote sync; surface any push/conflict so the user can resolve with
            // normal git (the change is already committed locally, so nothing is lost).
            for w in git::sync_all(&self.data_dir) {
                eprintln!("kanbanr: {w}");
            }
        }
        Ok(out)
    }

    /// Store a binary documentation asset (e.g. an image) directly as bytes, then commit. Used for
    /// files that aren't UTF-8 text and can't go through the JSON dispatch path. Serialized against
    /// other writers by the same advisory lock.
    pub fn write_doc_asset(&self, project: &str, rel: &str, bytes: &[u8]) -> Result<String> {
        self.with_write_lock(move || self.write_doc_asset_locked(project, rel, bytes))
    }

    fn write_doc_asset_locked(&self, project: &str, rel: &str, bytes: &[u8]) -> Result<String> {
        git::ensure_repo(&self.data_dir);
        let saved = self.store.write_doc_bytes(project, rel, bytes).map_err(|e| anyhow!(e.to_string()))?;
        let msg = format!("add document asset {saved}");
        let actor = git::identity(&self.data_dir).map(|(n, _)| n).unwrap_or_else(|| "kanbanr".into());
        activity::append(&self.data_dir, project, &actor, &msg, None);
        git::commit_local(&self.data_dir, &msg);
        for w in git::sync_all(&self.data_dir) {
            eprintln!("kanbanr: {w}");
        }
        Ok(saved)
    }

    fn whoami_json(&self) -> String {
        let (name, email) =
            git::identity(&self.data_dir).unwrap_or_else(|| ("kanbanr".into(), "kanbanr@local".into()));
        json!({ "user_id": name, "full_name": name, "email": email }).to_string()
    }
}

/// The project id from `/projects/<id>/...`.
fn project_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// `/projects/<id>/activity` -> the project id.
fn activity_project(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        ["projects", p, "activity"] => Some(p.to_string()),
        _ => None,
    }
}

/// The feature code from `/projects/<id>/features/<CODE>/...` (for tagging activity).
fn item_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next()?; // project id
    if segs.next()? != "features" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}
