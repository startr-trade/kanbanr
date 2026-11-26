//! Claude Code hook registration (FEAT-044). kanbanr's skill ships its hook scripts: SessionStart
//! (print the board so Claude resumes from it), Stop (nudge to record work), and session summaries
//! saved to the board (FEAT-101). They are registered
//! **once per machine** in the user's global Claude Code settings; the scripts themselves only act
//! in folders kanbanr tracks, so nothing is written into project folders.
//!
//! The settings file belongs to the user: it is merged, never replaced — other keys and hooks keep
//! their order, the write is atomic, and a file that isn't valid JSON is left alone.

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// (Claude Code event, script base name).
const CORE_HOOKS: [(&str, &str); 2] = [("SessionStart", "session-start"), ("Stop", "stop-check")];

/// Session summaries saved to the board (FEAT-099, shipped with the skill by FEAT-101): one script
/// for three events — a compaction's own summary, the end of a session, and a sweep at the next
/// start for sessions that ended without one. It is a bash + python script, so it is not offered
/// on Windows, where there is no equivalent yet.
const SUMMARY_HOOKS: [(&str, &str); 3] = [
    ("PostCompact", "session-summary"),
    ("SessionEnd", "session-summary"),
    ("SessionStart", "session-summary"),
];

/// Every script hook this platform registers.
fn script_hooks() -> Vec<(&'static str, &'static str)> {
    let mut hooks = CORE_HOOKS.to_vec();
    if !cfg!(windows) {
        hooks.extend(SUMMARY_HOOKS);
    }
    hooks
}

/// Hooks that run the CLI itself rather than a script in the skill folder: `(event, matcher,
/// command)`. Both watch Bash, because that is where test runs and commits happen — one reads the
/// output of a run to record which tracked tests actually passed (FEAT-053), the other checks a
/// `git commit` before it is attempted, so the feedback arrives with a suggestion rather than as a
/// failure after the fact (FEAT-056).
const CLI_HOOKS: [(&str, &str, &str); 5] = [
    ("PostToolUse", "Bash", "kanbanr capture"),
    ("PreToolUse", "Bash", "kanbanr git guard"),
    // Documentation belongs on the board (FEAT-040). The rule lived only in the skill's prose
    // until this hook; it decides from the path alone and stays silent where no board is active.
    ("PreToolUse", "Write|Edit", "kanbanr claude guard"),
    // A decision taken in the conversation (FTR-035): it reads the person's answer to a tagged
    // question from Claude Code's own record of it and makes the one-time pass the verdict needs.
    ("PostToolUse", "AskUserQuestion", "kanbanr decisions heard"),
    // It sees each tagged question before the person does (DEF-126): it refuses one the agent
    // answered itself and marks the rest, and the answer hook makes a pass only for a marked one.
    ("PreToolUse", "AskUserQuestion", "kanbanr decisions asked"),
];

/// Where a project's own hooks are registered: `<root>/.claude`, which Claude Code reads for that
/// project alone. This is the default scope, because kanbanr's hooks exist to serve a board — a
/// repository nobody tracks with kanbanr should carry none of them, and a machine should not answer
/// for every checkout on it. `--global` is the opt-in for someone who wants them everywhere.
pub fn project_config_dir(project_root: &Path) -> PathBuf {
    project_root.join(".claude")
}

/// The Claude Code config dir: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> Option<PathBuf> {
    kanbanr_core::claude::claude_config_dir()
}

pub fn settings_path(claude_dir: &Path) -> PathBuf {
    claude_dir.join("settings.json")
}

/// Where `make install-skill` puts the hook scripts, given the dir that holds the skill. The skill
/// is installed once per machine, so callers pass the HOME config dir here even when the settings
/// they are writing belong to one project — the project's settings then point at those same
/// scripts, rather than carrying copies that would drift.
pub fn scripts_dir(skill_home: &Path) -> PathBuf {
    skill_home.join("skills").join("kanbanr").join("hooks")
}

fn script_file(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.ps1")
    } else {
        format!("{name}.sh")
    }
}

fn script_command(scripts: &Path, name: &str) -> String {
    let path = scripts.join(script_file(name));
    if cfg!(windows) {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            path.display()
        )
    } else {
        path.display().to_string()
    }
}

/// The script a registered command runs: the quoted `-File` argument, or the command itself.
fn script_of(command: &str) -> PathBuf {
    match command.split_once("-File \"") {
        Some((_, rest)) => PathBuf::from(rest.split('"').next().unwrap_or(rest)),
        None => PathBuf::from(command.trim()),
    }
}

/// Is `command` kanbanr's own `name` script hook? Its script's file name is the script's, and the
/// folder holding it ends in `kanbanr/hooks`, compared by path components (L-46), so another
/// tool's command, or a user's own `~/bin/kanbanr-session-start-notes.sh`, never matches
/// (FTR-059 R-7). A substring match ("contains kanbanr and the name") once matched both.
fn is_kanbanr_hook(command: &str, name: &str) -> bool {
    let script = script_of(command);
    if script.file_name().and_then(|f| f.to_str()) != Some(script_file(name).as_str()) {
        return false;
    }
    let Some(folder) = script.parent() else {
        return false;
    };
    let mut parts = folder
        .components()
        .rev()
        .filter_map(|c| c.as_os_str().to_str());
    parts.next() == Some("hooks") && parts.next() == Some("kanbanr")
}

/// Every registered hook command, per event (`hooks.<event>[].hooks[].command`).
fn commands<'a>(settings: &'a Value, event: &str) -> impl Iterator<Item = &'a str> {
    settings["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
        .filter_map(|h| h["command"].as_str())
}

/// The kanbanr Claude Code plugin is enabled (it brings its own hooks).
pub fn plugin_enabled(settings: &Value) -> bool {
    settings["enabledPlugins"]
        .as_object()
        .is_some_and(|plugins| {
            plugins.iter().any(|(id, on)| {
                id.starts_with("kanbanr@") && !matches!(on, Value::Bool(false) | Value::Null)
            })
        })
}

/// Are the hooks that tell a sub-agent from the coordinator registered for this project or the
/// user: the Bash guard, the Write and Edit guard and the capture hook (DEF-027)? Each may sit in
/// the project's `settings.json` or `settings.local.json` or the user's `settings.json`, or the
/// kanbanr plugin, which brings them, is enabled. A scope is told apart from its agent by them.
pub fn identifying_hooks_registered(project_root: &Path) -> bool {
    identifying_hooks_registered_in(&kanbanr_core::claude::settings_files(project_root))
}

/// [`identifying_hooks_registered`] over the settings files given, so a test need not read the
/// user's own settings.
fn identifying_hooks_registered_in(files: &[PathBuf]) -> bool {
    let all: Vec<Value> = files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect();
    if all.iter().any(plugin_enabled) {
        return true;
    }
    CLI_HOOKS.iter().all(|(event, matcher, command)| {
        all.iter().any(|settings| {
            settings["hooks"][event]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|g| g["matcher"].as_str().unwrap_or("") == *matcher)
                .flat_map(|g| g["hooks"].as_array().into_iter().flatten())
                .filter_map(|h| h["command"].as_str())
                .any(|c| c.trim() == *command)
        })
    })
}

/// The script hooks not registered, as `(event, script)`: one event can carry two of them, so the
/// event alone cannot say which is missing. A registration whose script is gone counts as missing.
fn missing_scripts(settings: &Value) -> Vec<(&'static str, &'static str)> {
    script_hooks()
        .into_iter()
        .filter(|(event, name)| {
            !commands(settings, event).any(|c| is_kanbanr_hook(c, name) && script_of(c).is_file())
        })
        .collect()
}

/// The events whose kanbanr hook is not registered (a registration whose script no longer
/// exists counts as missing).
pub fn missing(settings: &Value) -> Vec<&'static str> {
    let mut missing: Vec<&'static str> = Vec::new();
    for (event, _) in missing_scripts(settings) {
        if !missing.contains(&event) {
            missing.push(event);
        }
    }
    // These run the CLI, so there is no script to check for — only the registration. Two share an
    // event, so the list is deduplicated: it is reported to a person, and "PreToolUse, PreToolUse"
    // reads like a bug.
    for (event, _, command) in CLI_HOOKS {
        if !commands(settings, event).any(|c| c.trim() == command) && !missing.contains(&event) {
            missing.push(event);
        }
    }
    missing
}

/// The events kanbanr registers that none of `files` registers (FTR-059 R-8): a hook counts as
/// registered when any one file holds it, so the project's `settings.local.json` and the user's
/// `settings.json` together can carry the set. Each file is read as JSON; one that is missing or
/// unreadable counts as empty. A registration whose script is gone counts as missing, as in
/// [`missing`]. In the order [`missing`] reports them, each event once.
pub fn missing_across(files: &[PathBuf]) -> Vec<&'static str> {
    let all: Vec<Value> = files
        .iter()
        .map(|f| {
            std::fs::read_to_string(f)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| json!({}))
        })
        .collect();
    let mut missing: Vec<&'static str> = Vec::new();
    for (event, name) in script_hooks() {
        let held = all
            .iter()
            .any(|s| !missing_scripts(s).contains(&(event, name)));
        if !held && !missing.contains(&event) {
            missing.push(event);
        }
    }
    for (event, _, command) in CLI_HOOKS {
        let held = all
            .iter()
            .any(|s| commands(s, event).any(|c| c.trim() == command));
        if !held && !missing.contains(&event) {
            missing.push(event);
        }
    }
    missing
}

fn read_all(files: &[PathBuf]) -> Vec<Value> {
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect()
}

/// Does `settings` register any of kanbanr's hooks, a CLI one or a script one?
fn holds_kanbanr_hooks(settings: &Value) -> bool {
    CLI_HOOKS
        .iter()
        .any(|(event, _, command)| commands(settings, event).any(|c| c.trim() == *command))
        || script_hooks()
            .into_iter()
            .any(|(event, name)| commands(settings, event).any(|c| is_kanbanr_hook(c, name)))
}

/// The [`CLI_HOOKS`] entries `(event, matcher, command)` that neither the project's settings
/// files nor the user's register (DEF-126): what a newer kanbanr registers that the settings
/// do not have yet. Empty when any file enables the kanbanr plugin, which brings its own hooks.
/// A file that is missing or does not parse counts as empty.
pub fn missing_cli_hooks(
    project: &[PathBuf],
    user: &[PathBuf],
) -> Vec<(&'static str, &'static str, &'static str)> {
    let all: Vec<Value> = read_all(project)
        .into_iter()
        .chain(read_all(user))
        .collect();
    if all.iter().any(plugin_enabled) {
        return Vec::new();
    }
    CLI_HOOKS
        .into_iter()
        .filter(|(event, matcher, command)| {
            !all.iter().any(|settings| {
                settings["hooks"][event]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|g| g["matcher"].as_str().unwrap_or("") == *matcher)
                    .flat_map(|g| g["hooks"].as_array().into_iter().flatten())
                    .filter_map(|h| h["command"].as_str())
                    .any(|c| c.trim() == *command)
            })
        })
        .collect()
}

/// The session summary's line when this kanbanr registers a hook the settings lack (DEF-126
/// R-5), naming where the existing kanbanr hooks are, so the refresh goes to the same place:
/// `None` when nothing is missing. `project` is the project's settings files, `user` the user's.
pub fn out_of_date_line(project: &[PathBuf], user: &[PathBuf]) -> Option<String> {
    let missing = missing_cli_hooks(project, user);
    if missing.is_empty() {
        return None;
    }
    let names = missing
        .iter()
        .map(|(_, _, c)| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let verb = if missing.len() == 1 { "is" } else { "are" };
    let in_project = read_all(project).iter().any(holds_kanbanr_hooks);
    let in_user = read_all(user).iter().any(holds_kanbanr_hooks);
    let place = match (in_project, in_user) {
        (true, true) => {
            "kanbanr's other hooks are in both the project's and the user's settings: run it \
             there and with `--global`"
        }
        (false, true) => "kanbanr's other hooks are in the user's settings: run it with `--global`",
        _ => "kanbanr's other hooks are in the project's settings: run it there",
    };
    Some(format!(
        "kanbanr's hooks are out of date: {names} {verb} not registered. Offer the person \
         `kanbanr hooks install` (then a new session); {place}."
    ))
}

/// Remove kanbanr's hook entries (all of them, or only those whose script is gone), dropping
/// groups and events left empty. Returns how many entries were removed.
fn remove(settings: &mut Value, only_stale: bool) -> usize {
    let mut removed = 0;
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    // These are matched by their command, not by a script path, so they are never "stale" — and
    // every other hook on the same event (another tool's, the user's own) is left untouched.
    for (event, _, command) in CLI_HOOKS {
        if only_stale {
            break;
        }
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = entries.len();
                entries.retain(|h| h["command"].as_str().unwrap_or("").trim() != command);
                removed += before - entries.len();
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|e| !e.is_empty()));
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    for (event, name) in script_hooks() {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = entries.len();
                entries.retain(|h| {
                    let c = h["command"].as_str().unwrap_or("");
                    !(is_kanbanr_hook(c, name) && (!only_stale || !script_of(c).is_file()))
                });
                removed += before - entries.len();
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|e| !e.is_empty()));
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        settings.as_object_mut().map(|s| s.remove("hooks"));
    }
    removed
}

/// Add the missing kanbanr hooks (after dropping stale ones). Returns the events added.
pub fn add(settings: &mut Value, scripts: &Path) -> anyhow::Result<Vec<&'static str>> {
    if !settings.is_object() {
        return Err(anyhow!("settings.json is not a JSON object"));
    }
    remove(settings, true);
    let added = missing(settings);
    let scripts_wanted = missing_scripts(settings);
    // Which CLI hooks are absent has to be decided BEFORE the settings are borrowed mutably — and
    // by command, not by event: two of them share PreToolUse, so matching on the event alone would
    // re-register one that is already there.
    let wanted: Vec<(&str, &str, &str)> = CLI_HOOKS
        .into_iter()
        .filter(|(event, _, command)| !commands(settings, event).any(|c| c.trim() == *command))
        .collect();
    let root = settings.as_object_mut().expect("checked above");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("\"hooks\" in settings.json is not an object"))?;
    for (event, matcher, command) in &wanted {
        hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| anyhow!("\"hooks.{event}\" in settings.json is not a list"))?
            .push(json!({
                "matcher": matcher,
                "hooks": [{ "type": "command", "command": command }],
            }));
    }
    for (event, name) in &scripts_wanted {
        let mut hook = json!({"type": "command", "command": script_command(scripts, name)});
        match *name {
            "session-start" => hook["statusMessage"] = json!("Recovering kanbanr board"),
            // It returns at once and detaches anything slow; the timeout only bounds a stuck write.
            "session-summary" => hook["timeout"] = json!(60),
            _ => {}
        }
        hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| anyhow!("\"hooks.{event}\" in settings.json is not a list"))?
            .push(json!({"hooks": [hook]}));
    }
    Ok(added)
}

/// The settings a walkthrough run gives its scratch project (FTR-037 W5): the run-only
/// `AskUserQuestion` hook that answers every question from the runbook, the guards and the capture
/// hook the project would have anyway (`kanbanr git guard` reads the run's allowed remotes), and
/// `Bash(kanbanr:*)` allowed. Written to the run project's `.claude/settings.local.json`; never
/// into the person's own settings.
pub fn walkthrough_project_settings() -> Value {
    json!({
        "permissions": {"allow": ["Bash(kanbanr:*)"]},
        "hooks": {
            "PreToolUse": [
                {"matcher": "AskUserQuestion", "hooks": [{"type": "command", "command": "kanbanr walkthrough answer"}]},
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "kanbanr git guard"}]},
                {"matcher": "Write|Edit", "hooks": [{"type": "command", "command": "kanbanr claude guard"}]}
            ],
            "PostToolUse": [
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "kanbanr capture"}]}
            ]
        }
    })
}

/// Write the walkthrough settings into `project_root/.claude/settings.local.json` (replacing a
/// file an earlier run left: the scratch project is the run's own).
pub fn write_walkthrough_settings(project_root: &Path) -> anyhow::Result<PathBuf> {
    let path = project_config_dir(project_root).join("settings.local.json");
    write(&path, &walkthrough_project_settings())?;
    Ok(path)
}

fn read(path: &Path) -> anyhow::Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(json!({})),
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            anyhow!(
                "{} is not valid JSON ({e}); leaving it untouched",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

/// Write whole or not at all (DEF-025), so an interrupted write never leaves a broken settings file.
fn write(path: &Path, settings: &Value) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    kanbanr_core::atomic::write(path, serde_json::to_string_pretty(settings)? + "\n")
        .with_context(|| format!("could not write {}", path.display()))
}

#[derive(Debug, PartialEq)]
pub enum Installed {
    Added(Vec<&'static str>),
    AlreadyPresent,
    ProvidedByPlugin,
    /// The skill (and so the scripts) isn't installed at this path.
    SkillMissing(PathBuf),
}

/// Register the hooks in `settings_dir`, pointing at the scripts in `scripts`. The two are
/// separate because the settings may belong to one project while the scripts are installed once
/// per machine (FEAT-065).
pub fn install_in(settings_dir: &Path, scripts: &Path) -> anyhow::Result<Installed> {
    let path = settings_path(settings_dir);
    let mut settings = read(&path)?;
    if plugin_enabled(&settings) {
        return Ok(Installed::ProvidedByPlugin);
    }
    if missing(&settings).is_empty() {
        return Ok(Installed::AlreadyPresent);
    }
    if !script_hooks()
        .iter()
        .all(|(_, name)| scripts.join(script_file(name)).is_file())
    {
        return Ok(Installed::SkillMissing(scripts.to_path_buf()));
    }
    let added = add(&mut settings, scripts)?;
    write(&path, &settings)?;
    Ok(Installed::Added(added))
}

/// Settings and scripts in the same place. Used by the tests, where one temp dir stands in for
/// both; the CLI always passes them separately.
#[cfg(test)]
pub fn install(claude_dir: &Path) -> anyhow::Result<Installed> {
    install_in(claude_dir, &scripts_dir(claude_dir))
}

/// Remove kanbanr's hooks; returns how many entries were removed.
pub fn uninstall(claude_dir: &Path) -> anyhow::Result<usize> {
    let path = settings_path(claude_dir);
    let mut settings = read(&path)?;
    let removed = remove(&mut settings, false);
    if removed > 0 {
        write(&path, &settings)?;
    }
    Ok(removed)
}

#[derive(Debug, serde::Serialize)]
pub struct Status {
    pub settings: String,
    pub plugin: bool,
    pub skill_installed: bool,
    /// Per event: the registered kanbanr command, if any, and whether its script exists.
    pub hooks: Vec<Value>,
    /// What a `PreToolUse` hook on `AskUserQuestion` in this settings file means for decisions
    /// taken in the conversation (FTR-035): a warning for a hook that can answer for the person,
    /// a note for the walkthrough's own. Empty when there is none.
    pub ask_user_question: Vec<String>,
}

/// The command a walkthrough run project registers on `AskUserQuestion` (FTR-037).
const WALKTHROUGH_ANSWER: &str = "kanbanr walkthrough answer";

/// One line per `PreToolUse` hook on `AskUserQuestion` in `settings` (FTR-035): such a hook can
/// answer the person's question through `updatedInput`, so a conversation decision made while one
/// is registered gets no pass. The walkthrough's own hook is named, not warned of.
fn ask_user_question_lines(settings: &Value) -> Vec<String> {
    settings["hooks"]["PreToolUse"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|group| {
            group["matcher"]
                .as_str()
                .is_some_and(|m| m.split('|').any(|t| t.trim() == "AskUserQuestion"))
        })
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
        .filter_map(|h| h["command"].as_str())
        .map(|command| {
            if command.trim() == WALKTHROUGH_ANSWER {
                format!(
                    "AskUserQuestion is answered by `{WALKTHROUGH_ANSWER}` (a walkthrough run \
                     project); no pass is made here"
                )
            } else {
                format!(
                    "a PreToolUse hook on AskUserQuestion (`{}`) can answer questions for the \
                     person; a conversation decision made while it is registered gets no pass",
                    command.trim()
                )
            }
        })
        .collect()
}

/// What is registered in `settings_dir`, checked against the scripts in `scripts`.
pub fn status_in(settings_dir: &Path, scripts: &Path) -> anyhow::Result<Status> {
    let path = settings_path(settings_dir);
    let settings = read(&path)?;
    let scripts = scripts.to_path_buf();
    Ok(Status {
        settings: path.display().to_string(),
        plugin: plugin_enabled(&settings),
        skill_installed: script_hooks()
            .iter()
            .all(|(_, name)| scripts.join(script_file(name)).is_file()),
        hooks: script_hooks()
            .iter()
            .map(|(event, name)| {
                let command = commands(&settings, event).find(|c| is_kanbanr_hook(c, name));
                json!({
                    "event": event,
                    "command": command,
                    "script_exists": command.map(|c| script_of(c).is_file()),
                })
            })
            // These run the CLI, so "does its script exist" is not a question that applies:
            // being registered is the whole of their health.
            .chain(CLI_HOOKS.iter().map(|(event, _, command)| {
                let found = commands(&settings, event).find(|c| c.trim() == *command);
                json!({
                    "event": event,
                    "command": found,
                    "script_exists": found.map(|_| true),
                })
            }))
            .collect(),
        ask_user_question: ask_user_question_lines(&settings),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_claude_dir(with_skill: bool) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-hooks-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if with_skill {
            let scripts = scripts_dir(&dir);
            std::fs::create_dir_all(&scripts).unwrap();
            for (_, name) in script_hooks() {
                std::fs::write(scripts.join(script_file(name)), "#!/bin/sh\nexit 0\n").unwrap();
            }
        }
        dir
    }

    /// Every event install adds to empty settings, in the order it reports them.
    fn all_events() -> Vec<&'static str> {
        let mut events = Vec::new();
        for event in script_hooks()
            .into_iter()
            .map(|(e, _)| e)
            .chain(CLI_HOOKS.iter().map(|(e, _, _)| *e))
        {
            if !events.contains(&event) {
                events.push(event);
            }
        }
        events
    }

    /// The summary script's registrations, as `(event, command)`.
    fn summary_commands(v: &Value) -> Vec<(String, String)> {
        ["PostCompact", "SessionEnd", "SessionStart"]
            .iter()
            .flat_map(|event| {
                commands(v, event)
                    .filter(|c| is_kanbanr_hook(c, "session-summary"))
                    .map(|c| (event.to_string(), c.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// FEAT-101 R-1: the summary hooks come with the install, pointing at the skill's own script,
    /// beside the session-start hook on the event they share — and a second install adds nothing.
    #[cfg(not(windows))]
    #[test]
    fn install_registers_session_summary() {
        let dir = temp_claude_dir(true);
        assert!(matches!(install(&dir).unwrap(), Installed::Added(_)));
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        let script = scripts_dir(&dir).join("session-summary.sh");
        let registered = summary_commands(&v);
        assert_eq!(
            registered
                .iter()
                .map(|(e, _)| e.as_str())
                .collect::<Vec<_>>(),
            ["PostCompact", "SessionEnd", "SessionStart"]
        );
        assert!(registered.iter().all(|(_, c)| script_of(c) == script));
        assert!(
            commands(&v, "SessionStart").any(|c| is_kanbanr_hook(c, "session-start")),
            "the board-recovery hook keeps its place beside it"
        );
        let summary = v["hooks"]["PostCompact"][0]["hooks"][0].clone();
        assert_eq!(summary["timeout"], 60);

        // With only the summary hook missing on SessionStart, only it is added back — the
        // event-level view would have re-added the recovery hook too.
        let mut partial = v.clone();
        partial["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .retain(|g| {
                !g["hooks"][0]["command"]
                    .as_str()
                    .unwrap()
                    .contains("session-summary")
            });
        assert_eq!(
            add(&mut partial, &scripts_dir(&dir)).unwrap(),
            ["SessionStart"]
        );
        assert_eq!(partial, v);

        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
    }

    /// FEAT-101 R-2: uninstall takes the summary hooks away with the rest, and nobody else's.
    #[cfg(not(windows))]
    #[test]
    fn uninstall_removes_session_summary() {
        let dir = temp_claude_dir(true);
        std::fs::write(
            settings_path(&dir),
            r#"{"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "notify-send bye"}]}]}}"#,
        )
        .unwrap();
        install(&dir).unwrap();
        uninstall(&dir).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        assert!(summary_commands(&v).is_empty(), "{v}");
        assert!(v["hooks"].get("PostCompact").is_none());
        assert_eq!(
            v["hooks"]["SessionEnd"][0]["hooks"][0]["command"],
            "notify-send bye"
        );
    }

    /// FEAT-102 R-2: the scripts the skill ships are the real ones. The tests' own fake scripts are
    /// `exit 0` stubs, and one was committed over the shipped scripts once: every session lost its
    /// board recovery while `hooks status` still reported them healthy, because the files existed.
    #[test]
    fn shipped_scripts_are_not_stubs() {
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skill/kanbanr/hooks");
        let names = CORE_HOOKS
            .iter()
            .chain(SUMMARY_HOOKS.iter())
            .map(|(_, n)| *n);
        for name in names {
            let file = shipped.join(format!("{name}.sh"));
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} is missing: {e}", file.display()));
            let code: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            assert!(
                code.len() > 5 && code != ["exit 0"],
                "{} is a stub, not the shipped hook",
                file.display()
            );
        }
    }

    #[test]
    fn install_merges_into_existing_settings_keeping_order_and_other_hooks() {
        let dir = temp_claude_dir(true);
        let original = r#"{
  "theme": "dark",
  "hooks": {
    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "rtk hook claude"}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "notify-send done"}]}]
  },
  "model": "opus"
}"#;
        std::fs::write(settings_path(&dir), original).unwrap();

        assert_eq!(install(&dir).unwrap(), Installed::Added(all_events()));
        let text = std::fs::read_to_string(settings_path(&dir)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "hooks", "model"], "key order preserved");
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "rtk hook claude"
        );
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            "notify-send done"
        );
        assert!(
            v["hooks"]["Stop"][1]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("stop-check")
        );
        assert!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("session-start")
        );
        // The capture hook runs the CLI on Bash commands, so test runs record themselves.
        let capture = &v["hooks"]["PostToolUse"][0];
        assert_eq!(capture["matcher"], "Bash");
        assert_eq!(capture["hooks"][0]["command"], "kanbanr capture");
        // Two guards sit on PreToolUse, beside whatever else was already watching Bash: the commit
        // check on Bash, and the documentation-location check on file writes.
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        let commit = pre
            .iter()
            .find(|g| g["hooks"][0]["command"] == "kanbanr git guard")
            .expect("the commit guard is registered");
        assert_eq!(commit["matcher"], "Bash");
        let docs = pre
            .iter()
            .find(|g| g["hooks"][0]["command"] == "kanbanr claude guard")
            .expect("the docs guard is registered");
        assert_eq!(docs["matcher"], "Write|Edit");
        // Registering again adds neither of them a second time — they share an event, and matching
        // on the event alone used to re-add whichever was already there.
        let before = pre.len();
        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
        let v2: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        assert_eq!(v2["hooks"]["PreToolUse"].as_array().unwrap().len(), before);

        // Idempotent.
        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
        assert_eq!(std::fs::read_to_string(settings_path(&dir)).unwrap(), text);

        // Uninstall removes only kanbanr's entries.
        assert_eq!(
            uninstall(&dir).unwrap(),
            script_hooks().len() + CLI_HOOKS.len(),
            "every script hook plus those that run the CLI"
        );
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        assert!(v["hooks"].get("SessionStart").is_none());
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "rtk hook claude"
        );
    }

    #[test]
    fn install_creates_settings_when_absent_and_repairs_stale_paths() {
        let dir = temp_claude_dir(true);
        assert!(matches!(install(&dir).unwrap(), Installed::Added(_)));

        // A registration pointing at a script that no longer exists is replaced.
        let stale = json!({"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": "/old/place/kanbanr/hooks/session-start.sh"}
        ]}]}});
        std::fs::write(settings_path(&dir), stale.to_string()).unwrap();
        assert_eq!(install(&dir).unwrap(), Installed::Added(all_events()));
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        let starts: Vec<&str> = commands(&v, "SessionStart")
            .filter(|c| is_kanbanr_hook(c, "session-start"))
            .collect();
        assert_eq!(starts.len(), 1, "stale entry replaced, not kept: {v}");
        assert!(!starts[0].starts_with("/old/place"));
    }

    /// FTR-035 R-2: `hooks status` warns of a `PreToolUse` hook on AskUserQuestion that could
    /// answer for the person, names the walkthrough's own without warning, and says nothing for a
    /// settings file with neither.
    #[test]
    fn status_warns_of_a_pre_tool_use_hook_on_ask_user_question_but_not_the_walkthroughs() {
        let status = |settings: &Value| {
            let dir = temp_claude_dir(true);
            std::fs::write(settings_path(&dir), settings.to_string()).unwrap();
            status_in(&dir, &scripts_dir(&dir))
                .unwrap()
                .ask_user_question
        };
        let other = json!({"hooks": {"PreToolUse": [
            {"matcher": "AskUserQuestion", "hooks": [{"type": "command", "command": "auto-answer --yes"}]},
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "kanbanr git guard"}]}
        ]}});
        let lines = status(&other);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("`auto-answer --yes`"), "{lines:?}");
        assert!(lines[0].contains("can answer questions for the person"));
        assert!(lines[0].contains("gets no pass"));

        let lines = status(&walkthrough_project_settings());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("answered by `kanbanr walkthrough answer`"),
            "{lines:?}"
        );
        assert!(!lines[0].contains("can answer questions for the person"));

        let neither = json!({"hooks": {"PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "kanbanr git guard"}]}
        ]}});
        assert!(status(&neither).is_empty());
        assert!(status(&json!({})).is_empty());
    }

    /// DEF-126 R-4: install registers the question hook (PreToolUse on AskUserQuestion, `kanbanr
    /// decisions asked`) once, beside the Bash guard on its event; settings without it are missing
    /// PreToolUse, and only it is added back.
    #[test]
    fn hooks_install_registers_decisions_asked() {
        let dir = temp_claude_dir(true);
        assert!(matches!(install(&dir).unwrap(), Installed::Added(_)));
        let read = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap()
        };
        let asked = |v: &Value| -> Vec<Value> {
            v["hooks"]["PreToolUse"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|g| g["hooks"][0]["command"] == "kanbanr decisions asked")
                .cloned()
                .collect()
        };
        let v = read();
        let groups = asked(&v);
        assert_eq!(groups.len(), 1, "{v}");
        assert_eq!(groups[0]["matcher"], "AskUserQuestion");
        assert!(commands(&v, "PreToolUse").any(|c| c == "kanbanr git guard"));
        assert!(missing(&v).is_empty());
        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
        assert_eq!(asked(&read()).len(), 1);

        let mut partial = v.clone();
        partial["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .retain(|g| g["hooks"][0]["command"] != "kanbanr decisions asked");
        assert_eq!(missing(&partial), ["PreToolUse"]);
        assert_eq!(
            add(&mut partial, &scripts_dir(&dir)).unwrap(),
            ["PreToolUse"]
        );
        assert_eq!(asked(&partial).len(), 1);
    }

    /// FTR-035 R-1: install registers the AskUserQuestion hook that makes a pass from the person's
    /// answer, once; uninstall takes it away and leaves the other hooks on its event alone; and a
    /// scope is not told apart from its agent without it.
    #[test]
    fn install_registers_the_ask_user_question_hook() {
        let dir = temp_claude_dir(true);
        std::fs::write(
            settings_path(&dir),
            r#"{"hooks": {"PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "notify-send ran"}]}]}}"#,
        )
        .unwrap();
        assert!(matches!(install(&dir).unwrap(), Installed::Added(_)));
        let read = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap()
        };
        let heard = |v: &Value| -> Vec<Value> {
            v["hooks"]["PostToolUse"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|g| g["hooks"][0]["command"] == "kanbanr decisions heard")
                .cloned()
                .collect()
        };
        let v = read();
        let groups = heard(&v);
        assert_eq!(groups.len(), 1, "{v}");
        assert_eq!(groups[0]["matcher"], "AskUserQuestion");
        assert!(
            commands(&v, "PostToolUse").any(|c| c == "kanbanr capture"),
            "the capture hook sits beside it"
        );
        assert!(missing(&v).is_empty());

        // A second install adds nothing.
        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
        assert_eq!(heard(&read()).len(), 1);

        // Without it the set is incomplete: PostToolUse is missing, and only it is added back.
        let mut partial = v.clone();
        partial["hooks"]["PostToolUse"]
            .as_array_mut()
            .unwrap()
            .retain(|g| g["hooks"][0]["command"] != "kanbanr decisions heard");
        assert_eq!(missing(&partial), ["PostToolUse"]);
        let scope_files = |settings: &Value| {
            let root = temp_claude_dir(false);
            let file = root.join("settings.json");
            std::fs::write(&file, settings.to_string()).unwrap();
            vec![file]
        };
        assert!(identifying_hooks_registered_in(&scope_files(&v)));
        assert!(
            !identifying_hooks_registered_in(&scope_files(&partial)),
            "all five CLI hooks must be registered"
        );
        assert_eq!(
            add(&mut partial, &scripts_dir(&dir)).unwrap(),
            ["PostToolUse"]
        );
        assert_eq!(heard(&partial).len(), 1);

        // Uninstall removes it, and leaves the person's own hook on the same event.
        uninstall(&dir).unwrap();
        let v = read();
        assert!(heard(&v).is_empty(), "{v}");
        assert_eq!(
            commands(&v, "PostToolUse").collect::<Vec<_>>(),
            ["notify-send ran"]
        );
    }

    #[test]
    fn install_skips_plugin_missing_skill_and_invalid_json() {
        let dir = temp_claude_dir(true);
        std::fs::write(
            settings_path(&dir),
            r#"{"enabledPlugins": {"kanbanr@startr-trade": true}}"#,
        )
        .unwrap();
        assert_eq!(install(&dir).unwrap(), Installed::ProvidedByPlugin);

        let bare = temp_claude_dir(false);
        assert!(matches!(
            install(&bare).unwrap(),
            Installed::SkillMissing(_)
        ));
        assert!(!settings_path(&bare).exists(), "nothing written");

        let broken = temp_claude_dir(true);
        std::fs::write(settings_path(&broken), "{ not json").unwrap();
        assert!(install(&broken).is_err());
        assert_eq!(
            std::fs::read_to_string(settings_path(&broken)).unwrap(),
            "{ not json"
        );
    }

    /// FTR-059 R-7: install and uninstall leave every other tool's entry where it was, byte for
    /// byte: katalogr's SessionStart, a command-rewriting PreToolUse Bash hook, and a user's own
    /// script whose name holds both "kanbanr" and "session-start" (which the old substring match
    /// took for kanbanr's and uninstall removed).
    #[test]
    fn install_and_uninstall_leave_other_tools_entries_in_place() {
        let dir = temp_claude_dir(true);
        let katalogr = json!({"type": "command", "command": "katalogr session-start"});
        let rtk = json!({"type": "command", "command": "rtk hook claude"});
        let notes = json!({"type": "command", "command": "~/bin/kanbanr-session-start-notes.sh"});
        let original = json!({
            "hooks": {
                "SessionStart": [
                    {"hooks": [katalogr.clone()]},
                    {"hooks": [notes.clone()]}
                ],
                "PreToolUse": [{"matcher": "Bash", "hooks": [rtk.clone()]}]
            }
        });
        let original_text = serde_json::to_string_pretty(&original).unwrap() + "\n";
        std::fs::write(settings_path(&dir), &original_text).unwrap();
        let theirs = |v: &Value| -> Vec<String> {
            ["SessionStart", "PreToolUse"]
                .iter()
                .flat_map(|e| v["hooks"][e].as_array().cloned().unwrap_or_default())
                .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                .filter(|h| [&katalogr, &rtk, &notes].contains(&h))
                .map(|h| h.to_string())
                .collect()
        };
        let read_back = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap()
        };
        let before = theirs(&original);
        assert_eq!(before.len(), 3);

        assert_eq!(install(&dir).unwrap(), Installed::Added(all_events()));
        let installed = read_back();
        assert_eq!(theirs(&installed), before, "kept, in order, after install");
        // Their groups are still the first ones on each event: kanbanr's come after.
        assert_eq!(installed["hooks"]["SessionStart"][0]["hooks"][0], katalogr);
        assert_eq!(installed["hooks"]["SessionStart"][1]["hooks"][0], notes);
        assert_eq!(installed["hooks"]["PreToolUse"][0]["hooks"][0], rtk);
        // Only kanbanr's own entries came: every added one is kanbanr's.
        let added: Vec<String> = installed["hooks"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(event, groups)| {
                groups
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|g| g["hooks"].as_array().unwrap().clone())
                    .filter_map(|h| h["command"].as_str().map(str::to_string))
                    .filter(|c| {
                        ![&katalogr, &rtk, &notes]
                            .iter()
                            .any(|o| o["command"] == c.as_str())
                    })
                    .map(|c| format!("{event} {c}"))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(added.len(), script_hooks().len() + CLI_HOOKS.len());
        for entry in &added {
            let (event, command) = entry.split_once(' ').unwrap();
            assert!(
                script_hooks()
                    .iter()
                    .any(|(e, n)| *e == event && is_kanbanr_hook(command, n))
                    || CLI_HOOKS
                        .iter()
                        .any(|(e, _, c)| *e == event && *c == command),
                "{entry} is kanbanr's own"
            );
        }

        assert_eq!(
            uninstall(&dir).unwrap(),
            script_hooks().len() + CLI_HOOKS.len(),
            "only kanbanr's own entries go"
        );
        let after = read_back();
        assert_eq!(theirs(&after), before, "kept, in order, after uninstall");
        assert_eq!(after, original);
        assert_eq!(
            std::fs::read_to_string(settings_path(&dir)).unwrap(),
            original_text,
            "the file is byte-equal to what it was"
        );

        // The matching rule itself: script file and `kanbanr/hooks` folder, by components.
        assert!(is_kanbanr_hook(
            &script_command(&scripts_dir(&dir), "session-start"),
            "session-start"
        ));
        assert!(!is_kanbanr_hook(
            "~/bin/kanbanr-session-start-notes.sh",
            "session-start"
        ));
        assert!(!is_kanbanr_hook(
            "/opt/kanbanr/session-start.sh",
            "session-start"
        ));
        assert!(!is_kanbanr_hook("katalogr session-start", "session-start"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-059 R-6: the Stop check never loops with another tool's blocking Stop hook. Run as
    /// Claude Code runs it, in a tracked temp project whose board is stale and whose tree is
    /// dirty: with `stop_hook_active: true` (a Stop hook, any tool's, already blocked this stop)
    /// it prints nothing; otherwise it blocks once, and the same session again gets nothing. A stub
    /// `kanbanr` first on PATH answers nothing, so no real board is read (L-41).
    #[cfg(unix)]
    #[test]
    fn the_stop_check_blocks_once_and_never_after_another_hooks_block() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        let dir = temp_claude_dir(false);
        assert!(dir.starts_with(std::env::temp_dir()));
        let git = |cwd: &Path, args: &[&str]| {
            let out = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@x"])
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        // The board: a git repo whose last commit is years old.
        let board = dir.join("board");
        std::fs::create_dir_all(&board).unwrap();
        git(&board, &["init", "-q"]);
        std::fs::write(board.join("x.yaml"), "x: 1\n").unwrap();
        git(&board, &["add", "."]);
        git(&board, &["commit", "-q", "--no-verify", "-m", "old"]);
        // The project: tracked, with work the board has not seen.
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q"]);
        std::fs::write(project.join(".kanbanr"), "project = \"stub\"\n").unwrap();
        std::fs::write(project.join("work.txt"), "unrecorded\n").unwrap();
        // A stub kanbanr: every call answers nothing and fails.
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stub = bin.join("kanbanr");
        std::fs::write(&stub, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = dir.join("stop-check.sh");
        std::fs::write(&script, embedded_script("hooks/stop-check.sh")).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |input: &str| -> String {
            use std::io::Write;
            let mut child = Command::new("bash")
                .arg(&script)
                .current_dir(&project)
                .env("PATH", &path)
                .env("TMPDIR", &dir)
                .env("KANBANR_DATA_DIR", &board)
                .env_remove("KANBANR_PROJECT")
                .env_remove("KANBANR_STOP_WINDOW_MIN")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "the Stop check exits 0: {out:?}");
            String::from_utf8_lossy(&out.stdout).to_string()
        };

        // Another hook already blocked this stop: nothing, before any other rule.
        assert_eq!(
            run(r#"{"session_id":"s-ftr059","stop_hook_active":true}"#),
            ""
        );
        // A fresh stop: one block.
        let first = run(r#"{"session_id":"s-ftr059","stop_hook_active":false}"#);
        let blocks = first
            .lines()
            .filter(|l| l.contains(r#""decision":"block""#));
        assert_eq!(blocks.count(), 1, "{first}");
        // The same session, the same reason: nothing more.
        assert_eq!(
            run(r#"{"session_id":"s-ftr059","stop_hook_active":false}"#),
            ""
        );
        // And while continuing because of that block: nothing.
        assert_eq!(
            run(r#"{"session_id":"s-ftr059","stop_hook_active":true}"#),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The hook script the binary carries, as `skill install` writes it.
    fn embedded_script(rel: &str) -> String {
        crate::skill::SKILL_FILES
            .iter()
            .find(|(r, _)| *r == rel)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_else(|| panic!("{rel} is not embedded"))
    }

    /// DEF-093 R-4, R-5: both SessionStart scripts print `kanbanr recover`, keep the 0.7.0
    /// sequence only as the fallback for an older kanbanr, cut that fallback to the budget, and
    /// still always exit 0.
    #[test]
    fn the_session_start_scripts_print_the_budgeted_summary() {
        let cases = [
            (
                "hooks/session-start.sh",
                "if ! kanbanr ${PROJECT_ARGS[@]+\"${PROJECT_ARGS[@]}\"} recover 2>/dev/null; then",
                "kanbanr ${PROJECT_ARGS[@]+\"${PROJECT_ARGS[@]}\"} board",
                "| head -c \"${KANBANR_SESSION_BUDGET:-8000}\"",
            ),
            (
                "hooks/session-start.ps1",
                "$out = & kanbanr recover 2>$null",
                "& kanbanr board",
                "| Out-String | ForEach-Object { $_.Substring(0, [Math]::Min($_.Length, $budget)) }",
            ),
        ];
        for (rel, recover_call, board_call, cut) in cases {
            let text = embedded_script(rel);
            let at = |needle: &str| {
                let found: Vec<_> = text.match_indices(needle).map(|(i, _)| i).collect();
                assert_eq!(
                    found.len(),
                    1,
                    "{rel}: `{needle}` once, found {}",
                    found.len()
                );
                found[0]
            };
            let (recover, board, cut) = (at(recover_call), at(board_call), at(cut));
            assert!(
                recover < board && board < cut,
                "{rel}: the 0.7.0 sequence runs only when recover fails, and is cut to the budget"
            );
            let last = text
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty() && !l.starts_with('#'));
            assert_eq!(last, Some("exit 0"), "{rel} ends with exit 0");
        }
        #[cfg(unix)]
        run_session_start_against_a_stub();
    }

    /// The 1-based lines of `script` that expand an array bare (`"${NAME[@]}"` or `${NAME[@]}`).
    /// Under `set -u`, bash before 4.4 (macOS's `/bin/bash`) reports an empty array expanded that
    /// way as unbound and stops; the safe form `${NAME[@]+"${NAME[@]}"}` is not flagged. Comment
    /// lines are skipped.
    fn bare_array_expansions(script: &str) -> Vec<usize> {
        let mut lines = Vec::new();
        for (i, line) in script.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            for (at, _) in line.match_indices("${") {
                let after = &line[at + 2..];
                let name_len = after
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
                if name_len > 0
                    && after[name_len..].starts_with("[@]}")
                    && !line[..at].ends_with("+\"")
                {
                    lines.push(i + 1);
                    break;
                }
            }
        }
        lines
    }

    /// DEF-101 R-1: under `set -u` no embedded hook script expands a possibly-empty array bare
    /// (bash 3.2 stops on it, so the hook prints nothing on stock macOS), and none uses a bash
    /// 4-only construct. The detector is proved on a fixture first.
    #[test]
    fn the_hook_scripts_expand_arrays_the_bash_3_2_way() {
        let fixture =
            "PA=()\necho \"${PA[@]}\"\necho ${PA[@]+\"${PA[@]}\"}\n# \"${PA[@]}\" in a comment\n";
        assert_eq!(bare_array_expansions(fixture), vec![2]);

        let mut checked = 0;
        for (rel, bytes) in crate::skill::SKILL_FILES {
            if !rel.starts_with("hooks/") || !rel.ends_with(".sh") {
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            if !text.contains("set -u") {
                continue;
            }
            checked += 1;
            let bare = bare_array_expansions(&text);
            assert!(
                bare.is_empty(),
                "{rel}: bare array expansion under set -u on lines {bare:?}; use ${{NAME[@]+\"${{NAME[@]}}\"}}"
            );
            for construct in ["mapfile", "readarray", "declare -A", ",,}", "^^}"] {
                assert!(
                    !text.contains(construct),
                    "{rel}: `{construct}` needs bash 4 and macOS ships 3.2"
                );
            }
        }
        assert!(
            checked >= 2,
            "the session and stop hooks are checked, found {checked}"
        );
    }

    /// Runs the embedded `session-start.sh` in a temp dir whose PATH starts with a stub
    /// `kanbanr`: no real board is read or written (L-41). Unix only: bash 3.2 on macOS runs it
    /// too, with an empty project-argument array under `set -u` (DEF-101).
    #[cfg(unix)]
    fn run_session_start_against_a_stub() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_claude_dir(false);
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let board: String = (0..400)
            .map(|n| {
                format!("  - FTR-{n:03} [Planned] a planned item, long enough to add up [0/0]\n")
            })
            .collect();
        assert!(board.len() > 20_000);
        std::fs::write(bin.join("board.txt"), &board).unwrap();
        let stub = bin.join("kanbanr");
        std::fs::write(
            &stub,
            "#!/bin/sh\n\
             case \"$1\" in\n\
             recover) if [ -n \"${STUB_RECOVER:-}\" ]; then echo RECOVERED; exit 0; fi\n\
             echo \"error: unrecognized subcommand 'recover'\" >&2; exit 2 ;;\n\
             board) cat \"$(dirname \"$0\")/board.txt\" ;;\n\
             *) exit 0 ;;\n\
             esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let work = dir.join("project");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join(".kanbanr"), "project = \"stub\"\n").unwrap();
        let script = dir.join("session-start.sh");
        std::fs::write(&script, embedded_script("hooks/session-start.sh")).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |recover_works: bool| {
            let mut cmd = std::process::Command::new("bash");
            cmd.arg(&script)
                .current_dir(&work)
                .env("PATH", &path)
                .env_remove("KANBANR_PROJECT")
                .env_remove("KANBANR_SESSION_BUDGET")
                .env_remove("KANBANR_DATA_DIR");
            if recover_works {
                cmd.env("STUB_RECOVER", "1");
            } else {
                cmd.env_remove("STUB_RECOVER");
            }
            cmd.output().unwrap()
        };

        // A kanbanr with `recover`: its output is the hook's, and nothing else runs.
        let out = run(true);
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "RECOVERED\n");

        // An older kanbanr: the 0.7.0 sequence, cut to the budget, and still exit 0.
        let out = run(false);
        assert!(out.status.success(), "exit 0");
        assert!(out.stdout.len() <= 8000, "{} bytes", out.stdout.len());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("## kanbanr — recovered project state (SessionStart)"));
        assert!(text.contains("FTR-000"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
