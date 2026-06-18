//! kanbanr view daemon — a small, localhost-first HTTP server that **reads** the data folder and
//! serves the read API + live SSE + the SPA. It never writes: all edits happen in the CLI's local
//! mode. There is no auth and no accounts — the data is your local folder, sharing is via git
//! remotes, and exposing the view beyond localhost is a job for a reverse proxy. This is a library
//! so the single `kanbanr` binary can run it via `kanbanr serve`.

mod routes;
mod watcher;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use kanbanr_core::Store;
use std::path::PathBuf;
use std::sync::Arc;
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
}

/// Run the view daemon until the process is stopped. `bind` like `127.0.0.1:8080`; `ui_dir` is the
/// built SPA directory (optional — when absent only `/api` is served).
pub async fn run(data_dir: PathBuf, bind: String, ui_dir: Option<String>) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir.join("projects"))?;
    let store = Arc::new(Store::new(data_dir.clone()));
    let (tx, _rx) = broadcast::channel::<String>(256);
    let state = AppState {
        store,
        data_dir: data_dir.clone(),
        tx: tx.clone(),
    };

    // Watch the data folder and push SSE "changed" events as the CLI edits files.
    let _watcher = watcher::spawn(data_dir.join("projects"), tx.clone())?;

    let api = Router::new()
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
        .with_state(state);

    let mut app = Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", api)
        .layer(axum::middleware::from_fn(log_requests))
        .layer(CorsLayer::permissive());

    if let Some(dir) = ui_dir {
        // Serve hashed assets, and fall back to index.html for any other non-/api path so
        // client-side routes work on direct load / refresh.
        let base = PathBuf::from(&dir);
        let index_html = std::fs::read_to_string(base.join("index.html")).unwrap_or_default();
        app = app
            .nest_service("/assets", ServeDir::new(base.join("assets")))
            .fallback(move || {
                let html = index_html.clone();
                async move { axum::response::Html(html) }
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
