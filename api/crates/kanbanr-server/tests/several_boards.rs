//! One read-only server over several boards (FTR-040): the union of their projects, each read
//! from its own board, nothing resolved across boards, and the boards listed at `/api/boards`.
//!
//! Every board is generated under the system temporary directory (L-41), never under the
//! repository, through `kanbanr_core::Store` and committed with git; the server is the library's
//! own `run_boards`, on a free port.

mod common;

use common::{Scratch, git, serve_boards, status};
use serde_json::{Value, json};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A board folder `<scratch>/<name>` holding `project` with item FTR-001 titled `title`, a doc,
/// and one activity entry naming the board, committed. Checked to be the test's own: under the
/// system temporary directory, compared by components after resolving both (L-41, L-46).
fn make_board(scratch: &Scratch, name: &str, project: &str, title: &str) -> PathBuf {
    let dir = scratch.0.join(name);
    std::fs::create_dir_all(dir.join("projects")).unwrap();
    let resolved = dir.canonicalize().unwrap();
    let tmp = std::env::temp_dir().canonicalize().unwrap();
    assert!(
        resolved.starts_with(&tmp),
        "the fixture board {} is not under the temporary directory {}",
        resolved.display(),
        tmp.display()
    );
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "Tester"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    let store = kanbanr_core::Store::new(dir.clone());
    store
        .init_project(
            project,
            kanbanr_core::config::ProjectConfig::default_for(project),
        )
        .unwrap();
    batch(
        &dir,
        project,
        vec![
            json!({"op": "milestone.add", "name": "First", "code": "M1"}),
            json!({"op": "feature.add", "code": "FTR-001", "title": title, "milestone": "M1"}),
            json!({"op": "doc.write", "path": format!("notes/{name}-note.md"), "content": format!("# Board {name}\n")}),
        ],
    );
    kanbanr_core::activity::append(
        &dir,
        project,
        "Tester",
        &format!("on board {name}: added {title}"),
        Some("FTR-001"),
    );
    commit(&dir, &format!("board {name}: start"));
    dir
}

/// Apply batch ops to `project` on the board at `dir`.
fn batch(dir: &Path, project: &str, ops: Vec<Value>) {
    let ops: Vec<kanbanr_core::batch::BatchOp> = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).unwrap())
        .collect();
    kanbanr_core::Store::new(dir.to_path_buf())
        .apply_batch(project, ops)
        .unwrap();
}

/// Commit everything on the board at `dir`.
fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

fn get(url: &str) -> (u16, String) {
    match ureq::get(url).call() {
        Ok(r) => (r.status(), r.into_string().unwrap()),
        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap_or_default()),
        Err(e) => panic!("GET {url}: {e}"),
    }
}

fn get_json(url: &str) -> Value {
    let (code, body) = get(url);
    assert_eq!(code, 200, "GET {url}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {url}: {e}: {body}"))
}

/// The two boards most tests use: `a` with project `alpha`, then `b` with project `beta`.
fn two_boards(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let a = make_board(scratch, "a", "alpha", "Alpha keeps the cart");
    let b = make_board(scratch, "b", "beta", "Beta ships the parcel");
    (a, b)
}

/// R-1: `/api/projects` lists board a's project, then board b's, each naming its board; and the
/// board list answers within its (loose) budget.
#[test]
fn one_server_lists_both_boards_projects() {
    let scratch = Scratch::new("boards-union");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    let started = Instant::now();
    let projects = get_json(&format!("{url}/api/projects"));
    let took_projects = started.elapsed();
    let got: Vec<(&str, &str)> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["board"].as_str().unwrap()))
        .collect();
    assert_eq!(got, [("alpha", "a"), ("beta", "b")], "{projects}");
    let started = Instant::now();
    let boards = get_json(&format!("{url}/api/boards"));
    let took_boards = started.elapsed();
    assert_eq!(boards.as_array().unwrap().len(), 2, "{boards}");
    eprintln!("/api/projects took {took_projects:?}, /api/boards took {took_boards:?}");
    assert!(
        took_boards < Duration::from_millis(500),
        "/api/boards took {took_boards:?}"
    );
}

/// R-1: a project's routes answer from the board that holds it.
#[test]
fn a_project_route_answers_from_its_own_board() {
    let scratch = Scratch::new("boards-own-route");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    let beta = get(&format!("{url}/api/projects/beta")).1;
    assert!(beta.contains("Beta ships the parcel"), "{beta}");
    assert!(!beta.contains("Alpha keeps the cart"), "{beta}");
    let alpha = get(&format!("{url}/api/projects/alpha")).1;
    assert!(alpha.contains("Alpha keeps the cart"), "{alpha}");
    let docs = get_json(&format!("{url}/api/projects/beta/docs")).to_string();
    assert!(docs.contains("b-note"), "{docs}");
    assert!(!docs.contains("a-note"), "{docs}");
    let (code, doc) = get(&format!(
        "{url}/api/projects/beta/docs/content?path=notes/b-note.md"
    ));
    assert_eq!(code, 200, "{doc}");
    assert!(doc.contains("# Board b"), "{doc}");
    // Through the passthrough: a project read core's dispatch answers.
    let index = get(&format!("{url}/api/projects/beta/index"));
    assert_eq!(index.0, 200, "{}", index.1);
    assert!(index.1.contains("Beta ships the parcel"), "{}", index.1);
    let (code, _) = get(&format!("{url}/api/projects/nope"));
    assert_eq!(code, 404);
}

/// R-2: no write route exists, and the daemon says it takes no writes.
#[test]
fn write_routes_are_absent_with_boards() {
    let scratch = Scratch::new("boards-no-writes");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    // With the page's own token, so the guard lets the request reach the router.
    let token = common::page_token(&url);
    let code = status(
        ureq::post(&format!("{url}/api/write/projects"))
            .set("x-kanbanr-token", &token)
            .set("content-type", "application/json")
            .send_string(r#"{"id":"gamma"}"#),
    );
    assert!(
        code == 405 || code == 404,
        "POST /api/write/projects: {code}"
    );
    let code = status(
        ureq::post(&format!("{url}/api/sync"))
            .set("x-kanbanr-token", &token)
            .call(),
    );
    assert!(code == 405 || code == 404, "POST /api/sync: {code}");
    let meta = get_json(&format!("{url}/api/meta"));
    assert_eq!(meta["writes"], false, "{meta}");
    assert_eq!(meta["identity"], Value::Null, "{meta}");
    assert!(!b.join("projects/gamma").exists());
    assert!(!a.join("projects/gamma").exists());
}

/// Every file under `dir`, relative, sorted, with its size.
fn listing(dir: &Path) -> Vec<(PathBuf, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((path.strip_prefix(root).unwrap().to_path_buf(), len));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// R-2: a pass over every route the tests call changes nothing in either board's folder.
#[test]
fn no_board_folder_changes_while_served() {
    let scratch = Scratch::new("boards-unchanged");
    let (a, b) = two_boards(&scratch);
    let before: Vec<_> = [&a, &b]
        .iter()
        .map(|d| (git(d, &["status", "--porcelain"]), listing(d)))
        .collect();
    let url = serve_boards(&scratch, &[&a, &b]);
    for path in [
        "/api/projects",
        "/api/boards",
        "/api/meta",
        "/api/portfolio",
        "/api/portfolio?board=b",
        "/api/portfolio/rollups",
        "/api/portfolio/board",
        "/api/ready",
        "/api/ready?board=b",
        "/api/blocked",
        "/api/doctor",
        "/api/projects/alpha",
        "/api/projects/beta",
        "/api/projects/beta/index",
        "/api/projects/beta/docs",
        "/api/projects/beta/docs/content?path=notes/b-note.md",
        "/api/projects/beta/activity",
        "/api/projects/alpha/ready",
        "/api/projects/beta/features/FTR-001/export",
    ] {
        let _ = get(&format!("{url}{path}"));
    }
    let after: Vec<_> = [&a, &b]
        .iter()
        .map(|d| (git(d, &["status", "--porcelain"]), listing(d)))
        .collect();
    assert_eq!(before, after);
    assert_eq!(before[0].0, "", "board a is clean");
    assert_eq!(before[1].0, "", "board b is clean");
}

/// R-4: a project's activity comes from its own board only.
#[test]
fn each_projects_activity_comes_from_its_own_board() {
    let scratch = Scratch::new("boards-activity");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    let activity = get_json(&format!("{url}/api/projects/beta/activity"));
    let messages: Vec<&str> = activity
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("on board b")),
        "{messages:?}"
    );
    assert!(
        !messages.iter().any(|m| m.contains("on board a")),
        "{messages:?}"
    );
}

/// R-5: an item on board a that depends on a project of board b is not resolved: completing the
/// dependency on board b leaves alpha's ready answer as it was.
#[test]
fn a_dependency_on_another_boards_project_is_not_resolved() {
    let scratch = Scratch::new("boards-external");
    let (a, b) = two_boards(&scratch);
    batch(
        &a,
        "alpha",
        vec![
            json!({"op": "feature.add", "code": "FTR-002", "title": "Alpha waits for beta",
                    "milestone": "M1"}),
        ],
    );
    // Board a's own writes refuse a dependency it cannot resolve, so it is written into the
    // item's file as a board that once held both projects would hold it.
    let file = listing(&a)
        .into_iter()
        .map(|(p, _)| a.join(p))
        .find(|p| p.file_name().is_some_and(|n| n == "FTR-002.yaml"))
        .expect("FTR-002's file");
    let yaml = std::fs::read_to_string(&file).unwrap();
    let kept: Vec<&str> = yaml.lines().filter(|l| *l != "depends_on: []").collect();
    assert!(!kept.iter().any(|l| l.starts_with("depends_on")), "{yaml}");
    let yaml = format!("{}\ndepends_on:\n- beta:FTR-001\n", kept.join("\n"));
    std::fs::write(&file, yaml).unwrap();
    let item = kanbanr_core::Store::new(a.clone())
        .load_feature("alpha", "FTR-002")
        .unwrap();
    assert_eq!(item.depends_on, ["beta:FTR-001"]);
    commit(&a, "board a: a dependency on beta");
    let url = serve_boards(&scratch, &[&a, &b]);
    let before = get(&format!("{url}/api/projects/alpha/ready"));
    assert_eq!(before.0, 200, "{}", before.1);
    let blocked_before = get(&format!("{url}/api/projects/alpha/blocked"));
    let statuses = kanbanr_core::Store::new(b.clone())
        .load_meta("beta")
        .unwrap()
        .config
        .statuses;
    let done = statuses.last().unwrap().clone();
    batch(
        &b,
        "beta",
        vec![json!({"op": "feature.move", "code": "FTR-001", "to": done,
                    "unapproved": "a test fixture"})],
    );
    commit(&b, "board b: beta's FTR-001 done");
    // A commit on board a too, so alpha's reads are computed afresh rather than served from the
    // memo at the old stamp.
    git(
        &a,
        &["commit", "-q", "--allow-empty", "-m", "board a: touch"],
    );
    let after = get(&format!("{url}/api/projects/alpha/ready"));
    assert_eq!(before, after, "alpha's ready answer moved with board b");
    let blocked_after = get(&format!("{url}/api/projects/alpha/blocked"));
    assert_eq!(blocked_before, blocked_after);
    // Board b itself saw the change.
    let beta = get(&format!("{url}/api/projects/beta")).1;
    assert!(beta.contains(&done), "{beta}");
}

/// R-6: a change on the second board reaches its project's event stream.
#[test]
fn a_change_on_the_second_board_fires_its_projects_event() {
    let scratch = Scratch::new("boards-events");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    let resp = ureq::get(&format!("{url}/api/projects/beta/events"))
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
    // The store writes atomically (a temporary file renamed into place).
    batch(
        &b,
        "beta",
        vec![json!({"op": "feature.edit", "code": "FTR-001", "title": "Beta ships it today"})],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut event = String::new();
    let mut fired = false;
    while !fired {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = rx.recv_timeout(left) else {
            break;
        };
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data:") {
            fired = event == "changed" && d.trim() == "beta";
        }
    }
    assert!(fired, "no `changed` / `beta` event within 5 s of the write");
}

/// R-7: a project a later board gains is listed there as shadowed, and still served once, from
/// the first board.
#[test]
fn the_board_list_reports_a_later_conflict() {
    let scratch = Scratch::new("boards-later");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    kanbanr_core::Store::new(b.clone())
        .init_project(
            "alpha",
            kanbanr_core::config::ProjectConfig::default_for("alpha"),
        )
        .unwrap();
    assert!(b.join("projects/alpha/config.yaml").is_file());
    // Asked until the watcher's change has moved the memo's stamp.
    let deadline = Instant::now() + Duration::from_secs(5);
    let boards = loop {
        let boards = get_json(&format!("{url}/api/boards"));
        if boards[1]["shadowed"] == json!(["alpha"]) || Instant::now() > deadline {
            break boards;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(boards[1]["shadowed"], json!(["alpha"]), "{boards}");
    assert_eq!(boards[1]["projects"], json!(["beta"]), "{boards}");
    assert_eq!(boards[0]["projects"], json!(["alpha"]), "{boards}");
    let projects = get_json(&format!("{url}/api/projects"));
    let alphas: Vec<&Value> = projects
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["id"] == "alpha")
        .collect();
    assert_eq!(alphas.len(), 1, "{projects}");
    assert_eq!(alphas[0]["board"], "a", "{projects}");
    let alpha = get(&format!("{url}/api/projects/alpha")).1;
    assert!(alpha.contains("Alpha keeps the cart"), "{alpha}");
}

/// Whether `s` reads as an RFC 3339 UTC time: `YYYY-MM-DDTHH:MM:SS` then `Z` or an offset.
fn is_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.into_iter().all(|i| b[i].is_ascii_digit());
    b.len() >= 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && (s.ends_with('Z') || s[19..].contains('+') || s[19..].contains('-'))
}

/// R-8: the board list names each board, its projects and its last commit, and no server path.
#[test]
fn the_board_list_names_each_boards_projects_and_last_commit() {
    let scratch = Scratch::new("boards-list");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    let body = get(&format!("{url}/api/boards")).1;
    let boards: Value = serde_json::from_str(&body).unwrap();
    for (i, (name, project, dir)) in [("a", "alpha", &a), ("b", "beta", &b)]
        .into_iter()
        .enumerate()
    {
        let board = &boards[i];
        assert_eq!(board["name"], name, "{body}");
        assert_eq!(board["projects"], json!([project]), "{body}");
        assert_eq!(board["head"], git(dir, &["rev-parse", "HEAD"]), "{body}");
        let at = board["head_at"].as_str().unwrap();
        assert!(is_rfc3339(at), "head_at {at}");
        assert!(board.get("history_unreadable").is_none(), "{body}");
    }
    let shown = scratch.0.display().to_string();
    let resolved = scratch.0.canonicalize().unwrap().display().to_string();
    assert!(!body.contains(&shown), "{body}");
    assert!(!body.contains(&resolved), "{body}");
}

/// The daemon over one data folder, read-only, as `kanbanr serve` runs it. Returns its base URL.
fn serve_data(data: &Path) -> String {
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
    for _ in 0..200 {
        if ureq::get(&format!("{url}/healthz")).call().is_ok() {
            return url;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the monitor did not come up on {bind}");
}

/// R-10: a list of one board answers as the data folder did, apart from the added `board`.
#[test]
fn a_list_of_one_board_serves_as_the_data_folder_did() {
    let scratch = Scratch::new("boards-one");
    let a = make_board(&scratch, "a", "alpha", "Alpha keeps the cart");
    let listed = serve_boards(&scratch, &[&a]);
    let plain = serve_data(&a);
    let mut from_list = get_json(&format!("{listed}/api/projects"));
    let from_folder = get_json(&format!("{plain}/api/projects"));
    assert_eq!(from_list[0]["board"], "a", "{from_list}");
    assert!(from_folder[0].get("board").is_some(), "{from_folder}");
    for p in from_list.as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("board");
    }
    let mut from_folder = from_folder;
    for p in from_folder.as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("board");
    }
    assert_eq!(from_list, from_folder);
    for path in ["/api/projects/alpha", "/api/portfolio", "/api/ready"] {
        assert_eq!(
            get(&format!("{listed}{path}")),
            get(&format!("{plain}{path}")),
            "{path}"
        );
    }
}

/// R-11: a global read answers the first board, or the board named by `?board=`; nothing in one
/// answer comes from both boards, and an unknown board is a 404.
#[test]
fn a_global_read_answers_the_first_board_unless_one_is_named() {
    let scratch = Scratch::new("boards-global");
    let (a, b) = two_boards(&scratch);
    let url = serve_boards(&scratch, &[&a, &b]);
    for route in ["/api/portfolio", "/api/ready", "/api/portfolio/board"] {
        let (code, first) = get(&format!("{url}{route}"));
        assert_eq!(code, 200, "{route}: {first}");
        assert!(first.contains("alpha"), "{route}: {first}");
        assert!(!first.contains("beta"), "{route}: {first}");
        let sep = if route.contains('?') { '&' } else { '?' };
        let (code, second) = get(&format!("{url}{route}{sep}board=b"));
        assert_eq!(code, 200, "{route}?board=b: {second}");
        assert!(second.contains("beta"), "{route}?board=b: {second}");
        assert!(!second.contains("alpha"), "{route}?board=b: {second}");
        let (code, named_a) = get(&format!("{url}{route}{sep}board=a"));
        assert_eq!(code, 200);
        assert_eq!(named_a, first, "{route}?board=a");
        let (code, unknown) = get(&format!("{url}{route}{sep}board=zz"));
        assert_eq!(code, 404, "{route}?board=zz: {unknown}");
        assert_eq!(unknown, "no board named 'zz'", "{route}");
    }
}
