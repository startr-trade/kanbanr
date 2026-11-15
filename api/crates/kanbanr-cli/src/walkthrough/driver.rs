//! The developer's Claude Code session, driven headlessly (FTR-037 W1).
//!
//! `claude -p --input-format stream-json --output-format stream-json` is one process for the whole
//! run: each user turn is one JSON line on its stdin, and the turn is over when stdout carries a
//! `{"type":"result",…}` line. The spike of 15 Nov 2026 (the Design's "What exists") saw exactly
//! that: the `init` line first, one `result` line per turn, `total_cost_usd` cumulative across the
//! session, and no `AskUserQuestion` tool under `-p`, so a question Claude has arrives as text in
//! the turn's final message. Every stdout line is appended to the run's `raw/transcripts/stream.jsonl`
//! before it is read, so a session that dies mid-turn still leaves what it said.

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// The environment variable a run exports to its session and its hooks: the run folder.
pub const RUN_VAR: &str = "KANBANR_WALKTHROUGH_RUN";

/// How a session is started.
#[derive(Debug, Clone)]
pub struct SessionCfg {
    /// The `claude` program: `claude` on PATH, or a fake one a test wrote.
    pub claude: PathBuf,
    /// The run project: the session's working directory.
    pub project: PathBuf,
    /// `CLAUDE_CONFIG_DIR` for the session: the run's own copy, never the person's.
    pub config_dir: PathBuf,
    /// `KANBANR_BOARD_FENCE` for the session and everything it runs.
    pub fence: PathBuf,
    /// The run folder (`KANBANR_WALKTHROUGH_RUN`); the transcript goes under `raw/transcripts/`.
    pub run_dir: PathBuf,
    pub model: String,
    pub budget_usd: f64,
    /// `--max-turns`, when the caller caps a session's agentic turns.
    pub max_turns: Option<u32>,
    /// `--append-system-prompt-file`: the persona.
    pub persona_file: Option<PathBuf>,
    /// How long one turn may take before the session is given up as stalled.
    pub turn_timeout: Duration,
}

impl SessionCfg {
    /// A configuration with the defaults a run uses: `claude` on PATH, a 30-minute turn.
    pub fn new(project: &Path, config_dir: &Path, fence: &Path, run_dir: &Path) -> SessionCfg {
        SessionCfg {
            claude: PathBuf::from("claude"),
            project: project.to_path_buf(),
            config_dir: config_dir.to_path_buf(),
            fence: fence.to_path_buf(),
            run_dir: run_dir.to_path_buf(),
            model: "sonnet".into(),
            budget_usd: 40.0,
            max_turns: None,
            persona_file: None,
            turn_timeout: Duration::from_secs(30 * 60),
        }
    }
}

/// Why a session stopped short.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// `--max-budget-usd` was reached: the result line says so.
    Budget,
    /// `--max-turns` was reached.
    MaxTurns,
    /// The result line was an error of another kind.
    Error(String),
    /// The process ended (stdout closed) before a result line.
    Exited,
    /// No result line within the turn timeout; the process was killed.
    Timeout,
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Budget => f.write_str("the session reached its budget"),
            StopReason::MaxTurns => f.write_str("the session reached its turn limit"),
            StopReason::Error(why) => write!(f, "the session ended in error: {why}"),
            StopReason::Exited => f.write_str("the session ended before answering"),
            StopReason::Timeout => {
                f.write_str("the session did not answer within the turn timeout")
            }
        }
    }
}

/// A question Claude asked in the text of its final message: the question and the options it
/// listed, if any (a numbered or bulleted list after the question).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextQuestion {
    pub text: String,
    pub options: Vec<String>,
}

/// What one `say` got back.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    /// The `result` line's text, or the last assistant text when there was none.
    pub final_text: String,
    /// A question asked in the final text, for the answer policy.
    pub asked: Option<TextQuestion>,
    /// What this turn cost: the difference between this result's cumulative cost and the last.
    pub cost_usd: f64,
    /// Set when the turn did not end in a normal result.
    pub stopped: Option<StopReason>,
    /// Assistant turns the result line reported (`num_turns`).
    pub num_turns: u64,
    /// stdout lines read for this turn.
    pub lines: usize,
}

/// One running `claude -p` process.
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
    transcript: std::fs::File,
    session_id: String,
    cost_so_far: f64,
    turns: u32,
    turn_timeout: Duration,
    stopped: Option<StopReason>,
}

impl Session {
    /// Spawn the session. Every `KANBANR_*` variable and `CLAUDECODE` are dropped from the
    /// environment first, so the session cannot inherit the person's board or caller; the fence,
    /// the config folder and the run folder are then set.
    pub fn start(cfg: &SessionCfg) -> Result<Session> {
        let transcripts = cfg.run_dir.join("raw").join("transcripts");
        std::fs::create_dir_all(&transcripts)
            .with_context(|| format!("could not make {}", transcripts.display()))?;
        let transcript = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(transcripts.join("stream.jsonl"))?;
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(transcripts.join("stderr.txt"))?;
        let mut cmd = Command::new(&cfg.claude);
        cmd.arg("-p")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--verbose")
            .args(["--model", &cfg.model])
            .args(["--max-budget-usd", &format!("{}", cfg.budget_usd)])
            .args(["--permission-mode", "bypassPermissions"]);
        if let Some(n) = cfg.max_turns {
            cmd.args(["--max-turns", &n.to_string()]);
        }
        if let Some(persona) = &cfg.persona_file {
            cmd.arg("--append-system-prompt-file").arg(persona);
        }
        cmd.current_dir(&cfg.project);
        cmd.env_clear();
        for (k, v) in std::env::vars_os() {
            let key = k.to_string_lossy();
            // The person's board, caller and config never reach the session; the run's own
            // variables (`KANBANR_WALKTHROUGH_*`: the allowed remotes) do.
            if (key.starts_with("KANBANR_") && !key.starts_with("KANBANR_WALKTHROUGH_"))
                || key == "CLAUDECODE"
                || key == "CLAUDE_CONFIG_DIR"
            {
                continue;
            }
            cmd.env(k, v);
        }
        cmd.env("CLAUDE_CONFIG_DIR", &cfg.config_dir)
            .env(kanbanr_core::project::FENCE_VAR, &cfg.fence)
            .env(RUN_VAR, &cfg.run_dir);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        let mut child = cmd
            .spawn()
            .with_context(|| format!("could not start {}", cfg.claude.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        // A reader thread, so a turn can time out without blocking on a pipe forever.
        let (tx, rx) = channel::<Option<String>>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(Some(l)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });
        Ok(Session {
            child,
            stdin: Some(stdin),
            lines: rx,
            transcript,
            session_id: String::new(),
            cost_so_far: 0.0,
            turns: 0,
            turn_timeout: cfg.turn_timeout,
            stopped: None,
        })
    }

    /// The session id the `init` or a `result` line named; empty until one has.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// What the session has cost so far, from the last result line.
    pub fn cost_usd(&self) -> f64 {
        self.cost_so_far
    }

    /// User turns sent so far.
    pub fn turns(&self) -> u32 {
        self.turns
    }

    /// Why the session stopped, once it has.
    pub fn stopped(&self) -> Option<&StopReason> {
        self.stopped.as_ref()
    }

    /// The stdin envelope for one user turn, as the spike found it.
    pub fn envelope(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": text},
        })
        .to_string()
    }

    /// Send one user message and read until the turn's `result` line. Every stdout line is appended
    /// to the transcript as it arrives. A session that has stopped answers the same stop again.
    pub fn say(&mut self, text: &str) -> Result<Turn> {
        if let Some(stop) = &self.stopped {
            return Ok(Turn {
                stopped: Some(stop.clone()),
                ..Default::default()
            });
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(Turn {
                stopped: Some(StopReason::Exited),
                ..Default::default()
            });
        };
        self.turns += 1;
        let line = Self::envelope(text);
        writeln!(self.transcript, "# >>> {line}")?;
        // A closed pipe is the process gone: the stop, not an error, so the run goes on to its
        // captures.
        if stdin.write_all(format!("{line}\n").as_bytes()).is_err() {
            self.stopped = Some(StopReason::Exited);
            return Ok(Turn {
                stopped: Some(StopReason::Exited),
                ..Default::default()
            });
        }
        let _ = stdin.flush();
        let deadline = Instant::now() + self.turn_timeout;
        let mut turn = Turn::default();
        let mut last_text = String::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let got = match self.lines.recv_timeout(left) {
                Ok(Some(l)) => l,
                Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                    self.stopped = Some(StopReason::Exited);
                    turn.stopped = Some(StopReason::Exited);
                    turn.final_text = last_text;
                    return Ok(turn);
                }
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    self.stopped = Some(StopReason::Timeout);
                    turn.stopped = Some(StopReason::Timeout);
                    turn.final_text = last_text;
                    return Ok(turn);
                }
            };
            turn.lines += 1;
            writeln!(self.transcript, "{got}")?;
            let Ok(ev) = serde_json::from_str::<Value>(&got) else {
                continue;
            };
            if let Some(id) = ev.get("session_id").and_then(Value::as_str)
                && !id.is_empty()
            {
                self.session_id = id.to_string();
            }
            match ev.get("type").and_then(Value::as_str) {
                Some("assistant") => {
                    if let Some(t) = assistant_text(&ev) {
                        last_text = t;
                    }
                }
                Some("result") => {
                    let total = ev
                        .get("total_cost_usd")
                        .and_then(Value::as_f64)
                        .unwrap_or(self.cost_so_far);
                    turn.cost_usd = (total - self.cost_so_far).max(0.0);
                    self.cost_so_far = total;
                    turn.num_turns = ev.get("num_turns").and_then(Value::as_u64).unwrap_or(0);
                    turn.final_text = ev
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(last_text);
                    let subtype = ev.get("subtype").and_then(Value::as_str).unwrap_or("");
                    let is_error = ev.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    if is_error || subtype.starts_with("error") {
                        let stop = stop_of(subtype, &turn.final_text);
                        self.stopped = Some(stop.clone());
                        turn.stopped = Some(stop);
                    } else {
                        turn.asked = parse_text_question(&turn.final_text);
                    }
                    return Ok(turn);
                }
                _ => {}
            }
        }
    }

    /// Close stdin and wait for the process to end; kill it if it does not within 15 s.
    pub fn close(mut self) -> Result<()> {
        self.end();
        Ok(())
    }

    /// The same as [`Session::close`], on a session that stays owned: nothing more is said to it.
    pub fn end(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Kill the process now (a budget or wall-time stop from outside the turn).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.stopped.is_none() {
            self.stopped = Some(StopReason::Exited);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The stop a result line's error names.
fn stop_of(subtype: &str, text: &str) -> StopReason {
    let s = subtype.to_ascii_lowercase();
    let t = text.to_ascii_lowercase();
    if s.contains("budget") || t.contains("max_budget") || t.contains("budget") {
        StopReason::Budget
    } else if s.contains("max_turns") || t.contains("max_turns") {
        StopReason::MaxTurns
    } else {
        StopReason::Error(if subtype.is_empty() {
            text.to_string()
        } else {
            format!("{subtype}: {text}")
        })
    }
}

/// The text blocks of an assistant line, joined.
fn assistant_text(ev: &Value) -> Option<String> {
    let content = ev.get("message")?.get("content")?.as_array()?;
    let texts: Vec<&str> = content
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// An option line: `1. Red`, `2) Blue (recommended)`, `- Red`, `* Blue`, `• Red`; markdown bold
/// stripped. None for any other line.
fn option_label(line: &str) -> Option<String> {
    let l = line.trim();
    let rest = if let Some((n, rest)) = l.split_once(['.', ')'])
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
    {
        rest
    } else if let Some(rest) = l
        .strip_prefix("- ")
        .or_else(|| l.strip_prefix("* "))
        .or_else(|| l.strip_prefix("• "))
    {
        rest
    } else {
        return None;
    };
    let label = rest.trim().replace("**", "");
    let label = label.trim().trim_end_matches('.').trim().to_string();
    (!label.is_empty()).then_some(label)
}

/// A question asked in a final message (the live path under `-p`, see the module doc): the last
/// line ending in `?`, when what follows it is only its options and at most one closing line
/// ("Reply with your choice"). None when the message asks nothing.
pub fn parse_text_question(text: &str) -> Option<TextQuestion> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let q_idx = lines
        .iter()
        .rposition(|l| l.trim().ends_with('?') && option_label(l).is_none())?;
    let mut options = Vec::new();
    let mut closing = 0;
    for l in &lines[q_idx + 1..] {
        if l.trim().is_empty() {
            continue;
        }
        if let Some(label) = option_label(l) {
            options.push(label);
            continue;
        }
        // One closing remark after the options is still a question; more prose is not.
        closing += 1;
        if closing > 1 || l.trim().ends_with('?') {
            return None;
        }
        let low = l.to_ascii_lowercase();
        if !(low.contains("reply")
            || low.contains("pick")
            || low.contains("choose")
            || low.contains("let me know")
            || low.contains("tell me")
            || low.contains("your choice")
            || low.contains("i haven't")
            || low.contains("i'll"))
        {
            return None;
        }
    }
    let question = lines[q_idx].trim().replace("**", "");
    Some(TextQuestion {
        text: question,
        options,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch run folder under the temporary directory.
    pub(crate) fn scratch(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-wt-driver-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::create_dir_all(dir.join("claude-config")).unwrap();
        dir
    }

    /// A fake `claude`: prints an init line, then for every stdin line one assistant line and one
    /// result line; from the `budget_at`-th turn on the result is a budget stop.
    #[cfg(unix)]
    pub(crate) fn fake_claude(dir: &Path, budget_at: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            "#!/bin/sh\n\
             n=0\n\
             printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fake-1\",\"model\":\"fake\",\"tools\":[]}}'\n\
             while IFS= read -r line; do\n\
               n=$((n+1))\n\
               printf '%s\\n' '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Which colour?\\n1. Red\\n2. Blue (recommended)\"}}]}}}}'\n\
               if [ $n -ge {budget_at} ]; then\n\
                 printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"error_max_budget_usd\",\"is_error\":true,\"session_id\":\"fake-1\",\"total_cost_usd\":2.5,\"num_turns\":1,\"result\":\"Reached max budget of $2\"}}'\n\
               else\n\
                 printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"session_id\":\"fake-1\",\"total_cost_usd\":0.5,\"num_turns\":1,\"result\":\"Which colour should the basket page use?\\n1. Red\\n2. Blue (recommended)\\n\\nReply with your choice.\"}}'\n\
               fi\n\
             done\n"
        );
        let path = dir.join("claude");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn cfg(dir: &Path, claude: &Path) -> SessionCfg {
        let mut c = SessionCfg::new(
            &dir.join("project"),
            &dir.join("claude-config"),
            &std::env::temp_dir(),
            dir,
        );
        c.claude = claude.to_path_buf();
        c.budget_usd = 2.0;
        c.turn_timeout = Duration::from_secs(20);
        c
    }

    /// FTR-037 W1: a turn is one stdin envelope and every stdout line up to and including the
    /// `result` line; the final text, the question in it and the turn's own cost are read from it,
    /// and the transcript holds every line.
    #[cfg(unix)]
    #[test]
    fn a_turn_ends_at_the_result_line() {
        let dir = scratch("turn");
        let claude = fake_claude(&dir, 99);
        let mut s = Session::start(&cfg(&dir, &claude)).unwrap();
        let t = s.say("Start using kanbanr for this project.").unwrap();
        assert!(t.stopped.is_none(), "{t:?}");
        assert_eq!(t.lines, 3, "init, assistant, result");
        assert!(
            t.final_text
                .starts_with("Which colour should the basket page use?")
        );
        let q = t
            .asked
            .expect("a text question is read from the final text");
        assert_eq!(q.text, "Which colour should the basket page use?");
        assert_eq!(q.options, ["Red", "Blue (recommended)"]);
        assert_eq!(s.session_id(), "fake-1");
        assert!((t.cost_usd - 0.5).abs() < 1e-9);
        // The second turn costs the difference of the cumulative figures.
        let t2 = s.say("Blue").unwrap();
        assert_eq!(
            t2.lines, 2,
            "assistant and result only: the init line came once"
        );
        assert!(t2.cost_usd.abs() < 1e-9, "{t2:?}");
        assert_eq!(s.turns(), 2);
        s.close().unwrap();
        let transcript = std::fs::read_to_string(dir.join("raw/transcripts/stream.jsonl")).unwrap();
        assert_eq!(
            transcript
                .lines()
                .filter(|l| l.starts_with("# >>> "))
                .count(),
            2
        );
        assert_eq!(
            transcript
                .lines()
                .filter(|l| l.contains("\"type\":\"result\""))
                .count(),
            2
        );
        assert!(transcript.contains("\"subtype\":\"init\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-037 W1 and R-8: a result line that names the budget stops the session; the turn says
    /// so, the transcript keeps the line, and every later `say` answers the same stop at once.
    #[cfg(unix)]
    #[test]
    fn a_budget_stop_is_reported() {
        let dir = scratch("budget");
        let claude = fake_claude(&dir, 2);
        let mut s = Session::start(&cfg(&dir, &claude)).unwrap();
        let first = s.say("Start using kanbanr for this project.").unwrap();
        assert!(first.stopped.is_none());
        let second = s.say("Blue").unwrap();
        assert_eq!(second.stopped, Some(StopReason::Budget), "{second:?}");
        assert!(second.asked.is_none(), "a stop is not a question");
        assert_eq!(s.stopped(), Some(&StopReason::Budget));
        let third = s.say("anything").unwrap();
        assert_eq!(third.stopped, Some(StopReason::Budget));
        assert_eq!(third.lines, 0, "nothing more is sent to a stopped session");
        s.close().unwrap();
        let transcript = std::fs::read_to_string(dir.join("raw/transcripts/stream.jsonl")).unwrap();
        assert!(transcript.contains("error_max_budget_usd"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_envelope_is_the_one_the_spike_found() {
        let v: Value = serde_json::from_str(&Session::envelope("hi there")).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"], "hi there");
    }

    #[test]
    fn a_text_question_is_read_with_its_options_and_prose_is_not() {
        let q = parse_text_question(
            "I can't use the tool here, so I'll ask in text.\n\nWhich colour should the basket page use?\n\n- **Blue** (recommended)\n- **Red**\n\nReply with your choice. I haven't picked one for you.",
        )
        .unwrap();
        assert_eq!(q.text, "Which colour should the basket page use?");
        assert_eq!(q.options, ["Blue (recommended)", "Red"]);
        let bare = parse_text_question("Done. Shall I start the sprint today?").unwrap();
        assert_eq!(bare.text, "Done. Shall I start the sprint today?");
        assert!(bare.options.is_empty());
        assert!(parse_text_question("The sprint is started. The item is at Vision.").is_none());
        assert!(
            parse_text_question("Is it done? Yes. Here is the board.\n\nThe item is at Vision.")
                .is_none(),
            "a question answered in the same breath is not asked"
        );
        assert_eq!(option_label("3) Keep it"), Some("Keep it".into()));
        assert_eq!(option_label("not an option"), None);
    }

    #[test]
    fn stops_are_named_from_the_result_line() {
        assert_eq!(stop_of("error_max_budget_usd", ""), StopReason::Budget);
        assert_eq!(stop_of("error_max_turns", ""), StopReason::MaxTurns);
        assert_eq!(
            stop_of("error_during_execution", "boom"),
            StopReason::Error("error_during_execution: boom".into())
        );
    }
}
