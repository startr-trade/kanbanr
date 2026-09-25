//! YAML-backed store. Loads/saves projects and implements all mutating operations.
//! The CLI calls the mutating methods; the server only reads.

use crate::config::ProjectConfig;
use crate::docs::{DocFile, DocFolder, FOLDER_META, FolderMeta};
use crate::error::{CoreError, Result};
use crate::models::{FeatureItem, IndexEntry, Milestone, Task, TaskState, TodoList};
use crate::{docs, now_rfc3339, validate};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// A fully-loaded project (config + all entities).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Project {
    pub id: String,
    pub config: ProjectConfig,
    pub features: Vec<FeatureItem>,
    pub milestones: Vec<Milestone>,
}

impl Project {
    pub fn feature(&self, code: &str) -> Result<&FeatureItem> {
        self.features
            .iter()
            .find(|f| f.code == code)
            .ok_or_else(|| CoreError::FeatureNotFound(code.to_string()))
    }
    fn feature_mut(&mut self, code: &str) -> Result<&mut FeatureItem> {
        self.features
            .iter_mut()
            .find(|f| f.code == code)
            .ok_or_else(|| CoreError::FeatureNotFound(code.to_string()))
    }
    pub fn milestone(&self, code: &str) -> Result<&Milestone> {
        self.milestones
            .iter()
            .find(|m| m.code == code)
            .ok_or_else(|| CoreError::MilestoneNotFound(code.to_string()))
    }
}

/// Deferred-write tracker. A single op (or a whole `apply_batch`) loads the project **once**,
/// mutates it in memory, and records here which entities to (re)write and which stale feature
/// files to drop; `Store::flush` then performs the minimal set of disk writes. This replaces the
/// old reload-and-persist-per-mutation pattern (a K-op batch used to mean K+ full project loads).
#[derive(Default)]
struct Pending {
    /// Feature codes whose current in-memory state must be written.
    persist_features: std::collections::BTreeSet<String>,
    /// Stale on-disk feature locations `(status, code)` to remove (left by a move/rename).
    remove_features: std::collections::BTreeSet<(String, String)>,
    /// Milestone codes to (re)write.
    persist_milestones: std::collections::BTreeSet<String>,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// The data folder this store owns. The activity and events logs live beside the projects
    /// rather than inside them, so callers that read those need it.
    pub fn data_dir(&self) -> &Path {
        &self.root
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.projects_dir().join(id)
    }
    fn milestones_dir(&self, id: &str) -> PathBuf {
        self.project_dir(id).join("milestones")
    }

    pub fn project_exists(&self, id: &str) -> bool {
        self.project_dir(id).join("config.yaml").is_file()
    }

    /// List project ids (folders containing a config.yaml), sorted.
    pub fn list_projects(&self) -> Result<Vec<String>> {
        let dir = self.projects_dir();
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
                && self.project_exists(name)
            {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    // ---- raw yaml helpers ----------------------------------------------------------------

    fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&text)?)
    }

    fn write_yaml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(value)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    fn read_dir_yaml<T: DeserializeOwned>(dir: &Path) -> Result<Vec<T>> {
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .map(|e| e == "yaml" || e == "yml")
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for p in paths {
            out.push(Self::read_yaml(&p)?);
        }
        Ok(out)
    }

    // ---- load ----------------------------------------------------------------------------

    /// Load a project **fully**: every feature carries its `specification` body (read from the
    /// co-located `.md`). This is the canonical, spec-populated load — its public contract is
    /// relied on by dispatch (`GET project`), export, `feature show`, and the daemon, so it must
    /// not change. Cheap callers that never touch spec bodies should prefer `load_meta` (FEAT-033).
    pub fn load(&self, id: &str) -> Result<Project> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let config: ProjectConfig = Self::read_yaml(&self.project_dir(id).join("config.yaml"))?;
        let features = self.read_features(id, &config.statuses)?;
        let milestones = Self::read_dir_yaml(&self.milestones_dir(id))?;
        Ok(Project {
            id: id.to_string(),
            config,
            features,
            milestones,
        })
    }

    /// Load a project's **metadata only**: every feature has `specification: String::new()` and no
    /// `.md` files are read. Use this for board/list/graph/rollup callers that never need spec
    /// bodies — it avoids one file read per feature. Pair with `feature_spec` to fetch a single
    /// spec on demand. The shape (`Project`) is identical to `load` so callers are interchangeable;
    /// the only difference is that feature spec bodies are empty (FEAT-033).
    pub fn load_meta(&self, id: &str) -> Result<Project> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let config: ProjectConfig = Self::read_yaml(&self.project_dir(id).join("config.yaml"))?;
        let features = self
            .read_feature_metas(id, &config.statuses)?
            .into_iter()
            .map(|(_status, meta)| FeatureItem::from_meta(meta, String::new()))
            .collect();
        let milestones = Self::read_dir_yaml(&self.milestones_dir(id))?;
        Ok(Project {
            id: id.to_string(),
            config,
            features,
            milestones,
        })
    }

    /// Load a single feature's specification body on demand. Scans the configured status folders
    /// for the feature's metadata to discover which `<status>/features-spec/<code>.md` to read.
    /// Returns the feature-not-found error if no metadata file for `code` exists. (FEAT-033)
    pub fn feature_spec(&self, id: &str, code: &str) -> Result<String> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let config: ProjectConfig = Self::read_yaml(&self.project_dir(id).join("config.yaml"))?;
        for (status, meta) in self.read_feature_metas(id, &config.statuses)? {
            if meta.code == code {
                let spec_path = self.spec_path(id, &status, code);
                return Ok(std::fs::read_to_string(&spec_path).unwrap_or_default());
            }
        }
        Err(CoreError::FeatureNotFound(code.to_string()))
    }

    // ---- per-project index cache (FEAT-033) ----------------------------------------------
    //
    // `<project>/index.yaml` is a compact, spec-free list of `IndexEntry` rows — one per feature.
    // It is a *cache*: the status-folder yaml/md files remain the source of truth, the index is
    // always rebuildable from them, and an absent/stale index is never fatal (callers fall back to
    // building it from `load_meta`). It is refreshed inside `flush` so it stays current after any
    // feature mutation, and it lives in the (committed) data repo, which is fine.

    fn index_path(&self, id: &str) -> PathBuf {
        self.project_dir(id).join("index.yaml")
    }

    /// Build the index rows for a project from its metadata (source of truth), ordered as
    /// `load_meta` returns features (configured-status order, then sorted within a status).
    fn build_index_entries(&self, id: &str) -> Result<Vec<IndexEntry>> {
        Ok(self
            .load_meta(id)?
            .features
            .iter()
            .map(IndexEntry::from_feature)
            .collect())
    }

    /// (Re)write `<project>/index.yaml` from the given rows.
    fn write_index(&self, id: &str, entries: &[IndexEntry]) -> Result<()> {
        Self::write_yaml(&self.index_path(id), &entries.to_vec())
    }

    /// Rebuild and persist the per-project index from the source-of-truth feature files. (FEAT-033)
    pub fn rebuild_index(&self, id: &str) -> Result<()> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let entries = self.build_index_entries(id)?;
        self.write_index(id, &entries)
    }

    /// Load the per-project index. Reads the single `index.yaml` if present; if it is missing it is
    /// built from `load_meta` and written, then returned. Treat the result as a cheap view — it
    /// carries no spec bodies. (FEAT-033)
    pub fn load_index(&self, id: &str) -> Result<Vec<IndexEntry>> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let path = self.index_path(id);
        if path.is_file() {
            // A corrupt/legacy index is non-fatal: fall back to rebuilding from the source of truth.
            if let Ok(entries) = Self::read_yaml::<Vec<IndexEntry>>(&path) {
                return Ok(entries);
            }
        }
        let entries = self.build_index_entries(id)?;
        self.write_index(id, &entries)?;
        Ok(entries)
    }

    // ---- project ops ---------------------------------------------------------------------

    pub fn init_project(&self, id: &str, config: ProjectConfig) -> Result<Project> {
        if !validate::valid_name(id) {
            return Err(CoreError::InvalidName(id.to_string()));
        }
        if self.project_exists(id) {
            return Err(CoreError::ProjectExists(id.to_string()));
        }
        if !config.default_state.is_empty() && !config.has_status(&config.default_state) {
            return Err(CoreError::UnknownStatus(config.default_state.clone()));
        }
        for s in &config.no_op_states {
            if !config.has_status(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
            if config.displayed_states.contains(s) {
                return Err(CoreError::DisplayedNoOp(s.clone()));
            }
        }
        std::fs::create_dir_all(self.milestones_dir(id))?;
        // A folder per status so the on-disk layout mirrors the kanban columns from the start.
        for status in &config.statuses {
            std::fs::create_dir_all(self.project_dir(id).join(status))?;
        }
        self.save_config(id, &config)?;
        self.load(id)
    }

    pub fn save_config(&self, id: &str, config: &ProjectConfig) -> Result<()> {
        // Forward-stamp the schema version on every write (minimal, non-destructive migration): a
        // config that loaded as a legacy version (0) is brought up to current on its next save.
        let mut config = config.clone();
        config.schema_version = crate::config::CURRENT_SCHEMA_VERSION;
        Self::write_yaml(&self.project_dir(id).join("config.yaml"), &config)
    }

    /// Feature metadata yaml lives under `<status>/<code>.yaml` (status folder at project root).
    fn feature_path(&self, id: &str, status: &str, code: &str) -> PathBuf {
        self.project_dir(id)
            .join(status)
            .join(format!("{code}.yaml"))
    }
    /// Feature specification markdown lives under `<status>/features-spec/<code>.md`,
    /// co-located within the status folder so it moves together with the metadata.
    fn spec_path(&self, id: &str, status: &str, code: &str) -> PathBuf {
        self.project_dir(id)
            .join(status)
            .join("features-spec")
            .join(format!("{code}.md"))
    }
    fn milestone_path(&self, id: &str, code: &str) -> PathBuf {
        self.milestones_dir(id).join(format!("{code}.yaml"))
    }

    /// Write a feature's metadata (yaml) and specification (md) under its current status folder.
    fn persist_feature(&self, id: &str, f: &FeatureItem) -> Result<()> {
        Self::write_yaml(&self.feature_path(id, &f.status, &f.code), &f.meta())?;
        let spec = self.spec_path(id, &f.status, &f.code);
        if let Some(parent) = spec.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&spec, &f.specification)?;
        Ok(())
    }

    /// Remove a feature's files at a given (status, code) location.
    fn remove_feature_files(&self, id: &str, status: &str, code: &str) {
        let _ = std::fs::remove_file(self.feature_path(id, status, code));
        let _ = std::fs::remove_file(self.spec_path(id, status, code));
    }

    /// Read feature **metadata only** by scanning each configured status folder `<status>/*.yaml`
    /// (the source of truth), returning each paired with the status folder it was found in. No
    /// `.md` spec files are read. Driving the scan from the configured status list keeps status
    /// folders from colliding with `milestones/`, `schedules/`, `docs/`, etc. (FEAT-033)
    fn read_feature_metas(
        &self,
        id: &str,
        statuses: &[String],
    ) -> Result<Vec<(String, crate::models::FeatureMeta)>> {
        let mut out = Vec::new();
        for status in statuses {
            let sdir = self.project_dir(id).join(status);
            if !sdir.is_dir() {
                continue;
            }
            let mut files: Vec<PathBuf> = std::fs::read_dir(&sdir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension()
                        .map(|e| e == "yaml" || e == "yml")
                        .unwrap_or(false)
                })
                .collect();
            files.sort();
            for path in files {
                let meta: crate::models::FeatureMeta = Self::read_yaml(&path)?;
                out.push((status.clone(), meta));
            }
        }
        Ok(out)
    }

    /// Load all features by reading their metadata (`read_feature_metas`) and pairing each with its
    /// `<status>/features-spec/<code>.md` specification body. This is the spec-populated path used
    /// by `load`.
    fn read_features(&self, id: &str, statuses: &[String]) -> Result<Vec<FeatureItem>> {
        let mut out = Vec::new();
        for (status, meta) in self.read_feature_metas(id, statuses)? {
            let spec_path = self.spec_path(id, &status, &meta.code);
            let spec = std::fs::read_to_string(&spec_path).unwrap_or_default();
            out.push(FeatureItem::from_meta(meta, spec));
        }
        Ok(out)
    }

    /// Persist everything a (possibly batched) set of in-memory mutations changed: (re)write each
    /// touched feature/milestone from the in-memory `project`, then drop any stale feature files a
    /// move/rename left behind. Writes happen **before** removals so a status/code change never
    /// deletes the file it just wrote, and a stale location still occupied by a feature is skipped.
    fn flush(&self, id: &str, project: &Project, pending: &Pending) -> Result<()> {
        for code in &pending.persist_features {
            if let Some(f) = project.features.iter().find(|f| &f.code == code) {
                self.persist_feature(id, f)?;
            }
        }
        for (status, code) in &pending.remove_features {
            let occupied = project
                .features
                .iter()
                .any(|f| &f.code == code && &f.status == status);
            if !occupied {
                self.remove_feature_files(id, status, code);
            }
        }
        for code in &pending.persist_milestones {
            if let Some(m) = project.milestones.iter().find(|m| &m.code == code) {
                Self::write_yaml(&self.milestone_path(id, code), m)?;
            }
        }
        // Refresh the per-project index cache after feature/milestone files are written, so it
        // stays current following any mutation. The in-memory `project` is the just-applied state
        // (already reflecting moves/removals), so we derive the index from it directly rather than
        // re-reading from disk (FEAT-033).
        let entries: Vec<IndexEntry> = project
            .features
            .iter()
            .map(IndexEntry::from_feature)
            .collect();
        self.write_index(id, &entries)?;
        Ok(())
    }

    // ---- feature ops ---------------------------------------------------------------------
    //
    // Each mutating op is split into a pure in-memory core (`*_on`, operating on `&mut Project` +
    // a `Pending`) and a thin public wrapper that loads once, applies the core, and flushes. The
    // cores are what `apply_batch` chains against a single loaded project (FEAT-028).

    pub fn add_feature(
        &self,
        id: &str,
        title: &str,
        specification: &str,
        milestone: &str,
        code: Option<String>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::add_feature_on(
            &mut project,
            &mut pending,
            title,
            specification,
            milestone,
            code,
        )?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    fn add_feature_on(
        project: &mut Project,
        pending: &mut Pending,
        title: &str,
        specification: &str,
        milestone: &str,
        code: Option<String>,
    ) -> Result<FeatureItem> {
        // A milestone is required and must exist.
        if milestone.is_empty() {
            return Err(CoreError::MilestoneRequired);
        }
        project.milestone(milestone)?;
        let existing: Vec<String> = project.features.iter().map(|f| f.code.clone()).collect();
        let code = match code {
            Some(c) => c,
            None => validate::next_code("FEAT", &existing),
        };
        if !validate::valid_name(&code) {
            return Err(CoreError::InvalidName(code));
        }
        if existing.iter().any(|c| c == &code) {
            return Err(CoreError::FeatureExists(code));
        }
        let now = now_rfc3339();
        let feature = FeatureItem {
            code: code.clone(),
            title: title.to_string(),
            specification: specification.to_string(),
            status: project.config.default_status(),
            milestone: milestone.to_string(),
            kind: None,
            priority: None,
            start: None,
            due: None,
            estimate_days: None,
            assignee: None,
            team: None,
            labels: Vec::new(),
            depends_on: Vec::new(),
            todo_lists: Vec::new(),
            source: None,
            issue: None,
            definition: None,
            defect: None,
            split_from: None,
            history: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        project.features.push(feature.clone());
        pending.persist_features.insert(code);
        Ok(feature)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_feature(
        &self,
        id: &str,
        code: &str,
        title: Option<String>,
        specification: Option<String>,
        milestone: Option<Option<String>>,
        new_code: Option<String>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::edit_feature_on(
            &mut project,
            &mut pending,
            code,
            title,
            specification,
            milestone,
            new_code,
        )?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_feature_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        title: Option<String>,
        specification: Option<String>,
        milestone: Option<Option<String>>,
        new_code: Option<String>,
    ) -> Result<FeatureItem> {
        // Validate milestone reference up front. A feature must keep a milestone, so a
        // detach (Some(None)) is rejected.
        match &milestone {
            Some(Some(ms)) => {
                project.milestone(ms)?;
            }
            Some(None) => return Err(CoreError::MilestoneRequired),
            None => {}
        }
        let rename = match &new_code {
            Some(nc) if nc != code => {
                if !validate::valid_name(nc) {
                    return Err(CoreError::InvalidName(nc.clone()));
                }
                if project.features.iter().any(|f| &f.code == nc) {
                    return Err(CoreError::FeatureExists(nc.clone()));
                }
                Some(nc.clone())
            }
            _ => None,
        };
        let feature = project.feature_mut(code)?;
        if let Some(t) = title {
            feature.title = t;
        }
        if let Some(s) = specification {
            feature.specification = s;
        }
        if let Some(Some(ms)) = milestone {
            feature.milestone = ms;
        }
        if let Some(nc) = &rename {
            feature.code = nc.clone();
        }
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        if rename.is_some() {
            // The renamed feature is written at its new code; drop the old-code files (same status).
            pending
                .remove_features
                .insert((updated.status.clone(), code.to_string()));
        }
        Ok(updated)
    }

    // NOTE: there is intentionally no `delete_feature`. Feature items are the project's work and
    // are permanent; this is what locks a project that has any features from being deleted.

    /// Set a feature's optional attributes. Each `Some` replaces (for kind/priority/due an empty
    /// string clears the field); `None` leaves it unchanged. `depends_on` is validated: every
    /// referenced feature must exist and the cross-feature graph must stay acyclic.
    #[allow(clippy::too_many_arguments)]
    pub fn set_feature_attrs(
        &self,
        id: &str,
        code: &str,
        kind: Option<String>,
        priority: Option<String>,
        due: Option<String>,
        assignee: Option<String>,
        team: Option<String>,
        labels: Option<Vec<String>>,
        depends_on: Option<Vec<String>>,
    ) -> Result<FeatureItem> {
        let deps_changed = depends_on.is_some();
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::set_feature_attrs_on(
            &mut project,
            &mut pending,
            code,
            kind,
            priority,
            due,
            assignee,
            team,
            labels,
            depends_on,
        )?;
        // When dependencies changed, validate them across the whole portfolio (qualified
        // `project:code` refs must resolve and the global graph must stay acyclic) before persisting.
        if deps_changed {
            crate::graph::validate_feature_deps(self, &project, id, code)?;
        }
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_feature_attrs_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        kind: Option<String>,
        priority: Option<String>,
        due: Option<String>,
        assignee: Option<String>,
        team: Option<String>,
        labels: Option<Vec<String>>,
        depends_on: Option<Vec<String>>,
    ) -> Result<FeatureItem> {
        project.feature(code)?; // ensure it exists

        if let Some(deps) = &depends_on {
            if deps.iter().any(|d| d == code) {
                return Err(CoreError::DependencyCycle(code.to_string()));
            }
            // Validate against the would-be graph before touching anything.
            let mut features = project.features.clone();
            if let Some(f) = features.iter_mut().find(|f| f.code == code) {
                f.depends_on = deps.clone();
            }
            validate::validate_feature_dependencies(&features, code)?;
        }

        let clean = |s: String| -> Option<String> {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        let feature = project.feature_mut(code)?;
        if let Some(k) = kind {
            feature.kind = clean(k);
        }
        if let Some(p) = priority {
            feature.priority = clean(p);
        }
        if let Some(d) = due {
            feature.due = clean(d);
        }
        if let Some(a) = assignee {
            feature.assignee = clean(a);
        }
        if let Some(t) = team {
            feature.team = clean(t);
        }
        if let Some(l) = labels {
            feature.labels = l.into_iter().filter(|s| !s.trim().is_empty()).collect();
        }
        if let Some(deps) = depends_on {
            feature.depends_on = deps.into_iter().filter(|s| !s.trim().is_empty()).collect();
        }
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        Ok(updated)
    }

    /// Replace a feature's definition — why it exists, what must be true, how it is verified
    /// (FEAT-047). `None` clears it. Kept off `set_feature_attrs` for the same reason
    /// `set_feature_schedule` is: that signature is already wide, and this is a block, not an
    /// attribute. Requirement ids left blank are assigned here, so the CLI and the batch path
    /// behave identically.
    pub fn set_feature_definition(
        &self,
        id: &str,
        code: &str,
        definition: Option<crate::models::FeatureDefinition>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::set_feature_definition_on(&mut project, &mut pending, code, definition)?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    /// Record what a defect cost and where it came from (FEAT-053). `None` clears the block.
    pub fn set_defect(
        &self,
        id: &str,
        code: &str,
        defect: Option<crate::models::Defect>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::set_defect_on(&mut project, &mut pending, code, defect)?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    /// Record that an item was sliced out of another (FEAT-054). `None` clears it.
    pub fn set_split_from(
        &self,
        id: &str,
        code: &str,
        parent: Option<String>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        // A parent that does not exist would make the wave's scope-growth account fiction.
        if let Some(parent) = parent.as_deref().filter(|p| !p.trim().is_empty()) {
            project.feature(parent)?;
        }
        let feature = project.feature_mut(code)?;
        feature.split_from = parent.filter(|p| !p.trim().is_empty());
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        self.flush(id, &project, &pending)?;
        Ok(updated)
    }

    fn set_defect_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        defect: Option<crate::models::Defect>,
    ) -> Result<FeatureItem> {
        // Whether a defect escaped is a fact about the board, not an opinion: it escaped if the
        // work that introduced it had already been called done when this was recorded. Deriving it
        // keeps the one quality ratio the board publishes out of reach of wishful self-reporting;
        // an explicit `true` still stands, for a defect whose origin is outside the board.
        let defect = defect.map(|mut d| {
            if !d.escaped
                && !d.introduced_by.trim().is_empty()
                && let Ok(origin) = project.feature(d.introduced_by.trim())
            {
                d.escaped = crate::graph::is_terminal_status(&project.config, &origin.status);
            }
            d
        });
        let feature = project.feature_mut(code)?;
        feature.defect = defect;
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        Ok(updated)
    }

    fn set_feature_definition_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        definition: Option<crate::models::FeatureDefinition>,
    ) -> Result<FeatureItem> {
        let previous = project.feature(code)?.definition.clone();
        let definition = definition.map(|mut def| {
            assign_requirement_ids(&mut def);
            // Carry the approval record forward: a redefinition must LAPSE the approval, not erase
            // it. Erasing would lose who agreed to what, and would report "never approved" for an
            // item whose scope simply changed after a yes — the exact distinction that matters.
            if let Some(prev) = previous.as_ref() {
                if def.approval.is_none() {
                    def.approval = prev.approval.clone();
                }
                if def.started_unapproved.trim().is_empty() {
                    def.started_unapproved = prev.started_unapproved.clone();
                }
            }
            def
        });
        let feature = project.feature_mut(code)?;
        feature.definition = definition;
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        Ok(updated)
    }

    /// Set a feature's scheduling attributes (FEAT-035), kept on a dedicated path so the broad
    /// `set_feature_attrs` signature (and its many call sites) is left untouched. `start` is an ISO
    /// date string (a `Some("")` clears it). `estimate` is effort in days (`Some(<= 0)` clears it).
    /// Either argument being `None` leaves that field unchanged.
    pub fn set_feature_schedule(
        &self,
        id: &str,
        code: &str,
        start: Option<String>,
        estimate: Option<f64>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        project.feature(code)?; // ensure it exists
        let feature = project.feature_mut(code)?;
        if let Some(s) = start {
            let t = s.trim();
            feature.start = (!t.is_empty()).then(|| t.to_string());
        }
        if let Some(e) = estimate {
            feature.estimate_days = (e > 0.0).then_some(e);
        }
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        self.flush(id, &project, &pending)?;
        Ok(updated)
    }

    /// Move one test along the TDD lifecycle: planned → red → green (FEAT-051).
    ///
    /// Small and targeted on purpose: flipping a state must not require re-sending the whole
    /// definition, or the prose gets rewritten (and garbled) on every red-to-green cycle.
    /// `checked_rev` records the project revision the result was observed at, so a green that has
    /// since gone stale can be told from one that still holds.
    pub fn set_test_state(
        &self,
        id: &str,
        code: &str,
        requirement: &str,
        test: &str,
        state: crate::models::TestState,
        checked_rev: Option<&str>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::set_test_state_on(
            &mut project,
            &mut pending,
            code,
            requirement,
            test,
            state,
            checked_rev,
        )?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_test_state_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        requirement: &str,
        test: &str,
        state: crate::models::TestState,
        checked_rev: Option<&str>,
    ) -> Result<FeatureItem> {
        let feature = project.feature_mut(code)?;
        let definition = feature
            .definition
            .as_mut()
            .ok_or_else(|| CoreError::Unsupported(format!("{code} has no definition")))?;
        let requirement_entry = definition
            .requirements
            .iter_mut()
            .find(|r| r.id == requirement)
            .ok_or_else(|| {
                CoreError::Unsupported(format!("{code} has no requirement '{requirement}'"))
            })?;
        let test_entry = requirement_entry
            .tests
            .iter_mut()
            .find(|t| t.name == test)
            .ok_or_else(|| {
                CoreError::Unsupported(format!(
                    "requirement {requirement} of {code} has no test '{test}'"
                ))
            })?;
        test_entry.state = state;
        if let Some(rev) = checked_rev {
            test_entry.checked_rev = rev.trim().to_string();
        }
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        Ok(updated)
    }

    /// Record agreement to an item's definition as it currently stands (FEAT-048).
    pub fn approve_feature(&self, id: &str, code: &str, by: &str) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let feature = project.feature_mut(code)?;
        let definition = feature.definition.as_mut().ok_or_else(|| {
            CoreError::Unsupported(format!(
                "{code} has no definition to approve — write one with `kanbanr feature define`"
            ))
        })?;
        definition.approval = Some(crate::models::Approval {
            by: by.to_string(),
            at: now_rfc3339(),
            rev: definition.content_rev(),
        });
        feature.updated_at = now_rfc3339();
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        self.flush(id, &project, &pending)?;
        Ok(updated)
    }

    /// Move a feature, refusing to start work whose reasoning nobody agreed to (FEAT-048).
    /// `unapproved` records an explicit reason to go ahead anyway.
    pub fn move_feature_approved(
        &self,
        id: &str,
        code: &str,
        to: &str,
        unapproved: Option<&str>,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let charter = crate::charter::load(self, id)?;
        Self::check_start_gate(&project, &charter, code, to, unapproved)?;
        if let Some(reason) = unapproved.filter(|r| !r.trim().is_empty())
            && let Ok(feature) = project.feature_mut(code)
            && let Some(def) = feature.definition.as_mut()
        {
            def.started_unapproved = reason.trim().to_string();
        }
        let f = Self::move_feature_on(&mut project, &mut pending, code, to)?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    /// The start gate: entering an **active** status — one that is neither the default backlog
    /// state, nor terminal, nor a no-op disposition — requires a current approval. Closing or
    /// dispositioning an item is never gated: you can always stop work.
    fn check_start_gate(
        project: &Project,
        charter: &crate::Charter,
        code: &str,
        to: &str,
        unapproved: Option<&str>,
    ) -> Result<()> {
        // Adoption is never retroactive. A project with no charter has not taken up the method, and
        // items created before the charter was adopted predate it — gating either would make
        // upgrading kanbanr break every board in existence, which no amount of rigour justifies.
        if charter.adopted_at.trim().is_empty() {
            return Ok(());
        }
        if project
            .feature(code)
            .is_ok_and(|f| f.created_at.as_str() < charter.adopted_at.as_str())
        {
            return Ok(());
        }
        let config = &project.config;
        let gated = to != config.default_state
            && !crate::graph::is_terminal_status(config, to)
            && !config.is_no_op(to);
        if !gated || unapproved.is_some_and(|r| !r.trim().is_empty()) {
            return Ok(());
        }
        let feature = project.feature(code)?;
        let missing = match feature.definition.as_ref() {
            None => "it has no definition (run `kanbanr feature define`)".to_string(),
            Some(def) => match def.approval_state() {
                crate::models::ApprovalState::Current => return Ok(()),
                crate::models::ApprovalState::Missing => {
                    "its definition is not approved (review it, then `kanbanr approve`)".to_string()
                }
                crate::models::ApprovalState::Lapsed => {
                    "its approval lapsed — the definition changed after it was approved, so \
                     re-approve what is now proposed"
                        .to_string()
                }
            },
        };
        Err(CoreError::Unsupported(format!(
            "cannot move {code} to '{to}': {missing}. To proceed anyway, record why with \
             --unapproved \"<reason>\""
        )))
    }

    pub fn move_feature(&self, id: &str, code: &str, to: &str) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::move_feature_on(&mut project, &mut pending, code, to)?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    fn move_feature_on(
        project: &mut Project,
        pending: &mut Pending,
        code: &str,
        to: &str,
    ) -> Result<FeatureItem> {
        if !project.config.has_status(to) {
            return Err(CoreError::UnknownStatus(to.to_string()));
        }
        let config = project.config.clone();
        let feature = project.feature_mut(code)?;
        let from = feature.status.clone();
        if !config.transition_allowed(&from, to) {
            return Err(CoreError::TransitionNotAllowed {
                from,
                to: to.to_string(),
            });
        }
        let at = now_rfc3339();
        if from != to {
            feature.history.push(crate::models::Transition {
                at: at.clone(),
                from: from.clone(),
                to: to.to_string(),
            });
        }
        feature.status = to.to_string();
        feature.updated_at = at;
        let updated = feature.clone();
        pending.persist_features.insert(updated.code.clone());
        if from != to {
            pending.remove_features.insert((from, code.to_string()));
        }
        Ok(updated)
    }

    // ---- todo-list & task ops ------------------------------------------------------------

    /// Add a persistent todo-list to a feature (codes auto-generate as TL-001, TL-002, …).
    pub fn add_todo_list(
        &self,
        id: &str,
        feature: &str,
        description: &str,
        todo_code: Option<String>,
    ) -> Result<TodoList> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let l =
            Self::add_todo_list_on(&mut project, &mut pending, feature, description, todo_code)?;
        self.flush(id, &project, &pending)?;
        Ok(l)
    }

    fn add_todo_list_on(
        project: &mut Project,
        pending: &mut Pending,
        feature: &str,
        description: &str,
        todo_code: Option<String>,
    ) -> Result<TodoList> {
        let f = project.feature_mut(feature)?;
        let existing: Vec<String> = f.todo_lists.iter().map(|l| l.code.clone()).collect();
        let todo_code = match todo_code {
            Some(c) => c,
            None => validate::next_code("TL", &existing),
        };
        if !validate::valid_name(&todo_code) {
            return Err(CoreError::InvalidName(todo_code));
        }
        if existing.iter().any(|c| c == &todo_code) {
            return Err(CoreError::TodoListExists(todo_code, feature.to_string()));
        }
        let list = TodoList {
            code: todo_code,
            description: description.to_string(),
            tasks: Vec::new(),
            created_at: now_rfc3339(),
        };
        f.todo_lists.push(list.clone());
        f.updated_at = now_rfc3339();
        pending.persist_features.insert(f.code.clone());
        Ok(list)
    }

    /// Add a task to a specific todo-list of a feature.
    pub fn add_task(
        &self,
        id: &str,
        feature: &str,
        todo: &str,
        text: &str,
        key: Option<String>,
    ) -> Result<Task> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let t = Self::add_task_on(&mut project, &mut pending, feature, todo, text, key)?;
        self.flush(id, &project, &pending)?;
        Ok(t)
    }

    fn add_task_on(
        project: &mut Project,
        pending: &mut Pending,
        feature: &str,
        todo: &str,
        text: &str,
        key: Option<String>,
    ) -> Result<Task> {
        let f = project.feature_mut(feature)?;
        let list = f
            .todo_lists
            .iter_mut()
            .find(|l| l.code == todo)
            .ok_or_else(|| CoreError::TodoListNotFound(todo.to_string(), feature.to_string()))?;
        let existing: Vec<String> = list.tasks.iter().map(|t| t.key.clone()).collect();
        let key = match key {
            Some(k) => k,
            None => validate::next_task_key(&existing),
        };
        if existing.iter().any(|k| k == &key) {
            return Err(CoreError::TaskExists(key, todo.to_string()));
        }
        let task = Task {
            key,
            text: text.to_string(),
            state: TaskState::NotStarted,
        };
        list.tasks.push(task.clone());
        f.updated_at = now_rfc3339();
        pending.persist_features.insert(f.code.clone());
        Ok(task)
    }

    /// Set a task's state within a todo-list. When every task across all of the feature's
    /// todo-lists is Completed, auto-move the feature to a "Completed" status if allowed.
    pub fn set_task_state(
        &self,
        id: &str,
        feature: &str,
        todo: &str,
        key: &str,
        state: TaskState,
    ) -> Result<FeatureItem> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let f = Self::set_task_state_on(&mut project, &mut pending, feature, todo, key, state)?;
        self.flush(id, &project, &pending)?;
        Ok(f)
    }

    fn set_task_state_on(
        project: &mut Project,
        pending: &mut Pending,
        feature: &str,
        todo: &str,
        key: &str,
        state: TaskState,
    ) -> Result<FeatureItem> {
        let config = project.config.clone();
        let f = project.feature_mut(feature)?;
        let from = f.status.clone();
        let list = f
            .todo_lists
            .iter_mut()
            .find(|l| l.code == todo)
            .ok_or_else(|| CoreError::TodoListNotFound(todo.to_string(), feature.to_string()))?;
        let task = list
            .tasks
            .iter_mut()
            .find(|t| t.key == key)
            .ok_or_else(|| CoreError::TaskNotFound(key.to_string(), todo.to_string()))?;
        task.state = state;

        // Auto-complete the feature when every task (across all lists) is done — but NOT when
        // the feature sits in a no-op state (those are functionally inert dispositions).
        if f.all_tasks_completed()
            && !config.is_no_op(&f.status)
            && let Some(completed) = config
                .statuses
                .iter()
                .find(|s| s.eq_ignore_ascii_case("Completed"))
            && config.transition_allowed(&f.status, completed)
        {
            f.status = completed.clone();
        }
        f.updated_at = now_rfc3339();
        let updated = f.clone();
        pending.persist_features.insert(updated.code.clone());
        if updated.status != from {
            pending.remove_features.insert((from, feature.to_string()));
        }
        Ok(updated)
    }

    // ---- milestone ops -------------------------------------------------------------------

    pub fn add_milestone(
        &self,
        id: &str,
        name: &str,
        description: &str,
        depends_on: Vec<String>,
        code: Option<String>,
    ) -> Result<Milestone> {
        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let m = Self::add_milestone_on(
            &mut project,
            &mut pending,
            name,
            description,
            depends_on,
            code,
        )?;
        self.flush(id, &project, &pending)?;
        Ok(m)
    }

    fn add_milestone_on(
        project: &mut Project,
        pending: &mut Pending,
        name: &str,
        description: &str,
        depends_on: Vec<String>,
        code: Option<String>,
    ) -> Result<Milestone> {
        let existing: Vec<String> = project.milestones.iter().map(|m| m.code.clone()).collect();
        let code = match code {
            Some(c) => c,
            None => validate::next_code("MS", &existing),
        };
        if !validate::valid_name(&code) {
            return Err(CoreError::InvalidName(code));
        }
        if existing.iter().any(|c| c == &code) {
            return Err(CoreError::MilestoneExists(code));
        }
        let milestone = Milestone {
            code: code.clone(),
            name: name.to_string(),
            description: description.to_string(),
            depends_on,
        };
        project.milestones.push(milestone.clone());
        validate::validate_dependencies(&project.milestones, &code)?;
        pending.persist_milestones.insert(code);
        Ok(milestone)
    }

    pub fn edit_milestone(
        &self,
        id: &str,
        code: &str,
        name: Option<String>,
        description: Option<String>,
        depends_on: Option<Vec<String>>,
    ) -> Result<Milestone> {
        let mut project = self.load(id)?;
        {
            let m = project
                .milestones
                .iter_mut()
                .find(|m| m.code == code)
                .ok_or_else(|| CoreError::MilestoneNotFound(code.to_string()))?;
            if let Some(n) = name {
                m.name = n;
            }
            if let Some(d) = description {
                m.description = d;
            }
            if let Some(deps) = depends_on {
                m.depends_on = deps;
            }
        }
        validate::validate_dependencies(&project.milestones, code)?;
        let updated = project.milestone(code)?.clone();
        Self::write_yaml(&self.milestone_path(id, code), &updated)?;
        Ok(updated)
    }

    pub fn delete_milestone(&self, id: &str, code: &str) -> Result<()> {
        let project = self.load(id)?;
        project.milestone(code)?;
        // Referential integrity: a milestone in use by features cannot be deleted.
        let refs = project
            .features
            .iter()
            .filter(|f| f.milestone == code)
            .count();
        if refs > 0 {
            return Err(CoreError::MilestoneInUse(code.to_string(), refs));
        }
        std::fs::remove_file(self.milestone_path(id, code))?;
        Ok(())
    }

    // ---- batch ---------------------------------------------------------------------------

    /// Apply a bundle of operations in one call. Operations run in order; a `ref` alias given to
    /// a created item resolves to its assigned code for later operations in the bundle.
    ///
    /// The whole project is **loaded once**; every op mutates that single in-memory `Project`
    /// (chaining the `*_on` cores) and the resulting changes are flushed to disk at the end —
    /// instead of the old reload-and-persist-per-op cost (FEAT-028). On the first failing op the
    /// batch stops and returns `BatchOpFailed(index, msg)`; the ops already applied before it are
    /// still flushed (persisted), preserving the prior contract.
    ///
    /// Imports (FEAT-042): a `feature.add` whose source key already exists in the project is
    /// skipped (its `ref` resolves to the existing feature), and so is every later op that targets
    /// that `ref` as its feature (or a todo-list created under it), so re-running an import never
    /// duplicates work.
    pub fn apply_batch(
        &self,
        id: &str,
        ops: Vec<crate::batch::BatchOp>,
    ) -> Result<Vec<serde_json::Value>> {
        self.apply_batch_with(id, ops, false)
    }

    /// [`Store::apply_batch`], optionally as a **dry run**: every op is validated and reported
    /// exactly as it would apply, but nothing is written to disk.
    pub fn apply_batch_with(
        &self,
        id: &str,
        ops: Vec<crate::batch::BatchOp>,
        dry_run: bool,
    ) -> Result<Vec<serde_json::Value>> {
        use crate::batch::BatchOp::*;
        use std::collections::{HashMap, HashSet};

        fn resolve(aliases: &HashMap<String, String>, s: &str) -> String {
            aliases.get(s).cloned().unwrap_or_else(|| s.to_string())
        }
        fn skip(op: &str, target: &str) -> serde_json::Value {
            serde_json::json!({
                "op": op, "skipped": true, "target": target,
                "reason": "targets an already-imported feature",
            })
        }

        let mut project = self.load(id)?;
        let charter = crate::charter::load(self, id)?;
        let mut pending = Pending::default();
        let mut aliases: HashMap<String, String> = HashMap::new();
        // `ref` aliases of skipped (already imported) features, and of todo-lists under them.
        let mut skipped: HashSet<String> = HashSet::new();
        let mut out = Vec::new();

        for (i, op) in ops.into_iter().enumerate() {
            let result: Result<serde_json::Value> = (|| match op {
                FeatureAdd {
                    alias,
                    title,
                    milestone,
                    spec,
                    code,
                    kind,
                    priority,
                    due,
                    assignee,
                    team,
                    labels,
                    depends_on,
                    source,
                    original,
                    issue,
                    definition,
                    defect,
                    split_from,
                } => {
                    let source = source.map(|mut src| {
                        if src.key.trim().is_empty() {
                            src.key = src.derive_key(&title);
                        }
                        if src.imported_at.trim().is_empty() {
                            src.imported_at = now_rfc3339();
                        }
                        src
                    });
                    if let Some(src) = &source {
                        let existing = project
                            .features
                            .iter()
                            .find(|f| f.source.as_ref().is_some_and(|s| s.key == src.key));
                        if let Some(existing) = existing {
                            if let Some(a) = &alias {
                                aliases.insert(a.clone(), existing.code.clone());
                                skipped.insert(a.clone());
                            }
                            return Ok(serde_json::json!({
                                "op": "feature.add", "skipped": true, "reason": "already imported",
                                "code": existing.code, "title": existing.title, "key": src.key,
                            }));
                        }
                    }
                    let ms = resolve(&aliases, &milestone);
                    let mut spec_text = spec.unwrap_or_default();
                    if let (Some(src), Some(orig)) = (&source, &original) {
                        spec_text.push_str(&imported_section(src, orig));
                    }
                    let f = Self::add_feature_on(
                        &mut project,
                        &mut pending,
                        &title,
                        &spec_text,
                        &ms,
                        code,
                    )?;
                    if source.is_some() || issue.is_some() {
                        let feature = project.feature_mut(&f.code)?;
                        feature.source = source;
                        feature.issue = issue;
                    }
                    if definition.is_some() {
                        Self::set_feature_definition_on(
                            &mut project,
                            &mut pending,
                            &f.code,
                            definition,
                        )?;
                    }
                    if defect.is_some() {
                        Self::set_defect_on(&mut project, &mut pending, &f.code, defect)?;
                    }
                    if let Some(parent) = split_from {
                        let parent = resolve(&aliases, &parent);
                        project.feature(&parent)?; // a parent that does not exist is fiction
                        let feature = project.feature_mut(&f.code)?;
                        feature.split_from = (!parent.trim().is_empty()).then_some(parent);
                        pending.persist_features.insert(f.code.clone());
                    }
                    if let Some(a) = &alias {
                        aliases.insert(a.clone(), f.code.clone());
                    }
                    let deps = depends_on.map(|d| d.iter().map(|x| resolve(&aliases, x)).collect());
                    let had_deps = deps.is_some();
                    let f = if kind.is_some()
                        || priority.is_some()
                        || due.is_some()
                        || assignee.is_some()
                        || team.is_some()
                        || labels.is_some()
                        || deps.is_some()
                    {
                        Self::set_feature_attrs_on(
                            &mut project,
                            &mut pending,
                            &f.code,
                            kind,
                            priority,
                            due,
                            assignee,
                            team,
                            labels,
                            deps,
                        )?
                    } else {
                        f
                    };
                    if had_deps {
                        crate::graph::validate_feature_deps(self, &project, id, &f.code)?;
                    }
                    Ok(serde_json::json!({
                        "op": "feature.add", "code": f.code, "title": f.title, "status": f.status,
                    }))
                }
                FeatureEdit {
                    code,
                    title,
                    spec,
                    milestone,
                    new_code,
                    kind,
                    priority,
                    due,
                    assignee,
                    team,
                    labels,
                    depends_on,
                    source,
                    issue,
                    definition,
                    defect,
                    split_from,
                } => {
                    if skipped.contains(&code) {
                        return Ok(skip("feature.edit", &code));
                    }
                    let ms = milestone.map(|m| Some(resolve(&aliases, &m)));
                    let f = Self::edit_feature_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &code),
                        title,
                        spec,
                        ms,
                        new_code,
                    )?;
                    let deps = depends_on.map(|d| d.iter().map(|x| resolve(&aliases, x)).collect());
                    let had_deps = deps.is_some();
                    let f = if kind.is_some()
                        || priority.is_some()
                        || due.is_some()
                        || assignee.is_some()
                        || team.is_some()
                        || labels.is_some()
                        || deps.is_some()
                    {
                        Self::set_feature_attrs_on(
                            &mut project,
                            &mut pending,
                            &f.code,
                            kind,
                            priority,
                            due,
                            assignee,
                            team,
                            labels,
                            deps,
                        )?
                    } else {
                        f
                    };
                    if had_deps {
                        crate::graph::validate_feature_deps(self, &project, id, &f.code)?;
                    }
                    if definition.is_some() {
                        Self::set_feature_definition_on(
                            &mut project,
                            &mut pending,
                            &f.code,
                            definition,
                        )?;
                    }
                    if defect.is_some() {
                        Self::set_defect_on(&mut project, &mut pending, &f.code, defect)?;
                    }
                    if let Some(parent) = split_from {
                        let parent = resolve(&aliases, &parent);
                        project.feature(&parent)?; // a parent that does not exist is fiction
                        let feature = project.feature_mut(&f.code)?;
                        feature.split_from = (!parent.trim().is_empty()).then_some(parent);
                        pending.persist_features.insert(f.code.clone());
                    }
                    if source.is_some() || issue.is_some() {
                        let feature = project.feature_mut(&f.code)?;
                        if let Some(mut src) = source {
                            if src.key.trim().is_empty() {
                                src.key = src.derive_key(&feature.title);
                            }
                            feature.source = Some(src);
                        }
                        if issue.is_some() {
                            feature.issue = issue;
                        }
                        feature.updated_at = now_rfc3339();
                        pending.persist_features.insert(f.code.clone());
                    }
                    Ok(serde_json::json!({"op":"feature.edit","code":f.code}))
                }
                FeatureMove {
                    code,
                    to,
                    unapproved,
                } => {
                    if skipped.contains(&code) {
                        return Ok(skip("feature.move", &code));
                    }
                    // The gate applies on the batch path too — it is the primary way work is
                    // started, so exempting it would make the gate decorative.
                    let resolved = resolve(&aliases, &code);
                    Self::check_start_gate(
                        &project,
                        &charter,
                        &resolved,
                        &to,
                        unapproved.as_deref(),
                    )?;
                    if let Some(reason) = unapproved.as_deref().filter(|r| !r.trim().is_empty())
                        && let Ok(feature) = project.feature_mut(&resolved)
                        && let Some(def) = feature.definition.as_mut()
                    {
                        def.started_unapproved = reason.trim().to_string();
                    }
                    let f = Self::move_feature_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &code),
                        &to,
                    )?;
                    Ok(serde_json::json!({"op":"feature.move","code":f.code,"status":f.status}))
                }
                TestState {
                    feature,
                    requirement,
                    test,
                    state,
                    checked_rev,
                } => {
                    if skipped.contains(&feature) {
                        return Ok(skip("test.state", &feature));
                    }
                    let parsed = crate::models::TestState::parse(&state)
                        .ok_or_else(|| CoreError::InvalidTaskState(state.clone()))?;
                    let f = Self::set_test_state_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &feature),
                        &requirement,
                        &test,
                        parsed,
                        checked_rev.as_deref(),
                    )?;
                    Ok(serde_json::json!({
                        "op": "test.state", "code": f.code, "requirement": requirement,
                        "test": test, "state": state,
                    }))
                }
                FeatureApprove { code, by } => {
                    let resolved = resolve(&aliases, &code);
                    let feature = project.feature_mut(&resolved)?;
                    let definition = feature.definition.as_mut().ok_or_else(|| {
                        CoreError::Unsupported(format!("{resolved} has no definition to approve"))
                    })?;
                    definition.approval = Some(crate::models::Approval {
                        by: by.unwrap_or_else(|| "unknown".to_string()),
                        at: now_rfc3339(),
                        rev: definition.content_rev(),
                    });
                    feature.updated_at = now_rfc3339();
                    pending.persist_features.insert(resolved.clone());
                    Ok(serde_json::json!({"op":"feature.approve","code":resolved}))
                }
                MilestoneAdd {
                    alias,
                    name,
                    code,
                    description,
                    depends_on,
                } => {
                    // Idempotent re-runs (FEAT-042): a milestone with the same name (and the same
                    // code, when one is given) is reused instead of failing or duplicating.
                    let same = project
                        .milestones
                        .iter()
                        .find(|m| m.name == name && code.as_ref().is_none_or(|c| *c == m.code));
                    if let Some(m) = same {
                        if let Some(a) = alias {
                            aliases.insert(a, m.code.clone());
                        }
                        return Ok(serde_json::json!({
                            "op": "milestone.add", "skipped": true, "reason": "already exists",
                            "code": m.code, "title": m.name,
                        }));
                    }
                    let deps: Vec<String> = depends_on
                        .unwrap_or_default()
                        .iter()
                        .map(|d| resolve(&aliases, d))
                        .collect();
                    let m = Self::add_milestone_on(
                        &mut project,
                        &mut pending,
                        &name,
                        description.as_deref().unwrap_or(""),
                        deps,
                        code,
                    )?;
                    if let Some(a) = alias {
                        aliases.insert(a, m.code.clone());
                    }
                    Ok(serde_json::json!({"op":"milestone.add","code":m.code,"title":m.name}))
                }
                TodoAdd {
                    alias,
                    feature,
                    description,
                    code,
                } => {
                    if skipped.contains(&feature) {
                        if let Some(a) = alias {
                            skipped.insert(a);
                        }
                        return Ok(skip("todo.add", &feature));
                    }
                    let feat = resolve(&aliases, &feature);
                    let tl = Self::add_todo_list_on(
                        &mut project,
                        &mut pending,
                        &feat,
                        description.as_deref().unwrap_or(""),
                        code,
                    )?;
                    if let Some(a) = alias {
                        aliases.insert(a, tl.code.clone());
                    }
                    Ok(serde_json::json!({"op":"todo.add","feature":feat,"code":tl.code}))
                }
                TaskAdd {
                    feature,
                    todo,
                    text,
                    key,
                } => {
                    if skipped.contains(&feature) || skipped.contains(&todo) {
                        return Ok(skip("task.add", &feature));
                    }
                    let t = Self::add_task_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &feature),
                        &resolve(&aliases, &todo),
                        &text,
                        key,
                    )?;
                    Ok(serde_json::json!({"op":"task.add","key":t.key}))
                }
                TaskState {
                    feature,
                    todo,
                    key,
                    state,
                } => {
                    if skipped.contains(&feature) || skipped.contains(&todo) {
                        return Ok(skip("task.state", &feature));
                    }
                    let st = crate::models::TaskState::parse(&state)
                        .ok_or_else(|| CoreError::InvalidTaskState(state.clone()))?;
                    let f = Self::set_task_state_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &feature),
                        &resolve(&aliases, &todo),
                        &key,
                        st,
                    )?;
                    Ok(serde_json::json!({"op":"task.state","status":f.status}))
                }
                // Doc ops write to disk directly (they don't touch the in-memory project); a dry
                // run only validates the path.
                DocFolder {
                    path,
                    name,
                    description,
                } => {
                    if dry_run {
                        self.doc_path(id, &path)?;
                    } else {
                        self.write_folder_meta(id, &path, name, description)?;
                    }
                    Ok(serde_json::json!({"op":"doc.folder","path":path}))
                }
                DocWrite { path, content } => {
                    let saved = if dry_run {
                        self.doc_path(id, &path)?;
                        path
                    } else {
                        self.write_doc(id, &path, &content)?
                    };
                    Ok(serde_json::json!({"op":"doc.write","path":saved}))
                }
            })();

            match result {
                Ok(v) => out.push(v),
                Err(e) => {
                    // Persist the ops applied before the failure (prior contract), then report it.
                    if !dry_run {
                        let _ = self.flush(id, &project, &pending);
                    }
                    return Err(CoreError::BatchOpFailed(i, e.to_string()));
                }
            }
        }
        if !dry_run {
            self.flush(id, &project, &pending)?;
        }
        Ok(out)
    }

    /// Delete a project — only allowed when it has no features and no milestones.
    pub fn delete_project(&self, id: &str) -> Result<()> {
        let project = self.load(id)?;
        if !project.features.is_empty() || !project.milestones.is_empty() {
            return Err(CoreError::ProjectNotEmpty(
                id.to_string(),
                project.features.len(),
                project.milestones.len(),
            ));
        }
        std::fs::remove_dir_all(self.project_dir(id))?;
        Ok(())
    }

    // ---- config ops ----------------------------------------------------------------------

    pub fn set_transition(
        &self,
        id: &str,
        from: &str,
        to: &str,
        allow: bool,
    ) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        if !project.config.has_status(from) {
            return Err(CoreError::UnknownStatus(from.to_string()));
        }
        if !project.config.has_status(to) {
            return Err(CoreError::UnknownStatus(to.to_string()));
        }
        let entry = project
            .config
            .transitions
            .entry(from.to_string())
            .or_default();
        entry.retain(|t| t != to);
        if allow {
            entry.push(to.to_string());
        }
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    /// Replace the whole workflow (statuses + transitions + default_state + displayed_states +
    /// terminal_states) in one shot, preserving the project's name/description. Validates that
    /// transitions, default_state, displayed_states, and terminal_states only reference the new
    /// status set.
    #[allow(clippy::too_many_arguments)]
    pub fn set_workflow(
        &self,
        id: &str,
        statuses: Vec<String>,
        transitions: std::collections::BTreeMap<String, Vec<String>>,
        default_state: Option<String>,
        displayed_states: Option<Vec<String>>,
        no_op_states: Option<Vec<String>>,
        terminal_states: Option<Vec<String>>,
    ) -> Result<ProjectConfig> {
        if statuses.is_empty() {
            return Err(CoreError::NoStatuses);
        }
        let known = |s: &str| statuses.iter().any(|x| x == s);
        for (from, tos) in &transitions {
            if !known(from) {
                return Err(CoreError::UnknownStatus(from.clone()));
            }
            for to in tos {
                if !known(to) {
                    return Err(CoreError::UnknownStatus(to.clone()));
                }
            }
        }
        let default_state = default_state.unwrap_or_else(|| statuses[0].clone());
        if !known(&default_state) {
            return Err(CoreError::UnknownStatus(default_state));
        }
        let no_ops = no_op_states.unwrap_or_default();
        for s in &no_ops {
            if !known(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
        }
        // No-op states are always non-displayed; default displayed to active states.
        let displayed = displayed_states.unwrap_or_else(|| {
            statuses
                .iter()
                .filter(|s| !no_ops.contains(s))
                .cloned()
                .collect()
        });
        for s in &displayed {
            if !known(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
            if no_ops.contains(s) {
                return Err(CoreError::DisplayedNoOp(s.clone()));
            }
        }
        let terminals = terminal_states.unwrap_or_default();
        for s in &terminals {
            if !known(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
        }
        let mut project = self.load(id)?;
        // Referential integrity: a status being removed must not still hold feature items.
        for old in &project.config.statuses {
            if !statuses.contains(old) {
                let refs = project.features.iter().filter(|f| &f.status == old).count();
                if refs > 0 {
                    return Err(CoreError::StatusInUse(old.clone(), refs));
                }
            }
        }
        project.config.statuses = statuses;
        project.config.transitions = transitions;
        project.config.default_state = default_state;
        project.config.displayed_states = displayed;
        project.config.no_op_states = no_ops;
        project.config.terminal_states = terminals;
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    pub fn set_default_state(&self, id: &str, state: &str) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        if !project.config.has_status(state) {
            return Err(CoreError::UnknownStatus(state.to_string()));
        }
        project.config.default_state = state.to_string();
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    pub fn set_displayed_states(&self, id: &str, states: Vec<String>) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        for s in &states {
            if !project.config.has_status(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
            // A no-op state can never be displayed.
            if project.config.is_no_op(s) {
                return Err(CoreError::DisplayedNoOp(s.clone()));
            }
        }
        project.config.displayed_states = states;
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    /// Set which statuses are functionally inert (no-op) dispositions. No-op states are removed
    /// from `displayed_states` to keep the "no-op ⇒ non-displayed" invariant.
    pub fn set_no_op_states(&self, id: &str, states: Vec<String>) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        for s in &states {
            if !project.config.has_status(s) {
                return Err(CoreError::UnknownStatus(s.clone()));
            }
        }
        project
            .config
            .displayed_states
            .retain(|s| !states.contains(s));
        project.config.no_op_states = states;
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    /// Rename a status everywhere: across the workflow config (`statuses`, `displayed_states`,
    /// `no_op_states`, `default_state`, and `transitions` — both the `from` keys and the `to`
    /// lists) AND migrate every feature currently in it (moving its on-disk `<status>/` files into
    /// the new folder and updating its `status`). A single, safe operation.
    pub fn rename_status(&self, id: &str, old: &str, new: &str) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        let new = new.trim().to_string();
        if new.is_empty() {
            return Err(CoreError::InvalidName(new));
        }
        if old == new {
            return Ok(project.config); // no-op
        }
        if !project.config.has_status(old) {
            return Err(CoreError::UnknownStatus(old.to_string()));
        }
        if project.config.has_status(&new) {
            return Err(CoreError::Unsupported(format!(
                "status '{new}' already exists"
            )));
        }

        // 1) Rename throughout the config.
        let swap = |s: &mut String| {
            if s == old {
                *s = new.clone();
            }
        };
        project.config.statuses.iter_mut().for_each(swap);
        project.config.displayed_states.iter_mut().for_each(swap);
        project.config.no_op_states.iter_mut().for_each(swap);
        swap(&mut project.config.default_state);
        let renamed: std::collections::BTreeMap<String, Vec<String>> = project
            .config
            .transitions
            .iter()
            .map(|(from, tos)| {
                let k = if from == old {
                    new.clone()
                } else {
                    from.clone()
                };
                let v = tos
                    .iter()
                    .map(|t| if t == old { new.clone() } else { t.clone() })
                    .collect();
                (k, v)
            })
            .collect();
        project.config.transitions = renamed;

        // 2) Migrate features in the old status to the new status folder.
        let migrating: Vec<FeatureItem> = project
            .features
            .iter()
            .filter(|f| f.status == old)
            .cloned()
            .collect();
        for f in &migrating {
            let mut moved = f.clone();
            moved.status = new.clone();
            moved.updated_at = now_rfc3339();
            self.persist_feature(id, &moved)?;
            self.remove_feature_files(id, old, &f.code);
        }
        // Drop the now-emptied old status folder (only its migrated feature files lived there).
        let _ = std::fs::remove_dir_all(self.project_dir(id).join(old));

        // Migrated feature statuses changed on disk; refresh the index cache from the in-memory
        // (post-rename) project so it doesn't go stale (this op bypasses `flush`) (FEAT-033).
        let entries: Vec<IndexEntry> = project
            .features
            .iter()
            .map(IndexEntry::from_feature)
            .collect();
        self.write_index(id, &entries)?;

        self.save_config(id, &project.config)?;
        Ok(project.config)
    }

    // ---- documentation ops ---------------------------------------------------------------
    //
    // Per-project documentation is a tree of markdown files under `<project>/docs/`. Folders
    // are arbitrary and nestable (e.g. `design/customer/overview.md`). The server reads these;
    // only the CLI writes them.

    fn docs_dir(&self, id: &str) -> PathBuf {
        self.project_dir(id).join("docs")
    }

    /// Validate a relative doc path (no absolute, no `..`) and resolve it under `docs/`.
    /// Ensures a `.md` extension.
    fn doc_path(&self, id: &str, rel: &str) -> Result<PathBuf> {
        use std::path::Component;
        let rel_norm = rel.replace('\\', "/");
        let mut p = PathBuf::from(&rel_norm);
        if p.extension().is_none() {
            p.set_extension("md");
        }
        for comp in p.components() {
            match comp {
                Component::Normal(_) => {}
                _ => return Err(CoreError::InvalidDocPath(rel.to_string())),
            }
        }
        if p.as_os_str().is_empty() {
            return Err(CoreError::InvalidDocPath(rel.to_string()));
        }
        Ok(self.docs_dir(id).join(p))
    }

    /// Relative path (forward-slashed, sans the docs/ root) for display/listing.
    fn doc_rel_display(base: &Path, path: &Path) -> String {
        path.strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// List all documentation files (relative paths), sorted.
    pub fn list_docs(&self, id: &str) -> Result<Vec<String>> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let base = self.docs_dir(id);
        let mut out = Vec::new();
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<()> {
            if !dir.is_dir() {
                return Ok(());
            }
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(&path, base, out)?;
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    out.push(Store::doc_rel_display(base, &path));
                }
            }
            Ok(())
        }
        walk(&base, &base, &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Read a documentation file's markdown content.
    pub fn read_doc(&self, id: &str, rel: &str) -> Result<String> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let path = self.doc_path(id, rel)?;
        if !path.is_file() {
            return Err(CoreError::DocNotFound(rel.to_string()));
        }
        Ok(std::fs::read_to_string(path)?)
    }

    /// Create or overwrite a documentation file (creating parent folders as needed).
    pub fn write_doc(&self, id: &str, rel: &str, content: &str) -> Result<String> {
        self.write_doc_bytes(id, rel, content.as_bytes())
    }

    /// Create or overwrite a documentation file from raw bytes (binary assets like images live in
    /// the docs tree alongside the markdown, so they travel inside the data repo).
    pub fn write_doc_bytes(&self, id: &str, rel: &str, bytes: &[u8]) -> Result<String> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let path = self.doc_path(id, rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(Self::doc_rel_display(&self.docs_dir(id), &path))
    }

    /// Read a documentation file's raw bytes (used to serve binary assets like images).
    pub fn read_doc_bytes(&self, id: &str, rel: &str) -> Result<Vec<u8>> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let path = self.doc_path(id, rel)?;
        if !path.is_file() {
            return Err(CoreError::DocNotFound(rel.to_string()));
        }
        Ok(std::fs::read(path)?)
    }

    /// Delete a documentation file.
    pub fn delete_doc(&self, id: &str, rel: &str) -> Result<()> {
        let path = self.doc_path(id, rel)?;
        if !path.is_file() {
            return Err(CoreError::DocNotFound(rel.to_string()));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// Resolve a relative folder path (no `..`/absolute) under `docs/`. Empty == docs root.
    fn folder_path(&self, id: &str, rel: &str) -> Result<PathBuf> {
        use std::path::Component;
        let rel_norm = rel.replace('\\', "/");
        let p = PathBuf::from(&rel_norm);
        for comp in p.components() {
            match comp {
                Component::Normal(_) | Component::CurDir => {}
                _ => return Err(CoreError::InvalidDocPath(rel.to_string())),
            }
        }
        Ok(self.docs_dir(id).join(p))
    }

    /// Create/update a documentation folder's metadata (name + short description),
    /// creating the folder if needed.
    pub fn write_folder_meta(
        &self,
        id: &str,
        rel: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<()> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let dir = self.folder_path(id, rel)?;
        std::fs::create_dir_all(&dir)?;
        let meta_path = dir.join(FOLDER_META);
        let mut meta: FolderMeta = if meta_path.is_file() {
            Self::read_yaml(&meta_path)?
        } else {
            FolderMeta::default()
        };
        if let Some(n) = name {
            meta.name = n;
        }
        if let Some(d) = description {
            meta.description = d;
        }
        Self::write_yaml(&meta_path, &meta)?;
        Ok(())
    }

    /// Build the documentation tree (folders carry name/description; markdown files are leaves).
    pub fn doc_tree(&self, id: &str) -> Result<DocFolder> {
        if !self.project_exists(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let base = self.docs_dir(id);
        build_folder(&base, &base, String::new())
    }

    pub fn set_project_meta(
        &self,
        id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<ProjectConfig> {
        let mut project = self.load(id)?;
        if let Some(n) = name {
            project.config.name = n;
        }
        if let Some(d) = description {
            project.config.description = d;
        }
        self.save_config(id, &project.config)?;
        Ok(project.config)
    }
}

/// Recursively build a `DocFolder` for `dir`. `base` is the docs root, `rel` is the
/// forward-slashed path of `dir` relative to `base` (empty at the root).
fn build_folder(dir: &Path, base: &Path, rel: String) -> Result<DocFolder> {
    // Folder display name/description from its `_folder.yaml` (falling back to the dir name).
    let meta_path = dir.join(FOLDER_META);
    let meta: FolderMeta = if meta_path.is_file() {
        Store::read_yaml(&meta_path).unwrap_or_default()
    } else {
        FolderMeta::default()
    };
    let default_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(docs::title_from_stem)
        .unwrap_or_default();
    let name = if meta.name.is_empty() {
        default_name
    } else {
        meta.name
    };

    let mut folders = Vec::new();
    let mut files = Vec::new();
    if dir.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for path in entries {
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if path.is_dir() {
                let child_rel = if rel.is_empty() {
                    fname.to_string()
                } else {
                    format!("{rel}/{fname}")
                };
                folders.push(build_folder(&path, base, child_rel)?);
            } else if fname == FOLDER_META {
                continue;
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                let rel_path = Store::doc_rel_display(base, &path);
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                files.push(DocFile {
                    path: rel_path,
                    title: docs::title_from_stem(stem),
                });
            }
        }
    }

    Ok(DocFolder {
        path: rel,
        name,
        description: meta.description,
        folders,
        docs: files,
    })
}

/// The "Imported from" spec section that preserves an imported item's original text (FEAT-042),
/// so the content outlives its source.
fn imported_section(src: &crate::models::Source, original: &str) -> String {
    let mut origin = format!("`{}` ({})", src.reference, src.system);
    if let Some(rev) = &src.revision {
        origin.push_str(&format!(" at commit `{rev}`"));
    }
    if let Some(url) = &src.url {
        origin.push_str(&format!(", <{url}>"));
    }
    let date = src.imported_at.get(..10).unwrap_or(&src.imported_at);
    // A fence longer than any backtick run in the original, so it can't close early.
    let longest = original
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!(
        "\n\n## Imported from\n\n{origin}, imported {date}.\n\n{fence}text\n{}\n{fence}\n",
        original.trim_end()
    )
}

/// Give every requirement an id, leaving existing ones alone, so an author can write requirements
/// without inventing identifiers and still get stable link targets for tests, commits and ADRs.
/// Ids are handed out above the highest in use: a lower free id may belong to a deleted
/// requirement that a commit trailer still references.
fn assign_requirement_ids(definition: &mut crate::models::FeatureDefinition) {
    let mut taken: Vec<String> = definition
        .requirements
        .iter()
        .map(|r| r.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    for requirement in &mut definition.requirements {
        if requirement.id.trim().is_empty() {
            let id = validate::next_key("R-", &taken);
            taken.push(id.clone());
            requirement.id = id;
        } else {
            requirement.id = requirement.id.trim().to_string();
        }
    }
}
