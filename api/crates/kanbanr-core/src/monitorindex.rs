//! The monitor's compact index (DEF-081): every item as a card, with no specs, history or
//! todo-lists, plus the milestones, sprints, releases, ADRs, docs and goals the shell, the link
//! index and the item lists need. About 100 KB on this project's own board where the whole board
//! is 4 MB. Derived and never written (ADR-0005).

use crate::config::CodesConfig;
use crate::error::Result;
use crate::releases::ReleaseState;
use crate::sprints::SprintState;
use crate::store::Store;
use serde::Serialize;

/// `GET /projects/{p}/index`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CompactIndex {
    pub project: String,
    pub config: IndexConfig,
    /// Every item, in the order the board reads them.
    pub items: Vec<IndexItem>,
    pub milestones: Vec<IndexMilestone>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sprints: Vec<IndexSprint>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<IndexRelease>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub adrs: Vec<IndexAdr>,
    /// Every doc path under the project's docs, forward-slashed.
    pub docs: Vec<String>,
    /// The charter's goals.
    pub goals: Vec<IndexGoal>,
}

/// The part of the configuration the shell, links and cards read.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexConfig {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub displayed_states: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub no_op_states: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub terminal_states: Vec<String>,
    pub cadence: IndexCadence,
    pub codes: CodesConfig,
}

/// Which planning the project uses.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexCadence {
    pub sprints: bool,
    pub releases: bool,
    pub git_model: bool,
}

/// One item as a card.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexItem {
    pub code: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub status: String,
    pub milestone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// The charter goals its definition serves.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    /// Its requirements' ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<String>,
    /// Tasks completed, across all its todo-lists.
    pub done: usize,
    /// Tasks in all its todo-lists.
    pub total: usize,
    /// When it was created, to the whole second (RFC 3339): the board's newest-first order. Nothing
    /// in the monitor reads `updated_at` from here, so it is not sent (DEF-081 T7).
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexMilestone {
    pub code: String,
    pub name: String,
    /// What the milestone is for, as the milestone pages show it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// A sprint, with its state as of today.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexSprint {
    pub code: String,
    pub name: String,
    pub start: String,
    pub end: String,
    pub state: SprintState,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IndexRelease {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub state: ReleaseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
/// An ADR as the link index needs it: its id and the doc it lives in. Its title and status are on
/// the ADR's own page, and nothing reads them from the index (DEF-081 T7).
pub struct IndexAdr {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
/// A charter goal as the link index needs it: its id. The statement is on the charter page.
pub struct IndexGoal {
    pub id: String,
}

/// `Some` of a string that is not empty.
fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn nonempty(s: &Option<String>) -> Option<String> {
    s.as_deref().and_then(some)
}

/// An RFC 3339 time without its fraction of a second: `2026-09-28T11:22:34.588221216Z` →
/// `2026-09-28T11:22:34Z`. A time with no fraction is returned as it is.
fn to_the_second(t: &str) -> String {
    match t.find('.') {
        Some(dot) => {
            let rest = &t[dot + 1..];
            let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            format!("{}{}", &t[..dot], &rest[digits..])
        }
        None => t.to_string(),
    }
}

/// Build from `load_meta` (no specs), `adr::list`, `store.list_docs`, `charter::load`, `sprints`,
/// `releases::load`. An unknown project is ProjectNotFound; a board with none of these is an
/// index with empty lists.
pub fn compact(store: &Store, project: &str) -> Result<CompactIndex> {
    let p = store.load_meta(project)?;
    let c = &p.config;
    let config = IndexConfig {
        name: c.name.clone(),
        description: c.description.clone(),
        statuses: c.statuses.clone(),
        displayed_states: c.displayed_states.clone(),
        no_op_states: c.no_op_states.clone(),
        terminal_states: c.terminal_states.clone(),
        cadence: IndexCadence {
            sprints: c.cadence.sprints,
            releases: c.cadence.releases,
            git_model: c.cadence.git_model,
        },
        codes: c.codes.clone(),
    };
    let items = p
        .features
        .iter()
        .map(|f| IndexItem {
            code: f.code.clone(),
            title: f.title.clone(),
            kind: nonempty(&f.kind),
            status: f.status.clone(),
            milestone: f.milestone.clone(),
            sprint: nonempty(&f.sprint),
            release: nonempty(&f.release),
            priority: nonempty(&f.priority),
            assignee: nonempty(&f.assignee),
            team: nonempty(&f.team),
            labels: f.labels.clone(),
            depends_on: f.depends_on.clone(),
            goals: f
                .definition
                .as_ref()
                .map(|d| d.goals.clone())
                .unwrap_or_default(),
            requirements: f
                .definition
                .as_ref()
                .map(|d| d.requirements.iter().map(|r| r.id.clone()).collect())
                .unwrap_or_default(),
            done: f.done_count(),
            total: f.task_count(),
            created_at: to_the_second(&f.created_at),
        })
        .collect();
    let milestones = p
        .milestones
        .iter()
        .map(|m| IndexMilestone {
            code: m.code.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            depends_on: m.depends_on.clone(),
        })
        .collect();
    let sprints = crate::sprints::as_of(
        &crate::sprints::load(store, project)?,
        crate::sprints::today(),
    )
    .into_iter()
    .map(|s| IndexSprint {
        code: s.code,
        name: s.name,
        start: s.start,
        end: s.end,
        state: s.state,
    })
    .collect();
    let releases = crate::releases::load(store, project)?
        .into_iter()
        .map(|r| IndexRelease {
            name: some(&r.name),
            stage: some(&r.stage),
            target: some(&r.target),
            version: r.version,
            state: r.state,
        })
        .collect();
    let adrs = crate::adr::list(store, project)?
        .into_iter()
        .map(|a| IndexAdr {
            id: a.id,
            path: a.path,
        })
        .collect();
    let docs = store.list_docs(project)?;
    let goals = crate::charter::load(store, project)?
        .goals
        .into_iter()
        .map(|g| IndexGoal { id: g.id })
        .collect();
    Ok(CompactIndex {
        project: project.to_string(),
        config,
        items,
        milestones,
        sprints,
        releases,
        adrs,
        docs,
        goals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TaskState};

    /// A board of its own under the system temp folder, never the repository's (L-41).
    fn temp_store(tag: &str) -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-monitorindex-{tag}-{}-{}",
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

    /// Three items, one with a definition and tasks, plus a sprint, a release, an ADR, a doc and a
    /// goal.
    fn sample(store: &Store) -> (String, String) {
        let mut p = store.load_meta("demo").unwrap();
        p.config.cadence.sprints = true;
        p.config.cadence.releases = true;
        store.save_config("demo", &p.config).unwrap();
        crate::charter::save(
            store,
            "demo",
            &crate::charter::Charter {
                purpose: "Keep the plan.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Sessions recover".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let ms = store
            .add_milestone("demo", "First", "The first cut", vec![], None)
            .unwrap()
            .code;
        let a = store
            .add_feature("demo", "Alpha", "ALPHA SPEC BODY", &ms, None)
            .unwrap()
            .code;
        store
            .add_feature("demo", "Beta", "BETA SPEC BODY", &ms, None)
            .unwrap();
        store
            .add_feature("demo", "Gamma", "GAMMA SPEC BODY", &ms, None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &a,
                Some(FeatureDefinition {
                    statement: "Alpha for the person".into(),
                    goals: vec!["G-1".into()],
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "The system shall alpha.".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        let list = store.add_todo_list("demo", &a, "Build", None).unwrap().code;
        let t1 = store.add_task("demo", &a, &list, "one", None).unwrap().key;
        store.add_task("demo", &a, &list, "two", None).unwrap();
        store
            .set_task_state("demo", &a, &list, &t1, TaskState::Completed)
            .unwrap();
        crate::sprints::add(
            store,
            "demo",
            "2026-11-01",
            Some(14),
            "",
            None,
            "Sprint one",
            None,
            None,
        )
        .unwrap();
        crate::releases::add(store, "demo", "1.0", "", "First").unwrap();
        crate::adr::create(store, "demo", "Use one table", Default::default()).unwrap();
        store
            .write_doc("demo", "notes/plan.md", "# Plan\n")
            .unwrap();
        (a, ms)
    }

    /// DEF-081 R-2.
    #[test]
    fn the_index_carries_card_fields_and_no_bodies() {
        let (store, dir) = temp_store("fields");
        let (a, ms) = sample(&store);
        let ix = compact(&store, "demo").unwrap();
        assert_eq!(ix.project, "demo");
        assert_eq!(ix.items.len(), 3);
        let alpha = ix.items.iter().find(|i| i.code == a).unwrap();
        assert_eq!(alpha.title, "Alpha");
        assert_eq!(alpha.milestone, ms);
        assert_eq!(alpha.goals, vec!["G-1".to_string()]);
        assert_eq!(alpha.requirements, vec!["R-1".to_string()]);
        assert_eq!((alpha.done, alpha.total), (1, 2));
        // Created to the whole second: the board's newest-first order, without the nanoseconds.
        assert!(alpha.created_at.ends_with('Z'), "{}", alpha.created_at);
        assert!(!alpha.created_at.contains('.'), "{}", alpha.created_at);
        assert_eq!(alpha.created_at.len(), "2026-11-17T10:00:00Z".len());
        assert!(ix.config.cadence.sprints && ix.config.cadence.releases);
        assert!(!ix.config.statuses.is_empty());
        assert_eq!(ix.milestones.len(), 1);
        assert_eq!(ix.milestones[0].description, "The first cut");
        assert_eq!(ix.sprints.len(), 1);
        assert_eq!(ix.sprints[0].name, "Sprint one");
        assert_eq!(ix.releases.len(), 1);
        assert_eq!(ix.releases[0].version, "1.0");
        assert_eq!(ix.releases[0].name.as_deref(), Some("First"));
        assert_eq!(ix.adrs.len(), 1);
        assert!(ix.adrs[0].path.starts_with("decisions/"));
        assert!(ix.docs.iter().any(|d| d == "notes/plan.md"));
        assert!(ix.docs.contains(&ix.adrs[0].path));
        assert_eq!(ix.goals.len(), 1);
        assert_eq!(ix.goals[0].id, "G-1");
        // No bodies: no spec text, history or todo-lists reach the index.
        let json = serde_json::to_string(&ix).unwrap();
        for absent in [
            "SPEC BODY",
            "\"specification\"",
            "\"history\"",
            "\"todo_lists\"",
            "Alpha for the person",
            "The system shall alpha",
            "Keep the plan",
            // Fields no page reads from the index (DEF-081 T7): an ADR's title and status, a
            // goal's statement, an item's last change.
            "Use one table",
            "\"status\":\"proposed\"",
            "Sessions recover",
            "\"updated_at\"",
        ] {
            assert!(!json.contains(absent), "the index carries {absent}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-081 T7: the index sends creation times to the second, which orders as well and costs less.
    #[test]
    fn a_time_is_cut_to_the_second() {
        assert_eq!(
            to_the_second("2026-09-28T11:22:34.588221216Z"),
            "2026-09-28T11:22:34Z"
        );
        assert_eq!(
            to_the_second("2026-09-28T11:22:34.5+02:00"),
            "2026-09-28T11:22:34+02:00"
        );
        assert_eq!(
            to_the_second("2026-09-28T11:22:34Z"),
            "2026-09-28T11:22:34Z"
        );
        assert_eq!(to_the_second(""), "");
    }

    /// DEF-081 R-2, L-29: an empty board answers an empty index, not an error.
    #[test]
    fn an_empty_board_has_an_empty_index() {
        let (store, dir) = temp_store("empty");
        let ix = compact(&store, "demo").unwrap();
        assert!(ix.items.is_empty() && ix.goals.is_empty() && ix.releases.is_empty());
        let v = serde_json::to_value(&ix).unwrap();
        assert_eq!(v["items"], serde_json::json!([]));
        assert_eq!(v["goals"], serde_json::json!([]));
        assert!(v["docs"].is_array(), "docs is always present");
        assert!(
            v.get("releases").is_none(),
            "an empty list that may be absent is skipped"
        );
        assert!(matches!(
            compact(&store, "nope"),
            Err(crate::CoreError::ProjectNotFound(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-081 R-2: one item with its spec, also with the index gone; a status slice without specs.
    #[test]
    fn one_item_reads_with_its_spec_and_a_status_slice_without_specs() {
        let (store, dir) = temp_store("slice");
        let (a, ms) = sample(&store);
        let one = store.load_feature("demo", &a).unwrap();
        assert_eq!(one.specification, "ALPHA SPEC BODY");
        assert_eq!(one.todo_lists.len(), 1);
        let index = dir.join("projects").join("demo").join("index.yaml");
        let _ = std::fs::remove_file(&index);
        let again = store.load_feature("demo", &a).unwrap();
        assert_eq!(again.specification, "ALPHA SPEC BODY");
        assert!(
            !index.exists(),
            "reading an item never writes the index (ADR-0005)"
        );
        assert!(matches!(
            store.load_feature("demo", "FEAT-999"),
            Err(crate::CoreError::FeatureNotFound(_))
        ));
        assert!(store.load_feature("demo", "../config").is_err());

        // Through dispatch, as the daemon answers it.
        let get = |path: &str| -> serde_json::Value {
            serde_json::from_str(&crate::dispatch::dispatch(&store, "GET", path, None).unwrap())
                .unwrap()
        };
        let body = get(&format!("/projects/demo/features/{a}"));
        assert_eq!(body["specification"], "ALPHA SPEC BODY");
        // Move one item, then slice by status.
        let config = store.load_meta("demo").unwrap().config;
        let other = config.transitions[&one.status]
            .iter()
            .find(|s| **s != one.status)
            .unwrap()
            .clone();
        store.move_feature("demo", &a, &other).unwrap();
        let q = other.replace(' ', "%20");
        let slice = get(&format!("/projects/demo/features?status={q}"));
        let rows = slice.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["code"], a.as_str());
        assert!(rows.iter().all(|r| r["specification"] == ""));
        let all = get("/projects/demo/features");
        assert_eq!(all.as_array().unwrap().len(), 3);
        assert!(!all.to_string().contains("SPEC BODY"));
        let by_ms = get(&format!(
            "/projects/demo/features?milestone={ms}&status={q}"
        ));
        assert_eq!(by_ms.as_array().unwrap().len(), 1);
        assert!(
            crate::dispatch::dispatch(&store, "GET", "/projects/demo/features?state=x", None)
                .is_err()
        );
        // The index and the config read through dispatch too.
        assert_eq!(
            get("/projects/demo/index")["items"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(get("/projects/demo/config")["name"], "demo");
        let _ = std::fs::remove_dir_all(dir);
    }
}
