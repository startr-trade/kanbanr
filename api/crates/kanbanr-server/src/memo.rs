//! The read memo (DEF-081): every GET under `/api` is answered once per board revision and kept in
//! memory, keyed by its canonical path and query and stamped with what it was computed from. A
//! request whose stamp matches is answered from memory, a browser that already holds it gets a 304,
//! and a change to the board (a watcher event, a commit, a new day, a new code HEAD for the reads
//! that look at the code repository) changes the stamp, so a stale body is never served.
//!
//! Everything here is derived and in memory, and nothing is ever written (ADR-0005).

use crate::AppState;
use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kanbanr_core::Store;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The memo's byte cap: every page of this board, plus a second project, without eviction.
pub const MEMO_CAP_BYTES: usize = 64 * 1024 * 1024;
/// The largest body that is buffered and kept; a larger one is streamed through.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// A read that uses the code repository is restamped this often, so an uncommitted edit there shows
/// within half a minute (DEF-081 R-13).
pub const CODE_BUCKET_SECS: u64 = 30;

/// Board generations: a counter per project, bumped on every change to it, and a global one for
/// changes to every project (`*`). A root route (no project) is stamped by `of(None)`, which moves
/// on every bump, because the root reads (the project list, the portfolio) aggregate every project.
#[derive(Default)]
pub struct Generations {
    per: Mutex<HashMap<String, u64>>,
    /// Bumped by `bump_all`: part of every project's generation.
    global: AtomicU64,
    /// Bumped by every `bump` and `bump_all`: the root routes' generation.
    any: AtomicU64,
}

impl Generations {
    /// A change to one project. Returns that project's new generation.
    pub fn bump(&self, project: &str) -> u64 {
        self.any.fetch_add(1, Ordering::SeqCst);
        let per = {
            let mut map = self.per.lock().unwrap_or_else(|e| e.into_inner());
            let n = map.entry(project.to_string()).or_insert(0);
            *n += 1;
            *n
        };
        self.global.load(Ordering::SeqCst).wrapping_add(per)
    }

    /// A change to every project (`*`).
    pub fn bump_all(&self) {
        self.global.fetch_add(1, Ordering::SeqCst);
        self.any.fetch_add(1, Ordering::SeqCst);
    }

    /// The generation of a project's reads, or of the root reads for `None`.
    pub fn of(&self, project: Option<&str>) -> u64 {
        match project {
            None => self.any.load(Ordering::SeqCst),
            Some(p) => {
                let per = self
                    .per
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(p)
                    .copied()
                    .unwrap_or(0);
                self.global.load(Ordering::SeqCst).wrapping_add(per)
            }
        }
    }
}

/// What a kept body was computed from. Two equal stamps mean the same answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub r#gen: u64,
    pub board_head: String,
    pub code_head: String,
    pub bucket: u64,
    pub day: String,
}

/// A kept body.
#[derive(Clone, Debug)]
pub struct Cached {
    pub body: Bytes,
    pub media: &'static str,
    pub etag: String,
}

/// The LRU behind the memo: one entry per key, in use order (least recent at the front).
#[derive(Default)]
struct Lru {
    order: VecDeque<String>,
    entries: HashMap<String, (Stamp, Cached)>,
    total: usize,
}

impl Lru {
    fn touch(&mut self, key: &str) {
        if let Some(i) = self.order.iter().position(|k| k == key)
            && let Some(k) = self.order.remove(i)
        {
            self.order.push_back(k);
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some((_, old)) = self.entries.remove(key) {
            self.total -= old.body.len();
        }
        if let Some(i) = self.order.iter().position(|k| k == key) {
            self.order.remove(i);
        }
    }
}

/// The read memo: kept bodies, the boot id that makes this process's ETags its own, and one lock per
/// key being computed, so concurrent requests at one stamp compute once.
pub struct Memo {
    lru: Mutex<Lru>,
    cap_bytes: usize,
    boot: u64,
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Memo {
    /// An empty memo holding at most `cap_bytes` of bodies. The boot id comes from the operating
    /// system, so a restarted daemon's ETags differ from the last one's.
    pub fn new(cap_bytes: usize) -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("the operating system has no random bytes to give");
        Memo {
            lru: Mutex::new(Lru::default()),
            cap_bytes,
            boot: u64::from_le_bytes(bytes),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// The weak ETag of a stamp in this process: `W/"<16 hex>"`, FNV-1a 64 over the boot id and the
    /// stamp's fields joined by NUL. Weak, because gzip changes the bytes and not the meaning.
    pub fn etag(&self, stamp: &Stamp) -> String {
        let joined = format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            self.boot, stamp.r#gen, stamp.board_head, stamp.code_head, stamp.bucket, stamp.day
        );
        format!("W/\"{}\"", kanbanr_core::hash::stable_hash(&joined))
    }

    /// The body kept for `key`, if it was computed at `stamp`.
    pub fn get(&self, key: &str, stamp: &Stamp) -> Option<Cached> {
        let mut lru = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        let hit = match lru.entries.get(key) {
            Some((kept, cached)) if kept == stamp => Some(cached.clone()),
            _ => None,
        };
        if hit.is_some() {
            lru.touch(key);
        }
        hit
    }

    /// Keep `body` for `key` at `stamp`, replacing what the key held. Evicts the least recently used
    /// until it fits; a body larger than a quarter of the cap is not kept.
    pub fn put(&self, key: &str, stamp: &Stamp, body: Bytes, media: &'static str) {
        let len = body.len();
        let mut lru = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        lru.remove(key);
        if len > self.cap_bytes / 4 {
            return;
        }
        while lru.total + len > self.cap_bytes {
            let Some(front) = lru.order.pop_front() else {
                break;
            };
            if let Some((_, old)) = lru.entries.remove(&front) {
                lru.total -= old.body.len();
            }
        }
        let cached = Cached {
            body,
            media,
            etag: self.etag(stamp),
        };
        lru.total += len;
        lru.order.push_back(key.to_string());
        lru.entries.insert(key.to_string(), (stamp.clone(), cached));
    }

    /// The lock for computing `key`: held across the computation, so a second request for the same
    /// key waits and then finds the first one's body. Locks nobody else holds are dropped here.
    pub async fn single_flight(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            map.retain(|k, l| k == key || Arc::strong_count(l) > 1);
            Arc::clone(map.entry(key.to_string()).or_default())
        };
        lock.lock_owned().await
    }
}

/// The memo key for a request: its path and query with the query's parameters sorted and any
/// `fetch` parameter dropped (a read never fetches, FEAT-212).
pub fn canonical(path_and_query: &str) -> String {
    let Some((path, query)) = path_and_query.split_once('?') else {
        return path_and_query.to_string();
    };
    let mut kept: Vec<&str> = query
        .split('&')
        .filter(|kv| !kv.is_empty() && kv.split('=').next() != Some("fetch"))
        .collect();
    kept.sort_unstable();
    if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// The stamp from its parts. `code_head` is `Some` only for a read that uses the code repository; it
/// then also carries the 30 s bucket of `unix_secs`. A board-only read ignores both.
pub fn stamp_at(
    generation: u64,
    board_head: &str,
    code_head: Option<&str>,
    unix_secs: u64,
    day: &str,
) -> Stamp {
    let (code_head, bucket) = match code_head {
        Some(head) => (head.to_string(), unix_secs / CODE_BUCKET_SECS),
        None => (String::new(), 0),
    };
    Stamp {
        r#gen: generation,
        board_head: board_head.to_string(),
        code_head,
        bucket,
        day: day.to_string(),
    }
}

/// The stamp for a canonical key now: the project's generation (the root generation for a route
/// with no project), the board HEAD of `store_for(st, project)`, and for the reads
/// `reads_code_repo` names the code repository's HEAD and the time bucket; the UTC day.
pub fn stamp_for(st: &AppState, key: &str) -> Stamp {
    let project = crate::routes::project_id(key);
    let generation = st.gens.of(project.as_deref());
    let store = store_for(st, project.as_deref().unwrap_or(""));
    let board_head = kanbanr_core::git::head_commit(store.data_dir()).unwrap_or_default();
    let code_head = crate::routes::reads_code_repo(key).then(|| {
        crate::routes::repo_for(st, key)
            .and_then(kanbanr_core::git::head_commit)
            .unwrap_or_default()
    });
    let unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now = kanbanr_core::now_rfc3339();
    let day = now.get(..10).unwrap_or(&now);
    stamp_at(
        generation,
        &board_head,
        code_head.as_deref(),
        unix_secs,
        day,
    )
}

/// The store holding a project's board. Today every project is on the one board; FTR-040 makes it
/// per board (X11).
pub(crate) fn store_for<'a>(st: &'a AppState, project: &str) -> &'a Store {
    let _ = project;
    &st.store
}

/// The event streams, which are never memoized.
fn is_event_stream(path: &str) -> bool {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segs.as_slice(), ["events"] | ["projects", _, "events"])
}

/// The latest background drift result (FTR-045), in all its query forms, which is never memoised:
/// a finished run changes it without moving any part of the stamp, and the route only reads the
/// in-memory result, so it is cheap and needs no cache (coordinator, 17 Nov 2026).
fn is_drift_latest(path: &str) -> bool {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segs.as_slice(), ["projects", _, "drift", "latest"])
}

/// A response's media type as a `&'static str`. The server answers with a handful of media types,
/// so each distinct one is kept once for the life of the process.
fn static_media(media: &str) -> &'static str {
    static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(known) = seen.get(media) {
        return known;
    }
    let leaked: &'static str = Box::leak(media.to_string().into_boxed_str());
    seen.insert(leaked);
    leaked
}

/// `ETag` and `Cache-Control: no-cache` on a response.
fn with_cache_headers(mut resp: Response, etag: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(etag) {
        resp.headers_mut().insert(ETAG, value);
    }
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

/// A kept body as a 200.
fn from_cached(cached: Cached) -> Response {
    let resp = ([(CONTENT_TYPE, cached.media)], cached.body).into_response();
    with_cache_headers(resp, &cached.etag)
}

/// Whether an `If-None-Match` header names `etag`.
fn none_match(req: &Request, etag: &str) -> bool {
    req.headers()
        .get_all(IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|tag| tag.trim() == etag)
}

/// The memo as axum middleware on the `/api` router. GET only, and never the event streams. The
/// stamp is taken first: an `If-None-Match` equal to its ETag is a 304; a kept body at that stamp is
/// served; otherwise the request takes the key's single-flight lock, looks again, runs the inner
/// service, and keeps a 200's body under the stamp taken BEFORE running, so a change that lands
/// during the computation is never hidden. Every 200 or 304 carries `ETag` and
/// `Cache-Control: no-cache`; anything else passes through untouched and is not kept.
pub async fn layer(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() != Method::GET
        || is_event_stream(req.uri().path())
        || is_drift_latest(req.uri().path())
    {
        return next.run(req).await;
    }
    let pq = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let key = canonical(pq);
    let stamp = stamp_for(&st, &key);
    let etag = st.memo.etag(&stamp);
    if none_match(&req, &etag) {
        let resp = StatusCode::NOT_MODIFIED.into_response();
        return with_cache_headers(resp, &etag);
    }
    if let Some(hit) = st.memo.get(&key, &stamp) {
        return from_cached(hit);
    }
    let _flight = st.memo.single_flight(&key).await;
    if let Some(hit) = st.memo.get(&key, &stamp) {
        return from_cached(hit);
    }
    let resp = next.run(req).await;
    if resp.status() != StatusCode::OK {
        return resp;
    }
    let media = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if media
        .as_deref()
        .is_some_and(|m| m.starts_with("text/event-stream"))
    {
        return resp;
    }
    // A body whose size is unknown or over the limit is streamed through, not kept.
    let fits = resp
        .body()
        .size_hint()
        .upper()
        .is_some_and(|n| n <= MAX_BODY_BYTES as u64);
    if !fits {
        return with_cache_headers(resp, &etag);
    }
    let (parts, body) = resp.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Some(m) = media.as_deref() {
        st.memo.put(&key, &stamp, bytes.clone(), static_media(m));
    }
    let resp = Response::from_parts(parts, Body::from(bytes));
    with_cache_headers(resp, &etag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn stamp(generation: u64) -> Stamp {
        stamp_at(generation, "board1", None, 0, "2026-11-17")
    }

    #[test]
    fn a_stamp_changes_with_a_board_event_a_commit_or_the_day() {
        let memo = Memo::new(MEMO_CAP_BYTES);
        let base = stamp_at(1, "board1", Some("code1"), 60, "2026-11-17");
        let same = stamp_at(1, "board1", Some("code1"), 60, "2026-11-17");
        assert_eq!(memo.etag(&base), memo.etag(&same));
        let etag = memo.etag(&base);
        assert!(etag.starts_with("W/\"") && etag.ends_with('"'), "{etag}");
        assert_eq!(etag.len(), "W/\"\"".len() + 16, "{etag}");

        let changed = [
            stamp_at(2, "board1", Some("code1"), 60, "2026-11-17"), // a board event
            stamp_at(1, "board2", Some("code1"), 60, "2026-11-17"), // a board commit
            stamp_at(1, "board1", Some("code2"), 60, "2026-11-17"), // a code commit
            stamp_at(1, "board1", Some("code1"), 90, "2026-11-17"), // another bucket
            stamp_at(1, "board1", Some("code1"), 60, "2026-11-18"), // the day
        ];
        for other in &changed {
            assert_ne!(memo.etag(&base), memo.etag(other), "{other:?}");
        }
        let restarted = Memo::new(MEMO_CAP_BYTES);
        assert_ne!(memo.etag(&base), restarted.etag(&base));
    }

    #[test]
    fn a_body_computed_before_a_change_is_never_served_after_it() {
        let gens = Generations::default();
        let memo = Memo::new(MEMO_CAP_BYTES);
        let before = stamp(gens.of(Some("p")));
        gens.bump("p");
        memo.put(
            "/projects/p/index",
            &before,
            Bytes::from_static(b"old"),
            "application/json",
        );
        let now = stamp(gens.of(Some("p")));
        assert_ne!(before, now);
        assert!(memo.get("/projects/p/index", &now).is_none());
        // The root reads move with any project's change, and every project with a `*` change.
        let root = gens.of(None);
        gens.bump("q");
        assert_ne!(root, gens.of(None));
        let p = gens.of(Some("p"));
        gens.bump_all();
        assert_ne!(p, gens.of(Some("p")));
    }

    #[test]
    fn the_memo_evicts_the_least_recently_used_beyond_its_byte_cap() {
        // A body over a quarter of the cap is not kept, so with a 100-byte cap each body is 25.
        let memo = Memo::new(100);
        let s = stamp(1);
        let body = || Bytes::from(vec![b'x'; 25]);
        for key in ["a", "b", "c", "d"] {
            memo.put(key, &s, body(), "application/json");
        }
        assert!(memo.get("a", &s).is_some());
        memo.put("e", &s, body(), "application/json");
        assert!(memo.get("b", &s).is_none(), "the least recently used goes");
        for key in ["a", "c", "d", "e"] {
            assert!(memo.get(key, &s).is_some(), "{key} stays");
        }
        // Too large to keep, but no error.
        memo.put("big", &s, Bytes::from(vec![b'x'; 26]), "application/json");
        assert!(memo.get("big", &s).is_none());
        assert!(memo.get("a", &s).is_some());
    }

    #[test]
    fn a_code_repository_read_is_restamped_by_its_head_and_every_thirty_seconds() {
        let t = 1_000 * CODE_BUCKET_SECS;
        let day = "2026-11-17";
        let at = |head: &str, secs: u64| stamp_at(1, "board1", Some(head), secs, day);
        assert_ne!(at("c1", t), at("c2", t), "a new code HEAD");
        assert_eq!(at("c1", t), at("c1", t + 29), "one bucket");
        assert_ne!(at("c1", t), at("c1", t + 30), "the next bucket");
        // A board-only read ignores both.
        assert_eq!(
            stamp_at(1, "board1", None, t, day),
            stamp_at(1, "board1", None, t + 300, day)
        );
        // Which reads are which is `reads_code_repo`'s.
        assert!(crate::routes::reads_code_repo("/projects/p/readiness"));
        assert!(!crate::routes::reads_code_repo("/projects/p/index"));
    }

    #[test]
    fn canonical_sorts_parameters_and_drops_fetch() {
        assert_eq!(
            canonical("/projects/x/features?status=Planned&milestone=MS-1"),
            "/projects/x/features?milestone=MS-1&status=Planned"
        );
        assert_eq!(
            canonical("/projects/x/releases/1/plan?fetch=1"),
            "/projects/x/releases/1/plan"
        );
        assert_eq!(canonical("/projects/x"), "/projects/x");
    }

    /// A scratch data dir under the system temp dir, never under the repository (L-41).
    fn scratch_state() -> (AppState, std::path::PathBuf) {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let dir = std::env::temp_dir().join(format!("kanbanr-memo-{}", u64::from_le_bytes(bytes)));
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        let st = AppState {
            store: Arc::new(Store::new(dir.clone())),
            data_dir: dir.clone(),
            tx,
            allow_writes: false,
            push: crate::PushPolicy::for_board(&dir).0,
            code_repo: None,
            gens: Arc::new(Generations::default()),
            memo: Arc::new(Memo::new(MEMO_CAP_BYTES)),
            slices: tokio::sync::broadcast::channel(4).0,
            background: Arc::new(OnceLock::new()),
        };
        (st, dir)
    }

    async fn get(addr: std::net::SocketAddr, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_requests_at_one_stamp_compute_once() {
        let (st, dir) = scratch_state();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let app = axum::Router::new()
            .route(
                "/projects/p/slow",
                axum::routing::get(move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        ([(CONTENT_TYPE, "application/json")], "{\"ok\":true}")
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(st.clone(), layer))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let (a, b) = tokio::join!(get(addr, "/projects/p/slow"), get(addr, "/projects/p/slow"));
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        for resp in [&a, &b] {
            assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
            assert!(resp.to_ascii_lowercase().contains("etag: w/\""), "{resp}");
            assert!(resp.contains("{\"ok\":true}"), "{resp}");
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn drift_latest_is_never_memoised() {
        let (st, dir) = scratch_state();
        let state = Arc::new(AtomicUsize::new(0));
        let current = Arc::clone(&state);
        let app = axum::Router::new()
            .route(
                "/projects/p/drift/latest",
                axum::routing::get(move || {
                    let current = Arc::clone(&current);
                    async move {
                        let state = if current.load(Ordering::SeqCst) == 0 {
                            "computing"
                        } else {
                            "fresh"
                        };
                        (
                            [(CONTENT_TYPE, "application/json")],
                            format!("{{\"state\":\"{state}\"}}"),
                        )
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(st.clone(), layer))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let first = get(addr, "/projects/p/drift/latest").await;
        let first_summary = get(addr, "/projects/p/drift/latest?flags=false").await;
        // The background run finishes: no board change, so the stamp is the same.
        state.store(1, Ordering::SeqCst);
        let second = get(addr, "/projects/p/drift/latest").await;
        let second_summary = get(addr, "/projects/p/drift/latest?flags=false").await;
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first.contains("\"state\":\"computing\""), "{first}");
        assert!(
            first_summary.contains("\"state\":\"computing\""),
            "{first_summary}"
        );
        assert!(second.contains("\"state\":\"fresh\""), "{second}");
        assert!(
            second_summary.contains("\"state\":\"fresh\""),
            "{second_summary}"
        );
        assert!(!second.to_ascii_lowercase().contains("etag:"), "{second}");
    }
}
