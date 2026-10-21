//! The schema-5 migration (FEAT-187): canonical revisions, every file rewritten, in one commit.
//!
//! Before schema 5 a revision was a hash of the YAML serde_yaml wrote. That library is gone, so the
//! old hash cannot be computed again — and a revision hashed one way never matches one hashed the
//! other way, which would lapse every approval on the board. So each stored revision is re-pinned
//! to the canonical revision of the content it was recorded against, and that content is read from
//! the board's own git history: the first commit whose file holds the verdict is the one that
//! recorded it, and its definition is what was agreed to. Whatever was current stays current, and
//! whatever had lapsed stays lapsed, with no old serializer shipped.
//!
//! What history cannot vouch for (no repository, a shallow clone, hand-edited history) is pinned to
//! [`UNVERIFIED`], which never matches: the item reads as lapsed, the report counts it and `doctor`
//! names it. Trusting it instead would let an approval stand that nothing shows was given to this
//! text.
//!
//! The migration is explicit — `kanbanr migrate` — and every other command refuses a board below
//! schema 5 with that hint ([`crate::CoreError::NeedsMigration`]): rewriting every file once is a
//! deliberate act, not a side effect of `kanbanr board`.

use crate::config::{CURRENT_SCHEMA_VERSION, MIN_READABLE_SCHEMA_VERSION, ProjectConfig};
use crate::error::{CoreError, Result};
use crate::git::{self, PushPolicy};
use crate::models::{FeatureDefinition, FeatureMeta};
use crate::write::WriteError;
use crate::{Store, yaml};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The revision of a verdict the board's history could not vouch for. It matches no content, so
/// what carries it reads as lapsed until it is given again.
pub const UNVERIFIED: &str = "unverified";

/// The migration commit's message.
pub const COMMIT_MESSAGE: &str = "migrate board to schema 5: canonical revisions, files rewritten";

/// What the migration did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MigrationReport {
    /// The schema the board is now at.
    pub schema: u32,
    /// Each project migrated, with the schema it was at. Empty when the board was already current.
    pub projects: Vec<MigratedProject>,
    /// Whether the board's git history could be read.
    pub history: bool,
    /// Revisions re-pinned from history.
    pub repinned: usize,
    /// Revisions history could not vouch for, now [`UNVERIFIED`].
    pub unverified: Vec<Unverified>,
    /// Files whose text changed.
    pub files_rewritten: usize,
    /// Whether the migration commit was made.
    pub committed: bool,
    /// Pushes the board's push policy ran and that failed (the commit is made either way).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub push_failures: Vec<String>,
}

impl MigrationReport {
    /// The unverified revisions that decide something now — a standing approval, the latest
    /// sign-off of a name, a project's process — rather than an older entry in a log.
    pub fn unverified_current(&self) -> impl Iterator<Item = &Unverified> {
        self.unverified.iter().filter(|u| u.current)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MigratedProject {
    pub id: String,
    pub from: u32,
}

/// One revision the board's history could not vouch for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Unverified {
    pub project: String,
    /// The item, or none for a project's process provenance.
    pub code: Option<String>,
    /// `approval`, `sign-off <name>` or `process`.
    pub what: String,
    /// When the verdict was given (empty for a process).
    pub at: String,
    /// Whether it is what decides the item's state now.
    pub current: bool,
}

/// Does any project on this board need the migration?
pub fn needed(store: &Store) -> Result<bool> {
    for id in store.list_projects()? {
        if store.read_config_unchecked(&id)?.schema_version < MIN_READABLE_SCHEMA_VERSION {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `kanbanr migrate`: the migration as a write — under the board's lock, authored by its identity,
/// one commit, then the board's push policy. Nothing is committed when the board is already current.
pub fn run(
    store: &Store,
    data_dir: &Path,
    policy: PushPolicy,
) -> std::result::Result<MigrationReport, WriteError> {
    crate::write::with_lock(data_dir, || {
        git::ensure_repo(data_dir);
        if !needed(store).map_err(WriteError::Core)? {
            return Ok(MigrationReport {
                schema: CURRENT_SCHEMA_VERSION,
                history: true,
                ..Default::default()
            });
        }
        git::require_identity(data_dir).map_err(WriteError::Refused)?;
        let mut report = migrate_to_v5(store).map_err(WriteError::Core)?;
        report.committed = git::commit_local(data_dir, COMMIT_MESSAGE);
        if report.committed
            && let Some(outcome) = git::after_commit(data_dir, policy)
        {
            report.push_failures = outcome.failures;
        }
        Ok(report)
    })
    .map_err(|e| WriteError::Refused(format!("write lock: {e}")))?
}

/// Bring every project below schema 5 up to it, in the working tree (no commit): re-pin each stored
/// revision from the board's history, stamp schema 5, and rewrite every board file in this build's
/// format. Nothing is written unless every file reads back to what it held.
pub fn migrate_to_v5(store: &Store) -> Result<MigrationReport> {
    let mut report = MigrationReport {
        schema: CURRENT_SCHEMA_VERSION,
        ..Default::default()
    };
    // What to migrate: every project below 5. A board from a newer kanbanr is refused outright.
    let mut projects = Vec::new();
    for id in store.list_projects()? {
        let config = store.read_config_unchecked(&id)?;
        if config.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(CoreError::SchemaTooNew {
                found: config.schema_version,
                understood: CURRENT_SCHEMA_VERSION,
            });
        }
        if config.schema_version < MIN_READABLE_SCHEMA_VERSION {
            let features = store.read_feature_metas(&id, &config.statuses)?;
            projects.push((id, config, features));
        }
    }
    if projects.is_empty() {
        report.history = true;
        return Ok(report);
    }

    let wanted: BTreeMap<String, BTreeSet<String>> = projects
        .iter()
        .map(|(id, _, features)| {
            (
                id.clone(),
                features.iter().map(|(_, m)| m.code.clone()).collect(),
            )
        })
        .collect();
    let history = History::read(store.data_dir(), &wanted);
    report.history = history.is_some();
    let history = history.unwrap_or_default();

    // The new text of every file, decided before anything is written.
    let mut files: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (id, mut config, features) in projects {
        report.projects.push(MigratedProject {
            id: id.clone(),
            from: config.schema_version,
        });
        for (status, mut meta) in features {
            if let Some(def) = meta.definition.as_mut() {
                repin_definition(&history, &id, &meta.code, def, &mut report);
            }
            let text = yaml::to_string(&meta)?;
            same_after_reading::<FeatureMeta>(&meta, &text)?;
            files.insert(store.feature_path(&id, &status, &meta.code), text);
        }
        if let Some(source) = config.process.as_mut() {
            match history
                .processes
                .get(&(id.clone(), crate::hash::canonical_json(source)))
            {
                Some(rev) => {
                    source.rev = rev.clone();
                    report.repinned += 1;
                }
                None => {
                    source.rev = UNVERIFIED.to_string();
                    report.unverified.push(Unverified {
                        project: id.clone(),
                        code: None,
                        what: "process".into(),
                        at: String::new(),
                        current: true,
                    });
                }
            }
        }
        config.schema_version = CURRENT_SCHEMA_VERSION;
        let text = yaml::to_string(&config)?;
        same_after_reading::<ProjectConfig>(&config, &text)?;
        files.insert(store.project_dir(&id).join("config.yaml"), text);

        // Every other board file of the project, in this build's format, holding what it held.
        let mut others = Vec::new();
        board_yaml_files(&store.project_dir(&id), &mut others)?;
        for path in others {
            if !files.contains_key(&path) {
                files.insert(path.clone(), rewritten(&path)?);
            }
        }
    }
    let mut shared = Vec::new();
    board_yaml_files(&store.processes_dir(), &mut shared)?;
    let workspace = store.data_dir().join("workspace.yaml");
    if workspace.is_file() {
        shared.push(workspace);
    }
    for path in shared {
        files.insert(path.clone(), rewritten(&path)?);
    }

    for (path, text) in &files {
        if std::fs::read_to_string(path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(path, text)?;
            report.files_rewritten += 1;
        }
    }
    Ok(report)
}

/// Re-pin a definition's approvals and sign-offs to the canonical revisions history gives them.
fn repin_definition(
    history: &History,
    project: &str,
    code: &str,
    def: &mut FeatureDefinition,
    report: &mut MigrationReport,
) {
    let key = |at: &str| (project.to_string(), code.to_string(), instant(at));
    let unverified = |what: &str, at: &str, current: bool| Unverified {
        project: project.to_string(),
        code: Some(code.to_string()),
        what: what.to_string(),
        at: at.to_string(),
        current,
    };
    let standing = def.approval.as_ref().map(|a| a.at.clone());
    for event in &mut def.approvals {
        if event.rev.is_empty() {
            continue;
        }
        match history.approvals.get(&key(&event.at)) {
            Some(rev) => {
                event.rev = rev.clone();
                report.repinned += 1;
            }
            None => {
                event.rev = UNVERIFIED.to_string();
                let current = standing.as_deref() == Some(event.at.as_str());
                report
                    .unverified
                    .push(unverified("approval", &event.at, current));
            }
        }
    }
    if let Some(approval) = def.approval.as_mut() {
        match history.approvals.get(&key(&approval.at)) {
            Some(rev) => {
                approval.rev = rev.clone();
                report.repinned += 1;
            }
            None => {
                approval.rev = UNVERIFIED.to_string();
                // Counted once: with its event when it has one, here when it has none.
                if !report.unverified.iter().any(|u| {
                    u.project == project
                        && u.code.as_deref() == Some(code)
                        && u.what == "approval"
                        && u.at == approval.at
                }) {
                    report
                        .unverified
                        .push(unverified("approval", &approval.at, true));
                }
            }
        }
    }
    let latest: BTreeMap<String, String> = def
        .signoffs
        .iter()
        .map(|s| (s.id.clone(), s.at.clone()))
        .collect();
    for signoff in &mut def.signoffs {
        // A withdrawal keeps its empty rev (FEAT-182): there is nothing it was pinned to.
        if signoff.rev.is_empty() {
            continue;
        }
        let k = (
            project.to_string(),
            code.to_string(),
            signoff.id.clone(),
            instant(&signoff.at),
        );
        match history.signoffs.get(&k) {
            Some(rev) => {
                signoff.rev = rev.clone();
                report.repinned += 1;
            }
            None => {
                signoff.rev = UNVERIFIED.to_string();
                let current = latest.get(&signoff.id) == Some(&signoff.at);
                report.unverified.push(unverified(
                    &format!("sign-off {}", signoff.id),
                    &signoff.at,
                    current,
                ));
            }
        }
    }
}

/// Refuse to write a record that would not read back as itself.
fn same_after_reading<T: Serialize + serde::de::DeserializeOwned>(
    value: &T,
    text: &str,
) -> Result<()> {
    let back: T = yaml::from_str(text)?;
    if crate::hash::canonical_json(&back) != crate::hash::canonical_json(value) {
        return Err(CoreError::Unsupported(
            "a record would not read back as it was written; nothing was migrated".into(),
        ));
    }
    Ok(())
}

/// A file's text in this build's format, refused unless it reads back to what it held (R-2).
fn rewritten(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path)?;
    let unreadable = |e: &dyn std::fmt::Display| {
        CoreError::Unsupported(format!(
            "{} does not read as YAML ({e}); fix or remove it, then run `kanbanr migrate` again — \
             nothing was migrated",
            path.display()
        ))
    };
    let value: yaml::Value = yaml::parse(&text).map_err(|e| unreadable(&e))?;
    let new = yaml::to_string(&value)?;
    let back: yaml::Value = yaml::parse(&new).map_err(|e| unreadable(&e))?;
    if crate::hash::canonical_json(&back) != crate::hash::canonical_json(&value) {
        return Err(CoreError::Unsupported(format!(
            "{} would not read back as it was written; nothing was migrated",
            path.display()
        )));
    }
    Ok(new)
}

/// The board's own YAML files under `dir`: everything but a docs tree, which holds the user's
/// documents rather than board records.
fn board_yaml_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "docs" && !name.starts_with('.') {
                board_yaml_files(&path, out)?;
            }
        } else if name.ends_with(".yaml") || name.ends_with(".yml") {
            out.push(path);
        }
    }
    Ok(())
}

/// What the board's history says each verdict was given on.
#[derive(Default)]
struct History {
    /// (project, item, at) → the canonical revision of the definition in the first commit holding
    /// an approval, or an approval event, given at `at`.
    approvals: HashMap<(String, String, String), String>,
    /// (project, item, sign-off, at) → likewise, for a sign-off.
    signoffs: HashMap<(String, String, String, String), String>,
    /// (project, canonical JSON of its process source) → the canonical revision of the project's
    /// workflow in the first commit whose config records that source.
    processes: HashMap<(String, String), String>,
}

impl History {
    /// Walk the board's commits oldest first. `None` when the board is not a repository; a
    /// repository with no commits has an empty history.
    fn read(data_dir: &Path, wanted: &BTreeMap<String, BTreeSet<String>>) -> Option<History> {
        let repo = git2::Repository::open(data_dir).ok()?;
        let mut history = History::default();
        let Ok(mut walk) = repo.revwalk() else {
            return Some(history);
        };
        if walk.push_head().is_err() {
            return Some(history);
        }
        let _ = walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE);
        // A file's content is read once per project: the first commit holding a blob is the
        // earliest.
        let mut seen: BTreeMap<&str, HashSet<git2::Oid>> = BTreeMap::new();
        for oid in walk.flatten() {
            let Ok(tree) = repo.find_commit(oid).and_then(|c| c.tree()) else {
                continue;
            };
            for (project, codes) in wanted {
                let seen = seen.entry(project.as_str()).or_default();
                history.read_project(&repo, &tree, project, codes, seen);
            }
        }
        Some(history)
    }

    fn read_project(
        &mut self,
        repo: &git2::Repository,
        tree: &git2::Tree,
        project: &str,
        codes: &BTreeSet<String>,
        seen: &mut HashSet<git2::Oid>,
    ) {
        let Ok(entry) = tree.get_path(Path::new("projects").join(project).as_path()) else {
            return;
        };
        let Ok(root) = repo.find_tree(entry.id()) else {
            return;
        };
        if let Some(text) = blob_once(repo, root.get_name("config.yaml").map(|e| e.id()), seen)
            && let Ok(config) = yaml::from_str::<ProjectConfig>(&text)
            && let Some(source) = &config.process
        {
            let key = (project.to_string(), crate::hash::canonical_json(source));
            self.processes
                .entry(key)
                .or_insert_with(|| crate::config::WorkflowFile::from_config(&config).content_rev());
        }
        // Status folders sit under `features/` (FEAT-071), or at the project root on an older board.
        let mut folders: Vec<git2::Oid> = Vec::new();
        for entry in root.iter() {
            if entry.kind() != Some(git2::ObjectType::Tree) {
                continue;
            }
            if entry.name().ok() == Some("features") {
                if let Ok(features) = repo.find_tree(entry.id()) {
                    folders.extend(
                        features
                            .iter()
                            .filter(|e| e.kind() == Some(git2::ObjectType::Tree))
                            .map(|e| e.id()),
                    );
                }
            } else {
                folders.push(entry.id());
            }
        }
        for folder in folders {
            let Ok(folder) = repo.find_tree(folder) else {
                continue;
            };
            for entry in folder.iter() {
                let Some(code) = entry.name().ok().and_then(|n| n.strip_suffix(".yaml")) else {
                    continue;
                };
                if !codes.contains(code) || entry.kind() != Some(git2::ObjectType::Blob) {
                    continue;
                }
                let Some(text) = blob_once(repo, Some(entry.id()), seen) else {
                    continue;
                };
                let Ok(meta) = yaml::from_str::<FeatureMeta>(&text) else {
                    continue;
                };
                if meta.code == code
                    && let Some(def) = &meta.definition
                {
                    self.record(project, code, def);
                }
            }
        }
    }

    fn record(&mut self, project: &str, code: &str, def: &FeatureDefinition) {
        let mut rev: Option<String> = None;
        let mut rev = || rev.get_or_insert_with(|| def.content_rev()).clone();
        let ats = def
            .approvals
            .iter()
            .map(|e| instant(&e.at))
            .chain(def.approval.iter().map(|a| instant(&a.at)));
        for at in ats {
            let key = (project.to_string(), code.to_string(), at);
            self.approvals.entry(key).or_insert_with(&mut rev);
        }
        for s in &def.signoffs {
            let key = (
                project.to_string(),
                code.to_string(),
                s.id.clone(),
                instant(&s.at),
            );
            self.signoffs.entry(key).or_insert_with(&mut rev);
        }
    }
}

/// The instant a verdict names, as a key: UTC, to the microsecond. A board whose files were
/// rewritten outside kanbanr can carry the same verdict's `at` in another form
/// (`…57.420470251Z` became `…57.420470+00:00` when a script re-serialised the files); keyed by
/// the text, the rewrite's commit would look like the one that recorded the verdict, and the
/// verdict would be pinned to whatever that commit changed. Text that is not a timestamp is its
/// own key.
fn instant(at: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    match time::OffsetDateTime::parse(at, &Rfc3339) {
        Ok(t) => {
            let t = t.to_offset(time::UtcOffset::UTC);
            format!("{}.{:06}", t.unix_timestamp(), t.microsecond())
        }
        Err(_) => at.to_string(),
    }
}

/// A blob's text, the first time it is asked for.
fn blob_once(
    repo: &git2::Repository,
    oid: Option<git2::Oid>,
    seen: &mut HashSet<git2::Oid>,
) -> Option<String> {
    let oid = oid?;
    if !seen.insert(oid) {
        return None;
    }
    let blob = repo.find_blob(oid).ok()?;
    std::str::from_utf8(blob.content()).ok().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ApprovalState;
    use serde_json::{Value, json};

    /// A copy of the v0.1.7 fixture board in the system temp dir, removed when dropped.
    struct Board {
        path: PathBuf,
    }

    impl Drop for Board {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// The fixture, with its history (`dot-git` becomes `.git`) or without any.
    fn fixture(history: bool) -> Board {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kanbanr-migrate-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        copy_dir(&crate::yaml::tests::v017_board(), &path);
        if history {
            std::fs::rename(path.join("dot-git"), path.join(".git")).unwrap();
        } else {
            std::fs::remove_dir_all(path.join("dot-git")).unwrap();
        }
        // L-41: the board written to is this test's own copy, in the temp dir, before any write.
        assert!(path.starts_with(std::env::temp_dir()), "{}", path.display());
        assert!(path.join("projects/demo/config.yaml").is_file());
        Board { path }
    }

    fn expected() -> Value {
        let text = std::fs::read_to_string(
            crate::yaml::tests::v017_board().join("../expected-states.json"),
        )
        .unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["projects"].clone()
    }

    /// What this build reports for every item and project, in expected-states.json's shape.
    fn states(store: &Store) -> Value {
        let mut out = serde_json::Map::new();
        for id in store.list_projects().unwrap() {
            let project = store.load(&id).unwrap();
            let mut items = serde_json::Map::new();
            for f in &project.features {
                let (approval, signoffs) = match &f.definition {
                    None => (ApprovalState::Missing, serde_json::Map::new()),
                    Some(def) => {
                        let names: BTreeSet<&str> =
                            def.signoffs.iter().map(|s| s.id.as_str()).collect();
                        let signoffs = names
                            .into_iter()
                            .map(|n| {
                                let state = if def.signoff_current(n).is_some() {
                                    "current"
                                } else {
                                    "lapsed"
                                };
                                (n.to_string(), json!(state))
                            })
                            .collect();
                        (def.approval_state(), signoffs)
                    }
                };
                items.insert(
                    f.code.clone(),
                    json!({ "approval": approval, "signoffs": signoffs }),
                );
            }
            let process = store
                .process_drift(&project, None)
                .unwrap()
                .map(|d| json!({ "edited": d.edited, "newer": d.newer, "gone": d.gone }));
            out.insert(id, json!({ "items": items, "process": process }));
        }
        Value::Object(out)
    }

    fn commits(dir: &Path) -> Vec<String> {
        let repo = git2::Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.flatten()
            .map(|oid| {
                repo.find_commit(oid)
                    .unwrap()
                    .summary()
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    /// FEAT-187 R-4: on a board kanbanr 0.1.7 made, every approval and sign-off that was current is
    /// current after the migration, every lapsed one is still lapsed, and so is every process's
    /// drift — in exactly one commit.
    #[test]
    fn migration_keeps_current_approvals_current() {
        let board = fixture(true);
        let store = Store::new(&board.path);
        let before = commits(&board.path);
        assert_eq!(before.len(), 72, "the fixture's history");

        // Until it is migrated, the board is refused with the remedy.
        let refused = store.load("demo").unwrap_err();
        assert!(
            matches!(
                refused,
                CoreError::NeedsMigration {
                    found: 2,
                    needed: 5
                }
            ),
            "{refused:?}"
        );
        assert!(refused.to_string().contains("kanbanr migrate"), "{refused}");
        assert!(needed(&store).unwrap());

        let report = run(&store, &board.path, PushPolicy::Off).unwrap();
        assert!(report.committed && report.history, "{report:?}");
        assert!(report.unverified.is_empty(), "{:?}", report.unverified);
        assert_eq!(report.projects.len(), 4);
        assert!(report.projects.iter().all(|p| p.from == 2));
        assert!(report.repinned > 20, "{report:?}");
        assert!(report.files_rewritten > 0);

        let after = commits(&board.path);
        assert_eq!(after.len(), before.len() + 1, "exactly one commit");
        assert_eq!(after[0], COMMIT_MESSAGE);
        let repo = git2::Repository::open(&board.path).unwrap();
        let dirty = repo.statuses(None).unwrap();
        let dirty: Vec<String> = dirty
            .iter()
            .filter(|e| !e.status().is_ignored())
            .map(|e| format!("{:?} {}", e.status(), e.path().unwrap_or("")))
            .collect();
        assert!(dirty.is_empty(), "everything is in the commit: {dirty:?}");

        assert_eq!(states(&store), expected());
        for id in store.list_projects().unwrap() {
            assert_eq!(store.load(&id).unwrap().config.schema_version, 5);
        }
        assert!(
            !crate::doctor::run(&store)
                .unwrap()
                .issues
                .iter()
                .any(|i| i.message.contains("could not be verified")),
        );

        // Once is enough: a migrated board has nothing to migrate, and no commit is made.
        let again = run(&store, &board.path, PushPolicy::Off).unwrap();
        assert!(again.projects.is_empty() && !again.committed, "{again:?}");
        assert_eq!(commits(&board.path).len(), after.len());
    }

    /// FEAT-187 R-4: a definition reworded outside kanbanr after it was approved stays lapsed, even
    /// when the same rewrite re-serialised the approval's timestamp (as a script re-writing the
    /// files does: `…05.210046972Z` becomes `…05.210046+00:00`). Keyed by its text, the rewrite's
    /// commit looked like the one that recorded the approval, and the approval came out current.
    #[test]
    fn a_hand_rewrite_after_approval_stays_lapsed() {
        let board = fixture(true);
        let file = board
            .path
            .join("projects/demo/features/Planned/FEAT-001.yaml");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("statement: Approved and current"),
            "the fixture item"
        );
        let rewritten = text
            .replace(
                "statement: Approved and current",
                "statement: Approved, then reworded",
            )
            .replace("05.210046972Z", "05.210046+00:00");
        assert_ne!(rewritten, text);
        std::fs::write(&file, rewritten).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-c", "user.name=T", "-c", "user.email=t@x", "-C"])
                .arg(&board.path)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "reword by hand"]);

        let store = Store::new(&board.path);
        run(&store, &board.path, PushPolicy::Off).unwrap();
        let mut want = expected();
        want["demo"]["items"]["FEAT-001"]["approval"] = Value::from("lapsed");
        assert_eq!(states(&store), want);
    }

    /// FEAT-187 R-5: with no history to read, nothing is trusted. Every revision reads as lapsed,
    /// the report counts each, and doctor names the items to give again.
    #[test]
    fn migration_without_history_lapses_and_reports() {
        let board = fixture(false);
        let store = Store::new(&board.path);
        let report = migrate_to_v5(&store).unwrap();
        assert!(!report.history);
        assert_eq!(report.repinned, 0);
        // Standing approvals 11, latest sign-offs given 4, projects on a process 3.
        assert_eq!(
            report.unverified_current().count(),
            18,
            "{:#?}",
            report.unverified_current().collect::<Vec<_>>()
        );
        assert!(
            report.unverified.len() > 18,
            "older log entries are counted too"
        );

        let mut want = expected();
        for project in want.as_object_mut().unwrap().values_mut() {
            for item in project["items"].as_object_mut().unwrap().values_mut() {
                if item["approval"] != "missing" {
                    item["approval"] = json!("lapsed");
                }
                for state in item["signoffs"].as_object_mut().unwrap().values_mut() {
                    *state = json!("lapsed");
                }
            }
            if project["process"].is_object() {
                project["process"]["edited"] = json!(true);
                // The saved process no longer matches what was applied, as far as anyone can tell.
                let version = if project["process"]["newer"].is_null() {
                    1
                } else {
                    2
                };
                project["process"]["newer"] = json!(version);
            }
        }
        assert_eq!(states(&store), want);

        let doctor = crate::doctor::run(&store).unwrap();
        let named = |code: &str, text: &str| {
            doctor.issues.iter().any(|i| {
                i.code.as_deref() == Some(code)
                    && i.message.contains(text)
                    && i.message
                        .contains("could not be verified during the schema-5 migration")
            })
        };
        assert!(named("FEAT-001", "approval of FEAT-001"));
        assert!(named("FEAT-014", "sign-off 'release' of FEAT-014"));
        assert!(
            !named("FEAT-003", "approval of FEAT-003"),
            "nothing stands to verify"
        );
        assert!(doctor.issues.iter().any(|i| {
            i.project == "flow"
                && i.code.is_none()
                && i.message
                    .contains("process this project's workflow came from")
        }));
    }

    /// FEAT-187 R-6: a migrated board says schema 5, which a reader of schema 4 (or older) refuses
    /// by name rather than reading every approval as lapsed.
    #[test]
    fn a_migrated_board_is_too_new_for_older_readers() {
        let board = fixture(true);
        let store = Store::new(&board.path);
        migrate_to_v5(&store).unwrap();
        let config = store.load("demo").unwrap().config;
        assert_eq!(config.schema_version, 5);
        let text = std::fs::read_to_string(board.path.join("projects/demo/config.yaml")).unwrap();
        assert!(text.starts_with("schema_version: 5\n"), "{text}");
        assert!(matches!(
            crate::config::check_schema(config.schema_version, 4),
            Err(CoreError::SchemaTooNew {
                found: 5,
                understood: 4
            })
        ));
        assert!(crate::config::check_schema(config.schema_version, CURRENT_SCHEMA_VERSION).is_ok());
        assert!(matches!(
            crate::config::check_schema(4, CURRENT_SCHEMA_VERSION),
            Err(CoreError::NeedsMigration { found: 4, .. })
        ));
    }
}
