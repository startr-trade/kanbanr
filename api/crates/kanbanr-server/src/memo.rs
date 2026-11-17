//! The read memo (DEF-081): every GET under `/api` is answered once per board revision and kept in
//! memory, keyed by its canonical path and query and stamped with what it was computed from. A
//! request whose stamp matches is answered from memory, a browser that already holds it gets a 304,
//! and a change to the board (a watcher event, a commit, a new day, a delegation that lapsed, a
//! change to the code repository for the reads that look at it) changes the stamp, so a stale body is never served.
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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The memo's byte cap: every page of this board, plus a second project, without eviction.
pub const MEMO_CAP_BYTES: usize = 64 * 1024 * 1024;
/// The largest body that is buffered and kept; a larger one is streamed through.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// The code repository's fingerprint is looked at again this often, so an uncommitted edit there
/// shows within half a minute (DEF-081 R-13). Between looks a read is served from the memo, however
/// long it took to compute (DEF-106).
pub const CODE_RECHECK_SECS: u64 = 30;

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
    /// The code repository's fingerprint (HEAD, refs, working-tree status); empty for a board-only
    /// read or a board with no code repository.
    pub code: String,
    pub day: String,
    /// How many granted delegations had ended: a lapse mid-day changes it (DEF-106).
    pub ended: usize,
}

impl Stamp {
    /// This stamp with the number of granted delegations that have ended.
    pub fn with_ended(mut self, ended: usize) -> Self {
        self.ended = ended;
        self
    }
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
    /// Per code repository: its fingerprint and when it was taken.
    code_states: Mutex<HashMap<PathBuf, (Instant, String)>>,
    recheck: Duration,
}

impl Memo {
    /// An empty memo holding at most `cap_bytes` of bodies. The boot id comes from the operating
    /// system, so a restarted daemon's ETags differ from the last one's.
    pub fn new(cap_bytes: usize) -> Self {
        Self::with_recheck(cap_bytes, Duration::from_secs(CODE_RECHECK_SECS))
    }

    /// As [`Memo::new`], looking at the code repository again every `recheck`.
    pub fn with_recheck(cap_bytes: usize, recheck: Duration) -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("the operating system has no random bytes to give");
        Memo {
            lru: Mutex::new(Lru::default()),
            cap_bytes,
            boot: u64::from_le_bytes(bytes),
            inflight: Mutex::new(HashMap::new()),
            code_states: Mutex::new(HashMap::new()),
            recheck,
        }
    }

    /// The code repository's fingerprint: the one kept if it is younger than the recheck, else a
    /// fresh one, kept with the time. The lock is not held while the fingerprint is computed.
    pub fn code_state(&self, repo: &Path) -> String {
        {
            let kept = self.code_states.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((at, state)) = kept.get(repo)
                && at.elapsed() < self.recheck
            {
                return state.clone();
            }
        }
        let state = kanbanr_core::coderepo::fingerprint(repo);
        self.code_states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repo.to_path_buf(), (Instant::now(), state.clone()));
        state
    }

    /// The weak ETag of a stamp in this process: `W/"<16 hex>"`, FNV-1a 64 over the boot id and the
    /// stamp's fields joined by NUL. Weak, because gzip changes the bytes and not the meaning.
    pub fn etag(&self, stamp: &Stamp) -> String {
        let joined = format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            self.boot, stamp.r#gen, stamp.board_head, stamp.code, stamp.day, stamp.ended
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

/// The stamp from its parts. `code` is `Some` only for a read that uses the code repository: its
/// fingerprint. A board-only read ignores it. There is no time in the stamp: a body is kept for as
/// long as nothing it was computed from has changed (DEF-106).
pub fn stamp_at(generation: u64, board_head: &str, code: Option<&str>, day: &str) -> Stamp {
    Stamp {
        r#gen: generation,
        board_head: board_head.to_string(),
        code: code.unwrap_or_default().to_string(),
        day: day.to_string(),
        ended: 0,
    }
}

/// The stamp for a canonical key now: the project's generation (the root generation for a route
/// with no project), the board HEAD of `store_for(st, project)` (`global_head` for a route with no
/// project), for the reads `reads_code_repo` names the code repository's fingerprint (re-checked
/// every `CODE_RECHECK_SECS`; none when there is no repository), the UTC day, and for a project's
/// read how many granted delegations have ended.
pub fn stamp_for(st: &AppState, key: &str) -> Stamp {
    let project = crate::routes::project_id(key);
    let generation = st.gens.of(project.as_deref());
    let board_head = match project.as_deref() {
        Some(p) => kanbanr_core::git::head_commit(store_for(st, p).data_dir()).unwrap_or_default(),
        None => global_head(st, key),
    };
    let code = crate::routes::reads_code_repo(key).then(|| {
        crate::routes::repo_for(st, key)
            .map(|repo| st.memo.code_state(repo))
            .unwrap_or_default()
    });
    let now = kanbanr_core::now_rfc3339();
    let day = now.get(..10).unwrap_or(&now);
    let ended = project.as_deref().map_or(0, |p| {
        kanbanr_core::decisions::ended_granted(store_for(st, p), p, &now)
    });
    stamp_at(generation, &board_head, code.as_deref(), day).with_ended(ended)
}

/// The store holding a project's board: the first board, in order, that holds it (FTR-040, X11).
/// A project no board holds gets the first board's, whose reads then answer as they do for an
/// unknown project (404).
pub(crate) fn store_for<'a>(st: &'a AppState, project: &str) -> &'a Store {
    st.boards
        .for_project(project)
        .map(|b| b.store.as_ref())
        .unwrap_or(st.boards.first().store.as_ref())
}

/// The board HEAD a read with no project is stamped with (FTR-040): the HEAD of the board its
/// `board=` parameter names, else every board's, joined, since the project list and the board list
/// answer from all of them. With one board this is that board's HEAD, as before.
fn global_head(st: &AppState, key: &str) -> String {
    let head = |s: &Store| kanbanr_core::git::head_commit(s.data_dir()).unwrap_or_default();
    match crate::routes::split_board(key)
        .0
        .and_then(|name| st.boards.by_name(&name))
    {
        Some(board) => head(&board.store),
        None => st
            .boards
            .all()
            .iter()
            .map(|b| head(&b.store))
            .collect::<Vec<_>>()
            .join(","),
    }
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
        stamp_at(generation, "board1", None, "2026-11-17")
    }

    #[test]
    fn a_stamp_changes_with_a_board_event_a_commit_or_the_day() {
        let memo = Memo::new(MEMO_CAP_BYTES);
        let base = stamp_at(1, "board1", Some("code1"), "2026-11-17");
        let same = stamp_at(1, "board1", Some("code1"), "2026-11-17");
        assert_eq!(memo.etag(&base), memo.etag(&same));
        let etag = memo.etag(&base);
        assert!(etag.starts_with("W/\"") && etag.ends_with('"'), "{etag}");
        assert_eq!(etag.len(), "W/\"\"".len() + 16, "{etag}");

        let changed = [
            stamp_at(2, "board1", Some("code1"), "2026-11-17"), // a board event
            stamp_at(1, "board2", Some("code1"), "2026-11-17"), // a board commit
            stamp_at(1, "board1", Some("code2"), "2026-11-17"), // a code change
            stamp_at(1, "board1", Some("code1"), "2026-11-18"), // the day
            base.clone().with_ended(1),                         // a delegation lapsed mid-day
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

    /// A git repository in the temp dir with one committed file (L-41).
    fn scratch_repo() -> std::path::PathBuf {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let dir =
            std::env::temp_dir().join(format!("kanbanr-memo-code-{}", u64::from_le_bytes(bytes)));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}: {out:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "one"]);
        dir
    }

    #[test]
    fn a_code_read_is_kept_while_the_code_repository_is_unchanged() {
        let repo = scratch_repo();
        let memo = Memo::with_recheck(MEMO_CAP_BYTES, Duration::ZERO);
        let first = memo.code_state(&repo);
        assert!(!first.is_empty(), "a repository has a fingerprint");
        // Looked at again on every call (recheck zero): nothing changed, so the stamp holds.
        assert_eq!(memo.code_state(&repo), first);
        let at = |code: &str| stamp_at(1, "b", Some(code), "2026-11-17");
        assert_eq!(at(&first), at(&memo.code_state(&repo)));
        // Which reads are which is `reads_code_repo`'s.
        assert!(crate::routes::reads_code_repo("/projects/p/readiness"));
        assert!(crate::routes::reads_code_repo("/projects/p/review/summary"));
        assert!(!crate::routes::reads_code_repo("/projects/p/index"));
        // No repository: no fingerprint.
        assert_eq!(memo.code_state(&repo.join("nowhere")), "");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn an_uncommitted_code_edit_changes_the_stamp_at_the_next_recheck() {
        let repo = scratch_repo();
        let memo = Memo::with_recheck(MEMO_CAP_BYTES, Duration::from_millis(300));
        let before = memo.code_state(&repo);
        std::fs::write(repo.join("a.txt"), "two, uncommitted").unwrap();
        assert_eq!(memo.code_state(&repo), before, "kept until the recheck");
        std::thread::sleep(Duration::from_millis(400));
        assert_ne!(memo.code_state(&repo), before, "seen at the next recheck");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_read_slower_than_the_recheck_is_served_from_the_memo_next_time() {
        let repo = scratch_repo();
        let (mut st, dir) = scratch_state();
        st.code_repo = Some(("p".to_string(), repo.clone()));
        st.memo = Arc::new(Memo::with_recheck(
            MEMO_CAP_BYTES,
            Duration::from_millis(100),
        ));
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let app = axum::Router::new()
            .route(
                "/projects/p/readiness",
                axum::routing::get(move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        ([(CONTENT_TYPE, "application/json")], "{\"ok\":true}")
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(st.clone(), layer))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let first = get(addr, "/projects/p/readiness").await;
        // The read outlived the recheck; the repository did not change.
        let second = get(addr, "/projects/p/readiness").await;
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
        assert!(first.starts_with("HTTP/1.1 200"), "{first}");
        assert!(second.starts_with("HTTP/1.1 200"), "{second}");
        assert_eq!(count.load(Ordering::SeqCst), 1, "the second read is a hit");
    }

    #[test]
    fn a_delegation_that_lapses_mid_day_changes_what_a_read_is_stamped_with() {
        let (st, dir) = scratch_state();
        let store = store_for(&st, "p");
        // Two seconds from now, as an RFC 3339 UTC time (days to a civil date, after H. Hinnant).
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2;
        let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        let soon = format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            rest / 3_600,
            rest % 3_600 / 60,
            rest % 60
        );
        std::fs::create_dir_all(store.project_dir("p")).unwrap();
        let stamped = "{by: Ada, at: \"2026-11-17T00:00:00Z\", origin: test}";
        std::fs::write(
            store.project_dir("p").join("decisions.yaml"),
            format!(
                "delegations:\n- id: DLG-1\n  verdicts: [approve]\n  all: true\n  until: \"{soon}\"\n  proposed: {stamped}\n  granted: {stamped}\n"
            ),
        )
        .unwrap();
        let before = stamp_for(&st, "/projects/p/review/summary");
        assert_eq!(before.ended, 0);
        std::thread::sleep(Duration::from_millis(2500));
        let after = stamp_for(&st, "/projects/p/review/summary");
        assert_eq!(
            after.ended, 1,
            "the lapse is counted at once, with no day change"
        );
        let memo = Memo::new(MEMO_CAP_BYTES);
        assert_ne!(memo.etag(&before), memo.etag(&after));
        let _ = std::fs::remove_dir_all(&dir);
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
        let boards = Arc::new(kanbanr_core::boards::Boards::single(dir.clone()));
        let st = AppState {
            store: Arc::clone(&boards.first().store),
            boards,
            data_dir: dir.clone(),
            tx,
            allow_writes: false,
            push: crate::PushPolicy::for_board(&dir).0,
            code_repo: None,
            gens: Arc::new(Generations::default()),
            memo: Arc::new(Memo::new(MEMO_CAP_BYTES)),
            slices: tokio::sync::broadcast::channel(4).0,
            background: Arc::new(OnceLock::new()),
            katalogr: kanbanr_core::katalogr::Settings::from_env(),
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
