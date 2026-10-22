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

/// A write's response with the diagram lint's findings added as `diagram_warnings` (FEAT-200),
/// only when there are any; the body is returned as it was otherwise.
fn with_diagram_warnings(
    body: String,
    warnings: &[crate::diagrams::DiagramFinding],
) -> Result<String> {
    if warnings.is_empty() {
        return Ok(body);
    }
    let mut v: Value = serde_json::from_str(&body)
        .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
    v["diagram_warnings"] = json!(warnings);
    ser(&v)
}

/// Who is recording this verdict — required, never defaulted (FEAT-077).
///
/// This used to fall back to the string "unknown", and the monitor sent "reviewed in the monitor":
/// a place, not a person. Twenty-eight approvals on this project's own board carry it. An approval
/// is the one human step the whole gate exists to obtain, so a record that cannot say who gave it
/// is worse than no record — it reads as accountability while carrying none. Refusing here means no
/// caller can produce one by omission; the CLI resolves `--by` from the commit identity and the
/// monitor from `/api/meta`, and a caller with neither is told to set one rather than quietly
/// attributed to nobody.
fn approver(b: &Value) -> Result<String> {
    match str_field(b, "by") {
        Some(who) if !who.trim().is_empty() => Ok(who),
        _ => Err(CoreError::Unsupported(
            "an approval must name who gave it: pass `by` (the CLI defaults it to the data \
             folder's commit identity — set one with `kanbanr identity --name … --email …`)"
                .into(),
        )),
    }
}

/// A verdict covers what the person was shown (FEAT-159). When the request names the revision of
/// the definition it was given on, it is refused if the definition has moved on since — so a brief
/// shown in one moment and a yes recorded in the next can never cover different text.
fn shown_rev(store: &Store, p: &str, code: &str, b: &Value) -> Result<()> {
    let Some(shown) = str_field(b, "rev").filter(|r| !r.trim().is_empty()) else {
        return Ok(());
    };
    let project = store.load_meta(p)?;
    let now = project
        .feature(code)?
        .definition
        .as_ref()
        .map(|d| d.content_rev())
        .unwrap_or_default();
    if now != shown.trim() {
        return Err(CoreError::Unsupported(format!(
            "{code}'s definition changed after it was shown (shown {shown}, now {now}): show the \
             brief again and ask again — nothing was recorded"
        )));
    }
    Ok(())
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

/// A project's codes as `config codes` shows them (FEAT-217): the effective default, each kind's
/// prefix, and per prefix configured or in use, the next code and how many items have it.
#[derive(Debug, Serialize)]
pub struct CodesView {
    pub default: String,
    pub kinds: BTreeMap<String, String>,
    pub next: BTreeMap<String, String>,
    pub in_use: BTreeMap<String, usize>,
}

pub fn codes_view(project: &crate::Project) -> CodesView {
    let codes = &project.config.codes;
    let existing: Vec<String> = project.features.iter().map(|f| f.code.clone()).collect();
    let mut in_use: BTreeMap<String, usize> = BTreeMap::new();
    for code in &existing {
        if let Some((prefix, n)) = code.rsplit_once('-')
            && !prefix.is_empty()
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            *in_use.entry(prefix.to_string()).or_default() += 1;
        }
    }
    let mut prefixes = codes.prefixes();
    prefixes.extend(in_use.keys().cloned());
    let next = prefixes
        .into_iter()
        .map(|p| {
            let code = crate::validate::next_code(&p, &existing);
            (p, code)
        })
        .collect();
    CodesView {
        default: codes.default_prefix().to_string(),
        kinds: codes.kinds.clone(),
        next,
        in_use,
    }
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
    let sched_keys = ["start", "estimate_days", "estimate", "points"];
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
        if estimate.is_some() || b.get("start").is_some() {
            f = Some(store.set_feature_schedule(p, code, s("start"), estimate)?);
        }
        if let Some(points) = b.get("points") {
            f = Some(store.set_feature_points(p, code, points.as_f64().unwrap_or(0.0))?);
        }
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

pub(crate) fn query_param(query: Option<&str>, key: &str) -> Option<String> {
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

/// One project on the board, as the home page lists it: `GET /projects`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectSummary {
    pub id: String,
    /// The project's name; its id when it has none.
    pub name: String,
    pub description: String,
    /// The statuses shown as board columns.
    pub displayed_states: Vec<String>,
    /// The statuses that are not columns.
    pub other_states: Vec<String>,
    pub no_op_states: Vec<String>,
    /// Items per status, for every status.
    pub counts: BTreeMap<String, usize>,
    pub total_features: usize,
    pub has_docs: bool,
}

fn list_summaries(store: &Store) -> Result<Vec<ProjectSummary>> {
    let mut out = Vec::new();
    for id in store.list_projects()? {
        let Ok(project) = store.load(&id) else {
            continue;
        };
        let cfg = &project.config;
        let displayed = cfg.board_states().to_vec();
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

fn build_project_config(store: &Store, name: &str, body: &Value) -> Result<ProjectConfig> {
    // A process chosen at creation: `"workflow": "<preset>"` (FEAT-116). An unknown name is refused
    // with the list, where it used to fall back silently to the default.
    if let Some(preset) = str_field(body, "workflow") {
        let (file, library) = store.resolve_process(&preset)?;
        let source = file.source(&preset, library);
        let mut config = file.into_config(name);
        config.process = Some(source);
        if let Some(d) = str_field(body, "description") {
            config.description = d;
        }
        return Ok(config);
    }
    // A process from the user's own library (FEAT-169): the CLI sends the file and where it came
    // from, because the board cannot see the user's home folder.
    if let Some(v) = body.get("workflow_file").filter(|v| !v.is_null()) {
        let file: crate::config::WorkflowFile = serde_json::from_value(v.clone())
            .map_err(|e| CoreError::Unsupported(format!("invalid workflow file: {e}")))?;
        if let Some(problem) = file.problems().into_iter().next() {
            return Err(problem);
        }
        let mut config = file.into_config(name);
        config.process = process_source(body)?;
        if let Some(d) = str_field(body, "description") {
            config.description = d;
        }
        return Ok(config);
    }
    let statuses = vec_field(body, "statuses");
    let displayed = vec_field(body, "displayed_states");
    let default_state = str_field(body, "default_state");
    let no_op = vec_field(body, "no_op_states");
    let mut config = match &statuses {
        Some(s) => {
            let no_ops = no_op.clone().unwrap_or_default();
            ProjectConfig {
                extra: Default::default(),
                branch_pattern: None,
                schema_version: crate::config::BASE_SCHEMA_VERSION,
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
                gates: Default::default(),
                estimate_unit: Default::default(),
                cadence: Default::default(),
                process: None,
                method: Default::default(),
                diagrams: Default::default(),
                codes: Default::default(),
                coverage: Vec::new(),
            }
        }
        None => {
            let mut c = ProjectConfig::for_new_project(name);
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
    Ok(config)
}

fn apply_workflow(store: &Store, p: &str, body: &Value) -> Result<String> {
    // A preset to start from (FEAT-116): `preset: <name>`, or the older `togaf` / `defaults` flags.
    let preset = str_field(body, "preset").or_else(|| {
        if bool_field(body, "togaf").unwrap_or(false) {
            Some("togaf".to_string())
        } else if bool_field(body, "defaults").unwrap_or(false) {
            Some("default".to_string())
        } else {
            None
        }
    });
    // The process this workflow comes from (FEAT-169): a named one, looked up on the board and
    // then among the built-in ones, or one the CLI read from the user's library and says so. A
    // workflow from a file or from flags comes from no saved process, and clears the record.
    let mut source = process_source(body)?;
    let base = match preset {
        Some(name) => {
            let (file, library) = store.resolve_process(&name)?;
            // Shaped by the project's own choice, so a part it turned off stays off (FEAT-193).
            let choice = store.load_meta(p)?.config.method;
            let file = crate::method::shaped(file, &choice);
            source = Some(file.source(&name, library));
            Some(file.into_config(p))
        }
        None => None,
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
    let gates: Option<BTreeMap<String, crate::config::Gate>> = match body.get("gates") {
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid gates: {e}")))?,
        ),
        None => base.as_ref().map(|x| x.gates.clone()),
    };
    let config = store.set_workflow_with_gates(
        p,
        statuses,
        transitions,
        default_state,
        displayed,
        no_ops,
        terminals,
        gates,
    )?;
    // A process that works in sprints, releases or points brings that with it (FEAT-122); one that
    // does not leaves the project's cadence as it was.
    let cadence: Option<crate::config::Cadence> = match body.get("cadence").filter(|v| !v.is_null())
    {
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid cadence: {e}")))?,
        ),
        None => base
            .as_ref()
            .map(|x| x.cadence.clone())
            .filter(|c| !c.is_off()),
    };
    let unit: Option<crate::config::EstimateUnit> =
        match body.get("estimate_unit").filter(|v| !v.is_null()) {
            Some(v) => Some(
                serde_json::from_value(v.clone())
                    .map_err(|e| CoreError::Unsupported(format!("invalid estimate unit: {e}")))?,
            ),
            None => base
                .as_ref()
                .map(|x| x.estimate_unit)
                .filter(|u| !u.is_days()),
        };
    let config = match (cadence, unit) {
        (None, None) => config,
        (cadence, unit) => {
            let mut cadence = cadence.unwrap_or_else(|| config.cadence.clone());
            // FEAT-212: a process that says nothing of the release stages' gates or preparation
            // keeps the project's own; if its stages no longer fit them, the refusal names the fix.
            if cadence.release_gates.is_empty() {
                cadence.release_gates = config.cadence.release_gates.clone();
            }
            if cadence.release_prepare.is_empty() {
                cadence.release_prepare = config.cadence.release_prepare.clone();
            }
            store.set_cadence(p, unit, cadence)?
        }
    };
    if config.process == source {
        return ser(&config);
    }
    ser(&store.set_process_source(p, source)?)
}

/// `"process": {name, library, version, rev}` in a body: where the workflow it carries came from.
fn process_source(body: &Value) -> Result<Option<crate::config::ProcessSource>> {
    match body.get("process").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| CoreError::Unsupported(format!("invalid process source: {e}"))),
    }
}

// ---- the dispatcher ------------------------------------------------------------------------

/// The media type of what a successful `GET` of `path` (with its query) answers with (FEAT-177):
/// JSON, unless the route renders text — a diagram, a document, a Markdown export. The daemon
/// labels its bodies with this, so a body labelled JSON is JSON.
pub fn read_media_type(path: &str) -> &'static str {
    const TEXT: &str = "text/plain; charset=utf-8";
    const MARKDOWN: &str = "text/markdown; charset=utf-8";
    const JSON: &str = "application/json";
    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let segs: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    let format = query_param(query, "format");
    match segs.as_slice() {
        ["projects", _, "workflow"] | ["projects", _, "gantt"] | ["gantt"] => TEXT,
        ["projects", _, "graph"] | ["graph"] if format.as_deref() == Some("dot") => TEXT,
        ["projects", _, "export"] | ["projects", _, "features", _, "export"]
            if format.as_deref() != Some("json") =>
        {
            MARKDOWN
        }
        ["projects", _, "docs", "content"] => MARKDOWN,
        ["projects", _, "scopes", _, "brief"] => MARKDOWN,
        _ => JSON,
    }
}

/// Apply a data operation against the store, returning the response body string. Reads and writes
/// alike — the caller (e.g. the CLI's local backend) decides whether to commit afterwards based on
/// the HTTP method.
pub fn dispatch(store: &Store, method: &str, path: &str, body: Option<&Value>) -> Result<String> {
    dispatch_in(store, method, path, body, None)
}

/// [`dispatch`], with the project's code repository when the caller has one (FEAT-183). Only
/// release promotion reaches it: the CLI passes the repository it runs in, the daemon the one it
/// was started in when that folder's marker names the project. Without one, the promotions that
/// need it — the first stage and the cut — are refused, never guessed.
pub fn dispatch_in(
    store: &Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&std::path::Path>,
) -> Result<String> {
    let out = dispatch_routes(store, method, path, body, repo)?;
    // After a write, the coverage rules it now fails (FEAT-197). A dry run changed nothing.
    if !is_mutation(method) || is_dry_run(body) {
        return Ok(out);
    }
    Ok(crate::coverage::annotate(
        store,
        method,
        path,
        body.unwrap_or(&Value::Null),
        out,
    ))
}

/// The routes themselves: [`dispatch_in`] without what it adds after a write.
fn dispatch_routes(
    store: &Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&std::path::Path>,
) -> Result<String> {
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
        // The board's README (FEAT-201): written now, keeping its owner's text.
        ("POST", ["readme"]) => ser(&json!({ "changed": crate::boardreadme::write(store)? })),
        ("GET", ["projects", p, "doctor"]) => {
            let mut report = crate::doctor::run_project(store, p)?;
            // With the git model on, the branches are checked against the board too (FEAT-191).
            if store.load_meta(p)?.config.cadence.git_model {
                report.issues.extend(git_drift(store, p, repo));
            }
            ser(&report)
        }
        // Drift between the board and the branches (FEAT-191), on demand.
        ("GET", ["projects", p, "git", "doctor"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "git doctor reads the code repository: run it from the project folder".into(),
                )
            })?;
            ser(&crate::gitdrift::scan(store, p, repo)?)
        }

        // ---- the board's process library (FEAT-169) ----
        ("GET", ["processes"]) => ser(&crate::library::listing(store)?),
        ("GET", ["processes", name]) => {
            let (file, library) = store.resolve_process(name)?;
            ser(&json!({ "name": name, "library": library, "file": file }))
        }
        ("PUT", ["processes", name]) => {
            let file: crate::config::WorkflowFile =
                serde_json::from_value(b.get("file").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CoreError::Unsupported(format!("invalid process file: {e}")))?;
            ser(&store.save_process(name, file, str_field(b, "description"))?)
        }

        // How a project stands against its saved process (FEAT-170): read-only, for the CLI and
        // the monitor. A personal process is the user's kanbanr's to compare; here it is named only.
        ("GET", ["projects", p, "process"]) => {
            let project = store.load(p)?;
            let drift = store.process_drift(&project, None)?;
            ser(&crate::library::ProcessStatus {
                source: project.config.process,
                messages: drift.as_ref().map(|d| d.messages()).unwrap_or_default(),
                drift,
            })
        }

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
            let config = build_project_config(store, &name, b)?;
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
            let spec = str_field(b, "specification").unwrap_or_default();
            let kind = str_field(b, "kind");
            let f = store.add_feature_of_kind(
                p,
                &str_field(b, "title").unwrap_or_default(),
                &spec,
                &str_field(b, "milestone").unwrap_or_default(),
                str_field(b, "code"),
                kind.as_deref(),
            )?;
            let warnings = crate::diagrams::lint_markdown(&format!("{} spec", f.code), &spec);
            let attrs = maybe_apply_attrs(store, p, &f.code, b)?;
            // Feedback on a release says where it was found, the way a defect does (FEAT-120):
            // recorded with the item, in the same write.
            let body = if let Some(version) = str_field(b, "found_in") {
                let defect = crate::models::Defect {
                    found_in: version,
                    ..Default::default()
                };
                ser(&store.set_defect(p, &f.code, Some(defect))?)?
            } else {
                match attrs {
                    Some(json) => json,
                    None => ser(&f)?,
                }
            };
            with_diagram_warnings(body, &warnings)
        }
        ("PATCH", ["projects", p, "features", code]) => {
            let milestone = if bool_field(b, "clear_milestone").unwrap_or(false) {
                Some(None)
            } else {
                str_field(b, "milestone").map(Some)
            };
            let spec = str_field(b, "specification");
            let edited = store.edit_feature(
                p,
                code,
                str_field(b, "title"),
                spec.clone(),
                milestone,
                str_field(b, "new_code"),
            )?;
            let warnings = spec
                .map(|s| crate::diagrams::lint_markdown(&format!("{} spec", edited.code), &s))
                .unwrap_or_default();
            // new_code may have renamed it; attrs apply to the (possibly new) code.
            let body = match maybe_apply_attrs(store, p, &edited.code, b)? {
                Some(json) => json,
                None => ser(&edited)?,
            };
            with_diagram_warnings(body, &warnings)
        }
        ("POST", ["projects", p, "features", code, "move"]) => {
            // Entering an active status needs a current approval (FEAT-048); an explicit
            // `unapproved` reason is recorded rather than silently allowed.
            // `override` is the name since gates became declarable (FEAT-113); `unapproved` stays.
            let reason = str_field(b, "override").or_else(|| str_field(b, "unapproved"));
            // With the git model on, a move that integrates merges the item's branch (FEAT-190).
            let moved = crate::gitmodel::move_item(
                store,
                p,
                code,
                &str_field(b, "to").unwrap_or_default(),
                reason.as_deref(),
                repo,
            )?;
            let mut out = serde_json::to_value(&moved.feature)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if !moved.warnings.is_empty() {
                out["gate_warnings"] = json!(
                    moved
                        .warnings
                        .iter()
                        .map(|g| &g.message)
                        .collect::<Vec<_>>()
                );
            }
            if let Some(merged) = &moved.merged {
                out["merged"] = json!(merged);
            }
            if let Some(v) = &moved.forward_needed {
                out["forward_needed"] = json!(v);
            }
            if let Some(note) = &moved.note {
                out["note"] = json!(note);
            }
            ser(&out)
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
        ("POST", ["projects", p, "features", code, "unapprove"]) => ser(&store.unapprove_feature(
            p,
            code,
            &approver(b)?,
            &str_field(b, "reason").unwrap_or_default(),
        )?),
        ("POST", ["projects", p, "features", code, "ratify"]) => ser(&{
            shown_rev(store, p, code, b)?;
            store.ratify_feature(
                p,
                code,
                &approver(b)?,
                &str_field(b, "reason").unwrap_or_default(),
            )?
        }),
        ("POST", ["projects", p, "features", code, "approve"]) => {
            shown_rev(store, p, code, b)?;
            ser(&store.approve_feature(p, code, &approver(b)?)?)
        }
        // Withdraw a sign-off given in error (FEAT-182). Matched before the sign-off route.
        ("POST", ["projects", p, "features", code, "signoff", name, "withdraw"]) => ser(&store
            .withdraw_signoff(
                p,
                code,
                &percent_decode(name),
                &approver(b)?,
                &str_field(b, "reason").unwrap_or_default(),
            )?),
        ("POST", ["projects", p, "features", code, "signoff", name]) => ser(&{
            shown_rev(store, p, code, b)?;
            store.signoff_feature(
                p,
                code,
                &percent_decode(name),
                &approver(b)?,
                &str_field(b, "note").unwrap_or_default(),
                &str_field(b, "doc").unwrap_or_default(),
            )?
        }),
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
            let hold = str_field(b, "hold_advance");
            let (feature, held) =
                store.set_task_state_held(p, code, todo, key, state, hold.as_deref())?;
            let mut out = serde_json::to_value(&feature)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if let Some(held) = held {
                out["auto_advance_held"] = json!(held);
            }
            ser(&out)
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
        // ---- releases (FEAT-120) ----
        ("GET", ["projects", p, "releases"]) => ser(&crate::releases::report_all(store, p)?),
        // `patch_of`: a patch of a cut release, branched from its tag (FEAT-184).
        ("POST", ["projects", p, "releases"]) => match str_field(b, "patch_of") {
            Some(parent) => ser(&crate::releases::add_patch(
                store,
                p,
                &str_field(b, "version").unwrap_or_default(),
                &str_field(b, "target").unwrap_or_default(),
                &str_field(b, "name").unwrap_or_default(),
                &parent,
                &str_field(b, "by").unwrap_or_default(),
                repo,
            )?),
            None => ser(&crate::releases::add(
                store,
                p,
                &str_field(b, "version").unwrap_or_default(),
                &str_field(b, "target").unwrap_or_default(),
                &str_field(b, "name").unwrap_or_default(),
            )?),
        },
        ("POST", ["projects", p, "releases", version, "plan"]) => {
            let items = vec_field(b, "items").unwrap_or_default();
            ser(&crate::releases::plan(
                store,
                p,
                &percent_decode(version),
                &items,
            )?)
        }
        ("POST", ["projects", p, "releases", version, "cut"]) => {
            ser(&crate::releases::cut(store, p, &percent_decode(version))?)
        }
        // What cutting a release now would ship and carry over, by the cut's own rule, with nothing
        // written (FEAT-185): the monitor's production confirmation lists exactly this.
        ("GET", ["projects", p, "releases", version, "cut-preview"]) => ser(
            &crate::releases::cut_preview(store, p, &percent_decode(version))?,
        ),
        // FEAT-183: `{stage?, by, reason?, expect?}` — the next stage, or the named one. `expect`
        // is the release tip the caller's plan showed (FEAT-212): a branch moved since is refused.
        ("POST", ["projects", p, "releases", version, "promote"]) => {
            let expect = str_field(b, "expect");
            ser(&crate::releases::promote_with(
                store,
                p,
                &percent_decode(version),
                str_field(b, "stage").as_deref(),
                &str_field(b, "by").unwrap_or_default(),
                &str_field(b, "reason").unwrap_or_default(),
                repo,
                crate::releases::PromoteOptions {
                    expect_tip: expect.as_deref(),
                    fetch: crate::releases::Fetch::Try,
                },
            )?)
        }
        // What promoting a release would do, with nothing changed (FEAT-212): `?stage=` (default
        // the next), `?reason=` for a move back, `?fetch=1` to fetch main first (the CLI's
        // `--plan --fetch`; the monitor never asks).
        ("GET", ["projects", p, "releases", version, "plan"]) => {
            let fetch = match query_param(query, "fetch").as_deref() {
                Some("1") | Some("true") => crate::releases::Fetch::Try,
                _ => crate::releases::Fetch::No,
            };
            ser(&crate::releases::plan_promotion(
                store,
                p,
                &percent_decode(version),
                query_param(query, "stage").as_deref(),
                &query_param(query, "reason").unwrap_or_default(),
                repo,
                fetch,
            )?)
        }
        // A release's notes as they are on release/<V> (FEAT-216), read with the code repository.
        ("GET", ["projects", p, "releases", version, "notes"]) => ser(
            &crate::releases::release_notes(store, p, &percent_decode(version), repo)?,
        ),
        // `{text, by}`: commit the notes onto release/<V>, nothing checked out (FEAT-216).
        ("PUT", ["projects", p, "releases", version, "notes"]) => {
            let Some(text) = str_field(b, "text") else {
                return Err(CoreError::Unsupported(
                    "saving release notes needs their text".into(),
                ));
            };
            ser(&crate::releases::set_release_notes(
                store,
                p,
                &percent_decode(version),
                repo,
                &text,
                &str_field(b, "by").unwrap_or_default(),
            )?)
        }
        // The release stages with the gate in force on each (FEAT-212).
        ("GET", ["projects", p, "release-stages"]) => {
            ser(&store.load_meta(p)?.config.cadence.release_stage_views())
        }
        // `{purpose?, requires?, on_enter?, clear?}`: declare a stage's gate, starting from the
        // one in force, or clear it back to the default (FEAT-212).
        ("PUT", ["projects", p, "config", "release-stages", stage]) => {
            let stage = percent_decode(stage);
            let gate = if bool_field(b, "clear").unwrap_or(false) {
                None
            } else {
                let cadence = store.load_meta(p)?.config.cadence;
                let mut gate = cadence
                    .effective_release_gates()
                    .into_iter()
                    .find(|(s, _)| *s == stage)
                    .map(|(_, g)| g)
                    .ok_or_else(|| {
                        CoreError::Unsupported(format!(
                            "'{stage}' is not a release stage — this project's are {}",
                            cadence.stages().join(", ")
                        ))
                    })?;
                if let Some(purpose) = str_field(b, "purpose") {
                    gate.purpose = purpose.trim().to_string();
                }
                if let Some(v) = b.get("requires").filter(|v| !v.is_null()) {
                    gate.requires = serde_json::from_value(v.clone())
                        .map_err(|e| CoreError::Unsupported(e.to_string()))?;
                }
                if let Some(v) = b.get("on_enter").filter(|v| !v.is_null()) {
                    gate.on_enter = serde_json::from_value(v.clone())
                        .map_err(|e| CoreError::Unsupported(e.to_string()))?;
                }
                Some(gate)
            };
            let config = store.set_release_gate(p, &stage, gate)?;
            ser(&config.cadence.release_stage_views())
        }
        // `{version_files?, changelog?, changelog_copies?, notes?, notes_listings?}`: what
        // `prepare` writes (FEAT-212). A field left out is kept; `""` clears a path.
        ("PUT", ["projects", p, "config", "release-prepare"]) => {
            let mut prep = store.load_meta(p)?.config.cadence.release_prepare;
            let invalid = |e: serde_json::Error| CoreError::Unsupported(e.to_string());
            if let Some(v) = b.get("version_files").filter(|v| !v.is_null()) {
                prep.version_files = serde_json::from_value(v.clone()).map_err(invalid)?;
            }
            if let Some(c) = str_field(b, "changelog") {
                prep.changelog = Some(c.trim().to_string()).filter(|c| !c.is_empty());
            }
            if let Some(copies) = vec_field(b, "changelog_copies") {
                prep.changelog_copies = copies
                    .into_iter()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
            }
            if let Some(n) = str_field(b, "notes") {
                prep.notes = Some(n.trim().to_string()).filter(|n| !n.is_empty());
            }
            if let Some(v) = b.get("notes_listings").filter(|v| !v.is_null()) {
                prep.notes_listings = serde_json::from_value(v.clone()).map_err(invalid)?;
            }
            ser(&store.set_release_prepare(p, prep)?.cadence.release_prepare)
        }
        // ---- sprints (FEAT-119) ----
        ("GET", ["projects", p, "sprints"]) => ser(&crate::sprints::load(store, p)?),
        ("GET", ["projects", p, "sprints", code]) => {
            let code = match *code {
                "active" => crate::sprints::active(&crate::sprints::load(store, p)?)
                    .map(|s| s.code.clone())
                    .ok_or_else(|| CoreError::Unsupported("no sprint is active".into()))?,
                c => c.to_string(),
            };
            ser(&crate::sprints::report(
                store,
                p,
                &code,
                crate::sprints::today(),
            )?)
        }
        ("POST", ["projects", p, "sprints"]) => ser(&crate::sprints::add(
            store,
            p,
            &str_field(b, "start").unwrap_or_default(),
            b.get("length_days")
                .and_then(Value::as_u64)
                .map(|d| d as u32),
            &str_field(b, "goal").unwrap_or_default(),
            b.get("capacity").and_then(Value::as_f64),
            &str_field(b, "name").unwrap_or_default(),
        )?),
        ("POST", ["projects", p, "sprints", code, "plan"]) => {
            let items = vec_field(b, "items").unwrap_or_default();
            ser(&crate::sprints::plan(store, p, code, &items)?)
        }
        // With the git model on, starting a sprint makes its branch (FEAT-189).
        ("POST", ["projects", p, "sprints", code, "start"]) => {
            ser(&crate::gitmodel::sprint_start(store, p, code, repo)?)
        }
        // The git model the cadence implies (FEAT-189): always derivable, with the repository's
        // facts when there is one to read.
        ("GET", ["projects", p, "git", "model"]) => {
            // `?sprints=on|off&releases=on|off`: the model that cadence would give, over this
            // board's (FEAT-194 R-2). A preview, so it needs no repository.
            let on = |k: &str| query_param(query, k).map(|v| v == "on");
            let (sprints_on, releases_on) = (on("sprints"), on("releases"));
            if sprints_on.is_some() || releases_on.is_some() {
                let config = store.load_meta(p)?.config;
                let sprints = crate::sprints::load(store, p)?;
                let model =
                    crate::gitmodel::preview(config, sprints_on, releases_on, &sprints, repo);
                return ser(&crate::gitmodel::view(&model));
            }
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "the git model reads the code repository: run it from the project folder"
                        .into(),
                )
            })?;
            let (model, facts) = crate::gitmodel::load(store, p, repo)?;
            // `?item=CODE`: where that item's branch starts (FEAT-189, R-2).
            let start = match query_param(query, "item") {
                Some(code) => {
                    let project = store.load_meta(p)?;
                    let item = project.feature(&code)?;
                    let releases = crate::releases::load(store, p).unwrap_or_default();
                    let sprints = crate::sprints::load(store, p)?;
                    Some(crate::gitmodel::start_base(
                        &model, &facts, item, &releases, &sprints,
                    ))
                }
                None => None,
            };
            let mut out = crate::gitmodel::view(&model);
            out["facts"] = serde_json::to_value(&facts).unwrap_or_default();
            out["start"] = serde_json::to_value(&start).unwrap_or_default();
            ser(&out)
        }
        // With the git model on, a sprints-only close moves production (FEAT-190).
        ("POST", ["projects", p, "sprints", code, "close"]) => {
            let closed = crate::gitmodel::sprint_close(
                store,
                p,
                code,
                str_field(b, "carry_to").as_deref(),
                repo,
            )?;
            let mut out = serde_json::to_value(&closed.sprint)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if let Some(tag) = &closed.tagged {
                out["tagged"] = json!(tag);
            }
            if !closed.catch_up.is_empty() {
                out["catch_up"] = json!(closed.catch_up);
            }
            ser(&out)
        }
        // A release's fixes merged forward into the line, on request (FEAT-190).
        ("POST", ["projects", p, "git", "forward", version]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "a forward merge acts on the code repository: run it from the project folder"
                        .into(),
                )
            })?;
            ser(&crate::gitmodel::forward(
                store,
                p,
                &percent_decode(version),
                repo,
            )?)
        }
        // The gates moves are judged by (FEAT-199): role-addressed gates on their stage, and the
        // built-in rule where a workflow declares none. The monitor's Workflow page reads these.
        ("GET", ["projects", p, "gates"]) => ser(&store.load_meta(p)?.config.effective_gates()),
        // Every guard rail the project has on, in one read (FEAT-213): the monitor's Workflow page
        // lists them. Internal, not in the published API.
        ("GET", ["projects", p, "guards"]) => ser(&crate::guards::report(store, p, repo)?),
        // The method, on by default (FEAT-193): which parts a project keeps.
        ("GET", ["projects", p, "config", "method"]) => {
            ser(&crate::method::status(&store.load_meta(p)?.config))
        }
        ("PUT", ["projects", p, "config", "method"]) => {
            let on = bool_field(b, "on").ok_or_else(|| {
                CoreError::Unsupported("say whether to turn the parts on or off: `on`".into())
            })?;
            let parts = crate::method::parse_parts(&vec_field(b, "parts").unwrap_or_default())?;
            let (config, notes) = store.set_method(p, on, &parts)?;
            ser(&serde_json::json!({
                "parts": crate::method::status(&config),
                "notes": notes,
            }))
        }
        // How the project treats diagrams the lint finds problems in (FEAT-200).
        ("GET", ["projects", p, "config", "diagrams"]) => {
            ser(&store.read_project_config(p)?.diagrams)
        }
        ("PUT", ["projects", p, "config", "diagrams"]) => {
            let strict = bool_field(b, "strict").ok_or_else(|| {
                CoreError::Unsupported("say whether to check diagrams strictly: `strict`".into())
            })?;
            let current = store.read_project_config(p)?.diagrams;
            let diagrams = crate::config::DiagramsConfig {
                strict,
                extra: current.extra,
            };
            ser(&store.set_diagrams(p, diagrams)?.diagrams)
        }
        // Which prefix each kind's code gets, and the next code per prefix (FEAT-217).
        ("GET", ["projects", p, "config", "codes"]) => ser(&codes_view(&store.load_meta(p)?)),
        ("PUT", ["projects", p, "config", "codes"]) => {
            let change: crate::config::CodesChange = if b.is_null() {
                Default::default()
            } else {
                serde_json::from_value(b.clone())
                    .map_err(|e| CoreError::Unsupported(format!("a codes change: {e}")))?
            };
            store.set_codes(p, change)?;
            ser(&codes_view(&store.load_meta(p)?))
        }
        // Architecture coverage rules (FEAT-197): each rule and what it leaves uncovered; set from
        // YAML text (`text`, parsed here in core) or as `rules`, refused when invalid.
        ("GET", ["projects", p, "coverage"]) => {
            let project = store.load_meta(p)?;
            if project.config.coverage.is_empty() {
                return ser(&crate::coverage::CoverageReport::default());
            }
            let snap = crate::coverage::Snapshot::load(store, &project)?;
            ser(&snap.report(&project))
        }
        ("PUT", ["projects", p, "coverage"]) => {
            let rules = match (str_field(b, "text"), b.get("rules")) {
                (Some(text), _) => crate::coverage::parse_rules(&text)?,
                (None, Some(rules)) => serde_json::from_value(rules.clone())
                    .map_err(|e| CoreError::Unsupported(format!("invalid coverage rules: {e}")))?,
                (None, None) => {
                    return Err(CoreError::Unsupported(
                        "give the coverage rules: `text` (YAML) or `rules`".into(),
                    ));
                }
            };
            store.set_coverage(p, rules)?;
            let project = store.load_meta(p)?;
            let snap = crate::coverage::Snapshot::load(store, &project)?;
            ser(&snap.report(&project))
        }
        ("PUT", ["projects", p, "config", "cadence"]) => {
            let unit = match str_field(b, "estimate_unit").as_deref() {
                None => None,
                Some("points") => Some(crate::config::EstimateUnit::Points),
                Some("days") => Some(crate::config::EstimateUnit::Days),
                Some(other) => {
                    return Err(CoreError::Unsupported(format!(
                        "estimate unit '{other}' is not days or points"
                    )));
                }
            };
            let current = store.load_meta(p)?.config.cadence;
            let cadence = crate::config::Cadence {
                extra: current.extra.clone(),
                // FEAT-212: the stages' gates and what `prepare` writes have routes of their own;
                // a cadence write that does not name them keeps them.
                release_gates: current.release_gates.clone(),
                release_prepare: current.release_prepare.clone(),
                sprints: bool_field(b, "sprints").unwrap_or(current.sprints),
                releases: bool_field(b, "releases").unwrap_or(current.releases),
                sprint_length_days: b
                    .get("sprint_length_days")
                    .and_then(Value::as_u64)
                    .map(|d| d as u32)
                    .or(current.sprint_length_days),
                release: str_field(b, "release").or(current.release),
                // FEAT-183. A list given empty is refused rather than read as "the default": the
                // last stage is the cut, so a project with none could never ship.
                release_stages: match vec_field(b, "release_stages") {
                    Some(stages) if stages.is_empty() => {
                        return Err(CoreError::Unsupported(
                            "a release needs at least one stage — the last one is the cut".into(),
                        ));
                    }
                    Some(stages) => stages.iter().map(|s| s.trim().to_string()).collect(),
                    None => current.release_stages,
                },
                // An empty string clears the branch back to the repository's default.
                integration_branch: match str_field(b, "integration_branch") {
                    Some(name) => Some(name.trim().to_string()).filter(|n| !n.is_empty()),
                    None => current.integration_branch,
                },
                main_branch: match str_field(b, "main_branch") {
                    Some(name) => Some(name.trim().to_string()).filter(|n| !n.is_empty()),
                    None => current.main_branch,
                },
                // FEAT-189: branching follows the git model only when switched on.
                git_model: bool_field(b, "git_model").unwrap_or(current.git_model),
            };
            ser(&store.set_cadence(p, unit, cadence)?)
        }
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
            let path = str_field(b, "path").unwrap_or_default();
            let content = str_field(b, "content").unwrap_or_default();
            // FEAT-200: a markdown doc's diagrams are linted before it is written — refused on a
            // strict project, reported as warnings otherwise.
            let warnings = if crate::diagrams::is_markdown_path(&path) {
                crate::diagrams::guard(
                    &store.read_project_config(p)?.diagrams,
                    &format!("docs/{path}"),
                    &content,
                )?
            } else {
                Vec::new()
            };
            let saved = store.write_doc(p, &path, &content)?;
            with_diagram_warnings(json!({ "path": saved }).to_string(), &warnings)
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

        // ---- lessons learned (FEAT-055) ----
        ("GET", ["projects", p, "lessons"]) => {
            // `for` narrows to the lessons that bear on one item; `all` includes retired ones.
            let all = query_param(query, "all").is_some();
            let lessons = match query_param(query, "for") {
                Some(code) => {
                    let project = store.load_meta(p)?;
                    let feature = project.feature(&code)?.clone();
                    crate::lessons::surfaced(store, p, Some(&feature))?
                }
                None if all => crate::lessons::load(store, p)?,
                None => crate::lessons::surfaced(store, p, None)?,
            };
            ser(&lessons)
        }
        ("POST", ["projects", p, "lessons"]) => {
            let lesson: crate::lessons::Lesson = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid lesson: {e}")))?;
            ser(&crate::lessons::add(store, p, lesson)?)
        }
        ("POST", ["projects", p, "lessons", id, verdict]) => {
            let affirm = match *verdict {
                "affirm" => true,
                "contradict" => false,
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "a lesson is affirmed or contradicted, not '{other}'"
                    )));
                }
            };
            ser(&crate::lessons::judge(
                store,
                p,
                id,
                affirm,
                &str_field(b, "note").unwrap_or_default(),
            )?)
        }

        // ---- traceability and decisions (FEAT-057) ----
        ("GET", ["projects", p, "trace"]) => {
            let subject = query_param(query, "subject").unwrap_or_default();
            let item = query_param(query, "item");
            let subject =
                crate::trace::parse_subject(&subject, item.as_deref()).ok_or_else(|| {
                    CoreError::Unsupported(
                    "trace what? a goal (G-2), an item (FEAT-046), a requirement (FEAT-046/R-2) or a doc (architecture/data.md)"
                        .to_string(),
                )
                })?;
            ser(&crate::trace::run(store, p, &subject)?)
        }
        ("GET", ["projects", p, "zachman"]) => ser(&crate::trace::zachman(
            store,
            p,
            query_param(query, "scope").as_deref(),
        )?),
        // ---- what is waiting for agreement (FEAT-067) ----
        // What an item is missing, from the one engine every surface uses (FEAT-112).
        // The diagrams of an item and the lint's findings (FEAT-200), for the review page's parse.
        ("GET", ["projects", p, "features", code, "diagrams"]) => {
            ser(&crate::diagrams::ItemDiagrams::load(store, p, code)?)
        }
        ("GET", ["projects", p, "features", code, "readiness"]) => {
            ser(&crate::readiness::ReadinessView::load(store, p, code)?)
        }
        // The board's cards, in one request: gaps per live item, keyed by code. Finished and
        // parked items are left out, as doctor leaves them out — they are history, not work.
        ("GET", ["projects", p, "readiness"]) => {
            ser(&crate::readiness::ProjectReadiness::load(store, p)?)
        }
        ("GET", ["projects", p, "review"]) => ser(&crate::review::pending(store, p)?),
        ("GET", ["projects", p, "adrs"]) => ser(&crate::adr::list(store, p)?),
        ("GET", ["projects", p, "claude-block"]) => ser(&crate::claude::block(store, p)?),
        ("POST", ["projects", p, "adrs"]) => {
            let title = str_field(b, "title").unwrap_or_default();
            let adr: crate::adr::Adr = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid decision: {e}")))?;
            ser(&crate::adr::create(store, p, &title, adr)?)
        }
        ("POST", ["projects", p, "adrs", id, "accept"]) => {
            ser(&crate::adr::decide(store, p, id, true, &approver(b)?, "")?)
        }
        ("POST", ["projects", p, "adrs", id, "reject"]) => ser(&crate::adr::decide(
            store,
            p,
            id,
            false,
            &approver(b)?,
            &str_field(b, "reason").unwrap_or_default(),
        )?),
        ("POST", ["projects", p, "adrs", id, "supersede"]) => {
            let replaces = str_field(b, "replaces").unwrap_or_default();
            ser(&crate::adr::supersede(store, p, id, &replaces)?)
        }

        // ---- wave retrospective (FEAT-054) ----
        ("GET", ["projects", p, "retro"]) => {
            let wave = crate::retro::Wave {
                milestone: query_param(query, "milestone"),
                since: query_param(query, "since"),
                label: query_param(query, "label"),
                sprint: query_param(query, "sprint"),
            };
            ser(&crate::retro::run(store, p, &wave)?)
        }
        ("GET", ["projects", p, "retro", "due"]) => ser(&crate::retro::due(store, p)?),

        // ---- an item sliced out of another (FEAT-054) ----
        ("PUT", ["projects", p, "features", code, "split-from"]) => {
            let parent = str_field(b, "parent");
            ser(&store.set_split_from(p, code, parent)?)
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

        // ---- agent scopes (FEAT-214): internal, like the guards ----
        ("GET", ["projects", p, "scopes"]) => {
            store.load_meta(p)?;
            let now = crate::now_rfc3339();
            let views: Vec<crate::agentscope::ScopeView> = crate::agentscope::load(store, p)?
                .iter()
                .map(|s| crate::agentscope::ScopeView::new(s, &now))
                .collect();
            ser(&views)
        }
        ("GET", ["projects", p, "scopes", "drift"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "agent drift reads the code repository: run it from the project folder".into(),
                )
            })?;
            ser(&crate::agentscope::drift(
                store,
                p,
                repo,
                &crate::now_rfc3339(),
            )?)
        }
        ("GET", ["projects", p, "scopes", id]) => {
            let scope = find_scope(store, p, id)?;
            ser(&crate::agentscope::ScopeView::new(
                &scope,
                &crate::now_rfc3339(),
            ))
        }
        ("GET", ["projects", p, "scopes", id, "brief"]) => {
            let scope = find_scope(store, p, id)?;
            Ok(crate::agentscope::brief(&scope, &store.load_meta(p)?))
        }
        ("POST", ["projects", p, "scopes"]) => {
            let role = str_field(b, "role").unwrap_or_default();
            let role = crate::agentscope::Role::parse(&role).ok_or_else(|| {
                CoreError::Unsupported(format!("an agent scope is design or build, not '{role}'"))
            })?;
            let req = crate::agentscope::IssueRequest {
                role,
                item: str_field(b, "item").unwrap_or_default(),
                hours: b.get("hours").and_then(Value::as_u64).map(|h| h as u32),
                note: str_field(b, "note").unwrap_or_default(),
                worktree: str_field(b, "worktree")
                    .filter(|w| !w.trim().is_empty())
                    .map(std::path::PathBuf::from),
                hooks_ready: bool_field(b, "hooks_ready").unwrap_or(false),
            };
            let by = crate::git::verdict_author(store.data_dir(), str_field(b, "by").as_deref())?;
            ser(&crate::agentscope::issue(
                store,
                p,
                &req,
                &by,
                repo,
                &crate::now_rfc3339(),
            )?)
        }
        ("POST", ["projects", p, "scopes", id, "claim"]) => {
            let wt = repo
                .and_then(crate::coderepo::worktree_facts)
                .ok_or_else(|| {
                    CoreError::Unsupported(
                        "a scope is claimed from inside the agent's worktree: run it there".into(),
                    )
                })?;
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::claim(store, p, id, &wt, &now)?;
            ser(&crate::agentscope::ScopeView::new(&scope, &now))
        }
        ("POST", ["projects", p, "scopes", id, "revoke"]) => {
            let by = crate::git::verdict_author(store.data_dir(), str_field(b, "by").as_deref())?;
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::revoke(
                store,
                p,
                id,
                &by,
                &str_field(b, "reason").unwrap_or_default(),
                &now,
            )?;
            ser(&crate::agentscope::ScopeView::new(&scope, &now))
        }
        ("POST", ["projects", p, "scopes", id, "renew"]) => {
            let hours = b
                .get("hours")
                .and_then(Value::as_u64)
                .map(|h| h as u32)
                .unwrap_or(crate::agentscope::DEFAULT_HOURS);
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::renew(store, p, id, hours, &now)?;
            ser(&crate::agentscope::ScopeView::new(&scope, &now))
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
            // Under an agent scope the write path holds every item where it is (FEAT-214).
            let hold = str_field(b, "hold_advance");
            let results = store.apply_batch_held(p, ops, dry_run, hold.as_deref())?;
            Ok(json!({ "results": results }).to_string())
        }

        _ => Err(CoreError::NoSuchRoute(format!("{method} {path_only}"))),
    }
}

/// One scope of project `p`, by id (FEAT-214).
fn find_scope(store: &Store, p: &str, id: &str) -> Result<crate::agentscope::Scope> {
    crate::agentscope::load(store, p)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::Unsupported(format!("no agent scope {id} in {p}")))
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
/// The git drift findings for a project's doctor (FEAT-191): what `gitdrift::scan` finds, or one
/// warning saying why it could not look. Never a failed doctor.
fn git_drift(store: &Store, p: &str, repo: Option<&std::path::Path>) -> Vec<crate::doctor::Issue> {
    let warn = |message: String| crate::doctor::Issue {
        severity: crate::doctor::Severity::Warning,
        project: p.to_string(),
        code: None,
        message,
    };
    match repo {
        None => vec![warn(
            "git drift not checked: run from the project folder".to_string(),
        )],
        Some(repo) => crate::gitdrift::scan(store, p, repo)
            .unwrap_or_else(|e| vec![warn(format!("git drift not checked: {e}"))]),
    }
}

/// [`commit_message`], completed from the write's response where only the response knows: the id
/// of an agent scope being issued (FEAT-214).
pub fn commit_message_after(
    method: &str,
    path: &str,
    body: Option<&Value>,
    response: &str,
) -> String {
    let msg = commit_message(method, path, body);
    let bare = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    if method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "scopes"])
        && let Ok(v) = serde_json::from_str::<Value>(response)
        && let Some(id) = v["scope"]["id"].as_str()
    {
        return msg.replacen(
            "issue agent scope (",
            &format!("issue agent scope {id} ("),
            1,
        );
    }
    msg
}

pub fn commit_message(method: &str, path: &str, body: Option<&Value>) -> String {
    let path_only = path.split('?').next().unwrap_or(path);
    let s: Vec<&str> = path_only.split('/').filter(|x| !x.is_empty()).collect();
    let del = method.eq_ignore_ascii_case("DELETE");
    match s.as_slice() {
        ["readme"] => "write the board README".to_string(),
        ["portfolio", "programs"] => body
            .and_then(|b| b.get("id").and_then(|v| v.as_str()))
            .map(|id| format!("add program {id}"))
            .unwrap_or_else(|| "update portfolio".into()),
        ["processes", name] => format!("save process {}", percent_decode(name)),
        ["projects"] => "create project".into(),
        ["projects", p] if del => format!("delete project {p}"),
        ["projects", p] => format!("edit project {p}"),
        ["projects", _p, "charter"] => "update project charter".into(),
        ["projects", _p, "features", c, "definition"] => format!("define feature {c}"),
        ["projects", _p, "features", c, "defect"] => format!("record defect details for {c}"),
        ["projects", _p, "lessons"] => "record a lesson".into(),
        ["projects", _p, "adrs"] => "record an architecture decision".into(),
        ["projects", _p, "adrs", a, "supersede"] => format!("{a} supersedes an earlier decision"),
        ["projects", _p, "adrs", a, "accept"] => format!("accept decision {a}"),
        ["projects", _p, "adrs", a, "reject"] => format!("reject decision {a}"),
        ["projects", _p, "lessons", l, v] => format!("{v} lesson {l}"),
        ["projects", _p, "features", c, "split-from"] => format!("record what {c} was split from"),
        ["projects", _p, "mirror"] => "configure issue mirror".into(),
        ["projects", _p, "features"] => "add feature item".into(),
        ["projects", _p, "features", c] => format!("edit feature {c}"),
        ["projects", _p, "features", c, "move"] => format!("move feature {c}"),
        ["projects", _p, "features", c, "approve"] => format!("approve definition of {c}"),
        ["projects", _p, "features", c, "signoff", name, "withdraw"] => {
            let reason = body
                .and_then(|b| b.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            format!(
                "withdraw sign-off {} on {c}: {}",
                percent_decode(name),
                reason.trim()
            )
        }
        ["projects", _p, "features", c, "signoff", name] => {
            format!("sign off {} on {c}", percent_decode(name))
        }
        ["projects", _p, "features", c, "ratify"] => {
            format!("ratify {c} — built under a recorded bypass, agreed to after the fact")
        }
        ["projects", _p, "features", c, "unapprove"] => {
            format!("withdraw the approval of {c}")
        }
        ["projects", _p, "features", c, "tests", r, t] => {
            // Decoded, because these arrive percent-encoded (a test name is free text and holds
            // `::`, spaces and `/`). Without this, `git log` carried lines like
            // `test doctor%3A%3Atests%3A%3Aan_unreconciled_bypass...` — the history of a tool whose
            // point is a readable record, written in an encoding meant for a URL.
            format!(
                "test {} of {c}/{} is now {}",
                percent_decode(t),
                percent_decode(r),
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
        ["projects", p, "config", "method"] => {
            let on = body
                .and_then(|b| b.get("on"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let parts: Vec<String> = body
                .and_then(|b| b.get("parts"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| vec!["all".to_string()]);
            format!(
                "method {} {} for {p}",
                parts.join(", "),
                if on { "on" } else { "off" }
            )
        }
        ["projects", _p, "config", what] => format!("update config: {what}"),
        ["projects", _p, "coverage"] => "config: coverage rules".into(),
        ["projects", _p, "sprints"] => "add a sprint".into(),
        ["projects", _p, "releases"] => "add a release".into(),
        // FEAT-183: the stage it went to, which a request for "the next one" does not name.
        ["projects", _p, "releases", v, "promote"] => format!(
            "promote release {} to {}",
            percent_decode(v),
            body.and_then(|b| str_field(b, "stage"))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "its next stage".into())
        ),
        ["projects", _p, "releases", v, "notes"] => {
            format!("release {}: notes edited", percent_decode(v))
        }
        ["projects", _p, "releases", v, what] => format!("{what} release {}", percent_decode(v)),
        ["projects", _p, "sprints", c, what] => format!("{what} sprint {c}"),
        ["projects", _p, "docs", "folder"] => "configure doc folder".into(),
        ["projects", _p, "docs", "content"] if del => "remove document".into(),
        ["projects", _p, "docs", "content"] => "update document".into(),
        // FEAT-214. The id of a scope being issued is the response's: see `commit_message_after`.
        ["projects", _p, "scopes"] => {
            let f = |k: &str| {
                body.and_then(|b| str_field(b, k))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            format!("issue agent scope ({}, {})", f("role"), f("item"))
        }
        ["projects", _p, "scopes", id, "claim"] => format!("claim agent scope {id}"),
        ["projects", _p, "scopes", id, "revoke"] => format!(
            "revoke agent scope {id}: {}",
            body.and_then(|b| str_field(b, "reason"))
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "revoked".into())
        ),
        ["projects", _p, "scopes", id, "renew"] => format!("renew agent scope {id}"),
        ["projects", _p, "batch"] => body
            .and_then(|b| b.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or_else(|| "batch update".into()),
        _ => format!("{method} {path_only}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board with releases on, for the release stage routes.
    fn release_board(tag: &str) -> (Store, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("shop", crate::ProjectConfig::default_for("shop"))
            .unwrap();
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"releases": true, "release_stages": ["candidate", "main", "production"]})),
        )
        .unwrap();
        (store, dir)
    }

    /// FEAT-217: the codes routes show the next code per prefix and set the map; a feature POSTed
    /// with a kind takes its prefix.
    #[test]
    fn codes_routes_show_set_and_allocate() {
        let (store, dir) = release_board("codes-routes");
        assert!(
            std::fs::canonicalize(&dir)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        let call = |method: &str, path: &str, body: Value| {
            dispatch(&store, method, path, Some(&body))
                .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        store
            .add_milestone("shop", "M", "", vec![], Some("MS-001".into()))
            .unwrap();
        let view = call("GET", "/projects/shop/config/codes", Value::Null).unwrap();
        assert_eq!(view["default"], "FEAT");
        assert_eq!(view["next"], serde_json::json!({"FEAT": "FEAT-001"}));
        let view = call(
            "PUT",
            "/projects/shop/config/codes",
            serde_json::json!({"set": {"defect": "DEF", "feature": "FTR"}}),
        )
        .unwrap();
        assert_eq!(view["kinds"]["defect"], "DEF");
        let f = call(
            "POST",
            "/projects/shop/features",
            serde_json::json!({"title": "Crash", "milestone": "MS-001", "kind": "defect"}),
        )
        .unwrap();
        assert_eq!(f["code"], "DEF-001");
        assert_eq!(f["kind"], "defect");
        let view = call("GET", "/projects/shop/config/codes", Value::Null).unwrap();
        assert_eq!(view["next"]["DEF"], "DEF-002");
        assert_eq!(view["next"]["FTR"], "FTR-001");
        assert_eq!(view["in_use"], serde_json::json!({"DEF": 1}));
        let err = call(
            "PUT",
            "/projects/shop/config/codes",
            serde_json::json!({"set": {"spike": "MS"}}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("kind 'spike'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-212 R-1: a stage's gate is declared through its route, starting from the one in force,
    /// read back by `release-stages`, and cleared to the default; `release-prepare` sets what
    /// `prepare` writes. Bad names and orders are refused, saying why.
    #[test]
    fn release_stage_routes_round_trip() {
        let (store, dir) = release_board("stage-routes");
        let call = |method: &str, path: &str, body: Value| {
            dispatch(&store, method, path, Some(&body))
                .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        let stages = call("GET", "/projects/shop/release-stages", Value::Null).unwrap();
        assert_eq!(stages[0]["on_enter"], serde_json::json!(["branch"]));
        assert_eq!(
            stages[2]["on_enter"],
            serde_json::json!(["move_main", "tag"])
        );
        assert_eq!(stages[2]["cut"], true);
        assert_eq!(stages[2]["declared"], false);

        call(
            "PUT",
            "/projects/shop/config/release-stages/candidate",
            serde_json::json!({"on_enter": ["branch", "prepare"]}),
        )
        .unwrap();
        call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"purpose": "main holds the release", "requires": ["items_finished"], "on_enter": ["move_main"]}),
        )
        .unwrap();
        // Only `requires` given: the actions in force (here, `tag` alone now) are kept.
        let stages = call(
            "PUT",
            "/projects/shop/config/release-stages/production",
            serde_json::json!({"requires": ["main_at_release", "pushed"]}),
        )
        .unwrap();
        assert_eq!(stages[1]["purpose"], "main holds the release");
        assert_eq!(stages[2]["on_enter"], serde_json::json!(["tag"]));
        assert_eq!(
            stages[2]["requires"],
            serde_json::json!(["main_at_release", "pushed"])
        );
        assert_eq!(stages[2]["declared"], true);
        let cadence = store.load_meta("shop").unwrap().config.cadence;
        assert_eq!(cadence.release_stages, ["candidate", "main", "production"]);
        assert_eq!(cadence.release_gates.len(), 3);

        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"requires": ["pushd"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'pushd' is not a check"), "{err}");
        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"on_enter": ["move_main", "tag"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("only the last stage"), "{err}");
        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/staging",
            serde_json::json!({"on_enter": ["branch"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'staging' is not a release stage"), "{err}");
        // Renaming a stage that has a gate is refused, naming the fix.
        let err = call(
            "PUT",
            "/projects/shop/config/cadence",
            serde_json::json!({"release_stages": ["candidate", "production"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("kanbanr config release-stage main --clear"),
            "{err}"
        );

        let prep = call(
            "PUT",
            "/projects/shop/config/release-prepare",
            serde_json::json!({
                "version_files": [{"path": "api/Cargo.toml"}, {"path": "api/Cargo.lock", "match": "name = \"x\"\nversion = \"{version}\""}],
                "changelog": "CHANGELOG.md",
                "notes": "docs/src/releases/{version}.md",
            }),
        )
        .unwrap();
        assert_eq!(
            prep["version_files"][1]["match"],
            "name = \"x\"\nversion = \"{version}\""
        );
        let prep = call(
            "PUT",
            "/projects/shop/config/release-prepare",
            serde_json::json!({"changelog": ""}),
        )
        .unwrap();
        assert!(prep.get("changelog").is_none(), "{prep}");
        assert_eq!(prep["notes"], "docs/src/releases/{version}.md");
        assert!(
            call(
                "PUT",
                "/projects/shop/config/release-prepare",
                serde_json::json!({"notes": "../out.md"}),
            )
            .is_err()
        );

        // Clearing goes back to the default.
        let stages = call(
            "PUT",
            "/projects/shop/config/release-stages/production",
            serde_json::json!({"clear": true}),
        )
        .unwrap();
        assert_eq!(stages[2]["declared"], false);
        assert_eq!(
            stages[2]["on_enter"],
            serde_json::json!(["tag"]),
            "main still moves at main"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-212 R-15: a cadence write that does not name the gates or the preparation keeps them.
    #[test]
    fn release_gates_survive_cadence_put() {
        let (store, dir) = release_board("gates-survive");
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/release-stages/main",
            Some(&serde_json::json!({"on_enter": ["move_main"]})),
        )
        .unwrap();
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/release-prepare",
            Some(&serde_json::json!({"changelog": "CHANGELOG.md"})),
        )
        .unwrap();
        let before = store.load_meta("shop").unwrap().config.cadence;
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": true, "main_branch": "trunk"})),
        )
        .unwrap();
        let after = store.load_meta("shop").unwrap().config.cadence;
        assert_eq!(after.release_gates, before.release_gates);
        assert_eq!(after.release_prepare, before.release_prepare);
        assert_eq!(after.main_branch.as_deref(), Some("trunk"));
        assert!(after.sprints);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-200 R-1 and R-2: with strict off (the default), a spec or markdown doc whose diagram
    /// fails the lint is written, and the response names each finding as `diagram_warnings` —
    /// file, block, file line, rule and message; a clean write carries no such key.
    #[test]
    fn write_reports_diagram_warnings() {
        let dir = std::env::temp_dir().join(format!("kanbanr-dispatch-dw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            serde_json::from_str(&dispatch(&store, method, path, Some(&body)).unwrap()).unwrap()
        };
        let spec = "# Spec\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```mermaid\n\
                    flowchart LR\n  A[f(x)] --> end\n```\n";

        let added = call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M", "specification": spec }),
        );
        assert_eq!(added["code"], "FEAT-001", "the item is written");
        let w = added["diagram_warnings"].as_array().expect("warnings");
        assert_eq!(w.len(), 1, "one finding per statement: {w:?}");
        assert_eq!(w[0]["file"], "FEAT-001 spec");
        assert_eq!(w[0]["block"], 2);
        assert_eq!(w[0]["line"], 10);
        assert_eq!(w[0]["rule"], "label_punctuation");
        assert!(w[0]["message"].as_str().unwrap().contains("quotes"));
        assert_eq!(store.feature_spec("demo", "FEAT-001").unwrap(), spec);

        let edited = call(
            "PATCH",
            "/projects/demo/features/FEAT-001",
            json!({ "specification": "```mermaid\nsequenceDiagram\n  A->>end: hi\n```\n" }),
        );
        assert_eq!(edited["diagram_warnings"][0]["rule"], "end_as_id");
        assert_eq!(edited["diagram_warnings"][0]["line"], 3);
        let titled = call(
            "PATCH",
            "/projects/demo/features/FEAT-001",
            json!({ "title": "Only the title" }),
        );
        assert!(titled.get("diagram_warnings").is_none(), "no spec, no lint");

        let doc = call(
            "PUT",
            "/projects/demo/docs/content",
            json!({ "path": "arch/flow.md", "content": "```mermaid\nflowchrt\n```\n" }),
        );
        assert_eq!(doc["path"], "arch/flow.md");
        assert_eq!(doc["diagram_warnings"][0]["file"], "docs/arch/flow.md");
        assert_eq!(doc["diagram_warnings"][0]["rule"], "unknown_type");
        assert!(
            store.read_doc("demo", "arch/flow.md").is_ok(),
            "the doc is written"
        );
        let clean = call(
            "PUT",
            "/projects/demo/docs/content",
            json!({ "path": "arch/ok.md", "content": "```mermaid\npie\n```\n" }),
        );
        assert!(clean.get("diagram_warnings").is_none());

        let batch = call(
            "POST",
            "/projects/demo/batch",
            json!({ "operations": [
                { "op": "feature.add", "ref": "x", "title": "B", "milestone": "M",
                  "spec": "```mermaid\n```\n" },
                { "op": "feature.edit", "code": "x", "spec": "```mermaid\nflowchart LR\n  A\n```\n" },
                { "op": "doc.write", "path": "b.md", "content": "```mermaid\nsequenceDiagram\n  junk\n```\n" },
            ]}),
        );
        let r = batch["results"].as_array().unwrap();
        assert_eq!(r[0]["diagram_warnings"][0]["rule"], "empty_block");
        assert_eq!(r[0]["diagram_warnings"][0]["file"], "FEAT-002 spec");
        assert!(r[1].get("diagram_warnings").is_none());
        assert_eq!(r[2]["diagram_warnings"][0]["rule"], "unknown_statement");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-214 R-10: the scopes route lists each scope of a project with its derived state —
    /// issued, active, expired, revoked — and the issue, revoke and renew routes change it.
    #[test]
    fn scopes_route_lists_state() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-scopes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let board = dir.join("board");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&board).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        crate::coderepo::git(&repo, &["init", "-q", "-b", "main"]).unwrap();
        let store = Store::new(board.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            let out = dispatch_in(&store, method, path, Some(&body), Some(&repo)).unwrap();
            serde_json::from_str(&out).unwrap_or(Value::String(out))
        };
        call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M" }),
        );
        let refused = dispatch_in(
            &store,
            "POST",
            "/projects/demo/scopes",
            Some(&json!({ "role": "design", "item": "FEAT-001", "by": "Ada" })),
            Some(&repo),
        )
        .unwrap_err();
        assert!(
            refused.to_string().contains("kanbanr git install-hooks"),
            "{refused}"
        );

        let issue = |note: &str| {
            call(
                "POST",
                "/projects/demo/scopes",
                json!({ "role": "design", "item": "FEAT-001", "by": "Ada", "note": note,
                        "hooks_ready": true }),
            )
        };
        let first = issue("one");
        assert_eq!(first["scope"]["id"], "SC-001");
        assert_eq!(first["scope"]["state"], "issued");
        assert_eq!(first["scope"]["issued_by"], "Ada");
        assert!(
            first["brief"]
                .as_str()
                .unwrap()
                .contains("kanbanr scope claim SC-001")
        );
        assert_eq!(
            commit_message_after(
                "POST",
                "/projects/demo/scopes",
                Some(&json!({"role": "design", "item": "FEAT-001"})),
                &first.to_string()
            ),
            "issue agent scope SC-001 (design, FEAT-001)"
        );
        issue("two");
        issue("three");
        call(
            "POST",
            "/projects/demo/scopes/SC-002/revoke",
            json!({ "by": "Ada", "reason": "done" }),
        );
        // SC-003 ran out: its record says so, nothing rewrote it.
        let path = store
            .project_dir("demo")
            .join(crate::agentscope::SCOPES_FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut scopes: Vec<crate::agentscope::Scope> = crate::yaml::from_str(&raw).unwrap();
        scopes[2].expires_at = "2020-01-01T00:00:00Z".into();
        std::fs::write(&path, crate::yaml::to_string(&scopes).unwrap()).unwrap();

        let listed = call("GET", "/projects/demo/scopes", Value::Null);
        let states: Vec<(&str, &str)> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].as_str().unwrap(), s["state"].as_str().unwrap()))
            .collect();
        assert_eq!(
            states,
            [
                ("SC-001", "issued"),
                ("SC-002", "revoked"),
                ("SC-003", "expired")
            ]
        );
        assert_eq!(listed[1]["revoked_reason"], "done");
        assert_eq!(listed[0]["project"], "demo");

        let renewed = call(
            "POST",
            "/projects/demo/scopes/SC-003/renew",
            json!({ "hours": 4 }),
        );
        assert_eq!(renewed["state"], "issued", "renewed, it is in force again");
        let too_long = dispatch_in(
            &store,
            "POST",
            "/projects/demo/scopes/SC-003/renew",
            Some(&json!({ "hours": 100 })),
            Some(&repo),
        );
        assert!(too_long.is_err(), "72 hours at most");
        assert_eq!(
            call("GET", "/projects/demo/scopes/SC-001", Value::Null)["note"],
            "one"
        );
        let brief = call("GET", "/projects/demo/scopes/SC-001/brief", Value::Null);
        assert!(brief.as_str().unwrap().starts_with("## Your boundaries"));
        assert_eq!(
            read_media_type("/projects/demo/scopes/SC-001/brief"),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            commit_message(
                "POST",
                "/projects/demo/scopes/SC-002/revoke",
                Some(&json!({"reason": "done"}))
            ),
            "revoke agent scope SC-002: done"
        );
        // The Workflow page's guard rail: the roles, and the scopes in force across items.
        let g = call("GET", "/projects/demo/guards", Value::Null);
        assert_eq!(g["agents"]["roles"][0]["role"], "design");
        assert_eq!(g["agents"]["roles"][0]["operations"], json!([]));
        assert_eq!(
            g["agents"]["roles"][1]["operations"],
            json!(["task.state", "test.state", "commit"])
        );
        let live: Vec<&str> = g["agents"]["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(live, ["SC-001", "SC-003"], "issued and active only");
        assert!(g["agents"]["push_hook_installed"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-213 R-1: one read reports the method's parts, the commit rules, diagram strictness,
    /// each coverage rule with what it leaves uncovered, and the board's push policy.
    #[test]
    fn guards_route_reports_every_guard_rail() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-guards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            serde_json::from_str(&dispatch(&store, method, path, Some(&body)).unwrap()).unwrap()
        };
        call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "sprints": true, "releases": true, "git_model": true }),
        );
        call(
            "POST",
            "/projects/demo/sprints",
            json!({ "start": "2026-10-19", "length_days": 14 }),
        );
        call(
            "POST",
            "/projects/demo/releases",
            json!({ "version": "v1.0.0" }),
        );
        call(
            "PUT",
            "/projects/demo/config/diagrams",
            json!({ "strict": true }),
        );
        call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M" }),
        );
        call(
            "PUT",
            "/projects/demo/coverage",
            json!({ "text": "- id: listed\n  every: item\n  lists: architecture/index.md\n" }),
        );

        let g = call("GET", "/projects/demo/guards", Value::Null);

        let parts: Vec<&str> = g["method"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["part"].as_str().unwrap())
            .collect();
        assert_eq!(
            parts,
            ["design", "design-review", "docs", "docs-folded", "links"],
            "{g}"
        );
        assert!(
            g["method"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["on"].is_boolean())
        );

        let commits = &g["commits"];
        assert_eq!(commits["item_branch"], true);
        assert_eq!(commits["trailer"], "Refs");
        assert_eq!(commits["branch_pattern"], "feat/{code}-{slug}");
        assert!(commits["escape"].as_str().unwrap().contains("[no-ref]"));
        let protected: Vec<&str> = commits["protected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap())
            .collect();
        for branch in ["main", "sprint/*", "release/*"] {
            assert!(
                protected.contains(&branch),
                "{branch} is protected: {protected:?}"
            );
        }
        assert!(
            commits["hooks_installed"].is_null(),
            "core cannot tell whether the CLI's hooks are installed"
        );

        assert_eq!(g["diagrams"]["strict"], true);
        assert_eq!(g["diagrams"]["lint_on_write"], true);

        let coverage = g["coverage"].as_array().unwrap();
        assert_eq!(coverage.len(), 1, "{g}");
        assert_eq!(coverage[0]["id"], "listed");
        assert_eq!(coverage[0]["subjects"], 1, "{g}");
        assert_eq!(coverage[0]["uncovered"], 1, "the one item lists no doc");

        let (policy, _) = crate::git::PushPolicy::for_board(&dir);
        assert_eq!(g["push_policy"]["policy"], policy.to_string());
        assert!(
            ["environment", "board", "default"]
                .contains(&g["push_policy"]["source"].as_str().unwrap())
        );
        assert_eq!(g["code_pushes"], "you");

        // With the git model off, only the main branch refuses commits.
        call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "git_model": false }),
        );
        let off = call("GET", "/projects/demo/guards", Value::Null);
        assert_eq!(off["commits"]["protected"], json!(["main"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
