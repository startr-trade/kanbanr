//! The read API contract (FEAT-177): the daemon serves every public route exactly as the published
//! OpenAPI document says — status, media type, and a body that validates against the schema — and
//! the published document is the one the code describes.
//!
//! The fixture board is built with `kanbanr_core::dispatch`, the function the CLI's local mode
//! writes through, in a folder under the system temporary directory; the daemon is the library's
//! own `run`, on a free port.

use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

/// A scratch board folder under the system temporary directory, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-contract-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(store: &kanbanr_core::Store, method: &str, path: &str, body: Value) -> Value {
    let out = kanbanr_core::dispatch::dispatch(store, method, path, Some(&body))
        .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
    serde_json::from_str(&out).unwrap_or(Value::Null)
}

/// A board with something on every public route: a Scrum project with a charter, items with
/// definitions (one approved and in a sprint and a release), a sprint, a release, an ADR, a lesson,
/// documents (one an image), activity, and a program.
fn fixture(data: &std::path::Path) {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    write(
        &store,
        "POST",
        "/projects",
        json!({"name": "shop", "workflow": "scrum", "description": "A shop"}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/milestones",
        json!({"name": "First", "code": "M1"}),
    );
    write(
        &store,
        "PUT",
        "/projects/shop/charter",
        json!({"purpose": "Sell things", "goals": [{"statement": "Customers can pay"}]}),
    );
    let definition = |statement: &str| {
        json!({
            "statement": statement,
            "goals": ["G-1"],
            "zachman": {"what": "w", "how": "h", "where": "e", "when": "n", "who": "o", "why": "y"},
            "requirements": [{
                "kind": "functional",
                "text": "THE SYSTEM SHALL keep the cart.",
                "tests": [{"name": "shop::keeps_the_cart", "state": "planned"}],
            }],
        })
    };
    for title in ["Cart", "Checkout", "Wishlist"] {
        let f = write(
            &store,
            "POST",
            "/projects/shop/features",
            json!({"title": title, "specification": "# Spec\n\nText.", "milestone": "M1"}),
        );
        let code = f["code"].as_str().unwrap().to_string();
        write(
            &store,
            "PUT",
            &format!("/projects/shop/features/{code}/definition"),
            definition(&format!("{title} matters")),
        );
    }
    write(
        &store,
        "POST",
        "/projects/shop/features/FEAT-001/approve",
        json!({"by": "Ada L"}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/features/FEAT-001/todos",
        json!({"description": "Build it"}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/releases",
        json!({"version": "1.0", "name": "First"}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/releases/1.0/plan",
        json!({"items": ["FEAT-001"]}),
    );
    let sprint = write(
        &store,
        "POST",
        "/projects/shop/sprints",
        json!({"start": "2026-10-19", "length_days": 14, "goal": "Pay", "capacity": 10.0}),
    );
    let sprint = sprint["code"].as_str().unwrap().to_string();
    write(
        &store,
        "POST",
        &format!("/projects/shop/sprints/{sprint}/plan"),
        json!({"items": ["FEAT-001"]}),
    );
    write(
        &store,
        "POST",
        &format!("/projects/shop/sprints/{sprint}/start"),
        json!({}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/adrs",
        json!({"title": "Keep carts in YAML"}),
    );
    write(
        &store,
        "POST",
        "/projects/shop/lessons",
        json!({"lesson": "Name the test before the code", "from_item": "FEAT-001", "confidence": 0.8}),
    );
    write(
        &store,
        "PUT",
        "/projects/shop/docs/content",
        json!({"path": "guide/intro.md", "content": "# Intro\n\nHello."}),
    );
    let png = data.join("projects/shop/docs/img");
    std::fs::create_dir_all(&png).unwrap();
    std::fs::write(png.join("dot.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    kanbanr_core::activity::append(data, "shop", "Ada L", "approve FEAT-001", Some("FEAT-001"));
    kanbanr_core::activity::append(data, "shop", "Ada L", "add milestone", None);
    write(
        &store,
        "POST",
        "/portfolio/programs",
        json!({"id": "retail", "name": "Retail", "projects": ["shop"]}),
    );
}

/// The daemon over `data`, on a free port, until the test ends. Returns its base URL.
fn serve(data: &std::path::Path) -> String {
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
    panic!("the daemon did not come up on {bind}");
}

/// A GET's status, media type and body, whatever the status.
fn get(url: &str) -> (u16, String, Vec<u8>) {
    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(10))
        .build();
    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("GET {url}: {e}"),
    };
    let status = resp.status();
    let mime = resp.header("content-type").unwrap_or_default().to_string();
    if mime == "text/event-stream" {
        // A stream never ends; its headers are the contract.
        return (status, mime, Vec::new());
    }
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut body).unwrap();
    (status, mime, body)
}

/// Every object schema closed to properties it does not name, unless it says otherwise. The
/// derived schemas leave objects open; closed, a field the code sends but the document does not
/// describe fails here instead of passing unnoticed.
fn closed(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            let names_properties = map.contains_key("properties");
            for v in map.values_mut() {
                closed(v);
            }
            if names_properties && !map.contains_key("additionalProperties") {
                map.insert("additionalProperties".into(), Value::Bool(false));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(closed),
        _ => {}
    }
}

/// The schema a response's `media` body must satisfy, with the document's components alongside so
/// its `$ref`s resolve.
fn validator(doc: &Value, schema: &Value) -> jsonschema::Validator {
    let mut root = json!({ "components": doc["components"].clone() });
    for (k, v) in schema.as_object().unwrap() {
        root[k] = v.clone();
    }
    closed(&mut root);
    jsonschema::draft202012::new(&root).expect("the response schema compiles")
}

/// The concrete requests that exercise a documented path.
fn requests(path: &str, sprint: &str) -> Vec<String> {
    let base = path
        .replace("{project}", "shop")
        .replace("{code}", "FEAT-001")
        .replace("{sprint}", sprint);
    let mut out = vec![];
    match path {
        "/api/projects/{project}/workflow" => out.push(format!("{base}?format=mermaid")),
        "/api/projects/{project}/docs/content" => out.push(format!("{base}?path=guide/intro.md")),
        "/api/projects/{project}/docs/raw" => {
            out.push(format!("{base}?path=img/dot.png"));
            out.push(format!("{base}?path=guide/intro.md"));
        }
        "/api/projects/{project}/features/{code}/export" => {
            out.push(base.clone());
            out.push(format!("{base}?format=json"));
            out.push(format!("{base}?format=md"));
        }
        "/api/projects/{project}/activity" => {
            out.push(base.clone());
            out.push(format!("{base}?ref=FEAT-001&limit=5"));
            out.push(format!("{base}?ongoing=true"));
        }
        "/api/projects/{project}/lessons" => {
            out.push(base.clone());
            out.push(format!("{base}?all"));
            out.push(format!("{base}?for=FEAT-001"));
        }
        "/api/projects/{project}/sprints/{sprint}" => {
            out.push(base.clone());
            out.push(
                path.replace("{project}", "shop")
                    .replace("{sprint}", "active"),
            );
        }
        "/api/projects/{project}/features/{code}/readiness" => {
            out.push(base.clone());
            out.push(base.replace("FEAT-001", "FEAT-002"));
        }
        _ => out.push(base),
    }
    out
}

/// Check one answer against what the document says `path` answers with `status`.
fn check(doc: &Value, path: &str, url: &str, (status, mime, body): (u16, String, Vec<u8>)) {
    let mut response = &doc["paths"][path]["get"]["responses"][status.to_string()];
    if let Some(r) = response["$ref"].as_str() {
        let name = r.trim_start_matches("#/components/responses/");
        response = &doc["components"]["responses"][name];
    }
    assert!(
        response.is_object(),
        "GET {url} answered {status}, which the document does not list for {path}: {}",
        String::from_utf8_lossy(&body)
    );
    let content = response["content"].as_object().unwrap();
    let schema = match content.get(&mime).or_else(|| content.get("*/*")) {
        Some(c) => &c["schema"],
        None => panic!(
            "GET {url} answered {status} as {mime:?}; the document lists {:?}",
            content.keys().collect::<Vec<_>>()
        ),
    };
    if mime == "application/json" {
        let value: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|e| panic!("GET {url}: labelled JSON, but is not ({e})"));
        let v = validator(doc, schema);
        let errors: Vec<String> = v
            .iter_errors(&value)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "GET {url}: the body does not match the document:\n{}\nbody: {}",
            errors.join("\n"),
            String::from_utf8_lossy(&body)
        );
    } else if mime.starts_with("text/") && mime != "text/event-stream" {
        assert!(
            String::from_utf8(body).is_ok(),
            "GET {url}: text that is not UTF-8"
        );
    }
}

mod server {
    use super::*;

    /// FEAT-177 R-1: every public route answers as the published, versioned description says, and
    /// that description is the one the code generates.
    #[test]
    fn contract_matches_openapi() {
        // The committed document is what the code describes, and what the daemon serves.
        let committed = include_str!("../../../../docs/src/reference/openapi.json");
        assert!(
            committed == kanbanr_core::openapi::document_text(),
            "docs/src/reference/openapi.json is out of date: run \
             `cargo run -p kanbanr-core --features schema --example openapi`"
        );
        // And so is the route table the HTTP API page shows.
        let page = include_str!("../../../../docs/src/reference/http-api.md");
        assert!(
            kanbanr_core::openapi::with_route_table(page).as_deref() == Some(page),
            "the route table in docs/src/reference/http-api.md is out of date: run \
             `cargo run -p kanbanr-core --features schema --example openapi`"
        );
        let doc: Value = serde_json::from_str(committed).unwrap();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], kanbanr_core::openapi::API_VERSION);

        let scratch = Scratch::new("r1");
        fixture(&scratch.0);
        let url = serve(&scratch.0);
        let (status, _, served) = get(&format!("{url}/api/openapi.json"));
        assert_eq!(status, 200);
        assert_eq!(
            served,
            committed.as_bytes(),
            "the daemon serves the document"
        );

        // Every documented path, with every success shape it has.
        let store = kanbanr_core::Store::new(scratch.0.clone());
        let sprint = kanbanr_core::sprints::load(&store, "shop").unwrap()[0]
            .code
            .clone();
        let paths = doc["paths"].as_object().unwrap();
        let mut seen_mimes = std::collections::BTreeSet::new();
        for path in paths.keys() {
            for request in requests(path, &sprint) {
                let answer = get(&format!("{url}{request}"));
                assert_eq!(
                    answer.0,
                    200,
                    "GET {request}: {}",
                    String::from_utf8_lossy(&answer.2)
                );
                seen_mimes.insert((path.clone(), answer.1.clone()));
                check(&doc, path, &request, answer);
            }
        }
        // Each media type the document lists for a route is one the route really sends.
        for (path, item) in paths {
            for mime in item["get"]["responses"]["200"]["content"]
                .as_object()
                .unwrap()
                .keys()
            {
                if mime == "*/*" {
                    continue;
                }
                assert!(
                    seen_mimes.contains(&(path.clone(), mime.clone())),
                    "{path} is documented as sending {mime}, and no request saw it"
                );
            }
        }

        // The errors are the documented ones, as plain text.
        for (path, request, want) in [
            ("/api/projects/{project}", "/api/projects/nope", 404),
            (
                "/api/projects/{project}/features/{code}/readiness",
                "/api/projects/shop/features/FEAT-999/readiness",
                404,
            ),
            (
                "/api/projects/{project}/workflow",
                "/api/projects/shop/workflow",
                400,
            ),
            (
                "/api/projects/{project}/docs/content",
                "/api/projects/shop/docs/content?path=nope.md",
                404,
            ),
        ] {
            let answer = get(&format!("{url}{request}"));
            assert_eq!(answer.0, want, "GET {request}");
            check(&doc, path, request, answer);
        }
    }

    /// FEAT-177: a GET of a path no route answers is a 404 — it was a 400, "unsupported operation".
    /// Internal routes stay served, unlisted.
    #[test]
    fn an_unknown_read_path_is_not_found_and_internal_routes_still_answer() {
        let scratch = Scratch::new("404");
        fixture(&scratch.0);
        let url = serve(&scratch.0);
        let (status, mime, body) = get(&format!("{url}/api/projects/shop/no-such-view"));
        assert_eq!(status, 404, "{}", String::from_utf8_lossy(&body));
        assert!(mime.starts_with("text/plain"), "{mime}");
        let doc: Value = serde_json::from_str(&kanbanr_core::openapi::document_text()).unwrap();
        for internal in [
            "/api/projects/shop/doctor",
            "/api/projects/shop/gates",
            "/api/projects/shop/graph",
            "/api/projects/shop/graph?format=dot",
            "/api/projects/shop/export",
            "/api/projects/shop/critical-path",
        ] {
            let (status, mime, body) = get(&format!("{url}{internal}"));
            assert_eq!(
                status,
                200,
                "{internal}: {}",
                String::from_utf8_lossy(&body)
            );
            let path = internal
                .split('?')
                .next()
                .unwrap()
                .replace("/shop", "/{project}");
            assert!(doc["paths"].get(&path).is_none(), "{path} is internal");
            // Labelled with what it is: JSON parses, text is not called JSON.
            if mime == "application/json" {
                serde_json::from_slice::<Value>(&body)
                    .unwrap_or_else(|e| panic!("{internal}: labelled JSON, but is not ({e})"));
            }
        }
        let (_, mime, _) = get(&format!("{url}/api/projects/shop/graph?format=dot"));
        assert!(mime.starts_with("text/plain"), "DOT is text: {mime}");
        let (_, mime, _) = get(&format!("{url}/api/projects/shop/export"));
        assert!(
            mime.starts_with("text/markdown"),
            "the project export is Markdown: {mime}"
        );
    }
}
