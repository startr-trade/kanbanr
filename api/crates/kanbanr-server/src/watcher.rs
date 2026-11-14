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
    // macOS reports an event under the folder's resolved path (`/private/var/…` for `/var/…`),
    // Linux and Windows under the path as watched: match either (DEF-062).
    let resolved = std::fs::canonicalize(&projects_dir).ok();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let mut sent_star = false;
            for path in &event.paths {
                let id = project_of(path, &base)
                    .or_else(|| resolved.as_deref().and_then(|r| project_of(path, r)));
                if let Some(id) = id {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// DEF-025: a board file is now replaced by a rename, not modified in place; the page must
    /// still hear about it.
    #[test]
    fn an_atomic_write_still_fires_a_change_event() {
        let base = std::env::temp_dir().join(format!("kanbanr-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("item.yaml");
        std::fs::write(&file, "old").unwrap();
        let (tx, mut rx) = broadcast::channel(64);
        let _watcher = spawn(base.clone(), tx).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        kanbanr_core::atomic::write(&file, "new").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut heard = false;
        while std::time::Instant::now() < deadline && !heard {
            while let Ok(id) = rx.try_recv() {
                heard |= id == "demo";
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            heard,
            "no change event for the project after an atomic write"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// DEF-062 R-2: a board watched through a linked folder still hears an atomic write, though
    /// macOS reports the event under the folder's resolved path.
    #[cfg(unix)]
    #[test]
    fn a_board_under_a_linked_folder_still_fires_a_change_event() {
        let root = std::env::temp_dir().join(format!("kanbanr-watch-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(real.join("demo")).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let file = link.join("demo").join("item.yaml");
        std::fs::write(&file, "old").unwrap();
        let (tx, mut rx) = broadcast::channel(64);
        let _watcher = spawn(link.clone(), tx).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        kanbanr_core::atomic::write(&file, "new").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut heard = false;
        while std::time::Instant::now() < deadline && !heard {
            while let Ok(id) = rx.try_recv() {
                heard |= id == "demo";
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            heard,
            "no change event for the project under a linked folder"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
