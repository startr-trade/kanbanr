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

/// How a project's workflow stands against the saved process it was applied from (FEAT-170).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Drift {
    /// What the project was given.
    pub source: crate::config::ProcessSource,
    /// The project's workflow no longer matches what was applied: someone edited it since.
    pub edited: bool,
    /// The saved process has changed since: its current version, when it has one.
    pub newer: Option<u32>,
    /// The saved process is no longer in its library.
    pub gone: bool,
}

impl Drift {
    /// Nothing to report.
    pub fn is_current(&self) -> bool {
        !self.edited && self.newer.is_none() && !self.gone
    }

    /// One sentence per thing to report, each saying what to run.
    pub fn messages(&self) -> Vec<String> {
        let s = &self.source;
        let label = match s.version {
            0 => format!("'{}' ({})", s.name, s.library.as_str()),
            v => format!("'{}' v{v} ({})", s.name, s.library.as_str()),
        };
        let mut out = Vec::new();
        if let Some(v) = self.newer {
            let now = if v == 0 {
                "it has changed since".to_string()
            } else {
                format!("v{v} is saved now")
            };
            out.push(format!(
                "this project uses {label}; {now} — see `kanbanr process diff`, apply it with \
                 `kanbanr process update`"
            ));
        }
        if self.gone {
            out.push(format!(
                "this project uses {label}, which is no longer saved there — save it again with \
                 `kanbanr process save {}`",
                s.name
            ));
        }
        if self.edited {
            out.push(format!(
                "this project's workflow was edited after {label} was applied — `kanbanr process \
                 diff` shows how; save it as a new version, or `kanbanr process update` to go back"
            ));
        }
        out
    }
}

/// Compare a project's workflow with the process it came from, given that process's current
/// copy in its library (`None` when it is no longer there). `None` when the project's workflow
/// came from no saved process.
pub fn drift(
    config: &crate::config::ProjectConfig,
    current: Option<&WorkflowFile>,
) -> Option<Drift> {
    let source = config.process.clone()?;
    let mine = WorkflowFile::from_config(config).content_rev();
    let newer = current
        .filter(|c| c.content_rev() != source.rev)
        .map(|c| c.version());
    Some(Drift {
        edited: mine != source.rev,
        newer,
        gone: current.is_none(),
        source,
    })
}

/// What changed between two workflows, a line each, `from` → `to` (FEAT-170).
pub fn diff(from: &WorkflowFile, to: &WorkflowFile) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut out = Vec::new();
    for s in &to.statuses {
        if !from.statuses.contains(s) {
            out.push(format!("+ status '{s}'"));
        }
    }
    for s in &from.statuses {
        if !to.statuses.contains(s) {
            out.push(format!("- status '{s}'"));
        }
    }
    if from.statuses != to.statuses
        && from.statuses.iter().collect::<BTreeSet<_>>() == to.statuses.iter().collect()
    {
        out.push(format!(
            "statuses reordered: {} → {}",
            from.statuses.join(", "),
            to.statuses.join(", ")
        ));
    }
    let field =
        |name: &str, a: &dyn std::fmt::Debug, b: &dyn std::fmt::Debug, out: &mut Vec<String>| {
            let (a, b) = (format!("{a:?}"), format!("{b:?}"));
            if a != b {
                out.push(format!("{name}: {a} → {b}"));
            }
        };
    field(
        "default state",
        &from.default_state,
        &to.default_state,
        &mut out,
    );
    field(
        "displayed",
        &from.displayed_states,
        &to.displayed_states,
        &mut out,
    );
    field("no-op", &from.no_op_states, &to.no_op_states, &mut out);
    field(
        "end states",
        &from.terminal_states,
        &to.terminal_states,
        &mut out,
    );
    let keys: BTreeSet<&String> = from
        .transitions
        .keys()
        .chain(to.transitions.keys())
        .collect();
    for k in keys {
        let (a, b) = (from.transitions.get(k), to.transitions.get(k));
        if a != b {
            out.push(format!(
                "moves from '{k}': {} → {}",
                a.map_or("none".into(), |v| v.join(", ")),
                b.map_or("none".into(), |v| v.join(", "))
            ));
        }
    }
    let keys: BTreeSet<&String> = from.gates.keys().chain(to.gates.keys()).collect();
    for k in keys {
        let yaml = |g: Option<&crate::config::Gate>| {
            g.map(|g| serde_json::to_value(g).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null)
        };
        let (a, b) = (yaml(from.gates.get(k)), yaml(to.gates.get(k)));
        if a == b {
            continue;
        }
        let empty = serde_json::Map::new();
        let (am, bm) = (
            a.as_object().unwrap_or(&empty),
            b.as_object().unwrap_or(&empty),
        );
        let fields: BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
        for f in fields {
            let (x, y) = (am.get(f), bm.get(f));
            if x != y {
                let show =
                    |v: Option<&serde_json::Value>| v.map_or("—".into(), |v| v.to_string());
                out.push(format!("gate '{k}' {f}: {} → {}", show(x), show(y)));
            }
        }
    }
    if from.cadence != to.cadence {
        out.push("sprints and releases differ".into());
    }
    if from.estimate_unit != to.estimate_unit {
        out.push(format!(
            "estimates: {:?} → {:?}",
            from.estimate_unit, to.estimate_unit
        ));
    }
    out
}
