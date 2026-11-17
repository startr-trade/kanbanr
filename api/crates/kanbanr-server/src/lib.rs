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

pub mod background;
mod embedded;
pub mod memo;
mod origin;
mod routes;
pub mod watcher;
mod write;

pub use write::PushPolicy;

use axum::Router;
use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use kanbanr_core::Store;
use kanbanr_core::boards::Boards;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::sync::broadcast;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

/// Hashed asset files never change under one name, so a browser keeps them for a year (DEF-081).
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `Cache-Control` for a served asset: only a success is kept, so a file asked for mid-rebuild
/// (a 404) is asked for again rather than remembered as missing for a year.
fn immutable_on_success(res: &Response) -> Option<axum::http::HeaderValue> {
    res.status()
        .is_success()
        .then(|| axum::http::HeaderValue::from_static(IMMUTABLE))
}

#[derive(Clone)]
pub struct AppState {
    /// The boards this daemon shows, in order (FTR-040): a data folder served the usual way is a
    /// list of one. Every read of a project goes to the board that holds it (`memo::store_for`).
    pub boards: Arc<Boards>,
    /// The first board's store: the one writes and `/sync` act on, which only exist with a single
    /// data folder (FTR-040).
    pub store: Arc<Store>,
    /// The first board's folder (also a git repo); writes and `/sync` act on it.
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
    /// Board generations, bumped on every change, that stamp the memo's entries (DEF-081).
    pub gens: Arc<memo::Generations>,
    /// The read memo: GET bodies kept per stamp, and the ETags they answer with (DEF-081).
    pub memo: Arc<memo::Memo>,
    /// Broadcast of what changed on a board, by slice tag: the `slices` SSE event (DEF-081).
    pub slices: broadcast::Sender<watcher::Slices>,
    /// The background worker (FTR-045): drift computed after each change, and warm summary reads.
    /// Set once the state is built, since the worker needs the state.
    pub background: Arc<OnceLock<Arc<background::Background>>>,
    /// Where katalogr is asked for a doc's query blocks (FTR-016): the environment's settings,
    /// with `--katalogr-url` over the URL. Nothing here is board config (ADR-0007).
    pub katalogr: kanbanr_core::katalogr::Settings,
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
    run_with(data_dir, bind, ui_dir, allow_writes, code_repo, false, None).await
}

/// `run`, with `remote_writes`: the person passed `--allow-remote-writes`. With writes on, a bind
/// other machines can reach is refused before anything starts unless that flag or
/// `KANBANR_ALLOW_REMOTE_WRITES` accepts it (DEF-026). `katalogr_url` is `--katalogr-url`: where
/// the daemon asks katalogr, over `KANBANR_KATALOGR_URL` (FTR-016).
pub async fn run_with(
    data_dir: PathBuf,
    bind: String,
    ui_dir: Option<String>,
    allow_writes: bool,
    code_repo: Option<(String, PathBuf)>,
    remote_writes: bool,
    katalogr_url: Option<String>,
) -> anyhow::Result<()> {
    let checked = check_settings(&bind, allow_writes, remote_writes)?;
    std::fs::create_dir_all(data_dir.join("projects"))?;
    let katalogr = katalogr_settings(katalogr_url.as_deref());
    serve(
        Boards::single(data_dir),
        bind,
        ui_dir,
        allow_writes,
        code_repo,
        remote_writes,
        checked,
        false,
        katalogr,
    )
    .await
}

/// The environment's katalogr settings, with the daemon's `--katalogr-url` over the URL.
fn katalogr_settings(url: Option<&str>) -> kanbanr_core::katalogr::Settings {
    let settings = kanbanr_core::katalogr::Settings::from_env();
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => settings.with_url(url),
        None => settings,
    }
}

/// Run one read-only daemon over the boards at `paths`, in order (FTR-040, `kanbanr serve --board
/// …`): their projects served together, each read from its own board. A list one server cannot
/// show is refused before anything binds, and nothing is written to any board.
pub async fn run_boards(
    paths: Vec<PathBuf>,
    bind: String,
    ui_dir: Option<String>,
) -> anyhow::Result<()> {
    let boards = Boards::open(&paths)?;
    let checked = check_settings(&bind, false, false)?;
    let katalogr = katalogr_settings(None);
    serve(
        boards, bind, ui_dir, false, None, false, checked, true, katalogr,
    )
    .await
}

/// The settings read from the environment and checked before anything starts.
struct Checked {
    /// Writes on a reachable address were accepted (flag or environment).
    accepted: bool,
    origins: Vec<String>,
}

/// Settings are read once, here, and checked where read (factor 3); every refusal comes before
/// the monitor binds, watches or writes anything.
fn check_settings(bind: &str, allow_writes: bool, remote_writes: bool) -> anyhow::Result<Checked> {
    let from_env =
        origin::accepted_from_env(std::env::var("KANBANR_ALLOW_REMOTE_WRITES").ok().as_deref())
            .map_err(anyhow::Error::msg)?;
    let accepted = remote_writes || from_env;
    if let Some(text) =
        origin::bind_refusal(bind, allow_writes, accepted).map_err(anyhow::Error::msg)?
    {
        anyhow::bail!(text);
    }
    let origins =
        origin::parse_origins(&std::env::var("KANBANR_ALLOWED_ORIGINS").unwrap_or_default())
            .map_err(anyhow::Error::msg)?;
    Ok(Checked { accepted, origins })
}

/// The daemon over `boards`: the body `run_with` had before FTR-040. `listed` is true when the
/// boards came from `--board` (`run_boards`), which answers `/api/meta` for the list and names the
/// boards in the start line; a single data folder answers as it always did.
#[allow(clippy::too_many_arguments)]
async fn serve(
    boards: Boards,
    bind: String,
    ui_dir: Option<String>,
    allow_writes: bool,
    code_repo: Option<(String, PathBuf)>,
    remote_writes: bool,
    checked: Checked,
    listed: bool,
    katalogr: kanbanr_core::katalogr::Settings,
) -> anyhow::Result<()> {
    let Checked { accepted, origins } = checked;
    let boards = Arc::new(boards);
    let data_dir = boards.first().dir.clone();
    let store = Arc::clone(&boards.first().store);
    let (tx, _rx) = broadcast::channel::<String>(256);
    let (slices, _slices_rx) = broadcast::channel::<watcher::Slices>(256);
    let meta_repo: Option<PathBuf> = code_repo.as_ref().map(|(_, p)| p.clone());
    let state = AppState {
        boards: Arc::clone(&boards),
        store,
        data_dir: data_dir.clone(),
        tx: tx.clone(),
        allow_writes,
        push: PushPolicy::for_board(&data_dir).0,
        code_repo,
        gens: Arc::new(memo::Generations::default()),
        memo: Arc::new(memo::Memo::new(memo::MEMO_CAP_BYTES)),
        slices: slices.clone(),
        background: Arc::new(OnceLock::new()),
        katalogr,
    };
    // The background worker warms every project the served boards hold (DEF-109). The pair below
    // feeds drift only: the project whose code repository the daemon was started in (FTR-045).
    // Without one, no drift runs and the route answers `unavailable`. A list of boards has no code
    // repository (FTR-040), so it gives no (project, repo) pair.
    let repos: Vec<(String, PathBuf)> = state.code_repo.iter().cloned().collect();
    let _ = state.background.set(background::Background::start(
        state.clone(),
        Arc::new(background::CoreRunner),
        repos,
    ));

    // Watch each board's folder and push SSE "changed" and "slices" events as the CLI edits files;
    // each change bumps the project's generation, so the memo misses at once (DEF-081). One
    // watcher per board, all on one channel: project ids are unique across boards (FTR-040).
    let mut _watchers = Vec::with_capacity(boards.all().len());
    for board in boards.all() {
        _watchers.push(watcher::spawn(
            board.dir.join("projects"),
            tx.clone(),
            slices.clone(),
            Arc::clone(&state.gens),
        )?);
    }

    let mut api = Router::new()
        .route("/portfolio", get(routes::portfolio))
        .route("/portfolio/rollups", get(routes::portfolio_rollups))
        .route("/portfolio/board", get(routes::portfolio_board))
        .route("/projects", get(routes::list_projects))
        .route("/boards", get(routes::list_boards))
        .route("/projects/:project", get(routes::get_project))
        .route(
            "/projects/:project/features/:code/export",
            get(routes::export_feature),
        )
        .route("/projects/:project/docs", get(routes::list_docs))
        .route("/projects/:project/docs/content", get(routes::get_doc))
        .route("/projects/:project/docs/raw", get(routes::get_doc_raw))
        .route(
            "/projects/:project/repo-docs/raw",
            get(routes::get_repo_doc_raw),
        )
        .route("/projects/:project/activity", get(routes::project_activity))
        .route("/projects/:project/events", get(routes::project_events))
        .route("/projects/:project/drift/latest", get(routes::drift_latest))
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
    // The read memo wraps every GET route, the fallback included, so it goes on after it (DEF-081).
    let api = api
        .fallback(routes::read_passthrough)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            memo::layer,
        ))
        .with_state(state);

    // Bound before the guard is built and the start line printed, so `--bind 127.0.0.1:0` (an
    // ephemeral port, as a walkthrough's scratch monitor asks for; FTR-037) answers for the port it
    // got and names it, not the `:0` it was asked for.
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let bound = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| bind.clone());
    // Who this monitor answers, and the token its own page is given at this start (DEF-024).
    // Read once, here (factor 3).
    let allowed: Vec<String> = std::env::var("KANBANR_ALLOWED_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .collect();
    let guard = Arc::new(origin::Guard::new(&bound, &allowed).with_origins(origins));
    let token = guard.token().to_string();

    let meta_dir = data_dir.clone();
    let meta_boards = Arc::clone(&boards);
    let mut app = Router::new()
        .route("/healthz", get(healthz))
        // What this daemon can do (FEAT-067). The monitor asks before offering an action: a button
        // that fails on click is worse than no button, and the answer is not guessable from the
        // read routes.
        .route(
            "/api/meta",
            get(move || async move {
                // A list of boards (FTR-040) answers for the list: no writes and no identity.
                if listed {
                    return axum::Json(kanbanr_core::server::Meta::for_boards(&meta_boards));
                }
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
                    meta_repo.as_deref(),
                ))
            }),
        )
        .nest("/api", api)
        .layer(axum::middleware::from_fn(log_requests));

    if let Some(dir) = ui_dir {
        // Serve hashed assets and any other file the build put in the dir (favicon, config.json —
        // FEAT-178), and fall back to index.html for any other non-/api path so client-side routes
        // work on direct load / refresh.
        let base = PathBuf::from(&dir);
        let token = token.clone();
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
            .merge(
                Router::new()
                    .nest_service("/assets", ServeDir::new(base.join("assets")))
                    .layer(SetResponseHeaderLayer::overriding(
                        axum::http::header::CACHE_CONTROL,
                        immutable_on_success,
                    )),
            )
            .fallback(move |uri: axum::http::Uri| {
                let path = index_path.clone();
                let last_good = Arc::clone(&last_good);
                let token = token.clone();
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
                    axum::response::Html(with_root_base(&with_token(&html, &token))).into_response()
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
            .map(|html| {
                with_root_base(&with_token(&String::from_utf8_lossy(html), &token)).into_bytes()
            })
            .unwrap_or_else(|| {
                with_token(
                    "<!doctype html><html><head><title>kanbanr</title></head></html>",
                    &token,
                )
                .into_bytes()
            });
        app = app.fallback(move |uri: axum::http::Uri| {
            let index = index.clone();
            async move {
                let path = uri.path().trim_start_matches('/');
                match files.get(path) {
                    // A hashed asset, the favicon or config.json: served as itself. The index is
                    // always the copy carrying its base, however it is asked for.
                    Some(body) if path != "index.html" => {
                        let mut res =
                            ([(CONTENT_TYPE, embedded::content_type(path))], body.clone())
                                .into_response();
                        // Hashed names under assets/ are immutable (DEF-081); the favicon and
                        // config.json keep the default, since their names do not change.
                        if path.starts_with("assets/") {
                            res.headers_mut().insert(
                                axum::http::header::CACHE_CONTROL,
                                axum::http::HeaderValue::from_static(IMMUTABLE),
                            );
                        }
                        res
                    }
                    // Anything else is a client-side route, so the app decides what it means.
                    _ => ([(CONTENT_TYPE, "text/html; charset=utf-8")], index).into_response(),
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

    // Gzip sits just inside the guard, so it covers the API and the monitor's files alike; its
    // default predicate already skips `text/event-stream`, images and bodies under 32 bytes
    // (DEF-081: the whole-board read is 4 MB raw and about 0.6 MB gzipped).
    let app = app.layer(CompressionLayer::new().gzip(true));
    // The guard goes on last, so it covers the page and its files as well as the API: a layer only
    // wraps the routes that exist when it is added (DEF-024).
    let app = app.layer(axum::middleware::from_fn_with_state(
        guard.clone(),
        origin::check,
    ));

    // Keep the watchers alive for the lifetime of the server.
    let _keep = _watchers;
    if allow_writes && accepted && !origin::bind_is_loopback(&bind).map_err(anyhow::Error::msg)? {
        let how = if remote_writes {
            "--allow-remote-writes"
        } else {
            "KANBANR_ALLOW_REMOTE_WRITES"
        };
        eprintln!(
            "kanbanr: WARNING writes are ON at {bind}, an address other machines can reach: anyone who can open the page can record decisions. You accepted this with {how}."
        );
    }
    let accepted_from = if allow_writes {
        let mut from: Vec<String> = guard
            .hosts()
            .iter()
            .map(|h| format!("http://{h}"))
            .collect();
        from.extend(guard.origins().iter().cloned());
        format!("\nwrites accepted from: {}", from.join(", "))
    } else {
        String::new()
    };
    let serving = if listed {
        let each: Vec<String> = boards
            .all()
            .iter()
            .map(|b| {
                let n = b.store.list_projects().map(|p| p.len()).unwrap_or(0);
                format!("{} [{n} projects]", b.name)
            })
            .collect();
        format!("read-only; boards: {}", each.join(", "))
    } else {
        format!("data: {}", data_dir.display())
    };
    eprintln!(
        "kanbanr view daemon on http://{bound}  ({serving})\nanswering for: {}{accepted_from}",
        guard.hosts().join(", ")
    );
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

/// The monitor's index with the token its own page sends with a write, as the first thing after
/// `<head>` (DEF-024). An index with no head is left as it is.
fn with_token(html: &str, token: &str) -> String {
    let lower = html.to_ascii_lowercase();
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
    format!(
        "{}<meta name=\"kanbanr-token\" content=\"{token}\">{}",
        &html[..end],
        &html[end..]
    )
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
