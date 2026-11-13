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
