//! Filesystem watcher: turns YAML file changes into broadcast events keyed by project id.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path, PathBuf};
use tokio::sync::broadcast;

/// Watch `projects_dir` recursively; on any change, broadcast the affected project id
/// (and always "*" so the projects-list view refreshes too).
pub fn spawn(
    projects_dir: PathBuf,
    tx: broadcast::Sender<String>,
) -> anyhow::Result<RecommendedWatcher> {
    let base = projects_dir.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let mut sent_star = false;
            for path in &event.paths {
                if let Some(id) = project_of(path, &base) {
                    let _ = tx.send(id);
                }
            }
            if !sent_star {
                let _ = tx.send("*".to_string());
                sent_star = true;
            }
            let _ = sent_star;
        }
    })?;
    watcher.watch(&projects_dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

/// Extract the project id (first path component under projects_dir) from a changed path.
fn project_of(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    match rel.components().next()? {
        Component::Normal(s) => s.to_str().map(|s| s.to_string()),
        _ => None,
    }
}
