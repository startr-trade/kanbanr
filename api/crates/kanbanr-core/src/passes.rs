//! One-time passes for decisions taken in the conversation (FTR-035).
//!
//! In `conversation` mode Claude asks the person for a verdict with the question
//! `kanbanr decisions question` prints, which ends in a tag: the verdict command itself, in
//! brackets. Claude Code's PostToolUse hook on AskUserQuestion (`kanbanr decisions heard`) reads
//! the person's answer from `tool_response.answers`, which Claude Code writes from the choice made
//! in its own UI, and when that answer is the approving option writes a pass: a ticket matched to
//! the project, item, verdict and revision, which lasts [`TTL_MINUTES`] and is used once.
//!
//! A pass is not board data. It lives under the board's git dir (`<git dir>/kanbanr/passes/`),
//! outside the worktree, so `git status` never sees it and no commit carries one; what it carried
//! is kept on the verdict record as `heard`. Expired and used passes are deleted.

use crate::store::Store;
use crate::{CoreError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How long a pass lasts, in minutes (the definition's number).
pub const TTL_MINUTES: i64 = 15;
/// Where passes live, under the board's git dir.
pub const DIR: &str = "kanbanr/passes";
/// The longest question a pass is made for.
const MAX_QUESTION: usize = 2_000;
/// The most of a session transcript [`model_call`] reads, from its end.
const TRANSCRIPT_TAIL: u64 = 4 * 1024 * 1024;

/// A question's tag, read by [`parse_tag`]: the verdict command the pass stands in for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tag {
    /// `approve`, `ratify` or `signoff:<name>`.
    pub verdict: String,
    pub code: String,
    /// The sign-off's name; empty for `approve` and `ratify`.
    pub signoff: String,
    pub rev: String,
    pub project: String,
}

fn is_code(s: &str) -> bool {
    let Some((prefix, number)) = s.split_once('-') else {
        return false;
    };
    let mut chars = prefix.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

fn is_rev(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn is_project(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The tag of a question: the **last** `[kanbanr …]` bracket group, read as whitespace-separated
/// tokens. Exactly `kanbanr approve <CODE> --rev <REV> --project <P>`, the same with `ratify`, or
/// `kanbanr signoff <CODE> <NAME> --rev <REV> --project <P>`; anything else is `None` (L-23:
/// tokens with boundaries, never substrings).
pub fn parse_tag(question: &str) -> Option<Tag> {
    let mut last: Option<&str> = None;
    let mut rest = question;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        let group = &after[..close];
        // A nested '[' starts a new group: only the innermost one counts.
        let group = group.rsplit('[').next().unwrap_or(group);
        if group.split_whitespace().next() == Some("kanbanr") {
            last = Some(group);
        }
        rest = &after[close + 1..];
    }
    let tokens: Vec<&str> = last?.split_whitespace().collect();
    let (verdict, code, signoff, tail) = match tokens.as_slice() {
        ["kanbanr", v @ ("approve" | "ratify"), code, tail @ ..] => {
            (v.to_string(), *code, String::new(), tail)
        }
        ["kanbanr", "signoff", code, name, tail @ ..] if !name.starts_with('-') => {
            (format!("signoff:{name}"), *code, name.to_string(), tail)
        }
        _ => return None,
    };
    let ["--rev", rev, "--project", project] = tail else {
        return None;
    };
    (is_code(code) && is_rev(rev) && is_project(project)).then(|| Tag {
        verdict,
        code: code.to_string(),
        signoff,
        rev: rev.to_string(),
        project: project.to_string(),
    })
}

/// The bracket group for `t`: the inverse of [`parse_tag`].
pub fn tag_text(t: &Tag) -> String {
    let (word, name) = match t.verdict.strip_prefix("signoff:") {
        Some(name) => ("signoff", format!(" {name}")),
        None => (t.verdict.as_str(), String::new()),
    };
    format!(
        "[kanbanr {word} {}{name} --rev {} --project {}]",
        t.code, t.rev, t.project
    )
}

/// The option label that makes a pass for `verdict`, compared exactly after trimming.
pub fn approving_label(verdict: &str) -> &'static str {
    match verdict {
        "approve" => "Approve",
        "ratify" => "Ratify",
        _ => "Sign off",
    }
}

/// A tagged question the person answered with its approving option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub tag: Tag,
    pub question: String,
    pub answer: String,
    pub tool_use_id: String,
    pub session_id: String,
}

/// What the hook heard in one payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listened {
    /// Nothing for kanbanr, with why (for the debug log).
    Quiet(String),
    /// Tagged questions the person answered with the approving option, and beside them the ones
    /// they answered with anything else (empty when none), each in payload order.
    Answered(Vec<Answered>, Vec<(Tag, String)>),
    /// Tagged questions answered with anything else, with the answer ("" when none), when the
    /// payload holds no approved question.
    Declined(Vec<(Tag, String)>),
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The agent's own call for `tool_use_id`, as the session transcript records it (DEF-126): the
/// `input` of the assistant `tool_use` block with that `id`, or `None` when the transcript holds
/// none. Reads at most the last [`TRANSCRIPT_TAIL`] bytes (the call is the transcript's latest tool
/// use), searching from the end; a line that does not parse is skipped.
pub fn model_call(transcript: &Path, tool_use_id: &str) -> Result<Option<Value>> {
    let mut file = std::fs::File::open(transcript)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(TRANSCRIPT_TAIL).read_to_end(&mut bytes)?;
    let tail = String::from_utf8_lossy(&bytes);
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if text(&entry, "type") != "assistant" {
            continue;
        }
        let Some(content) = entry
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        if let Some(block) = content
            .iter()
            .find(|b| text(b, "type") == "tool_use" && text(b, "id") == tool_use_id)
        {
            return Ok(Some(block.get("input").cloned().unwrap_or(Value::Null)));
        }
    }
    Ok(None)
}

/// Read a PostToolUse payload. Pure. `Quiet` unless it is a PostToolUse on AskUserQuestion, from
/// no sub-agent, whose `model_input` (the agent's own call, from [`model_call`]) was found and
/// carried no answers (a question answered before the person saw it is not the person's; Claude
/// Code itself writes the person's choice into the payload's `tool_input.answers`, DEF-126), and
/// with at least one single-select tagged question. A question whose trimmed
/// answer is its tag's approving label, offered among its options, is `Answered`; any other
/// answer, or none, is declined. When a payload holds both, both are given, in payload order within
/// each list (DEF-110).
pub fn listen(payload: &Value, model_input: Option<&Value>) -> Listened {
    if text(payload, "hook_event_name") != "PostToolUse" {
        return Listened::Quiet("not a PostToolUse payload".into());
    }
    if text(payload, "tool_name") != "AskUserQuestion" {
        return Listened::Quiet("not an AskUserQuestion call".into());
    }
    if !text(payload, "agent_id").trim().is_empty() {
        return Listened::Quiet("asked by a sub-agent, which never takes a verdict".into());
    }
    let Some(model_input) = model_input else {
        return Listened::Quiet(
            "the agent's own call was not found in the session transcript".into(),
        );
    };
    match model_input.get("answers") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) if m.is_empty() => {}
        Some(_) => {
            return Listened::Quiet(
                "the question was answered before the person saw it (the agent's own call carried answers)"
                    .into(),
            );
        }
    }
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    let response = payload.get("tool_response").unwrap_or(&Value::Null);
    let questions = response
        .get("questions")
        .and_then(Value::as_array)
        .or_else(|| input.get("questions").and_then(Value::as_array));
    let Some(questions) = questions else {
        return Listened::Quiet("no questions in the payload".into());
    };
    let answers = response.get("answers");
    let (tool_use_id, session_id) = (text(payload, "tool_use_id"), text(payload, "session_id"));
    let (mut answered, mut declined) = (Vec::new(), Vec::new());
    for q in questions {
        let question = text(q, "question");
        let Some(tag) = parse_tag(question) else {
            continue;
        };
        if q.get("multiSelect").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let answer = answers
            .and_then(|a| a.get(question))
            .and_then(Value::as_str)
            .unwrap_or("");
        let label = approving_label(&tag.verdict);
        let offered = q
            .get("options")
            .and_then(Value::as_array)
            .is_some_and(|os| os.iter().any(|o| text(o, "label") == label));
        if answer.trim() == label && offered {
            answered.push(Answered {
                tag,
                question: question.to_string(),
                answer: label.to_string(),
                tool_use_id: tool_use_id.to_string(),
                session_id: session_id.to_string(),
            });
        } else {
            declined.push((tag, answer.to_string()));
        }
    }
    if !answered.is_empty() {
        Listened::Answered(answered, declined)
    } else if !declined.is_empty() {
        Listened::Declined(declined)
    } else {
        Listened::Quiet("no single-select tagged question".into())
    }
}

/// What Claude is told of a tagged question the person did not approve (`answer` is "" when they
/// gave none).
pub fn declined_line(tag: &Tag, answer: &str) -> String {
    let heard = if answer.is_empty() {
        "they gave no answer".to_string()
    } else {
        format!("they answered: \"{answer}\"")
    };
    format!(
        "kanbanr: the person did not choose {} for {} ({heard}); nothing is recorded, nothing is \
         pending",
        approving_label(&tag.verdict),
        tag.code
    )
}

/// A one-time pass, one JSON file under [`dir`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pass {
    pub id: String,
    pub project: String,
    pub code: String,
    /// `approve`, `ratify` or `signoff:<name>`.
    pub verdict: String,
    pub rev: String,
    pub question: String,
    pub answer: String,
    pub tool_use_id: String,
    pub session_id: String,
    pub at: String,
    pub expires: String,
}

/// The folder passes live in: `<git dir>/kanbanr/passes`.
pub fn dir(store: &Store) -> PathBuf {
    crate::git::git_dir(store.data_dir()).join(DIR)
}

fn instant(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s.trim(), &Rfc3339).ok()
}

/// Has `expires` been reached at `now`? A value that is not a time has.
fn expired(expires: &str, now: &str) -> bool {
    match (instant(expires), instant(now)) {
        (Some(e), Some(n)) => e <= n,
        _ => true,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Make a pass for an answer the hook heard. `Ok(Err(why))` is a quiet refusal the hook reports
/// as context, never an error: the project is not in conversation mode, the item does not exist,
/// a sign-off no gate asks for, a ratify with nothing to ratify, a revision that is not the
/// definition's now, an over-long question, or a pass for this answer already made.
pub fn issue(store: &Store, a: &Answered, now: &str) -> Result<std::result::Result<Pass, String>> {
    let tag = &a.tag;
    let mode = crate::decisions::load(store, &tag.project)?.mode();
    if mode != crate::decisions::Mode::Conversation {
        return Ok(Err(format!(
            "project {} is not in conversation mode (mode: {})",
            tag.project,
            mode.as_str()
        )));
    }
    let project = match store.load(&tag.project) {
        Ok(p) => p,
        Err(CoreError::ProjectNotFound(_)) => {
            return Ok(Err(format!("there is no project {}", tag.project)));
        }
        Err(e) => return Err(e),
    };
    let Ok(feature) = project.feature(&tag.code) else {
        return Ok(Err(format!(
            "there is no item {} in project {}",
            tag.code, tag.project
        )));
    };
    if tag.verdict.starts_with("signoff:") {
        let gates = project.config.effective_gates();
        if !gates
            .values()
            .any(|g| g.signoffs.iter().any(|x| x == &tag.signoff))
        {
            return Ok(Err(format!(
                "no gate of project {} asks for sign-off `{}`",
                tag.project, tag.signoff
            )));
        }
    }
    if tag.verdict == "ratify" && !crate::doctor::awaiting_ratification(feature) {
        return Ok(Err(format!("{} has nothing to ratify", tag.code)));
    }
    let Some(definition) = feature.definition.as_ref() else {
        return Ok(Err(format!("{} has no definition", tag.code)));
    };
    if definition.content_rev() != tag.rev {
        return Ok(Err(format!(
            "the definition changed since you asked: {} is at rev {} now, the question names {}; \
             ask again",
            tag.code,
            definition.content_rev(),
            tag.rev
        )));
    }
    if a.question.chars().count() > MAX_QUESTION {
        return Ok(Err(format!(
            "the question is longer than {MAX_QUESTION} characters"
        )));
    }
    let Some(at) = instant(now) else {
        return Ok(Err(format!("`{now}` is not a time")));
    };
    let expires = (at + time::Duration::minutes(TTL_MINUTES))
        .format(&Rfc3339)
        .map_err(|e| CoreError::Unsupported(e.to_string()))?;
    sweep(store, now);
    let hash = crate::hash::stable_hash(&format!("{}\n{}\n{now}", a.tool_use_id, a.question));
    let pass = Pass {
        id: format!("P-{}", &hash[..12]),
        project: tag.project.clone(),
        code: tag.code.clone(),
        verdict: tag.verdict.clone(),
        rev: tag.rev.clone(),
        question: a.question.clone(),
        answer: a.answer.clone(),
        tool_use_id: a.tool_use_id.clone(),
        session_id: a.session_id.clone(),
        at: now.to_string(),
        expires,
    };
    let folder = dir(store);
    std::fs::create_dir_all(&folder)?;
    let file = folder.join(format!("{}.json", pass.id));
    if file.exists() {
        return Ok(Err(format!(
            "pass {} was already made for this answer",
            pass.id
        )));
    }
    let bytes =
        serde_json::to_vec_pretty(&pass).map_err(|e| CoreError::Unsupported(e.to_string()))?;
    crate::atomic::write(&file, bytes)?;
    Ok(Ok(pass))
}

/// The pass `id`, when its file exists, parses and has not expired at `now`. An expired file is
/// deleted on the way. An id with any character outside `[A-Za-z0-9-]` is `None` before any path
/// is built.
pub fn load(store: &Store, id: &str, now: &str) -> Option<Pass> {
    if !valid_id(id) {
        return None;
    }
    let file = dir(store).join(format!("{id}.json"));
    let pass: Pass = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
    if expired(&pass.expires, now) {
        let _ = std::fs::remove_file(&file);
        return None;
    }
    (pass.id == id).then_some(pass)
}

/// Use the pass `id` up: its file goes. A pass already gone is fine.
pub fn consume(store: &Store, id: &str) -> std::io::Result<()> {
    if !valid_id(id) {
        return Ok(());
    }
    match std::fs::remove_file(dir(store).join(format!("{id}.json"))) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Every pass file, with what it holds (`None` when it does not parse).
fn files(store: &Store) -> Vec<(PathBuf, Option<Pass>)> {
    let Ok(entries) = std::fs::read_dir(dir(store)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let pass = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Pass>(&b).ok());
            (p, pass)
        })
        .collect()
}

/// Remove every pass of `project`; returns how many went.
pub fn clear(store: &Store, project: &str) -> usize {
    files(store)
        .into_iter()
        .filter(|(_, p)| p.as_ref().is_some_and(|p| p.project == project))
        .filter(|(path, _)| std::fs::remove_file(path).is_ok())
        .count()
}

/// Remove expired and unreadable pass files, best effort.
pub fn sweep(store: &Store, now: &str) {
    for (path, pass) in files(store) {
        if pass.is_none_or(|p| expired(&p.expires, now)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The one line the hook hands Claude for a pass it made; `command` is the verdict command that
/// uses it.
pub fn context_line(p: &Pass, command: &str) -> String {
    let at = p.at.get(11..16).unwrap_or(&p.at);
    format!(
        "kanbanr: the person chose {} for {} at {at}; it is theirs to record once, within \
         {TTL_MINUTES} minutes: `{command}`",
        approving_label(&p.verdict),
        p.code
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Gate, ProjectConfig};
    use crate::decisions::{Decisions, Mode, ModeChange};
    use crate::models::FeatureDefinition;
    use serde_json::json;

    const NOW: &str = "2026-11-15T10:15:00Z";
    const REV: &str = "3f9a1c2d8e7b4a60";

    #[test]
    fn a_tag_is_read_by_its_tokens_and_nothing_else() {
        let approve = parse_tag(
            "Approve FTR-35 \"Cart\" 's definition as it stands (rev 3f9a1c2d)? \
             [kanbanr approve FTR-35 --rev 3f9a1c2d8e7b4a60 --project demo]",
        )
        .expect("approve parses");
        assert_eq!(
            approve,
            Tag {
                verdict: "approve".into(),
                code: "FTR-35".into(),
                signoff: String::new(),
                rev: REV.into(),
                project: "demo".into(),
            }
        );
        let ratify = parse_tag("[kanbanr ratify DEF-2 --rev 0123abcd --project my.proj_1-x]")
            .expect("ratify parses");
        assert_eq!(
            (ratify.verdict.as_str(), ratify.project.as_str()),
            ("ratify", "my.proj_1-x")
        );
        let signoff =
            parse_tag("Sign off? [kanbanr signoff F-1 design-review --rev 0123abcd --project p]")
                .expect("signoff parses");
        assert_eq!(signoff.verdict, "signoff:design-review");
        assert_eq!(signoff.signoff, "design-review");

        for t in [&approve, &ratify, &signoff] {
            assert_eq!(parse_tag(&tag_text(t)).as_ref(), Some(t), "round trip");
        }

        for bad in [
            // extra words
            "[kanbanr approve F-1 --rev 0123abcd --project p please]",
            "[kanbanr approve F-1 now --rev 0123abcd --project p]",
            // a lower-case code
            "[kanbanr approve f-1 --rev 0123abcd --project p]",
            "[kanbanr approve Ftr-1 --rev 0123abcd --project p]",
            // a rev of 7 characters, and one in upper case
            "[kanbanr approve F-1 --rev 0123abc --project p]",
            "[kanbanr approve F-1 --rev 0123ABCD --project p]",
            // a missing --project
            "[kanbanr approve F-1 --rev 0123abcd]",
            // flags out of order
            "[kanbanr approve F-1 --project p --rev 0123abcd]",
            // a signoff without its name
            "[kanbanr signoff F-1 --rev 0123abcd --project p]",
            // another verdict
            "[kanbanr withdraw F-1 --rev 0123abcd --project p]",
            // a bracket in prose that is not a tag
            "Approve F-1 [as discussed] now?",
            "kanbanr approve F-1 --rev 0123abcd --project p",
            "",
        ] {
            assert_eq!(parse_tag(bad), None, "{bad:?} is not a tag");
        }

        // Two groups: only the last kanbanr group counts.
        assert_eq!(
            parse_tag(
                "[kanbanr approve F-9 --rev 0123abcd --project q] or rather \
                 [kanbanr approve F-1 --rev 0123abcd --project p]?"
            )
            .map(|t| t.code),
            Some("F-1".into())
        );
        assert_eq!(
            parse_tag("[see notes] then [kanbanr approve F-1 --rev 0123abcd --project p]")
                .map(|t| t.code),
            Some("F-1".into())
        );
        assert_eq!(
            parse_tag(
                "[kanbanr approve F-1 --rev 0123abcd --project p] [kanbanr approve F-2 extra]"
            ),
            None,
            "the last group is the tag, and it is not a valid one"
        );
    }

    /// A board under the system temp dir (L-41), in conversation mode, with one feature whose
    /// definition is set, and that feature's code.
    fn board(mode: Mode) -> (Store, PathBuf, String) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-passes-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::git::ensure_repo(&dir);
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.gates.insert(
            "Planned".into(),
            Gate {
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        store.init_project("demo", config).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let code = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap()
            .code;
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Checkout keeps the cart".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let decisions = Decisions {
            modes: vec![ModeChange {
                mode,
                by: "Ada".into(),
                at: NOW.into(),
                origin: "terminal".into(),
                reason: String::new(),
                extra: Default::default(),
            }],
            ..Default::default()
        };
        crate::atomic::write(
            store
                .project_dir("demo")
                .join(crate::decisions::DECISIONS_FILE),
            crate::yaml::to_string(&decisions).unwrap(),
        )
        .unwrap();
        (store, dir, code)
    }

    fn rev_of(store: &Store, code: &str) -> String {
        store
            .load("demo")
            .unwrap()
            .feature(code)
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .content_rev()
    }

    /// A PostToolUse payload of the fixture's shape: one question, answered with `answer`.
    fn payload(question: &str, labels: &[&str], answer: &str) -> Value {
        let options: Vec<Value> = labels
            .iter()
            .map(|l| json!({"label": l, "description": "…"}))
            .collect();
        let questions = json!([{
            "question": question,
            "header": "Approve",
            "options": options,
            "multiSelect": false,
        }]);
        json!({
            "session_id": "00000000-0000-0000-0000-000000000000",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/tmp/project",
            "permission_mode": "default",
            "hook_event_name": "PostToolUse",
            "tool_name": "AskUserQuestion",
            "tool_use_id": "toolu_01TEST",
            "tool_input": {"questions": questions},
            "tool_response": {"questions": questions, "answers": {question: answer}},
        })
    }

    /// The agent's own call for a payload of [`payload`]'s shape, as the transcript records it: the
    /// questions only.
    fn call(v: &Value) -> Value {
        json!({"questions": v["tool_input"]["questions"]})
    }

    /// What the hook hears of `v` when the agent's own call carried no answers.
    fn hear(v: &Value) -> Listened {
        listen(v, Some(&call(v)))
    }

    fn answered(v: &Value) -> Vec<Answered> {
        match hear(v) {
            Listened::Answered(a, _) => a,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn a_payload_that_approves_one_and_declines_one_gives_both() {
        let (_store, _root, code) = board(Mode::Conversation);
        let tag = Tag {
            verdict: "approve".into(),
            code: code.clone(),
            signoff: String::new(),
            rev: "0".repeat(16),
            project: "demo".into(),
        };
        let question = format!("Approve {code} as it stands? {}", tag_text(&tag));
        let yes = payload(&question, &["Approve", "Not yet"], "Approve");
        let mut mixed = yes.clone();
        let second = question.replace(&code, "FEAT-9");
        for part in ["tool_input", "tool_response"] {
            let mut q = mixed[part]["questions"][0].clone();
            q["question"] = json!(second);
            mixed[part]["questions"].as_array_mut().unwrap().push(q);
        }
        mixed["tool_response"]["answers"][second.as_str()] = json!("Not yet");
        match hear(&mixed) {
            Listened::Answered(a, d) => {
                assert_eq!(a.len(), 1);
                assert_eq!(a[0].tag, tag);
                assert_eq!(d.len(), 1);
                assert_eq!(
                    (d[0].0.code.as_str(), d[0].1.as_str()),
                    ("FEAT-9", "Not yet")
                );
            }
            other => panic!("expected both, got {other:?}"),
        }
    }

    #[test]
    fn an_approving_answer_makes_a_matching_one_time_pass() {
        let (store, root, code) = board(Mode::Conversation);
        let rev = rev_of(&store, &code);
        let tag = Tag {
            verdict: "approve".into(),
            code: code.clone(),
            signoff: String::new(),
            rev: rev.clone(),
            project: "demo".into(),
        };
        let question = format!("Approve {code} as it stands? {}", tag_text(&tag));
        let yes = payload(&question, &["Approve", "Not yet"], "Approve");

        // The approving answer makes a pass that matches the tag.
        let heard = answered(&yes);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].tag, tag);
        assert_eq!(heard[0].tool_use_id, "toolu_01TEST");
        let pass = issue(&store, &heard[0], NOW).unwrap().expect("a pass");
        assert!(
            pass.id.starts_with("P-") && pass.id.len() == 14,
            "{}",
            pass.id
        );
        assert_eq!(
            (
                pass.project.as_str(),
                pass.code.as_str(),
                pass.verdict.as_str(),
                pass.rev.as_str()
            ),
            ("demo", code.as_str(), "approve", rev.as_str())
        );
        assert_eq!(pass.question, question);
        assert_eq!(pass.answer, "Approve");
        assert_eq!(pass.at, NOW);
        assert_eq!(
            instant(&pass.expires).unwrap() - instant(&pass.at).unwrap(),
            time::Duration::minutes(TTL_MINUTES)
        );
        // It lives under the board's git dir, compared resolved (L-46).
        let file = root
            .join(".git")
            .join(DIR)
            .join(format!("{}.json", pass.id));
        assert!(file.is_file());
        assert_eq!(
            std::fs::canonicalize(dir(&store)).unwrap(),
            std::fs::canonicalize(root.join(".git").join("kanbanr").join("passes")).unwrap()
        );
        assert_eq!(load(&store, &pass.id, NOW), Some(pass.clone()));
        assert!(
            context_line(&pass, "kanbanr approve X").contains("within 15 minutes"),
            "{}",
            context_line(&pass, "kanbanr approve X")
        );

        // Written once: the same answer again is refused, and the file is untouched.
        let again = issue(&store, &heard[0], NOW).unwrap();
        assert!(
            again.as_ref().is_err_and(|w| w.contains("already")),
            "{again:?}"
        );

        // Used once.
        consume(&store, &pass.id).unwrap();
        assert_eq!(load(&store, &pass.id, NOW), None);
        consume(&store, &pass.id).expect("a pass already gone is fine");

        // Expired at TTL: unloadable, and deleted on the way.
        let other = Answered {
            tool_use_id: "toolu_02".into(),
            ..heard[0].clone()
        };
        let p2 = issue(&store, &other, NOW).unwrap().unwrap();
        assert_eq!(load(&store, &p2.id, &p2.expires), None);
        assert!(
            !root
                .join(".git")
                .join(DIR)
                .join(format!("{}.json", p2.id))
                .exists()
        );
        assert_eq!(load(&store, "../../etc/passwd", NOW), None);

        // No pass for anything that is not the approving option.
        for no in ["Not yet", "Yes, approve it", " ", ""] {
            assert!(
                matches!(hear(&payload(&question, &["Approve", "Not yet"], no)),
                    Listened::Declined(ref d) if d.len() == 1 && d[0].1 == no),
                "{no:?}"
            );
        }
        // Nor for an approving label the question never offered.
        assert!(matches!(
            hear(&payload(&question, &["Yes", "No"], "Approve")),
            Listened::Declined(_)
        ));
        // Nor for a multi-select question.
        let mut multi = yes.clone();
        for side in ["tool_input", "tool_response"] {
            multi[side]["questions"][0]["multiSelect"] = json!(true);
        }
        assert!(matches!(hear(&multi), Listened::Quiet(_)));
        // Nor for a question the agent's own call answered before the person saw it (DEF-126).
        let mut pre = call(&yes);
        pre["answers"] = json!({question.clone(): "Approve"});
        assert!(matches!(listen(&yes, Some(&pre)), Listened::Quiet(_)));
        let mut empty_pre = call(&yes);
        empty_pre["answers"] = json!({});
        assert!(matches!(
            listen(&yes, Some(&empty_pre)),
            Listened::Answered(..)
        ));
        // Nor for a sub-agent's question.
        let mut agent = yes.clone();
        agent["agent_id"] = json!("a1b2");
        assert!(matches!(hear(&agent), Listened::Quiet(_)));
        // Nor for another tool or another event.
        let mut tool = yes.clone();
        tool["tool_name"] = json!("Bash");
        assert!(matches!(hear(&tool), Listened::Quiet(_)));
        let mut event = yes.clone();
        event["hook_event_name"] = json!("PreToolUse");
        assert!(matches!(hear(&event), Listened::Quiet(_)));
        // Nor for an untagged question.
        assert!(matches!(
            hear(&payload("Approve it?", &["Approve", "Not yet"], "Approve")),
            Listened::Quiet(_)
        ));

        // A stale rev makes no pass.
        let stale = Answered {
            tag: Tag {
                rev: "0123456789abcdef".into(),
                ..tag.clone()
            },
            tool_use_id: "toolu_03".into(),
            ..heard[0].clone()
        };
        let why = issue(&store, &stale, NOW).unwrap().unwrap_err();
        assert!(why.contains("the definition changed"), "{why}");
        // Nor a sign-off no gate asks for, a ratify with nothing to ratify, or an unknown item.
        for (verdict, signoff, code) in [
            ("signoff:nobody", "nobody", code.as_str()),
            ("ratify", "", code.as_str()),
            ("approve", "", "FTR-999"),
        ] {
            let a = Answered {
                tag: Tag {
                    verdict: verdict.into(),
                    signoff: signoff.into(),
                    code: code.into(),
                    ..tag.clone()
                },
                tool_use_id: format!("toolu_{verdict}_{code}"),
                ..heard[0].clone()
            };
            assert!(issue(&store, &a, NOW).unwrap().is_err(), "{verdict} {code}");
        }
        // A sign-off a gate asks for does.
        let signoff = Answered {
            tag: Tag {
                verdict: "signoff:design-review".into(),
                signoff: "design-review".into(),
                ..tag.clone()
            },
            tool_use_id: "toolu_04".into(),
            ..heard[0].clone()
        };
        assert!(issue(&store, &signoff, NOW).unwrap().is_ok());
        // Nor an over-long question.
        let long = Answered {
            question: "x".repeat(MAX_QUESTION + 1),
            tool_use_id: "toolu_05".into(),
            ..heard[0].clone()
        };
        assert!(issue(&store, &long, NOW).unwrap().is_err());
        let _ = std::fs::remove_dir_all(&root);

        // A board not in conversation mode makes none.
        let (store, root, code) = board(Mode::Monitor);
        let a = Answered {
            tag: Tag {
                code: code.clone(),
                rev: rev_of(&store, &code),
                ..tag
            },
            ..heard[0].clone()
        };
        let why = issue(&store, &a, NOW).unwrap().unwrap_err();
        assert!(why.contains("not in conversation mode"), "{why}");
        assert!(!root.join(".git").join(DIR).exists() || files(&store).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A transcript under the system temp dir holding `lines`, one JSON value a line.
    fn transcript(name: &str, lines: &[String]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanbanr-passes-tr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{name}.jsonl"));
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(&file, text).unwrap();
        file
    }

    /// One assistant transcript line, as Claude Code writes it, with one `tool_use` block.
    fn tool_use_line(id: &str, input: &Value) -> String {
        json!({
            "type": "assistant",
            "isSidechain": false,
            "sessionId": "00000000-0000-0000-0000-000000000000",
            "message": {
                "role": "assistant",
                "type": "message",
                "content": [
                    {"type": "text", "text": "Asking."},
                    {"type": "tool_use", "id": id, "name": "AskUserQuestion", "input": input}
                ]
            }
        })
        .to_string()
    }

    /// The user line that carries the tool's result, which names the call by `tool_use_id`.
    fn tool_result_line(id: &str) -> String {
        json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": "answered"}
            ]},
            "toolUseResult": {"questions": [], "answers": {}, "annotations": {}}
        })
        .to_string()
    }

    /// The payload Claude Code sent on 26 Nov 2026 (FTR-035 R-2 manual check), ids made neutral:
    /// the person's answer and annotations in both `tool_input` and `tool_response`.
    fn payload_as_sent(question: &str, answer: &str) -> Value {
        let questions = json!([{
            "question": question,
            "header": "Approve",
            "options": [
                {"label": "Approve", "description": "Record it now as your decision; kanbanr keeps this question and your answer on the record"},
                {"label": "Not yet", "description": "Record nothing; tell your agent what to change or what you need to see first"}
            ],
            "multiSelect": false
        }]);
        json!({
            "session_id": "00000000-0000-0000-0000-000000000000",
            "transcript_path": "/tmp/kanbanr-fixture/transcript.jsonl",
            "cwd": "/tmp/kanbanr-fixture/project",
            "scratchpad_dir": "/tmp/kanbanr-fixture/scratchpad",
            "prompt_id": "00000000-0000-0000-0000-000000000001",
            "permission_mode": "auto",
            "effort": {"level": "medium"},
            "hook_event_name": "PostToolUse",
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": questions, "answers": {question: answer}, "annotations": {}},
            "tool_response": {"questions": questions, "answers": {question: answer}, "annotations": {}},
            "tool_use_id": "toolu_01NEUTRAL",
            "duration_ms": 0
        })
    }

    fn sent_question() -> String {
        format!(
            "Approve FTR-1 \"Example item\" 's definition as it stands (rev {})? [kanbanr approve FTR-1 --rev {REV} --project demo]",
            &REV[..8]
        )
    }

    /// DEF-126 R-1: the person's answer, which Claude Code writes into `tool_input.answers` too,
    /// makes the pass when the agent's own call (from the transcript) carried only questions.
    #[test]
    fn the_persons_answer_in_the_payload_as_claude_code_sends_it_makes_a_pass() {
        let question = sent_question();
        let payload = payload_as_sent(&question, "Approve");
        let model = json!({"questions": payload["tool_input"]["questions"]});
        let file = transcript(
            "r1",
            &[
                tool_use_line("toolu_01NEUTRAL", &model),
                tool_result_line("toolu_01NEUTRAL"),
            ],
        );
        let found = model_call(&file, "toolu_01NEUTRAL").unwrap();
        assert_eq!(found.as_ref(), Some(&model));
        match listen(&payload, found.as_ref()) {
            Listened::Answered(a, d) => {
                assert_eq!(a.len(), 1);
                assert!(d.is_empty());
                assert_eq!(a[0].answer, "Approve");
                assert_eq!(a[0].tag.code, "FTR-1");
                assert_eq!(a[0].tag.rev, REV);
                assert_eq!(a[0].tool_use_id, "toolu_01NEUTRAL");
            }
            other => panic!("expected a pass to be made, got {other:?}"),
        }
        let _ = std::fs::remove_file(&file);
    }

    /// DEF-126 R-2: a question the agent's own call answered makes no pass, whatever the payload.
    #[test]
    fn a_question_the_agents_own_call_answered_makes_no_pass() {
        let question = sent_question();
        let payload = payload_as_sent(&question, "Approve");
        let mut model = json!({"questions": payload["tool_input"]["questions"]});
        model["answers"] = json!({question.clone(): "Approve"});
        match listen(&payload, Some(&model)) {
            Listened::Quiet(why) => assert!(
                why.contains("the agent's own call carried answers"),
                "{why}"
            ),
            other => panic!("expected no pass, got {other:?}"),
        }
    }

    /// DEF-126 R-2: a question whose call the transcript does not hold makes no pass, and a
    /// tool result naming the id is not the call.
    #[test]
    fn a_question_whose_call_is_not_in_the_transcript_makes_no_pass() {
        let question = sent_question();
        let payload = payload_as_sent(&question, "Approve");
        let model = json!({"questions": payload["tool_input"]["questions"]});
        let file = transcript(
            "r2-missing",
            &[
                tool_use_line("toolu_01OTHER", &model),
                tool_result_line("toolu_01NEUTRAL"),
            ],
        );
        let found = model_call(&file, "toolu_01NEUTRAL").unwrap();
        assert_eq!(found, None);
        match listen(&payload, found.as_ref()) {
            Listened::Quiet(why) => assert!(
                why.contains("the agent's own call was not found in the session transcript"),
                "{why}"
            ),
            other => panic!("expected no pass, got {other:?}"),
        }
        let _ = std::fs::remove_file(&file);
    }

    /// DEF-126 R-2: `model_call` returns the input of the assistant tool_use with that id, read
    /// from the transcript's last 4 MiB, skipping lines that do not parse; a missing file is an error.
    #[test]
    fn model_call_finds_the_tool_use_by_its_id() {
        let first = json!({"questions": [{"question": "first?"}]});
        let wanted = json!({"questions": [{"question": "wanted?"}]});
        let later = json!({"questions": [{"question": "later?"}]});
        let file = transcript(
            "find",
            &[
                tool_use_line("toolu_A", &first),
                "{ not json".to_string(),
                tool_use_line("toolu_B", &wanted),
                tool_result_line("toolu_B"),
                tool_use_line("toolu_C", &later),
            ],
        );
        assert_eq!(model_call(&file, "toolu_B").unwrap(), Some(wanted.clone()));
        assert_eq!(model_call(&file, "toolu_A").unwrap(), Some(first.clone()));
        assert_eq!(model_call(&file, "toolu_Z").unwrap(), None);

        // Only the last 4 MiB is read: a call before them is not found, one after is.
        let filler =
            json!({"type": "user", "message": {"role": "user", "content": "x".repeat(1024)}})
                .to_string();
        let mut lines = vec![tool_use_line("toolu_OLD", &first)];
        let n = (TRANSCRIPT_TAIL as usize) / filler.len() + 2;
        lines.extend(std::iter::repeat_n(filler, n));
        lines.push(tool_use_line("toolu_NEW", &wanted));
        let big = transcript("big", &lines);
        assert!(std::fs::metadata(&big).unwrap().len() > TRANSCRIPT_TAIL);
        assert_eq!(model_call(&big, "toolu_NEW").unwrap(), Some(wanted));
        assert_eq!(model_call(&big, "toolu_OLD").unwrap(), None);

        let gone = file.with_file_name("no-such-transcript.jsonl");
        assert!(model_call(&gone, "toolu_B").is_err());
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&big);
    }
}
