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
    /// `PATH` for the session and its hooks: the run's own `bin/` first, so `kanbanr` is the run's
    /// binary (G-6). None keeps this process's `PATH`.
    pub path: Option<std::ffi::OsString>,
    /// `CLAUDE_CODE_OAUTH_TOKEN` for the session, from the run's token file; never written to a
    /// file or a log. None leaves the session to the run's config copy.
    pub oauth_token: Option<super::credentials::Secret>,
    /// How long one turn may take before the session is given up as stalled.
    pub turn_timeout: Duration,
    /// How long the stream must stay quiet, with no agent running, for the session to be idle.
    pub quiet: Duration,
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
            path: None,
            oauth_token: None,
            turn_timeout: Duration::from_secs(30 * 60),
            quiet: Duration::from_secs(5),
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
    /// The session could not sign in (a 401, "Failed to authenticate", "Not logged in"): a
    /// harness stop, never a product finding.
    Auth(String),
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
            StopReason::Auth(why) => write!(f, "the session could not sign in: {why}"),
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
    /// The whole final message: the policy reads it when the question alone matches no rule.
    pub context: String,
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
    /// Background agents still running when the turn ended (sub-agent builds).
    pub agents_running: usize,
    /// Agents this turn started.
    pub agents_started: usize,
    /// A background agent that was running before this turn ended or reported during it: the
    /// reply may be about that, not about what the turn asked (RC3).
    pub notified: bool,
}

/// What waiting for the session to go idle saw.
#[derive(Debug, Clone, Default)]
pub struct IdleWait {
    /// No agent left running, and the stream went quiet.
    pub idle: bool,
    /// Turns Claude took on its own meanwhile (reacting to an agent's notification).
    pub unsolicited: usize,
    /// The last of their texts.
    pub last_text: String,
    pub lines: usize,
}

/// One running `claude -p` process.
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
    transcript: std::fs::File,
    /// Lines in `stream.jsonl` so far, the `# >>>` input lines included: what a finding's
    /// transcript line is counted in, so it is the line `sed -n` shows.
    transcript_lines: usize,
    session_id: String,
    cost_so_far: f64,
    turns: u32,
    turn_timeout: Duration,
    stopped: Option<StopReason>,
    /// Background agents running now, by task id (`local_agent` tasks only: a Monitor watch or a
    /// background shell never ends on its own and is not waited for).
    agents: std::collections::BTreeSet<String>,
    /// How long the stream must stay quiet for the session to count as idle.
    quiet: Duration,
}

impl Session {
    /// Spawn the session. Every `KANBANR_*` variable and `CLAUDECODE` are dropped from the
    /// environment first, so the session cannot inherit the person's board or caller; the fence,
    /// the config folder and the run folder are then set.
    pub fn start(cfg: &SessionCfg) -> Result<Session> {
        let transcripts = cfg.run_dir.join("raw").join("transcripts");
        std::fs::create_dir_all(&transcripts)
            .with_context(|| format!("could not make {}", transcripts.display()))?;
        let stream = transcripts.join("stream.jsonl");
        let transcript_lines = std::fs::read_to_string(&stream)
            .map(|t| t.lines().count())
            .unwrap_or(0);
        let transcript = super::append_to(stream)?;
        let stderr = super::append_to(transcripts.join("stderr.txt"))?;
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
                || key == super::credentials::SESSION_TOKEN_VAR
                || key == super::credentials::TOKEN_FILE_VAR
            {
                continue;
            }
            cmd.env(k, v);
        }
        if let Some(path) = &cfg.path {
            cmd.env("PATH", path);
        }
        if let Some(token) = &cfg.oauth_token {
            cmd.env(super::credentials::SESSION_TOKEN_VAR, token.expose());
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
            transcript_lines,
            session_id: String::new(),
            cost_so_far: 0.0,
            turns: 0,
            turn_timeout: cfg.turn_timeout,
            stopped: None,
            agents: Default::default(),
            quiet: cfg.quiet,
        })
    }

    /// The number of the transcript's last line, as the file counts it (input lines included).
    pub fn transcript_line(&self) -> usize {
        self.transcript_lines
    }

    /// Background agents running now.
    pub fn agents_running(&self) -> usize {
        self.agents.len()
    }

    /// Follow the session's background tasks from one stream event. Returns what changed:
    /// `(started, ended)` agent counts.
    fn track(&mut self, ev: &Value) -> (usize, usize) {
        if ev.get("type").and_then(Value::as_str) != Some("system") {
            return (0, 0);
        }
        let mut started = 0;
        let mut ended = 0;
        let id = ev["task_id"].as_str().unwrap_or("").to_string();
        match ev["subtype"].as_str().unwrap_or("") {
            "background_tasks_changed" => {
                let now: std::collections::BTreeSet<String> = ev["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|t| t["task_type"] == "local_agent")
                    .filter_map(|t| t["task_id"].as_str().map(str::to_string))
                    .collect();
                started = now.difference(&self.agents).count();
                ended = self.agents.difference(&now).count();
                self.agents = now;
            }
            "task_started" if ev["task_type"] == "local_agent" && !id.is_empty() => {
                if self.agents.insert(id) {
                    started = 1;
                }
            }
            "task_updated" => {
                let status = ev["patch"]["status"].as_str().unwrap_or("");
                if matches!(status, "completed" | "failed" | "killed" | "cancelled")
                    && self.agents.remove(&id)
                {
                    ended = 1;
                }
            }
            "task_notification" if self.agents.remove(&id) => {
                ended = 1;
            }
            _ => {}
        }
        (started, ended)
    }

    /// Read the stream without sending until no background agent runs and nothing has arrived for
    /// the quiet period, or `timeout` passes. Turns Claude takes on its own meanwhile are kept in
    /// the transcript and counted.
    pub fn wait_idle(&mut self, timeout: Duration) -> Result<IdleWait> {
        let deadline = Instant::now() + timeout;
        let mut out = IdleWait::default();
        let mut last_line = Instant::now();
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(out);
            }
            if self.agents.is_empty() && now.duration_since(last_line) >= self.quiet {
                out.idle = true;
                return Ok(out);
            }
            let wait = (deadline - now).min(Duration::from_millis(200));
            let got = match self.lines.recv_timeout(wait) {
                Ok(Some(l)) => l,
                Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                    self.stopped = Some(StopReason::Exited);
                    return Ok(out);
                }
                Err(RecvTimeoutError::Timeout) => continue,
            };
            last_line = Instant::now();
            out.lines += 1;
            writeln!(self.transcript, "{got}")?;
            self.transcript_lines += 1;
            let Ok(ev) = serde_json::from_str::<Value>(&got) else {
                continue;
            };
            self.track(&ev);
            if ev.get("type").and_then(Value::as_str) == Some("result") {
                out.unsolicited += 1;
                if let Some(total) = ev.get("total_cost_usd").and_then(Value::as_f64) {
                    self.cost_so_far = total;
                }
                out.last_text = ev["result"].as_str().unwrap_or("").to_string();
            }
        }
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
        self.transcript_lines += 1;
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
        let running_before = !self.agents.is_empty();
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
            self.transcript_lines += 1;
            let Ok(ev) = serde_json::from_str::<Value>(&got) else {
                continue;
            };
            if let Some(id) = ev.get("session_id").and_then(Value::as_str)
                && !id.is_empty()
            {
                self.session_id = id.to_string();
            }
            let (started, ended) = self.track(&ev);
            turn.agents_started += started;
            if ended > 0 || (running_before && ev["subtype"] == "task_progress") {
                turn.notified = true;
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
                    turn.agents_running = self.agents.len();
                    turn.notified |= running_before;
                    return Ok(turn);
                }
                _ => {}
            }
        }
    }

    /// Close stdin and wait for the process to end; kill it if it does not within 15 s. Nothing
    /// more is said to the session afterwards.
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
    } else if super::credentials::is_auth_failure(text) {
        StopReason::Auth(text.to_string())
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
    let numbered = l
        .split_once(['.', ')'])
        .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map(|(_, rest)| rest);
    let bulleted = || {
        l.strip_prefix("- ")
            .or_else(|| l.strip_prefix("* "))
            .or_else(|| l.strip_prefix("• "))
    };
    let rest = numbered.or_else(bulleted)?;
    let label = rest.trim().replace("**", "");
    let label = label.trim().trim_end_matches('.').trim().to_string();
    (!label.is_empty()).then_some(label)
}

/// Markdown emphasis and code marks taken out, for reading a message as words.
fn plain(line: &str) -> String {
    line.replace("**", "").replace("__", "").replace('`', "")
}

/// The last sentence of `line` that ends in `?` (markdown stripped), or None.
fn question_sentence(line: &str) -> Option<String> {
    let p = plain(line);
    let at = p.rfind('?')?;
    let head = &p[..=at];
    // The sentence starts after the last sentence end before the question mark.
    let start = head[..head.len() - 1]
        .rfind(['.', '!', '?', ':'])
        .map(|i| i + 1)
        .unwrap_or(0);
    let q = head[start..].trim();
    // "Question 1: what is this project for?" keeps its lead-in when the colon opens the line.
    let q = if start > 0 && head[..start].split_whitespace().count() <= 3 {
        head.trim()
    } else {
        q
    };
    (q.chars().filter(|c| c.is_alphabetic()).count() >= 3).then(|| q.to_string())
}

/// Phrases that ask for an answer without a question mark ("A reply of just 1 is enough",
/// "I've asked for the board location twice", "setup is blocked on that one answer").
const ASKING: &[&str] = &[
    "a reply of",
    "reply with",
    "your answer",
    "need your answer",
    "i still need your",
    "tell me which",
];

/// Openers of a request addressed to the person: a sentence that starts with one asks for an
/// answer even with no `?` (preview-6: "**Confirm this draft as it is, or tell me what to change
/// or add.**").
const REQUESTS: &[&str] = &[
    "confirm",
    "tell me",
    "let me know",
    "reply with",
    "reply to",
    "choose",
    "pick",
    "say which",
    "say whether",
    "select",
    "decide",
    "approve this",
    "approve the draft",
    "approve the plan",
    "approve the charter",
    "give me",
    "send me",
];

/// Is this sentence a request to the person? It opens with one of `REQUESTS` (after an optional
/// "please"), and is not a conditional offer ("let me know if you want changes", "tell me when
/// you've decided"), which a settled turn often ends with.
fn is_request(sentence: &str) -> bool {
    let s = sentence
        .trim()
        .trim_start_matches(['-', '*', ' '])
        .to_ascii_lowercase();
    let s = s.strip_prefix("please ").unwrap_or(&s).trim_start();
    REQUESTS.iter().any(|r| {
        s.strip_prefix(r).is_some_and(|rest| {
            // A whole word: "confirmed" is no request.
            let whole = s
                .as_bytes()
                .get(r.len())
                .is_none_or(|b| !b.is_ascii_alphabetic());
            let rest = rest.trim_start_matches([',', ' ']);
            whole && !rest.starts_with("if ") && !rest.starts_with("when ")
        })
    })
}

/// The sentences of `text` (markdown stripped) split at `.`, `!` and line ends.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let p = plain(line);
        let mut start = 0;
        for (i, c) in p.char_indices() {
            if matches!(c, '.' | '!') {
                let s = p[start..=i].trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
                start = i + c.len_utf8();
            }
        }
        let s = p[start..].trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
    }
    out
}

/// The request a message ends on or puts in bold: the last bold sentence that is a request, else
/// the message's last sentence when it is one.
fn request_sentence(text: &str) -> Option<String> {
    let mut bold: Vec<String> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split("**").collect();
        // Odd parts sit between a pair of `**`.
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 && i + 1 < parts.len() {
                bold.extend(sentences(part));
            }
        }
    }
    if let Some(s) = bold.iter().rev().find(|s| is_request(s)) {
        return Some(s.clone());
    }
    sentences(text).into_iter().last().filter(|s| is_request(s))
}

/// A question asked in a final message (the live path under `-p`, see the module doc). The last
/// sentence ending in `?` anywhere in the message is the question, bold or not, with or without
/// prose after it; its options are the numbered or bulleted lines that follow it. A message with
/// no question mark still asks when it lists an option marked "(recommended)" or uses a phrase
/// that asks for an answer: the question is then that sentence. With no question mark, a request
/// sentence (in bold, or the message's last sentence) is the question. Only a message that asks
/// nothing at all is settled (None). `context` keeps the whole message for the policy.
pub fn parse_text_question(text: &str) -> Option<TextQuestion> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let options_after = |from: usize| -> Vec<String> {
        let mut out = Vec::new();
        for l in &lines[from..] {
            if l.trim().is_empty() {
                if out.is_empty() {
                    continue;
                }
                break;
            }
            match option_label(l) {
                Some(label) => out.push(plain(&label).trim().to_string()),
                None if out.is_empty() => {
                    // Prose between the question and its list ("It's its own git repo…").
                    continue;
                }
                None => break,
            }
        }
        out
    };
    let context = text.to_string();
    if let Some(idx) = lines
        .iter()
        .rposition(|l| option_label(l).is_none() && question_sentence(l).is_some())
    {
        let mut question = question_sentence(lines[idx]).unwrap_or_default();
        let mut options = options_after(idx + 1);
        if options.is_empty() {
            // "…list… Which one?": the options come before a short closing question, and the
            // question they answer is the one above them.
            if let Some(last) = (0..idx).rev().find(|&j| option_label(lines[j]).is_some()) {
                let mut first = last;
                while first > 0
                    && (option_label(lines[first - 1]).is_some()
                        || (lines[first - 1].trim().is_empty()
                            && first >= 2
                            && option_label(lines[first - 2]).is_some()))
                {
                    first -= 1;
                }
                options = options_after(first);
                if question.split_whitespace().count() <= 4
                    && let Some(q) = (0..first).rev().find_map(|j| {
                        (option_label(lines[j]).is_none())
                            .then(|| question_sentence(lines[j]))
                            .flatten()
                    })
                {
                    question = q;
                }
            }
        }
        return Some(TextQuestion {
            text: question,
            options,
            context,
        });
    }
    // No question mark: an option list with a recommendation, or a phrase that asks.
    let all_options: Vec<String> = lines
        .iter()
        .filter_map(|l| option_label(l))
        .map(|l| plain(&l).trim().to_string())
        .collect();
    let recommended = all_options
        .iter()
        .any(|o| crate::walkthrough::answer::is_recommended(o));
    let low = text.to_ascii_lowercase();
    let phrase = ASKING.iter().find(|p| low.contains(*p));
    if !recommended && phrase.is_none() {
        // A request to the person, in bold or as the last sentence, asks too (preview-6).
        return request_sentence(text).map(|text| TextQuestion {
            text,
            options: Vec::new(),
            context,
        });
    }
    let sentence = phrase
        .and_then(|p| {
            let p_low = low.find(p)?;
            let start = low[..p_low]
                .rfind(['.', '!', '\n'])
                .map(|i| i + 1)
                .unwrap_or(0);
            let end = low[p_low..]
                .find(['.', '!', '\n'])
                .map(|i| p_low + i + 1)
                .unwrap_or(low.len());
            Some(plain(&text[start..end]).trim().to_string())
        })
        .unwrap_or_else(|| "Which one?".to_string());
    Some(TextQuestion {
        text: sentence,
        options: if recommended { all_options } else { Vec::new() },
        context,
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

    /// RC2: the session gets the token file's token as `CLAUDE_CODE_OAUTH_TOKEN`, the token is in
    /// no transcript line, and a sign-in failure is the harness stop `Auth`.
    #[cfg(unix)]
    #[test]
    fn the_token_reaches_the_session_and_never_the_stream() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("token");
        let path = dir.join("claude");
        std::fs::write(
            &path,
            "#!/bin/sh\n\
             while IFS= read -r line; do\n\
               if [ \"$CLAUDE_CODE_OAUTH_TOKEN\" = \"tok-not-a-real-one\" ]; then\n\
                 printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"total_cost_usd\":0.1,\"num_turns\":1,\"result\":\"signed in\"}'\n\
               else\n\
                 printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\"total_cost_usd\":0.1,\"num_turns\":1,\"result\":\"Failed to authenticate. API Error: 401 OAuth access token is invalid\"}'\n\
               fi\n\
             done\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut c = cfg(&dir, &path);
        c.oauth_token = Some(
            super::super::credentials::read_token_file(
                &{
                    let f = dir.join("walkthrough-token");
                    std::fs::write(&f, "tok-not-a-real-one\n").unwrap();
                    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
                    f
                },
                &dir.join("project"),
            )
            .unwrap()
            .unwrap(),
        );
        assert!(!format!("{c:?}").contains("tok-not"));
        let mut s = Session::start(&c).unwrap();
        let t = s.say("Hello.").unwrap();
        assert_eq!(t.final_text, "signed in", "{t:?}");
        s.end();
        let transcript = std::fs::read_to_string(dir.join("raw/transcripts/stream.jsonl")).unwrap();
        assert!(!transcript.contains("tok-not"), "{transcript}");
        // Without the token the fake fails to sign in: a harness stop.
        c.oauth_token = None;
        let mut s = Session::start(&c).unwrap();
        let t = s.say("Hello.").unwrap();
        assert!(matches!(t.stopped, Some(StopReason::Auth(_))), "{t:?}");
        s.end();
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
        let s_line = s.transcript_line();
        s.end();
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
        // A finding's line is the file's own line number, the input lines counted too.
        assert_eq!(s_line, transcript.lines().count());
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
        s.end();
        let transcript = std::fs::read_to_string(dir.join("raw/transcripts/stream.jsonl")).unwrap();
        assert!(transcript.contains("error_max_budget_usd"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RC3: a turn that starts a background build reports it as running; waiting for the session
    /// to go idle reads the agent's end and the turn Claude takes on its own, and only then is the
    /// session idle. A turn sent while an agent runs is marked, so the run can wait and say it again.
    #[cfg(unix)]
    #[test]
    fn a_turn_that_starts_a_build_waits_for_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("build");
        let p = |s: &str| format!("printf '%s\\n' '{s}'");
        let script = format!(
            "#!/bin/sh\n\
             {init}\n\
             n=0\n\
             while IFS= read -r line; do\n\
               n=$((n+1))\n\
               if [ $n -eq 1 ]; then\n\
                 {bg}\n\
                 {started}\n\
                 {result1}\n\
                 ( sleep 1; {updated}; {notified}; {bg0}; {assistant2}; {result2} ) &\n\
               else\n\
                 {result3}\n\
               fi\n\
             done\n\
             wait\n",
            init = p(r#"{"type":"system","subtype":"init","session_id":"fake-2"}"#),
            bg = p(
                r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"a1","task_type":"local_agent"},{"task_id":"m1","task_type":"local_bash"}]}"#
            ),
            started = p(
                r#"{"type":"system","subtype":"task_started","task_id":"a1","task_type":"local_agent","is_backgrounded":true}"#
            ),
            result1 = p(
                r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.5,"num_turns":1,"result":"The build has started."}"#
            ),
            updated = p(
                r#"{"type":"system","subtype":"task_updated","task_id":"a1","patch":{"status":"completed"}}"#
            ),
            notified = p(
                r#"{"type":"system","subtype":"task_notification","task_id":"a1","status":"completed"}"#
            ),
            bg0 = p(
                r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"m1","task_type":"local_bash"}]}"#
            ),
            assistant2 = p(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"The build finished."}]}}"#
            ),
            result2 = p(
                r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.75,"num_turns":1,"result":"The build finished."}"#
            ),
            result3 = p(
                r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.9,"num_turns":1,"result":"Done."}"#
            ),
        );
        let claude = dir.join("claude");
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut c = cfg(&dir, &claude);
        c.quiet = Duration::from_millis(300);
        let mut s = Session::start(&c).unwrap();
        let t = s.say("Have a sub-agent build these tasks.").unwrap();
        assert_eq!(t.final_text, "The build has started.");
        assert_eq!((t.agents_started, t.agents_running), (1, 1), "{t:?}");
        assert!(!t.notified, "nothing was running before this turn");
        assert_eq!(s.agents_running(), 1, "a Monitor watch is not an agent");
        let w = s.wait_idle(Duration::from_secs(20)).unwrap();
        assert!(w.idle, "{w:?}");
        assert_eq!(w.unsolicited, 1);
        assert_eq!(w.last_text, "The build finished.");
        assert_eq!(s.agents_running(), 0);
        assert!((s.cost_usd() - 0.75).abs() < 1e-9);
        let t2 = s.say("Move the saved basket on.").unwrap();
        assert_eq!(t2.final_text, "Done.");
        assert!(!t2.notified);
        s.end();
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
