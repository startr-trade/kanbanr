//! The CLI is a **local-only writer**: it operates on the data folder directly via `kanbanr-core`
//! (store + dispatch + git), with no server and no network. Sharing/centralization is handled by
//! git remotes; the view daemon (`kanbanr serve`) only reads the same folder.
//!
//! Every command speaks the same `(method, path, body) -> body` shape, so handlers don't care that
//! it's local. Data routes go through the shared `kanbanr_core::dispatch`; each write appends to the
//! project's activity changelog and commits the data repo under the configured identity.
//!
//! ## Push policy (FEAT-034 — debounced push)
//! Historically every write did a network pull+push (`git::sync_all`) *while holding the write
//! lock*, putting the network on the hot path of every mutation. That is now configurable via
//! [`PushPolicy`] (env `KANBANR_PUSH`), and the default is **debounced**:
//!   - `commit_local` always runs per write — commits are **local-first**, so nothing is ever lost
//!     even if the remote is unreachable.
//!   - The network push is batched off the hot path: a write marks the repo "unpushed" and only
//!     pushes when a debounce threshold is crossed (≥ K unpushed commits), or never automatically
//!     (`off`), or always (`auto`, the legacy behaviour). An explicit `kanbanr sync` always pushes.
//!
//! Push warnings/conflicts are surfaced exactly as before — `git::sync_all` returns per-remote
//! guidance and we print it; the change is already committed locally so it is safe to defer.

use anyhow::{anyhow, Result};
use kanbanr_core::{activity, dispatch, eventing, git, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// Best-effort webhook delivery for eventing (FEAT-036), backed by the CLI's `ureq` dependency.
/// Failures (unreachable endpoint, non-2xx, timeout) are swallowed — eventing must never fail or
/// slow a write. A short timeout keeps a slow webhook off the (already-committed) write's tail.
struct UreqSender;

impl eventing::WebhookSender for UreqSender {
    fn post(&self, url: &str, body: &str) {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(3))
            .build();
        if let Err(e) = agent
            .post(url)
            .set("content-type", "application/json")
            .send_string(body)
        {
            eprintln!("kanbanr: webhook delivery to {url} failed: {e}");
        }
    }
}

/// When the local-first commit gets pushed to remotes (FEAT-034). Selected by `KANBANR_PUSH`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PushPolicy {
    /// Push on every write, synchronously (the pre-FEAT-034 behaviour). `KANBANR_PUSH=auto`.
    Auto,
    /// Commit locally every write; push only once a threshold of unpushed commits accumulates, so
    /// the network stays off most writes. The default. `KANBANR_PUSH=debounce` (or unset).
    Debounce { every: u32 },
    /// Never push automatically — only an explicit `kanbanr sync` pushes. `KANBANR_PUSH=off`.
    Off,
}

/// Default number of unpushed commits that triggers a debounced push.
const DEBOUNCE_DEFAULT_EVERY: u32 = 10;

impl PushPolicy {
    /// Resolve the policy from `KANBANR_PUSH` (`auto` | `off` | `debounce[:N]`; default debounce).
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
    push: PushPolicy,
    /// Unpushed commits accumulated by *this* process since its last push (debounce counter). The
    /// durable cross-process signal is the on-disk `git::UNPUSHED_FILE` marker; this is just the
    /// in-process tally that decides when a debounce tick fires.
    pending: std::cell::Cell<u32>,
    /// Set while an issue-mirror sync runs, so the sync's own write-back doesn't sync again.
    mirroring: std::cell::Cell<bool>,
}

impl Backend {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_policy(data_dir, PushPolicy::from_env())
    }

    /// Construct a backend with an explicit push policy (used by tests and the daemon).
    pub fn with_policy(data_dir: PathBuf, push: PushPolicy) -> Self {
        let _ = std::fs::create_dir_all(data_dir.join("projects"));
        let store = Store::new(data_dir.clone());
        Backend {
            data_dir,
            store,
            push,
            pending: std::cell::Cell::new(0),
            mirroring: std::cell::Cell::new(false),
        }
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
            return Ok(serde_json::to_string(&activity::read(
                &self.data_dir,
                &project,
                25,
            ))?);
        }
        if let Some(project) = events_project(bare) {
            let limit = query_param(path, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(25);
            return Ok(serde_json::to_string(&eventing::read(
                &self.data_dir,
                &project,
                limit,
            ))?);
        }
        dispatch::dispatch(&self.store, "GET", path, None).map_err(|e| anyhow!(e.to_string()))
    }

    /// Run a mutating route as a preview: dispatch only, with no lock, activity entry or commit.
    /// Only meaningful for routes that honor a dry run (the batch route with `dry_run: true`).
    pub fn preview(&self, method: Method, path: &str, body: Option<Value>) -> Result<String> {
        dispatch::dispatch(&self.store, method_str(method), path, body.as_ref())
            .map_err(|e| anyhow!(e.to_string()))
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
            // The lock file holds no content (advisory locking only); never truncate it.
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
        let dry_run = dispatch::is_dry_run(body.as_ref());
        let out = self.with_write_lock(move || self.write_locked(method, path, body))?;
        if let (false, Some(project)) = (dry_run, project_of(path)) {
            self.auto_mirror(&project);
        }
        Ok(out)
    }

    /// Turn the automatic issue mirror off for this backend (tests must never reach GitHub).
    #[cfg(test)]
    pub fn disable_auto_mirror(&self) {
        self.mirroring.set(true);
    }

    /// Run `f` with the automatic issue mirror switched off (for the mirror's own writes).
    pub fn with_mirror_suppressed<T>(&self, f: impl FnOnce() -> T) -> T {
        let was = self.mirroring.replace(true);
        let out = f();
        self.mirroring.set(was);
        out
    }

    /// After a successful write, keep the project's mirrored issues in step (FEAT-043). Runs only
    /// when the project has the mirror enabled, outside the write lock, and never fails the write:
    /// problems are reported on stderr and a later `kanbanr mirror sync` catches up. Off for a
    /// session with `KANBANR_MIRROR=off`.
    fn auto_mirror(&self, project: &str) {
        if self.mirroring.get() || std::env::var("KANBANR_MIRROR").as_deref() == Ok("off") {
            return;
        }
        let config = self
            .data_dir
            .join("projects")
            .join(project)
            .join(kanbanr_core::mirror::MIRROR_FILE);
        if !config.is_file() {
            return;
        }
        match crate::mirror::sync(self, &crate::mirror::GhCli::new(), project, false) {
            Ok(out) => {
                for (code, n) in &out.created {
                    eprintln!("kanbanr: mirrored {code} to {}#{n} (created)", out.repo);
                }
                for (code, n) in &out.updated {
                    eprintln!("kanbanr: mirrored {code} to {}#{n} (updated)", out.repo);
                }
                for (code, e) in &out.failed {
                    eprintln!("kanbanr: could not mirror {code}: {e}");
                }
            }
            Err(e) => eprintln!(
                "kanbanr: issue mirror skipped: {e} (run `kanbanr mirror sync` once that's fixed)"
            ),
        }
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
        let out = dispatch::dispatch(&self.store, m, path, body.as_ref())
            .map_err(|e| anyhow!(e.to_string()))?;
        if dispatch::is_mutation(m) && !dispatch::is_dry_run(body.as_ref()) {
            let msg = dispatch::commit_message(m, path, body.as_ref());
            if let Some(project) = project_of(path) {
                let actor = git::identity(&self.data_dir)
                    .map(|(n, _)| n)
                    .unwrap_or_else(|| "kanbanr".into());
                activity::append(
                    &self.data_dir,
                    &project,
                    &actor,
                    &msg,
                    item_of(path).as_deref(),
                );
            }
            if git::commit_local(&self.data_dir, &msg) {
                self.after_commit();
            }
            // Eventing (FEAT-036): an additive, best-effort, opt-in tail step. Runs only after the
            // mutation is already durable, never returns an error into the write path, and does
            // nothing observable (beyond the local events log) unless a webhook is configured.
            eventing::emit(&self.store, &self.data_dir, m, path, &out, &UreqSender);
        }
        Ok(out)
    }

    /// After a successful local commit, apply the push policy (FEAT-034). In `Auto` we push
    /// immediately (legacy); otherwise we mark the repo "unpushed" and only push once enough
    /// commits have accumulated (`Debounce`) — or never automatically (`Off`). The commit is
    /// already durable locally, so deferring the push never loses work.
    fn after_commit(&self) {
        match self.push {
            PushPolicy::Auto => self.push_now(),
            PushPolicy::Off => git::mark_unpushed(&self.data_dir),
            PushPolicy::Debounce { every } => {
                git::mark_unpushed(&self.data_dir);
                let n = self.pending.get() + 1;
                if n >= every {
                    self.push_now();
                } else {
                    self.pending.set(n);
                }
            }
        }
    }

    /// Push to every remote now (pull+push via `git::sync_all`), surfacing per-remote
    /// warnings/conflicts exactly as before, then clear the debounce state. Best-effort: the change
    /// is already committed locally, so a failed push only defers, it never loses work.
    fn push_now(&self) {
        for w in git::sync_all(&self.data_dir) {
            eprintln!("kanbanr: {w}");
        }
        git::clear_unpushed(&self.data_dir);
        self.pending.set(0);
    }

    /// Explicit `kanbanr sync`: push any local commits to remotes regardless of policy. Holds the
    /// write lock so it can't race a concurrent write's commit. Returns the number of remotes and
    /// whether there was anything pending, for a friendly CLI message. (FEAT-034)
    pub fn sync(&self) -> Result<bool> {
        self.with_write_lock(|| {
            let had_pending = git::has_unpushed(&self.data_dir);
            self.push_now();
            Ok(had_pending)
        })
    }

    /// Store a binary documentation asset (e.g. an image) directly as bytes, then commit. Used for
    /// files that aren't UTF-8 text and can't go through the JSON dispatch path. Serialized against
    /// other writers by the same advisory lock.
    pub fn write_doc_asset(&self, project: &str, rel: &str, bytes: &[u8]) -> Result<String> {
        self.with_write_lock(move || self.write_doc_asset_locked(project, rel, bytes))
    }

    fn write_doc_asset_locked(&self, project: &str, rel: &str, bytes: &[u8]) -> Result<String> {
        git::ensure_repo(&self.data_dir);
        let saved = self
            .store
            .write_doc_bytes(project, rel, bytes)
            .map_err(|e| anyhow!(e.to_string()))?;
        let msg = format!("add document asset {saved}");
        let actor = git::identity(&self.data_dir)
            .map(|(n, _)| n)
            .unwrap_or_else(|| "kanbanr".into());
        activity::append(&self.data_dir, project, &actor, &msg, None);
        if git::commit_local(&self.data_dir, &msg) {
            self.after_commit();
        }
        Ok(saved)
    }

    /// Emit a sample notification event for the given project (FEAT-036 `events test`): append it
    /// to the events log and POST it to any configured webhook, best-effort. Returns whether any
    /// webhook endpoint was configured (so the caller can tell the user delivery actually ran).
    pub fn emit_test_event(&self, project: &str) -> bool {
        let event = eventing::Event::new(
            project,
            eventing::EventKind::FeatureMoved,
            None,
            "test event from `kanbanr events test`",
            json!({ "test": true }),
        );
        eventing::append(&self.data_dir, project, &event);
        let config = eventing::WebhookConfig::load(&self.data_dir);
        eventing::deliver(&config, &UreqSender, &event);
        config.is_enabled()
    }

    /// List the data folder's project ids (for portfolio-wide maintenance like `index`).
    pub fn list_projects(&self) -> Result<Vec<String>> {
        self.store
            .list_projects()
            .map_err(|e| anyhow!(e.to_string()))
    }

    /// Rebuild the per-project `index.yaml` cache for each given project, then commit the data repo
    /// (a maintenance write). Serialized against other writers by the data-dir advisory lock. The
    /// index is a derivable cache, so this never touches the source-of-truth feature files. (FEAT-033)
    pub fn rebuild_index(&self, ids: &[String]) -> Result<()> {
        self.with_write_lock(move || {
            git::ensure_repo(&self.data_dir);
            for id in ids {
                self.store
                    .rebuild_index(id)
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
            let msg = if ids.len() == 1 {
                format!("rebuild index for {}", ids[0])
            } else {
                format!("rebuild index for {} projects", ids.len())
            };
            if git::commit_local(&self.data_dir, &msg) {
                self.after_commit();
            }
            Ok(())
        })
    }

    fn whoami_json(&self) -> String {
        let (name, email) = git::identity(&self.data_dir)
            .unwrap_or_else(|| ("kanbanr".into(), "kanbanr@local".into()));
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

/// `/projects/<id>/events` -> the project id (FEAT-036 events-log read route).
fn events_project(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        ["projects", p, "events"] => Some(p.to_string()),
        _ => None,
    }
}

/// Read a single query-string parameter from a full path (e.g. `?limit=50`).
fn query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn head_count(dir: &std::path::Path) -> usize {
        let repo = git2::Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.count()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kanbanr-be-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// FEAT-034: with the default debounce and NO reachable remote configured, a write commits
    /// locally (the change is durable) and never blocks/loses on the network; it is marked unpushed.
    #[test]
    fn debounced_write_commits_locally_without_a_remote() {
        let dir = temp_dir("debounce");
        let be = Backend::with_policy(dir.clone(), PushPolicy::Debounce { every: 10 });

        let before = {
            git::ensure_repo(&dir);
            head_count(&dir)
        };
        be.write(Method::Post, "/projects", Some(json!({ "name": "demo" })))
            .expect("create project");

        // The write produced a local commit even though there is no remote to push to.
        assert_eq!(head_count(&dir), before + 1, "write should commit locally");
        // And it recorded that there is something to push later (debounced, not pushed now).
        assert!(
            git::has_unpushed(&dir),
            "debounced write should mark the repo unpushed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// FEAT-034: an explicit `sync` runs the push path and clears the debounce marker. With no
    /// remote configured, `sync_all` is a no-op (no warnings), so this is hermetic — no network.
    #[test]
    fn explicit_sync_clears_pending_state() {
        let dir = temp_dir("sync");
        let be = Backend::with_policy(dir.clone(), PushPolicy::Off);
        be.write(Method::Post, "/projects", Some(json!({ "name": "demo" })))
            .expect("create project");
        assert!(git::has_unpushed(&dir), "Off policy marks unpushed");

        let _ = be.sync().expect("sync");
        assert!(
            !git::has_unpushed(&dir),
            "sync should clear the unpushed marker"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// FEAT-034: `auto` policy pushes per write, so it never leaves a lingering unpushed marker
    /// (push_now clears it). Hermetic: no remote means sync_all is a no-op.
    #[test]
    fn auto_policy_does_not_defer() {
        let dir = temp_dir("auto");
        let be = Backend::with_policy(dir.clone(), PushPolicy::Auto);
        be.write(Method::Post, "/projects", Some(json!({ "name": "demo" })))
            .expect("create project");
        assert!(
            !git::has_unpushed(&dir),
            "auto policy pushes immediately and clears the marker"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
