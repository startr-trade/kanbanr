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

mod routes;
mod watcher;
mod write;

pub use write::PushPolicy;

use axum::Router;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use kanbanr_core::Store;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
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
    /// Debounced-push policy and the in-process unpushed-commit tally (FEAT-034).
    pub push: PushPolicy,
    pub pending: Arc<AtomicU32>,
}

/// Run the view daemon until the process is stopped. `bind` like `127.0.0.1:8080`; `ui_dir` is the
/// built SPA directory (optional — when absent only `/api` is served). When `allow_writes` is true,
/// write routes are also mounted (opt-in single-writer daemon, FEAT-034); otherwise read-only.
pub async fn run(
    data_dir: PathBuf,
    bind: String,
    ui_dir: Option<String>,
    allow_writes: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir.join("projects"))?;
    let store = Arc::new(Store::new(data_dir.clone()));
    let (tx, _rx) = broadcast::channel::<String>(256);
    let state = AppState {
        store,
        data_dir: data_dir.clone(),
        tx: tx.clone(),
        allow_writes,
        push: PushPolicy::from_env(),
        pending: Arc::new(AtomicU32::new(0)),
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
        .route("/events", get(routes::all_events));

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

    let mut app = Router::new()
        .route("/healthz", get(healthz))
        // What this daemon can do (FEAT-067). The monitor asks before offering an action: a button
        // that fails on click is worse than no button, and the answer is not guessable from the
        // read routes.
        .route(
            "/api/meta",
            get(move || async move {
                axum::Json(serde_json::json!({
                    "writes": allow_writes,
                    // The schema this build understands (FEAT-072). A monitor that receives an
                    // empty board can compare it with the board's own version and say "upgrade"
                    // rather than leaving the reader to conclude their data is gone.
                    "schema_version": kanbanr_core::config::CURRENT_SCHEMA_VERSION,
                }))
            }),
        )
        .nest("/api", api)
        .layer(axum::middleware::from_fn(log_requests))
        .layer(CorsLayer::permissive());

    if let Some(dir) = ui_dir {
        // Serve hashed assets, and fall back to index.html for any other non-/api path so
        // client-side routes work on direct load / refresh.
        let base = PathBuf::from(&dir);
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
            .fallback(move || {
                let path = index_path.clone();
                let last_good = Arc::clone(&last_good);
                async move {
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
                    axum::response::Html(html)
                }
            });
        eprintln!("serving SPA from {dir}");
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

/// Liveness/readiness probe.
async fn healthz() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "status": "ok" }))
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
