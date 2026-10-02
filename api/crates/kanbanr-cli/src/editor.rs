//! The VS Code extension, installed into the editors on this machine (FEAT-158).
//!
//! Every release carries `kanbanr-vscode-<version>.vsix` (FEAT-152), listed in its `SHA256SUMS`.
//! VS Code cannot see Open VSX, so for VS Code that file is the only way in, and fetching it by hand
//! was a second install step. The installers now ask the program to do it — `kanbanr editor install`
//! — and `self-update` keeps an installed copy in step, as both already do for the skill.
//!
//! Done here, once, rather than in `install.sh`, `install.ps1` and `self-update` separately: the
//! download and its https-only, checksum-checked handling already live in the program.

use crate::self_update;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The extension's id in every editor's extension list.
pub const EXTENSION_ID: &str = "kanbanr.kanbanr";

/// The editors that take a `.vsix` with `--install-extension`, by their command names.
pub const EDITORS: [&str; 4] = ["code", "codium", "cursor", "windsurf"];

/// The release file for this program's version.
pub fn vsix_name(version: &str) -> String {
    format!("kanbanr-vscode-{}.vsix", version.trim_start_matches('v'))
}

/// Where `name` resolves on PATH, if anywhere. On Windows the editors are `.cmd` shims.
pub fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) {
        &[".cmd", ".exe", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .find(|p| p.is_file())
    })
}

/// Run an editor's CLI. A `.cmd` shim has to go through `cmd /C` — `CreateProcess` will not run one.
fn editor_command(editor: &Path) -> Command {
    let is_shim = editor
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if is_shim {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(editor);
        c
    } else {
        Command::new(editor)
    }
}

/// Whether `editor` already lists the kanbanr extension.
pub fn has_extension(editor: &Path) -> bool {
    editor_command(editor)
        .arg("--list-extensions")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim().eq_ignore_ascii_case(EXTENSION_ID))
        })
        .unwrap_or(false)
}

/// The editors to install into: the ones named (each must be on PATH), or every known one found.
pub fn choose(named: &[String]) -> Result<Vec<(String, PathBuf)>> {
    if named.is_empty() {
        return Ok(EDITORS
            .iter()
            .filter_map(|e| on_path(e).map(|p| (e.to_string(), p)))
            .collect());
    }
    named
        .iter()
        .map(|e| {
            on_path(e)
                .map(|p| (e.clone(), p))
                .with_context(|| format!("no `{e}` command on PATH"))
        })
        .collect()
}

/// Download this release's `.vsix`, check it against `SHA256SUMS`, and install it into `editors`.
/// `fetch` is the program's https-only downloader; tests pass their own. Returns one line per
/// editor, for the caller to print.
pub fn install_into(
    editors: &[(String, PathBuf)],
    version: &str,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Vec<String>> {
    if editors.is_empty() {
        return Ok(vec![format!(
            "VS Code extension: no editor found on PATH among {}",
            EDITORS.join(", ")
        )]);
    }
    let tag = format!("v{}", version.trim_start_matches('v'));
    let file = vsix_name(version);
    let dl = format!(
        "https://github.com/{}/releases/download/{tag}",
        self_update::REPO
    );
    let sums = String::from_utf8(fetch(&format!("{dl}/SHA256SUMS"))?)
        .context("SHA256SUMS was not valid UTF-8")?;
    let Some(want) = self_update::sum_for(&sums, &file) else {
        bail!("{tag} carries no {file} (releases before v0.1.2 have no VS Code extension)");
    };
    let bytes = fetch(&format!("{dl}/{file}"))?;
    self_update::verify(&file, &bytes, &want)?;

    let dir = std::env::temp_dir().join(format!("kanbanr-vsix-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&file);
    std::fs::write(&path, &bytes)?;
    let mut said = Vec::new();
    for (name, editor) in editors {
        let out = editor_command(editor)
            .arg("--install-extension")
            .arg(&path)
            .arg("--force")
            .output();
        said.push(match out {
            Ok(o) if o.status.success() => {
                format!("VS Code extension {} installed into {name}", tag)
            }
            Ok(o) => format!(
                "VS Code extension: {name} refused it: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => format!("VS Code extension: could not run {name}: {e}"),
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(said)
}

/// `kanbanr editor install`: into the named editors, or every known one on PATH. With
/// `if_installed`, only into editors that already have the extension — what `self-update` runs.
pub fn install(named: &[String], if_installed: bool) -> Result<Vec<String>> {
    install_with(
        choose(named)?,
        if_installed,
        env!("CARGO_PKG_VERSION"),
        &self_update::fetch,
    )
}

/// [`install`], given the editors and the downloader.
pub fn install_with(
    mut editors: Vec<(String, PathBuf)>,
    if_installed: bool,
    version: &str,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Vec<String>> {
    if if_installed {
        editors.retain(|(_, p)| has_extension(p));
        if editors.is_empty() {
            return Ok(Vec::new());
        }
    }
    install_into(&editors, version, fetch)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in editor: records its arguments, and lists the extension when `has` is set.
    fn stub_editor(dir: &Path, name: &str, has: bool) -> PathBuf {
        let path = dir.join(name);
        let log = dir.join(format!("{name}.log"));
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$1\" = --list-extensions ] && {} \nexit 0\n",
                log.display(),
                if has { "echo kanbanr.kanbanr" } else { "true" }
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn release(vsix: &[u8]) -> impl Fn(&str) -> Result<Vec<u8>> + '_ {
        move |url: &str| {
            if url.ends_with("/SHA256SUMS") {
                Ok(format!(
                    "{}  {}\n",
                    self_update::sha256_bytes(vsix),
                    vsix_name("0.1.4")
                )
                .into_bytes())
            } else if url.ends_with(&vsix_name("0.1.4")) {
                Ok(vsix.to_vec())
            } else {
                bail!("unexpected {url}")
            }
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kanbanr-editor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// FEAT-158 R-1: the checked `.vsix` goes into each editor given, and a file that does not match
    /// `SHA256SUMS` goes into none.
    #[test]
    fn the_checked_vsix_is_installed_into_each_editor() {
        let dir = temp("install");
        let code = stub_editor(&dir, "code", false);
        let codium = stub_editor(&dir, "codium", false);
        let editors = vec![("code".to_string(), code), ("codium".to_string(), codium)];
        let said = install_into(&editors, "0.1.4", &release(b"the extension")).unwrap();
        assert_eq!(said.len(), 2, "{said:?}");
        for name in ["code", "codium"] {
            let log = std::fs::read_to_string(dir.join(format!("{name}.log"))).unwrap();
            assert!(
                log.contains("--install-extension") && log.contains("kanbanr-vscode-0.1.4.vsix"),
                "{name}: {log}"
            );
        }

        let tampered = |url: &str| -> Result<Vec<u8>> {
            if url.ends_with("/SHA256SUMS") {
                release(b"the extension")(url)
            } else {
                Ok(b"something else".to_vec())
            }
        };
        let only = vec![("cursor".to_string(), stub_editor(&dir, "cursor", false))];
        assert!(install_into(&only, "0.1.4", &tampered).is_err());
        assert!(
            !dir.join("cursor.log").exists(),
            "nothing reached the editor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-158 R-3: `self-update` updates the extension only where it is already installed.
    #[test]
    fn self_update_installs_only_where_the_extension_already_is() {
        let dir = temp("follow");
        let with = stub_editor(&dir, "code", true);
        let without = stub_editor(&dir, "codium", false);
        let editors = vec![("code".to_string(), with), ("codium".to_string(), without)];
        let said = install_with(editors, true, "0.1.4", &release(b"x")).unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        let code = std::fs::read_to_string(dir.join("code.log")).unwrap();
        let codium = std::fs::read_to_string(dir.join("codium.log")).unwrap();
        assert!(code.contains("--install-extension"), "{code}");
        assert!(!codium.contains("--install-extension"), "{codium}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
