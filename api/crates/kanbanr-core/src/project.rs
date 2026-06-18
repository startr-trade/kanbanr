//! Resolution of the data directory and the active project name.

use std::path::{Path, PathBuf};

/// Resolve the data directory: explicit arg → `KANBANR_DATA_DIR` → `<cwd-or-default>/data`.
pub fn resolve_data_dir(explicit: Option<&str>) -> PathBuf {
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("KANBANR_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from("data")
}

/// Resolve the active project name: explicit arg → `KANBANR_PROJECT` → `.kanbanr` marker file
/// in cwd → current directory's basename.
pub fn resolve_project(explicit: Option<&str>) -> Option<String> {
    if let Some(p) = explicit {
        return Some(p.to_string());
    }
    if let Ok(p) = std::env::var("KANBANR_PROJECT") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    if let Ok(content) = std::fs::read_to_string(".kanbanr") {
        let name = content.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    std::env::current_dir()
        .ok()
        .and_then(|p| basename(&p))
}

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}
