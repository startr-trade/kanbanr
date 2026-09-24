//! Project charter (FEAT-046): why this project exists, what it is trying to achieve, for whom,
//! and what it deliberately will not do.
//!
//! A board that records only *what* is being built loses the reasoning behind it. The charter is
//! the root of that reasoning: work items link to **goal ids**, so tooling can ask whether a piece
//! of work serves anything, and whether a stated goal has any work behind it.
//!
//! Stored as a per-project side file (`charter.yaml`) following the `mirror.yaml` pattern — absent
//! means "no charter" rather than an error, and an empty charter removes the file. It is
//! deliberately **not** part of `Project`, so `GET /projects/{p}` and the project export are
//! unchanged by its presence.

use crate::error::Result;
use crate::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Per-project charter file, next to `config.yaml`.
pub const CHARTER_FILE: &str = "charter.yaml";

/// The reasoning root of a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Charter {
    /// Why the project exists — the problem or opportunity.
    #[serde(default)]
    pub purpose: String,
    /// Optional one-line statement of the desired end state.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vision: String,
    /// Outcomes this project commits to. Work items link these by id.
    #[serde(default)]
    pub goals: Vec<Goal>,
    /// What this project deliberately will not do — often the most useful section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub non_goals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stakeholders: Vec<Stakeholder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    /// When the charter was first written. Gap reporting ignores items created before this, so
    /// adopting the method never floods the report with pre-existing work.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adopted_at: String,
}

/// An outcome the project commits to, not an output it produces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// `G-1`, `G-2`, … Assigned on save when left blank.
    #[serde(default)]
    pub id: String,
    pub statement: String,
    /// How we would know it happened.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub measure: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub horizon: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stakeholder {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// What they need from this project, and why they care.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub interest: String,
}

impl Charter {
    /// Nothing worth storing: an empty charter removes the file rather than leaving a husk.
    pub fn is_empty(&self) -> bool {
        self.purpose.trim().is_empty()
            && self.vision.trim().is_empty()
            && self.goals.is_empty()
            && self.non_goals.is_empty()
            && self.stakeholders.is_empty()
            && self.constraints.is_empty()
    }

    /// Every declared goal id, for validating the links work items make.
    pub fn goal_ids(&self) -> BTreeSet<String> {
        self.goals
            .iter()
            .map(|g| g.id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect()
    }

    pub fn goal(&self, id: &str) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    /// Give every goal an id, leaving existing ones alone, so an author can write goals without
    /// inventing identifiers and still get stable link targets.
    pub fn assign_goal_ids(&mut self) {
        let mut taken: Vec<String> = self.goal_ids().into_iter().collect();
        for goal in &mut self.goals {
            if goal.id.trim().is_empty() {
                let id = crate::validate::next_key("G-", &taken);
                taken.push(id.clone());
                goal.id = id;
            } else {
                goal.id = goal.id.trim().to_string();
            }
        }
    }
}

fn charter_path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(CHARTER_FILE)
}

/// Load a project's charter. An absent, empty or unreadable file means "no charter yet", which is
/// a normal state a new project is in — never an error.
pub fn load(store: &Store, id: &str) -> Result<Charter> {
    match std::fs::read_to_string(charter_path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(Charter::default()),
    }
}

/// Save a project's charter, stamping `adopted_at` the first time one is written. An empty charter
/// deletes the file.
pub fn save(store: &Store, id: &str, charter: &Charter) -> Result<Charter> {
    let path = charter_path(store, id);
    if charter.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(Charter::default());
    }
    let mut charter = charter.clone();
    charter.assign_goal_ids();
    if charter.adopted_at.trim().is_empty() {
        charter.adopted_at = load(store, id)
            .map(|existing| existing.adopted_at)
            .unwrap_or_default();
        if charter.adopted_at.trim().is_empty() {
            charter.adopted_at = crate::now_rfc3339();
        }
    }
    std::fs::write(path, serde_yaml::to_string(&charter)?)?;
    Ok(charter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn temp_store() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-charter-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn sample() -> Charter {
        Charter {
            purpose: "Keep the plan, the work and the reasoning in one git-backed folder.".into(),
            goals: vec![
                Goal {
                    id: String::new(),
                    statement: "A resumed session recovers full state without a human recap".into(),
                    measure: "kanbanr board gets Claude to working context in one step".into(),
                    horizon: String::new(),
                },
                Goal {
                    id: "G-7".into(),
                    statement: "Every item states why it exists".into(),
                    ..Goal::default()
                },
            ],
            non_goals: vec!["Multi-user auth".into()],
            ..Charter::default()
        }
    }

    #[test]
    fn absent_charter_is_a_default_not_an_error() {
        let (store, dir) = temp_store();
        let charter = load(&store, "demo").unwrap();
        assert_eq!(charter, Charter::default());
        assert!(charter.is_empty());
        assert!(charter.goal_ids().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_assigns_goal_ids_stamps_adoption_and_round_trips() {
        let (store, dir) = temp_store();
        let saved = save(&store, "demo", &sample()).unwrap();

        // Blank ids are filled past the highest in use; hand-written ones are left alone. Ids are
        // never handed out below the maximum, because a lower id may belong to a deleted goal that
        // items still reference — reuse would silently re-point those links.
        assert_eq!(saved.goals[0].id, "G-8");
        assert_eq!(saved.goals[1].id, "G-7");
        assert!(!saved.adopted_at.is_empty(), "adoption is stamped once");
        assert_eq!(
            saved.goal_ids(),
            ["G-7", "G-8"].iter().map(|s| s.to_string()).collect()
        );
        assert_eq!(
            saved.goal("G-7").unwrap().statement,
            "Every item states why it exists"
        );

        let loaded = load(&store, "demo").unwrap();
        assert_eq!(loaded, saved, "round-trips through yaml unchanged");

        // A later save keeps the original adoption date, so the gap cutoff can't drift forward.
        let mut edited = loaded.clone();
        edited.purpose = "Changed".into();
        edited.adopted_at = String::new();
        let resaved = save(&store, "demo", &edited).unwrap();
        assert_eq!(resaved.adopted_at, saved.adopted_at);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_charter_removes_the_file() {
        let (store, dir) = temp_store();
        save(&store, "demo", &sample()).unwrap();
        assert!(charter_path(&store, "demo").is_file());
        let cleared = save(&store, "demo", &Charter::default()).unwrap();
        assert!(cleared.is_empty());
        assert!(!charter_path(&store, "demo").exists());
        assert_eq!(load(&store, "demo").unwrap(), Charter::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
