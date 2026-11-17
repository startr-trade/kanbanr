//! A planning decision is the person's, made on a monitor page (FTR-049): the daemon records it as
//! theirs in one commit, and Claude, through core's one write path, is refused it.

mod common;

use common::{Scratch, board, git, page_token, serve};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A write through core's one write path, as the CLI makes it from `origin`.
fn write(
    data: &Path,
    method: &str,
    path: &str,
    body: Value,
    origin: kanbanr_core::Origin,
) -> Result<kanbanr_core::write::Applied, kanbanr_core::write::WriteError> {
    let store = kanbanr_core::Store::new(data.to_path_buf());
    kanbanr_core::write::apply_as(
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
}

/// A board in the temp folder with sprints and releases on, one item, one sprint and one
/// release, all committed.
fn planning_board(s: &Scratch) -> (PathBuf, String) {
    let data = board(s);
    // The board is the test's own, never one a walk up from the repository would find (L-41).
    assert!(data.starts_with(std::env::temp_dir()));
    let terminal = kanbanr_core::Origin::Terminal;
    write(
        &data,
        "POST",
        "/projects",
        json!({"name": "demo"}),
        terminal,
    )
    .unwrap();
    write(
        &data,
        "PUT",
        "/projects/demo/config/cadence",
        json!({"sprints": true, "releases": true}),
        terminal,
    )
    .unwrap();
    let store = kanbanr_core::Store::new(data.clone());
    store
        .add_milestone("demo", "M", "", vec![], Some("M".into()))
        .unwrap();
    let f = store.add_feature("demo", "Cart", "", "M", None).unwrap();
    kanbanr_core::sprints::add(
        &store,
        "demo",
        "2026-11-16",
        Some(14),
        "",
        None,
        "",
        None,
        None,
    )
    .unwrap();
    kanbanr_core::releases::add(&store, "demo", "v0.9.0", "", "").unwrap();
    git(&data, &["add", "-A"]);
    git(&data, &["commit", "-q", "-m", "fixture"]);
    (data, f.code)
}

fn commits(data: &Path) -> u32 {
    git(data, &["rev-list", "--count", "HEAD"]).parse().unwrap()
}

/// FTR-049 R-1: a planning decision posted on a write-enabled monitor is one commit, kept as
/// `planning/PD-1.yaml` with origin monitor and the board's identity as `by`; it changes no plan.
#[test]
fn a_planning_decision_from_the_monitor_is_recorded_as_the_person_in_one_commit() {
    let s = Scratch::new("planning-monitor");
    let (data, code) = planning_board(&s);
    let project = data.join("projects").join("demo");
    let sprints_before = std::fs::read(project.join("sprints.yaml")).unwrap();
    let before = commits(&data);

    let url = serve(&s, &data);
    let token = page_token(&url);
    let meta: Value = serde_json::from_str(
        &ureq::get(&format!("{url}/api/meta"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    let who = meta["identity"]["name"].as_str().unwrap().to_string();
    assert_eq!(who, "Tester");

    let r = ureq::post(&format!("{url}/api/write/projects/demo/planning"))
        .set("x-kanbanr-token", &token)
        .set("Origin", &url)
        .set("content-type", "application/json")
        // A body that claims another origin is stamped over by the front end.
        .send_string(
            &json!({"action": "into-sprint", "target": "SP-001", "items": [code],
                    "by": who, "origin": "conversation", "note": "before the demo"})
            .to_string(),
        );
    let answer = match r {
        Ok(r) => r.into_string().unwrap(),
        Err(ureq::Error::Status(code, r)) => panic!("{code}: {}", r.into_string().unwrap()),
        Err(e) => panic!("{e}"),
    };
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["id"], "PD-1", "{answer}");

    assert_eq!(commits(&data), before + 1, "one commit");
    // The commit holds the decision and the activity line every write keeps, and nothing else.
    let changed = git(&data, &["show", "--name-only", "--format=", "HEAD"]);
    let changed: Vec<&str> = changed
        .lines()
        .filter(|f| !f.starts_with("projects/demo/activity/"))
        .collect();
    assert_eq!(changed, vec!["projects/demo/planning/PD-1.yaml"]);
    let kept: kanbanr_core::planning::PlanningDecision = kanbanr_core::yaml::from_str(
        &std::fs::read_to_string(project.join("planning").join("PD-1.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            kept.by.as_str(),
            kept.origin.as_str(),
            kept.action.as_str(),
            kept.target.as_str()
        ),
        ("Tester", "monitor", "into-sprint", "SP-001")
    );
    assert_eq!(kept.items, vec![code.clone()]);
    assert_eq!(kept.note, "before the demo");
    // No plan changed: the sprint file is as it was, and the item is in no sprint.
    assert_eq!(
        std::fs::read(project.join("sprints.yaml")).unwrap(),
        sprints_before
    );
    let store = kanbanr_core::Store::new(data.clone());
    let item = store
        .load_meta("demo")
        .unwrap()
        .feature(&code)
        .unwrap()
        .clone();
    assert_eq!(item.sprint, None);

    // The monitor reads it back as pending.
    let listed: Value = serde_json::from_str(
        &ureq::get(&format!("{url}/api/projects/demo/planning?state=pending"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed[0]["decision"]["id"], "PD-1", "{listed}");
    assert_eq!(listed[0]["state"], "pending");
    let check: Value = serde_json::from_str(
        &ureq::get(&format!("{url}/api/projects/demo/planning/PD-1/check"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(check["decision"], "PD-1", "{check}");
}

/// FTR-049 R-2: Claude (the CLI with CLAUDECODE set) is refused the planning route by core's one
/// write path, and nothing is written; declining is not refused.
#[test]
fn claude_cannot_record_a_planning_decision() {
    let s = Scratch::new("planning-claude");
    let (data, code) = planning_board(&s);
    let before = commits(&data);
    let claude = kanbanr_core::Origin::of_cli_in(|k| (k == "CLAUDECODE").then(|| "1".to_string()));
    assert_eq!(claude, kanbanr_core::Origin::Conversation);

    let body =
        json!({"action": "into-sprint", "target": "SP-001", "items": [code], "by": "Claude"});
    let err = match write(
        &data,
        "POST",
        "/projects/demo/planning",
        body.clone(),
        claude,
    ) {
        Ok(out) => panic!("Claude recorded a planning decision: {}", out.body),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("a planning decision is the person's"), "{err}");
    assert_eq!(commits(&data), before, "nothing was committed");
    assert!(
        !data.join("projects/demo/planning").exists(),
        "nothing was written"
    );
    assert_eq!(git(&data, &["status", "--porcelain"]), "");

    // The person's terminal may record it, and Claude may then decline it.
    write(
        &data,
        "POST",
        "/projects/demo/planning",
        json!({"action": "into-sprint", "target": "SP-001", "items": [code], "by": "Tester"}),
        kanbanr_core::Origin::Terminal,
    )
    .unwrap();
    write(
        &data,
        "POST",
        "/projects/demo/planning/PD-1/decline",
        json!({"by": "Claude", "reason": "superseded by PD-2"}),
        claude,
    )
    .unwrap();
    let store = kanbanr_core::Store::new(data.clone());
    let v = kanbanr_core::planning::get(&store, "demo", "PD-1").unwrap();
    assert_eq!(v.state, "declined");
    assert_eq!(v.outcome.unwrap().origin, "conversation");
}
