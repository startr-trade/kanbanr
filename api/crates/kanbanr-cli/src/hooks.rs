//! Claude Code hook registration (FEAT-044). kanbanr's skill ships two hooks: SessionStart (print
//! the board so Claude resumes from it) and Stop (nudge to record work). They are registered
//! **once per machine** in the user's global Claude Code settings; the scripts themselves only act
//! in folders kanbanr tracks, so nothing is written into project folders.
//!
//! The settings file belongs to the user: it is merged, never replaced — other keys and hooks keep
//! their order, the write is atomic, and a file that isn't valid JSON is left alone.

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// (Claude Code event, script base name).
const HOOKS: [(&str, &str); 2] = [("SessionStart", "session-start"), ("Stop", "stop-check")];

/// Hooks that run the CLI itself rather than a script in the skill folder: `(event, matcher,
/// command)`. Both watch Bash, because that is where test runs and commits happen — one reads the
/// output of a run to record which tracked tests actually passed (FEAT-053), the other checks a
/// `git commit` before it is attempted, so the feedback arrives with a suggestion rather than as a
/// failure after the fact (FEAT-056).
const CLI_HOOKS: [(&str, &str, &str); 3] = [
    ("PostToolUse", "Bash", "kanbanr capture"),
    ("PreToolUse", "Bash", "kanbanr git guard"),
    // Documentation belongs on the board (FEAT-040). The rule lived only in the skill's prose
    // until this hook; it decides from the path alone and stays silent where no board is active.
    ("PreToolUse", "Write|Edit", "kanbanr claude guard"),
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
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| kanbanr_core::project::home_dir().map(|h| h.join(".claude")))
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

fn is_kanbanr_hook(command: &str, name: &str) -> bool {
    command.contains("kanbanr") && command.contains(name)
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

/// The events whose kanbanr hook is not registered (a registration whose script no longer
/// exists counts as missing).
pub fn missing(settings: &Value) -> Vec<&'static str> {
    let mut missing: Vec<&'static str> = HOOKS
        .iter()
        .filter(|(event, name)| {
            !commands(settings, event).any(|c| is_kanbanr_hook(c, name) && script_of(c).is_file())
        })
        .map(|(event, _)| *event)
        .collect();
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
    for (event, name) in HOOKS {
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
    for (event, name) in HOOKS.iter().filter(|(e, _)| added.contains(e)) {
        let mut hook = json!({"type": "command", "command": script_command(scripts, name)});
        if *event == "SessionStart" {
            hook["statusMessage"] = json!("Recovering kanbanr board");
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

/// Write via a temp file + rename, so an interrupted write never leaves a broken settings file.
fn write(path: &Path, settings: &Value) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.kanbanr-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(settings)? + "\n")?;
    std::fs::rename(&tmp, path).with_context(|| format!("could not write {}", path.display()))
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
    if !HOOKS
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
}

/// What is registered in `settings_dir`, checked against the scripts in `scripts`.
pub fn status_in(settings_dir: &Path, scripts: &Path) -> anyhow::Result<Status> {
    let path = settings_path(settings_dir);
    let settings = read(&path)?;
    let scripts = scripts.to_path_buf();
    Ok(Status {
        settings: path.display().to_string(),
        plugin: plugin_enabled(&settings),
        skill_installed: HOOKS
            .iter()
            .all(|(_, name)| scripts.join(script_file(name)).is_file()),
        hooks: HOOKS
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
            for (_, name) in HOOKS {
                std::fs::write(scripts.join(script_file(name)), "#!/bin/sh\nexit 0\n").unwrap();
            }
        }
        dir
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

        assert_eq!(
            install(&dir).unwrap(),
            Installed::Added(vec!["SessionStart", "Stop", "PostToolUse", "PreToolUse"])
        );
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
            5,
            "two script hooks plus the three that run the CLI"
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
        assert_eq!(
            install(&dir).unwrap(),
            Installed::Added(vec!["SessionStart", "Stop", "PostToolUse", "PreToolUse"])
        );
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        let starts = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 1, "stale entry replaced, not kept: {v}");
        assert!(
            !starts[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .starts_with("/old/place")
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
}
