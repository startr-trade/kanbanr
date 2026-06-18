//! Per-project activity log — a changelog the writer appends to, stored as plain data in the data
//! folder (`projects/<id>/activity.yaml`): a capped, newest-first list of `{time, actor, message}`.
//! The view (web monitor, or any future viewer) just reads this file; no git plumbing required.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Keep the most recent N entries (older history still lives in git).
const MAX: usize = 200;
const FILE: &str = "activity.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// RFC3339 timestamp.
    pub time: String,
    /// Who made the change (the commit identity name).
    pub actor: String,
    /// A short human description of the change.
    pub message: String,
    /// The work item (feature code) this change touched, when applicable — enables per-item and
    /// per-kind activity streams. Serialized as `item`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

fn path(data_dir: &Path, project: &str) -> PathBuf {
    data_dir.join("projects").join(project).join(FILE)
}

fn load(file: &Path) -> Vec<Activity> {
    std::fs::read_to_string(file).ok().and_then(|s| serde_yaml::from_str(&s).ok()).unwrap_or_default()
}

/// The most recent `limit` entries, newest first.
pub fn read(data_dir: &Path, project: &str, limit: usize) -> Vec<Activity> {
    load(&path(data_dir, project)).into_iter().take(limit).collect()
}

/// Append an entry (newest first), capping the file to the most recent `MAX`. `item` is the work
/// item (feature code) the change touched, when applicable.
pub fn append(data_dir: &Path, project: &str, actor: &str, message: &str, item: Option<&str>) {
    let file = path(data_dir, project);
    let mut list = load(&file);
    list.insert(
        0,
        Activity {
            time: crate::now_rfc3339(),
            actor: actor.to_string(),
            message: message.to_string(),
            item: item.map(String::from),
        },
    );
    list.truncate(MAX);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(yaml) = serde_yaml::to_string(&list) {
        let _ = std::fs::write(&file, yaml);
    }
}
