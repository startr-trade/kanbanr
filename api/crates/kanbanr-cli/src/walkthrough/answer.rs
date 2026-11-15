//! Every question Claude asks during a run is answered, never waited on (FTR-037 W5, R-2): from
//! the runbook's rules (the current step's first, then the shared table, then the other steps' in
//! order), else the recommended option, else the first option; a free-text question with no rule
//! gets "Your call; keep it simple". Each answer is logged to `raw/questions.tsv` with the policy
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
            })
            .collect()
    }
}

/// What was picked, and which policy picked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub pick: String,
    /// `runbook`, `recommended`, `first` or `free-text`.
    pub policy: &'static str,
    /// The rule's pattern, when the runbook chose.
    pub rule: Option<String>,
}

/// Is this option marked recommended? Claude writes "(Recommended)" after the label in a dialog
/// and "(recommended)" in text.
pub fn is_recommended(label: &str) -> bool {
    label.to_ascii_lowercase().contains("(recommended)")
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

    /// The rules in the order they are tried: the current step's, the shared table, every other
    /// step's in step order.
    fn rules(&self) -> Vec<&'a AnswerRule> {
        let mut out: Vec<&AnswerRule> = Vec::new();
        if let Some(step) = &self.current_step
            && let Some(s) = self.runbook.step(step)
        {
            out.extend(s.answers.iter());
        }
        out.extend(self.runbook.answers.iter());
        for s in &self.runbook.steps {
            if self.current_step.as_deref() != Some(s.id.as_str()) {
                out.extend(s.answers.iter());
            }
        }
        out
    }

    /// Answer a question: the first matching rule, else the recommended option(s), else the first,
    /// else the free-text pick.
    pub fn answer(&self, q: &Question) -> Picked {
        if let Some(rule) = self.rules().into_iter().find(|r| r.matches(&q.text)) {
            return Picked {
                pick: self.fill(&rule.pick),
                policy: "runbook",
                rule: Some(rule.pattern.clone()),
            };
        }
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
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
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
    std::fs::write(run.raw().join(CURRENT_STEP_FILE), step)?;
    Ok(())
}

/// Keep the run's placeholder values, for the hook.
pub fn write_vars(run: &RunDirs, vars: &BTreeMap<String, String>) -> Result<()> {
    std::fs::write(
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
        }
    }

    /// FTR-037 R-2: the runbook's rule answers first (the current step's before the shared table
    /// and the other steps'), then the option marked recommended, then the first option; a free-text
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
        // Another step's rule answers too, when the current step has none.
        let p = Policy::new(&rb, Some("5.8")).answer(&question(
            "What is the project for?",
            &["A shop", "A blog"],
            Source::Dialog,
        ));
        assert_eq!(
            (p.policy, p.pick.as_str()),
            ("runbook", "A small web shop.")
        );
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
