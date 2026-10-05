//! The personal process library (FEAT-169), and the order a process name is looked up in.
//!
//! The board's library lives in the board ([`crate::Store::processes_dir`]) and travels with its
//! remote. The personal one is a folder on the user's machine, `~/.kanbanr/processes/`, for carrying
//! a process from one board to another. The CLI reads it, never the board: a board served by a
//! daemon has no business in anyone's home folder. Names resolve board, then personal, then
//! built-in, so a team's agreed process always wins over one person's copy.

use crate::Result;
use crate::config::{Library, WorkflowFile};
use std::path::{Path, PathBuf};

/// The personal library: `$KANBANR_PROCESSES_DIR` when set, else `<home>/.kanbanr/processes`.
/// `find_marker` matches only a *file* named `.kanbanr` and skips the home folder, so this folder
/// is never taken for a project's marker.
pub fn personal_dir(home: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("KANBANR_PROCESSES_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".kanbanr").join("processes")))
}

/// The processes in a personal library, by name. A missing folder is an empty library.
pub fn list(dir: &Path) -> Result<Vec<(String, WorkflowFile)>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".yaml"))
        else {
            continue;
        };
        let text = std::fs::read_to_string(&path)?;
        out.push((name.to_string(), serde_yaml::from_str(&text)?));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// One personal process, if there is one by that name.
pub fn get(dir: &Path, name: &str) -> Result<Option<WorkflowFile>> {
    if crate::config::check_process_name(name).is_err() {
        return Ok(None);
    }
    let path = dir.join(format!("{name}.yaml"));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(serde_yaml::from_str(&std::fs::read_to_string(path)?)?))
}

/// Save a personal process, versioned as a board save is ([`crate::config::versioned`]).
pub fn save(
    dir: &Path,
    name: &str,
    file: WorkflowFile,
    description: Option<String>,
) -> Result<WorkflowFile> {
    let existing = get(dir, name)?;
    let (file, changed) = crate::config::versioned(name, file, existing.as_ref(), description)?;
    if changed {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join(format!("{name}.yaml")),
            serde_yaml::to_string(&file)?,
        )?;
    }
    Ok(file)
}

/// Which copy of `name` applies: the board's, else the personal one, else the built-in one.
pub fn pick(
    name: &str,
    board: Option<WorkflowFile>,
    personal: Option<WorkflowFile>,
) -> Option<(WorkflowFile, Library)> {
    board
        .map(|f| (f, Library::Board))
        .or_else(|| personal.map(|f| (f, Library::Personal)))
        .or_else(|| {
            crate::config::preset(name)
                .ok()
                .map(|f| (f, Library::Builtin))
        })
}
