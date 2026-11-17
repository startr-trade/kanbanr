//! The person accepts the system baseline on the monitor (FTR-006): the write-enabled daemon's
//! `POST …/catalog/baseline/accept` records who and where, and the pins; a read-only daemon has no
//! write route at all.

mod common;

use common::{Scratch, board, page_token, serve, serve_boards, status};
use kanbanr_core::catalog::{Catalog, Level, Row, Source, SourceKind};
use serde_json::{Value, json};

/// A write through core's one write path, as the CLI makes it from `origin`.
fn write(data: &std::path::Path, path: &str, body: Value, origin: kanbanr_core::Origin) -> Value {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    let out = kanbanr_core::write::apply_as(
        &store,
        data,
        kanbanr_core::git::PushPolicy::Off,
        "POST",
        path,
        Some(&body),
        None,
        &kanbanr_core::eventing::NullSender,
        &kanbanr_core::agentscope::ProcessScope::Unscoped,
        origin,
    )
    .unwrap_or_else(|e| panic!("POST {path}: {e}"));
    serde_json::from_str(&out.body).unwrap()
}

/// A project with a one-row system catalog, its doc written, and the baseline initialised.
fn board_with_baseline(s: &Scratch) -> std::path::PathBuf {
    let data = board(s);
    write(
        &data,
        "/projects",
        json!({"name": "demo"}),
        kanbanr_core::Origin::Terminal,
    );
    let store = kanbanr_core::Store::new(data.clone());
    kanbanr_core::catalog::save(
        &store,
        "demo",
        &Catalog {
            schema: kanbanr_core::catalog::SCHEMA,
            rows: vec![Row {
                id: "context".into(),
                title: "context".into(),
                answers: vec!["what".into()],
                for_: vec!["architect".into()],
                level: Level::System,
                source: Source {
                    kind: SourceKind::Written,
                    path: "architecture/context.md".into(),
                    ..Source::default()
                },
                ..Row::default()
            }],
            ..Catalog::default()
        },
    )
    .unwrap();
    store
        .write_doc(
            "demo",
            "architecture/context.md",
            "# Context\n\nThe system.\n",
        )
        .unwrap();
    write(
        &data,
        "/projects/demo/catalog/baseline/init",
        json!({}),
        kanbanr_core::Origin::Terminal,
    );
    data
}

/// FTR-006 R-2: the monitor's accept records the person, `origin: monitor` and one pin per
/// applying system row, and the same state reads back; Claude's session is refused the same
/// route; a read-only daemon has no such route.
#[test]
fn accepting_the_system_baseline_on_the_monitor_records_the_person_and_the_pins() {
    let s = Scratch::new("system-baseline");
    let data = board_with_baseline(&s);
    let url = serve(&s, &data);
    let token = page_token(&url);
    let accept = format!("{url}/api/write/projects/demo/catalog/baseline/accept");

    let before: Value = serde_json::from_str(
        &ureq::get(&format!("{url}/api/projects/demo/catalog/baseline"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(before["state"]["kind"], "awaiting", "{before}");
    let seen = before["state"]["pins_rev"].as_str().unwrap().to_string();

    let r = ureq::post(&accept)
        .set("x-kanbanr-token", &token)
        .set("Origin", &url)
        .set("content-type", "application/json")
        .send_string(
            &json!({"by": "Tester", "note": "agreed on the call", "rev": seen}).to_string(),
        );
    let answer = match r {
        Ok(r) => r.into_string().unwrap(),
        Err(ureq::Error::Status(code, r)) => panic!("{code}: {}", r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    };
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["state"]["kind"], "accepted", "{answer}");

    let store = kanbanr_core::Store::new(data.clone());
    let file = kanbanr_core::systembaseline::load(&store, "demo")
        .unwrap()
        .unwrap();
    assert_eq!(file.accepted.len(), 1);
    let a = &file.accepted[0];
    assert_eq!(
        (a.by.as_str(), a.origin.as_str(), a.note.as_str()),
        ("Tester", "monitor", "agreed on the call")
    );
    assert_eq!(a.pins.len(), 1);
    assert_eq!(a.pins[0].row, "context");
    assert!(!a.pins[0].rev.is_empty());
    assert_eq!(a.rev, seen);
    let log = common::git(&data, &["log", "--format=%s", "-1"]);
    assert_eq!(log, "system baseline: accepted");

    // A read-only daemon takes no writes.
    let ro = serve_boards(&s, &[&data]);
    let code = status(
        ureq::post(&format!(
            "{ro}/api/write/projects/demo/catalog/baseline/accept"
        ))
        .set("content-type", "application/json")
        .send_string(&json!({"by": "Tester"}).to_string()),
    );
    assert!(matches!(code, 403..=405), "read-only accept: {code}");
}
