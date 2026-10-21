//! Read-only HTTP handlers for the view daemon. The server never mutates data — writes happen in
//! the CLI's local mode. Reads delegate to `kanbanr_core::dispatch` (one source of truth) or call
//! the store/activity log directly.

use crate::AppState;
use axum::Json;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::StreamExt;
use kanbanr_core::dispatch;
use std::convert::Infallible;
use tokio_stream::Stream;
use tokio_stream::wrappers::BroadcastStream;

/// Map a CoreError to an HTTP status + message. The mapping is core's (FEAT-176), so a read and
/// a write that fail the same way answer with the same code.
fn core_err(e: kanbanr_core::CoreError) -> (StatusCode, String) {
    let code = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, e.to_string())
}

/// The published description of the public read API (FEAT-177), as committed: generated from the
/// response types by `cargo run -p kanbanr-core --features schema --example openapi`, and checked
/// against them by the contract test, so these bytes are what the code describes.
pub const OPENAPI_JSON: &str = include_str!("../../../../docs/src/reference/openapi.json");

/// `GET /api/openapi.json`.
pub async fn openapi() -> impl IntoResponse {
    ([("content-type", "application/json")], OPENAPI_JSON)
}

/// Return a dispatcher result as a JSON response.
fn json_body(r: kanbanr_core::Result<String>) -> axum::response::Response {
    match r {
        Ok(s) => ([("content-type", "application/json")], s).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

/// Generic GET passthrough for any core read route not explicitly wired above — `workflow`, `gantt`,
/// `ready`/`blocked`/`graph`/`impact`, `critical-path`, `query`, `doctor`, etc. This keeps the read
/// daemon's surface in lockstep with `kanbanr_core::dispatch` (one source of truth) without
/// per-route wiring, so a new read route in core is automatically served. Mounted as the `/api`
/// router fallback; non-GET methods are rejected (writes only exist behind `--allow-writes`).
pub async fn read_passthrough(
    State(st): State<AppState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> axum::response::Response {
    if method != Method::GET {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            "this daemon route is read-only",
        )
            .into_response();
    }
    // OriginalUri keeps the `/api` prefix + query string; dispatch wants the path without `/api`.
    let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let dpath = pq.strip_prefix("/api").unwrap_or(pq);
    // Labelled with what the route renders (FEAT-177): a diagram or a document is not JSON.
    match dispatch::dispatch(&st.store, "GET", dpath, None) {
        Ok(body) => ([("content-type", dispatch::read_media_type(dpath))], body).into_response(),
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

/// `GET /projects/:project/features/:code/export` — through core's dispatch, as the CLI exports
/// (FEAT-176): one rendering of an item, labelled with what it rendered (Markdown, or JSON for
/// `?format=json`).
pub async fn export_feature(
    st: State<AppState>,
    method: Method,
    uri: OriginalUri,
) -> axum::response::Response {
    read_passthrough(st, method, uri).await
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

/// Serve a documentation file's RAW bytes with a content-type (binary assets like images).
pub async fn get_doc_raw(
    State(st): State<AppState>,
    Path(p): Path<String>,
    Query(q): Query<DocQuery>,
) -> impl IntoResponse {
    match st.store.read_doc_bytes(&p, &q.path) {
        Ok(bytes) => (
            [("content-type", kanbanr_core::docs::content_type(&q.path))],
            bytes,
        )
            .into_response(),
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
    let filter = kanbanr_core::activity::Filter {
        item: q.item,
        ongoing: q.ongoing,
        limit: q.limit,
    };
    Json(kanbanr_core::activity::read_filtered(
        &st.store, &p, &filter,
    ))
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
    // The code repository the daemon was started in, for its own project only (FEAT-183): any
    // other project's promotions that need one are refused, never run against the wrong code.
    let repo = st
        .code_repo
        .as_ref()
        .filter(|(project, _)| project_id(dispatch_path).as_deref() == Some(project.as_str()))
        .map(|(_, root)| root.as_path());
    let outcome = crate::write::write(
        &st.store,
        &st.data_dir,
        st.push,
        method.as_str(),
        dispatch_path,
        body,
        repo,
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
        // The same status a read failing the same way answers with (FEAT-176): core's mapping, not
        // a guess from the words of the message.
        Err(e) => {
            let code = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST);
            (code, e.to_string()).into_response()
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
    let outcome = crate::write::sync(&st.data_dir);
    for w in &outcome.failures {
        eprintln!("kanbanr: {w}");
    }
    Json(serde_json::json!({
        "synced": outcome.failures.is_empty(),
        "pushed": outcome.pushed,
        "warnings": outcome.failures,
    }))
}
