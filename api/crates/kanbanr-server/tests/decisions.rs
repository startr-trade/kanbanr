//! Delegated decisions come back on the Review page (FTR-028): Claude takes a sign-off under a
//! delegation the person granted, the monitor lists it, and the person keeps one and undoes another.

mod common;

use common::{Scratch, board, page_token, serve};
use serde_json::{Value, json};

/// A write through core's one write path, as the CLI makes it from `origin`.
fn write(
    data: &std::path::Path,
    method: &str,
    path: &str,
    body: Value,
    origin: kanbanr_core::Origin,
) -> Value {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    let out = kanbanr_core::write::apply_as(
        &store,
        data,
        kanbanr_core::git::PushPolicy::Off,
        method,
        path,
        Some(&body),
        None,
        &kanbanr_core::eventing::NullSender,
        &kanbanr_core::agentscope::ProcessScope::Unscoped,
        origin,
    )
    .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
    serde_json::from_str(&out.body).unwrap()
}

fn get(url: &str, path: &str) -> Value {
    let body = ureq::get(&format!("{url}/api{path}"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    serde_json::from_str(&body).unwrap()
}

/// FTR-028 R-3: a delegated sign-off is listed on the Review page's decisions; keeping it removes
/// it from the list; undoing another withdraws that sign-off and records the review.
#[test]
fn delegated_decisions_are_listed_on_the_review_page_to_keep_or_undo() {
    use kanbanr_core::Origin::{Conversation, Terminal};
    let s = Scratch::new("decisions");
    let data = board(&s);
    write(
        &data,
        "POST",
        "/projects",
        json!({"name": "demo"}),
        Terminal,
    );
    let store = kanbanr_core::Store::new(data.clone());
    store
        .add_milestone("demo", "M", "", vec![], Some("M".into()))
        .unwrap();
    let mut codes = Vec::new();
    for title in ["Crash on save", "Typo in help"] {
        let code = store
            .add_feature_of_kind("demo", title, "spec", "M", None, Some("defect"))
            .unwrap()
            .code;
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(kanbanr_core::models::FeatureDefinition {
                    statement: format!("{title}: fixed"),
                    ..Default::default()
                }),
            )
            .unwrap();
        codes.push(code);
    }
    kanbanr_core::git::commit_local(&data, "two defects");

    write(
        &data,
        "PUT",
        "/projects/demo/decisions/mode",
        json!({"mode": "delegated", "by": "Tester"}),
        Terminal,
    );
    // The default end: 14 days from now.
    let until = kanbanr_core::decisions::until_of("", &kanbanr_core::now_rfc3339()).unwrap();
    let proposed = write(
        &data,
        "POST",
        "/projects/demo/decisions/delegations",
        json!({"verdicts": ["signoff:design-review"], "kinds": ["defect"], "until": until}),
        Conversation,
    );
    assert_eq!(proposed["proposed"]["by"], "Claude", "{proposed}");
    let id = proposed["id"].as_str().unwrap().to_string();
    let view: Value = serde_json::from_str(
        &kanbanr_core::dispatch::dispatch(&store, "GET", "/projects/demo/decisions", None).unwrap(),
    )
    .unwrap();
    let rev = view["delegations"][0]["rev"].as_str().unwrap().to_string();
    write(
        &data,
        "POST",
        &format!("/projects/demo/decisions/delegations/{id}/grant"),
        json!({"rev": rev, "by": "Tester"}),
        Terminal,
    );
    // Claude takes both design reviews under the delegation, from its session.
    for code in &codes {
        write(
            &data,
            "POST",
            &format!("/projects/demo/features/{code}/signoff/design-review"),
            json!({"delegation": id, "by": "Tester"}),
            Conversation,
        );
    }

    let url = serve(&s, &data);
    let token = page_token(&url);
    let listed = get(&url, "/projects/demo/decisions");
    assert_eq!(listed["mode"], "delegated");
    let to_review = listed["to_review"].as_array().unwrap().clone();
    assert_eq!(to_review.len(), 2, "{listed}");
    assert!(to_review.iter().all(|d| d["delegation"] == id.as_str()));
    let decision = |code: &str| {
        to_review.iter().find(|d| d["code"] == code).unwrap()["decision"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let post = |body: Value| {
        ureq::post(&format!("{url}/api/write/projects/demo/decisions/reviews"))
            .set("x-kanbanr-token", &token)
            .set("Origin", &url)
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .unwrap_or_else(|e| match e {
                ureq::Error::Status(code, r) => panic!("{code}: {}", r.into_string().unwrap()),
                e => panic!("{e}"),
            })
    };
    post(json!({"decision": decision(&codes[0]), "verdict": "keep", "by": "Tester"}));
    let listed = get(&url, "/projects/demo/decisions");
    let left = listed["to_review"].as_array().unwrap();
    assert_eq!(left.len(), 1, "{listed}");
    assert_eq!(left[0]["code"], codes[1].as_str());

    post(
        json!({"decision": decision(&codes[1]), "verdict": "undo", "reason": "too soon",
                "by": "Tester"}),
    );
    let listed = get(&url, "/projects/demo/decisions");
    assert!(
        listed["to_review"].as_array().unwrap().is_empty(),
        "{listed}"
    );
    let project = store.load("demo").unwrap();
    assert!(
        project
            .feature(&codes[0])
            .unwrap()
            .signoff_current("design-review")
            .is_some(),
        "the kept sign-off stands"
    );
    assert!(
        project
            .feature(&codes[1])
            .unwrap()
            .signoff_current("design-review")
            .is_none(),
        "the undone sign-off is withdrawn"
    );
    let d = kanbanr_core::decisions::load(&store, "demo").unwrap();
    assert_eq!(d.reviews.len(), 2);
    assert_eq!(d.reviews[1].origin, "monitor");
    assert_eq!(d.reviews[1].by, "Tester");
}
