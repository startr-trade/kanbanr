//! The monitor's reads over the daemon (DEF-081): the compact index carries no bodies, JSON reads
//! are gzipped when asked and the event stream is not, an unchanged read revalidates as 304, a
//! board write changes the ETag of its own project only, the event stream sends `slices` beside
//! `changed`, and a board this size stays within the byte budget.
//!
//! The fixture is generated under the system temporary directory (L-41), never under the
//! repository: N items with M requirements and spec text of a given size, written through
//! `kanbanr_core::Store` and committed with `git2`. The daemon is the library's own `run`, on a
//! free port, as `contract.rs` starts it.

use serde_json::{Value, json};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A scratch folder under the system temporary directory, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-monitor-reads-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// The board folder inside the scratch folder, checked to be the test's own: under the system
    /// temporary directory, compared by components after resolving both (L-41, L-46).
    fn data(&self) -> PathBuf {
        let data = self.0.join("data");
        std::fs::create_dir_all(data.join("projects")).unwrap();
        let data = data.canonicalize().unwrap();
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            data.starts_with(&tmp),
            "the fixture board {} is not under the temporary directory {}",
            data.display(),
            tmp.display()
        );
        data
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a generated project holds.
#[derive(Clone, Copy)]
struct Shape {
    items: usize,
    requirements: usize,
    spec_bytes: usize,
}

/// The statuses of the fixture's workflow (the `scheduled` preset the core tests use), in the
/// order items are moved through them.
const STATUSES: [&str; 3] = ["Planned", "Scheduled", "Completed"];
const MILESTONES: usize = 8;

/// The title the generator gives item `i` of `project`.
fn title(project: &str, i: usize) -> String {
    format!("{project} item {i}: keep the cart when the customer signs in again")
}

/// Spec text of about `bytes` bytes: a heading and repeated paragraphs.
fn spec(i: usize, bytes: usize) -> String {
    let mut s = format!("# Item {i}\n\n");
    let para = "The customer's cart is kept on the server and restored on the next sign-in, so a \
                change of device loses nothing. ";
    while s.len() < bytes {
        s.push_str(para);
        if s.len() % 4 == 0 {
            s.push_str("\n\n");
        }
    }
    s.truncate(bytes);
    s
}

/// Generate `project` on the board at `data` through the core `Store`: a charter with goals,
/// milestones, `shape.items` items with definitions, `shape.requirements` requirements each,
/// spec text of `shape.spec_bytes`, a todo-list of three tasks (some done), spread over the
/// workflow's statuses, and a few docs. Returns the item codes in order.
fn generate(data: &Path, project: &str, shape: Shape) -> Vec<String> {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    store
        .init_project(
            project,
            kanbanr_core::config::ProjectConfig::default_for(project),
        )
        .unwrap();
    kanbanr_core::dispatch::dispatch(
        &store,
        "PUT",
        &format!("/projects/{project}/charter"),
        Some(&json!({
            "purpose": "Sell things",
            "goals": [
                {"statement": "Customers can pay"},
                {"statement": "Customers keep their cart"},
                {"statement": "The shop stays fast"},
            ],
        })),
    )
    .unwrap();
    let mut ops = Vec::new();
    for m in 1..=MILESTONES {
        ops.push(json!({"op": "milestone.add", "name": format!("Milestone {m}"), "code": format!("M{m}")}));
    }
    let codes: Vec<String> = (1..=shape.items).map(|i| format!("FEAT-{i:03}")).collect();
    for (i, code) in codes.iter().enumerate() {
        let requirements: Vec<Value> = (1..=shape.requirements)
            .map(|r| {
                json!({
                    "kind": "functional",
                    "text": format!("WHEN the customer signs in THE SYSTEM SHALL restore cart {r}."),
                    "tests": [{"name": format!("shop::restores_cart_{i}_{r}"), "state": "planned"}],
                })
            })
            .collect();
        ops.push(json!({
            "op": "feature.add",
            "code": code,
            "title": title(project, i + 1),
            "milestone": format!("M{}", i % MILESTONES + 1),
            "spec": spec(i + 1, shape.spec_bytes),
            "priority": (["high", "medium", "low"][i % 3]),
            "labels": ["cart"],
            "definition": {
                "statement": "Customers keep their cart across devices, so they buy more.",
                "goals": [format!("G-{}", i % 3 + 1)],
                "zachman": {"what": "w", "how": "h", "where": "e", "when": "n", "who": "o", "why": "y"},
                "requirements": requirements,
            },
        }));
        ops.push(
            json!({"op": "todo.add", "feature": code, "code": "TL-001", "description": "Build"}),
        );
        for t in 1..=3 {
            ops.push(json!({"op": "task.add", "feature": code, "todo": "TL-001", "text": format!("Step {t}"), "key": format!("T{t}")}));
        }
        for t in 1..=(i % 4).min(3) {
            ops.push(json!({"op": "task.state", "feature": code, "todo": "TL-001", "key": format!("T{t}"), "state": "Completed"}));
        }
        for to in STATUSES.iter().take(i % STATUSES.len() + 1).skip(1) {
            ops.push(json!({"op": "feature.move", "code": code, "to": to, "unapproved": "a generated fixture"}));
        }
    }
    for d in 1..=5 {
        ops.push(json!({"op": "doc.write", "path": format!("design/note-{d}.md"), "content": format!("# Note {d}\n\nText.\n")}));
    }
    let ops: Vec<kanbanr_core::batch::BatchOp> = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).unwrap())
        .collect();
    store.apply_batch(project, ops).unwrap();
    codes
}

/// Commit everything on the board at `data` with `git2`, creating the repository if needed.
fn commit_all(data: &Path, message: &str) {
    let repo =
        git2::Repository::open(data).unwrap_or_else(|_| git2::Repository::init(data).unwrap());
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Tester", "t@example.com").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .unwrap();
}

/// The daemon over `data`, read-only, on a free port, until the test ends. Returns its base URL.
fn serve(data: &Path) -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bind = format!("127.0.0.1:{port}");
    let (dir, addr) = (data.to_path_buf(), bind.clone());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(kanbanr_server::run(dir, addr, None, false, None))
            .unwrap();
    });
    let url = format!("http://{bind}");
    for _ in 0..400 {
        if ureq::get(&format!("{url}/healthz")).call().is_ok() {
            return url;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the daemon did not come up on {bind}");
}

/// A response: status, headers by lower-case name, and the body as sent (still gzipped if it was).
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body, decoded with `flate2` when it came gzipped.
    fn decoded(&self) -> Vec<u8> {
        if self.header("content-encoding") == Some("gzip") {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&self.body[..])
                .read_to_end(&mut out)
                .unwrap();
            out
        } else {
            self.body.clone()
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.decoded()).unwrap_or_else(|e| {
            panic!(
                "not JSON ({e}): {}",
                String::from_utf8_lossy(&self.decoded())
            )
        })
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(30))
        .build()
}

/// A GET with these request headers, whatever the status. An event stream's body is not read.
fn get(url: &str, headers: &[(&str, &str)]) -> Reply {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("GET {url}: {e}"),
    };
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers_names()
        .into_iter()
        .map(|k| {
            let v = resp.header(&k).unwrap_or_default().to_string();
            (k.to_ascii_lowercase(), v)
        })
        .collect();
    let mut body = Vec::new();
    let streaming = headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.starts_with("text/event-stream"));
    if !streaming {
        resp.into_reader().read_to_end(&mut body).unwrap();
    }
    Reply {
        status,
        headers,
        body,
    }
}

/// A GET as the monitor's browser sends it: gzip accepted.
fn get_gz(url: &str) -> Reply {
    let r = get(url, &[("Accept-Encoding", "gzip")]);
    assert_eq!(
        r.status,
        200,
        "GET {url}: {}",
        String::from_utf8_lossy(&r.decoded())
    );
    r
}

/// Every object key anywhere in `v`.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.push(k.clone());
                keys(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

/// A small committed board with one project of three items.
fn small_board(scratch: &Scratch, projects: &[&str]) -> (PathBuf, Vec<Vec<String>>) {
    let data = scratch.data();
    let shape = Shape {
        items: 3,
        requirements: 2,
        spec_bytes: 2_000,
    };
    let codes = projects.iter().map(|p| generate(&data, p, shape)).collect();
    commit_all(&data, "fixture");
    (data, codes)
}

/// The path of an item's YAML file on the board, whichever status folder holds it.
fn item_file(data: &Path, project: &str, code: &str) -> PathBuf {
    let features = data.join("projects").join(project).join("features");
    for status in std::fs::read_dir(&features).unwrap() {
        let file = status.unwrap().path().join(format!("{code}.yaml"));
        if file.is_file() {
            return file;
        }
    }
    panic!("{code} has no file under {}", features.display());
}

/// Rename an item by rewriting its YAML file atomically, as a CLI write does.
fn retitle(data: &Path, project: &str, code: &str, from: &str, to: &str) {
    let file = item_file(data, project, code);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(from),
        "{} holds no title {from}",
        file.display()
    );
    kanbanr_core::atomic::write(&file, text.replacen(from, to, 1)).unwrap();
}

/// R-1: the compact index the shell and the link index read has every item's code and title, and
/// no item body.
#[test]
fn the_shell_and_link_index_read_only_the_compact_index() {
    let scratch = Scratch::new("index");
    let (data, codes) = small_board(&scratch, &["shop"]);
    let url = serve(&data);
    let index = get_gz(&format!("{url}/api/projects/shop/index")).json();
    let items = index["items"].as_array().expect("items");
    assert_eq!(items.len(), codes[0].len(), "{index}");
    for (i, code) in codes[0].iter().enumerate() {
        let item = items
            .iter()
            .find(|it| it["code"] == code.as_str())
            .unwrap_or_else(|| panic!("{code} is not in the index"));
        assert_eq!(item["title"], title("shop", i + 1).as_str(), "{item}");
    }
    let mut all = Vec::new();
    keys(&index, &mut all);
    for body in ["specification", "history", "todo_lists"] {
        assert!(!all.iter().any(|k| k == body), "the index carries `{body}`");
    }
}

/// R-3: JSON reads are gzipped when the client accepts gzip; the event stream is not.
#[test]
fn json_reads_are_gzipped_when_asked_and_the_event_stream_is_not() {
    let scratch = Scratch::new("gzip");
    let (data, _) = small_board(&scratch, &["shop"]);
    let url = serve(&data);
    for path in ["/api/projects/shop/index", "/api/projects/shop"] {
        let r = get_gz(&format!("{url}{path}"));
        assert_eq!(r.header("content-encoding"), Some("gzip"), "{path}");
        assert!(r.json().is_object(), "{path} decodes to a JSON object");
    }
    let plain = get(&format!("{url}/api/projects/shop/index"), &[]);
    assert_eq!(
        plain.header("content-encoding"),
        None,
        "not asked, not gzipped"
    );
    for path in ["/api/events", "/api/projects/shop/events"] {
        let r = get(&format!("{url}{path}"), &[("Accept-Encoding", "gzip")]);
        assert_eq!(r.status, 200, "{path}");
        assert!(
            r.header("content-type")
                .is_some_and(|m| m.starts_with("text/event-stream")),
            "{path} is an event stream"
        );
        assert_eq!(
            r.header("content-encoding"),
            None,
            "{path} is not compressed"
        );
    }
}

/// R-4: a second read that names the ETag it holds is answered 304 with an empty body.
#[test]
fn an_unchanged_read_revalidates_as_not_modified() {
    let scratch = Scratch::new("etag");
    let (data, _) = small_board(&scratch, &["shop"]);
    let url = serve(&data);
    let index = format!("{url}/api/projects/shop/index");
    let first = get_gz(&index);
    let etag = first.header("etag").expect("an ETag").to_string();
    assert!(etag.starts_with("W/\""), "a weak ETag: {etag}");
    assert_eq!(first.header("cache-control"), Some("no-cache"));
    let again = get(
        &index,
        &[("Accept-Encoding", "gzip"), ("If-None-Match", &etag)],
    );
    assert_eq!(again.status, 304);
    assert!(again.body.is_empty(), "a 304 has no body");
    assert_eq!(again.header("etag"), Some(etag.as_str()));
    assert_eq!(again.header("cache-control"), Some("no-cache"));
}

/// R-5: a write to one project's board file changes that project's ETag, and its body shows the
/// change; another project's ETag stays as it was.
#[test]
fn a_board_write_changes_the_etag_of_its_project_only() {
    let scratch = Scratch::new("invalidate");
    let (data, codes) = small_board(&scratch, &["alpha", "beta"]);
    let url = serve(&data);
    let a = format!("{url}/api/projects/alpha/index");
    let b = format!("{url}/api/projects/beta/index");
    let etag_of = |u: &str| get_gz(u).header("etag").expect("an ETag").to_string();
    let (a0, b0) = (etag_of(&a), etag_of(&b));
    // The watcher is running once the daemon answers, but give it a moment to have its watch set.
    std::thread::sleep(Duration::from_millis(300));
    let code = &codes[0][0];
    retitle(
        &data,
        "alpha",
        code,
        &title("alpha", 1),
        "Renamed by the test",
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let changed = loop {
        let r = get_gz(&a);
        if r.header("etag") != Some(a0.as_str()) {
            break r;
        }
        assert!(Instant::now() < deadline, "alpha's ETag never changed");
        std::thread::sleep(Duration::from_millis(50));
    };
    let item = changed.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["code"] == code.as_str())
        .cloned()
        .unwrap();
    assert_eq!(item["title"], "Renamed by the test", "{item}");
    // Past the watcher's coalescing window, beta is still at its first stamp.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(etag_of(&b), b0, "beta's ETag moved with alpha's write");
}

/// R-7: after a write, the project's event stream sends `changed`, as API v1 does, and `slices`
/// with what changed.
#[test]
fn the_event_stream_still_sends_changed_and_adds_slices() {
    let scratch = Scratch::new("slices");
    let (data, codes) = small_board(&scratch, &["shop"]);
    let url = serve(&data);
    let resp = agent()
        .get(&format!("{url}/api/projects/shop/events"))
        .call()
        .expect("the event stream opens");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(resp.into_reader()).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    std::thread::sleep(Duration::from_millis(300));
    let code = &codes[0][0];
    retitle(
        &data,
        "shop",
        code,
        &title("shop", 1),
        "Renamed by the test",
    );
    let (mut changed, mut slices, mut event) = (false, None::<Value>, String::new());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(changed && slices.is_some()) {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = rx.recv_timeout(left) else {
            break;
        };
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data:") {
            match event.as_str() {
                "changed" => changed = true,
                "slices" => slices = Some(serde_json::from_str(d.trim()).unwrap()),
                _ => {}
            }
        }
    }
    assert!(changed, "no `changed` event within 5 s of the write");
    let slices = slices.expect("no `slices` event within 5 s of the write");
    assert_eq!(slices["project"], "shop", "{slices}");
    let tags: Vec<&str> = slices["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        tags.contains(&format!("item:{code}").as_str()) && tags.contains(&"items"),
        "the slices name the item: {slices}"
    );
}

/// R-10: on a generated 400-item board the compact index stays within 120 KB raw and 25 KB gzip,
/// and the reads of each migrated page, the shell's index included, sum to at most 150 KB gzip.
#[test]
fn reads_on_a_board_this_size_stay_within_the_byte_budget() {
    let scratch = Scratch::new("budget");
    let data = scratch.data();
    let shape = Shape {
        items: 400,
        requirements: 4,
        spec_bytes: 6_500,
    };
    let codes = generate(&data, "shop", shape);
    commit_all(&data, "fixture");
    let url = serve(&data);
    let api = |path: &str| format!("{url}/api/projects/shop{path}");

    let index = get_gz(&api("/index"));
    let (raw, gz) = (index.decoded().len(), index.body.len());
    assert_eq!(index.json()["items"].as_array().unwrap().len(), 400);
    assert!(
        raw <= 120 * 1024,
        "the index is {raw} bytes raw, over 120 KB"
    );
    assert!(
        gz <= 25 * 1024,
        "the index is {gz} bytes gzipped, over 25 KB"
    );

    // The largest status slice, the one the Status page reads at its worst.
    let status = STATUSES
        .iter()
        .max_by_key(|s| {
            codes
                .iter()
                .enumerate()
                .filter(|(i, _)| STATUSES[i % STATUSES.len()] == **s)
                .count()
        })
        .unwrap()
        .to_string();
    let pages: Vec<(&str, Vec<String>)> = vec![
        ("BoardPage", vec![api("/index"), api("/readiness")]),
        ("MilestonePage", vec![api("/index")]),
        ("MilestonesPage", vec![api("/index")]),
        ("CharterPage", vec![api("/index"), api("/charter")]),
        (
            "StatusPage",
            vec![api("/index"), api(&format!("/features?status={status}"))],
        ),
        (
            "FeaturePage",
            vec![api("/index"), api(&format!("/features/{}", codes[0]))],
        ),
        ("WorkflowPage", vec![api("/index"), api("/config")]),
    ];
    let mut report = Vec::new();
    for (page, reads) in &pages {
        let bytes: usize = reads.iter().map(|u| get_gz(u).body.len()).sum();
        report.push(format!("{page}: {bytes} B gzip"));
        assert!(
            bytes <= 150 * 1024,
            "{page} reads {bytes} bytes gzipped, over 150 KB: {reads:?}"
        );
    }
    eprintln!("index {raw} B raw, {gz} B gzip; {}", report.join("; "));
}
