//! The Claude Code skill this binary carries, and putting it where Claude Code finds it (FEAT-141).
//!
//! kanbanr used to be two installs: the program from a release, the skill from a clone or the
//! plugin marketplace — a second step, tied to no version, so a skill could describe commands its
//! program lacked. The skill is now embedded at build time (`build.rs`), and `kanbanr skill install`
//! writes it to Claude Code's personal skills folder. The installers run it; `self-update` keeps it
//! in step.
//!
//! A folder kanbanr did not write is never touched: a link to a clone (how this project's own
//! maintainer works on the skill), or one a plugin manager owns. kanbanr knows its own by a stamp
//! file it writes beside the skill.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/skill_files.rs"));

/// The stamp kanbanr leaves in a skill folder it wrote: its version and the content's digest.
const STAMP: &str = ".kanbanr-skill";

/// What is installed where the skill belongs.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing there.
    Missing,
    /// kanbanr's own copy; `current` when it is exactly the skill this binary carries.
    Installed { version: String, current: bool },
    /// Something kanbanr did not write — a link to a clone, or another tool's copy. Left alone.
    Foreign { link: Option<PathBuf> },
}

/// Claude Code's personal skills folder for kanbanr: `<claude dir>/skills/kanbanr`.
pub fn target() -> Result<PathBuf> {
    let dir = crate::hooks::claude_dir()
        .context("no home directory to install the skill into: set CLAUDE_CONFIG_DIR")?;
    Ok(dir.join("skills").join("kanbanr"))
}

/// The digest of the embedded skill: what an installed copy is compared against.
fn digest() -> String {
    let mut h = Sha256::new();
    for (rel, bytes) in SKILL_FILES {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(bytes);
    }
    format!("{:x}", h.finalize())
}

fn stamp_text() -> String {
    format!(
        "version: {}\ndigest: {}\n",
        env!("CARGO_PKG_VERSION"),
        digest()
    )
}

/// What is at `dir`.
pub fn status_at(dir: &Path) -> Status {
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.file_type().is_symlink()
    {
        return Status::Foreign {
            link: std::fs::read_link(dir).ok(),
        };
    }
    if !dir.exists() {
        return Status::Missing;
    }
    let Ok(stamp) = std::fs::read_to_string(dir.join(STAMP)) else {
        return Status::Foreign { link: None };
    };
    let field = |name: &str| {
        stamp
            .lines()
            .find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()))
            .unwrap_or_default()
    };
    Status::Installed {
        version: field("version:"),
        current: field("digest:") == digest(),
    }
}

/// Write the embedded skill to `dir`, replacing a copy kanbanr wrote earlier. Refuses to touch a
/// folder kanbanr did not write. Returns what happened, for the caller to print.
pub fn install_at(dir: &Path) -> Result<String> {
    if SKILL_FILES.is_empty() {
        bail!("this binary carries no skill (it was built outside the kanbanr repository)");
    }
    match status_at(dir) {
        Status::Foreign { link: Some(to) } => {
            return Ok(format!(
                "left {} alone: it links to {} — a skill you manage yourself",
                dir.display(),
                to.display()
            ));
        }
        Status::Foreign { link: None } => {
            return Ok(format!(
                "left {} alone: kanbanr did not write it (another tool's copy?). Remove it to let \
                 kanbanr install its own",
                dir.display()
            ));
        }
        Status::Installed {
            current: true,
            version,
        } => {
            return Ok(format!(
                "skill {version} already installed at {}",
                dir.display()
            ));
        }
        Status::Missing | Status::Installed { current: false, .. } => {}
    }
    // Written beside the target and swapped in, so a half-written skill is never what Claude
    // Code loads.
    let parent = dir.parent().context("the skill folder has no parent")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(".kanbanr.installing");
    let _ = std::fs::remove_dir_all(&staging);
    for (rel, bytes) in SKILL_FILES {
        let path = staging.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, bytes)
            .with_context(|| format!("could not write {}", path.display()))?;
        #[cfg(unix)]
        if rel.ends_with(".sh") || rel.ends_with(".py") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    std::fs::write(staging.join(STAMP), stamp_text())?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::rename(&staging, dir)?;
    Ok(format!(
        "installed the kanbanr skill {} at {}",
        env!("CARGO_PKG_VERSION"),
        dir.display()
    ))
}

/// Remove kanbanr's own copy; leave anything else.
pub fn uninstall_at(dir: &Path) -> Result<String> {
    match status_at(dir) {
        Status::Missing => Ok(format!("no skill at {}", dir.display())),
        Status::Foreign { .. } => Ok(format!(
            "left {} alone: kanbanr did not install it",
            dir.display()
        )),
        Status::Installed { .. } => {
            std::fs::remove_dir_all(dir)?;
            Ok(format!("removed {}", dir.display()))
        }
    }
}

/// `kanbanr skill status`, in words.
pub fn describe(dir: &Path) -> String {
    match status_at(dir) {
        Status::Missing => format!(
            "no kanbanr skill at {} — `kanbanr skill install` adds the one this program carries",
            dir.display()
        ),
        Status::Installed {
            version,
            current: true,
        } => {
            format!("skill {version} at {}: matches this program", dir.display())
        }
        Status::Installed {
            version,
            current: false,
        } => format!(
            "skill {version} at {}: does NOT match this program ({}) — `kanbanr skill install` \
             updates it",
            dir.display(),
            env!("CARGO_PKG_VERSION")
        ),
        Status::Foreign { link: Some(to) } => format!(
            "{} links to {}: you manage it; kanbanr leaves it alone",
            dir.display(),
            to.display()
        ),
        Status::Foreign { link: None } => format!(
            "{} was not written by kanbanr (a plugin or another tool's copy); kanbanr leaves it alone",
            dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kanbanr-skill-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// FEAT-141 R-1: the skill written is the one embedded, stamped, and recognised as current; a
    /// stale copy kanbanr wrote is replaced.
    #[test]
    fn install_writes_the_embedded_skill_and_status_matches() {
        let root = temp("install");
        let dir = root.join("skills").join("kanbanr");
        assert_eq!(status_at(&dir), Status::Missing);
        install_at(&dir).unwrap();
        let skill = std::fs::read(dir.join("SKILL.md")).unwrap();
        let embedded = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .unwrap()
            .1;
        assert_eq!(
            skill, embedded,
            "the skill on disk is the one this binary carries"
        );
        assert!(dir.join("hooks").join("session-start.sh").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("hooks/session-start.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "hook scripts are executable");
        }
        assert!(matches!(
            status_at(&dir),
            Status::Installed { current: true, .. }
        ));

        // An older copy of ours is noticed and replaced.
        std::fs::write(dir.join("SKILL.md"), "an older skill").unwrap();
        std::fs::write(dir.join(STAMP), "version: 0.0.1\ndigest: stale\n").unwrap();
        assert!(matches!(
            status_at(&dir),
            Status::Installed { current: false, .. }
        ));
        install_at(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join("SKILL.md")).unwrap(), embedded);
        assert!(uninstall_at(&dir).unwrap().starts_with("removed"));
        assert_eq!(status_at(&dir), Status::Missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// FEAT-141 R-2: a skill kanbanr did not write — a link to a clone, or another tool's folder — is
    /// left exactly as it is, by install and by uninstall.
    #[cfg(unix)]
    #[test]
    fn a_linked_skill_is_left_alone() {
        let root = temp("linked");
        let clone = root.join("clone-skill");
        std::fs::create_dir_all(&clone).unwrap();
        std::fs::write(clone.join("SKILL.md"), "the maintainer's working copy").unwrap();
        let skills = root.join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        let dir = skills.join("kanbanr");
        std::os::unix::fs::symlink(&clone, &dir).unwrap();

        assert!(matches!(status_at(&dir), Status::Foreign { link: Some(_) }));
        assert!(install_at(&dir).unwrap().starts_with("left"));
        assert!(uninstall_at(&dir).unwrap().starts_with("left"));
        assert_eq!(
            std::fs::read_to_string(clone.join("SKILL.md")).unwrap(),
            "the maintainer's working copy"
        );
        assert!(
            std::fs::symlink_metadata(&dir)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        // Another tool's plain folder, with no stamp, is just as off-limits.
        let other = root.join("other").join("kanbanr");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("SKILL.md"), "someone else's").unwrap();
        assert!(install_at(&other).unwrap().starts_with("left"));
        assert_eq!(
            std::fs::read_to_string(other.join("SKILL.md")).unwrap(),
            "someone else's"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
