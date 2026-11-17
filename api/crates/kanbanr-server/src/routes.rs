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
pub const OPENAPI_JSON: &str = include_str!("../../../../docs/guide/reference/openapi.json");

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
    // A read never reaches the network (FEAT-212): a release plan's `fetch` is the CLI's alone.
    let unfetched = without_fetch(dpath);
    let dpath = unfetched.as_str();
    // The reads that look at the code repository get the one the daemon was started in, as
    // release promotion does (FEAT-213): the git model, git doctor, the guard rails, a release's
    // plan (FEAT-212) and its notes (FEAT-216). Every other read answers from the board alone.
    let repo = reads_code_repo(dpath)
        .then(|| repo_for(&st, dpath))
        .flatten();
    // A project's read goes to the board that holds it; any other read to the board `?board=`
    // names, else the first (FTR-040).
    let (store, global_path) = match project_id(dpath) {
        Some(p) => (crate::memo::store_for(&st, &p), None),
        None => match global_store(&st, dpath) {
            Ok((store, path)) => (store, Some(path)),
            Err(name) => return no_board(&name),
        },
    };
    let dpath = global_path.as_deref().unwrap_or(dpath);
    // The katalogr reads ask the URL this daemon was given (`--katalogr-url`), not the
    // environment's (FTR-016).
    if let Some(answer) = dispatch::katalogr_read(store, dpath, repo, &st.katalogr) {
        return match answer {
            Ok(body) => ([("content-type", "application/json")], body).into_response(),
            Err(e) => core_err(e).into_response(),
        };
    }
    // Labelled with what the route renders (FEAT-177): a diagram or a document is not JSON.
    match dispatch::dispatch_in(store, "GET", dpath, None, repo) {
        Ok(body) => ([("content-type", dispatch::read_media_type(dpath))], body).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

/// `GET /api/projects`: every board's projects, board by board in order, each naming its board
/// (FTR-040). A project an earlier board already listed is skipped: it is served from there.
pub async fn list_projects(State(st): State<AppState>) -> axum::response::Response {
    let mut all: Vec<dispatch::ProjectSummary> = Vec::new();
    for board in st.boards.all() {
        let summaries = match dispatch::list_summaries_for(&board.store, Some(&board.name)) {
            Ok(s) => s,
            Err(e) => return core_err(e).into_response(),
        };
        for summary in summaries {
            if let Some(first) = all.iter().find(|p| p.id == summary.id) {
                let first = first.board.clone().unwrap_or_default();
                kanbanr_core::git::warn_once(
                    &format!("boards-shadowed:{}:{}", board.name, summary.id),
                    &format!(
                        "project '{}' is also on board '{}'; serving it from '{first}'",
                        summary.id, board.name
                    ),
                );
                continue;
            }
            all.push(summary);
        }
    }
    Json(all).into_response()
}

/// `GET /api/boards`: the boards this server shows, in order (FTR-040). Names no server path.
pub async fn list_boards(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.boards.info())
}

/// A `board=<name>` query parameter taken out of a dispatch path: the name, and the path without
/// it (FTR-040). Parameters are split on `&` and the key matched whole (L-23).
pub(crate) fn split_board(dispatch_path: &str) -> (Option<String>, String) {
    let Some((path, query)) = dispatch_path.split_once('?') else {
        return (None, dispatch_path.to_string());
    };
    let mut board = None;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let (key, value) = kv.split_once('=').unwrap_or((kv, ""));
            if key == "board" {
                board = Some(value.to_string());
                false
            } else {
                true
            }
        })
        .collect();
    let rest = if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    };
    (board, rest)
}

/// The store a read with no project answers from, and its path without `board=`: the board the
/// parameter names, else the first (FTR-040 R-11). An unknown name is the error, answered by
/// [`no_board`].
fn global_store<'a>(
    st: &'a AppState,
    dispatch_path: &str,
) -> Result<(&'a kanbanr_core::Store, String), String> {
    let (name, rest) = split_board(dispatch_path);
    match name {
        None => Ok((st.boards.first().store.as_ref(), rest)),
        Some(name) => match st.boards.by_name(&name) {
            Some(board) => Ok((board.store.as_ref(), rest)),
            None => Err(name),
        },
    }
}

/// `404 no board named '<name>'` (FTR-040 R-11).
fn no_board(name: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, format!("no board named '{name}'")).into_response()
}

/// A global read through core's dispatch, on the board its `?board=` names (FTR-040).
fn global_read(st: &AppState, uri: &axum::http::Uri, route: &str) -> axum::response::Response {
    let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or(route);
    let dpath = pq.strip_prefix("/api").unwrap_or(pq);
    match global_store(st, dpath) {
        Ok((store, _)) => json_body(dispatch::dispatch(store, "GET", route, None)),
        Err(name) => no_board(&name),
    }
}

/// Portfolio index (workspace + programs). (FEAT-030)
pub async fn portfolio(
    State(st): State<AppState>,
    OriginalUri(uri): OriginalUri,
) -> axum::response::Response {
    global_read(&st, &uri, "/portfolio")
}

/// Cross-project task-based rollups. (FEAT-030)
pub async fn portfolio_rollups(
    State(st): State<AppState>,
    OriginalUri(uri): OriginalUri,
) -> axum::response::Response {
    global_read(&st, &uri, "/portfolio/rollups")
}

/// Cross-project board (normalized lanes). (FEAT-030)
pub async fn portfolio_board(
    State(st): State<AppState>,
    OriginalUri(uri): OriginalUri,
) -> axum::response::Response {
    global_read(&st, &uri, "/portfolio/board")
}

pub async fn get_project(State(st): State<AppState>, Path(p): Path<String>) -> impl IntoResponse {
    json_body(dispatch::dispatch(
        crate::memo::store_for(&st, &p),
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
    match crate::memo::store_for(&st, &p).doc_tree(&p) {
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
    match crate::memo::store_for(&st, &p).read_doc(&p, &q.path) {
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
    match crate::memo::store_for(&st, &p).read_doc_bytes(&p, &q.path) {
        Ok(bytes) => (
            [("content-type", kanbanr_core::docs::content_type(&q.path))],
            bytes,
        )
            .into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

/// `GET /api/projects/:project/repo-docs/raw?path=&at=`: a code repository file's bytes at the
/// pick (FTR-004), as they are, labelled by the file's extension. Served apart from the passthrough
/// because a dispatch answer is text, and an image is not.
pub async fn get_repo_doc_raw(
    State(st): State<AppState>,
    Path(p): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> axum::response::Response {
    let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let dpath = pq.strip_prefix("/api").unwrap_or(pq);
    let query = uri.query();
    let repo = repo_for(&st, dpath);
    let store = crate::memo::store_for(&st, &p);
    match dispatch::repo_doc_raw(store, &p, query, repo) {
        Ok((bytes, mime)) => ([("content-type", mime)], bytes).into_response(),
        Err(e) => core_err(e).into_response(),
    }
}

/// `?flags=` on `/drift/latest` (FTR-045): include the flags; true when absent.
#[derive(serde::Deserialize)]
pub struct LatestQuery {
    flags: Option<bool>,
}

/// `GET /api/projects/:project/drift/latest[?flags=false]`: the latest background drift result,
/// from memory (FTR-045). It never computes drift; 404 for an unknown project.
pub async fn drift_latest(
    State(st): State<AppState>,
    Path(p): Path<String>,
    Query(q): Query<LatestQuery>,
) -> axum::response::Response {
    if let Err(e) = crate::memo::store_for(&st, &p).load_meta(&p) {
        return core_err(e).into_response();
    }
    match st.background.get() {
        Some(bg) => Json(bg.latest(&p, q.flags.unwrap_or(true))).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "the background worker has not started",
        )
            .into_response(),
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
        crate::memo::store_for(&st, &p),
        &p,
        &filter,
    ))
}

/// A `slices` SSE event: the JSON of what changed (DEF-081).
fn slices_event(s: &crate::watcher::Slices) -> Option<Result<Event, Infallible>> {
    serde_json::to_string(s)
        .ok()
        .map(|data| Ok(Event::default().event("slices").data(data)))
}

/// `GET /api/projects/:project/events`: `changed` with the project id, as in API v1, and
/// `slices` with what changed on this project's board (DEF-081).
pub async fn project_events(
    State(st): State<AppState>,
    Path(project): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let changed = {
        let project = project.clone();
        BroadcastStream::new(st.tx.subscribe()).filter_map(move |msg| {
            let project = project.clone();
            async move {
                match msg {
                    Ok(changed) if changed == project || changed == "*" => {
                        Some(Ok(Event::default().event("changed").data(changed)))
                    }
                    _ => None,
                }
            }
        })
    };
    let slices = BroadcastStream::new(st.slices.subscribe()).filter_map(move |msg| {
        let project = project.clone();
        async move {
            match msg {
                Ok(s) if s.project == project => slices_event(&s),
                _ => None,
            }
        }
    });
    Sse::new(futures::stream::select(changed, slices)).keep_alive(KeepAlive::default())
}

/// `GET /api/events`: `changed` for every project, and every project's `slices` (DEF-081).
pub async fn all_events(
    State(st): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let changed = BroadcastStream::new(st.tx.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(changed) => Some(Ok(Event::default().event("changed").data(changed))),
            Err(_) => None,
        }
    });
    let slices = BroadcastStream::new(st.slices.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(s) => slices_event(&s),
            Err(_) => None,
        }
    });
    Sse::new(futures::stream::select(changed, slices)).keep_alive(KeepAlive::default())
}

// ---- write routes (single-writer daemon, FEAT-034; mounted only when --allow-writes) ----------

use axum::http::Method as HttpMethod;

/// The project id from a dispatch path `/projects/<id>/...` (for SSE notification).
pub(crate) fn project_id(path: &str) -> Option<String> {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "projects" {
        return None;
    }
    segs.next().map(|s| s.to_string())
}

/// The code repository the daemon was started in, for its own project only (FEAT-183): any
/// other project's requests that need one answer as they do without one, never against the wrong
/// code.
pub(crate) fn repo_for<'a>(st: &'a AppState, dispatch_path: &str) -> Option<&'a std::path::Path> {
    st.code_repo
        .as_ref()
        .filter(|(project, _)| project_id(dispatch_path).as_deref() == Some(project.as_str()))
        .map(|(_, root)| root.as_path())
}

/// The reads that answer from the code repository (FEAT-213): `…/git/model`, `…/git/doctor`,
/// `…/guards`, `…/releases/<V>/plan` (FEAT-212) and `…/releases/<V>/notes` (FEAT-216), `…/releases/<V>/items`, whose docs-folded pill asks whether the branch moved on (FTR-041), and the readiness and review reads that judge the repository
/// docs of an item (DEF-033), the drift reads (FTR-013), and the items list, whose stale evidence
/// is judged against HEAD (FTR-048), the attention read, whose review groups and git doctor
/// read it (FTR-048), the Overview, which embeds attention and judges staleness against HEAD
/// (FTR-048), and the Review page's group decision, which judges a row's
/// blocks as the page does, and the page's group reads that judge them (FTR-038), query string
/// aside, the position picker and the repository doc reads (FTR-004), and the project's doctor,
/// which checks repository doc links at each item's position (FTR-004), and the two katalogr
/// reads, which ask at the checkout's live position (FTR-016). (A write gets the repository from
/// `run_write` either way; the decide route is listed so the set says which routes read it.)
pub(crate) fn reads_code_repo(dispatch_path: &str) -> bool {
    let path = dispatch_path.split('?').next().unwrap_or(dispatch_path);
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(
        segs.as_slice(),
        ["projects", _, "git", "model"]
            | ["projects", _, "git", "doctor"]
            | ["projects", _, "git", "tidy"]
            | ["projects", _, "guards"]
            | ["projects", _, "releases", _, "plan"]
            | ["projects", _, "releases", _, "notes"]
            | ["projects", _, "releases", _, "items"]
            | ["projects", _, "review"]
            | ["projects", _, "readiness"]
            | ["projects", _, "features", _, "readiness"]
            | ["projects", _, "drift"]
            | ["projects", _, "features", _, "drift"]
            | ["projects", _, "doctor"]
            | ["projects", _, "scopes", "drift"]
            | ["projects", _, "prepared"]
            | ["projects", _, "position"]
            | ["projects", _, "positions"]
            | ["projects", _, "positions", "resolve"]
            | ["projects", _, "repo-docs"]
            | ["projects", _, "repo-docs", "content"]
            | ["projects", _, "repo-docs", "raw"]
            | ["projects", _, "views", _]
            | ["projects", _, "docs", "views"]
            | ["projects", _, "katalogr"]
            | ["projects", _, "items"]
            | ["projects", _, "attention"]
            | ["projects", _, "overview"]
            | ["projects", _, "catalog"]
            | ["projects", _, "catalog", "baseline"]
            | ["projects", _, "tests", "reconcile"]
            | ["projects", _, "retro"]
            | ["projects", _, "review", "decide"]
            | ["projects", _, "review", "summary"]
            | ["projects", _, "review", "groups", ..]
    )
}

/// The path with any `fetch` query parameter dropped.
fn without_fetch(dispatch_path: &str) -> String {
    let Some((path, query)) = dispatch_path.split_once('?') else {
        return dispatch_path.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| kv.split('=').next() != Some("fetch"))
        .collect();
    if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// Run a daemon write through the shared dispatch+commit+push path, then notify SSE listeners.
/// `dispatch_path` is the reconstructed real route (e.g. `/projects/X/features`).
fn run_write(
    st: &AppState,
    method: &HttpMethod,
    dispatch_path: &str,
    body: Option<&serde_json::Value>,
) -> axum::response::Response {
    let repo = repo_for(st, dispatch_path);
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
            // The memo misses from here on: the generation is bumped before anyone is told
            // (DEF-081). Then tell the live monitor which project changed (or "*" for structural
            // changes), and, since the commit may touch several files, that everything in it did.
            match project_id(dispatch_path) {
                Some(project) => {
                    let r#gen = st.gens.bump(&project);
                    let _ = st.tx.send(project.clone());
                    let _ = st.slices.send(crate::watcher::Slices {
                        project,
                        r#gen,
                        tags: vec!["*".to_string()],
                    });
                }
                None => {
                    st.gens.bump_all();
                    let _ = st.tx.send("*".to_string());
                }
            }
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
