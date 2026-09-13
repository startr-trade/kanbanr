//! Claude Code hook registration (FEAT-044). kanbanr's skill ships two hooks: SessionStart (print
//! the board so Claude resumes from it) and Stop (nudge to record work). They are registered
//! **once per machine** in the user's global Claude Code settings; the scripts themselves only act
//! in folders kanbanr tracks, so nothing is written into project folders.
//!
//! The settings file belongs to the user: it is merged, never replaced — other keys and hooks keep
//! their order, the write is atomic, and a file that isn't valid JSON is left alone.

use anyhow::{anyhow, Context};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// (Claude Code event, script base name).
const HOOKS: [(&str, &str); 2] = [("SessionStart", "session-start"), ("Stop", "stop-check")];

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

/// Where `make install-skill` puts the hook scripts.
pub fn scripts_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("skills").join("kanbanr").join("hooks")
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
    HOOKS
        .iter()
        .filter(|(event, name)| {
            !commands(settings, event).any(|c| is_kanbanr_hook(c, name) && script_of(c).is_file())
        })
        .map(|(event, _)| *event)
        .collect()
}

/// Remove kanbanr's hook entries (all of them, or only those whose script is gone), dropping
/// groups and events left empty. Returns how many entries were removed.
fn remove(settings: &mut Value, only_stale: bool) -> usize {
    let mut removed = 0;
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
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
    let root = settings.as_object_mut().expect("checked above");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("\"hooks\" in settings.json is not an object"))?;
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

pub fn install(claude_dir: &Path) -> anyhow::Result<Installed> {
    let path = settings_path(claude_dir);
    let mut settings = read(&path)?;
    if plugin_enabled(&settings) {
        return Ok(Installed::ProvidedByPlugin);
    }
    if missing(&settings).is_empty() {
        return Ok(Installed::AlreadyPresent);
    }
    let scripts = scripts_dir(claude_dir);
    if !HOOKS
        .iter()
        .all(|(_, name)| scripts.join(script_file(name)).is_file())
    {
        return Ok(Installed::SkillMissing(scripts));
    }
    let added = add(&mut settings, &scripts)?;
    write(&path, &settings)?;
    Ok(Installed::Added(added))
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

pub fn status(claude_dir: &Path) -> anyhow::Result<Status> {
    let path = settings_path(claude_dir);
    let settings = read(&path)?;
    let scripts = scripts_dir(claude_dir);
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
            Installed::Added(vec!["SessionStart", "Stop"])
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
        assert!(v["hooks"]["Stop"][1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("stop-check"));
        assert!(v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("session-start"));

        // Idempotent.
        assert_eq!(install(&dir).unwrap(), Installed::AlreadyPresent);
        assert_eq!(std::fs::read_to_string(settings_path(&dir)).unwrap(), text);

        // Uninstall removes only kanbanr's entries.
        assert_eq!(uninstall(&dir).unwrap(), 2);
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
            Installed::Added(vec!["SessionStart", "Stop"])
        );
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        let starts = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 1, "stale entry replaced, not kept: {v}");
        assert!(!starts[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .starts_with("/old/place"));
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
