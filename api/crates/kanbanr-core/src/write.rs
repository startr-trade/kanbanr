//! The one write recipe (FEAT-176): every change to a board goes through here, from the CLI and
//! from the writes-enabled daemon alike.
//!
//! Under the cross-process advisory lock (FEAT-029): make sure the board is a repository; refuse
//! before anything changes when nobody can author the commit (FEAT-128); dispatch; append the
//! activity entry and record events *before* the commit, so they land in the same commit as the
//! change they describe (FEAT-059); commit; apply the board's push policy (FEAT-142); and only then
//! deliver events, which may touch the network and must never delay or fail a durable write.
//!
//! This used to exist twice — once in the CLI's backend and once in the daemon — and each copy had
//! to be kept in step with the other by hand.

use crate::git::{self, PushPolicy};
use crate::{Store, activity, dispatch, eventing};
use serde_json::Value;
use std::path::Path;

/// What a write did: the dispatcher's response, and any push that ran and failed (the commit is
/// durable either way; a failed push leaves the board marked unpushed).
#[derive(Debug, Default)]
pub struct Applied {
    pub body: String,
    pub push_failures: Vec<String>,
}

/// Run `f` holding the board's exclusive advisory write lock (FEAT-029). The OS releases it if the
/// process dies. Reads are intentionally not locked.
pub fn with_lock<T>(data_dir: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
    use fs2::FileExt;
    let lock_path = data_dir.join(git::WRITE_LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        // The lock file holds no content; never truncate it.
        .truncate(false)
        .open(&lock_path)?;
    file.lock_exclusive()?;
    let result = f();
    let _ = file.unlock();
    Ok(result)
}

/// Apply one data-route write: the whole recipe, under the lock. `method` is POST/PUT/PATCH/DELETE;
/// a dry run (a batch with `dry_run: true`) dispatches and records nothing. Errors are the
/// dispatcher's, or the lock's, as text for the front end to render.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&Path>,
    sender: &dyn eventing::WebhookSender,
) -> Result<Applied, String> {
    with_lock(data_dir, || {
        git::ensure_repo(data_dir);
        let records = dispatch::is_mutation(method) && !dispatch::is_dry_run(body);
        if records {
            git::require_identity(data_dir)?;
        }
        let out =
            dispatch::dispatch_in(store, method, path, body, repo).map_err(|e| e.to_string())?;
        let mut applied = Applied {
            body: out,
            ..Default::default()
        };
        if records {
            let msg = dispatch::commit_message(method, path, body);
            if let Some(project) = project_of(path) {
                let actor = git::identity(data_dir)
                    .map(|(n, _)| n)
                    .unwrap_or_else(|| "kanbanr".into());
                activity::append(data_dir, &project, &actor, &msg, item_of(path).as_deref());
            }
            let events = eventing::record(store, data_dir, method, path, &applied.body);
            if git::commit_local(data_dir, &msg)
                && let Some(outcome) = git::after_commit(data_dir, policy)
            {
                applied.push_failures = outcome.failures;
            }
            eventing::deliver_all(data_dir, &events, sender);
        }
        Ok(applied)
    })
    .map_err(|e| format!("write lock: {e}"))?
}

/// The project id from `/projects/<id>/...`.
pub fn project_of(path: &str) -> Option<String> {
    let bare = path.split('?').next().unwrap_or(path);
    let mut segs = bare.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// The feature code from `/projects/<id>/features/<CODE>/...`, for tagging activity.
pub fn item_of(path: &str) -> Option<String> {
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

    #[test]
    fn paths_name_their_project_and_item() {
        assert_eq!(
            project_of("/projects/demo/features/FEAT-1/move").as_deref(),
            Some("demo")
        );
        assert_eq!(
            item_of("/projects/demo/features/FEAT-1/move?x=1").as_deref(),
            Some("FEAT-1")
        );
        assert_eq!(item_of("/projects/demo/milestones"), None);
        assert_eq!(project_of("/portfolio/programs"), None);
    }
}
