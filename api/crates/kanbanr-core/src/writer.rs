//! Which kanbanr wrote the board last (FTR-036).
//!
//! The monitor shows the build it runs and warns when the board was last written by a newer one,
//! so no decision is taken under rules older than the ones the work was built for. The board
//! keeps one small file, `writer.yaml`, at its root; a write stamps it before committing, so the
//! stamp lands in the write's own commit. The record only moves forward, a board without it
//! reads as "no writer recorded", and reading never rewrites it (ADR-0005).

use crate::models::Extra;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The file at the board's root.
pub const FILE: &str = "writer.yaml";

/// A kanbanr build: its release version and the commit it was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub commit: String,
}

/// The board's writer record: the newest kanbanr that wrote the board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Writer {
    pub version: String,
    pub commit: String,
    pub at: String,
    /// Whether the build was made from a modified tree (its `commit` ends in `-dirty`).
    #[serde(default)]
    pub dirty: bool,
    /// Fields a later kanbanr adds are kept on rewrite.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The commit a build names, without the `-dirty` mark of a build from a modified tree.
pub fn base_commit(commit: &str) -> &str {
    commit.strip_suffix("-dirty").unwrap_or(commit)
}

/// Whether a commit string names a build from a modified tree.
pub fn is_dirty(commit: &str) -> bool {
    commit.ends_with("-dirty")
}

static RUNNING: OnceLock<Build> = OnceLock::new();

/// Record the build this process is, once. A second call is ignored.
pub fn set_running(build: Build) {
    let _ = RUNNING.set(build);
}

/// The build this process is; the core crate's version with commit `unknown` when unset.
pub fn running() -> Build {
    RUNNING.get().cloned().unwrap_or_else(|| Build {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: "unknown".to_string(),
    })
}

/// `major.minor.patch`, cutting any `-pre` or `+build` tail; anything else is `None`.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().split(['-', '+']).next()?;
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether the recorded writer is newer than the running build: a higher version, or the same
/// version with a different commit that descends from the running one in the code repository.
/// An unparsable version never counts as newer.
pub fn newer(running: &Build, recorded: &Writer, descends: Option<bool>) -> bool {
    match (
        parse_version(&running.version),
        parse_version(&recorded.version),
    ) {
        (Some(r), Some(w)) if w > r => true,
        (Some(r), Some(w)) if w == r => {
            base_commit(&running.commit) != base_commit(&recorded.commit) && descends == Some(true)
        }
        _ => false,
    }
}

/// Whether `commit` is a descendant of `ancestor` in the repository at `repo`. `None` when the
/// repository or either commit cannot be found.
pub fn descends(repo: &Path, ancestor: &str, commit: &str) -> Option<bool> {
    let repo = git2::Repository::discover(repo).ok()?;
    let a = repo.revparse_single(ancestor).ok()?.peel_to_commit().ok()?;
    let c = repo.revparse_single(commit).ok()?.peel_to_commit().ok()?;
    repo.graph_descendant_of(c.id(), a.id()).ok()
}

/// The board's writer record; `None` when absent or unreadable.
pub fn recorded(data_dir: &Path) -> Option<Writer> {
    let text = std::fs::read_to_string(data_dir.join(FILE)).ok()?;
    crate::yaml::from_str(&text).ok()
}

/// Write the record when `build` moves it forward; true when it wrote.
pub fn stamp_as(data_dir: &Path, build: &Build, now: &str) -> bool {
    if base_commit(&build.commit) == "unknown" {
        return false;
    }
    if let Some(cur) = recorded(data_dir) {
        match (parse_version(&cur.version), parse_version(&build.version)) {
            (Some(c), Some(b)) if c > b => return false,
            (Some(c), Some(b))
                if c == b && base_commit(&cur.commit) == base_commit(&build.commit) =>
            {
                return false;
            }
            _ => {}
        }
    }
    let record = Writer {
        version: build.version.clone(),
        commit: build.commit.clone(),
        at: now.to_string(),
        dirty: is_dirty(&build.commit),
        extra: recorded(data_dir).map(|w| w.extra).unwrap_or_default(),
    };
    match crate::yaml::to_string(&record) {
        Ok(text) => crate::atomic::write(data_dir.join(FILE), text).is_ok(),
        Err(_) => false,
    }
}

/// Stamp with the running build and the current time. A failure is ignored: stamping never
/// fails a board write.
pub fn stamp(data_dir: &Path) -> bool {
    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    stamp_as(data_dir, &running(), &now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(v: &str, c: &str) -> Build {
        Build {
            version: v.into(),
            commit: c.into(),
        }
    }
    fn rec(v: &str, c: &str) -> Writer {
        Writer {
            version: v.into(),
            commit: c.into(),
            at: "2026-11-13T09:12:00Z".into(),
            dirty: false,
            extra: Extra::new(),
        }
    }
    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kanbanr-writer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn newer_compares_versions_then_commit_ancestry() {
        let run = build("0.5.0", "aaa");
        assert!(newer(&run, &rec("0.6.0", "bbb"), None));
        assert!(
            newer(&run, &rec("0.10.0", "bbb"), None),
            "not a string compare"
        );
        assert!(!newer(&run, &rec("0.4.9", "bbb"), Some(true)));
        assert!(!newer(&run, &rec("0.5.0", "aaa"), Some(true)));
        assert!(newer(&run, &rec("0.5.0", "bbb"), Some(true)));
        assert!(!newer(&run, &rec("0.5.0", "bbb"), Some(false)));
        assert!(!newer(&run, &rec("0.5.0", "bbb"), None));
        assert!(!newer(&run, &rec("banana", "bbb"), Some(true)));
        assert!(!newer(&build("x", "a"), &rec("9.0.0", "b"), None));
        assert_eq!(parse_version("1.2.3-rc.1+x"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn stamping_only_moves_the_record_forward() {
        let d = scratch("stamp");
        let t = "2026-11-13T09:12:00Z";
        assert!(!stamp_as(&d, &build("0.6.0", "unknown"), t), "rule 1");
        assert!(!d.join(FILE).exists());
        assert!(stamp_as(&d, &build("0.6.0", "aaa"), t), "rule 2");
        assert!(!stamp_as(&d, &build("0.5.0", "zzz"), t), "rule 3");
        assert_eq!(recorded(&d).unwrap().version, "0.6.0");
        assert!(stamp_as(&d, &build("0.7.0", "bbb"), t), "rule 4");
        assert!(stamp_as(&d, &build("0.7.0", "ccc"), t), "rule 5");
        assert_eq!(recorded(&d).unwrap().commit, "ccc");
        let bytes = std::fs::read(d.join(FILE)).unwrap();
        let mtime = std::fs::metadata(d.join(FILE)).unwrap().modified().unwrap();
        assert!(
            !stamp_as(&d, &build("0.7.0", "ccc"), "2027-01-01T00:00:00Z"),
            "rule 6"
        );
        assert_eq!(std::fs::read(d.join(FILE)).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(d.join(FILE)).unwrap().modified().unwrap(),
            mtime
        );
        // an unreadable record is replaced
        std::fs::write(d.join(FILE), ":::\n- [").unwrap();
        assert!(stamp_as(&d, &build("0.1.0", "ddd"), t));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_dirty_build_compares_as_its_base_commit() {
        assert_eq!(base_commit("abc123-dirty"), "abc123");
        assert_eq!(base_commit("abc123"), "abc123");
        let run = build("0.5.0", "aaa-dirty");
        // the same base commit is not newer, even where ancestry would say so
        assert!(!newer(&run, &rec("0.5.0", "aaa"), Some(true)));
        assert!(newer(&run, &rec("0.5.0", "bbb-dirty"), Some(true)));
        let d = scratch("dirty");
        let t = "2026-11-13T09:12:00Z";
        assert!(!stamp_as(&d, &build("0.6.0", "unknown-dirty"), t));
        assert!(stamp_as(&d, &build("0.6.0", "aaa"), t));
        assert!(!recorded(&d).unwrap().dirty);
        // a dirty rebuild of the same commit leaves the record alone
        assert!(!stamp_as(&d, &build("0.6.0", "aaa-dirty"), t));
        assert_eq!(recorded(&d).unwrap().commit, "aaa");
        // a dirty build of a new commit is kept as built, marked dirty
        assert!(stamp_as(&d, &build("0.6.0", "bbb-dirty"), t));
        let w = recorded(&d).unwrap();
        assert_eq!((w.commit.as_str(), w.dirty), ("bbb-dirty", true));
        assert!(!stamp_as(&d, &build("0.6.0", "bbb"), t));
        // a record written before `dirty` existed reads as clean
        std::fs::write(d.join(FILE), "version: 0.6.0\ncommit: ccc\nat: x\n").unwrap();
        assert!(!recorded(&d).unwrap().dirty);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn writer_yaml_round_trips_and_keeps_unknown_fields() {
        let d = scratch("yaml");
        std::fs::write(
            d.join(FILE),
            "version: 0.6.0\ncommit: 1138833\nat: 2026-11-13T09:12:00Z\nfuture: kept\n",
        )
        .unwrap();
        let w = recorded(&d).unwrap();
        assert_eq!(w.version, "0.6.0");
        assert_eq!(w.extra.get("future").and_then(|v| v.as_str()), Some("kept"));
        assert!(stamp_as(&d, &build("0.7.0", "abc"), "2026-11-14T00:00:00Z"));
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(text.contains("future"), "{text}");
        assert_eq!(recorded(&d).unwrap().version, "0.7.0");
        std::fs::write(d.join(FILE), "not: [valid").unwrap();
        assert!(recorded(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn descends_reads_the_code_repository() {
        let d = scratch("descends");
        let repo = git2::Repository::init(&d).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let first = repo
            .commit(Some("HEAD"), &sig, &sig, "one", &tree, &[])
            .unwrap();
        let fc = repo.find_commit(first).unwrap();
        let second = repo
            .commit(Some("HEAD"), &sig, &sig, "two", &tree, &[&fc])
            .unwrap();
        let (a, b) = (first.to_string(), second.to_string());
        assert_eq!(descends(&d, &a, &b), Some(true));
        assert_eq!(descends(&d, &b, &a), Some(false));
        assert_eq!(
            descends(&d, &a, "0123456789abcdef0123456789abcdef01234567"),
            None
        );
        assert_eq!(descends(&d.join("nope"), &a, &b), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
