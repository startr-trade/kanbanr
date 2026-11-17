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

// ---- the reads (T2): summary, a group's slice, one row's evidence ------------------------------

/// `GET {url}/api/projects/{project}{path}`: the status and the body as JSON (`Null` when the body
/// is not JSON, such as an error's text).
fn read(url: &str, project: &str, path: &str) -> (u16, Value) {
    let r = ureq::get(&format!("{url}/api/projects/{project}{path}")).call();
    let (code, body) = match r {
        Ok(r) => (r.status(), r.into_string().unwrap()),
        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    };
    (code, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// Percent-encode everything but the unreserved characters, as the page encodes a row key.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `waiting_board`, plus one approved, signed-off and finished item whose requirement is checked
/// by hand, so the manual group has a row. Returns the data dir, the two approval codes and the
/// manual item's code.
fn board_with_a_manual_check(s: &Scratch) -> (std::path::PathBuf, Vec<String>, String) {
    use kanbanr_core::models::{FeatureDefinition, Requirement, TestRef};
    let (data, approvals, _) = waiting_board(s);
    let store = kanbanr_core::Store::new(data.clone());
    let f = store.add_feature("demo", "Checked", "", "M", None).unwrap();
    store
        .set_feature_definition(
            "demo",
            &f.code,
            Some(FeatureDefinition {
                statement: "Checked by hand".into(),
                requirements: vec![Requirement {
                    text: "THE SYSTEM SHALL show it".into(),
                    tests: vec![TestRef {
                        name: "the person sees it / on a phone".into(),
                        kind: "manual".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }),
        )
        .unwrap();
    store.approve_feature("demo", &f.code, "Ada").unwrap();
    store
        .signoff_feature("demo", &f.code, "design-review", "Ada", "", "")
        .unwrap();
    store.move_feature("demo", &f.code, "Scheduled").unwrap();
    store.move_feature("demo", &f.code, "Completed").unwrap();
    common::git(&data, &["add", "-A"]);
    common::git(&data, &["commit", "-q", "-m", "manual"]);
    (data, approvals, f.code)
}

/// The shape of the generated board: `items` items, each with a definition (never approved, so
/// each waits in the approve group), two requirements and `spec_bytes` of spec; every third one
/// started without agreement, so it also waits in the ratify group.
fn generated_board(
    s: &Scratch,
    items: usize,
    spec_bytes: usize,
) -> (std::path::PathBuf, Vec<String>) {
    let data = board(s);
    let root = std::fs::canonicalize(&data).unwrap();
    assert!(
        root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
        "a test board lives in the temp dir: {}",
        root.display()
    );
    let store = kanbanr_core::Store::new(data.clone());
    store
        .init_project(
            "shop",
            kanbanr_core::config::ProjectConfig::default_for("shop"),
        )
        .unwrap();
    let para = "The customer's cart is kept on the server and restored on the next sign-in. ";
    let mut ops = vec![json!({"op": "milestone.add", "name": "Milestone", "code": "M1"})];
    let codes: Vec<String> = (1..=items).map(|i| format!("FEAT-{i:03}")).collect();
    for (i, code) in codes.iter().enumerate() {
        let spec: String = para.repeat(spec_bytes / para.len() + 1)[..spec_bytes].to_string();
        let requirements: Vec<Value> = (1..=2)
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
            "title": format!("Item {i}: keep the cart when the customer signs in again"),
            "milestone": "M1",
            "spec": spec,
            "definition": {
                "statement": "Customers keep their cart across devices.",
                "requirements": requirements,
            },
        }));
        if i % 3 == 0 {
            ops.push(
                json!({"op": "feature.move", "code": code, "to": "Scheduled",
                            "unapproved": "a generated fixture"}),
            );
        }
    }
    let ops: Vec<kanbanr_core::batch::BatchOp> = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).unwrap())
        .collect();
    store.apply_batch("shop", ops).unwrap();
    common::git(&data, &["add", "-A"]);
    common::git(&data, &["commit", "-q", "-m", "generated"]);
    (data, codes)
}

/// FTR-038 R-1: through the daemon, `GET /review/summary` answers what `reviewgroups::summary`
/// computes — each group once, counted, in the fixed order — and a group emptied by a write is
/// gone from the next read. An unknown group is a 404.
#[test]
fn the_summary_counts_each_group_and_hides_the_empty_ones() {
    let s = Scratch::new("review-groups-summary");
    let (data, approvals, _) = board_with_a_manual_check(&s);
    let url = serve(&s, &data);
    let core = |data: &Path| {
        let store = kanbanr_core::Store::new(data.to_path_buf());
        serde_json::to_value(kanbanr_core::reviewgroups::summary(&store, "demo", None).unwrap())
            .unwrap()
    };
    let (code, summary) = read(&url, "demo", "/review/summary");
    assert_eq!(code, 200, "{summary}");
    assert_eq!(summary, core(&data));
    let keys: Vec<&str> = summary
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["approve", "design-review", "manual"], "{summary}");
    let approve = &summary[0];
    assert_eq!(approve["count"], json!(approvals.len()), "{summary}");
    assert_eq!(approve["action"], json!("Approve"), "{summary}");
    for g in summary.as_array().unwrap() {
        assert!(
            g["count"].as_u64().unwrap() > 0,
            "an empty group is shown: {g}"
        );
    }

    // The group's slice is the same rows the decision takes.
    let (code, slice) = read(&url, "demo", "/review/groups/approve");
    assert_eq!(code, 200, "{slice}");
    let codes: Vec<&str> = slice["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        approvals.iter().map(String::as_str).collect::<Vec<_>>()
    );
    let (code, _) = read(&url, "demo", "/review/groups/nothing");
    assert_eq!(code, 404, "an unknown group is no such route");

    // Approving both empties the group; the next read no longer lists it.
    let (code, answer) = decide(
        &url,
        &json!({"group": "approve", "action": "Approve", "by": "Tester",
                "items": items(&data, "approve")}),
    );
    assert_eq!(code, 200, "{answer}");
    let (code, after) = read(&url, "demo", "/review/summary");
    assert_eq!(code, 200, "{after}");
    assert_eq!(after, core(&data));
    let keys: Vec<&str> = after
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["design-review", "manual"], "{after}");
}

/// FTR-038 R-7: one row's evidence is read alone. `GET /review/groups/approve/rows/<CODE>` answers
/// that item's entry and nothing else; a manual row's key, `<CODE>/<R-n>/<test>`, arrives
/// percent-encoded; and on a generated 400-item board the row answers faster than the summary
/// computes cold, because it reads `review::entry` for one item, never the whole queue.
#[test]
fn a_rows_evidence_is_read_alone() {
    let s = Scratch::new("review-groups-row");
    let (data, approvals, checked) = board_with_a_manual_check(&s);
    let url = serve(&s, &data);
    let (code, row) = read(
        &url,
        "demo",
        &format!("/review/groups/approve/rows/{}", approvals[0]),
    );
    assert_eq!(code, 200, "{row}");
    assert_eq!(row["entry"]["code"], json!(approvals[0]), "{row}");
    for other in ["manual", "adr", "delegated", "delegation"] {
        assert!(row[other].is_null(), "{other} in {row}");
    }
    let key = format!("{checked}/R-1/the person sees it / on a phone");
    let (code, manual) = read(
        &url,
        "demo",
        &format!("/review/groups/manual/rows/{}", encode(&key)),
    );
    assert_eq!(code, 200, "{manual}");
    assert_eq!(manual["manual"]["code"], json!(checked), "{manual}");
    assert_eq!(
        manual["manual"]["test"],
        json!("the person sees it / on a phone"),
        "{manual}"
    );
    let (code, _) = read(
        &url,
        "demo",
        &format!("/review/groups/approve/rows/{checked}"),
    );
    assert_eq!(code, 404, "a row not waiting in the group is no such route");

    let big = Scratch::new("review-groups-row-big");
    let (data, codes) = generated_board(&big, 400, 2_000);
    let store = kanbanr_core::Store::new(data.clone());
    let started = std::time::Instant::now();
    let summary = kanbanr_core::reviewgroups::summary(&store, "shop", None).unwrap();
    let cold = started.elapsed();
    assert_eq!(summary[0].key, "approve");
    assert_eq!(summary[0].count, 400, "{summary:?}");
    let url = serve(&big, &data);
    let started = std::time::Instant::now();
    let (code, row) = read(
        &url,
        "shop",
        &format!("/review/groups/approve/rows/{}", codes[200]),
    );
    let one = started.elapsed();
    assert_eq!(code, 200, "{row}");
    assert_eq!(row["entry"]["code"], json!(codes[200]));
    eprintln!("summary cold {cold:?}; one row through the daemon {one:?}");
    assert!(
        one < cold,
        "one row took {one:?}, the summary cold {cold:?}"
    );
}

/// The median of `n` timed reads of `path`.
fn median_read(url: &str, project: &str, path: &str, n: usize) -> std::time::Duration {
    let mut times: Vec<std::time::Duration> = (0..n)
        .map(|_| {
            let started = std::time::Instant::now();
            let (code, body) = read(url, project, path);
            let took = started.elapsed();
            assert_eq!(code, 200, "{path}: {body}");
            took
        })
        .collect();
    times.sort();
    times[n / 2]
}

/// FTR-038 R-8: on a generated 400-item board, the median of 20 warm reads is under 50 ms for
/// the summary and under 150 ms for the largest group's slice.
#[test]
fn the_summary_and_a_group_slice_meet_their_budgets() {
    let s = Scratch::new("review-groups-budget");
    let (data, _) = generated_board(&s, 400, 2_000);
    let url = serve(&s, &data);
    // The first reads fill the memo; the budget is for warm reads.
    let (code, summary) = read(&url, "shop", "/review/summary");
    assert_eq!(code, 200, "{summary}");
    let largest = summary
        .as_array()
        .unwrap()
        .iter()
        .max_by_key(|g| g["count"].as_u64().unwrap())
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let slice_path = format!("/review/groups/{largest}");
    let (code, slice) = read(&url, "shop", &slice_path);
    assert_eq!(code, 200, "{slice}");
    assert_eq!(slice["rows"].as_array().unwrap().len(), 400);
    let summary_ms = median_read(&url, "shop", "/review/summary", 20);
    let slice_ms = median_read(&url, "shop", &slice_path, 20);
    eprintln!("summary median {summary_ms:?}; {largest} slice median {slice_ms:?}");
    assert!(
        summary_ms < std::time::Duration::from_millis(50),
        "the summary's median warm read is {summary_ms:?}"
    );
    assert!(
        slice_ms < std::time::Duration::from_millis(150),
        "the {largest} slice's median warm read is {slice_ms:?}"
    );
}
