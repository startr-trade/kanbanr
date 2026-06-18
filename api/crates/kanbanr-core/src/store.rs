//! YAML-backed store. Loads/saves projects and implements all mutating operations.
//! The CLI calls the mutating methods; the server only reads.

use crate::config::ProjectConfig;
use crate::docs::{DocFile, DocFolder, FolderMeta, FOLDER_META};
use crate::error::{CoreError, Result};
use crate::models::{FeatureItem, Milestone, Task, TaskState, TodoList};
use crate::{docs, now_rfc3339, validate};
use serde::de::DeserializeOwned;
use serde::Serialize;
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
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if self.project_exists(name) {
                        out.push(name.to_string());
                    }
                }
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

    /// Load all features by scanning each configured status folder `<status>/*.yaml` and
    /// pairing each with its `<status>/features-spec/<code>.md` specification. Driving the scan
    /// from the configured status list keeps status folders from colliding with
    /// `milestones/`, `schedules/`, `docs/`, etc.
    fn read_features(&self, id: &str, statuses: &[String]) -> Result<Vec<FeatureItem>> {
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
                let spec_path = self.spec_path(id, status, &meta.code);
                let spec = std::fs::read_to_string(&spec_path).unwrap_or_default();
                out.push(FeatureItem::from_meta(meta, spec));
            }
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
            due: None,
            labels: Vec::new(),
            depends_on: Vec::new(),
            todo_lists: Vec::new(),
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
        feature.status = to.to_string();
        feature.updated_at = now_rfc3339();
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
        if f.all_tasks_completed() && !config.is_no_op(&f.status) {
            if let Some(completed) = config
                .statuses
                .iter()
                .find(|s| s.eq_ignore_ascii_case("Completed"))
            {
                if config.transition_allowed(&f.status, completed) {
                    f.status = completed.clone();
                }
            }
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
    pub fn apply_batch(
        &self,
        id: &str,
        ops: Vec<crate::batch::BatchOp>,
    ) -> Result<Vec<serde_json::Value>> {
        use crate::batch::BatchOp::*;
        use std::collections::HashMap;

        fn resolve(aliases: &HashMap<String, String>, s: &str) -> String {
            aliases.get(s).cloned().unwrap_or_else(|| s.to_string())
        }

        let mut project = self.load(id)?;
        let mut pending = Pending::default();
        let mut aliases: HashMap<String, String> = HashMap::new();
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
                    labels,
                    depends_on,
                } => {
                    let ms = resolve(&aliases, &milestone);
                    let f = Self::add_feature_on(
                        &mut project,
                        &mut pending,
                        &title,
                        spec.as_deref().unwrap_or(""),
                        &ms,
                        code,
                    )?;
                    if let Some(a) = &alias {
                        aliases.insert(a.clone(), f.code.clone());
                    }
                    let deps = depends_on.map(|d| d.iter().map(|x| resolve(&aliases, x)).collect());
                    let had_deps = deps.is_some();
                    let f = if kind.is_some()
                        || priority.is_some()
                        || due.is_some()
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
                            labels,
                            deps,
                        )?
                    } else {
                        f
                    };
                    if had_deps {
                        crate::graph::validate_feature_deps(self, &project, id, &f.code)?;
                    }
                    Ok(serde_json::json!({"op":"feature.add","code":f.code,"status":f.status}))
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
                    labels,
                    depends_on,
                } => {
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
                            labels,
                            deps,
                        )?
                    } else {
                        f
                    };
                    if had_deps {
                        crate::graph::validate_feature_deps(self, &project, id, &f.code)?;
                    }
                    Ok(serde_json::json!({"op":"feature.edit","code":f.code}))
                }
                FeatureMove { code, to } => {
                    let f = Self::move_feature_on(
                        &mut project,
                        &mut pending,
                        &resolve(&aliases, &code),
                        &to,
                    )?;
                    Ok(serde_json::json!({"op":"feature.move","code":f.code,"status":f.status}))
                }
                MilestoneAdd {
                    alias,
                    name,
                    code,
                    description,
                    depends_on,
                } => {
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
                    Ok(serde_json::json!({"op":"milestone.add","code":m.code}))
                }
                TodoAdd {
                    alias,
                    feature,
                    description,
                    code,
                } => {
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
                // Doc ops write to disk directly (they don't touch the in-memory project).
                DocFolder {
                    path,
                    name,
                    description,
                } => {
                    self.write_folder_meta(id, &path, name, description)?;
                    Ok(serde_json::json!({"op":"doc.folder","path":path}))
                }
                DocWrite { path, content } => {
                    let saved = self.write_doc(id, &path, &content)?;
                    Ok(serde_json::json!({"op":"doc.write","path":saved}))
                }
            })();

            match result {
                Ok(v) => out.push(v),
                Err(e) => {
                    // Persist the ops applied before the failure (prior contract), then report it.
                    let _ = self.flush(id, &project, &pending);
                    return Err(CoreError::BatchOpFailed(i, e.to_string()));
                }
            }
        }
        self.flush(id, &project, &pending)?;
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

    /// Replace the whole workflow (statuses + transitions + default_state + displayed_states)
    /// in one shot, preserving the project's name/description. Validates that transitions,
    /// default_state, and displayed_states only reference the new status set.
    pub fn set_workflow(
        &self,
        id: &str,
        statuses: Vec<String>,
        transitions: std::collections::BTreeMap<String, Vec<String>>,
        default_state: Option<String>,
        displayed_states: Option<Vec<String>>,
        no_op_states: Option<Vec<String>>,
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
