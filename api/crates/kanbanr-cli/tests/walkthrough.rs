//! `kanbanr walkthrough run` end to end with a fake `claude` (FTR-037 W9): the integration tests of
//! R-2, R-8 and R-10. The fake is a script the test writes first; it reads stdin lines and prints
//! canned stream-json: the first turn sets the scratch board up (as Claude's setup would) and asks a
//! question in text, one turn calls the AskUserQuestion hook command itself, one turn exceeds the
//! budget. Everything — the target board, the run folder, the remotes — lives under the system
//! temporary directory with `KANBANR_BOARD_FENCE` set to it (L-41).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kanbanr"))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// The fake `claude`: a Python script (JSON in, JSON out) beside a `kanbanr` link, so the turn that
/// "sets the project up" runs the real CLI as Claude would, under the session's environment.
#[cfg(unix)]
fn fake_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(cli(), bin.join("kanbanr")).unwrap();
    let script = r#"#!/usr/bin/env python3
import json, os, subprocess, sys
def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()
def result(text, cost, error=None):
    r = {"type": "result", "subtype": error or "success", "is_error": bool(error), "session_id": "fake-1",
         "total_cost_usd": cost, "num_turns": 1, "result": text}
    out(r)
def say(text):
    out({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
def run(*args):
    return subprocess.run(["kanbanr", *args], capture_output=True, text=True)
out({"type": "system", "subtype": "init", "session_id": "fake-1", "model": "fake", "tools": ["Bash"]})
cost = 0.0
n = 0
for line in sys.stdin:
    n += 1
    cost += 0.5
    try:
        text = json.loads(line)["message"]["content"]
    except Exception:
        text = ""
    low = text.lower()
    if low.startswith("start using kanbanr"):
        run("init", "shop", "--data-dir", "../board", "--author", "Dev", "--email", "dev@example.com", "--no-hooks")
        run("milestone", "add", "--name", "M1", "--code", "MS-1")
        run("feature", "add", "--title", "Saved basket", "--milestone", "MS-1")
        with open("def.yaml", "w") as f:
            f.write("statement: A shopper's basket survives between visits\nrequirements:\n  - id: R-1\n    text: WHEN a shopper returns THE SYSTEM SHALL show the basket they left\n    tests:\n      - name: 'manual: reopen the page and the item is still there'\n        kind: manual\n")
        run("feature", "define", "FEAT-001", "--file", "def.yaml")
        say("The board is set up.")
        result("Where should the board live?\n1. Beside the project (recommended)\n2. Inside the project\n\nReply with your choice.", cost)
    elif "ask the tool" in low:
        payload = {"tool_name": "AskUserQuestion", "tool_input": {"questions": [
            {"question": "Which colour should the basket page use?", "header": "Colour",
             "options": [{"label": "Red", "description": ""}, {"label": "Blue (Recommended)", "description": ""}]}]}}
        hook = subprocess.run(["kanbanr", "walkthrough", "answer"], input=json.dumps(payload), capture_output=True, text=True)
        say("hook said: " + hook.stdout.strip())
        result("You picked " + (json.loads(hook.stdout)["hookSpecificOutput"]["updatedInput"]["answers"]["Which colour should the basket page use?"] if hook.stdout.strip() else "nothing"), cost)
    elif "spend everything" in low:
        say("Spending.")
        result("Reached max budget", 99.0, "error_max_budget_usd")
    else:
        say("Done: " + text)
        result("Done.", cost)
"#;
    let path = bin.join("claude");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A temporary world: the fence, a target board with the item the runbook's checks name, a
/// project folder pointing at it (where the run is started from), and the runs folder.
struct World {
    root: PathBuf,
    target_board: PathBuf,
    target_project: PathBuf,
    runs: PathBuf,
    claude: PathBuf,
}

#[cfg(unix)]
fn world(name: &str) -> World {
    let root = std::env::temp_dir().join(format!("kanbanr-wt-run-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let target_board = root.join("target-board");
    let target_project = root.join("target-project");
    let runs = root.join("runs");
    std::fs::create_dir_all(&target_project).unwrap();
    std::fs::create_dir_all(&runs).unwrap();
    let k = |args: &[&str]| -> Output {
        let out = Command::new(cli())
            .args(args)
            .current_dir(&target_project)
            .env("KANBANR_BOARD_FENCE", &root)
            .env_remove("KANBANR_DATA_DIR")
            .env_remove("KANBANR_PROJECT")
            .env_remove("CLAUDECODE")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "kanbanr {args:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    };
    Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&target_project)
        .output()
        .unwrap();
    std::fs::write(target_project.join("README.md"), "# target\n").unwrap();
    Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "add", "."])
        .current_dir(&target_project)
        .output()
        .unwrap();
    Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "init"])
        .current_dir(&target_project)
        .output()
        .unwrap();
    let board = target_board.display().to_string();
    k(&["init", "target", "--data-dir", &board, "--author", "Tester", "--email", "t@example.com", "--no-hooks"]);
    k(&["milestone", "add", "--name", "M1", "--code", "MS-1"]);
    k(&["feature", "add", "--title", "One-click decisions", "--milestone", "MS-1"]);
    let def = root.join("target-def.yaml");
    std::fs::write(
        &def,
        "statement: every decision is one click\nrequirements:\n  - id: R-5\n    text: WHEN a walkthrough runs THE SYSTEM SHALL take every decision with one click\n    tests:\n      - name: 'manual: in a walkthrough, every sign-off, promotion and CI result is taken with one click and nothing typed'\n        kind: manual\n",
    )
    .unwrap();
    k(&["feature", "define", "FEAT-001", "--file", &def.display().to_string()]);
    let claude = fake_claude(&root);
    World {
        root,
        target_board,
        target_project,
        runs,
        claude,
    }
}

const RUNBOOK: &str = r#"# test runbook

```yaml
runbook: smoke
process: agile
model: fake
budget_usd: 2
max_minutes: 10
checks:
  MT-1: {item: FEAT-001, requirement: R-5, test: "manual: in a walkthrough, every sign-off, promotion and CI result is taken with one click and nothing typed"}
answers:
  - {match: "where .*board", pick: "Beside the project, with the remote {board_remote}."}
steps:
  - id: "1.1"
    say: "Start using kanbanr for this project."
    expect: {board: {project: exists}}
  - id: "1.2"
    say: "Please ask the tool which colour the basket page should use."
  - id: "2.1"
    decide: {approve: {item: newest}}
    offer: {page: review, button: Approve, subject: {item: newest}}
    check: MT-1
  - id: "2.2"
    push: {repo: code, refspec: main}
STEPS_TAIL```
"#;

fn runbook_file(w: &World, name: &str, tail: &str) -> PathBuf {
    let path = w.root.join(format!("{name}.md"));
    std::fs::write(&path, RUNBOOK.replace("STEPS_TAIL", tail)).unwrap();
    path
}

fn run_walkthrough(w: &World, runbook: &Path, name: &str) -> Output {
    Command::new(cli())
        .args([
            "--data-dir",
            &w.target_board.display().to_string(),
            "--project",
            "target",
            "walkthrough",
            "run",
            &runbook.display().to_string(),
            "--runs",
            &w.runs.display().to_string(),
            "--name",
            name,
            "--claude",
            &w.claude.display().to_string(),
            "--no-browser",
            "--ui-dir",
            &fixtures().join("walkthrough-ui").display().to_string(),
        ])
        .current_dir(&w.target_project)
        .env("KANBANR_BOARD_FENCE", &w.root)
        .env("PATH", format!("{}:{}", w.root.join("bin").display(), std::env::var("PATH").unwrap_or_default()))
        .env_remove("KANBANR_DATA_DIR")
        .env_remove("KANBANR_PROJECT")
        .env_remove("CLAUDECODE")
        .env_remove("KANBANR_WALKTHROUGH_REMOTES")
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn git_status(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// FTR-037 R-2 — `walkthrough: every question asked in a run has one row in questions.tsv`: the
/// question Claude asked in text and the one it asked through the AskUserQuestion hook each have one
/// row, with the source, the options, the pick and the policy; neither stopped the run.
#[cfg(unix)]
#[test]
fn every_question_asked_in_a_run_has_one_row_in_questions_tsv() {
    let w = world("questions");
    let rb = runbook_file(&w, "smoke", "");
    let out = run_walkthrough(&w, &rb, "q1");
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    let run = w.runs.join("q1");
    let tsv = std::fs::read_to_string(run.join("raw/questions.tsv")).unwrap_or_else(|_| panic!("no questions.tsv\n{shown}"));
    let rows: Vec<Vec<&str>> = tsv.lines().skip(1).map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 2, "one row per question:\n{tsv}\n{shown}");
    let by_source = |s: &str| rows.iter().find(|r| r[2] == s).unwrap_or_else(|| panic!("no {s} row in\n{tsv}"));
    let in_text = by_source("text");
    assert_eq!(in_text[1], "1.1");
    assert_eq!(in_text[3], "Where should the board live?");
    assert!(in_text[4].contains("Beside the project (recommended)"), "{tsv}");
    assert!(in_text[5].starts_with("Beside the project, with the remote "), "the runbook's pick, filled in: {tsv}");
    assert!(in_text[6].starts_with("runbook"), "{tsv}");
    let dialog = by_source("dialog");
    assert_eq!(dialog[1], "1.2");
    assert_eq!(dialog[3], "Which colour should the basket page use?");
    assert_eq!(dialog[5], "Blue (Recommended)");
    assert_eq!(dialog[6], "recommended");
    // The hook's answer reached the session: the transcript has it.
    let stream = std::fs::read_to_string(run.join("raw/transcripts/stream.jsonl")).unwrap();
    assert!(stream.contains("You picked Blue (Recommended)"), "{stream}");
    // Nothing stopped: every step ran, the decision went through, the check passed.
    assert!(shown.contains("4 step(s)"), "{shown}");
    let checks = std::fs::read_to_string(run.join("raw/checks.tsv")).unwrap();
    assert!(checks.contains("MT-1\t2.1\ttrue"), "{checks}\n{shown}");
    let findings = std::fs::read_to_string(run.join("raw/findings.md")).unwrap();
    assert!(!findings.contains("question-unanswered"), "{findings}");
    let _ = std::fs::remove_dir_all(&w.root);
}

/// FTR-037 R-8 — `walkthrough: a run past its budget stops within one turn and still snapshots`:
/// the turn that reaches the budget is the last one sent, the stop is a finding, and the captures
/// (manifest, truth, tables) are still made.
#[cfg(unix)]
#[test]
fn a_run_past_its_budget_stops_within_one_turn_and_still_snapshots() {
    let w = world("budget");
    let rb = runbook_file(
        &w,
        "budget",
        "  - id: \"3.1\"\n    say: \"Spend everything.\"\n  - id: \"3.2\"\n    say: \"This is never said.\"\n  - id: \"3.3\"\n    say: \"Nor this.\"\n",
    );
    let out = run_walkthrough(&w, &rb, "b1");
    let shown = text(&out);
    assert!(!out.status.success(), "a stopped run exits non-zero:\n{shown}");
    assert!(shown.contains("stopped early"), "{shown}");
    let run = w.runs.join("b1");
    let stream = std::fs::read_to_string(run.join("raw/transcripts/stream.jsonl")).unwrap();
    let sent: Vec<&str> = stream.lines().filter(|l| l.starts_with("# >>> ")).collect();
    assert!(sent.last().unwrap().contains("Spend everything."), "the budget turn is the last one sent:\n{stream}");
    assert!(!stream.contains("This is never said."), "{stream}");
    assert!(stream.contains("error_max_budget_usd"));
    let findings = std::fs::read_to_string(run.join("raw/findings.md")).unwrap();
    assert!(findings.contains("budget-stop"), "{findings}");
    // The captures are complete.
    assert!(run.join("raw/MANIFEST.sha256").exists(), "{shown}");
    assert!(run.join("raw/truth.txt").exists());
    assert!(run.join("results/decisions.tsv").exists());
    assert!(run.join("results/metrics.md").exists());
    let verify = Command::new(cli())
        .args(["walkthrough", "verify", &run.display().to_string()])
        .current_dir(&w.target_project)
        .env("KANBANR_BOARD_FENCE", &w.root)
        .output()
        .unwrap();
    assert!(verify.status.success(), "{}", text(&verify));
    let setup = std::fs::read_to_string(run.join("raw/setup.log")).unwrap();
    assert!(setup.contains("stopped "), "{setup}");
    let _ = std::fs::remove_dir_all(&w.root);
}

/// FTR-037 R-10 — `walkthrough: a fake-claude run leaves the public board and the kanbanr
/// repository untouched`: after a run, the target board has gained only the prepared batch (its
/// `prepared/` folder), the repository the run was started from has no change, and every push went
/// to the run's own remotes.
#[cfg(unix)]
#[test]
fn a_fake_claude_run_leaves_the_public_board_and_the_kanbanr_repository_untouched() {
    let w = world("untouched");
    let rb = runbook_file(&w, "smoke", "");
    let before_board: Vec<String> = walk(&w.target_board);
    let before_repo = git_status(&w.target_project);
    let head_before = Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&w.target_project).output().unwrap();
    let out = run_walkthrough(&w, &rb, "u1");
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    // The repository the run was started from: untouched.
    assert_eq!(git_status(&w.target_project), before_repo, "{shown}");
    let head_after = Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&w.target_project).output().unwrap();
    assert_eq!(head_before.stdout, head_after.stdout);
    // The target board: only prepared records (and the board's own activity/events/commit of them).
    let after_board: Vec<String> = walk(&w.target_board);
    let new: Vec<&String> = after_board.iter().filter(|f| !before_board.contains(f)).collect();
    assert!(!new.is_empty(), "the batch was prepared:\n{shown}");
    for f in &new {
        assert!(
            f.contains("/prepared/") || f.contains("/activity/") || f.contains("/events/") || f.contains("/.git/"),
            "{f} appeared on the target board\n{shown}"
        );
    }
    let changed: Vec<&String> = before_board
        .iter()
        .filter(|f| !f.contains("/.git/") && !f.contains("/activity/") && !f.contains("/events/"))
        .filter(|f| {
            let p = Path::new(f);
            !p.exists()
        })
        .collect();
    assert!(changed.is_empty(), "files vanished from the target board: {changed:?}");
    let prepared: Vec<&String> = new.iter().copied().filter(|f| f.contains("/prepared/") && f.ends_with(".yaml")).collect();
    assert_eq!(prepared.len(), 1, "one check, one prepared record: {prepared:?}");
    let record = std::fs::read_to_string(prepared[0]).unwrap();
    assert!(record.contains("batch: u1"), "{record}");
    assert!(record.contains("kind: manual-test"), "{record}");
    assert!(record.contains("walkthrough run u1 step 2.1"), "{record}");
    // The item on the target board is as it was: nothing recorded, only prepared.
    let item = std::fs::read_to_string(w.target_board.join("projects/target/features/Planned/FEAT-001.yaml")).unwrap();
    assert!(!item.contains("state: green"), "{item}");
    // Every push went to the run's own remote.
    let pushes = std::fs::read_to_string(w.runs.join("u1/raw/pushes.tsv")).unwrap();
    assert!(pushes.contains("code\tmain\ttrue"), "{pushes}");
    let remotes = std::fs::read_to_string(w.runs.join("u1/remotes.env")).unwrap();
    assert!(remotes.contains(&w.runs.join("u1/remotes/code.git").display().to_string()), "{remotes}");
    let refs = std::fs::read_to_string(w.runs.join("u1/raw/remote-refs.txt")).unwrap();
    assert!(refs.contains("refs/heads/main"), "{refs}");
    let _ = std::fs::remove_dir_all(&w.root);
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn go(dir: &Path, out: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    go(&p, out);
                } else {
                    out.push(p.display().to_string());
                }
            }
        }
    }
    go(dir, &mut out);
    out.sort();
    out
}
