//! The system baseline (FTR-006): the catalog's system-level rows, pinned and accepted by the
//! person before the first item enters the build role.
//!
//! Stored as a per-project side file (`system-baseline.yaml`) beside `catalog.yaml`. Absent means
//! "not initialised": nothing asks for it until the project opts in. `rows` records the system
//! rows that applied at init; each acceptance pins every applying system row's source by the hash
//! of its bytes (a board doc, a repository doc, a folder's docs, or a kanbanr generator's canonical
//! JSON). Acceptances are appended, oldest first, and never rewritten. What has changed since the
//! latest acceptance is derived ([`state`]), never stored (ADR-0002).
//!
//! Not the engineering baseline of FTR-033 (`baseline.rs`): a separate record with a separate name.

use crate::Store;
use crate::catalog::{self, Level, SourceKind, SourceState};
use crate::error::{CoreError, Result};
use crate::models::Extra;
use crate::store::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-project system baseline file, next to `catalog.yaml`.
pub const BASELINE_FILE: &str = "system-baseline.yaml";
/// The file's own schema version.
pub const SCHEMA: u32 = 1;

/// One system row's source as an acceptance saw it. `rev` is empty when the source cannot be
/// pinned (a `katalogr:` view, an enforced source) or was missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Pin {
    #[serde(default)]
    pub row: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub rev: String,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One acceptance of the system baseline, by the person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Acceptance {
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The prepared record confirmed, when one was.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prepared: String,
    /// [`pins_rev`] of `pins`.
    #[serde(default)]
    pub rev: String,
    #[serde(default)]
    pub pins: Vec<Pin>,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

fn default_schema() -> u32 {
    SCHEMA
}

/// A project's system baseline file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Baseline {
    #[serde(default = "default_schema")]
    pub schema: u32,
    #[serde(default)]
    pub initialised_at: String,
    /// The system rows that applied at init: the record, not the rule.
    #[serde(default)]
    pub rows: Vec<String>,
    /// Append-only, oldest first.
    #[serde(default)]
    pub accepted: Vec<Acceptance>,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl Baseline {
    /// The latest acceptance, if there is one.
    pub fn latest(&self) -> Option<&Acceptance> {
        self.accepted.last()
    }
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(BASELINE_FILE)
}

/// Load a project's system baseline. Absent or blank is `None`; a file that does not parse is an
/// error.
pub fn load(store: &Store, id: &str) -> Result<Option<Baseline>> {
    match std::fs::read_to_string(path(store, id)) {
        Ok(s) if s.trim().is_empty() => Ok(None),
        Ok(s) => Ok(Some(crate::yaml::parse::<Baseline>(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn save(store: &Store, id: &str, b: &Baseline) -> Result<()> {
    crate::atomic::write(path(store, id), crate::yaml::to_string(b)?)?;
    Ok(())
}

/// The system rows that apply now, in catalog order.
fn applying_system_rows(status: &catalog::CatalogStatus) -> Vec<&catalog::RowStatus> {
    status
        .rows
        .iter()
        .filter(|r| r.level == Level::System && r.applies.applies)
        .collect()
}

/// Write the file from the catalog's applying system rows, and turn the method's
/// `system-baseline` part on (`system_baseline_accepted` at the build role, saved in the config).
/// Refuses without a catalog and when the file exists. If the config cannot be saved after the
/// file was written, the error says `kanbanr config method --on system-baseline` completes it.
pub fn init(store: &Store, id: &str) -> Result<Baseline> {
    if catalog::load(store, id)?.is_none() {
        return Err(CoreError::Unsupported(
            "no catalog: `kanbanr catalog init` first".into(),
        ));
    }
    if path(store, id).exists() {
        return Err(CoreError::Unsupported(
            "the system baseline is already initialised; if the build gate does not ask for it \
             yet, `kanbanr config method --on system-baseline` turns it on"
                .into(),
        ));
    }
    let project = store.load_meta(id)?;
    let status = catalog::status(store, &project, None)?;
    let baseline = Baseline {
        schema: SCHEMA,
        initialised_at: crate::now_rfc3339(),
        rows: applying_system_rows(&status)
            .into_iter()
            .map(|r| r.id.clone())
            .collect(),
        accepted: Vec::new(),
        extra: Extra::new(),
    };
    save(store, id, &baseline)?;
    store
        .set_method(id, true, &[crate::method::Part::SystemBaseline])
        .map_err(|e| {
            CoreError::Unsupported(format!(
                "the system baseline was written, but the build gate was not turned on ({e}): \
                 `kanbanr config method --on system-baseline` completes it"
            ))
        })?;
    Ok(baseline)
}

/// How one row's source reads now: its pin, and whether it is missing or cannot be pinned.
struct Now {
    pin: Pin,
    missing: bool,
    unpinned: bool,
}

fn source_name(source: &catalog::Source) -> String {
    match source.kind {
        SourceKind::Written => source.path.trim().to_string(),
        SourceKind::Generated => source.generator.trim().to_string(),
        SourceKind::Enforced => source.test.trim().to_string(),
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    crate::hash::stable_hash(&String::from_utf8_lossy(bytes))
}

/// `rel` under `root`, only when it resolves inside it (compared by components, L-46).
fn repo_file(root: &Path, rel: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let p = root.join(rel).canonicalize().ok()?;
    (p.starts_with(&root) && p.is_file()).then_some(p)
}

fn row_now(
    store: &Store,
    project: &Project,
    repo: Option<&Path>,
    row: &catalog::RowStatus,
    listing: &mut Option<Vec<String>>,
) -> Result<Now> {
    let source = source_name(&row.source);
    let mut now = Now {
        pin: Pin {
            row: row.id.clone(),
            source: source.clone(),
            ..Pin::default()
        },
        missing: false,
        unpinned: false,
    };
    match row.source.kind {
        SourceKind::Enforced => now.unpinned = true,
        SourceKind::Generated => {
            if source.starts_with("katalogr:") {
                now.unpinned = true;
            } else {
                let value = catalog::generate(store, project, &source)?;
                now.pin.rev = crate::hash::stable_hash(&crate::hash::canonical_json(&value));
            }
        }
        SourceKind::Written => match &row.state {
            SourceState::Missing => now.missing = true,
            SourceState::NoRepository => now.unpinned = true,
            SourceState::Folder { .. } => {
                let prefix = {
                    let p = source.trim_start_matches("./").trim_start_matches('/');
                    let p = p.strip_prefix("docs/").unwrap_or(p);
                    format!("{}/", p.trim_end_matches('/'))
                };
                let docs =
                    listing.get_or_insert_with(|| store.list_docs(&project.id).unwrap_or_default());
                let mut pairs: Vec<(String, String)> = Vec::new();
                for d in docs.iter().filter(|d| d.starts_with(&prefix)) {
                    let bytes = store.read_doc_bytes(&project.id, d)?;
                    pairs.push((d.clone(), hash_bytes(&bytes)));
                }
                now.pin.rev = crate::hash::stable_hash(&crate::hash::canonical_json(&pairs));
            }
            _ => {
                let bytes = if let Some(rel) = source.strip_prefix(catalog::REPO_PREFIX) {
                    repo.and_then(|r| repo_file(r, rel))
                        .and_then(|p| std::fs::read(p).ok())
                } else {
                    store
                        .read_doc_bytes(&project.id, &crate::refs::doc_key(&source))
                        .ok()
                };
                match bytes {
                    Some(b) => now.pin.rev = hash_bytes(&b),
                    None => now.missing = true,
                }
            }
        },
    }
    Ok(now)
}

fn rows_now(store: &Store, project: &Project, repo: Option<&Path>) -> Result<Vec<Now>> {
    let status = catalog::status(store, project, repo)?;
    let mut listing = None;
    applying_system_rows(&status)
        .into_iter()
        .map(|r| row_now(store, project, repo, r, &mut listing))
        .collect()
}

/// The pins now: one per applying system row, in row order (`catalog::status` decides
/// applicability).
pub fn pins_now(store: &Store, project: &Project, repo: Option<&Path>) -> Result<Vec<Pin>> {
    Ok(rows_now(store, project, repo)?
        .into_iter()
        .map(|n| n.pin)
        .collect())
}

/// The hash of a pins list's canonical JSON.
pub fn pins_rev(pins: &[Pin]) -> String {
    crate::hash::stable_hash(&crate::hash::canonical_json(pins))
}

/// Appends an acceptance. Refuses when an applying system row with a written source is Missing
/// (names them), or when the project has no baseline file.
pub fn accept(
    store: &Store,
    id: &str,
    by: &str,
    origin: &str,
    note: &str,
    prepared: &str,
    repo: Option<&Path>,
) -> Result<Baseline> {
    let project = store.load_meta(id)?;
    let st = state(store, &project, repo)?;
    if !st.can_accept {
        return Err(CoreError::Unsupported(st.cannot_accept_why));
    }
    let mut baseline = load(store, id)?.ok_or_else(|| CoreError::Unsupported(not_initialised()))?;
    let pins = pins_now(store, &project, repo)?;
    baseline.accepted.push(Acceptance {
        at: crate::now_rfc3339(),
        by: by.to_string(),
        origin: origin.to_string(),
        note: note.trim().to_string(),
        prepared: prepared.trim().to_string(),
        rev: pins_rev(&pins),
        pins,
        extra: Extra::new(),
    });
    save(store, id, &baseline)?;
    Ok(baseline)
}

fn not_initialised() -> String {
    "no system baseline: `kanbanr catalog baseline init` first".to_string()
}

/// Where the baseline stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StateKind {
    /// No `system-baseline.yaml`.
    NotInitialised,
    /// Initialised, never accepted.
    Awaiting,
    /// Accepted, and every pinned source reads as it did.
    Accepted,
    /// Accepted, and some row changed since.
    Changed,
}

/// One row that differs from the latest acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Change {
    pub row: String,
    pub source: String,
    /// The pin the latest acceptance holds; empty for a row switched on since.
    pub pinned: String,
    /// The source's hash now; empty when it is missing.
    pub now: String,
    pub missing: bool,
}

/// The system baseline as it stands, derived from the file, the catalog and the sources. Holds no
/// clock and reads in row order, so two reads of one board give the same bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct State {
    pub kind: StateKind,
    /// The file's recorded rows; before init, the applying system rows now.
    pub rows: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Acceptance>,
    pub accepted_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
    /// Rows whose source cannot be pinned: a `katalogr:` view, an enforced source, a `repo:` doc
    /// with no code repository here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpinned: Vec<String>,
    /// Applying system rows whose written source is missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    pub can_accept: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cannot_accept_why: String,
    /// [`pins_rev`] of the pins now.
    pub pins_rev: String,
}

/// The system baseline's state for `project`. `repo` is the code repository the surface has, for
/// `repo:` sources.
pub fn state(store: &Store, project: &Project, repo: Option<&Path>) -> Result<State> {
    let file = load(store, &project.id)?;
    let now = rows_now(store, project, repo)?;
    let missing: Vec<String> = now
        .iter()
        .filter(|n| n.missing)
        .map(|n| n.pin.row.clone())
        .collect();
    let unpinned: Vec<String> = now
        .iter()
        .filter(|n| n.unpinned)
        .map(|n| n.pin.row.clone())
        .collect();
    let pins: Vec<Pin> = now.iter().map(|n| n.pin.clone()).collect();
    let latest = file.as_ref().and_then(|b| b.latest()).cloned();
    let mut changes = Vec::new();
    if let Some(acc) = &latest {
        let pinned: BTreeMap<&str, &Pin> = acc.pins.iter().map(|p| (p.row.as_str(), p)).collect();
        for n in &now {
            let was = pinned.get(n.pin.row.as_str());
            let changed = n.missing
                || match was {
                    None => true,
                    Some(p) => p.rev != n.pin.rev,
                };
            if changed {
                changes.push(Change {
                    row: n.pin.row.clone(),
                    source: n.pin.source.clone(),
                    pinned: was.map(|p| p.rev.clone()).unwrap_or_default(),
                    now: n.pin.rev.clone(),
                    missing: n.missing,
                });
            }
        }
    }
    let kind = match (&file, &latest) {
        (None, _) => StateKind::NotInitialised,
        (Some(_), None) => StateKind::Awaiting,
        (Some(_), Some(_)) if changes.is_empty() => StateKind::Accepted,
        _ => StateKind::Changed,
    };
    let cannot_accept_why = if kind == StateKind::NotInitialised {
        not_initialised()
    } else if !missing.is_empty() {
        let named: Vec<String> = now
            .iter()
            .filter(|n| n.missing)
            .map(|n| {
                if n.pin.source.is_empty() {
                    n.pin.row.clone()
                } else {
                    format!("{} ({})", n.pin.row, n.pin.source)
                }
            })
            .collect();
        format!(
            "write the system docs first: {} {} no source yet",
            named.join(", "),
            if named.len() == 1 { "has" } else { "have" }
        )
    } else {
        String::new()
    };
    Ok(State {
        kind,
        rows: match &file {
            Some(b) => b.rows.clone(),
            None => pins.iter().map(|p| p.row.clone()).collect(),
        },
        accepted_count: file.as_ref().map(|b| b.accepted.len()).unwrap_or(0),
        accepted: latest,
        changes,
        unpinned,
        missing,
        can_accept: cannot_accept_why.is_empty(),
        cannot_accept_why,
        pins_rev: pins_rev(&pins),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, Row, Source};
    use crate::config::ProjectConfig;

    fn temp_store() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-sysbaseline-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        // L-41: the board is this test's own, under the system temp folder.
        assert!(store.data_dir().starts_with(std::env::temp_dir()));
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn written(id: &str, level: Level) -> Row {
        Row {
            id: id.into(),
            title: id.into(),
            answers: vec!["what".into()],
            for_: vec!["architect".into()],
            level,
            source: Source {
                kind: SourceKind::Written,
                path: format!("architecture/{id}.md"),
                ..Source::default()
            },
            ..Row::default()
        }
    }

    fn generated(id: &str, generator: &str) -> Row {
        Row {
            source: Source {
                kind: SourceKind::Generated,
                generator: generator.into(),
                ..Source::default()
            },
            ..written(id, Level::System)
        }
    }

    fn save_catalog(store: &Store, rows: Vec<Row>) {
        catalog::save(
            store,
            "demo",
            &Catalog {
                schema: catalog::SCHEMA,
                rows,
                ..Catalog::default()
            },
        )
        .unwrap();
    }

    fn body(what: &str) -> String {
        format!(
            "# {what}\n\n{}\n",
            "The system as it stands, in words. ".repeat(4)
        )
    }

    fn project(store: &Store) -> Project {
        store.load_meta("demo").unwrap()
    }

    /// R-1: with a catalog, `init` writes the applying system rows only and adds the part to
    /// `@build`; without a catalog it refuses; a second `init` refuses.
    #[test]
    fn init_lists_the_applying_system_rows_and_needs_a_catalog() {
        let (store, dir) = temp_store();
        let err = init(&store, "demo").unwrap_err().to_string();
        assert!(
            err.contains("no catalog: `kanbanr catalog init` first"),
            "{err}"
        );
        assert!(load(&store, "demo").unwrap().is_none());
        // A process with a build role, from before the part: the method without it.
        let mut wf = crate::config::preset("togaf").unwrap();
        crate::method::remove(&mut wf, &[crate::method::Part::SystemBaseline]);
        store.save_config("demo", &wf.into_config("demo")).unwrap();
        let part_at = |store: &Store| {
            let config = store.load_meta("demo").unwrap().config;
            crate::method::present(
                &crate::config::WorkflowFile::from_config(&config),
                crate::method::Part::SystemBaseline,
            )
        };
        assert_eq!(part_at(&store), None);

        let off = Row {
            applies: Some(false),
            why: "not this project".into(),
            ..written("deployment", Level::System)
        };
        save_catalog(
            &store,
            vec![
                written("context", Level::System),
                written("component-docs", Level::Component),
                off,
                generated("decisions", "kanbanr:adr-index"),
                written("item-design", Level::Item),
            ],
        );
        let b = init(&store, "demo").unwrap();
        assert_eq!(b.rows, vec!["context", "decisions"]);
        assert!(b.accepted.is_empty());
        assert!(!b.initialised_at.is_empty());
        assert_eq!(load(&store, "demo").unwrap(), Some(b));
        assert_eq!(
            part_at(&store).as_deref(),
            Some("Implementation"),
            "init turns system_baseline_accepted on at the build role"
        );
        let gate = store.load_meta("demo").unwrap().config.effective_gates();
        assert!(
            gate.values()
                .any(|g| g.requires.contains(&crate::readiness::Condition::Check(
                    crate::readiness::Check::SystemBaselineAccepted
                )))
        );

        let again = init(&store, "demo").unwrap_err().to_string();
        assert!(again.contains("already initialised"), "{again}");
        assert!(
            again.contains("config method --on system-baseline"),
            "{again}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-2: pins hold one entry per applying system row with the doc's hash; a katalogr row is
    /// listed with an empty rev; a missing written source refuses with the row named; a second
    /// acceptance appends.
    #[test]
    fn acceptance_pins_each_source_and_is_refused_while_a_row_has_none() {
        let (store, dir) = temp_store();
        save_catalog(
            &store,
            vec![
                written("context", Level::System),
                generated("decisions", "kanbanr:adr-index"),
                generated("code-structure", "katalogr:structure"),
                written("security", Level::System),
            ],
        );
        let none = accept(&store, "demo", "me", "monitor", "", "", None)
            .unwrap_err()
            .to_string();
        assert!(none.contains("catalog baseline init"), "{none}");
        init(&store, "demo").unwrap();
        store
            .write_doc("demo", "architecture/context.md", &body("Context"))
            .unwrap();

        let refused = accept(&store, "demo", "me", "monitor", "", "", None)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("security (architecture/security.md)"),
            "{refused}"
        );
        assert!(!refused.contains("context"), "{refused}");
        assert!(load(&store, "demo").unwrap().unwrap().accepted.is_empty());

        store
            .write_doc("demo", "architecture/security.md", &body("Security"))
            .unwrap();
        let b = accept(&store, "demo", "me", "monitor", "first", "P-1", None).unwrap();
        let acc = b.latest().unwrap();
        assert_eq!((acc.by.as_str(), acc.origin.as_str()), ("me", "monitor"));
        assert_eq!((acc.note.as_str(), acc.prepared.as_str()), ("first", "P-1"));
        let rows: Vec<&str> = acc.pins.iter().map(|p| p.row.as_str()).collect();
        assert_eq!(
            rows,
            vec!["context", "decisions", "code-structure", "security"]
        );
        let bytes = store
            .read_doc_bytes("demo", "architecture/context.md")
            .unwrap();
        assert_eq!(acc.pins[0].rev, hash_bytes(&bytes));
        assert_eq!(acc.pins[0].source, "architecture/context.md");
        assert!(!acc.pins[1].rev.is_empty(), "a kanbanr view is pinned");
        assert_eq!(acc.pins[2].source, "katalogr:structure");
        assert_eq!(acc.pins[2].rev, "", "a katalogr view is listed, not pinned");
        assert_eq!(acc.rev, pins_rev(&acc.pins));

        let b = accept(&store, "demo", "me", "terminal", "", "", None).unwrap();
        assert_eq!(b.accepted.len(), 2);
        assert_eq!(b.accepted[0].note, "first", "an earlier acceptance is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-6: the four kinds; changes name the row, pinned and now hashes; a row switched on since
    /// reads as a change with an empty pin; two reads byte-equal; 20 rows under 100 ms (debug).
    #[test]
    fn state_is_derived_and_names_the_changed_rows() {
        let (store, dir) = temp_store();
        let mut rows: Vec<Row> = (0..19)
            .map(|i| written(&format!("doc-{i:02}"), Level::System))
            .collect();
        rows.push(generated("code-structure", "katalogr:structure"));
        rows.push(Row {
            applies: Some(false),
            why: "later".into(),
            ..written("late", Level::System)
        });
        save_catalog(&store, rows.clone());
        for i in 0..19 {
            store
                .write_doc(
                    "demo",
                    &format!("architecture/doc-{i:02}.md"),
                    &body(&format!("Doc {i}")),
                )
                .unwrap();
        }

        let s = state(&store, &project(&store), None).unwrap();
        assert_eq!(s.kind, StateKind::NotInitialised);
        assert!(!s.can_accept);
        assert!(s.cannot_accept_why.contains("catalog baseline init"));
        assert_eq!(s.rows.len(), 20);

        init(&store, "demo").unwrap();
        let s = state(&store, &project(&store), None).unwrap();
        assert_eq!(s.kind, StateKind::Awaiting);
        assert!(s.can_accept, "{}", s.cannot_accept_why);
        assert_eq!(s.unpinned, vec!["code-structure"]);
        assert_eq!(s.accepted_count, 0);

        accept(&store, "demo", "me", "monitor", "", "", None).unwrap();
        let p = project(&store);
        let started = std::time::Instant::now();
        let s = state(&store, &p, None).unwrap();
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_millis(100), "{took:?}");
        assert_eq!(s.kind, StateKind::Accepted);
        assert!(s.changes.is_empty());
        assert_eq!(s.accepted_count, 1);
        assert_eq!(s.pins_rev, s.accepted.as_ref().unwrap().rev);
        let again = state(&store, &p, None).unwrap();
        assert_eq!(
            serde_json::to_vec(&s).unwrap(),
            serde_json::to_vec(&again).unwrap(),
            "two reads give the same bytes"
        );

        // A pinned doc changes: the row is named with both hashes.
        let pinned = s.accepted.as_ref().unwrap().pins[3].rev.clone();
        store
            .write_doc("demo", "architecture/doc-03.md", &body("Doc 3, revised"))
            .unwrap();
        let s = state(&store, &project(&store), None).unwrap();
        assert_eq!(s.kind, StateKind::Changed);
        assert_eq!(s.changes.len(), 1);
        let c = &s.changes[0];
        assert_eq!(c.row, "doc-03");
        assert_eq!(c.source, "architecture/doc-03.md");
        assert_eq!(c.pinned, pinned);
        assert_ne!(c.now, pinned);
        assert!(!c.now.is_empty() && !c.missing);
        assert!(s.can_accept);

        // A row switched on since the acceptance reads as a change with an empty pin; its
        // missing doc is listed and blocks a new acceptance.
        catalog::set_row(&store, "demo", "late", true, "").unwrap();
        let s = state(&store, &project(&store), None).unwrap();
        let late = s.changes.iter().find(|c| c.row == "late").unwrap();
        assert_eq!(late.pinned, "");
        assert!(late.missing && late.now.is_empty());
        assert_eq!(s.missing, vec!["late"]);
        assert!(!s.can_accept);
        assert!(s.cannot_accept_why.contains("late (architecture/late.md)"));
        store
            .write_doc("demo", "architecture/late.md", &body("Late"))
            .unwrap();
        let s = state(&store, &project(&store), None).unwrap();
        let late = s.changes.iter().find(|c| c.row == "late").unwrap();
        assert!(late.pinned.is_empty() && !late.now.is_empty() && !late.missing);
        assert!(s.can_accept);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
