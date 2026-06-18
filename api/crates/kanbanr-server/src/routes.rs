//! Read-only HTTP handlers for the view daemon. The server never mutates data — writes happen in
//! the CLI's local mode. Reads delegate to `kanbanr_core::dispatch` (one source of truth) or call
//! the store/activity log directly.

use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures::stream::StreamExt;
use kanbanr_core::{dispatch, export};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::Stream;

/// Map a CoreError to an HTTP status + message.
fn core_err(e: kanbanr_core::CoreError) -> (StatusCode, String) {
    use kanbanr_core::CoreError::*;
    let code = match &e {
        ProjectNotFound(_)
        | FeatureNotFound(_)
        | MilestoneNotFound(_)
        | TaskNotFound(_, _)
        | TodoListNotFound(_, _)
        | DocNotFound(_) => StatusCode::NOT_FOUND,
        ProjectExists(_)
        | FeatureExists(_)
        | MilestoneExists(_)
        | TaskExists(_, _)
        | TodoListExists(_, _)
        | MilestoneInUse(_, _)
        | StatusInUse(_, _)
        | ProjectNotEmpty(_, _, _) => StatusCode::CONFLICT,
        InvalidDocPath(_)
        | InvalidName(_)
        | UnknownStatus(_)
        | TransitionNotAllowed { .. }
        | InvalidTaskState(_)
        | DependencyCycle(_)
        | UnknownDependency(_)
        | MilestoneRequired
        | NoStatuses
        | DisplayedNoOp(_)
        | BatchOpFailed(_, _)
        | InvalidMermaid(_)
        | Unsupported(_) => StatusCode::BAD_REQUEST,
        Io(_) | Yaml(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, e.to_string())
}

/// Return a dispatcher result as a JSON response.
fn json_body(r: kanbanr_core::Result<String>) -> axum::response::Response {
    match r {
        Ok(s) => ([("content-type", "application/json")], s).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

pub async fn list_projects(State(st): State<AppState>) -> impl IntoResponse {
    json_body(dispatch::dispatch(&st.store, "GET", "/projects", None))
}

/// Portfolio index (workspace + programs). (FEAT-030)
pub async fn portfolio(State(st): State<AppState>) -> impl IntoResponse {
    json_body(dispatch::dispatch(&st.store, "GET", "/portfolio", None))
}

/// Cross-project task-based rollups. (FEAT-030)
pub async fn portfolio_rollups(State(st): State<AppState>) -> impl IntoResponse {
    json_body(dispatch::dispatch(
        &st.store,
        "GET",
        "/portfolio/rollups",
        None,
    ))
}

/// Cross-project board (normalized lanes). (FEAT-030)
pub async fn portfolio_board(State(st): State<AppState>) -> impl IntoResponse {
    json_body(dispatch::dispatch(
        &st.store,
        "GET",
        "/portfolio/board",
        None,
    ))
}

pub async fn get_project(State(st): State<AppState>, Path(p): Path<String>) -> impl IntoResponse {
    json_body(dispatch::dispatch(
        &st.store,
        "GET",
        &format!("/projects/{p}"),
        None,
    ))
}

#[derive(serde::Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    format: Option<String>,
}

pub async fn export_feature(
    State(st): State<AppState>,
    Path((project, code)): Path<(String, String)>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    let project = match st.store.load(&project) {
        Ok(p) => p,
        Err(e) => return core_err(e).into_response(),
    };
    let feature = match project.feature(&code) {
        Ok(f) => f,
        Err(e) => return core_err(e).into_response(),
    };
    let ms = project.milestone(&feature.milestone).ok();
    match q.format.as_deref() {
        Some("json") => match export::to_json(feature, ms) {
            Ok(s) => ([("content-type", "application/json")], s).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        _ => (
            [("content-type", "text/markdown; charset=utf-8")],
            export::to_markdown(feature, ms),
        )
            .into_response(),
    }
}

pub async fn list_docs(State(st): State<AppState>, Path(p): Path<String>) -> impl IntoResponse {
    match st.store.doc_tree(&p) {
        Ok(tree) => Json(tree).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct DocQuery {
    path: String,
}

pub async fn get_doc(
    State(st): State<AppState>,
    Path(p): Path<String>,
    Query(q): Query<DocQuery>,
) -> impl IntoResponse {
    match st.store.read_doc(&p, &q.path) {
        Ok(content) => {
            ([("content-type", "text/markdown; charset=utf-8")], content).into_response()
        }
        Err(e) => core_err(e).into_response(),
    }
}

fn content_type_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "md" | "markdown" | "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serve a documentation file's RAW bytes with a content-type (binary assets like images).
pub async fn get_doc_raw(
    State(st): State<AppState>,
    Path(p): Path<String>,
    Query(q): Query<DocQuery>,
) -> impl IntoResponse {
    match st.store.read_doc_bytes(&p, &q.path) {
        Ok(bytes) => ([("content-type", content_type_for(&q.path))], bytes).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ActivityQuery {
    /// Filter to a single work item (feature code).
    #[serde(rename = "ref")]
    item: Option<String>,
    /// Maintenance stream: only changes to items currently in a non-displayed (ongoing) status.
    #[serde(default)]
    ongoing: bool,
    #[serde(default)]
    limit: Option<usize>,
}

/// Recent activity for a project (from its changelog file), newest first. Optionally filtered to a
/// single item (`?ref=`) or to ongoing/maintenance items (`?ongoing=true`).
pub async fn project_activity(
    State(st): State<AppState>,
    Path(p): Path<String>,
    Query(q): Query<ActivityQuery>,
) -> impl IntoResponse {
    let mut entries = kanbanr_core::activity::read(&st.data_dir, &p, 200);
    if let Some(item) = &q.item {
        entries.retain(|e| e.item.as_deref() == Some(item.as_str()));
    }
    if q.ongoing {
        match st.store.load(&p) {
            Ok(project) => {
                let displayed: std::collections::HashSet<&String> =
                    if project.config.displayed_states.is_empty() {
                        project.config.statuses.iter().collect()
                    } else {
                        project.config.displayed_states.iter().collect()
                    };
                // Feature codes whose current status is NOT displayed (ongoing/maintenance).
                let ongoing_codes: std::collections::HashSet<String> = project
                    .features
                    .iter()
                    .filter(|f| !displayed.contains(&f.status))
                    .map(|f| f.code.clone())
                    .collect();
                entries.retain(|e| {
                    e.item
                        .as_ref()
                        .map(|c| ongoing_codes.contains(c))
                        .unwrap_or(false)
                });
            }
            Err(_) => entries.clear(),
        }
    }
    entries.truncate(q.limit.unwrap_or(25));
    Json(entries)
}

pub async fn project_events(
    State(st): State<AppState>,
    Path(project): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = st.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let project = project.clone();
        async move {
            match msg {
                Ok(changed) if changed == project || changed == "*" => {
                    Some(Ok(Event::default().event("changed").data(changed)))
                }
                _ => None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn all_events(
    State(st): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = st.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
        match msg {
            Ok(changed) => Some(Ok(Event::default().event("changed").data(changed))),
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---- write routes (single-writer daemon, FEAT-034; mounted only when --allow-writes) ----------

use axum::http::Method as HttpMethod;

/// The project id from a dispatch path `/projects/<id>/...` (for SSE notification).
fn project_id(path: &str) -> Option<String> {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// Run a daemon write through the shared dispatch+commit+push path, then notify SSE listeners.
/// `dispatch_path` is the reconstructed real route (e.g. `/projects/X/features`).
fn run_write(
    st: &AppState,
    method: &HttpMethod,
    dispatch_path: &str,
    body: Option<&serde_json::Value>,
) -> axum::response::Response {
    let outcome = crate::write::write(
        &st.store,
        &st.data_dir,
        &st.pending,
        st.push,
        method.as_str(),
        dispatch_path,
        body,
    );
    match outcome {
        Ok(out) => {
            for w in &out.warnings {
                eprintln!("kanbanr: {w}");
            }
            // Tell the live monitor which project changed (or "*" for structural changes).
            let _ = st
                .tx
                .send(project_id(dispatch_path).unwrap_or_else(|| "*".to_string()));
            ([("content-type", "application/json")], out.body).into_response()
        }
        // The dispatch error text mirrors the read path; map structurally to a 400/conflict bucket.
        Err(e) => {
            let code = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else if e.contains("already exists") || e.contains("in use") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (code, e).into_response()
        }
    }
}

/// Catch-all write handler for `/write/<rest>` — reconstructs `/{rest}` as the dispatch path.
pub async fn write_route(
    State(st): State<AppState>,
    method: HttpMethod,
    Path(rest): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let dispatch_path = format!("/{rest}");
    run_write(&st, &method, &dispatch_path, body.as_ref().map(|b| &b.0))
}

/// Write handler for the bare `/write` route (maps to dispatch path `/projects`, e.g. create
/// project / portfolio root operations).
pub async fn write_route_root(
    State(st): State<AppState>,
    method: HttpMethod,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    // The only bare mutation is "create project" (POST /projects); portfolio ops carry a sub-path.
    run_write(&st, &method, "/projects", body.as_ref().map(|b| &b.0))
}

/// Explicit push of pending local commits (the daemon's `/sync`).
pub async fn sync_now(State(st): State<AppState>) -> impl IntoResponse {
    let warnings = crate::write::sync(&st.data_dir, &st.pending);
    for w in &warnings {
        eprintln!("kanbanr: {w}");
    }
    Json(serde_json::json!({ "synced": true, "warnings": warnings }))
}
