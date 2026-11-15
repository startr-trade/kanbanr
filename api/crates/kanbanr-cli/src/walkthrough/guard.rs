//! Before a run starts (FTR-037 W3, R-1, R-10): the run acts only inside the fence, only on its own
//! remotes, and never on the target board; and `setup.log` names what was under test — the
//! installed kanbanr, the skill's tree checksum, the Claude Code version, the model and the
//! checksums of the person's rules — so a run's findings can be read against what produced them.
//!
//! The push guard is here too: `allowed_remotes` is exported to the session as
//! `KANBANR_WALKTHROUGH_REMOTES_ALLOWED`, and `kanbanr git guard` (`push_refusal`) refuses a
//! `git push` to anything else.

use anyhow::{Context, Result, anyhow};
use kanbanr_core::project;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The environment variable the session and its hooks read the allowed remotes from, `;`-separated.
pub const REMOTES_VAR: &str = "KANBANR_WALKTHROUGH_REMOTES_ALLOWED";

/// The folders of one run: `<runs>/<name>/{project, board, claude-config, remotes.env, raw, results}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDirs {
    pub root: PathBuf,
}

impl RunDirs {
    pub fn new(root: &Path) -> RunDirs {
        RunDirs {
            root: root.to_path_buf(),
        }
    }

    /// The run's name: its folder's name.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "run".into())
    }

    pub fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    pub fn board(&self) -> PathBuf {
        self.root.join("board")
    }

    pub fn claude_config(&self) -> PathBuf {
        self.root.join("claude-config")
    }

    pub fn remotes_env(&self) -> PathBuf {
        self.root.join("remotes.env")
    }

    /// Where the bare repositories go when no `--remotes` prefix is given.
    pub fn local_remotes(&self) -> PathBuf {
        self.root.join("remotes")
    }

    pub fn raw(&self) -> PathBuf {
        self.root.join("raw")
    }

    pub fn results(&self) -> PathBuf {
        self.root.join("results")
    }

    pub fn setup_log(&self) -> PathBuf {
        self.raw().join("setup.log")
    }

    pub fn findings(&self) -> PathBuf {
        self.raw().join("findings.md")
    }

    /// The lock that keeps a run folder to one run at a time.
    pub fn lock_file(&self) -> PathBuf {
        self.root.join(".run.lock")
    }

    /// Make the folders a run writes into.
    pub fn make(&self) -> Result<()> {
        for d in [
            self.project(),
            self.claude_config(),
            self.raw(),
            self.results(),
        ] {
            std::fs::create_dir_all(&d)
                .with_context(|| format!("could not make {}", d.display()))?;
        }
        Ok(())
    }
}

/// The run's own remotes, from `remotes.env` (`CODE_REMOTE=…`, `BOARD_REMOTE=…`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemotesEnv {
    pub code: String,
    pub board: String,
}

impl RemotesEnv {
    /// Parse `KEY=VALUE` lines; `#` lines and blanks are skipped; quotes around a value are dropped.
    pub fn parse(text: &str) -> Result<RemotesEnv> {
        let mut out = RemotesEnv::default();
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let l = l.strip_prefix("export ").unwrap_or(l);
            let Some((k, v)) = l.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "CODE_REMOTE" => out.code = v,
                "BOARD_REMOTE" => out.board = v,
                _ => {}
            }
        }
        if out.code.is_empty() || out.board.is_empty() {
            return Err(anyhow!(
                "remotes.env names CODE_REMOTE and BOARD_REMOTE; one is missing"
            ));
        }
        Ok(out)
    }

    pub fn read(run: &RunDirs) -> Result<RemotesEnv> {
        let path = run.remotes_env();
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("no {}: make the run's remotes first", path.display()))?;
        RemotesEnv::parse(&text)
    }

    pub fn to_text(&self) -> String {
        format!(
            "# The run's own remotes: the only places a run ever pushes to.\nCODE_REMOTE={}\nBOARD_REMOTE={}\n",
            self.code, self.board
        )
    }

    pub fn write(&self, run: &RunDirs) -> Result<()> {
        kanbanr_core::atomic::write(run.remotes_env(), self.to_text())?;
        Ok(())
    }
}

/// What preflight reads from outside the run folder.
#[derive(Debug, Clone)]
pub struct Env {
    /// `KANBANR_BOARD_FENCE`, resolved; `None` when unset.
    pub fence: Option<PathBuf>,
    /// The target board: where the run's checks are prepared; never inside the run.
    pub target_board: Option<PathBuf>,
    /// The `claude` program, for its version line.
    pub claude: PathBuf,
    pub model: String,
    /// The `kanbanr` under test, for its version line: this very program in a run.
    pub kanbanr: PathBuf,
    /// The person's installed kanbanr skill, compared with the one the run uses (a note only).
    pub person_skill: Option<PathBuf>,
}

/// `git push <remote> <refspec>` in `repo`, run directly: core's repository wrapper refuses every
/// push by design (FEAT-189, kanbanr never pushes), and a walkthrough's pushes are the one exception
/// the Design makes — to the run's own remotes only, which `push_refusal` holds the session to and
/// `preflight` checks. Returns git's output, or its reason.
pub fn git_push(repo: &Path, remote: &str, refspec: &str) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["push", remote, refspec])
        .current_dir(repo)
        .output()
        .with_context(|| format!("could not run git push in {}", repo.display()))?;
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(said)
    } else {
        Err(anyhow!("git push {remote} {refspec}: {said}"))
    }
}

/// `key value` lines, as SPK-002's `setup.log`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupLog {
    pub entries: Vec<(String, String)>,
}

impl SetupLog {
    pub fn push(&mut self, key: &str, value: impl Into<String>) {
        self.entries.push((key.to_string(), value.into()));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.entries {
            out.push_str(k);
            out.push(' ');
            out.push_str(v);
            out.push('\n');
        }
        out
    }

    /// Write (or rewrite) `raw/setup.log`.
    pub fn write(&self, run: &RunDirs) -> Result<PathBuf> {
        std::fs::create_dir_all(run.raw())?;
        let path = run.setup_log();
        kanbanr_core::atomic::write(&path, self.to_text())?;
        Ok(path)
    }

    /// Read `raw/setup.log` back.
    pub fn read(run: &RunDirs) -> Result<SetupLog> {
        let text = std::fs::read_to_string(run.setup_log())?;
        let mut log = SetupLog::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(' ') {
                log.push(k, v);
            } else if !line.trim().is_empty() {
                log.push(line, "");
            }
        }
        Ok(log)
    }
}

/// sha256 of a file's bytes, as hex; "missing" when it is not there.
pub fn sha256_file(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => format!("{:x}", Sha256::digest(&bytes)),
        Err(_) => "missing".into(),
    }
}

/// sha256 over a folder: every regular file under it in sorted relative-path order, each as its
/// path, a zero byte and its bytes (the shape `skill.rs` hashes the embedded skill in). Symlinked
/// folders are followed, so a skill linked into the config folder hashes as its target.
pub fn sha256_tree(dir: &Path) -> String {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, path));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    let mut h = Sha256::new();
    for (rel, path) in files {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(std::fs::read(&path).unwrap_or_default());
    }
    format!("{:x}", h.finalize())
}

/// The `@name` imports a rules file names, each on its own line or at a line's start.
pub fn rule_imports(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let t = l.trim();
            let name = t.strip_prefix('@')?;
            let name = name.split_whitespace().next()?;
            (!name.is_empty() && !name.contains("://")).then(|| name.to_string())
        })
        .collect()
}

/// The first line a program prints for `--version`, or why it could not be run.
fn version_line(program: &Path, args: &[&str]) -> String {
    match std::process::Command::new(program).args(args).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
        Ok(out) => format!(
            "exit {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("not found ({e})"),
    }
}

/// The remotes a run may push to: the two from `remotes.env`, as written.
pub fn allowed_remotes(run: &RunDirs) -> Vec<String> {
    match RemotesEnv::read(run) {
        Ok(r) => vec![r.code, r.board],
        Err(_) => Vec::new(),
    }
}

/// The value of `KANBANR_WALKTHROUGH_REMOTES_ALLOWED` for `allowed`.
pub fn remotes_env_value(allowed: &[String]) -> String {
    allowed.join(";")
}

/// A remote as compared: a local path canonicalised, a URL trimmed of a trailing `/` or `.git`.
fn remote_key(remote: &str, cwd: &Path) -> String {
    let r = remote.trim();
    if r.contains("://") || (r.contains('@') && r.contains(':')) {
        return r
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase();
    }
    let p = if Path::new(r).is_absolute() {
        PathBuf::from(r)
    } else {
        cwd.join(r)
    };
    project::normalize(&p).to_string_lossy().to_string()
}

/// Why a `git push` in `command` (run in `cwd`) must not happen: its remote is not one of
/// `allowed`. A remote named rather than given as a URL is resolved with `git remote get-url`;
/// a push with no remote means `origin`. None for any command that is not a push, and for a push
/// to an allowed remote.
pub fn push_refusal(command: &str, cwd: &Path, allowed: &[String]) -> Option<String> {
    let allowed_keys: Vec<String> = allowed.iter().map(|a| remote_key(a, cwd)).collect();
    for (_, words) in crate::simple_commands(command) {
        let Some(call) = crate::git_call(&words) else {
            continue;
        };
        if call.sub != Some("push") {
            continue;
        }
        let dir = call
            .dirs
            .first()
            .map(|d| cwd.join(d))
            .unwrap_or_else(|| cwd.to_path_buf());
        // The remote is the first argument that is not an option; `--repo=<url>` names one too.
        let mut remote: Option<String> = None;
        for a in &call.args {
            if let Some(r) = a.strip_prefix("--repo=") {
                remote = Some(r.to_string());
                break;
            }
            if a.starts_with('-') {
                continue;
            }
            remote = Some(a.to_string());
            break;
        }
        let remote = remote.unwrap_or_else(|| "origin".to_string());
        let url = if remote.contains("://") || remote.contains('/') || remote.contains(':') {
            remote.clone()
        } else {
            kanbanr_core::coderepo::git(&dir, &["remote", "get-url", &remote])
                .unwrap_or_else(|_| remote.clone())
        };
        if !allowed_keys.contains(&remote_key(&url, &dir)) {
            return Some(format!(
                "refused: this walkthrough pushes only to its own remotes, never to {remote}. A \
                 run writes nothing outside its folder, its scratch board and its own remotes \
                 (FTR-037)."
            ));
        }
    }
    None
}

/// Everything that must hold before a run starts, and the `setup.log` of what is under test.
/// Refuses (an error, nothing written) unless the fence is set and the run's project and board
/// resolve inside it, unless `remotes.env` names the run's own remotes and the run's repositories
/// point only at them, and unless the target board is not inside the run folder.
pub fn preflight(run: &RunDirs, env: &Env) -> Result<SetupLog> {
    let fence = env.fence.clone().ok_or_else(|| {
        anyhow!(
            "{} is not set: a walkthrough acts only on boards inside a fence",
            project::FENCE_VAR
        )
    })?;
    for (what, dir) in [("project", run.project()), ("board", run.board())] {
        project::check_fence(&dir, &fence)
            .map_err(|why| anyhow!("the run's {what} is outside the fence: {why}"))?;
    }
    let remotes = RemotesEnv::read(run)?;
    let allowed = [remotes.code.clone(), remotes.board.clone()];
    for repo in [run.project(), run.board()] {
        if !repo.join(".git").exists() {
            continue;
        }
        for (name, url) in kanbanr_core::git::list_remotes(&repo) {
            let known = allowed
                .iter()
                .any(|a| remote_key(a, &repo) == remote_key(&url, &repo));
            if !known {
                return Err(anyhow!(
                    "{} has a remote `{name}` that is not the run's own ({url}): a run pushes only \
                     to the remotes in remotes.env",
                    repo.display()
                ));
            }
        }
    }
    if let Some(target) = &env.target_board {
        let target = project::normalize(target);
        let root = project::normalize(&run.root);
        if target.starts_with(&root) {
            return Err(anyhow!(
                "the target board {} is inside the run folder: a run prepares its checks on the \
                 board it was started from, never on its own scratch board",
                target.display()
            ));
        }
    }
    // What is under test.
    let mut log = SetupLog::default();
    log.push("started", kanbanr_core::now_rfc3339());
    log.push("run", run.root.display().to_string());
    log.push("fence", fence.display().to_string());
    log.push("kanbanr", version_line(&env.kanbanr, &["--version"]));
    log.push("kanbanr_path", env.kanbanr.display().to_string());
    let skill = run.claude_config().join("skills").join("kanbanr");
    let skill_sum = if skill.exists() {
        sha256_tree(&skill)
    } else {
        "missing".to_string()
    };
    log.push("skill_sha256", skill_sum.clone());
    // The run's skill is the one this binary carries (T16): its stamp says so.
    let stamped = std::fs::read_to_string(skill.join(".kanbanr-skill")).is_ok();
    log.push(
        "skill_source",
        if stamped {
            format!("the skill this binary carries ({})", env.kanbanr.display())
        } else {
            "not the binary's: no kanbanr stamp in the run's skill folder".to_string()
        },
    );
    if let Some(theirs) = &env.person_skill
        && theirs.exists()
    {
        let their_sum = sha256_tree(theirs);
        if their_sum != skill_sum {
            log.push(
                "skill_note",
                format!(
                    "the installed skill at {} differs from this binary's (sha256 {their_sum})",
                    theirs.display()
                ),
            );
        }
    }
    log.push("claude", version_line(&env.claude, &["--version"]));
    log.push("model", env.model.clone());
    let rules = run.claude_config().join("CLAUDE.md");
    log.push("rules_sha256(CLAUDE.md)", sha256_file(&rules));
    if let Ok(text) = std::fs::read_to_string(&rules) {
        for import in rule_imports(&text) {
            log.push(
                &format!("rules_sha256({import})"),
                sha256_file(&run.claude_config().join(&import)),
            );
        }
    }
    log.push("code_remote", remotes.code.clone());
    log.push("board_remote", remotes.board.clone());
    if let Some(target) = &env.target_board {
        let head = kanbanr_core::coderepo::git(target, &["rev-parse", "HEAD"])
            .unwrap_or_else(|_| "unknown".into());
        log.push("target_board_head", head);
    }
    Ok(log)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A run folder under the temporary directory with a config copy (rules, one import, a skill
    /// folder) and `remotes.env` naming two bare repositories beside it.
    pub(crate) fn scratch_run(name: &str) -> (RunDirs, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "kanbanr-wt-guard-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let run = RunDirs::new(&base.join("runs").join(name));
        run.make().unwrap();
        let cfg = run.claude_config();
        std::fs::write(cfg.join("CLAUDE.md"), "@RTK.md\n\n## Rules\n- be brief\n").unwrap();
        std::fs::write(cfg.join("RTK.md"), "# RTK\n").unwrap();
        std::fs::create_dir_all(cfg.join("skills").join("kanbanr").join("hooks")).unwrap();
        std::fs::write(cfg.join("skills/kanbanr/SKILL.md"), "# skill\n").unwrap();
        std::fs::write(cfg.join("skills/kanbanr/hooks/a.sh"), "#!/bin/sh\n").unwrap();
        let remotes = run.local_remotes();
        for r in ["code.git", "board.git"] {
            let p = remotes.join(r);
            std::fs::create_dir_all(&p).unwrap();
            kanbanr_core::coderepo::git(&p, &["init", "--bare", "-q"]).unwrap();
        }
        RemotesEnv {
            code: remotes.join("code.git").display().to_string(),
            board: remotes.join("board.git").display().to_string(),
        }
        .write(&run)
        .unwrap();
        (run, base)
    }

    /// A fake program that answers `--version` with `line`.
    #[cfg(unix)]
    fn fake_version(dir: &Path, name: &str, line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// The unit tests of the binary have no built `kanbanr` to ask: a fake stands in for it, as
    /// for `claude`.
    fn env(fence: Option<PathBuf>, target: Option<PathBuf>, claude: PathBuf) -> Env {
        let kanbanr = claude
            .parent()
            .map(|d| d.join("kanbanr"))
            .unwrap_or_else(|| PathBuf::from("kanbanr-that-is-not-run"));
        Env {
            fence,
            target_board: target,
            claude,
            model: "sonnet".into(),
            kanbanr,
            person_skill: None,
        }
    }

    /// T16 (G-6): the run's config gets the skill this binary carries — a stamped copy, not a link
    /// to the person's installed one — `setup.log` names its checksum as the binary's, and a person's
    /// installed copy that differs is noted, never refused.
    #[cfg(unix)]
    #[test]
    fn the_run_uses_the_skill_this_binary_carries() {
        let (run, base) = scratch_run("own-skill");
        let skill = run.claude_config().join("skills").join("kanbanr");
        // An earlier run's link to someone's copy is replaced.
        std::fs::remove_dir_all(&skill).unwrap();
        let theirs = base.join("their-skill");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("SKILL.md"), "# an older skill\n").unwrap();
        std::os::unix::fs::symlink(&theirs, &skill).unwrap();
        let said = crate::walkthrough::install_run_skill(&run).unwrap();
        assert!(said.starts_with("installed the kanbanr skill"), "{said}");
        let meta = std::fs::symlink_metadata(&skill).unwrap();
        assert!(!meta.file_type().is_symlink() && meta.is_dir());
        assert!(skill.join(".kanbanr-skill").exists());
        assert!(skill.join("SKILL.md").exists());
        assert_eq!(
            std::fs::read_to_string(theirs.join("SKILL.md")).unwrap(),
            "# an older skill\n",
            "the person's copy is untouched"
        );
        let claude = fake_version(&base, "claude", "9.9.9 (Claude Code, fake)");
        fake_version(
            &base,
            "kanbanr",
            "kanbanr 0.7.0 (0000000, built 2026-11-15)",
        );
        let mut e = env(Some(project::normalize(&base)), None, claude);
        e.person_skill = Some(theirs.clone());
        let log = preflight(&run, &e).unwrap();
        assert_eq!(log.get("skill_sha256"), Some(sha256_tree(&skill).as_str()));
        assert!(
            log.get("skill_source")
                .unwrap()
                .starts_with("the skill this binary carries")
        );
        let note = log.get("skill_note").unwrap();
        assert!(note.contains("differs from this binary's"), "{note}");
        // The same copy as the binary's: no note.
        e.person_skill = Some(skill.clone());
        assert_eq!(preflight(&run, &e).unwrap().get("skill_note"), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-1: no fence, or a run whose board or project resolves outside it, or a target
    /// board inside the run, and nothing starts; a repository pointing at a remote that is not
    /// the run's own is refused too.
    #[test]
    fn a_board_outside_the_fence_refuses_to_start() {
        let (run, base) = scratch_run("fence");
        let claude = PathBuf::from("claude-that-is-not-run");
        let why = preflight(&run, &env(None, None, claude.clone()))
            .unwrap_err()
            .to_string();
        assert!(why.contains("KANBANR_BOARD_FENCE is not set"), "{why}");
        // A fence that does not hold the run.
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let why = preflight(
            &run,
            &env(Some(project::normalize(&elsewhere)), None, claude.clone()),
        )
        .unwrap_err()
        .to_string();
        assert!(
            why.contains("outside the fence") && why.contains("outside KANBANR_BOARD_FENCE"),
            "{why}"
        );
        // The target board inside the run folder.
        let fence = project::normalize(&base);
        let why = preflight(
            &run,
            &env(Some(fence.clone()), Some(run.board()), claude.clone()),
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("inside the run folder"), "{why}");
        // A foreign remote on the run's project.
        let project_dir = run.project();
        kanbanr_core::coderepo::git(&project_dir, &["init", "-q", "-b", "main"]).unwrap();
        kanbanr_core::coderepo::git(
            &project_dir,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/someone/shop.git",
            ],
        )
        .unwrap();
        let why = preflight(&run, &env(Some(fence.clone()), None, claude.clone()))
            .unwrap_err()
            .to_string();
        assert!(why.contains("not the run's own"), "{why}");
        // Nothing was written.
        assert!(!run.setup_log().exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-1: `setup.log` names the installed kanbanr (its version line, with commit and
    /// date), the skill's tree checksum, the Claude Code version, the model, the checksum of the
    /// rules file and of each `@` import, the run's remotes and the target board's HEAD.
    #[cfg(unix)]
    #[test]
    fn setup_log_names_binary_skill_model_and_rules() {
        let (run, base) = scratch_run("setup");
        let claude = fake_version(&base, "claude", "9.9.9 (Claude Code, fake)");
        fake_version(
            &base,
            "kanbanr",
            "kanbanr 0.7.0 (0000000000000000000000000000000000000000, built 2026-11-15)",
        );
        let target = base.join("target-board");
        std::fs::create_dir_all(&target).unwrap();
        kanbanr_core::git::ensure_repo_as(&target, Some(("Ada", "ada@example.com")));
        let fence = project::normalize(&base);
        let log = preflight(&run, &env(Some(fence), Some(target.clone()), claude)).unwrap();
        let kanbanr = log.get("kanbanr").unwrap();
        assert!(kanbanr.starts_with("kanbanr "), "{kanbanr}");
        assert!(
            kanbanr.contains("built"),
            "name, commit and date: {kanbanr}"
        );
        assert_eq!(log.get("claude"), Some("9.9.9 (Claude Code, fake)"));
        assert_eq!(log.get("model"), Some("sonnet"));
        let skill = log.get("skill_sha256").unwrap();
        assert_eq!(skill.len(), 64);
        assert_eq!(
            skill,
            &sha256_tree(&run.claude_config().join("skills/kanbanr"))
        );
        assert_eq!(
            log.get("rules_sha256(CLAUDE.md)"),
            Some(sha256_file(&run.claude_config().join("CLAUDE.md")).as_str())
        );
        assert_eq!(
            log.get("rules_sha256(RTK.md)"),
            Some(sha256_file(&run.claude_config().join("RTK.md")).as_str())
        );
        let head = kanbanr_core::coderepo::git(&target, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(log.get("target_board_head"), Some(head.as_str()));
        assert!(log.get("code_remote").unwrap().ends_with("code.git"));
        // Written as `key value` lines and read back the same.
        let path = log.write(&run).unwrap();
        assert_eq!(path, run.setup_log());
        assert_eq!(SetupLog::read(&run).unwrap(), log);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.lines().any(|l| l.starts_with("model sonnet")),
            "{text}"
        );
        // The skill checksum changes with the skill.
        std::fs::write(
            run.claude_config().join("skills/kanbanr/SKILL.md"),
            "# changed\n",
        )
        .unwrap();
        assert_ne!(
            sha256_tree(&run.claude_config().join("skills/kanbanr")),
            *skill
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-10: a `git push` to a remote that is not one of the run's own is refused, whether
    /// the remote is named or given as a URL, with or without `-C`; a push to the run's remote and
    /// any other git command pass.
    #[test]
    fn a_push_to_a_remote_that_is_not_the_runs_is_refused() {
        let (run, base) = scratch_run("push");
        let allowed = allowed_remotes(&run);
        assert_eq!(allowed.len(), 2);
        assert_eq!(remotes_env_value(&allowed), allowed.join(";"));
        let project_dir = run.project();
        kanbanr_core::coderepo::git(&project_dir, &["init", "-q", "-b", "main"]).unwrap();
        kanbanr_core::coderepo::git(&project_dir, &["remote", "add", "origin", &allowed[0]])
            .unwrap();
        kanbanr_core::coderepo::git(
            &project_dir,
            &[
                "remote",
                "add",
                "public",
                "https://example.invalid/someone/shop.git",
            ],
        )
        .unwrap();
        assert_eq!(
            push_refusal("git push origin main", &project_dir, &allowed),
            None
        );
        assert_eq!(push_refusal("git push", &project_dir, &allowed), None);
        assert_eq!(
            push_refusal(
                "git status && git push -u origin main",
                &project_dir,
                &allowed
            ),
            None
        );
        let direct = format!("git push {} v0.1.0", allowed[0]);
        assert_eq!(push_refusal(&direct, &project_dir, &allowed), None);
        let why = push_refusal("git push public main", &project_dir, &allowed).unwrap();
        assert!(why.contains("never to public"), "{why}");
        let why = push_refusal(
            "git push https://example.invalid/someone/shop.git main",
            &project_dir,
            &allowed,
        )
        .unwrap();
        assert!(why.contains("refused"), "{why}");
        assert!(
            push_refusal("git push unknown-remote main", &project_dir, &allowed).is_some(),
            "a remote git cannot resolve is not one of the run's"
        );
        // From another folder, `-C` names where the push runs.
        let cmd = format!("git -C {} push public main", project_dir.display());
        assert!(push_refusal(&cmd, &base, &allowed).is_some());
        assert_eq!(
            push_refusal("git log --oneline", &project_dir, &allowed),
            None
        );
        assert_eq!(push_refusal("kanbanr board", &project_dir, &allowed), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn remotes_env_reads_the_two_urls_and_imports_are_read_from_the_rules() {
        let r = RemotesEnv::parse(
            "# comment\nexport CODE_REMOTE=\"/tmp/x/code.git\"\nBOARD_REMOTE=/tmp/x/board.git\nCI_LIST=none\n",
        )
        .unwrap();
        assert_eq!(r.code, "/tmp/x/code.git");
        assert_eq!(r.board, "/tmp/x/board.git");
        assert!(RemotesEnv::parse("CODE_REMOTE=x\n").is_err());
        assert_eq!(
            rule_imports(
                "@RTK.md\n@rust-ci-practices.md extra words\nnot an import\n  @indented.md\n"
            ),
            ["RTK.md", "rust-ci-practices.md", "indented.md"]
        );
    }
}
