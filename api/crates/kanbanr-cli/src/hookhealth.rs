//! Hook health (FTR-059): does each kanbanr hook still run, beside whatever other tools registered
//! on the same points? Another tool can write its own git hook over kanbanr's, a hook manager can
//! point `core.hooksPath` somewhere kanbanr's hooks are not, and another installer can rewrite
//! Claude Code's `settings.json` without kanbanr's entries. Each would leave kanbanr's rules
//! silently unguarded; these checks name it, with the fix that keeps the other tool's hook too.
//!
//! Every check is read-only and best effort: a git or file error yields no finding rather than a
//! false one, except a missing hook file, which is the finding.

use crate::{hooks, scm};
use kanbanr_core::coderepo::{self, git};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

/// One thing wrong with kanbanr's hooks, and the command that puts it right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub what: String,
    pub fix: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what, self.fix)
    }
}

/// The git hooks of the repository at `root` that kanbanr installs and that do not run: absent,
/// replaced by another tool's, or bypassed by a `core.hooksPath` that points where none of
/// kanbanr's hooks are. Empty when `root` is not a repository.
pub fn git_hook_findings(root: &Path) -> Vec<Finding> {
    if coderepo::repo_root_in(root).is_none() {
        return Vec::new();
    }
    let Ok(dir) = scm::hooks_dir(root) else {
        return Vec::new();
    };
    let shown = dir.display();
    let redirected = git(root, &["config", "--get", "core.hooksPath"])
        .ok()
        .is_some_and(|p| !p.is_empty());
    if redirected
        && !scm::HOOK_NAMES
            .iter()
            .any(|name| scm::is_ours(&dir.join(name)))
    {
        return vec![Finding {
            what: format!(
                "core.hooksPath points at {shown} (another hook manager's?), where kanbanr's \
                 hooks are not"
            ),
            fix: "run `kanbanr git install-hooks --force`: it keeps each hook already there as \
                  <name>.pre-kanbanr and runs it first, or call `kanbanr git check-msg \"$1\"`, \
                  `kanbanr git check-branch` and `kanbanr git check-push` from that manager's own \
                  hooks"
                .to_string(),
        }];
    }
    let mut findings = Vec::new();
    for name in scm::HOOK_NAMES {
        let path = dir.join(name);
        if !path.exists() {
            findings.push(Finding {
                what: format!("no kanbanr {name} hook in {shown}"),
                fix: "run `kanbanr git install-hooks`".to_string(),
            });
        } else if !scm::is_ours(&path) {
            findings.push(Finding {
                what: format!(
                    "{} is another tool's hook, so kanbanr's {name} check does not run",
                    path.display()
                ),
                fix: format!(
                    "run `kanbanr git install-hooks --force`: it keeps that hook as \
                     {name}.pre-kanbanr and runs it first"
                ),
            });
        }
    }
    findings
}

/// The Claude Code events kanbanr registers that no settings file of `project_root` (the
/// project's two, then the user's) registers. Empty when any of them enables the kanbanr plugin,
/// which brings its own hooks.
pub fn claude_hook_findings(project_root: &Path) -> Vec<Finding> {
    claude_findings_in(&kanbanr_core::claude::settings_files(project_root))
}

/// [`claude_hook_findings`] over the given settings files, so a test needs no environment.
fn claude_findings_in(files: &[PathBuf]) -> Vec<Finding> {
    let enabled = files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .is_some_and(|s| hooks::plugin_enabled(&s))
    });
    if enabled {
        return Vec::new();
    }
    hooks::missing_across(files)
        .into_iter()
        .map(|event| Finding {
            what: format!(
                "kanbanr's Claude Code {event} hook is not registered in any settings file"
            ),
            fix: "run `kanbanr hooks install`".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-hookhealth-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scratch_repo(name: &str) -> PathBuf {
        let dir = scratch(name);
        git(&dir, &["init", "--initial-branch=main"]).unwrap();
        dir
    }

    #[test]
    fn a_replaced_missing_or_redirected_git_hook_is_named_with_its_fix() {
        let repo = scratch_repo("git");
        let dir = scm::hooks_dir(&repo).unwrap();

        // Installed: nothing to say.
        scm::install_hooks(&repo, false).unwrap();
        assert_eq!(git_hook_findings(&repo), Vec::<Finding>::new());

        // Another tool writes its commit-msg over kanbanr's.
        std::fs::write(dir.join("commit-msg"), "#!/bin/sh\necho theirs\n").unwrap();
        let found = git_hook_findings(&repo);
        assert_eq!(found.len(), 1, "{found:?}");
        let line = found[0].to_string();
        assert!(
            line.contains("commit-msg") && line.contains("another tool's"),
            "{line}"
        );
        assert!(
            line.contains("--force") && line.contains("commit-msg.pre-kanbanr"),
            "{line}"
        );

        // Put it right, keeping theirs; then the pre-push hook goes missing.
        scm::install_hooks(&repo, true).unwrap();
        assert!(git_hook_findings(&repo).is_empty());
        std::fs::remove_file(dir.join("pre-push")).unwrap();
        let found = git_hook_findings(&repo);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].what.contains("no kanbanr pre-push hook"),
            "{:?}",
            found[0]
        );
        assert!(
            found[0].fix.contains("kanbanr git install-hooks"),
            "{:?}",
            found[0]
        );
        assert!(!found[0].fix.contains("--force"), "{:?}", found[0]);

        // A hook manager takes over core.hooksPath: one finding, not one per hook.
        let husky = repo.join(".husky");
        std::fs::create_dir_all(&husky).unwrap();
        std::fs::write(
            husky.join("commit-msg"),
            "#!/bin/sh\nnpx commitlint --edit \"$1\"\n",
        )
        .unwrap();
        git(&repo, &["config", "core.hooksPath", ".husky"]).unwrap();
        let found = git_hook_findings(&repo);
        assert_eq!(found.len(), 1, "{found:?}");
        let line = found[0].to_string();
        assert!(
            line.contains("core.hooksPath") && line.contains(".husky"),
            "{line}"
        );
        assert!(line.contains("install-hooks --force"), "{line}");
        for call in [
            "`kanbanr git check-msg \"$1\"`",
            "`kanbanr git check-branch`",
            "`kanbanr git check-push`",
        ] {
            assert!(line.contains(call), "{line}");
        }

        // Not a repository: nothing.
        let plain = scratch("plain");
        assert!(git_hook_findings(&plain).is_empty());
        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[test]
    fn kanbanr_claude_hooks_dropped_from_settings_are_named() {
        let root = scratch("claude");
        let project = root.join("project");
        let user = root.join("user");
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let files = vec![
            project.join(".claude").join("settings.local.json"),
            project.join(".claude").join("settings.json"),
            user.join("settings.json"),
        ];

        // No settings at all: every event kanbanr registers is named.
        let found = claude_findings_in(&files);
        let all = hooks::missing(&json!({}));
        assert_eq!(found.len(), all.len(), "{found:?}");
        for event in ["SessionStart", "Stop", "PostToolUse", "PreToolUse"] {
            assert!(
                found
                    .iter()
                    .any(|f| f.what.contains(&format!(" {event} hook "))),
                "{event}: {found:?}"
            );
        }
        assert!(found.iter().all(|f| f.fix == "run `kanbanr hooks install`"));

        // kanbanr's entries in the project's settings.local.json alone: nothing missing.
        let scripts = hooks::scripts_dir(&user);
        std::fs::create_dir_all(&scripts).unwrap();
        for name in ["session-start", "stop-check", "session-summary"] {
            for ext in ["sh", "ps1"] {
                std::fs::write(scripts.join(format!("{name}.{ext}")), "").unwrap();
            }
        }
        let mut local = json!({});
        hooks::add(&mut local, &scripts).unwrap();
        std::fs::write(&files[0], serde_json::to_string_pretty(&local).unwrap()).unwrap();
        assert_eq!(claude_findings_in(&files), Vec::<Finding>::new());

        // Only another tool's entries, but the plugin enabled in the user's file: nothing.
        std::fs::write(
            &files[0],
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"katalogr session-start"}]}]}}"#,
        )
        .unwrap();
        assert!(!claude_findings_in(&files).is_empty());
        std::fs::write(&files[2], r#"{"enabledPlugins":{"kanbanr@kanbanr":true}}"#).unwrap();
        assert!(claude_findings_in(&files).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
