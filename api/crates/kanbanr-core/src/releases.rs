//! Releases: planned up front, cut from finished work (FEAT-120).
//!
//! Only for projects that switch them on (`cadence.releases`, FEAT-121). A release is planned —
//! items are assigned to it as they are to a sprint — and **cut**: its planned items that are
//! finished ship, its release notes are written from their definitions, and what did not make it
//! is carried to the next planned release. Feedback afterwards points back at the version
//! (`feature add --found-in <version>`).
//!
//! Stored in `projects/<id>/releases.yaml`; notes go to the board doc `releases/<version>.md`.

use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::store::{Capability, Project, Store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RELEASES_FILE: &str = "releases.yaml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    #[default]
    Planned,
    Shipped,
}

/// A planned item that did not ship, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carry {
    pub code: String,
    /// The release it moved to, or `unplanned`.
    pub to: String,
    /// Why it did not ship.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Target date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(default)]
    pub state: ReleaseState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shipped_at: String,
    /// What shipped in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shipped: Vec<String>,
    /// The board doc holding its release notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes_doc: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<Carry>,
}

/// A release with what it holds and how much of that is finished, as the monitor shows it
/// (FEAT-138). Derived, never stored.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseReport {
    #[serde(flatten)]
    pub release: Release,
    /// Planned into it, or shipped in it.
    pub items: Vec<String>,
    /// How many of `items` are finished, by the same rule as the sprint's: an end status, or a stage
    /// marked `done` (FEAT-137). The page used to keep its own copy of the rule, counting only the
    /// end status — a Done item in a planned scrum release read as unfinished.
    pub finished: usize,
}

/// Every release, reported.
pub fn report_all(store: &Store, id: &str) -> Result<Vec<ReleaseReport>> {
    let project = store.load_meta(id)?;
    Ok(load(store, id)?
        .into_iter()
        .map(|release| {
            let items: Vec<&crate::FeatureItem> = project
                .features
                .iter()
                .filter(|f| {
                    f.release.as_deref() == Some(release.version.as_str())
                        || release.shipped.contains(&f.code)
                })
                .collect();
            let finished = items
                .iter()
                .filter(|f| project.config.counts_as_done(&f.status))
                .count();
            ReleaseReport {
                items: items.iter().map(|f| f.code.clone()).collect(),
                finished,
                release,
            }
        })
        .collect())
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(RELEASES_FILE)
}

/// A project's releases, in the order they were added. None is a normal state.
pub fn load(store: &Store, id: &str) -> Result<Vec<Release>> {
    match std::fs::read_to_string(path(store, id)) {
        Ok(s) if !s.trim().is_empty() => Ok(serde_yaml::from_str(&s)?),
        _ => Ok(Vec::new()),
    }
}

fn save(store: &Store, id: &str, releases: &[Release]) -> Result<()> {
    std::fs::write(path(store, id), serde_yaml::to_string(releases)?)?;
    Ok(())
}

fn cadence_on(store: &Store, id: &str) -> Result<Project> {
    let project = store.load(id)?;
    Store::require_cadence(&project, Capability::Releases)?;
    Ok(project)
}

/// Add a planned release.
pub fn add(store: &Store, id: &str, version: &str, target: &str, name: &str) -> Result<Release> {
    cadence_on(store, id)?;
    let version = version.trim();
    if version.is_empty() {
        return Err(CoreError::Unsupported(
            "a release needs a version (e.g. v1.2.0)".into(),
        ));
    }
    if !target.trim().is_empty() && crate::gantt::parse_ymd(target).is_none() {
        return Err(CoreError::Unsupported(format!(
            "'{target}' is not a date (YYYY-MM-DD)"
        )));
    }
    let mut releases = load(store, id)?;
    if releases.iter().any(|r| r.version == version) {
        return Err(CoreError::Unsupported(format!("{version} already exists")));
    }
    let release = Release {
        version: version.to_string(),
        name: name.trim().to_string(),
        target: target.trim().to_string(),
        ..Release::default()
    };
    releases.push(release.clone());
    save(store, id, &releases)?;
    Ok(release)
}

/// Plan items into a release.
pub fn plan(store: &Store, id: &str, version: &str, items: &[String]) -> Result<Release> {
    cadence_on(store, id)?;
    let releases = load(store, id)?;
    let release = releases
        .iter()
        .find(|r| r.version == version)
        .cloned()
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if release.state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has shipped — plan into a coming release"
        )));
    }
    for item in items {
        store.set_feature_release(id, item, Some(version.to_string()))?;
    }
    Ok(release)
}

/// Whether an item can ship, and to which end status — or why not. It ships if it is already
/// finished, or if its work is done (every task complete) and the workflow allows it to an end
/// status whose gate it meets. A workflow that declares no gates holds it to what `finish` asks.
fn ship_to(
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    f: &FeatureItem,
) -> std::result::Result<Option<String>, String> {
    let config = &project.config;
    let is_end = |s: &str| crate::graph::is_terminal_status(config, s) && !config.is_no_op(s);
    if is_end(&f.status) {
        return Ok(None); // already finished: it ships as it is
    }
    let open = f
        .todo_lists
        .iter()
        .flat_map(|l| &l.tasks)
        .filter(|t| t.state != crate::models::TaskState::Completed)
        .count();
    if open > 0 {
        return Err(format!("{open} task(s) still open"));
    }
    let Some(end) = config
        .statuses
        .iter()
        .find(|s| is_end(s) && config.transition_allowed(&f.status, s))
    else {
        return Err(format!("not finished — it is at {}", f.status));
    };
    if config.gates.is_empty() {
        let gaps = crate::readiness::evaluate(f, None, crate::readiness::CHECK);
        if let Some(first) = gaps.first() {
            return Err(first.message.clone());
        }
    }
    Store::check_gate(project, charter, ctx, &f.code, end, None).map_err(|e| e.to_string())?;
    Ok(Some(end.clone()))
}

/// What a cut did.
#[derive(Debug, Clone, Serialize)]
pub struct Cut {
    pub release: Release,
    pub notes: String,
}

/// Cut a release: its planned items that are finished ship (moving to their end status through its
/// gate), release notes are written from their definitions, and the rest is carried to the next
/// planned release — or back to unplanned — with the reason it did not make it.
pub fn cut(store: &Store, id: &str, version: &str) -> Result<Cut> {
    let project = cadence_on(store, id)?;
    let charter = crate::charter::load(store, id)?;
    let ctx = crate::readiness::Context::load(store, id, &project.config);
    let mut releases = load(store, id)?;
    let index = releases
        .iter()
        .position(|r| r.version == version)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {version}")))?;
    if releases[index].state == ReleaseState::Shipped {
        return Err(CoreError::Unsupported(format!(
            "{version} has already shipped"
        )));
    }
    let next = releases
        .iter()
        .skip(index + 1)
        .find(|r| r.state == ReleaseState::Planned)
        .map(|r| r.version.clone());

    let mut planned: Vec<FeatureItem> = project
        .features
        .iter()
        .filter(|f| f.release.as_deref() == Some(version))
        .cloned()
        .collect();
    // In code order, so the notes and the record read the same way every time.
    planned.sort_by(|a, b| a.code.cmp(&b.code));
    let mut shipped: Vec<FeatureItem> = Vec::new();
    let mut carried: Vec<Carry> = Vec::new();
    for f in &planned {
        match ship_to(&project, &charter, &ctx, f) {
            Ok(end) => {
                let moved = match end {
                    Some(end) => store.move_feature_approved(id, &f.code, &end, None)?,
                    None => f.clone(),
                };
                shipped.push(moved);
            }
            Err(why) => {
                store.set_feature_release(id, &f.code, next.clone())?;
                carried.push(Carry {
                    code: f.code.clone(),
                    to: next.clone().unwrap_or_else(|| "unplanned".to_string()),
                    why,
                });
            }
        }
    }

    let doc = format!("releases/{version}.md");
    let notes = notes(&releases[index], &shipped, &carried);
    store.write_doc(id, &doc, &notes)?;
    let release = &mut releases[index];
    release.state = ReleaseState::Shipped;
    release.shipped_at = crate::now_rfc3339();
    release.shipped = shipped.iter().map(|f| f.code.clone()).collect();
    release.notes_doc = doc;
    release.carried = carried;
    let release = release.clone();
    save(store, id, &releases)?;
    Ok(Cut { release, notes })
}

/// Release notes, from what each shipped item said it was for and what it had to do.
fn notes(release: &Release, shipped: &[FeatureItem], carried: &[Carry]) -> String {
    let mut out = format!("# {}", release.version);
    if !release.name.is_empty() {
        out.push_str(&format!(" — {}", release.name));
    }
    out.push_str(&format!(
        "\n\n_Released {}._\n\n## What shipped\n\n",
        crate::gantt::fmt_date(crate::sprints::today())
    ));
    if shipped.is_empty() {
        out.push_str("Nothing planned for this release was finished.\n");
    }
    for f in shipped {
        out.push_str(&format!("### {} — {}\n\n", f.code, f.title));
        if let Some(def) = &f.definition {
            if !def.statement.trim().is_empty() {
                out.push_str(&format!("{}\n\n", def.statement.trim()));
            }
            for r in &def.requirements {
                out.push_str(&format!("- {}\n", r.text.trim()));
            }
            if !def.requirements.is_empty() {
                out.push('\n');
            }
        }
    }
    if !carried.is_empty() {
        out.push_str("## Carried over\n\n");
        for c in carried {
            out.push_str(&format!("- {} → {} ({})\n", c.code, c.to, c.why));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::{ProjectConfig, Store};

    /// FEAT-138: a planned release's progress counts what is Done in a scrum project, as the sprint
    /// does — not only what has shipped.
    #[test]
    fn a_release_counts_done_items_as_finished() {
        let dir = std::env::temp_dir().join(format!("kanbanr-releases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let scrum = crate::config::preset("scrum").unwrap();
        let mut config = ProjectConfig::default_for("shop");
        config.statuses = scrum.statuses;
        config.default_state = scrum.default_state;
        config.terminal_states = scrum.terminal_states;
        config.no_op_states = scrum.no_op_states;
        config.transitions = scrum.transitions;
        config.gates = scrum.gates;
        store.init_project("shop", config).unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let codes: Vec<String> = ["A", "B", "C"]
            .iter()
            .map(|t| store.add_feature("shop", t, "", "M", None).unwrap().code)
            .collect();
        super::add(&store, "shop", "v0.2.0", "", "").unwrap();
        super::plan(&store, "shop", "v0.2.0", &codes).unwrap();
        for stage in ["Ready", "In Progress", "Review", "Testing", "Done"] {
            store.move_feature("shop", &codes[0], stage).unwrap();
        }
        let reports = super::report_all(&store, "shop").unwrap();
        let r = reports
            .iter()
            .find(|r| r.release.version == "v0.2.0")
            .unwrap();
        assert_eq!(r.items.len(), 3);
        assert_eq!(r.finished, 1, "Done counts, as it does for the sprint");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
