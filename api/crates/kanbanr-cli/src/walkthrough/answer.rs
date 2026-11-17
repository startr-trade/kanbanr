//! Every question Claude asks during a run is answered, never waited on (FTR-037 W5, R-2): from
//! the runbook's rules (the current step's first, then the shared table; never another step's),
//! else the recommended option, else a rule matching the whole message (only for a question with
//! no options), else the first option; a free-text question with no rule gets "Your call; keep it
//! simple". Each answer is logged to `raw/questions.tsv` with the policy
//! that chose it.
//!
//! Two paths bring a question here. The live one under `-p` (the spike of 15 Nov 2026) is text: the
//! driver reads it from the turn's final message and the run answers on the next `say`. The other is
//! the `AskUserQuestion` tool, when a permission host offers it: the PreToolUse hook `kanbanr
//! walkthrough answer` reads the tool call on stdin and answers it with the same policy.

use super::guard::RunDirs;
use super::runbook::{AnswerRule, Runbook};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// The answer a free-text question gets when no rule matches it.
pub const FREE_TEXT_PICK: &str = "Your call; keep it simple";

/// Where a question came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The `AskUserQuestion` tool, through the hook.
    Dialog,
    /// The turn's final text.
    Text,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Dialog => "dialog",
            Source::Text => "text",
        }
    }
}

/// One option Claude offered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QOption {
    pub label: String,
    pub description: String,
}

/// A question, however it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub text: String,
    pub options: Vec<QOption>,
    pub multi: bool,
    pub source: Source,
    /// The whole message the question came in, when it came in text: rules that match nothing in
    /// the question are tried on it.
    pub context: String,
}

impl Question {
    /// A question read from the final text (`driver::TextQuestion`).
    pub fn from_text(q: &super::driver::TextQuestion) -> Question {
        Question {
            text: q.text.clone(),
            options: q
                .options
                .iter()
                .map(|o| QOption {
                    label: o.clone(),
                    description: String::new(),
                })
                .collect(),
            multi: false,
            source: Source::Text,
            context: q.context.clone(),
        }
    }

    /// The questions of an `AskUserQuestion` tool call's input.
    pub fn from_tool_input(input: &Value) -> Vec<Question> {
        input["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|q| Question {
                text: q["question"].as_str().unwrap_or("").to_string(),
                options: q["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| QOption {
                        label: o["label"].as_str().unwrap_or("").to_string(),
                        description: o["description"].as_str().unwrap_or("").to_string(),
                    })
                    .collect(),
                multi: q["multiSelect"].as_bool().unwrap_or(false),
                source: Source::Dialog,
                context: String::new(),
            })
            .collect()
    }
}

/// What was picked, and which policy picked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub pick: String,
    /// `runbook`, `recommended`, `runbook-context`, `first` or `free-text`.
    pub policy: &'static str,
    /// The rule's pattern, when the runbook chose.
    pub rule: Option<String>,
}

/// Is this option marked recommended? Claude writes "(Recommended)" after the label in a dialog
/// and "(recommended)" in text.
pub fn is_recommended(label: &str) -> bool {
    // "(recommended)", "(Recommended)", and "(recommended, since …)" / "(recommended if …)".
    let l = label.to_ascii_lowercase();
    l.contains("(recommended)")
        || l.contains("(recommended,")
        || l.contains("(recommended ")
        || l.contains("(recommended:")
}

/// The answer policy of one run: the runbook's rules, the current step, and the values that fill
/// `{board_remote}`, `{code_remote}`, `{monitor_url}` and `{today}` in a pick.
pub struct Policy<'a> {
    pub runbook: &'a Runbook,
    pub current_step: Option<String>,
    pub vars: BTreeMap<String, String>,
}

impl<'a> Policy<'a> {
    pub fn new(runbook: &'a Runbook, current_step: Option<&str>) -> Policy<'a> {
        Policy {
            runbook,
            current_step: current_step.map(str::to_string),
            vars: BTreeMap::new(),
        }
    }

    pub fn with_vars(mut self, vars: BTreeMap<String, String>) -> Policy<'a> {
        self.vars = vars;
        self
    }

    /// `{name}` placeholders in a pick, filled from the run's values.
    pub fn fill(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (k, v) in &self.vars {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }

    /// The rules in the order they are tried: the current step's, then the shared table. Another
    /// step's rules never answer (preview-7 RC2: step 6.8's "file it as a defect" answered a
    /// question at 2.3 and made a defect nobody asked for).
    fn rules(&self) -> Vec<&'a AnswerRule> {
        let mut out: Vec<&AnswerRule> = Vec::new();
        if let Some(step) = &self.current_step
            && let Some(s) = self.runbook.step(step)
        {
            out.extend(s.answers.iter());
        }
        out.extend(self.runbook.answers.iter());
        out
    }

    /// Answer a question, in this order: a rule matching the question itself (`runbook`); the
    /// recommended option(s) (`recommended`); a rule matching the whole message, only when the
    /// question has no options (`runbook-context`); the first option (`first`); the free-text pick
    /// (`free-text`). The policy is logged with every answer.
    pub fn answer(&self, q: &Question) -> Picked {
        let rules = self.rules();
        if let Some(rule) = rules.iter().find(|r| r.matches(&q.text)) {
            return Picked {
                pick: self.fill(&rule.pick),
                policy: "runbook",
                rule: Some(rule.pattern.clone()),
            };
        }
        // Then the option Claude recommends: a rule that matches only somewhere in the message
        // must not beat it (the second real run answered 42 of 65 questions with a canned line
        // that answered nothing).
        let recommended: Vec<&str> = q
            .options
            .iter()
            .filter(|o| is_recommended(&o.label))
            .map(|o| o.label.as_str())
            .collect();
        if !recommended.is_empty() {
            let pick = if q.multi {
                recommended.join(", ")
            } else {
                recommended[0].to_string()
            };
            return Picked {
                pick,
                policy: "recommended",
                rule: None,
            };
        }
        // A question with no options ("A reply of just 1 is enough"): the message it came in says
        // what it is about.
        if q.options.is_empty()
            && !q.context.is_empty()
            && let Some(rule) = rules.iter().find(|r| r.matches(&q.context))
        {
            return Picked {
                pick: self.fill(&rule.pick),
                policy: "runbook-context",
                rule: Some(rule.pattern.clone()),
            };
        }
        if let Some(first) = q.options.first() {
            return Picked {
                pick: first.label.clone(),
                policy: "first",
                rule: None,
            };
        }
        Picked {
            pick: FREE_TEXT_PICK.to_string(),
            policy: "free-text",
            rule: None,
        }
    }
}

/// One field of `questions.tsv`: no tabs or line breaks.
fn cell(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Append one row to `raw/questions.tsv`: `time step source question options pick policy`.
pub fn log(run: &RunDirs, step: &str, q: &Question, picked: &Picked) -> Result<()> {
    std::fs::create_dir_all(run.raw())?;
    let path = run.raw().join("questions.tsv");
    let new = !path.exists();
    let mut f = super::append_to(&path)?;
    if new {
        writeln!(f, "time\tstep\tsource\tquestion\toptions\tpick\tpolicy")?;
    }
    let options: Vec<String> = q.options.iter().map(|o| cell(&o.label)).collect();
    writeln!(
        f,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        kanbanr_core::now_rfc3339(),
        cell(step),
        q.source.as_str(),
        cell(&q.text),
        options.join(" | "),
        cell(&picked.pick),
        match &picked.rule {
            Some(r) => format!("{} ({})", picked.policy, cell(r)),
            None => picked.policy.to_string(),
        }
    )?;
    Ok(())
}

/// The hook's answer to an `AskUserQuestion` call: allow it, with every question answered.
pub fn hook_output(questions: &Value, answers: &BTreeMap<String, String>) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "questions": questions,
                "answers": answers,
            }
        }
    })
}

/// Answer a PreToolUse payload with the policy: the output to print, and each question with its
/// pick (for the log). A payload that is not an `AskUserQuestion` call gets `None`.
pub fn answer_hook_payload(
    payload: &Value,
    policy: &Policy,
) -> Option<(Value, Vec<(Question, Picked)>)> {
    if payload["tool_name"].as_str() != Some("AskUserQuestion") {
        return None;
    }
    let input = &payload["tool_input"];
    let questions = Question::from_tool_input(input);
    let mut answers = BTreeMap::new();
    let mut picked = Vec::new();
    for q in questions {
        let p = policy.answer(&q);
        answers.insert(q.text.clone(), p.pick.clone());
        picked.push((q, p));
    }
    Some((hook_output(&input["questions"], &answers), picked))
}

/// The file the run writes its current step id to, for the hook.
pub const CURRENT_STEP_FILE: &str = "current-step";
/// The file the run writes its placeholder values to, for the hook.
pub const VARS_FILE: &str = "vars.json";
/// The run's copy of the runbook it follows.
pub const RUNBOOK_FILE: &str = "runbook.md";

/// Note the step a run is at, for the hook.
pub fn set_current_step(run: &RunDirs, step: &str) -> Result<()> {
    std::fs::create_dir_all(run.raw())?;
    kanbanr_core::atomic::write(run.raw().join(CURRENT_STEP_FILE), step)?;
    Ok(())
}

/// Keep the run's placeholder values, for the hook.
pub fn write_vars(run: &RunDirs, vars: &BTreeMap<String, String>) -> Result<()> {
    kanbanr_core::atomic::write(
        run.root.join(VARS_FILE),
        serde_json::to_string_pretty(vars)?,
    )?;
    Ok(())
}

fn read_vars(run: &RunDirs) -> BTreeMap<String, String> {
    std::fs::read_to_string(run.root.join(VARS_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// `kanbanr walkthrough answer`: the hook command. Reads the PreToolUse payload from `stdin`, the
/// run from `KANBANR_WALKTHROUGH_RUN`, answers with the run's policy, logs, and returns what to
/// print (nothing for a call that is not a question, or outside a run).
pub fn run_hook(stdin: &str) -> Result<Option<String>> {
    let Some(run_dir) = std::env::var_os(super::driver::RUN_VAR).map(PathBuf::from) else {
        return Ok(None);
    };
    let run = RunDirs::new(&run_dir);
    let payload: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
    if payload["tool_name"].as_str() != Some("AskUserQuestion") {
        return Ok(None);
    }
    let text = std::fs::read_to_string(run.root.join(RUNBOOK_FILE))
        .with_context(|| format!("no {} in the run folder", RUNBOOK_FILE))?;
    let runbook = Runbook::parse(&text)?;
    let step = std::fs::read_to_string(run.raw().join(CURRENT_STEP_FILE))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let policy = Policy::new(&runbook, (!step.is_empty()).then_some(step.as_str()))
        .with_vars(read_vars(&run));
    let Some((out, picked)) = answer_hook_payload(&payload, &policy) else {
        return Ok(None);
    };
    for (q, p) in &picked {
        log(&run, &step, q, p)?;
    }
    Ok(Some(out.to_string()))
}

/// The error a hook prints when it cannot answer: never a stop for the run, the tool goes on.
pub fn hook_error(e: &anyhow::Error) -> String {
    json!({"systemMessage": format!("kanbanr walkthrough answer could not answer: {e}")})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walkthrough::driver::TextQuestion;
    use crate::walkthrough::runbook::tests::SMALL;

    fn question(text: &str, options: &[&str], source: Source) -> Question {
        Question {
            text: text.into(),
            options: options
                .iter()
                .map(|o| QOption {
                    label: o.to_string(),
                    description: String::new(),
                })
                .collect(),
            multi: false,
            source,
            context: String::new(),
        }
    }

    /// FTR-037 R-2: the runbook's rule answers first (the current step's before the shared table;
    /// another step's never), then the option marked recommended, then the first option; a free-text
    /// question with no rule gets the literal pick; every answer names its policy.
    #[test]
    fn runbook_then_recommended_then_first() {
        let rb = Runbook::parse(SMALL).unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("board_remote".to_string(), "/tmp/x/board.git".to_string());
        let policy = Policy::new(&rb, Some("1.1")).with_vars(vars);
        // The current step's rule.
        let p = policy.answer(&question(
            "Where should the board live?",
            &["Beside the project (Recommended)", "Inside"],
            Source::Dialog,
        ));
        assert_eq!(p.policy, "runbook");
        assert_eq!(p.pick, "../board, with the board remote from remotes.env");
        assert_eq!(p.rule.as_deref(), Some("where .* board"));
        // The shared table, with a placeholder filled.
        let shared = Runbook::parse(
            "```yaml\nrunbook: s\nprocess: p\nanswers:\n  - {match: \"remote\", pick: \"Its remote is {board_remote}.\"}\nsteps:\n  - id: a\n    say: hi\n```",
        )
        .unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("board_remote".to_string(), "/tmp/x/board.git".to_string());
        let p = Policy::new(&shared, Some("a"))
            .with_vars(vars)
            .answer(&question("Which remote?", &[], Source::Text));
        assert_eq!(
            (p.policy, p.pick.as_str()),
            ("runbook", "Its remote is /tmp/x/board.git.")
        );
        // Another step's rule never answers: at 5.8 the question falls through to the first option.
        let p = Policy::new(&rb, Some("5.8")).answer(&question(
            "What is the project for?",
            &["A shop", "A blog"],
            Source::Dialog,
        ));
        assert_eq!((p.policy, p.pick.as_str()), ("first", "A shop"));
        // No rule: the recommended option, wherever it stands.
        let p = policy.answer(&question(
            "Which colour should the basket page use?",
            &["Red", "Blue (Recommended)"],
            Source::Dialog,
        ));
        assert_eq!(
            (p.policy, p.pick.as_str()),
            ("recommended", "Blue (Recommended)")
        );
        // No recommendation: the first.
        let p = policy.answer(&question(
            "Which colour should the basket page use?",
            &["Red", "Blue"],
            Source::Dialog,
        ));
        assert_eq!((p.policy, p.pick.as_str()), ("first", "Red"));
        // Multi-select: every recommended one.
        let mut multi = question(
            "Which checks?",
            &["Lint (Recommended)", "Tests (Recommended)", "Scan"],
            Source::Dialog,
        );
        multi.multi = true;
        let p = policy.answer(&multi);
        assert_eq!(p.pick, "Lint (Recommended), Tests (Recommended)");
        // Free text with no rule.
        let p = policy.answer(&question(
            "What should the shop be called?",
            &[],
            Source::Text,
        ));
        assert_eq!((p.policy, p.pick.as_str()), ("free-text", FREE_TEXT_PICK));
    }

    /// The four final texts of the first real TOGAF run (15 Nov 2026), paths replaced: each is
    /// read as a question, and the runbook's board or purpose rule answers it, never the free-text
    /// fallback. The run answered none of them, and so never made its board.
    #[test]
    fn the_real_togaf_runs_questions_are_detected_and_answered_by_rule() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/walkthrough-texts");
        let mut texts: Vec<serde_json::Value> = Vec::new();
        for f in ["togaf-v070-preview-1.json", "togaf-v070-preview-2.json"] {
            let t: Vec<serde_json::Value> =
                serde_json::from_str(&std::fs::read_to_string(fixture.join(f)).unwrap()).unwrap();
            texts.extend(t);
        }
        let runbook_text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/walkthroughs/togaf.md"),
        )
        .unwrap();
        let rb = Runbook::parse(&runbook_text).unwrap();
        let policy = Policy::new(&rb, Some("1.1"));
        assert!(texts.len() >= 4);
        for t in &texts {
            let text = t["text"].as_str().unwrap();
            let want = t["rule"].as_str().unwrap();
            let q = crate::walkthrough::driver::parse_text_question(text)
                .unwrap_or_else(|| panic!("not read as a question: {text}"));
            let picked = policy.answer(&Question::from_text(&q));
            match want {
                "recommended" | "first" | "free-text" => {
                    assert_eq!(picked.policy, want, "{:?} got {:?}", q.text, picked.pick)
                }
                _ => assert_eq!(picked.rule.as_deref(), Some(want), "{:?}", q.text),
            }
            if t["recommended"].as_bool() == Some(true) {
                assert!(
                    q.options.iter().any(|o| is_recommended(o)),
                    "{:?}",
                    q.options
                );
            }
        }
    }

    /// Preview-5 triage: in both runs Claude asked what to build first during setup, and the run
    /// took Claude's own suggestion (a greeting list). Each such text of those runs (paths and
    /// remotes scrubbed) is now answered by rule: during setup the run defers, and in the step
    /// that says what to build, the step's own rule answers with the guide's words.
    #[test]
    fn the_first_build_question_is_answered_with_the_build_steps_words() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let texts: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("walkthrough-texts/preview-5-first-build.json"))
                .unwrap(),
        )
        .unwrap();
        assert!(texts.len() >= 4);
        for file in ["togaf.md", "agile.md"] {
            let rb = Runbook::parse(
                &std::fs::read_to_string(dir.join("walkthroughs").join(file)).unwrap(),
            )
            .unwrap();
            let build_say = "A shopper opens their basket on our shop's web page, leaves, and comes back the next day to find what they chose still there.";
            for t in &texts {
                let text = t["text"].as_str().unwrap();
                let q = crate::walkthrough::driver::parse_text_question(text)
                    .unwrap_or_else(|| panic!("not read as a question: {text}"));
                let q = Question::from_text(&q);
                let setup = Policy::new(&rb, Some("1.1")).answer(&q);
                assert!(
                    setup.pick.starts_with("Nothing yet"),
                    "{file} 1.1: {:?} -> {:?} ({})",
                    q.text,
                    setup.pick,
                    setup.policy
                );
                let build = Policy::new(&rb, Some("2.1")).answer(&q);
                assert_eq!(build.pick, build_say, "{file} 2.1: {:?}", q.text);
            }
        }
    }

    /// Preview-7 triage RC2: at agile 2.3 Claude asked "how should I record it?" about the
    /// manual test, and step 6.8's own rule ("File it as a defect…") answered it, making a defect
    /// nobody asked for. Now another step's rule never answers: at 2.3 the recommended option
    /// wins, and at 6.8 its rule still does.
    #[test]
    fn another_steps_rule_never_answers_a_question() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let texts: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("walkthrough-texts/agile-v070-preview-4.json"))
                .unwrap(),
        )
        .unwrap();
        let text = texts[0]["text"].as_str().unwrap();
        let rb =
            Runbook::parse(&std::fs::read_to_string(dir.join("walkthroughs/agile.md")).unwrap())
                .unwrap();
        let q = crate::walkthrough::driver::parse_text_question(text).unwrap();
        assert_eq!(q.text, "Question: how should I record it?");
        let q = Question::from_text(&q);
        let at_2_3 = Policy::new(&rb, Some("2.3")).answer(&q);
        assert_eq!(at_2_3.policy, "recommended", "{at_2_3:?}");
        assert!(
            at_2_3
                .pick
                .starts_with("Your check becomes the item's one manual test"),
            "{at_2_3:?}"
        );
        assert!(!at_2_3.pick.contains("defect"));
        let defect_step = rb
            .steps
            .iter()
            .find(|s| s.names.contains_key("defect"))
            .unwrap();
        let at_defect = Policy::new(&rb, Some(&defect_step.id)).answer(&q);
        assert!(
            at_defect.pick.starts_with("File it as a defect"),
            "{at_defect:?}"
        );
    }

    /// Preview-7 triage RC5: the agile run's real wordings ("what is the page built with?", "which
    /// test runner…") take the stack rule; the UI question gets "It has a screen." and nothing
    /// about the look, which its own rule answers when asked; and at 6.1 the pipeline-capability
    /// question keeps the separate, rerunnable stages and caching.
    #[test]
    fn the_stack_ui_style_and_pipeline_questions_take_their_rules() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for file in ["togaf.md", "agile.md"] {
            let rb = Runbook::parse(
                &std::fs::read_to_string(dir.join("walkthroughs").join(file)).unwrap(),
            )
            .unwrap();
            let ask = |step: &str, text: &str, options: &[&str]| {
                Policy::new(&rb, Some(step)).answer(&question(text, options, Source::Text))
            };
            for q in [
                "What is the page built with?",
                "Which test runner should the end-to-end tests use?",
                "Which language should the server use?",
            ] {
                let p = ask("3.2", q, &["Plain HTML and Node (recommended)", "Python"]);
                assert!(
                    p.pick.starts_with("Python 3 standard library"),
                    "{file} {q}: {p:?}"
                );
            }
            let ui = ask("2.1", "Does the product have a user interface?", &[]);
            assert_eq!(ui.pick, "It has a screen.", "{file}");
            let style = ask(
                "3.2",
                "How should the basket page be styled?",
                &[
                    "A utility-class framework (recommended)",
                    "Hand-written CSS",
                ],
            );
            assert!(
                style.pick.starts_with("Hand-written CSS"),
                "{file}: {style:?}"
            );
        }
        let rb =
            Runbook::parse(&std::fs::read_to_string(dir.join("walkthroughs/agile.md")).unwrap())
                .unwrap();
        let p = Policy::new(&rb, Some("6.1")).answer(&question(
            "Which of these can your plan and settings run?",
            &["Format and lint", "Test", "Build"],
            Source::Text,
        ));
        assert!(
            p.pick.contains("rerun") && p.pick.contains("caching"),
            "{p:?}"
        );
    }

    /// Preview-6 triage: the TOGAF run stopped at setup after 4 turns. Its last message, the
    /// charter draft, asked "**Confirm this draft as it is, or tell me what to change or add.**"
    /// with no `?`, so the turn read as settled. Every final text of that run (scrubbed) now reads
    /// as a question, and the charter draft is answered by the runbook's confirm rule.
    #[test]
    fn a_request_without_a_question_mark_is_asked_and_the_charter_draft_confirmed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let texts: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("walkthrough-texts/togaf-v070-preview-6.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(texts.len(), 4);
        let rb =
            Runbook::parse(&std::fs::read_to_string(dir.join("walkthroughs/togaf.md")).unwrap())
                .unwrap();
        let policy = Policy::new(&rb, Some("1.1"));
        for t in &texts {
            let text = t["text"].as_str().unwrap();
            assert!(
                crate::walkthrough::driver::parse_text_question(text).is_some(),
                "settled, but it asks: {text}"
            );
        }
        let charter = texts.last().unwrap()["text"].as_str().unwrap();
        assert!(!charter.contains('?'));
        let q = crate::walkthrough::driver::parse_text_question(charter).unwrap();
        assert_eq!(
            q.text,
            "Confirm this draft as it is, or tell me what to change or add."
        );
        let picked = policy.answer(&Question::from_text(&q));
        assert_eq!(picked.pick, "I confirm it as drafted.", "{picked:?}");
        assert!(
            picked
                .rule
                .as_deref()
                .is_some_and(|r| r.starts_with("confirm (this|the)")),
            "{picked:?}"
        );
        // A conditional offer at the end of a settled turn still asks nothing.
        for settled in [
            "The board is set up. Let me know if you want changes.",
            "DLG-001 is proposed on the Review page. Tell me when you've decided.",
            "The draft is confirmed.",
        ] {
            assert!(
                crate::walkthrough::driver::parse_text_question(settled).is_none(),
                "{settled}"
            );
        }
    }

    /// FTR-037 R-2: a question asked in text is parsed into the same `Question` and answered by the
    /// same policy as a dialog; both are logged as one row of `questions.tsv` naming the source,
    /// the options, the pick and the policy; and the hook answers an AskUserQuestion payload with
    /// `permissionDecision: allow` and every question's pick.
    #[test]
    fn a_question_in_text_is_answered_like_a_dialog() {
        let rb = Runbook::parse(SMALL).unwrap();
        let policy = Policy::new(&rb, Some("1.1"));
        let text = TextQuestion {
            text: "Which colour should the basket page use?".into(),
            options: vec!["Red".into(), "Blue (recommended)".into()],
            context: String::new(),
        };
        let as_text = Question::from_text(&text);
        assert_eq!(as_text.source, Source::Text);
        let dialog = question(
            "Which colour should the basket page use?",
            &["Red", "Blue (recommended)"],
            Source::Dialog,
        );
        let a = policy.answer(&as_text);
        let b = policy.answer(&dialog);
        assert_eq!(a.pick, b.pick);
        assert_eq!(a.policy, "recommended");
        // The log: one row each, same shape.
        let dir = std::env::temp_dir().join(format!("kanbanr-wt-answer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = RunDirs::new(&dir);
        run.make().unwrap();
        log(&run, "1.1", &as_text, &a).unwrap();
        log(&run, "1.1", &dialog, &b).unwrap();
        let tsv = std::fs::read_to_string(run.raw().join("questions.tsv")).unwrap();
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(
            lines[0],
            "time\tstep\tsource\tquestion\toptions\tpick\tpolicy"
        );
        assert_eq!(lines.len(), 3);
        let row: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(row[1], "1.1");
        assert_eq!(row[2], "text");
        assert_eq!(row[3], "Which colour should the basket page use?");
        assert_eq!(row[4], "Red | Blue (recommended)");
        assert_eq!(row[5], "Blue (recommended)");
        assert_eq!(row[6], "recommended");
        assert_eq!(lines[2].split('\t').nth(2), Some("dialog"));
        // The hook path.
        let payload = json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [
                {"question": "Where should the board live?", "header": "Board",
                 "options": [{"label": "Beside", "description": "../board"}, {"label": "Inside", "description": ""}]},
                {"question": "Which colour?", "options": [{"label": "Red"}, {"label": "Blue (Recommended)"}]}
            ]}
        });
        let (out, picked) = answer_hook_payload(&payload, &policy).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "allow");
        let answers = &out["hookSpecificOutput"]["updatedInput"]["answers"];
        assert_eq!(
            answers["Where should the board live?"],
            "../board, with the board remote from remotes.env"
        );
        assert_eq!(answers["Which colour?"], "Blue (Recommended)");
        assert_eq!(
            out["hookSpecificOutput"]["updatedInput"]["questions"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].0.source, Source::Dialog);
        // Not a question: nothing to say.
        assert!(answer_hook_payload(&json!({"tool_name": "Bash"}), &policy).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
