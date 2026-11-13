//! A record Claude prepared is confirmed on the monitor (FTR-034): the click makes the person's
//! verdict through its own route, and the prepared record is kept beside the person's answer.

mod common;

use common::{Scratch, board, page_token, serve, status};
use serde_json::{Value, json};

const SHA: &str = "9f078bd0c1e2aaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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

/// FTR-034 R-3: the monitor confirms a prepared CI result with one edit; the gate result is the
/// person's, with the edited note; the prepared file is unchanged; the answer is kept with its
/// origin and the change; the listing says it was confirmed.
#[test]
fn confirming_a_prepared_record_makes_the_persons_record_and_keeps_the_prepared_one() {
    let s = Scratch::new("prepared");
    let data = board(&s);
    write(
        &data,
        "/projects",
        json!({"name": "demo"}),
        kanbanr_core::Origin::Terminal,
    );
    let prepared = write(
        &data,
        "/projects/demo/prepared",
        json!({
            "kind": "gate-result",
            "subject": {"gate": "ci", "commit": SHA},
            "fields": {"passed": true, "url": "https://ci.example/run/4512",
                       "note": "all 412 tests passed"},
            "sources": [{"label": "release tip", "url": format!("commit:{SHA}")}],
        }),
        kanbanr_core::Origin::Conversation,
    );
    assert_eq!(prepared["by"], "Claude");
    let id = prepared["id"].as_str().unwrap().to_string();
    let file = data.join(format!("projects/demo/prepared/{id}.yaml"));
    let before = std::fs::read(&file).unwrap();

    let url = serve(&s, &data);
    let token = page_token(&url);
    let r = ureq::post(&format!(
        "{url}/api/write/projects/demo/prepared/{id}/confirm"
    ))
    .set("x-kanbanr-token", &token)
    .set("Origin", &url)
    .set("content-type", "application/json")
    .send_string(
        &json!({"by": "Tester", "edits": {"note": "all 412 tests passed; checked the smoke run too"}})
            .to_string(),
    );
    let answer = match r {
        Ok(r) => r.into_string().unwrap(),
        Err(ureq::Error::Status(code, r)) => panic!("{code}: {}", r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    };
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["prepared"], Value::String(id.clone()), "{answer}");

    let store = kanbanr_core::Store::new(data.clone());
    let results = kanbanr_core::gates::results(&store, "demo", None).unwrap();
    assert_eq!(results.len(), 1);
    let g = &results[0];
    assert_eq!(
        (
            g.by.as_str(),
            g.origin.as_str(),
            g.commit.as_str(),
            g.passed
        ),
        ("Tester", "monitor", SHA, true)
    );
    assert_eq!(g.note, "all 412 tests passed; checked the smoke run too");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        before,
        "the prepared file changed"
    );
    let resolved = data.join(format!("projects/demo/prepared/{id}.resolved.yaml"));
    let resolution: kanbanr_core::prepared::Resolution =
        kanbanr_core::yaml::from_str(&std::fs::read_to_string(resolved).unwrap()).unwrap();
    assert_eq!(resolution.origin, "monitor");
    assert_eq!(resolution.by, "Tester");
    assert_eq!(resolution.changed.len(), 1);
    assert_eq!(resolution.changed[0].field, "note");

    let listed = ureq::get(&format!("{url}/api/projects/demo/prepared?state=all"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["state"], "confirmed", "{listed}");
    // Nothing is open any more, and a second click is refused.
    let open = ureq::get(&format!("{url}/api/projects/demo/prepared"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert_eq!(open.trim(), "[]");
    let again = ureq::post(&format!(
        "{url}/api/write/projects/demo/prepared/{id}/confirm"
    ))
    .set("x-kanbanr-token", &token)
    .set("Origin", &url)
    .set("content-type", "application/json")
    .send_string(&json!({"by": "Tester"}).to_string());
    assert_eq!(status(again), 400);
}

/// A board with one item holding a manual test, built the same way each time.
fn manual_board(s: &Scratch, name: &str) -> std::path::PathBuf {
    use kanbanr_core::models::{FeatureDefinition, Requirement, TestRef};
    let data = s.0.join(name);
    std::fs::create_dir_all(data.join("projects")).unwrap();
    common::git(&data, &["init", "-q"]);
    common::git(&data, &["config", "user.name", "Tester"]);
    common::git(&data, &["config", "user.email", "t@example.com"]);
    common::git(&data, &["commit", "-q", "--allow-empty", "-m", "start"]);
    write(
        &data,
        "/projects",
        json!({"name": "demo"}),
        kanbanr_core::Origin::Terminal,
    );
    let store = kanbanr_core::Store::new(data.clone());
    store
        .add_milestone("demo", "M", "", vec![], Some("M".into()))
        .unwrap();
    let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
    store
        .set_feature_definition(
            "demo",
            &f.code,
            Some(FeatureDefinition {
                requirements: vec![Requirement {
                    id: "R-1".into(),
                    text: "x".into(),
                    tests: vec![TestRef {
                        name: "manual: the cart looks right".into(),
                        kind: "manual".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }),
        )
        .unwrap();
    data
}

/// DEF-040 R-2: recording Passed on the Review page makes the same record `kanbanr test` makes:
/// two boards from one fixture, one recorded by the monitor and one by the terminal's request.
/// The items' tests are equal and the kept results agree apart from where and when.
#[test]
fn a_manual_result_recorded_from_the_monitor_matches_kanbanr_tests_record() {
    let s = Scratch::new("manual-result");
    let (a, b) = (manual_board(&s, "a"), manual_board(&s, "b"));
    let route = "projects/demo/features/FEAT-001/tests/R-1/manual%3A%20the%20cart%20looks%20right";
    let body =
        json!({"state": "green", "by": "Tester", "checked_rev": "abc1234", "note": "on staging"});

    let url = serve(&s, &a);
    let token = page_token(&url);
    let r = ureq::put(&format!("{url}/api/write/{route}"))
        .set("x-kanbanr-token", &token)
        .set("Origin", &url)
        .set("content-type", "application/json")
        .send_string(&body.to_string());
    assert_eq!(status(r), 200);

    let store = kanbanr_core::Store::new(b.clone());
    kanbanr_core::write::apply_as(
        &store,
        &b,
        kanbanr_core::git::PushPolicy::Off,
        "PUT",
        &format!("/{route}"),
        Some(&body),
        None,
        &kanbanr_core::eventing::NullSender,
        &kanbanr_core::agentscope::ProcessScope::Unscoped,
        kanbanr_core::Origin::Terminal,
    )
    .unwrap();

    let test_of = |data: &std::path::Path| {
        let store = kanbanr_core::Store::new(data.to_path_buf());
        let p = store.load("demo").unwrap();
        p.feature("FEAT-001")
            .unwrap()
            .definition
            .clone()
            .unwrap()
            .requirements[0]
            .tests[0]
            .clone()
    };
    assert_eq!(test_of(&a), test_of(&b));
    assert_eq!(test_of(&a).state, kanbanr_core::models::TestState::Green);
    let kept = |data: &std::path::Path| {
        let store = kanbanr_core::Store::new(data.to_path_buf());
        kanbanr_core::manual::results(&store, "demo", Some("FEAT-001")).unwrap()
    };
    let (ka, kb) = (kept(&a), kept(&b));
    assert_eq!((ka.len(), kb.len()), (1, 1));
    assert_eq!(
        (ka[0].origin.as_str(), kb[0].origin.as_str()),
        ("monitor", "terminal")
    );
    let strip = |mut r: kanbanr_core::manual::ManualResult| {
        r.origin.clear();
        r.at.clear();
        r
    };
    assert_eq!(strip(ka[0].clone()), strip(kb[0].clone()));
    assert_eq!(ka[0].by, "Tester");

    // The list no longer holds it.
    let listed = ureq::get(&format!("{url}/api/projects/demo/manual-tests"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert_eq!(listed.trim(), "[]");
}
