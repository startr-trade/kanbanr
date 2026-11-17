//! Walkthroughs that run themselves (FTR-037): a guide's runs driven headlessly, decisions taken on
//! the scratch board by a scripted person, every question and board claim captured.
//!
//! The pieces, in the order a run uses them: `runbook` (the steps and the answer policy), `guard`
//! (the fence and `setup.log`), `remotes` (the run's own repositories), `driver` (the developer's
//! Claude Code session), `answer` (every question Claude asks), `person` (the scripted person on
//! the monitor), `truth` and `capture` (what was really so, and the raw record of it), `derive` (the
//! tables). `run` here strings them together: preflight, remotes, the config copy, the session, the
//! steps, then truth, snapshot, derive and the batch of checks prepared on the target board. Any
//! error after preflight still runs truth, snapshot and derive, then exits non-zero.

pub mod answer;
pub mod capture;
pub mod credentials;
pub mod derive;
pub mod driver;
pub mod guard;
pub mod person;
pub mod remotes;
pub mod runbook;
pub mod truth;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use answer::{Policy, Question};
use driver::{Session, SessionCfg, StopReason};
use guard::{Env, RemotesEnv, RunDirs, SetupLog};
use person::{Browser, Found, Monitor, is_no_browser};
use runbook::{Expect, Runbook, Step, lint};

/// `kanbanr walkthrough …`
#[derive(Subcommand)]
// `Run`'s arguments outgrew the other variants with `--rev` (DEF-089); the value is parsed once per
// process, so its size does not matter.
#[allow(clippy::large_enum_variant)]
pub enum WalkthroughCmd {
    /// Run a runbook headlessly: the developer's Claude Code session in a scratch project, every
    /// question answered from the runbook, every decision taken on the scratch board by the scripted
    /// person, everything captured, and the run's checks prepared on this board as one batch.
    Run(RunArgs),
    /// Check a runbook before a run: every step has an id and something to do, every check is
    /// defined, every decision names its offer, every answer rule compiles.
    Lint {
        /// A board doc (`walkthroughs/togaf.md`) or a file.
        runbook: String,
    },
    /// The PreToolUse hook a run gives its session: answers an AskUserQuestion from the runbook.
    /// Reads the tool call on stdin; silent outside a run.
    Answer,
    /// The repositories runs created on the remote server.
    #[command(subcommand)]
    Remotes(RemotesCmd),
    /// Re-check a run's `raw/MANIFEST.sha256`.
    Verify {
        /// The run folder.
        run: PathBuf,
    },
    /// Derive a run's tables (`results/decisions.tsv`, `results/metrics.md`) from `raw/` again.
    Derive {
        /// The run folder.
        run: PathBuf,
    },
    /// Prepare one batch on this board from several runs of one build (DEF-089, for FTR-054):
    /// their checks merged one per marker, every result at the runs' shared evidence revision.
    Prepare {
        /// The run folders (`walkthrough run --no-batch`), read from their `raw/` files only.
        #[arg(required = true)]
        runs: Vec<PathBuf>,
        /// The batch's name on this board.
        #[arg(long)]
        batch: String,
    },
}

#[derive(Subcommand)]
pub enum RemotesCmd {
    /// Every repository the runs created, oldest first.
    List {
        /// The runs folder (default: `--runs`'s default).
        #[arg(long)]
        runs: Option<PathBuf>,
    },
    /// Delete them with `gh repo delete --yes`; what cannot be deleted is listed for you.
    Clean {
        #[arg(long)]
        runs: Option<PathBuf>,
    },
}

#[derive(Args)]
pub struct RunArgs {
    /// A board doc (`walkthroughs/togaf.md`) or a file holding the runbook.
    pub runbook: String,
    /// The model for the session (default: the runbook's, `sonnet`).
    #[arg(long)]
    pub model: Option<String>,
    /// Stop the session at this spend (default: the runbook's, 40).
    #[arg(long)]
    pub budget_usd: Option<f64>,
    /// Stop the run after this many minutes of wall time (default: the runbook's, 180).
    #[arg(long)]
    pub max_minutes: Option<u64>,
    /// Make the run's two remotes as private repositories under this prefix with `gh repo create`
    /// (also `KANBANR_WALKTHROUGH_REMOTES`). Without it, two bare repositories under the run folder.
    #[arg(long)]
    pub remotes: Option<String>,
    /// The command that takes a screenshot of a page (default: chromium, google-chrome or
    /// chromium-browser on PATH, headless).
    #[arg(long)]
    pub screenshot_cmd: Option<String>,
    /// Wait at each decision step until you have acted on the monitor, instead of the scripted
    /// person acting.
    #[arg(long)]
    pub pause_at_decisions: bool,
    /// Where runs are kept (default: `KANBANR_WALKTHROUGH_RUNS`, else `~/.cache/kanbanr-walkthroughs`).
    #[arg(long)]
    pub runs: Option<PathBuf>,
    /// The run's name (default: `<runbook>-<date>-<time>`); also the batch's name on this board.
    #[arg(long)]
    pub name: Option<String>,
    /// The `claude` program to drive (default: `claude` on PATH).
    #[arg(long)]
    pub claude: Option<PathBuf>,
    /// A built monitor to serve, when this binary carries none.
    #[arg(long)]
    pub ui_dir: Option<PathBuf>,
    /// Do not open a browser: every press and page step is the "no browser" finding.
    #[arg(long)]
    pub no_browser: bool,
    /// The commit the build under test was made from, when the binary cannot say (a dirty or
    /// unknown build): every result the run puts on this board names it.
    #[arg(long)]
    pub rev: Option<String>,
    /// Put nothing on this board: the run's checks wait in its folder for `kanbanr walkthrough
    /// prepare`, which makes one batch of several runs of one build.
    #[arg(long)]
    pub no_batch: bool,
}

/// The most answers one step's say gives before it records a finding and goes on.
pub const MAX_ANSWERS_PER_STEP: usize = 20;

/// One finding of a run: never a stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub step: String,
    /// `offer-missing`, `truth-mismatch`, `question-unanswered`, `budget-stop`,
    /// `board-expect-failed`, `page-expect-failed`, `talk-expect-failed`, `no-browser`,
    /// `decision-failed`, `press-failed`, `push-failed`, `setup`, `offer-present` (a button the
    /// runbook says is absent for an item was offered), `offer-unchecked` (it could not be read).
    pub kind: &'static str,
    /// What the board (or the runbook) said.
    pub board: String,
    /// What was true.
    pub truth: String,
    /// The transcript line the finding was at.
    pub line: usize,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "- step {} {}: {}; truth: {} (transcript line {})",
            self.step, self.kind, self.board, self.truth, self.line
        )
    }
}

/// One `check` a step named, and how it came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub marker: String,
    pub step: String,
    pub passed: bool,
    pub why: String,
    /// The step checked something: a decision, a press, a push or an expectation. A check on a
    /// step that only says something is "not measured", never passed, and is not prepared.
    pub measured: bool,
}

/// The reason a check on a step that checked nothing carries.
pub const NOT_MEASURED: &str = "not measured: the step checked nothing";

/// The checks of a run, one per marker: a marker named on several steps passed only if every one
/// of them passed; `step` names them all (`2.2b+2.2c`), and the reason joins the failed steps',
/// each prefixed with its step id. Markers keep the order they were first named in.
pub fn merge_checks(checks: &[CheckResult]) -> Vec<CheckResult> {
    let mut order: Vec<String> = Vec::new();
    let mut by: BTreeMap<String, Vec<&CheckResult>> = BTreeMap::new();
    for c in checks {
        if !by.contains_key(&c.marker) {
            order.push(c.marker.clone());
        }
        by.entry(c.marker.clone()).or_default().push(c);
    }
    order
        .into_iter()
        .map(|marker| {
            let steps = &by[&marker];
            let measured: Vec<&&CheckResult> = steps.iter().filter(|c| c.measured).collect();
            let failed: Vec<String> = measured
                .iter()
                .filter(|c| !c.passed)
                .map(|c| format!("{}: {}", c.step, c.why))
                .collect();
            let is_measured = !measured.is_empty();
            let passed = is_measured && failed.is_empty();
            let why = if !is_measured {
                NOT_MEASURED.to_string()
            } else if !failed.is_empty() {
                failed.join("; ")
            } else if steps.len() > 1 {
                "every step went through as the guide says".into()
            } else {
                measured[0].why.clone()
            };
            CheckResult {
                marker,
                step: steps
                    .iter()
                    .map(|c| c.step.as_str())
                    .collect::<Vec<_>>()
                    .join("+"),
                passed,
                why,
                measured: is_measured,
            }
        })
        .collect()
}

/// The checks of several runs as one result per marker (DEF-089 R-4, for FTR-054): each result's
/// `step` is prefixed with its run's name (`togaf-v080-1:6.20a`), then [`merge_checks`] runs over
/// the whole list, so a marker passed only if every step that measured it, in every run, passed;
/// an unmeasured result beside a measured one is ignored, and failed reasons name their run's step.
pub fn checks_across(runs: &[(String, Vec<CheckResult>)]) -> Vec<CheckResult> {
    let all: Vec<CheckResult> = runs
        .iter()
        .flat_map(|(run, checks)| {
            checks.iter().map(move |c| CheckResult {
                step: format!("{run}:{}", c.step),
                ..c.clone()
            })
        })
        .collect();
    merge_checks(&all)
}

/// Where runs are kept by default: outside every repository and outside the temporary directory.
pub fn default_runs_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KANBANR_WALKTHROUGH_RUNS").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    kanbanr_core::project::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("kanbanr-walkthroughs")
}

/// The runbook's text: a file when `spec` names one, else the board doc of this project.
pub fn read_runbook(cli: &crate::Cli, spec: &str) -> Result<(String, String)> {
    let as_file = Path::new(spec);
    if as_file.is_file() {
        return Ok((
            std::fs::read_to_string(as_file)?,
            as_file.display().to_string(),
        ));
    }
    let data_dir = kanbanr_core::project::resolve_data_dir(cli.data_dir.as_deref());
    let project =
        kanbanr_core::project::resolve_project(cli.project.as_deref()).ok_or_else(|| {
            anyhow!(
                "{spec} is not a file, and there is no project here to read it from as a board doc"
            )
        })?;
    let path = kanbanr_core::Store::new(&data_dir)
        .project_dir(&project)
        .join("docs")
        .join(spec);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{spec} is neither a file nor a doc of {project} ({})",
            path.display()
        )
    })?;
    Ok((text, format!("{project} doc {spec}")))
}

/// `kanbanr walkthrough …`
pub fn run_command(cli: &crate::Cli, cmd: &WalkthroughCmd) -> Result<()> {
    match cmd {
        WalkthroughCmd::Answer => {
            let stdin = crate::hook_stdin();
            match answer::run_hook(&stdin) {
                Ok(Some(out)) => println!("{out}"),
                Ok(None) => {}
                Err(e) => println!("{}", answer::hook_error(&e)),
            }
            Ok(())
        }
        WalkthroughCmd::Lint { runbook } => {
            let (text, shown) = read_runbook(cli, runbook)?;
            let rb = Runbook::parse(&text)?;
            let errors = lint(&rb);
            for w in runbook::lint_warnings(&rb) {
                eprintln!("warning: {w}");
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "runbook": rb.name,
                        "steps": rb.steps.len(),
                        "checks": rb.checks.keys().collect::<Vec<_>>(),
                        "errors": errors.iter().map(|e| e.to_string()).collect::<Vec<_>>(),
                    })
                );
            } else if errors.is_empty() {
                println!(
                    "{shown}: runbook {} lints clean: {} steps, {} checks ({})",
                    rb.name,
                    rb.steps.len(),
                    rb.checks.len(),
                    rb.check_markers().join(", ")
                );
            } else {
                for e in &errors {
                    println!("{e}");
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                bail!("{} lint error(s) in {shown}", errors.len())
            }
        }
        WalkthroughCmd::Verify { run } => {
            capture::verify(&RunDirs::new(run))?;
            println!(
                "{}: every capture matches raw/MANIFEST.sha256",
                run.display()
            );
            Ok(())
        }
        WalkthroughCmd::Derive { run } => {
            let dirs = RunDirs::new(run);
            let d = derive::tables(&dirs)?;
            println!(
                "{}: {} decision row(s) in results/decisions.tsv, results/metrics.md written",
                run.display(),
                d.rows
            );
            Ok(())
        }
        WalkthroughCmd::Prepare { runs, batch } => prepare_runs(cli, runs, batch),
        WalkthroughCmd::Remotes(RemotesCmd::List { runs }) => {
            let dir = runs.clone().unwrap_or_else(default_runs_dir);
            let list = remotes::list(&dir);
            if list.is_empty() {
                println!("no repository was created by a run under {}", dir.display());
            }
            for c in list {
                println!("{}\t{}\t{}\t{}", c.at, c.run, c.which, c.url);
            }
            Ok(())
        }
        WalkthroughCmd::Remotes(RemotesCmd::Clean { runs }) => {
            let dir = runs.clone().unwrap_or_else(default_runs_dir);
            let cleaned = remotes::clean(&dir, Path::new("gh"))?;
            for c in cleaned.deleted {
                println!("deleted {}", c.url);
            }
            if !cleaned.left.is_empty() {
                println!("could not delete, remove these yourself:");
                for (c, why) in cleaned.left {
                    println!("  {}  ({why})", c.url);
                }
            }
            Ok(())
        }
        WalkthroughCmd::Run(args) => run(cli, args),
    }
}

/// `{board_remote}`, `{code_remote}`, `{monitor_url}`, `{today}` in a runbook text.
fn fill(text: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Open one of the run folder's append-only logs (a TSV, the transcript stream, a serve log,
/// `raw/findings.md`) to add to it, creating it if need be. Rows are added while the run goes on,
/// so the file is appended to, never rewritten (the one exception in `atomic_writes`).
pub(crate) fn append_to(path: impl AsRef<Path>) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true).open(path)
}

/// Append one finding to `raw/findings.md`.
fn append_finding(run: &RunDirs, f: &Finding) -> Result<()> {
    std::fs::create_dir_all(run.raw())?;
    let path = run.findings();
    let new = !path.exists();
    let mut file = append_to(&path)?;
    if new {
        writeln!(file, "# Findings\n")?;
    }
    writeln!(file, "{f}")?;
    Ok(())
}

/// Append one push to `raw/pushes.tsv`.
fn append_push(run: &RunDirs, repo: &str, refspec: &str, ok: bool, said: &str) -> Result<()> {
    std::fs::create_dir_all(run.raw())?;
    let path = run.raw().join("pushes.tsv");
    let new = !path.exists();
    let mut f = append_to(&path)?;
    if new {
        writeln!(f, "time\trepo\trefspec\tok\tsaid")?;
    }
    writeln!(
        f,
        "{}\t{repo}\t{refspec}\t{ok}\t{}",
        kanbanr_core::now_rfc3339(),
        said.replace(['\t', '\n', '\r'], " ")
    )?;
    Ok(())
}

/// Write `raw/checks.tsv`: every check a step named and how it came out.
fn write_checks(run: &RunDirs, checks: &[CheckResult]) -> Result<()> {
    let mut text = String::from("marker\tstep\tpassed\twhy\n");
    for c in checks {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            c.marker,
            c.step,
            if c.measured {
                c.passed.to_string()
            } else {
                "not-measured".into()
            },
            c.why.replace(['\t', '\n', '\r'], " ")
        ));
    }
    kanbanr_core::atomic::write(run.raw().join("checks.tsv"), text)?;
    Ok(())
}

/// Read `raw/checks.tsv` back: one result per marker, as the run wrote it.
fn read_checks(run: &RunDirs) -> Result<Vec<CheckResult>> {
    let path = run.raw().join("checks.tsv");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no checks in {}", path.display()))?;
    Ok(text
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut cols = l.splitn(4, '\t');
            let marker = cols.next().unwrap_or("").to_string();
            let step = cols.next().unwrap_or("").to_string();
            let passed = cols.next().unwrap_or("");
            let why = cols.next().unwrap_or("").to_string();
            CheckResult {
                marker,
                step,
                passed: passed == "true",
                why,
                measured: passed != "not-measured",
            }
        })
        .collect())
}

/// `kanbanr walkthrough prepare --batch <name> <run>…` (DEF-089 R-4, for FTR-054): one batch on
/// this board from several runs of one build. Each run's setup must have completed, its manifest
/// verify, and its `evidence_rev` be every other run's; a marker must mean the same check in
/// every runbook. Reads only the runs' files, so the batch is derived and repeatable; a record
/// already open in the batch is not prepared again.
fn prepare_runs(cli: &crate::Cli, runs: &[PathBuf], batch: &str) -> Result<()> {
    let mut rev: Option<(String, String)> = None;
    let mut defs: BTreeMap<String, (runbook::CheckRef, String)> = BTreeMap::new();
    let mut all: Vec<(String, Vec<CheckResult>)> = Vec::new();
    for run in runs {
        let dirs = RunDirs::new(run);
        let shown = run.display().to_string();
        let log = SetupLog::read(&dirs)
            .with_context(|| format!("{shown}: no raw/setup.log, so it is not a run folder"))?;
        if log.get("setup_failed").is_some() {
            bail!("{shown}: the run's setup did not complete, so it says nothing about the guide");
        }
        let Some(r) = log.get("evidence_rev") else {
            bail!(
                "{shown}: raw/setup.log names no evidence_rev (a run from before DEF-089), so its \
                 results could not say what they proved: run it again"
            );
        };
        match &rev {
            Some((a, at)) if a != r => {
                bail!("two builds are not one release's evidence: {a} ({at}) and {r} ({shown})")
            }
            Some(_) => {}
            None => rev = Some((r.to_string(), shown.clone())),
        }
        capture::verify(&dirs)
            .with_context(|| format!("{shown}: raw/MANIFEST.sha256 does not verify"))?;
        if let Some(s) = log.get("stopped") {
            println!("note: {shown} stopped early ({s}); what it did not reach is not measured");
        }
        let copy = dirs.root.join(answer::RUNBOOK_FILE);
        let (text, _) = read_runbook(cli, &copy.display().to_string())?;
        let rb = Runbook::parse(&text).with_context(|| format!("{shown}: its runbook"))?;
        for (marker, def) in &rb.checks {
            match defs.get(marker) {
                Some((prev, at)) if prev != def => bail!(
                    "{marker} is defined differently in {at} ({} {} \"{}\") and {shown} ({} {} \"{}\")",
                    prev.item,
                    prev.requirement,
                    prev.test,
                    def.item,
                    def.requirement,
                    def.test
                ),
                Some(_) => {}
                None => {
                    defs.insert(marker.clone(), (def.clone(), shown.clone()));
                }
            }
        }
        let name = log.get("run_name").map(str::to_string).unwrap_or_else(|| {
            run.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| shown.clone())
        });
        all.push((name, read_checks(&dirs)?));
    }
    let Some((rev, _)) = rev else {
        bail!("no run folder given");
    };
    let merged = checks_across(&all);
    let defs: BTreeMap<String, runbook::CheckRef> =
        defs.into_iter().map(|(m, (d, _))| (m, d)).collect();
    let target_board = kanbanr_core::project::resolve_data_dir(cli.data_dir.as_deref());
    let target_project = kanbanr_core::project::resolve_project(cli.project.as_deref());
    let o = prepare_batch(cli, &merged, &defs, batch, &target_board, &rev)?;
    println!(
        "{} run(s), {} check(s) ({} passed), evidence at {rev}",
        all.len(),
        merged.len(),
        merged.iter().filter(|c| c.passed).count()
    );
    print_batch(&o, batch, target_project.as_deref());
    Ok(())
}

/// The lines that say what a batch put on the target board.
fn print_batch(o: &BatchOutcome, batch: &str, target_project: Option<&str>) {
    if !o.prepared.is_empty() {
        println!(
            "prepared {} result(s) as batch {batch} on {}: confirm them at {}/p/{}/review",
            o.prepared.len(),
            target_project.unwrap_or("the board"),
            crate::monitor_url(),
            target_project.unwrap_or("<project>")
        );
    }
    for r in &o.recorded {
        println!("recorded walkthrough test: {r}");
    }
    for k in &o.skipped {
        println!("not put on the board: {k}");
    }
    if o.prepared.is_empty() && o.recorded.is_empty() {
        println!("no check to prepare on the target board");
    }
}

/// The run project's Claude Code configuration: a copy of the person's rules file and its imports,
/// their `settings.json`, the sign-in `creds` names (an access-only credentials file, or none: a
/// token reaches the session through its environment), and the skill this binary carries —
/// nothing else of theirs.
fn make_claude_config(
    run: &RunDirs,
    creds: &credentials::Credentials,
    log: &mut Vec<String>,
) -> Result<()> {
    let dst = run.claude_config();
    std::fs::create_dir_all(&dst)?;
    let Some(src) = crate::hooks::claude_dir() else {
        log.push(
            "no Claude Code configuration folder was found: the session starts with none".into(),
        );
        return Ok(());
    };
    let mut copied = Vec::new();
    // The person's credentials file is never copied whole: its refresh token rotates (RC2).
    credentials::install(&dst, creds)?;
    for name in ["CLAUDE.md", "settings.json"] {
        let from = src.join(name);
        if from.is_file() {
            std::fs::copy(&from, dst.join(name))?;
            copied.push(name.to_string());
        }
    }
    if let Ok(rules) = std::fs::read_to_string(src.join("CLAUDE.md")) {
        for import in guard::rule_imports(&rules) {
            let from = src.join(&import);
            if from.is_file() {
                std::fs::copy(&from, dst.join(&import))?;
                copied.push(import);
            }
        }
    }
    log.push(install_run_skill(run)?);
    log.push(format!("claude-config copied: {}", copied.join(", ")));
    Ok(())
}

/// The skill this binary carries, installed into the run's own config folder with the code
/// `kanbanr skill install` runs (T16, G-6): a run exercises the skill and the binary that ship
/// together, never the person's installed copy. A link or copy an earlier run left is replaced.
pub fn install_run_skill(run: &RunDirs) -> Result<String> {
    let dir = run.claude_config().join("skills").join("kanbanr");
    if let Ok(meta) = std::fs::symlink_metadata(&dir)
        && (meta.file_type().is_symlink() || !dir.join(".kanbanr-skill").exists())
    {
        if meta.file_type().is_symlink() || meta.is_file() {
            std::fs::remove_file(&dir)?;
        } else {
            std::fs::remove_dir_all(&dir)?;
        }
    }
    crate::skill::install_at(&dir)
}

/// The persona appended to the session's system prompt.
fn persona(process: &str) -> String {
    format!(
        "You are a developer following this project's guide for the {process} process, with kanbanr. \
         A person is working with you from the monitor: they take every decision there (approvals, \
         sign-offs, promotions, the cut, CI results, delegations) and you never take one yourself, \
         never record a verdict, and never ask them to type a command. Answer the person's questions \
         in your own words, briefly. When you need something only they can decide, ask one question \
         at a time, in text, with the options listed and the recommended one marked \
         \"(recommended)\". The monitor is already running with writes on; never start another \
         one, and never push unless the person says to.\n"
    )
}

/// The run of one runbook.
struct Running {
    runbook: Runbook,
    dirs: RunDirs,
    fence: PathBuf,
    remotes: RemotesEnv,
    project: Option<String>,
    monitor: Option<Monitor>,
    browser: Option<Browser>,
    browser_absent: Option<String>,
    no_browser: bool,
    session: Session,
    vars: BTreeMap<String, String>,
    findings: Vec<Finding>,
    checks: Vec<CheckResult>,
    started: Instant,
    max_minutes: u64,
    /// The last question Claude asked, for the finding when setup never completes.
    last_question: Option<String>,
    /// Setup never made the board: the run stops and prepares no batch.
    setup_failed: bool,
    /// The names steps bound (`names:`), to the item codes they stand for.
    names: BTreeMap<String, String>,
    lines: usize,
    stopped: Option<String>,
    pause: bool,
    screenshot_cmd: Option<String>,
    ui_dir: Option<PathBuf>,
    kanbanr: PathBuf,
    ci: Option<truth::CiRuns>,
    gh: Option<PathBuf>,
    /// The text questions of every step already run, in runbook order: one entry per step
    /// reached, a skipped one with an empty list (DEF-087). A talk claim's `since` reads it.
    asked_log: Vec<(String, Vec<String>)>,
}

/// The questions a talk claim reads (DEF-087), each with its step: `here` alone (step id `""`)
/// when `since` is None, else every question `log` holds from the step `since` names to the end,
/// then `here`. A `since` that `log` does not hold (lint stops one before a run) reads `here` alone.
pub fn asked_in_span<'a>(
    log: &'a [(String, Vec<String>)],
    since: Option<&str>,
    here: &'a [String],
) -> Vec<(&'a str, &'a str)> {
    let start = since.and_then(|s| log.iter().position(|(id, _)| id == s));
    let earlier = start.map(|j| &log[j..]).unwrap_or(&[]);
    earlier
        .iter()
        .flat_map(|(id, qs)| qs.iter().map(move |q| (id.as_str(), q.as_str())))
        .chain(here.iter().map(|q| ("", q.as_str())))
        .collect()
}

impl Running {
    fn finding(&mut self, step: &str, kind: &'static str, board: String, truth: String) {
        let f = Finding {
            step: step.to_string(),
            kind,
            board,
            truth,
            line: self.session.transcript_line(),
        };
        eprintln!("finding: {f}");
        let _ = append_finding(&self.dirs, &f);
        self.findings.push(f);
    }

    /// `approve_waiting` / `signoff_waiting` (preview-7 RC3): every item planned into the release
    /// that the page offers the decision for gets it, through the page's own route, one by one;
    /// a `swept-in` finding lists them, so the person sees what came along. An item the page does
    /// not offer it for is not waiting, and is left alone.
    fn sweep(
        &mut self,
        id: &str,
        project: &str,
        sweep: &runbook::Decision,
        offer: &runbook::Offer,
        ok: &mut bool,
        why: &mut Vec<String>,
    ) {
        let (release, name) = match sweep {
            runbook::Decision::ApproveWaiting { release } => (release, None),
            runbook::Decision::SignoffWaiting { release, name } => (release, Some(name)),
            _ => return,
        };
        let version = release.trim_start_matches('v');
        let planned: Vec<(String, String)> = kanbanr_core::Store::new(self.dirs.board())
            .load(project)
            .map(|p| {
                let mut f: Vec<_> = p
                    .features
                    .iter()
                    .filter(|f| {
                        f.release
                            .as_deref()
                            .is_some_and(|r| r.trim_start_matches('v') == version)
                    })
                    .collect();
                f.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.code.cmp(&b.code)));
                f.into_iter()
                    .map(|f| (f.code.clone(), f.title.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let Some(m) = self.monitor.as_ref() else {
            self.finding(
                id,
                "offer-missing",
                format!(
                    "the guide promises \"{}\" on the {} page",
                    offer.button, offer.page
                ),
                "no monitor".into(),
            );
            *ok = false;
            why.push("no monitor".into());
            return;
        };
        let (taken, failed) = m.sweep(
            project,
            &planned,
            name.map(String::as_str),
            offer,
            id,
            Some(&self.dirs),
            self.pause,
        );
        if !taken.is_empty() {
            self.finding(
                id,
                "swept-in",
                format!(
                    "{} for every item in {release} the page offers it for",
                    offer.button
                ),
                format!("taken for {}", taken.join(", ")),
            );
        }
        if !failed.is_empty() {
            self.finding(
                id,
                "decision-failed",
                format!("{} for the waiting items of {release}", offer.button),
                failed.join("; "),
            );
            *ok = false;
            why.push(failed.join("; "));
        }
    }

    /// The run board's items, code and title, oldest first; none before setup made the board.
    fn board_items(&mut self) -> Vec<(String, String)> {
        let project = self.project_name();
        let Ok(p) = kanbanr_core::Store::new(self.dirs.board()).load(&project) else {
            return Vec::new();
        };
        let mut items: Vec<_> = p.features.iter().collect();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.code.cmp(&b.code)));
        items
            .into_iter()
            .map(|f| (f.code.clone(), f.title.clone()))
            .collect()
    }

    fn project_name(&mut self) -> String {
        if self.project.is_none()
            && let Some(found) = kanbanr_core::project::find_marker(&self.dirs.project(), None)
            && let Some(p) = found.marker.project
        {
            self.project = Some(p);
        }
        self.project
            .clone()
            .unwrap_or_else(|| self.runbook.name.clone())
    }

    /// Start the monitor once the board exists (the first step makes it); started with the run's
    /// board named, so a board not yet made is still the one it serves.
    fn ensure_monitor(&mut self) -> bool {
        if self.monitor.is_some() {
            return true;
        }
        let project = self.project_name();
        match Monitor::start(
            &self.dirs,
            &self.kanbanr,
            &self.fence,
            self.ui_dir.as_deref(),
            Some(&self.dirs.board()),
        ) {
            Ok(m) => {
                if !m.has_token() {
                    self.finding(
                        "-",
                        "setup",
                        "the monitor serves no page with its token".into(),
                        "writes from the scripted person cannot go through; this binary carries no monitor (pass --ui-dir)".into(),
                    );
                }
                self.vars.insert("monitor_url".into(), m.url.clone());
                let _ = answer::write_vars(&self.dirs, &self.vars);
                eprintln!("monitor: {} (project {project})", m.url);
                self.monitor = Some(m);
                true
            }
            Err(e) => {
                self.finding("-", "setup", "no monitor".into(), e.to_string());
                false
            }
        }
    }

    fn browser(&mut self) -> Option<&Browser> {
        if self.no_browser {
            return None;
        }
        if self.browser.is_none() && self.browser_absent.is_none() {
            match Browser::open() {
                Ok(b) => self.browser = Some(b),
                Err(e) if is_no_browser(&e) => self.browser_absent = Some(e.to_string()),
                Err(e) => self.browser_absent = Some(format!("the browser failed: {e}")),
            }
        }
        self.browser.as_ref()
    }

    /// A screenshot of a page before or after a decision: through the browser when one is open,
    /// else with the screenshot command; silently none when neither exists.
    fn screenshot(&mut self, step: &str, page: &str, when: &str) {
        if self.monitor.is_none() {
            return;
        }
        let project = self.project_name();
        let url = self
            .monitor
            .as_ref()
            .map(|m| m.page_url(&project, page))
            .unwrap_or_default();
        let file = capture::screenshots_dir(&self.dirs).join(format!("{step}-{when}-{page}.png"));
        if let Some(b) = self.browser() {
            if let Ok(p) = b.page(&url) {
                let _ = p.screenshot(&file);
            }
            return;
        }
        let cmd = self.screenshot_cmd.clone();
        if let Some(m) = self.monitor.as_ref()
            && let Err(e) = m.screenshot(&url, &file, cmd.as_deref())
        {
            eprintln!("screenshot {step} {when}: {e}");
        }
    }

    /// Wait, within the run's time left, for the session's background agents to finish and the
    /// stream to go quiet; a timeout is a finding and the run goes on.
    fn wait_for_agents(&mut self, step: &str) {
        if self.session.agents_running() == 0 {
            return;
        }
        let left =
            Duration::from_secs(self.max_minutes * 60).saturating_sub(self.started.elapsed());
        eprintln!(
            "step {step}: waiting for {} background agent(s) to finish",
            self.session.agents_running()
        );
        match self.session.wait_idle(left) {
            Ok(w) => {
                self.lines += w.lines;
                if !w.idle {
                    self.finding(
                        step,
                        "agents-still-running",
                        format!(
                            "{} background agent(s) still running",
                            self.session.agents_running()
                        ),
                        "the run's time ran out waiting; the run goes on".into(),
                    );
                }
            }
            Err(e) => eprintln!("step {step}: waiting for the session: {e}"),
        }
    }

    fn over_time(&self) -> bool {
        self.started.elapsed() > Duration::from_secs(self.max_minutes * 60)
    }

    /// One step. `Ok(false)` when the run must stop (budget, wall time, the session gone). Every
    /// step reached leaves its questions in `asked_log`, whichever way it returns.
    fn step(&mut self, step: &Step) -> Result<bool> {
        let mut asked_here: Vec<String> = Vec::new();
        let out = self.step_inner(step, &mut asked_here);
        self.asked_log.push((step.id.clone(), asked_here));
        out
    }

    fn step_inner(&mut self, step: &Step, asked_here: &mut Vec<String>) -> Result<bool> {
        let id = step.id.clone();
        answer::set_current_step(&self.dirs, &id)?;
        if self.over_time() {
            self.finding(
                &id,
                "budget-stop",
                format!("the run's wall time passed {} minutes", self.max_minutes),
                "the session is stopped; the captures follow".into(),
            );
            self.session.kill();
            self.stopped = Some("wall time".into());
            return Ok(false);
        }
        // A step that runs only when the board holds a claim (the defect step, once the release is
        // a candidate): otherwise it is skipped, and its check measures nothing.
        if let Some(claim) = &step.only_if {
            let project = self.project_name();
            let holds = truth::board_claim(&self.dirs.board(), &project, claim);
            if let Some(instead) = match holds {
                Ok(Ok(())) => None,
                Ok(Err(instead)) => Some(instead),
                Err(e) => Some(e.to_string()),
            } {
                self.finding(
                    &id,
                    "skipped",
                    format!("the step runs only if {claim:?}"),
                    instead.clone(),
                );
                if let Some(marker) = &step.check {
                    self.checks.push(CheckResult {
                        marker: marker.clone(),
                        step: id.clone(),
                        passed: false,
                        why: format!("{NOT_MEASURED} (skipped: {instead})"),
                        measured: false,
                    });
                }
                return Ok(true);
            }
        }
        let mut step_ok = true;
        let mut why = Vec::new();
        let mut final_text = String::new();
        // RC3: a step starts on an idle session — no sub-agent build still running, nothing still
        // arriving — so its say is not answered by a turn about earlier work.
        self.wait_for_agents(&id);
        // The board's items as the step starts, for the names it binds from what it creates.
        let items_before = if step.names.is_empty() {
            Vec::new()
        } else {
            self.board_items()
        };
        // Answers given in this step, against the step's limit of MAX_ANSWERS_PER_STEP.
        let mut answered = 0usize;
        if let Some(say) = &step.say {
            let mut resends = 0;
            let mut text = fill(say, &self.vars);
            // A step's say goes on while Claude asks: a setup interview asks one question per turn.
            // The same question answered three times without settling is a finding; twenty
            // answers in one step is a runaway guard.
            let mut seen: Vec<(String, u32)> = Vec::new();
            for round in 0..=MAX_ANSWERS_PER_STEP {
                let turn = self.session.say(&text)?;
                self.lines += turn.lines;
                final_text = turn.final_text.clone();
                if let Some(stop) = &turn.stopped {
                    let kind = match stop {
                        StopReason::Budget => "budget-stop",
                        StopReason::Auth(_) => "harness-auth",
                        _ => "session-stop",
                    };
                    // A sign-in failure is the harness's, never the product's: the run snapshots
                    // and prepares nothing on the target board.
                    if matches!(stop, StopReason::Auth(_)) {
                        self.setup_failed = true;
                    }
                    self.finding(
                        &id,
                        kind,
                        format!("the session stopped: {stop}"),
                        "the captures follow".into(),
                    );
                    self.stopped = Some(stop.to_string());
                    return Ok(false);
                }
                if turn.agents_running > 0 || turn.notified {
                    self.wait_for_agents(&id);
                }
                // A reply that came while a background agent was running may be about that agent,
                // not about this step: once the session is idle, the step is said again (twice at most).
                if turn.notified && turn.agents_started == 0 && turn.asked.is_none() && resends < 2
                {
                    resends += 1;
                    eprintln!(
                        "step {id}: the reply came during background work; saying the step again"
                    );
                    text = fill(say, &self.vars);
                    continue;
                }
                let Some(q) = &turn.asked else { break };
                asked_here.push(q.text.clone());
                self.last_question = Some(q.text.clone());
                let key = q.text.to_lowercase();
                let times = match seen.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => {
                        *n += 1;
                        *n
                    }
                    None => {
                        seen.push((key, 1));
                        1
                    }
                };
                if times > 3 || round == MAX_ANSWERS_PER_STEP {
                    self.finding(
                        &id,
                        "question-unanswered",
                        format!("Claude kept asking: {}", q.text),
                        if times > 3 {
                            "three answers did not settle it; the run goes on".into()
                        } else {
                            format!("{MAX_ANSWERS_PER_STEP} answers in one step; the run goes on")
                        },
                    );
                    step_ok = false;
                    why.push("a question was not settled".into());
                    break;
                }
                let question = Question::from_text(q);
                let picked = Policy::new(&self.runbook, Some(&id))
                    .with_vars(self.vars.clone())
                    .answer(&question);
                answer::log(&self.dirs, &id, &question, &picked)?;
                eprintln!(
                    "step {id}: Claude asked \"{}\" -> {} ({})",
                    q.text, picked.pick, picked.policy
                );
                answered += 1;
                text = picked.pick;
            }
        }
        // The monitor starts once the first step has made the board.
        if (step.decide.is_some()
            || !step.presses().is_empty()
            || step.expect.is_some()
            || step.say.is_some())
            && !self.ensure_monitor()
        {
            step_ok = false;
            why.push("no monitor".into());
        }
        // Names (preview-5 triage): the step's say is done, so bind the names it gives, then read
        // every `named:` selector of its decision, press, offer and claims as the bound code. A
        // name never bound leaves those undone: a finding, never a guess.
        if !step.names.is_empty() {
            let items_after = self.board_items();
            for (name, source) in &step.names {
                let bound = self.names.contains_key(name);
                match runbook::bind_name(source, &items_before, &items_after, bound) {
                    None => {}
                    Some(Ok(code)) => {
                        eprintln!("step {id}: `{name}` is {code}");
                        self.names.insert(name.clone(), code);
                    }
                    Some(Err(candidates)) => {
                        self.finding(
                            &id,
                            "naming-failed",
                            format!("the step names one item `{name}`"),
                            candidates.clone(),
                        );
                        step_ok = false;
                        why.push(format!("`{name}` not bound: {candidates}"));
                    }
                }
            }
        }
        let original = step;
        let resolved = match step.with_names(&self.names) {
            Ok(s) => s,
            Err(unbound) => {
                for n in &unbound {
                    self.finding(
                        &id,
                        "unbound-name",
                        format!("the step acts on the item named `{n}`"),
                        format!("no step bound `{n}`; its decision and claims were not taken"),
                    );
                    why.push(format!("`{n}` was never bound"));
                }
                step_ok = false;
                let mut s = step.clone();
                s.decide = None;
                s.press = None;
                s.offer = None;
                s.expect = None;
                s
            }
        };
        let step = &resolved;
        let project = self.project_name();
        if let Some(
            sweep @ (runbook::Decision::ApproveWaiting { .. }
            | runbook::Decision::SignoffWaiting { .. }),
        ) = &step.decide
        {
            let offer = step
                .offer
                .clone()
                .expect("lint: a decide step has an offer");
            self.screenshot(&id, &offer.page, "before");
            self.sweep(&id, &project, sweep, &offer, &mut step_ok, &mut why);
            self.screenshot(&id, &offer.page, "after");
        } else if let Some(decision) = &step.decide {
            let offer = step
                .offer
                .clone()
                .expect("lint: a decide step has an offer");
            // A sign-off on an item whose approval lapsed: approve first, as the guide says. The
            // person needs to know the lapse happened; it is not a failure.
            if let (runbook::Decision::Signoff { item, .. }, Some(m), false) =
                (decision, self.monitor.as_ref(), self.pause)
            {
                let by = m.approver().unwrap_or_else(|_| "the person".into());
                match m.reapprove_if_lapsed(&project, item, &by, &id) {
                    Ok(Some(code)) => self.finding(
                        &id,
                        "reapproved",
                        format!("{code}'s approval had lapsed: the Review page showed Approve"),
                        "approved again before the sign-off, as the guide says".into(),
                    ),
                    Ok(None) => {}
                    Err(e) => self.finding(
                        &id,
                        "decision-failed",
                        "approving again before the sign-off".into(),
                        e.to_string(),
                    ),
                }
            }
            self.screenshot(&id, &offer.page, "before");
            let found = match self.monitor.as_ref() {
                Some(m) => m.offers(&project, &offer, Some(decision)),
                None => Ok(Found::Absent("no monitor".into())),
            };
            match found {
                Ok(Found::Offered) => {
                    if self.pause {
                        println!(
                            "step {id}: waiting for you to press \"{}\" on the {} page ({})",
                            offer.button,
                            offer.page,
                            self.monitor
                                .as_ref()
                                .map(|m| m.page_url(&project, &offer.page))
                                .unwrap_or_default()
                        );
                        let changed = self
                            .monitor
                            .as_ref()
                            .map(|m| {
                                m.wait_for_change(
                                    &project,
                                    decision,
                                    Duration::from_secs(5),
                                    Duration::from_secs(60 * 60),
                                )
                            })
                            .transpose()?
                            .unwrap_or(false);
                        if !changed {
                            self.finding(
                                &id,
                                "decision-failed",
                                format!("{} was not taken", decision.kind()),
                                "nothing changed within an hour".into(),
                            );
                            step_ok = false;
                            why.push("the decision was not taken".into());
                        }
                    } else {
                        let by = self
                            .monitor
                            .as_ref()
                            .map(|m| m.approver())
                            .transpose()
                            .unwrap_or(None)
                            .unwrap_or_else(|| "the person".into());
                        let out = self
                            .monitor
                            .as_ref()
                            .map(|m| m.decide(&project, decision, &by, &id, Some(&self.dirs)))
                            .transpose();
                        match out {
                            Ok(Some(o)) if o.ok() => {
                                eprintln!("step {id}: {} -> {}", o.route, o.status);
                            }
                            Ok(Some(o)) => {
                                self.finding(
                                    &id,
                                    "decision-failed",
                                    format!("{} answered {}", o.route, o.status),
                                    o.body.chars().take(300).collect(),
                                );
                                step_ok = false;
                                why.push(format!("the route answered {}", o.status));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                self.finding(
                                    &id,
                                    "decision-failed",
                                    decision.kind().into(),
                                    e.to_string(),
                                );
                                step_ok = false;
                                why.push(e.to_string());
                            }
                        }
                    }
                }
                Ok(Found::Absent(instead)) => {
                    self.finding(
                        &id,
                        "offer-missing",
                        format!(
                            "the guide promises \"{}\" on the {} page",
                            offer.button, offer.page
                        ),
                        instead,
                    );
                    step_ok = false;
                    why.push("the offer was missing".into());
                }
                Err(e) => {
                    let kind = if e.to_string().starts_with("ambiguous") {
                        "ambiguous-selector"
                    } else {
                        "offer-missing"
                    };
                    self.finding(
                        &id,
                        kind,
                        format!("\"{}\" on the {} page", offer.button, offer.page),
                        e.to_string(),
                    );
                    step_ok = false;
                    why.push(e.to_string());
                }
            }
            self.screenshot(&id, &offer.page, "after");
        }
        if let Some(first) = step.presses().first() {
            // FTR-063: every press's subject is resolved first; an ambiguous one is never pressed
            // for, and then no press of the list is made.
            let mut subjects: Vec<Option<String>> = Vec::with_capacity(step.presses().len());
            let mut ambiguous = false;
            for press in step.presses() {
                let subject = match (&press.subject, self.monitor.as_ref()) {
                    (Some(sel), Some(m)) => match m.resolve(&project, sel) {
                        Ok(found) => found.map(|i| i.code),
                        Err(e) => {
                            self.finding(
                                &id,
                                "ambiguous-selector",
                                format!("press \"{}\"", press.button),
                                e.to_string(),
                            );
                            step_ok = false;
                            why.push("the press's subject was ambiguous".into());
                            ambiguous = true;
                            None
                        }
                    },
                    _ => None,
                };
                subjects.push(subject);
            }
            // `press "<b1>", then "<b2>" on <page>`: one press reads as before.
            let buttons = step
                .presses()
                .iter()
                .map(|p| format!("\"{}\"", p.button))
                .collect::<Vec<_>>()
                .join(", then ");
            let what = format!("press {buttons} on {}", first.page);
            let url = self
                .monitor
                .as_ref()
                .map(|m| m.page_url(&project, &first.page))
                .unwrap_or_default();
            if !ambiguous {
                let presses: Vec<(&str, Option<&str>)> = step
                    .presses()
                    .iter()
                    .zip(&subjects)
                    .map(|(p, s)| (p.button.as_str(), s.as_deref()))
                    .collect();
                let pressed = match self.browser() {
                    None => Err(None),
                    Some(b) => b
                        .page(&url)
                        .and_then(|p| p.press_in_turn(&presses))
                        .map_err(Some),
                };
                match pressed {
                    Ok(()) => {
                        eprintln!("step {id}: pressed {buttons} on {}", first.page);
                        // The click posts on its own; give the board a moment to change.
                        std::thread::sleep(Duration::from_secs(2));
                    }
                    Err(None) => {
                        let why_not = self
                            .browser_absent
                            .clone()
                            .unwrap_or_else(|| "--no-browser".into());
                        self.finding(&id, "no-browser", what, why_not);
                        step_ok = false;
                        why.push("no browser".into());
                    }
                    Err(Some(e)) => {
                        self.finding(&id, "press-failed", what, e.to_string());
                        step_ok = false;
                        why.push(e.to_string());
                    }
                }
            }
        }
        if let Some(push) = &step.push {
            let (repo, remote) = match push.repo {
                runbook::CodeOrBoard::Code => (self.dirs.project(), self.remotes.code.clone()),
                runbook::CodeOrBoard::Board => (self.dirs.board(), self.remotes.board.clone()),
            };
            let which = match push.repo {
                runbook::CodeOrBoard::Code => "code",
                runbook::CodeOrBoard::Board => "board",
            };
            // `{board_branch}` / `{code_branch}`: the repository's own branch, whatever git named it.
            let mut vars = self.vars.clone();
            for (k, dir) in [
                ("board_branch", self.dirs.board()),
                ("code_branch", self.dirs.project()),
            ] {
                if let Some(b) = kanbanr_core::coderepo::current_branch(&dir) {
                    vars.insert(k.into(), b);
                }
            }
            let refspec = fill(&push.refspec, &vars);
            match guard::git_push(&repo, &remote, &refspec) {
                Ok(said) => {
                    append_push(&self.dirs, which, &refspec, true, &said)?;
                    eprintln!("step {id}: pushed {which} {refspec}");
                }
                Err(e) => {
                    append_push(&self.dirs, which, &refspec, false, &e.to_string())?;
                    self.finding(
                        &id,
                        "push-failed",
                        format!("push {which} {refspec}"),
                        e.to_string(),
                    );
                    step_ok = false;
                    why.push("the push failed".into());
                }
            }
        }
        if let Some(expect) = &step.expect {
            self.expect(&id, expect, asked_here, &final_text, &mut step_ok, &mut why);
        }
        // A board that setup never made fails every later step for the same reason: stop here,
        // say what Claude last asked, and prepare nothing on the target board. While Claude's last
        // message still asks something (preview-6: a charter draft ending "Confirm this draft…"),
        // it is answered again first, within the step's answers.
        let claims_project = step
            .expect
            .as_ref()
            .is_some_and(|e| e.board == Some(runbook::BoardClaim::ProjectExists));
        let no_project = |me: &mut Self| {
            truth::board_claim(
                &me.dirs.board(),
                &me.project_name(),
                &runbook::BoardClaim::ProjectExists,
            )
            .map(|r| r.is_err())
            .unwrap_or(true)
        };
        while claims_project
            && answered < MAX_ANSWERS_PER_STEP
            && no_project(self)
            && let Some(q) = driver::parse_text_question(&final_text)
        {
            asked_here.push(q.text.clone());
            self.last_question = Some(q.text.clone());
            let question = Question::from_text(&q);
            let picked = Policy::new(&self.runbook, Some(&id))
                .with_vars(self.vars.clone())
                .answer(&question);
            answer::log(&self.dirs, &id, &question, &picked)?;
            eprintln!(
                "step {id}: setup is not done and Claude still asks \"{}\" -> {} ({})",
                q.text, picked.pick, picked.policy
            );
            answered += 1;
            let turn = self.session.say(&picked.pick)?;
            self.lines += turn.lines;
            final_text = turn.final_text.clone();
            if turn.stopped.is_some() {
                break;
            }
            if turn.agents_running > 0 || turn.notified {
                self.wait_for_agents(&id);
            }
        }
        if claims_project && no_project(self) {
            let last = self
                .last_question
                .clone()
                .unwrap_or_else(|| "none (Claude asked nothing)".into());
            self.finding(
                &id,
                "setup-incomplete",
                format!("setup did not complete: Claude's last question was \"{last}\""),
                "the run stops; no batch is prepared".into(),
            );
            self.setup_failed = true;
            self.stopped = Some(format!(
                "setup did not complete: Claude's last question was \"{last}\""
            ));
            return Ok(false);
        }
        if let Some(marker) = &step.check {
            // Measured by what the runbook asked of the step, even when an unbound name left it
            // undone: that is a failed check, not an unmeasured one.
            let measured = original.decide.is_some()
                || !original.presses().is_empty()
                || original.push.is_some()
                || original.expect.as_ref().is_some_and(|e| !e.is_empty());
            self.checks.push(CheckResult {
                marker: marker.clone(),
                step: id.clone(),
                passed: step_ok && measured,
                why: if !measured {
                    NOT_MEASURED.into()
                } else if step_ok {
                    "the step went through as the guide says".into()
                } else {
                    why.join("; ")
                },
                measured,
            });
            eprintln!(
                "step {id}: check {marker} {}",
                if step_ok { "passed" } else { "failed" }
            );
        }
        Ok(true)
    }

    fn expect(
        &mut self,
        id: &str,
        expect: &Expect,
        asked: &[String],
        final_text: &str,
        ok: &mut bool,
        why: &mut Vec<String>,
    ) {
        let project = self.project_name();
        if let Some(claim) = &expect.board {
            match truth::board_claim(&self.dirs.board(), &project, claim) {
                Ok(Ok(())) => {}
                Ok(Err(instead)) => {
                    self.finding(id, "board-expect-failed", format!("{claim:?}"), instead);
                    *ok = false;
                    why.push("a board claim failed".into());
                }
                Err(e) => {
                    self.finding(
                        id,
                        "board-expect-failed",
                        format!("{claim:?}"),
                        e.to_string(),
                    );
                    *ok = false;
                    why.push(e.to_string());
                }
            }
        }
        if let Some(claim) = &expect.truth {
            let line = truth::claim(
                &self.dirs,
                &project,
                &self.remotes,
                self.ci.as_ref(),
                &self.kanbanr,
                claim,
            );
            match line.verdict {
                truth::Verdict::Ok => {}
                truth::Verdict::Mismatch => {
                    self.finding(id, "truth-mismatch", line.board.clone(), line.truth.clone());
                    *ok = false;
                    why.push("a truth claim failed".into());
                }
                truth::Verdict::Unknown => eprintln!("step {id}: truth unknown: {line}"),
            }
        }
        if let Some(claim) = &expect.page {
            let url = if claim.url.starts_with("http") {
                claim.url.clone()
            } else {
                let path = claim.url.replace("{project}", &project);
                format!(
                    "{}{}",
                    self.monitor
                        .as_ref()
                        .map(|m| m.url.clone())
                        .unwrap_or_default(),
                    path
                )
            };
            let text = match self.browser() {
                None => Err(None),
                Some(b) => b.page(&url).and_then(|p| p.text()).map_err(Some),
            };
            match text {
                Ok(text) => {
                    let missing = person::check_page_claim(&text, claim);
                    if !missing.is_empty() {
                        self.finding(
                            id,
                            "page-expect-failed",
                            claim.url.clone(),
                            missing.join("; "),
                        );
                        *ok = false;
                        why.push("a page claim failed".into());
                    }
                }
                Err(None) => {
                    let why_not = self
                        .browser_absent
                        .clone()
                        .unwrap_or_else(|| "--no-browser".into());
                    self.finding(id, "no-browser", format!("read {}", claim.url), why_not);
                    *ok = false;
                    why.push("no browser".into());
                }
                Err(Some(e)) => {
                    self.finding(id, "page-expect-failed", claim.url.clone(), e.to_string());
                    *ok = false;
                    why.push(e.to_string());
                }
            }
        }
        // DEF-076 Q1: an item-scoped button the page must not offer.
        if let Some(offer) = &expect.offer
            && expect.absent
        {
            let found = match self.monitor.as_ref() {
                Some(m) => {
                    let known = match &offer.subject {
                        Some(sel) => m.resolve(&project, sel).map(|r| r.is_some()),
                        None => Ok(true),
                    };
                    known.and_then(|known| {
                        if known {
                            m.offers(&project, offer, None)
                                .map(|f| (true, matches!(f, Found::Offered)))
                        } else {
                            Ok((false, false))
                        }
                    })
                }
                None => Err(anyhow!("no monitor")),
            };
            let board = format!(
                "the guide says \"{}\" is not offered on the {} page",
                offer.button, offer.page
            );
            match found {
                Ok((known, offered)) => {
                    if let Some((kind, truth)) = offer.judge_absent(known, offered) {
                        self.finding(id, kind, board, truth);
                        *ok = false;
                        why.push(if known {
                            "a button the guide says is absent was offered".into()
                        } else {
                            "the absent-offer claim's subject matches no item".into()
                        });
                    }
                }
                Err(e) => {
                    let kind = if e.to_string().starts_with("ambiguous") {
                        "ambiguous-selector"
                    } else {
                        "offer-unchecked"
                    };
                    self.finding(id, kind, board, e.to_string());
                    *ok = false;
                    why.push(e.to_string());
                }
            }
        }
        if let Some(talk) = &expect.talk {
            let re = |p: &str| {
                regex::RegexBuilder::new(p)
                    .case_insensitive(true)
                    .build()
                    .ok()
            };
            // DEF-087: with `since`, the questions of every step from that one to this.
            let since = talk.since.as_deref();
            let log = std::mem::take(&mut self.asked_log);
            let span: Vec<(String, String)> = asked_in_span(&log, since, asked)
                .into_iter()
                .map(|(step, q)| {
                    (
                        if step.is_empty() { id } else { step }.to_string(),
                        q.to_string(),
                    )
                })
                .collect();
            self.asked_log = log;
            if let Some(p) = &talk.asked
                && let Some(r) = re(p)
                && !span.iter().any(|(_, q)| r.is_match(q))
            {
                let (board, truth) = match since {
                    None => (
                        format!("a question matching /{p}/ was expected"),
                        format!(
                            "asked: {}",
                            if asked.is_empty() {
                                "nothing".to_string()
                            } else {
                                asked.join(" | ")
                            }
                        ),
                    ),
                    Some(s) => (
                        format!("a question matching /{p}/ was expected in steps {s}–{id}"),
                        format!(
                            "asked: {}",
                            if span.is_empty() {
                                "nothing".to_string()
                            } else {
                                span.iter()
                                    .map(|(step, q)| format!("{step}: {q}"))
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                            }
                        ),
                    ),
                };
                self.finding(id, "talk-expect-failed", board, truth);
                *ok = false;
                why.push("the expected question was not asked".into());
            }
            if let Some(p) = &talk.not_asked
                && let Some(r) = re(p)
                && let Some((step, q)) = span.iter().find(|(_, q)| r.is_match(q))
            {
                self.finding(
                    id,
                    "talk-expect-failed",
                    format!("no question matching /{p}/ was expected"),
                    match since {
                        None => format!("asked: {q}"),
                        Some(_) => format!("asked at {step}: {q}"),
                    },
                );
                *ok = false;
                why.push("an unwanted question was asked".into());
            }
            if let Some(p) = &talk.said
                && let Some(r) = re(p)
                && !r.is_match(final_text)
            {
                self.finding(
                    id,
                    "talk-expect-failed",
                    format!("the answer was expected to match /{p}/"),
                    final_text.chars().take(300).collect(),
                );
                *ok = false;
                why.push("the answer did not say what was expected".into());
            }
        }
    }
}

/// `kanbanr walkthrough run`.
pub fn run(cli: &crate::Cli, args: &RunArgs) -> Result<()> {
    let (text, shown) = read_runbook(cli, &args.runbook)?;
    let runbook = Runbook::parse(&text)?;
    for w in runbook::lint_warnings(&runbook) {
        eprintln!("warning: {w}");
    }
    let errors = lint(&runbook);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("{e}");
        }
        bail!(
            "{shown} has {} lint error(s); fix them before a run",
            errors.len()
        );
    }
    // The commit every result names: the build's own, else the one `--rev` states. Refused here,
    // before any remote, session or spend exists (DEF-089).
    let (rev, rev_from) =
        evidence_rev(env!("KANBANR_GIT_SHA"), args.rev.as_deref()).map_err(|e| anyhow!(e))?;
    let runs = args.runs.clone().unwrap_or_else(default_runs_dir);
    std::fs::create_dir_all(&runs)?;
    let name = args.name.clone().unwrap_or_else(|| {
        let now = kanbanr_core::now_rfc3339();
        let stamp: String = now
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(14)
            .collect();
        format!(
            "{}-{}-{}",
            runbook.name,
            &stamp[..8.min(stamp.len())],
            &stamp[8.min(stamp.len())..]
        )
    });
    let dirs = RunDirs::new(&runs.join(&name));
    dirs.make()?;
    // One run at a time per run folder.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dirs.lock_file())?;
    {
        use fs2::FileExt;
        lock.try_lock_exclusive()
            .map_err(|_| anyhow!("another run is using {}", dirs.root.display()))?;
    }
    // The fence: the person's, else the runs folder. Everything the run spawns gets it.
    let fence =
        kanbanr_core::project::fence().unwrap_or_else(|| kanbanr_core::project::normalize(&runs));
    let fence = kanbanr_core::project::normalize(&fence);
    kanbanr_core::atomic::write(dirs.root.join(answer::RUNBOOK_FILE), &text)?;
    let mut setup_notes: Vec<String> = Vec::new();
    // Remotes.
    let prefix = args.remotes.clone().or_else(|| {
        std::env::var("KANBANR_WALKTHROUGH_REMOTES")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let gh = which("gh");
    if prefix.is_some() && gh.is_none() {
        bail!("--remotes needs `gh` on PATH to create the repositories");
    }
    let remotes = remotes::create(
        &dirs,
        prefix.as_deref(),
        gh.as_deref().unwrap_or(Path::new("gh")),
    )?;
    if prefix.is_none() {
        setup_notes.push(
            "remotes: two bare repositories under the run folder; CI truth is unknown".into(),
        );
    }
    // The run project: a repository with the code remote, and the run's Claude Code settings.
    let project_dir = dirs.project();
    if !project_dir.join(".git").exists() {
        let git = |args: &[&str]| {
            kanbanr_core::coderepo::git(&project_dir, args).map_err(|e| anyhow!("{e}"))
        };
        git(&["init", "-q", "-b", "main"])?;
        // The session's commits need an identity; the run's own, never the person's.
        git(&["config", "user.name", "walkthrough"])?;
        git(&["config", "user.email", "walkthrough@localhost"])?;
        git(&["remote", "add", "origin", &remotes.code])?;
        // One commit, so `main` exists before the guide's first push.
        kanbanr_core::atomic::write(
            project_dir.join("README.md"),
            format!(
                "# {}\n\nA scratch project of the walkthrough run {name}.\n",
                runbook.name
            ),
        )?;
        git(&["add", "README.md"])?;
        git(&["commit", "-qm", "walkthrough: the scratch project"])?;
    }
    crate::hooks::write_walkthrough_settings(&project_dir)?;
    // How the session signs in (RC2): the token file, else an access-only copy that outlives the
    // run, else none; refused before anything starts.
    let creds = credentials::resolve(
        &fence,
        crate::hooks::claude_dir().as_deref(),
        Duration::from_secs(args.max_minutes.unwrap_or(runbook.max_minutes) * 60),
    )?;
    make_claude_config(&dirs, &creds, &mut setup_notes)?;
    // Preflight and setup.log.
    let model = args.model.clone().unwrap_or_else(|| runbook.model.clone());
    let target_board = kanbanr_core::project::resolve_data_dir(cli.data_dir.as_deref());
    let target_project = kanbanr_core::project::resolve_project(cli.project.as_deref());
    let kanbanr = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kanbanr"));
    let env = Env {
        fence: Some(fence.clone()),
        target_board: Some(target_board.clone()),
        claude: args
            .claude
            .clone()
            .unwrap_or_else(|| PathBuf::from("claude")),
        model: model.clone(),
        kanbanr: kanbanr.clone(),
        person_skill: crate::hooks::claude_dir().map(|d| d.join("skills").join("kanbanr")),
        session_path: Some(guard::run_bin(&dirs, &kanbanr)?),
    };
    let mut log = guard::preflight(&dirs, &env)?;
    log.push("runbook", shown.clone());
    log.push("run_name", name.clone());
    log.push("credentials", creds.mode());
    if let Some(p) = &target_project {
        log.push("target_project", p.clone());
    }
    for n in &setup_notes {
        log.push("note", n.clone());
    }
    log.push("evidence_rev", rev.clone());
    log.push("evidence_rev_from", rev_from.to_string());
    log.write(&dirs)?;
    let allowed = guard::allowed_remotes(&dirs);
    // The persona and the session.
    let persona_file = dirs.root.join("persona.md");
    kanbanr_core::atomic::write(&persona_file, persona(&runbook.process))?;
    let mut cfg = SessionCfg::new(&project_dir, &dirs.claude_config(), &fence, &dirs.root);
    cfg.claude = env.claude.clone();
    cfg.model = model.clone();
    cfg.budget_usd = args.budget_usd.unwrap_or(runbook.budget_usd);
    cfg.persona_file = Some(persona_file);
    // The session and every hook it runs resolve `kanbanr` to this binary (G-6).
    cfg.path = env.session_path.clone();
    cfg.oauth_token = creds.session_token();
    // The allowed remotes reach the session's hooks (`kanbanr git guard`) through the environment
    // every child of this process inherits, so they are exported before the session starts.
    // SAFETY: this process is single-threaded here, before the session's reader thread exists.
    unsafe {
        std::env::set_var(guard::REMOTES_VAR, guard::remotes_env_value(&allowed));
    }
    let session = Session::start(&cfg)?;
    let today = kanbanr_core::now_rfc3339()
        .chars()
        .take(10)
        .collect::<String>();
    let mut vars = BTreeMap::new();
    vars.insert("board_remote".into(), remotes.board.clone());
    vars.insert("code_remote".into(), remotes.code.clone());
    vars.insert("today".into(), today);
    vars.insert("monitor_url".into(), String::new());
    answer::write_vars(&dirs, &vars)?;
    let mut running = Running {
        max_minutes: args.max_minutes.unwrap_or(runbook.max_minutes),
        runbook,
        dirs: dirs.clone(),
        fence: fence.clone(),
        remotes: remotes.clone(),
        project: None,
        monitor: None,
        browser: None,
        browser_absent: None,
        no_browser: args.no_browser,
        session,
        vars,
        findings: Vec::new(),
        checks: Vec::new(),
        started: Instant::now(),
        last_question: None,
        setup_failed: false,
        names: BTreeMap::new(),
        lines: 0,
        stopped: None,
        pause: args.pause_at_decisions,
        screenshot_cmd: args.screenshot_cmd.clone(),
        ui_dir: args.ui_dir.clone(),
        kanbanr: kanbanr.clone(),
        ci: None,
        gh: gh.clone(),
        asked_log: Vec::new(),
    };
    println!(
        "run {name}: {} steps, model {model}, budget {} USD, fence {}",
        running.runbook.steps.len(),
        cfg.budget_usd,
        fence.display()
    );
    // The steps. An error here still gets the captures below.
    let steps = running.runbook.steps.clone();
    let mut step_error: Option<anyhow::Error> = None;
    for step in &steps {
        match running.step(step) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                step_error = Some(e);
                break;
            }
        }
    }
    // The tail: truth, snapshot, derive, the batch.
    let cost = running.session.cost_usd();
    let turns = running.session.turns();
    let session_id = running.session.session_id().to_string();
    let session_stop = running.session.stopped().map(|s| s.to_string());
    running.session.end();
    let project = running.project_name();
    if prefix.is_some()
        && let Some(gh) = &running.gh
    {
        running.ci = truth::fetch_ci_runs(&dirs, &remotes.code, gh);
    }
    let truth_lines = truth::check(&dirs, &project, &remotes, running.ci.as_ref(), &kanbanr)?;
    for l in truth_lines
        .iter()
        .filter(|l| l.verdict == truth::Verdict::Mismatch)
    {
        running.finding("end", "truth-mismatch", l.board.clone(), l.truth.clone());
    }
    // One result per marker, in checks.tsv as on the target board.
    let merged = merge_checks(&running.checks);
    write_checks(&dirs, &merged)?;
    if !dirs.findings().exists() {
        kanbanr_core::atomic::write(dirs.findings(), "# Findings\n\nNone.\n")?;
    }
    let mut log = SetupLog::read(&dirs).unwrap_or_default();
    log.push("ended", kanbanr_core::now_rfc3339());
    log.push("cost_usd", format!("{cost:.2}"));
    log.push("turns", turns.to_string());
    log.push("findings", running.findings.len().to_string());
    log.push("project", project.clone());
    log.push("session_id", session_id);
    if let Some(s) = session_stop {
        log.push("session_stopped", s);
    }
    if let Some(s) = &running.stopped {
        log.push("stopped", s.clone());
    }
    // `walkthrough prepare` refuses a run whose setup failed (DEF-089).
    if running.setup_failed {
        log.push("setup_failed", "true");
    }
    log.write(&dirs)?;
    if let Some(m) = running.monitor.take() {
        m.stop();
    }
    drop(running.browser.take());
    let manifest = capture::snapshot(&dirs, Some(&remotes))?;
    let derived = derive::tables(&dirs)?;
    // The batch on the target board.
    // A run whose setup never made its board prepares nothing: its checks say nothing about the
    // guide, only that the run could not follow it.
    let batch = if running.setup_failed || args.no_batch {
        Ok(BatchOutcome::default())
    } else {
        prepare_batch(
            cli,
            &running.checks,
            &running.runbook.checks,
            &name,
            &target_board,
            &rev,
        )
    };
    println!();
    println!(
        "run {name} done: {} step(s), {} finding(s), {} check(s) ({} passed), cost {cost:.2} USD, {} capture(s) in the manifest, {} decision row(s)",
        steps.len(),
        running.findings.len(),
        merged.len(),
        merged.iter().filter(|c| c.passed).count(),
        manifest.entries.len(),
        derived.rows
    );
    for f in &running.findings {
        println!("{f}");
    }
    match batch {
        Ok(_) if running.setup_failed => {
            println!(
                "no batch prepared: setup did not complete, so the run says nothing about the guide"
            )
        }
        Ok(_) if args.no_batch => {
            println!(
                "no batch prepared (--no-batch): prepare it with `kanbanr walkthrough prepare`"
            )
        }
        Ok(o) => print_batch(&o, &name, target_project.as_deref()),
        Err(e) => println!("the batch could not be prepared on the target board: {e}"),
    }
    println!("run folder: {}", dirs.root.display());
    if let Some(e) = step_error {
        return Err(e.context("the run stopped on an error; its captures are complete"));
    }
    if let Some(s) = running.stopped {
        bail!("the run stopped early: {s}");
    }
    Ok(())
}

/// What putting a run's checks on the target board did.
#[derive(Debug, Default)]
pub struct BatchOutcome {
    /// Prepared records (manual tests), for the person to confirm as one batch.
    pub prepared: Vec<String>,
    /// Walkthrough-kind tests recorded as the run's own automated evidence.
    pub recorded: Vec<String>,
    /// Checks not put anywhere, and why (not measured, no definition, an error).
    pub skipped: Vec<String>,
}

/// The commit a run's results name (DEF-089): `flag` (`--rev`) when given, as 7 to 40 hex digits,
/// lowercased and never resolved in any repository; else the build's own commit (`build_sha`,
/// the binary's `KANBANR_GIT_SHA`). A dirty or unknown build without `--rev` is refused. The
/// second value says where the revision came from: `--rev` or `the build`.
pub fn evidence_rev(
    build_sha: &str,
    flag: Option<&str>,
) -> std::result::Result<(String, &'static str), String> {
    let is_hex = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
    if let Some(f) = flag {
        let f = f.trim();
        return if (7..=40).contains(&f.len()) && is_hex(f) {
            Ok((f.to_ascii_lowercase(), "--rev"))
        } else {
            Err(format!(
                "--rev '{f}' is not a commit: give 7 to 40 hex digits"
            ))
        };
    }
    if build_sha.len() >= 7 && is_hex(build_sha) {
        return Ok((build_sha.to_string(), "the build"));
    }
    if let Some(h) = build_sha.strip_suffix("-dirty") {
        return Err(format!(
            "the binary under test was built from a modified tree at {h}, so no commit holds the \
             code it would prove: rebuild it from a clean tree, or pass --rev <commit> to name the \
             commit you vouch for"
        ));
    }
    Err(
        "the binary under test names no commit (it was built without git metadata), so its \
         results could not say what they proved: rebuild it from a checkout, or pass --rev \
         <commit>"
            .to_string(),
    )
}

/// Put every measured check of the run on the target board, one per marker. A `walkthrough:`
/// test is the run's own automated evidence: recorded with the tests route as `kanbanr test`
/// records any automated result, at `rev` (the commit of the build under test, DEF-089) — no
/// person's verdict is involved, so none is asked for. A `manual:` test is a person's check:
/// prepared as a `manual-test` record in one batch named for the run, for the person to confirm
/// (ADR-0018), carrying `rev` and its check id. A check whose steps checked nothing is not put
/// anywhere. `defs` are the check definitions: one runbook's, or several runs' merged.
fn prepare_batch(
    cli: &crate::Cli,
    checks: &[CheckResult],
    defs: &BTreeMap<String, runbook::CheckRef>,
    batch: &str,
    target_board: &Path,
    rev: &str,
) -> Result<BatchOutcome> {
    let checks = merge_checks(checks);
    let mut out = BatchOutcome::default();
    if checks.is_empty() {
        return Ok(out);
    }
    let project = kanbanr_core::project::resolve_project(cli.project.as_deref())
        .ok_or_else(|| anyhow!("no project here to prepare the checks on; pass --project"))?;
    let backend = crate::Backend::new(target_board.to_path_buf());
    let board = kanbanr_core::Store::new(target_board).load(&project).ok();
    let test_of = |item: &str, req: &str, test: &str| -> Option<kanbanr_core::models::TestRef> {
        let f = board.as_ref()?.feature(item).ok()?;
        f.definition
            .as_ref()?
            .requirements
            .iter()
            .find(|r| r.id == req)?
            .tests
            .iter()
            .find(|t| t.name == test)
            .cloned()
    };
    let repo = crate::scm::repo_root();
    // What this batch already holds: a record open for the same subject is not prepared twice.
    let open = kanbanr_core::prepared::open_in_batch(
        &kanbanr_core::Store::new(target_board),
        &project,
        batch,
        repo.as_deref(),
    )
    .unwrap_or_default();
    for c in &checks {
        let Some(def) = defs.get(&c.marker) else {
            out.skipped
                .push(format!("{}: not defined under checks", c.marker));
            continue;
        };
        if !c.measured {
            out.skipped
                .push(format!("{} ({}): {NOT_MEASURED}", c.marker, c.step));
            continue;
        }
        let note = format!(
            "walkthrough run {batch} step {} ({}): {}",
            c.step, c.marker, c.why
        );
        let stored = test_of(&def.item, &def.requirement, &def.test);
        if stored
            .as_ref()
            .is_some_and(|t| t.kind.trim().eq_ignore_ascii_case("walkthrough"))
        {
            let state = if c.passed {
                kanbanr_core::models::TestState::Green
            } else {
                kanbanr_core::models::TestState::Red
            };
            // A rerun leaves a result that already says this, at this revision, as it is.
            if stored
                .as_ref()
                .is_some_and(|t| t.state == state && t.checked_rev == rev)
            {
                out.skipped.push(format!(
                    "{} {}: already recorded at {rev}",
                    def.requirement, def.test
                ));
                continue;
            }
            let body = serde_json::json!({
                "state": if c.passed { "green" } else { "red" },
                "note": note,
                "checked_rev": rev,
            });
            match backend.write_in(
                crate::Method::Put,
                &format!(
                    "/projects/{project}/features/{}/tests/{}/{}",
                    person::urlencode(&def.item),
                    person::urlencode(&def.requirement),
                    person::urlencode(&def.test)
                ),
                Some(body),
                repo.as_deref(),
            ) {
                Ok(_) => out.recorded.push(format!(
                    "{} {} {} {}",
                    c.marker,
                    def.item,
                    def.requirement,
                    if c.passed { "green" } else { "red" }
                )),
                Err(e) => out
                    .skipped
                    .push(format!("{}: could not record: {e}", c.marker)),
            }
            continue;
        }
        if let Some(v) = open.iter().find(|v| {
            v.record.kind == "manual-test"
                && v.record.subject.item == def.item
                && v.record.subject.requirement == def.requirement
                && v.record.subject.test == def.test
        }) {
            out.skipped.push(format!(
                "{}: already prepared in batch {batch} ({})",
                c.marker, v.record.id
            ));
            continue;
        }
        let body = serde_json::json!({
            "kind": "manual-test",
            "subject": {"item": def.item, "requirement": def.requirement, "test": def.test},
            "fields": {
                "state": if c.passed { "green" } else { "red" },
                "note": c.why,
                "checked_rev": rev,
                "check": c.marker,
            },
            "sources": [{"label": format!(
                "walkthrough run {batch} step {}, check {}",
                c.step, c.marker
            )}],
            "by": "Claude",
            "batch": batch,
        });
        match backend.write_in(
            crate::Method::Post,
            &format!("/projects/{project}/prepared"),
            Some(body),
            repo.as_deref(),
        ) {
            Ok(resp) => {
                let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                out.prepared
                    .push(v["id"].as_str().unwrap_or("").to_string());
            }
            Err(e) => out
                .skipped
                .push(format!("{}: could not prepare: {e}", c.marker)),
        }
    }
    Ok(out)
}

fn which(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(marker: &str, step: &str, passed: bool, why: &str) -> CheckResult {
        CheckResult {
            marker: marker.into(),
            step: step.into(),
            passed,
            why: why.into(),
            measured: true,
        }
    }

    /// DEF-089 R-1, R-2, R-6: a clean build names itself; a dirty or unknown one is refused
    /// unless `--rev` names a commit, which is taken as given (lowercased, never looked up).
    #[test]
    fn the_evidence_rev_is_the_builds_commit_or_the_named_one() {
        let full = "6a973c690bb2a0909aae0b8e1f5a75e867cec7f1";
        assert_eq!(
            evidence_rev(full, None),
            Ok((full.to_string(), "the build"))
        );
        assert_eq!(
            evidence_rev("6a973c690bb2", None),
            Ok(("6a973c690bb2".to_string(), "the build"))
        );
        let dirty = evidence_rev("abc1234-dirty", None).unwrap_err();
        assert!(
            dirty.contains("--rev") && dirty.contains("abc1234"),
            "{dirty}"
        );
        assert!(dirty.contains("modified tree"), "{dirty}");
        let unknown = evidence_rev("unknown", None).unwrap_err();
        assert!(unknown.contains("--rev"), "{unknown}");
        assert!(unknown.contains("names no commit"), "{unknown}");
        assert_eq!(
            evidence_rev(
                "abc1234-dirty",
                Some("0123456789abcdef0123456789abcdef01234567")
            ),
            Ok((
                "0123456789abcdef0123456789abcdef01234567".to_string(),
                "--rev"
            ))
        );
        assert_eq!(
            evidence_rev("unknown", Some(" ABCDEF1 ")),
            Ok(("abcdef1".to_string(), "--rev"))
        );
        // A well-formed --rev wins over a clean build too: it is the person's statement.
        assert_eq!(
            evidence_rev(full, Some("abcdef1")),
            Ok(("abcdef1".to_string(), "--rev"))
        );
        for bad in ["xyz", "abc12", "g123456", &"a".repeat(41)] {
            let e = evidence_rev(full, Some(bad)).unwrap_err();
            assert!(e.contains("is not a commit"), "{bad}: {e}");
        }
    }

    /// A check on a step that checked nothing is "not measured", never passed: alone it stays so,
    /// and beside a measured step it is ignored (the vacuous MT-5 pass of the second real run).
    #[test]
    fn a_check_on_a_step_that_checked_nothing_is_not_measured() {
        let unmeasured = CheckResult {
            measured: false,
            passed: false,
            why: NOT_MEASURED.into(),
            ..check("MT-5", "6.7", false, "")
        };
        let alone = merge_checks(std::slice::from_ref(&unmeasured));
        assert!(!alone[0].measured && !alone[0].passed);
        assert_eq!(alone[0].why, NOT_MEASURED);
        let beside = merge_checks(&[unmeasured, check("MT-5", "6.8", true, "ok")]);
        assert!(beside[0].measured && beside[0].passed, "{beside:?}");
        assert_eq!(beside[0].step, "6.7+6.8");
    }

    /// T16: a marker named on two steps is one result — passed only if both passed, naming both
    /// steps, with the failed step's reason prefixed by its id — and markers keep their order.
    #[test]
    fn a_check_named_on_two_steps_is_one_result() {
        let merged = merge_checks(&[
            check(
                "MT-7",
                "2.2b",
                true,
                "the step went through as the guide says",
            ),
            check(
                "MT-1",
                "3.1",
                true,
                "the step went through as the guide says",
            ),
            check("MT-7", "2.2c", false, "the offer was missing"),
            check("MT-7", "2.2d", false, "a board claim failed"),
        ]);
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(merged[0].marker, "MT-7");
        assert_eq!(merged[0].step, "2.2b+2.2c+2.2d");
        assert!(!merged[0].passed);
        assert_eq!(
            merged[0].why,
            "2.2c: the offer was missing; 2.2d: a board claim failed"
        );
        assert_eq!(
            merged[1],
            check(
                "MT-1",
                "3.1",
                true,
                "the step went through as the guide says"
            )
        );
        let both = merge_checks(&[
            check("MT-2", "1.3", true, "ok"),
            check("MT-2", "4.1", true, "ok"),
        ]);
        assert_eq!(both.len(), 1);
        assert!(both[0].passed);
        assert_eq!(both[0].step, "1.3+4.1");
    }

    /// DEF-089 R-4 (FTR-054): the checks of two runs are one result per marker. A marker failed in
    /// one run fails, naming that run's step; passed in both passes; measured in one run only (or
    /// unmeasured in the other) takes the measured verdict.
    #[test]
    fn checks_from_two_runs_merge_into_one_result_per_marker() {
        let unmeasured = |marker: &str, step: &str| CheckResult {
            measured: false,
            passed: false,
            why: NOT_MEASURED.into(),
            ..check(marker, step, false, "")
        };
        let togaf = vec![
            check("MT-1", "2.1", true, "ok"),
            check("MT-10", "6.20a", true, "ok"),
            unmeasured("MT-3", "3.1"),
        ];
        let agile = vec![
            check("MT-1", "2.4", false, "the offer was missing"),
            check("MT-10", "5.2", true, "ok"),
            check("MT-6", "4.1", false, "a board claim failed"),
            check("MT-3", "3.2", true, "ok"),
        ];
        let merged = checks_across(&[
            ("togaf-v080-1".to_string(), togaf),
            ("agile-v080-1".to_string(), agile),
        ]);
        let by = |m: &str| {
            merged
                .iter()
                .find(|c| c.marker == m)
                .unwrap_or_else(|| panic!("no {m} in {merged:?}"))
        };
        assert_eq!(merged.len(), 4, "one result per marker: {merged:?}");
        let mt1 = by("MT-1");
        assert!(mt1.measured && !mt1.passed, "{mt1:?}");
        assert_eq!(mt1.step, "togaf-v080-1:2.1+agile-v080-1:2.4");
        assert_eq!(mt1.why, "agile-v080-1:2.4: the offer was missing");
        let mt10 = by("MT-10");
        assert!(mt10.measured && mt10.passed, "{mt10:?}");
        assert_eq!(mt10.step, "togaf-v080-1:6.20a+agile-v080-1:5.2");
        let mt6 = by("MT-6");
        assert!(mt6.measured && !mt6.passed, "{mt6:?}");
        assert_eq!(mt6.step, "agile-v080-1:4.1");
        assert_eq!(mt6.why, "agile-v080-1:4.1: a board claim failed");
        let mt3 = by("MT-3");
        assert!(mt3.measured && mt3.passed, "{mt3:?}");
        assert_eq!(mt3.step, "togaf-v080-1:3.1+agile-v080-1:3.2");
        // Markers keep the order they were first named in, across the runs.
        let order: Vec<&str> = merged.iter().map(|c| c.marker.as_str()).collect();
        assert_eq!(order, ["MT-1", "MT-10", "MT-3", "MT-6"]);
    }

    /// DEF-087 R-1: a talk claim with `since` reads the questions of every step from that one to
    /// its own, each with its step; without `since`, or with one the log does not hold, its own
    /// step alone.
    #[test]
    fn a_talk_expectation_reads_every_step_of_its_span() {
        let log: Vec<(String, Vec<String>)> = vec![
            ("2.1".into(), vec!["Which process?".into()]),
            ("2.2".into(), vec!["How should the page look?".into()]),
            ("3.1".into(), vec![]),
            ("3.2".into(), vec![]),
        ];
        let here = vec!["Ready?".to_string()];
        assert_eq!(
            asked_in_span(&log, Some("2.1"), &here),
            [
                ("2.1", "Which process?"),
                ("2.2", "How should the page look?"),
                ("", "Ready?")
            ]
        );
        assert_eq!(asked_in_span(&log, Some("3.1"), &here), [("", "Ready?")]);
        assert_eq!(asked_in_span(&log, None, &here), [("", "Ready?")]);
        assert_eq!(asked_in_span(&log, Some("9.9"), &here), [("", "Ready?")]);
    }

    /// DEF-087 R-5: three doc comments in the walkthrough say what the code does: a title
    /// selector's several fits are ambiguous, a pick's policy can be `runbook-context`, and a text
    /// question without a question mark can be a request sentence.
    #[test]
    fn the_walkthrough_doc_comments_match_the_code() {
        let runbook = include_str!("runbook.rs");
        let answer = include_str!("answer.rs");
        let driver = include_str!("driver.rs");
        let gone = ["the newest when", "several do."].concat();
        assert!(
            !runbook.contains(&gone),
            "runbook.rs still says the newest wins"
        );
        assert_eq!(
            runbook
                .matches("several equal fits are\n    /// ambiguous (`ambiguous-selector`), never a guess.")
                .count(),
            2
        );
        let old_policy = ["`runbook`, `recommended`, ", "`first` or `free-text`."].concat();
        assert!(
            !answer.contains(&old_policy),
            "answer.rs leaves out runbook-context"
        );
        assert!(
            answer.contains("`runbook`, `recommended`, `runbook-context`, `first` or `free-text`.")
        );
        assert!(driver.contains(
            "With no question mark, a request\n/// sentence (in bold, or the message's last sentence) is the question."
        ));
    }
}
