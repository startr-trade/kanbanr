//! The personal process library (FEAT-169), and the order a process name is looked up in.
//!
//! The board's library lives in the board ([`crate::Store::processes_dir`]) and travels with its
//! remote. The personal one is a folder on the user's machine, `~/.kanbanr/processes/`, for carrying
//! a process from one board to another. The CLI reads it, never the board: a board served by a
//! daemon has no business in anyone's home folder. Names resolve board, then personal, then
//! built-in, so a team's agreed process always wins over one person's copy.

use crate::config::{Library, ProcessSource, ProjectConfig, WorkflowFile};
use crate::{CoreError, Result, Store};
use std::collections::BTreeMap;
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
pub fn list_personal(dir: &Path) -> Result<Vec<(String, WorkflowFile)>> {
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

/// A process saved on the board, if there is one by that name. One that cannot be read counts as
/// none here: the board's copy is a preference, and the other libraries still answer.
fn board_copy(store: Option<&Store>, name: &str) -> Option<WorkflowFile> {
    store?.board_process(name).ok().flatten()
}

/// A process by name, for a front end that can also see the user's personal library (FEAT-169):
/// the board's (when there is a board), then the personal one, then the built-in one.
pub fn resolve(
    store: Option<&Store>,
    name: &str,
    personal_dir: Option<&Path>,
) -> Result<(WorkflowFile, Library)> {
    let board = board_copy(store, name);
    let personal = match personal_dir {
        Some(dir) => get(dir, name)?,
        None => None,
    };
    pick(name, board, personal).ok_or_else(|| {
        CoreError::Refused(format!(
            "no process named '{name}' on this board, in your personal library or built in — \
             `kanbanr process list` shows them"
        ))
    })
}

/// A personal process the board does not shadow, with where it came from: what the board must be
/// sent, file and all, because it cannot read the user's home folder (FEAT-169).
pub fn personal_only(
    store: &Store,
    name: &str,
    personal_dir: Option<&Path>,
) -> Result<Option<(WorkflowFile, ProcessSource)>> {
    if board_copy(Some(store), name).is_some() || crate::config::is_builtin(name) {
        return Ok(None);
    }
    let Some(dir) = personal_dir else {
        return Ok(None);
    };
    Ok(get(dir, name)?.map(|file| {
        let source = file.source(name, Library::Personal);
        (file, source)
    }))
}

/// The processes this board can apply (FEAT-169): its own library, then the built-in ones, each
/// with the projects on the board that use it. `GET /processes`.
pub fn listing(store: &Store) -> Result<Vec<ProcessListing>> {
    let mut used: BTreeMap<(String, Library), Vec<String>> = BTreeMap::new();
    for id in store.list_projects()? {
        if let Some(src) = store.load(&id)?.config.process {
            used.entry((src.name, src.library)).or_default().push(id);
        }
    }
    let row = |name: &str, library: Library, file: &WorkflowFile, about: String| ProcessListing {
        name: name.to_string(),
        library,
        version: Some(file.version()),
        description: Some(about),
        rev: Some(file.content_rev()),
        used_by: used
            .get(&(name.to_string(), library))
            .cloned()
            .unwrap_or_default(),
    };
    let mut out = Vec::new();
    for (name, file) in store.list_processes()? {
        let about = file
            .process
            .as_ref()
            .map(|h| h.description.clone())
            .unwrap_or_default();
        out.push(row(&name, Library::Board, &file, about));
    }
    for (name, about) in crate::config::presets() {
        out.push(row(
            name,
            Library::Builtin,
            &crate::config::preset(name)?,
            about,
        ));
    }
    // Personal processes live on someone's machine, not here; the board knows only which projects
    // use one, and which version they applied. A front end fills in the user's own ([`list`]).
    for ((name, library), projects) in &used {
        if *library == Library::Personal {
            out.push(ProcessListing {
                name: name.clone(),
                library: *library,
                version: None,
                description: None,
                rev: None,
                used_by: projects.clone(),
            });
        }
    }
    Ok(out)
}

/// One process as `kanbanr process list` shows it (FEAT-169): the board's listing with the user's
/// personal library filled in.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListedProcess {
    pub name: String,
    pub library: Library,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    pub used_by: Vec<String>,
    /// A personal process the board has one of the same name of: the board's copy is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by_board: Option<bool>,
    /// Someone's personal process, used on the board but not in this user's library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_here: Option<bool>,
}

impl From<ProcessListing> for ListedProcess {
    fn from(l: ProcessListing) -> Self {
        ListedProcess {
            name: l.name,
            library: l.library,
            version: l.version,
            description: l.description,
            rev: l.rev,
            used_by: l.used_by,
            shadowed_by_board: None,
            not_here: None,
        }
    }
}

/// Every process a user can apply here (FEAT-169): the board's, then their personal library's,
/// then the built-in ones. Without a board, the built-in ones alone stand in for the board's
/// listing. A personal process the board lists as used but this user does not have is listed as
/// someone else's.
pub fn list(store: Option<&Store>, personal_dir: Option<&Path>) -> Result<Vec<ListedProcess>> {
    let mut rows: Vec<ListedProcess> = match store {
        Some(store) => listing(store)?.into_iter().map(Into::into).collect(),
        None => crate::config::presets()
            .into_iter()
            .map(|(name, about)| ListedProcess {
                name: name.to_string(),
                library: Library::Builtin,
                version: Some(0),
                description: Some(about),
                rev: None,
                used_by: Vec::new(),
                shadowed_by_board: None,
                not_here: None,
            })
            .collect(),
    };
    let board_names: Vec<String> = rows
        .iter()
        .filter(|r| r.library == Library::Board)
        .map(|r| r.name.clone())
        .collect();
    // The board lists personal processes only as "used by these projects"; the files are on
    // people's machines. Yours fill in from your library; any left are someone else's.
    let (mut used_personal, rest): (Vec<ListedProcess>, Vec<ListedProcess>) = rows
        .into_iter()
        .partition(|r| r.library == Library::Personal);
    rows = rest;
    let mut mine = Vec::new();
    if let Some(dir) = personal_dir {
        for (name, file) in list_personal(dir)? {
            let used = used_personal
                .iter()
                .position(|r| r.name == name)
                .map(|i| used_personal.remove(i).used_by)
                .unwrap_or_default();
            mine.push(ListedProcess {
                library: Library::Personal,
                version: Some(file.version()),
                description: Some(
                    file.process
                        .as_ref()
                        .map(|h| h.description.clone())
                        .unwrap_or_default(),
                ),
                rev: Some(file.content_rev()),
                used_by: used,
                shadowed_by_board: Some(board_names.contains(&name)),
                not_here: None,
                name,
            });
        }
    }
    for r in used_personal {
        mine.push(ListedProcess {
            name: r.name,
            library: Library::Personal,
            version: Some(0),
            description: Some("someone's personal process — not in your library".into()),
            rev: None,
            used_by: r.used_by,
            shadowed_by_board: None,
            not_here: Some(true),
        });
    }
    let at = rows
        .iter()
        .position(|r| r.library == Library::Builtin)
        .unwrap_or(rows.len());
    rows.splice(at..at, mine);
    Ok(rows)
}

/// The projects on this board using process `name` from `library` at a revision other than `rev`
/// (FEAT-171), with the version each has. A project that cannot be read is skipped.
pub fn behind(
    store: &Store,
    name: &str,
    library: Library,
    rev: &str,
) -> Result<Vec<(String, u32)>> {
    let mut out = Vec::new();
    for id in store.list_projects()? {
        let Ok(project) = store.load(&id) else {
            continue;
        };
        if let Some(src) = &project.config.process
            && src.name == name
            && src.library == library
            && src.rev != rev
        {
            out.push((id, src.version));
        }
    }
    Ok(out)
}

/// The copy of `source` in the library it was given from — not by lookup order: an update brings
/// a project its own process's new version (FEAT-170). A personal process with no personal library
/// to look in reads as gone.
pub(crate) fn copy_in(
    store: &Store,
    source: &ProcessSource,
    personal_dir: Option<&Path>,
) -> Result<Option<WorkflowFile>> {
    Ok(match source.library {
        Library::Board => store.board_process(&source.name)?,
        Library::Personal => match personal_dir {
            Some(dir) => get(dir, &source.name)?,
            None => None,
        },
        Library::Builtin => crate::config::preset(&source.name).ok(),
    })
}

/// The current copy of the process a project was given, as this project would have it: the parts
/// of the method it turned off stay off (FEAT-193).
pub fn current_copy(
    store: &Store,
    config: &ProjectConfig,
    source: &ProcessSource,
    personal_dir: Option<&Path>,
) -> Result<Option<WorkflowFile>> {
    Ok(copy_in(store, source, personal_dir)?.map(|f| crate::method::shaped(f, &config.method)))
}

/// How a project's workflow stands against the saved process it was applied from (FEAT-170).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

/// How a project stands against its saved process (FEAT-170): `GET /projects/{p}/process`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProcessStatus {
    /// The process the project's workflow was applied from; null when none was.
    pub source: Option<crate::config::ProcessSource>,
    /// How it has moved apart from that process; null when there is nothing to compare, or the
    /// process is a personal one, which only the user's own kanbanr can see.
    pub drift: Option<Drift>,
    /// [`Drift::messages`]: one sentence per thing to report.
    pub messages: Vec<String>,
}

/// One process the board can apply (FEAT-169): `GET /processes`. A personal process is known
/// here only by name and the projects that use it, so it carries no version, description or rev.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProcessListing {
    pub name: String,
    pub library: Library,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    pub used_by: Vec<String>,
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
    // The process as this project takes it: a part of the method it turned off is a choice, not
    // an edit, and not news (FEAT-193).
    let current = current.map(|c| crate::method::shaped(c.clone(), &config.method));
    let newer = current
        .as_ref()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A board and a personal library, each in its own temp folder, removed on drop.
    struct Dirs {
        board: PathBuf,
        personal: PathBuf,
    }
    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.board);
            let _ = std::fs::remove_dir_all(&self.personal);
        }
    }
    fn dirs() -> Dirs {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base = std::env::temp_dir();
        let d = Dirs {
            board: base.join(format!("kanbanr-library-board-{}-{n}", std::process::id())),
            personal: base.join(format!("kanbanr-library-mine-{}-{n}", std::process::id())),
        };
        let _ = std::fs::remove_dir_all(&d.board);
        let _ = std::fs::remove_dir_all(&d.personal);
        d
    }
    fn titled(about: &str) -> WorkflowFile {
        let mut file = crate::config::preset("default").unwrap();
        file.statuses.push(about.to_string());
        file
    }

    /// FEAT-176: lookup order is board, then personal, then built-in; nothing found names the
    /// three places.
    #[test]
    fn resolve_prefers_board_then_personal_then_builtin() {
        let d = dirs();
        let store = Store::new(d.board.clone());
        save(&d.personal, "mine", titled("Personal"), None).unwrap();
        let (_, lib) = resolve(Some(&store), "mine", Some(&d.personal)).unwrap();
        assert_eq!(lib, Library::Personal);
        store.save_process("mine", titled("Board"), None).unwrap();
        let (file, lib) = resolve(Some(&store), "mine", Some(&d.personal)).unwrap();
        assert_eq!(lib, Library::Board);
        assert!(file.statuses.contains(&"Board".to_string()));
        assert_eq!(resolve(None, "togaf", None).unwrap().1, Library::Builtin);
        let err = resolve(None, "nope", None).unwrap_err().to_string();
        assert!(err.contains("no process named 'nope'"), "{err}");
    }

    /// FEAT-176: a personal process goes to the board as a file only when the board does not
    /// shadow it and it is not built in.
    #[test]
    fn personal_only_skips_shadowed_and_builtin() {
        let d = dirs();
        let store = Store::new(d.board.clone());
        save(&d.personal, "mine", titled("P"), None).unwrap();
        let (_, source) = personal_only(&store, "mine", Some(&d.personal))
            .unwrap()
            .unwrap();
        assert_eq!(source.library, Library::Personal);
        assert!(personal_only(&store, "mine", None).unwrap().is_none());
        assert!(
            personal_only(&store, "togaf", Some(&d.personal))
                .unwrap()
                .is_none()
        );
        store.save_process("mine", titled("B"), None).unwrap();
        assert!(
            personal_only(&store, "mine", Some(&d.personal))
                .unwrap()
                .is_none()
        );
    }

    /// FEAT-176: the merged listing puts the user's own processes between the board's and the
    /// built-in ones, flags a shadowed one, and names someone else's that a project uses.
    #[test]
    fn list_merges_personal_between_board_and_builtin() {
        let d = dirs();
        let store = Store::new(d.board.clone());
        store.save_process("shared", titled("S"), None).unwrap();
        save(&d.personal, "shared", titled("S2"), None).unwrap();
        save(&d.personal, "solo", titled("X"), None).unwrap();
        let mut config = crate::config::ProjectConfig::default_for("demo");
        config.process = Some(titled("Y").source("theirs", Library::Personal));
        store.init_project("demo", config).unwrap();
        let rows = list(Some(&store), Some(&d.personal)).unwrap();
        let names: Vec<(&str, Library)> =
            rows.iter().map(|r| (r.name.as_str(), r.library)).collect();
        assert_eq!(names[0], ("shared", Library::Board));
        assert_eq!(names[1], ("shared", Library::Personal));
        assert_eq!(names[2], ("solo", Library::Personal));
        assert_eq!(names[3], ("theirs", Library::Personal));
        assert_eq!(names[4].1, Library::Builtin);
        assert_eq!(rows[1].shadowed_by_board, Some(true));
        assert_eq!(rows[3].not_here, Some(true));
        assert_eq!(rows[3].used_by, vec!["demo".to_string()]);
        // Without a board, the built-in ones carry version 0 and no rev.
        let bare = list(None, None).unwrap();
        assert!(bare.iter().all(|r| r.library == Library::Builtin));
        assert!(bare.iter().all(|r| r.version == Some(0) && r.rev.is_none()));
    }

    /// FEAT-176: a project on another revision of the same process from the same library is
    /// behind; one on the saved revision, or from another library, is not.
    #[test]
    fn behind_names_projects_on_another_revision() {
        let d = dirs();
        let store = Store::new(d.board.clone());
        let old = titled("Old");
        let new = titled("New");
        let mut config = crate::config::ProjectConfig::default_for("a");
        config.process = Some(old.source("p", Library::Board));
        store.init_project("a", config.clone()).unwrap();
        config.process = Some(new.source("p", Library::Board));
        store.init_project("b", config.clone()).unwrap();
        config.process = Some(old.source("p", Library::Personal));
        store.init_project("c", config).unwrap();
        let behind = behind(&store, "p", Library::Board, &new.content_rev()).unwrap();
        assert_eq!(behind, vec![("a".to_string(), 0)]);
    }

    /// FEAT-176: a personal process's drift needs the personal library; without one there is
    /// nothing to say (the daemon's case), with one it is compared.
    #[test]
    fn process_drift_reads_the_personal_library_when_given() {
        let d = dirs();
        let store = Store::new(d.board.clone());
        let file = titled("Mine");
        save(&d.personal, "mine", file.clone(), None).unwrap();
        let saved = get(&d.personal, "mine").unwrap().unwrap();
        let mut config = saved.clone().into_config("demo");
        config.process = Some(saved.source("mine", Library::Personal));
        let project = store.init_project("demo", config).unwrap();
        assert!(store.process_drift(&project, None).unwrap().is_none());
        let drift = store
            .process_drift(&project, Some(&d.personal))
            .unwrap()
            .unwrap();
        assert!(!drift.gone, "{drift:?}");
    }

    /// FEAT-176: the workflow body applies the whole file, says where it came from only when
    /// told, and leaves an off cadence and a days estimate as null.
    #[test]
    fn to_request_carries_the_whole_file() {
        let file = crate::config::preset("togaf").unwrap();
        let body = file.to_request(None);
        assert!(body.get("process").is_none());
        assert_eq!(body["statuses"], serde_json::json!(file.statuses));
        assert!(body["cadence"].is_null() && body["estimate_unit"].is_null());
        let source = file.source("togaf", Library::Builtin);
        let body = file.to_request(Some(&source));
        assert_eq!(body["process"]["name"], "togaf");
        let back = WorkflowFile::from_yaml(&file.to_yaml().unwrap()).unwrap();
        assert_eq!(back, file);
    }

    /// FEAT-193: drift compares against the process as the project takes it — a part of the
    /// method it turned off is neither an edit nor news.
    #[test]
    fn drift_honours_method_choice() {
        use crate::method::{MethodChoice, Part};
        let file = crate::config::preset("togaf").unwrap();
        let choice = MethodChoice {
            off: Part::ALL.to_vec(),
            chosen_at: "2026-10-19T00:00:00Z".into(),
        };
        let shaped_file = crate::method::shaped(file.clone(), &choice);
        let mut config = shaped_file.clone().into_config("p");
        config.method = choice;
        config.process = Some(shaped_file.source("togaf", crate::config::Library::Builtin));
        let d = drift(&config, Some(&file)).unwrap();
        assert!(!d.edited && d.newer.is_none(), "{d:?}");
        // Without the choice recorded, the same workflow reads as behind the process.
        config.method = MethodChoice::default();
        let d = drift(&config, Some(&file)).unwrap();
        assert!(d.newer.is_some(), "{d:?}");
    }
}
