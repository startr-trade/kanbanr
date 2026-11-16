//! The Review page's decision groups through the daemon (FTR-038): a group decision is the
//! person's, posted once from the monitor, and records one verdict per selected row in one commit.

mod common;

use common::{Scratch, board, page_token, serve, status};
use serde_json::{Value, json};
use std::path::Path;

/// A board under the temp dir (L-41) with a design-review gate, two items waiting for approval
/// and one approved item waiting on its design-review sign-off. Returns the data dir, the two
/// approval codes and the sign-off code.
fn waiting_board(s: &Scratch) -> (std::path::PathBuf, Vec<String>, String) {
    use kanbanr_core::config::{Gate, ProjectConfig};
    use kanbanr_core::models::FeatureDefinition;
    let data = board(s);
    let root = std::fs::canonicalize(&data).unwrap();
    assert!(
        root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
        "a test board lives in the temp dir: {}",
        root.display()
    );
    let store = kanbanr_core::Store::new(data.clone());
    let mut config = ProjectConfig::default_for("demo");
    config.gates.insert(
        "Scheduled".into(),
        Gate {
            signoffs: vec!["design-review".into()],
            ..Default::default()
        },
    );
    store.init_project("demo", config).unwrap();
    store
        .add_milestone("demo", "M", "", vec![], Some("M".into()))
        .unwrap();
    let define = |title: &str| {
        let f = store.add_feature("demo", title, "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: format!("{title} is agreed"),
                    ..Default::default()
                }),
            )
            .unwrap();
        f.code
    };
    let approvals = vec![define("One"), define("Two")];
    let signed = define("Signed");
    store.approve_feature("demo", &signed, "Ada").unwrap();
    common::git(&data, &["add", "-A"]);
    common::git(&data, &["commit", "-q", "-m", "board"]);
    (data, approvals, signed)
}

fn commits(data: &Path) -> usize {
    common::git(data, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

/// The rows of group `key` as the page reads them, as `{key, code, rev, spec_rev}` items.
fn items(data: &Path, key: &str) -> Vec<Value> {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    kanbanr_core::reviewgroups::slice(&store, "demo", key, None)
        .unwrap()
        .rows
        .iter()
        .map(|r| json!({"key": r.key, "code": r.code, "rev": r.rev, "spec_rev": r.spec_rev}))
        .collect()
}

/// `POST /api/write/projects/demo/review/decide` from the monitor's own page: the status and the
/// body.
fn decide(url: &str, body: &Value) -> (u16, String) {
    let token = page_token(url);
    match ureq::post(&format!("{url}/api/write/projects/demo/review/decide"))
        .set("x-kanbanr-token", &token)
        .set("Origin", url)
        .set("content-type", "application/json")
        .send_string(&body.to_string())
    {
        Ok(r) => (r.status(), r.into_string().unwrap()),
        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    }
}

/// FTR-038 R-3: end to end through `POST /review/decide` on a write-enabled daemon, two
/// approvals are recorded in one commit, each as the person, stamped as the monitor's and pinned
/// to the revision the page showed; the group is then empty.
#[test]
fn a_group_decision_from_the_monitor_records_each_verdict_as_the_person() {
    let s = Scratch::new("review-groups-decide");
    let (data, approvals, _) = waiting_board(&s);
    let shown = items(&data, "approve");
    assert_eq!(shown.len(), 2, "{shown:?}");
    let url = serve(&s, &data);
    let before = commits(&data);
    let (code, answer) = decide(
        &url,
        &json!({
            "group": "approve", "action": "Approve", "by": "Tester",
            // A request cannot name the origin: the daemon stamps the monitor's.
            "origin": "terminal",
            "items": shown,
        }),
    );
    assert_eq!(code, 200, "{answer}");
    let answer: Value = serde_json::from_str(&answer).unwrap();
    let results = answer["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{answer}");
    assert!(results.iter().all(|r| r.get("error").is_none()), "{answer}");
    assert_eq!(commits(&data), before + 1, "one press, one commit");
    let store = kanbanr_core::Store::new(data.clone());
    let project = store.load("demo").unwrap();
    for (code, item) in approvals.iter().zip(&shown) {
        let a = project
            .feature(code)
            .unwrap()
            .definition
            .clone()
            .unwrap()
            .approval
            .expect("approved");
        assert_eq!(
            (a.by.as_str(), a.origin.as_str(), a.rev.as_str()),
            ("Tester", "monitor", item["rev"].as_str().unwrap())
        );
    }
    let log = common::git(&data, &["log", "-1", "--format=%B"]);
    assert!(log.starts_with("decide approve: Approve (2 of 2)"), "{log}");
    assert!(log.contains("Kanbanr-Origin: monitor"), "{log}");
    assert!(items(&data, "approve").is_empty());
}

/// FTR-038 R-4: an approve request holding a row that waits on a sign-off is refused whole with
/// a 400 and nothing is written; the same request made by Claude (`CLAUDECODE` set, so the
/// conversation's origin) is refused.
#[test]
fn a_group_decision_that_mixes_decisions_or_comes_from_claude_is_refused() {
    let s = Scratch::new("review-groups-refused");
    let (data, approvals, signed) = waiting_board(&s);
    let mut mixed = items(&data, "approve");
    let signoff = items(&data, "design-review");
    assert_eq!(signoff.len(), 1, "{signoff:?}");
    assert_eq!(signoff[0]["code"], Value::String(signed.clone()));
    mixed.extend(signoff);
    let url = serve(&s, &data);
    let before = commits(&data);
    let body = json!({"group": "approve", "action": "Approve", "by": "Tester", "items": mixed});
    let (code, text) = decide(&url, &body);
    assert_eq!(code, 400, "{text}");
    assert!(text.contains(&signed), "{text}");
    assert_eq!(commits(&data), before, "nothing is written");
    let store = kanbanr_core::Store::new(data.clone());
    let project = store.load("demo").unwrap();
    for code in &approvals {
        let def = project.feature(code).unwrap().definition.clone().unwrap();
        assert!(def.approval.is_none(), "{code} was approved");
    }

    // The same request from Claude's shell, through core's one write path.
    let claude = kanbanr_core::Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| "1".to_string()));
    assert_eq!(claude, kanbanr_core::Origin::Conversation);
    let clean = json!({"group": "approve", "action": "Approve", "by": "Claude",
                       "items": items(&data, "approve")});
    let e = kanbanr_core::write::apply_as(
        &store,
        &data,
        kanbanr_core::git::PushPolicy::Off,
        "POST",
        "/projects/demo/review/decide",
        Some(&clean),
        None,
        &kanbanr_core::eventing::NullSender,
        &kanbanr_core::agentscope::ProcessScope::Unscoped,
        claude,
    )
    .unwrap_err();
    assert!(e.to_string().contains("refused"), "{e}");
    assert!(e.to_string().contains("group decision"), "{e}");
    assert_eq!(commits(&data), before, "nothing is written");
    // A read-only route answers the same request with where it is taken.
    let token = page_token(&url);
    let r = ureq::post(&format!("{url}/api/projects/demo/review/decide"))
        .set("x-kanbanr-token", &token)
        .set("Origin", &url)
        .set("content-type", "application/json")
        .send_string(&clean.to_string());
    assert_ne!(status(r), 200);
}
