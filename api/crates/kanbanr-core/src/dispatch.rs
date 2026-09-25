//! Shared data-route dispatcher: maps an HTTP-shaped `(method, path, body)` onto `Store` calls and
//! returns the response BODY exactly as the server would (JSON for structured results, raw text for
//! export and doc reads). This is the single source of truth for the `/projects…` data routes,
//! used by the CLI's local (serverless) mode — and available for the server to delegate to.
//!
//! It deliberately does NOT cover auth/users/remotes — those are server/identity concerns handled
//! by their respective layers.

use crate::batch::BatchOp;
use crate::config::ProjectConfig;
use crate::error::{CoreError, Result};
use crate::graph::DependencyView;
use crate::models::TaskState;
use crate::{Store, export};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

// ---- small helpers -------------------------------------------------------------------------

fn ser<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))
}

fn str_field(body: &Value, k: &str) -> Option<String> {
    body.get(k).and_then(|v| v.as_str()).map(String::from)
}
fn bool_field(body: &Value, k: &str) -> Option<bool> {
    body.get(k).and_then(|v| v.as_bool())
}
fn vec_field(body: &Value, k: &str) -> Option<Vec<String>> {
    body.get(k).and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    })
}

/// If the body carries any optional feature attribute
/// (kind/priority/due/assignee/team/labels/depends_on), apply them to `code` and return the
/// serialized updated feature; otherwise `None`.
fn maybe_apply_attrs(store: &Store, p: &str, code: &str, b: &Value) -> Result<Option<String>> {
    let attr_keys = [
        "kind",
        "priority",
        "due",
        "assignee",
        "team",
        "labels",
        "depends_on",
    ];
    // Scheduling attributes (FEAT-035) take a dedicated store path so the broad set_feature_attrs
    // signature is untouched.
    let sched_keys = ["start", "estimate_days", "estimate"];
    let has_attrs = attr_keys.iter().any(|k| b.get(k).is_some());
    let has_sched = sched_keys.iter().any(|k| b.get(k).is_some());
    if !has_attrs && !has_sched {
        return Ok(None);
    }
    // A present string key sets it (empty string clears); a present array replaces the list.
    let s = |k: &str| b.get(k).map(|v| v.as_str().unwrap_or("").to_string());
    let v = |k: &str| {
        b.get(k).map(|val| {
            val.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        })
    };
    let mut f = None;
    if has_attrs {
        f = Some(store.set_feature_attrs(
            p,
            code,
            s("kind"),
            s("priority"),
            s("due"),
            s("assignee"),
            s("team"),
            v("labels"),
            v("depends_on"),
        )?);
    }
    if has_sched {
        // `estimate` is an alias for `estimate_days`; either an empty string or <= 0 clears it.
        let estimate = b
            .get("estimate_days")
            .or_else(|| b.get("estimate"))
            .map(|val| val.as_f64().unwrap_or(0.0));
        let updated = store.set_feature_schedule(p, code, s("start"), estimate)?;
        f = Some(updated);
    }
    match f {
        Some(f) => Ok(Some(ser(&f)?)),
        None => Ok(None),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// A boolean query-string flag: present with no/empty/"true"/"1" value -> true.
fn query_flag(query: Option<&str>, key: &str) -> bool {
    match query_param(query, key) {
        Some(v) => v.is_empty() || v == "true" || v == "1",
        None => query
            .map(|q| q.split('&').any(|kv| kv == key))
            .unwrap_or(false),
    }
}

/// Build a feature [`Query`](crate::query::Query) from the URL query string. `project` scopes the
/// search to one project (the per-project route) or `None` for portfolio-wide.
fn build_query(project: Option<&str>, query: Option<&str>) -> crate::query::Query {
    let labels = query_param(query, "label")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    crate::query::Query {
        project: project.map(String::from),
        status: query_param(query, "status"),
        milestone: query_param(query, "milestone"),
        kind: query_param(query, "kind"),
        priority: query_param(query, "priority"),
        labels,
        assignee: query_param(query, "assignee"),
        team: query_param(query, "team"),
        due_after: query_param(query, "due_after"),
        due_before: query_param(query, "due_before"),
        ready: query_flag(query, "ready"),
        blocked: query_flag(query, "blocked"),
        text: query_param(query, "text"),
        full_text: query_flag(query, "full_text"),
        goal: query_param(query, "goal"),
        gap: query_param(query, "gap"),
    }
}

// ---- project summaries (mirrors the server's home-tile shape, sans user filtering) ----------

#[derive(Serialize)]
struct ProjectSummary {
    id: String,
    name: String,
    description: String,
    displayed_states: Vec<String>,
    other_states: Vec<String>,
    no_op_states: Vec<String>,
    counts: BTreeMap<String, usize>,
    total_features: usize,
    has_docs: bool,
}

fn list_summaries(store: &Store) -> Result<Vec<ProjectSummary>> {
    let mut out = Vec::new();
    for id in store.list_projects()? {
        let Ok(project) = store.load(&id) else {
            continue;
        };
        let cfg = &project.config;
        let displayed = if cfg.displayed_states.is_empty() {
            cfg.statuses.clone()
        } else {
            cfg.displayed_states.clone()
        };
        let other_states: Vec<String> = cfg
            .statuses
            .iter()
            .filter(|s| !displayed.contains(s))
            .cloned()
            .collect();
        let mut counts = BTreeMap::new();
        for s in &cfg.statuses {
            counts.insert(
                s.clone(),
                project.features.iter().filter(|f| &f.status == s).count(),
            );
        }
        let has_docs = store
            .doc_tree(&id)
            .map(|t| !t.folders.is_empty() || !t.docs.is_empty())
            .unwrap_or(false);
        let name = if cfg.name.is_empty() {
            id.clone()
        } else {
            cfg.name.clone()
        };
        out.push(ProjectSummary {
            id,
            name,
            description: cfg.description.clone(),
            displayed_states: displayed,
            other_states,
            no_op_states: cfg.no_op_states.clone(),
            counts,
            total_features: project.features.len(),
            has_docs,
        });
    }
    Ok(out)
}

// ---- config builders shared with the server's create-project / workflow routes --------------

fn build_project_config(name: &str, body: &Value) -> ProjectConfig {
    // An opt-in phase model, chosen at creation: `"workflow": "togaf"`.
    if str_field(body, "workflow").as_deref() == Some("togaf") {
        let mut config = crate::config::togaf_preset(name);
        if let Some(d) = str_field(body, "description") {
            config.description = d;
        }
        return config;
    }
    let statuses = vec_field(body, "statuses");
    let displayed = vec_field(body, "displayed_states");
    let default_state = str_field(body, "default_state");
    let no_op = vec_field(body, "no_op_states");
    let mut config = match &statuses {
        Some(s) => {
            let no_ops = no_op.clone().unwrap_or_default();
            ProjectConfig {
                branch_pattern: None,
                schema_version: crate::config::CURRENT_SCHEMA_VERSION,
                name: name.to_string(),
                description: String::new(),
                displayed_states: displayed
                    .clone()
                    .unwrap_or_else(|| s.iter().filter(|x| !no_ops.contains(x)).cloned().collect()),
                default_state: default_state
                    .clone()
                    .unwrap_or_else(|| s.first().cloned().unwrap_or_default()),
                transitions: Default::default(),
                no_op_states: no_ops,
                terminal_states: vec_field(body, "terminal_states").unwrap_or_default(),
                statuses: s.clone(),
            }
        }
        None => {
            let mut c = ProjectConfig::default_for(name);
            if let Some(ds) = &displayed {
                c.displayed_states = ds.clone();
            }
            if let Some(d) = &default_state {
                c.default_state = d.clone();
            }
            if let Some(n) = &no_op {
                c.no_op_states = n.clone();
                c.displayed_states.retain(|s| !n.contains(s));
            }
            if let Some(t) = vec_field(body, "terminal_states") {
                c.terminal_states = t;
            }
            c
        }
    };
    if let Some(d) = str_field(body, "description") {
        config.description = d;
    }
    config
}

fn apply_workflow(store: &Store, p: &str, body: &Value) -> Result<String> {
    let base = if bool_field(body, "togaf").unwrap_or(false) {
        Some(crate::config::togaf_preset(p))
    } else if bool_field(body, "defaults").unwrap_or(false) {
        Some(ProjectConfig::default_for(p))
    } else {
        None
    };
    let statuses = match (vec_field(body, "statuses"), &base) {
        (Some(s), _) => s,
        (None, Some(base)) => base.statuses.clone(),
        (None, None) => {
            return Err(CoreError::Unsupported(
                "provide statuses (or defaults)".into(),
            ));
        }
    };
    let transitions: BTreeMap<String, Vec<String>> = body
        .get("transitions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .or_else(|| base.as_ref().map(|x| x.transitions.clone()))
        .unwrap_or_default();
    let default_state =
        str_field(body, "default_state").or_else(|| base.as_ref().map(|x| x.default_state.clone()));
    let displayed = vec_field(body, "displayed_states")
        .or_else(|| base.as_ref().map(|x| x.displayed_states.clone()));
    let no_ops =
        vec_field(body, "no_op_states").or_else(|| base.as_ref().map(|x| x.no_op_states.clone()));
    let terminals = vec_field(body, "terminal_states")
        .or_else(|| base.as_ref().map(|x| x.terminal_states.clone()));
    ser(&store.set_workflow(
        p,
        statuses,
        transitions,
        default_state,
        displayed,
        no_ops,
        terminals,
    )?)
}

// ---- the dispatcher ------------------------------------------------------------------------

/// Apply a data operation against the store, returning the response body string. Reads and writes
/// alike — the caller (e.g. the CLI's local backend) decides whether to commit afterwards based on
/// the HTTP method.
pub fn dispatch(store: &Store, method: &str, path: &str, body: Option<&Value>) -> Result<String> {
    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let segs: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    let m = method.to_uppercase();
    let null = Value::Null;
    let b = body.unwrap_or(&null);

    match (m.as_str(), segs.as_slice()) {
        // ---- doctor (portfolio integrity) ----
        ("GET", ["doctor"]) => ser(&crate::doctor::run(store)?),
        ("GET", ["projects", p, "doctor"]) => ser(&crate::doctor::run_project(store, p)?),

        // ---- portfolio / program hierarchy (FEAT-030) ----
        ("GET", ["portfolio"]) => ser(&crate::portfolio::view(store)?),
        ("GET", ["portfolio", "rollups"]) => ser(&crate::portfolio::rollups(store)?),
        ("GET", ["portfolio", "board"]) => ser(&crate::portfolio::cross_project_board(store)?),
        ("POST", ["portfolio", "programs"]) => {
            let id = str_field(b, "id").unwrap_or_default();
            let projects = vec_field(b, "projects").unwrap_or_default();
            let ws = crate::portfolio::add_program(
                store,
                &id,
                str_field(b, "name"),
                str_field(b, "description"),
                projects,
            )?;
            ser(&ws)
        }

        // ---- projects ----
        ("GET", ["projects"]) => ser(&list_summaries(store)?),
        ("POST", ["projects"]) => {
            let name = str_field(b, "name").unwrap_or_default();
            let config = build_project_config(&name, b);
            ser(&store.init_project(&name, config)?)
        }
        ("GET", ["projects", p]) => ser(&store.load(p)?),

        // ---- derived dependency state (FEAT-027) ----
        ("GET", ["projects", p, "ready"]) => {
            store.load(p)?; // 404 on unknown project
            let view = DependencyView::build(store, None)?;
            ser(&view.ready(Some(p)))
        }
        ("GET", ["projects", p, "blocked"]) => {
            store.load(p)?;
            let view = DependencyView::build(store, None)?;
            ser(&view.blocked(Some(p)))
        }
        ("GET", ["projects", p, "graph"]) => {
            store.load(p)?;
            let view = DependencyView::build(store, None)?;
            match query_param(query, "format").as_deref() {
                Some("dot") => Ok(view.to_dot(Some(p))),
                _ => Ok(view.to_json_value(Some(p)).to_string()),
            }
        }
        // ---- scheduling: critical path & Gantt (FEAT-035) ----
        ("GET", ["projects", p, "critical-path"]) => {
            store.load(p)?; // 404 on unknown project
            let view = DependencyView::build(store, None)?;
            Ok(view.schedule(Some(p)).to_json_value().to_string())
        }
        ("GET", ["projects", p, "gantt"]) => Ok(crate::gantt::project_gantt(store, p)?),
        ("GET", ["critical-path"]) => {
            let view = DependencyView::build(store, None)?;
            Ok(view.schedule(None).to_json_value().to_string())
        }
        ("GET", ["gantt"]) => Ok(crate::gantt::portfolio_gantt(store)?),
        ("GET", ["projects", p, "features", code, "impact"]) => {
            let project = store.load(p)?;
            project.feature(code)?; // 404 on unknown feature
            let view = DependencyView::build(store, None)?;
            ser(&view.impact(&crate::graph::qualify(p, code)))
        }
        // Portfolio-wide (cross-project) variants.
        ("GET", ["ready"]) => {
            let view = DependencyView::build(store, None)?;
            ser(&view.ready(None))
        }
        ("GET", ["blocked"]) => {
            let view = DependencyView::build(store, None)?;
            ser(&view.blocked(None))
        }
        ("GET", ["graph"]) => {
            let view = DependencyView::build(store, None)?;
            match query_param(query, "format").as_deref() {
                Some("dot") => Ok(view.to_dot(None)),
                _ => Ok(view.to_json_value(None).to_string()),
            }
        }

        // ---- query: rich filters + full-text, scoped or portfolio-wide (FEAT-032) ----
        ("GET", ["query"]) => ser(&crate::query::run(store, &build_query(None, query))?),
        ("GET", ["projects", p, "query"]) => {
            store.load_meta(p)?; // 404 on unknown project
            ser(&crate::query::run(store, &build_query(Some(p), query))?)
        }
        ("GET", ["projects", p, "export"]) => {
            let project = store.load(p)?;
            match query_param(query, "format").as_deref() {
                Some("json") => ser(&project),
                _ => Ok(export::project_to_markdown(&project)),
            }
        }
        ("PATCH", ["projects", p]) => {
            ser(&store.set_project_meta(p, str_field(b, "name"), str_field(b, "description"))?)
        }
        ("DELETE", ["projects", p]) => {
            store.delete_project(p)?;
            Ok(String::new())
        }

        // ---- features ----
        ("GET", ["projects", p, "features", code, "export"]) => {
            let project = store.load(p)?;
            let feature = project.feature(code)?;
            let ms = project.milestone(&feature.milestone).ok();
            match query_param(query, "format").as_deref() {
                Some("json") => export::to_json(feature, ms)
                    .map_err(|e| CoreError::Unsupported(format!("export: {e}"))),
                _ => Ok(export::to_markdown(feature, ms)),
            }
        }
        ("POST", ["projects", p, "features"]) => {
            let f = store.add_feature(
                p,
                &str_field(b, "title").unwrap_or_default(),
                &str_field(b, "specification").unwrap_or_default(),
                &str_field(b, "milestone").unwrap_or_default(),
                str_field(b, "code"),
            )?;
            match maybe_apply_attrs(store, p, &f.code, b)? {
                Some(json) => Ok(json),
                None => ser(&f),
            }
        }
        ("PATCH", ["projects", p, "features", code]) => {
            let milestone = if bool_field(b, "clear_milestone").unwrap_or(false) {
                Some(None)
            } else {
                str_field(b, "milestone").map(Some)
            };
            let edited = store.edit_feature(
                p,
                code,
                str_field(b, "title"),
                str_field(b, "specification"),
                milestone,
                str_field(b, "new_code"),
            )?;
            // new_code may have renamed it; attrs apply to the (possibly new) code.
            match maybe_apply_attrs(store, p, &edited.code, b)? {
                Some(json) => Ok(json),
                None => ser(&edited),
            }
        }
        ("POST", ["projects", p, "features", code, "move"]) => {
            // Entering an active status needs a current approval (FEAT-048); an explicit
            // `unapproved` reason is recorded rather than silently allowed.
            ser(&store.move_feature_approved(
                p,
                code,
                &str_field(b, "to").unwrap_or_default(),
                str_field(b, "unapproved").as_deref(),
            )?)
        }
        ("PUT", ["projects", p, "features", code, "tests", requirement, test]) => {
            // Path segments arrive percent-encoded: a test name is free text and routinely holds
            // `::`, spaces or `/`, none of which survive a raw path match.
            let (requirement, test) = (&percent_decode(requirement), &percent_decode(test));
            let state = crate::models::TestState::parse(&str_field(b, "state").unwrap_or_default())
                .ok_or_else(|| {
                    CoreError::Unsupported("state must be planned, red or green".into())
                })?;
            ser(&store.set_test_state(
                p,
                code,
                requirement.as_str(),
                test.as_str(),
                state,
                str_field(b, "checked_rev").as_deref(),
            )?)
        }
        ("POST", ["projects", p, "features", code, "approve"]) => ser(&store.approve_feature(
            p,
            code,
            &str_field(b, "by").unwrap_or_else(|| "unknown".to_string()),
        )?),
        ("POST", ["projects", p, "features", code, "todos"]) => ser(&store.add_todo_list(
            p,
            code,
            &str_field(b, "description").unwrap_or_default(),
            str_field(b, "code"),
        )?),
        ("POST", ["projects", p, "features", code, "todos", todo, "tasks"]) => ser(&store
            .add_task(
                p,
                code,
                todo,
                &str_field(b, "text").unwrap_or_default(),
                str_field(b, "key"),
            )?),
        ("PUT", ["projects", p, "features", code, "todos", todo, "tasks", key]) => {
            let raw = str_field(b, "state").unwrap_or_default();
            let state = TaskState::parse(&raw).ok_or(CoreError::InvalidTaskState(raw))?;
            ser(&store.set_task_state(p, code, todo, key, state)?)
        }

        // ---- milestones ----
        ("POST", ["projects", p, "milestones"]) => ser(&store.add_milestone(
            p,
            &str_field(b, "name").unwrap_or_default(),
            &str_field(b, "description").unwrap_or_default(),
            vec_field(b, "depends_on").unwrap_or_default(),
            str_field(b, "code"),
        )?),
        ("PATCH", ["projects", p, "milestones", code]) => ser(&store.edit_milestone(
            p,
            code,
            str_field(b, "name"),
            str_field(b, "description"),
            vec_field(b, "depends_on"),
        )?),
        ("DELETE", ["projects", p, "milestones", code]) => {
            store.delete_milestone(p, code)?;
            Ok(String::new())
        }

        // ---- config ----
        ("POST", ["projects", p, "config", "transition"]) => ser(&store.set_transition(
            p,
            &str_field(b, "from").unwrap_or_default(),
            &str_field(b, "to").unwrap_or_default(),
            bool_field(b, "allow").unwrap_or(false),
        )?),
        ("PUT", ["projects", p, "config", "displayed-states"]) => {
            ser(&store.set_displayed_states(p, vec_field(b, "states").unwrap_or_default())?)
        }
        ("PUT", ["projects", p, "config", "default-state"]) => {
            ser(&store.set_default_state(p, &str_field(b, "state").unwrap_or_default())?)
        }
        ("PUT", ["projects", p, "config", "no-op-states"]) => {
            ser(&store.set_no_op_states(p, vec_field(b, "states").unwrap_or_default())?)
        }
        ("POST", ["projects", p, "config", "rename-status"]) => ser(&store.rename_status(
            p,
            &str_field(b, "old").unwrap_or_default(),
            &str_field(b, "new").unwrap_or_default(),
        )?),
        ("PUT", ["projects", p, "config", "workflow"]) => apply_workflow(store, p, b),
        // Workflow export (FEAT-039): the config rendered as a Mermaid state diagram.
        ("GET", ["projects", p, "workflow"]) => {
            let project = store.load(p)?;
            match query_param(query, "format").as_deref() {
                Some("mermaid") => Ok(crate::mermaid::to_state_diagram(&project.config)),
                other => Err(CoreError::Unsupported(format!(
                    "workflow format '{}' (only 'mermaid' is supported)",
                    other.unwrap_or("")
                ))),
            }
        }

        // ---- docs ----
        ("GET", ["projects", p, "docs"]) => ser(&store.doc_tree(p)?),
        ("PUT", ["projects", p, "docs", "folder"]) => {
            let folder = str_field(b, "path").unwrap_or_default();
            store.write_folder_meta(
                p,
                &folder,
                str_field(b, "name"),
                str_field(b, "description"),
            )?;
            Ok(json!({ "path": folder }).to_string())
        }
        ("GET", ["projects", p, "docs", "content"]) => {
            let rel = query_param(query, "path").unwrap_or_default();
            store.read_doc(p, &rel)
        }
        ("PUT", ["projects", p, "docs", "content"]) => {
            let saved = store.write_doc(
                p,
                &str_field(b, "path").unwrap_or_default(),
                &str_field(b, "content").unwrap_or_default(),
            )?;
            Ok(json!({ "path": saved }).to_string())
        }
        ("DELETE", ["projects", p, "docs", "content"]) => {
            let rel = query_param(query, "path").unwrap_or_default();
            store.delete_doc(p, &rel)?;
            Ok(String::new())
        }

        // ---- feature definition (FEAT-047) ----
        ("PUT", ["projects", p, "features", code, "definition"]) => {
            let definition: Option<crate::models::FeatureDefinition> = if b.is_null() {
                None // an explicit null clears the block
            } else {
                Some(
                    serde_json::from_value(b.clone())
                        .map_err(|e| CoreError::Unsupported(format!("invalid definition: {e}")))?,
                )
            };
            ser(&store.set_feature_definition(p, code, definition)?)
        }

        // ---- defect record (FEAT-053) ----
        ("PUT", ["projects", p, "features", code, "defect"]) => {
            let defect: Option<crate::models::Defect> = if b.is_null() {
                None // an explicit null clears the block
            } else {
                Some(
                    serde_json::from_value(b.clone())
                        .map_err(|e| CoreError::Unsupported(format!("invalid defect: {e}")))?,
                )
            };
            ser(&store.set_defect(p, code, defect)?)
        }

        // ---- flow and quality report (FEAT-053) ----
        ("GET", ["projects", p, "report"]) => {
            let window = crate::report::Window {
                since: query_param(query, "since"),
            };
            ser(&crate::report::run(
                store,
                p,
                &window,
                query_param(query, "rev").as_deref(),
            )?)
        }

        // ---- project charter (FEAT-046) ----
        ("GET", ["projects", p, "charter"]) => {
            store.load_meta(p)?; // the project must exist; an absent charter is a valid default
            ser(&crate::charter::load(store, p)?)
        }
        ("PUT", ["projects", p, "charter"]) => {
            store.load_meta(p)?;
            let charter: crate::charter::Charter = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid charter: {e}")))?;
            ser(&crate::charter::save(store, p, &charter)?)
        }

        // ---- issue mirror (FEAT-043) ----
        ("GET", ["projects", p, "mirror"]) => ser(&crate::mirror::load_config(store, p)?),
        ("PUT", ["projects", p, "mirror"]) => {
            let config: crate::mirror::MirrorConfig = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid mirror config: {e}")))?;
            crate::mirror::save_config(store, p, &config)?;
            ser(&config)
        }

        // ---- batch ----
        ("POST", ["projects", p, "batch"]) => {
            let ops: Vec<BatchOp> = b
                .get("operations")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| CoreError::Unsupported(format!("invalid batch: {e}")))?
                .unwrap_or_default();
            let dry_run = b.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
            let results = store.apply_batch_with(p, ops, dry_run)?;
            Ok(json!({ "results": results }).to_string())
        }

        _ => Err(CoreError::Unsupported(format!("{method} {path_only}"))),
    }
}

/// Whether a write request is a dry run (`"dry_run": true` in the body, honored by the batch
/// route): it changes nothing, so writers must not log activity or commit for it. (FEAT-042)
pub fn is_dry_run(body: Option<&Value>) -> bool {
    body.and_then(|b| b.get("dry_run"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether an HTTP method mutates state (and therefore should be committed in local mode).
pub fn is_mutation(method: &str) -> bool {
    !matches!(method.to_uppercase().as_str(), "GET" | "HEAD" | "OPTIONS")
}

/// A meaningful commit message for a data write, derived from the route (mirrors the server). For
/// a batch, an explicit `message` in the body wins.
pub fn commit_message(method: &str, path: &str, body: Option<&Value>) -> String {
    let path_only = path.split('?').next().unwrap_or(path);
    let s: Vec<&str> = path_only.split('/').filter(|x| !x.is_empty()).collect();
    let del = method.eq_ignore_ascii_case("DELETE");
    match s.as_slice() {
        ["portfolio", "programs"] => body
            .and_then(|b| b.get("id").and_then(|v| v.as_str()))
            .map(|id| format!("add program {id}"))
            .unwrap_or_else(|| "update portfolio".into()),
        ["projects"] => "create project".into(),
        ["projects", p] if del => format!("delete project {p}"),
        ["projects", p] => format!("edit project {p}"),
        ["projects", _p, "charter"] => "update project charter".into(),
        ["projects", _p, "features", c, "definition"] => format!("define feature {c}"),
        ["projects", _p, "features", c, "defect"] => format!("record defect details for {c}"),
        ["projects", _p, "mirror"] => "configure issue mirror".into(),
        ["projects", _p, "features"] => "add feature item".into(),
        ["projects", _p, "features", c] => format!("edit feature {c}"),
        ["projects", _p, "features", c, "move"] => format!("move feature {c}"),
        ["projects", _p, "features", c, "approve"] => format!("approve definition of {c}"),
        ["projects", _p, "features", c, "tests", r, t] => {
            format!(
                "test {t} of {c}/{r} is now {}",
                str_field(body.unwrap_or(&Value::Null), "state").unwrap_or_default()
            )
        }
        ["projects", _p, "features", c, "todos"] => format!("add todo-list to {c}"),
        ["projects", _p, "features", c, "todos", t, "tasks"] => format!("add task to {c}/{t}"),
        ["projects", _p, "features", c, "todos", t, "tasks", k] => {
            format!("update task {k} ({c}/{t})")
        }
        ["projects", _p, "milestones"] => "add milestone".into(),
        ["projects", _p, "milestones", c] if del => format!("delete milestone {c}"),
        ["projects", _p, "milestones", c] => format!("edit milestone {c}"),
        ["projects", _p, "config", what] => format!("update config: {what}"),
        ["projects", _p, "docs", "folder"] => "configure doc folder".into(),
        ["projects", _p, "docs", "content"] if del => "remove document".into(),
        ["projects", _p, "docs", "content"] => "update document".into(),
        ["projects", _p, "batch"] => body
            .and_then(|b| b.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or_else(|| "batch update".into()),
        _ => format!("{method} {path_only}"),
    }
}
