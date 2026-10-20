//! kanbanr view daemon — a small, localhost-first HTTP server over the data folder. By default it
//! **reads only** (the live web monitor: read API + SSE + SPA), and all edits happen in the CLI's
//! local mode. There is no auth and no accounts — the data is your local folder, sharing is via git
//! remotes, and exposing the view beyond localhost is a job for a reverse proxy. This is a library
//! so the single `kanbanr` binary can run it via `kanbanr serve`.
//!
//! ## Optional single-writer daemon (FEAT-034, `serve --allow-writes`)
//! When `allow_writes` is set, the daemon ALSO exposes the dispatch write routes (POST/PUT/PATCH/
//! DELETE) and serializes every mutation through this one process. It reuses the exact CLI write
//! recipe (`write.rs` → shared `kanbanr_core::dispatch` + commit + debounced push) and still takes
//! the same cross-process advisory write lock, so it coexists with a CLI writing the same folder
//! directly. This is purely additive and OFF by default: the read-only monitor is unchanged, and
//! the CLI-writes-directly default is fully preserved.

mod embedded;
mod routes;
mod watcher;
mod write;

pub use write::PushPolicy;

use axum::Router;
use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use kanbanr_core::Store;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    /// The data directory (also a git repo); used for the activity changelog.
    pub data_dir: PathBuf,
    /// Broadcast of changed project ids ("*" = structural/projects-list change).
    pub tx: broadcast::Sender<String>,
    /// Whether mutation routes are mounted (single-writer daemon). False = read-only monitor.
    pub allow_writes: bool,
    /// The board's push policy (FEAT-034, FEAT-142): its own setting, or `KANBANR_PUSH`.
    pub push: PushPolicy,
    /// The code repository the daemon was started in, and the project its marker names
    /// (FEAT-183). Release promotion acts on it for that project only.
    pub code_repo: Option<(String, PathBuf)>,
}

/// Run the view daemon until the process is stopped. `bind` like `127.0.0.1:8080`; `ui_dir` is the
/// built SPA directory (optional — when absent only `/api` is served). When `allow_writes` is true,
/// write routes are also mounted (opt-in single-writer daemon, FEAT-034); otherwise read-only.
/// `code_repo` is the code repository it was started in and the project that folder's marker
/// names, when there is one (FEAT-183).
pub async fn run(
    data_dir: PathBuf,
    bind: String,
    ui_dir: Option<String>,
    allow_writes: bool,
    code_repo: Option<(String, PathBuf)>,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir.join("projects"))?;
    let store = Arc::new(Store::new(data_dir.clone()));
    let (tx, _rx) = broadcast::channel::<String>(256);
    let state = AppState {
        store,
        data_dir: data_dir.clone(),
        tx: tx.clone(),
        allow_writes,
        push: PushPolicy::for_board(&data_dir).0,
        code_repo,
    };

    // Watch the data folder and push SSE "changed" events as the CLI edits files.
    let _watcher = watcher::spawn(data_dir.join("projects"), tx.clone())?;

    let mut api = Router::new()
        .route("/portfolio", get(routes::portfolio))
        .route("/portfolio/rollups", get(routes::portfolio_rollups))
        .route("/portfolio/board", get(routes::portfolio_board))
        .route("/projects", get(routes::list_projects))
        .route("/projects/:project", get(routes::get_project))
        .route(
            "/projects/:project/features/:code/export",
            get(routes::export_feature),
        )
        .route("/projects/:project/docs", get(routes::list_docs))
        .route("/projects/:project/docs/content", get(routes::get_doc))
        .route("/projects/:project/docs/raw", get(routes::get_doc_raw))
        .route("/projects/:project/activity", get(routes::project_activity))
        .route("/projects/:project/events", get(routes::project_events))
        .route("/events", get(routes::all_events))
        .route("/openapi.json", get(routes::openapi));

    if allow_writes {
        // Single-writer daemon (FEAT-034): a dedicated write prefix carries any mutation to the
        // shared dispatch write path, plus an explicit /sync. Using a `/write/*` prefix (rather
        // than overlaying methods on the typed read routes) keeps the router free of `:param` vs
        // `*wildcard` conflicts and keeps the write surface obvious. The handler reconstructs the
        // real dispatch path (`/write/projects/X` -> `/projects/X`) so dispatch is unchanged.
        api = api
            .route("/sync", post(routes::sync_now))
            .route(
                "/write/*rest",
                post(routes::write_route)
                    .put(routes::write_route)
                    .patch(routes::write_route)
                    .delete(routes::write_route),
            )
            .route("/write", post(routes::write_route_root));
        eprintln!("kanbanr: write routes ENABLED (single-writer daemon)");
    }

    // Any GET read route supported by core `dispatch` but not explicitly wired above (workflow,
    // gantt, ready/blocked/graph/impact, critical-path, query, doctor, …) is served by this
    // fallback, keeping the daemon's read surface in lockstep with the CLI. (Without it such paths
    // fell through to the SPA index.html — which broke the web Gantt page.)
    let api = api.fallback(routes::read_passthrough).with_state(state);

    let meta_dir = data_dir.clone();
    let mut app = Router::new()
        .route("/healthz", get(healthz))
        // What this daemon can do (FEAT-067). The monitor asks before offering an action: a button
        // that fails on click is worse than no button, and the answer is not guessable from the
        // read routes.
        .route(
            "/api/meta",
            get(move || async move {
                // `identity` is who an approval recorded here would be attributed to (FEAT-077).
                // The CLI defaults `--by` to this same identity; the monitor was instead recording
                // the string "reviewed in the monitor", which names the place rather than the
                // person — twenty-eight times on this project's own board before anyone noticed.
                // An approval that cannot say who agreed is not evidence of agreement, so when it
                // is null the monitor refuses to offer the action. `schema_version` lets a monitor
                // that receives an empty board say "upgrade" rather than leave the reader to
                // conclude their data is gone (FEAT-072).
                axum::Json(kanbanr_core::server::Meta::for_board(
                    &meta_dir,
                    allow_writes,
                ))
            }),
        )
        .nest("/api", api)
        .layer(axum::middleware::from_fn(log_requests))
        .layer(CorsLayer::permissive());

    if let Some(dir) = ui_dir {
        // Serve hashed assets and any other file the build put in the dir (favicon, config.json —
        // FEAT-178), and fall back to index.html for any other non-/api path so client-side routes
        // work on direct load / refresh.
        let base = PathBuf::from(&dir);
        let files_root = base.clone();
        let index_path = base.join("index.html");
        // Read per request, not once at startup (FEAT-070). Vite writes hashed asset names on every
        // build, so a cached index points at a bundle that has been deleted — the app then fails in
        // a way that reads as a broken feature rather than as a stale process. The file is small and
        // the daemon already touches the disk for every asset, so correctness is the cheaper trade.
        // The last copy read is kept only as a fallback for a request that lands mid-rebuild.
        let last_good: Arc<Mutex<String>> = Arc::new(Mutex::new(
            std::fs::read_to_string(&index_path).unwrap_or_default(),
        ));
        app = app
            .nest_service("/assets", ServeDir::new(base.join("assets")))
            .fallback(move |uri: axum::http::Uri| {
                let path = index_path.clone();
                let last_good = Arc::clone(&last_good);
                let file = ui_file(&files_root, uri.path());
                async move {
                    if let Some((rel, body)) = file {
                        return ([(CONTENT_TYPE, embedded::content_type(&rel))], body)
                            .into_response();
                    }
                    let html = match std::fs::read_to_string(&path) {
                        Ok(fresh) if !fresh.trim().is_empty() => {
                            if let Ok(mut cached) = last_good.lock() {
                                *cached = fresh.clone();
                            }
                            fresh
                        }
                        // Missing or half-written: serve what worked last rather than a blank page.
                        _ => last_good.lock().map(|c| c.clone()).unwrap_or_default(),
                    };
                    axum::response::Html(with_root_base(&html)).into_response()
                }
            });
        eprintln!("serving SPA from {dir}");
    } else if !embedded::is_empty() {
        // The monitor baked into this binary (FEAT-084). An explicit --ui-dir still wins above, so
        // developing against a live `npm run build` is unchanged; this is the path a downloaded
        // binary takes, where there is no dist folder to point at and never was.
        let files = embedded::assets();
        let index = files
            .get("index.html")
            .map(|html| with_root_base(&String::from_utf8_lossy(html)).into_bytes())
            .unwrap_or_else(|| b"<!doctype html><title>kanbanr</title>".to_vec());
        app = app.fallback(move |uri: axum::http::Uri| {
            let index = index.clone();
            async move {
                let path = uri.path().trim_start_matches('/');
                match files.get(path) {
                    // A hashed asset, the favicon or config.json: served as itself. The index is
                    // always the copy carrying its base, however it is asked for.
                    Some(body) if path != "index.html" => {
                        ([(CONTENT_TYPE, embedded::content_type(path))], body.clone())
                    }
                    // Anything else is a client-side route, so the app decides what it means.
                    _ => ([(CONTENT_TYPE, "text/html; charset=utf-8")], index),
                }
            }
        });
        eprintln!("serving the built-in monitor ({} files)", files.len());
    } else {
        // Saying nothing here would be this project's own recurring failure: an absence reported as
        // silence (FEAT-084 R-3). A binary built without the monitor serves a working API, and the
        // only symptom is a blank page.
        eprintln!(
            "note: this build has no built-in monitor, so only /api is served.\n      \
             Point at a built SPA with `--ui-dir <dir>` (build it with `npm --prefix web run build`),\n      \
             or use a release binary, which has it baked in."
        );
    }

    // Keep the watcher alive for the lifetime of the server.
    let _keep = _watcher;
    eprintln!(
        "kanbanr view daemon on http://{bind}  (data: {})",
        data_dir.display()
    );
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// The monitor's index with `<base href="/">` as the first thing in its head (FEAT-178).
///
/// The monitor is built with relative asset paths (`./assets/…`), so the same build works from any
/// folder of a static host. Served by the daemon, a deep link such as `/p/x/feature/Y` would then
/// resolve them against `/p/x/feature/` and load nothing. A root base makes them resolve from `/`,
/// which is where the daemon serves them. An index that already names a base, or has no head, is
/// left as it is.
fn with_root_base(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if lower.contains("<base") {
        return html.to_string();
    }
    let Some(start) = lower.find("<head").filter(|&i| {
        matches!(
            lower.as_bytes().get(i + 5),
            Some(b'>' | b' ' | b'\t' | b'\n' | b'\r')
        )
    }) else {
        return html.to_string();
    };
    let Some(end) = lower[start..].find('>').map(|i| start + i + 1) else {
        return html.to_string();
    };
    format!("{}<base href=\"/\">{}", &html[..end], &html[end..])
}

/// A file under `--ui-dir` named by a request path, if one exists there: the favicon, the runtime
/// `config.json` (FEAT-178), anything the build emits beside `index.html`. Only plain path segments
/// are followed, so nothing outside the dir can be named. `index.html` itself is not returned: it is
/// always served through the index path, which gives it its base.
fn ui_file(root: &std::path::Path, path: &str) -> Option<(String, Vec<u8>)> {
    use std::path::Component;
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel == "index.html" {
        return None;
    }
    let rel_path = std::path::Path::new(rel);
    if !rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let full = root.join(rel_path);
    if !full.is_file() {
        return None;
    }
    std::fs::read(&full)
        .ok()
        .map(|body| (rel.to_string(), body))
}

/// Liveness/readiness probe.
async fn healthz() -> impl IntoResponse {
    axum::Json(kanbanr_core::server::Health::ok())
}

/// One concise access-log line per request: method, path, status, latency.
async fn log_requests(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let started = Instant::now();
    let resp = next.run(req).await;
    eprintln!(
        "{method} {path} -> {} ({}ms)",
        resp.status().as_u16(),
        started.elapsed().as_millis()
    );
    resp
}
