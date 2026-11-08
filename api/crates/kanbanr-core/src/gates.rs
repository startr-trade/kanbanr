//! Gate results (FTR-002): a name (`ci`, `local`, `security`) and a verdict recorded against one
//! commit. kanbanr runs no gate and reads no result from any server: a person looks at their CI
//! and records the verdict, or the project's own `make ci` records its local run.
//!
//! Each result is a file of its own, `gates/<name>/<YYYYMMDDTHHMMSSZ>-<sha7>.yaml` in the project
//! folder, and is never rewritten: the raw record is kept, and the newest result for a commit
//! decides. `cadence.commit_gates` names the gates a release stage needing `gate_passed` requires.

use crate::error::{CoreError, Result};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The two places a result can come from.
pub const SOURCES: [&str; 2] = ["person", "local"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GateResult {
    pub name: String,
    /// The full commit the result is about.
    pub commit: String,
    pub passed: bool,
    pub by: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// `person` or `local`.
    pub source: String,
    /// A link to the run, `http(s)` only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

fn unsupported(msg: String) -> CoreError {
    CoreError::Unsupported(msg)
}

/// A gate name is `[a-z0-9][a-z0-9_-]*`.
pub fn check_name(name: &str) -> std::result::Result<(), String> {
    let mut chars = name.chars();
    let ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "'{name}' is not a gate name: use lowercase letters, digits, '-' and '_', starting \
             with a letter or digit"
        ))
    }
}

fn check(r: &GateResult) -> Result<()> {
    check_name(&r.name).map_err(unsupported)?;
    if r.commit.len() != 40 || !r.commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(unsupported(format!(
            "'{}' is not a full commit: a gate result names the 40-digit hash",
            r.commit
        )));
    }
    if !SOURCES.contains(&r.source.as_str()) {
        return Err(unsupported(format!(
            "'{}' is not a gate source: it is one of {}",
            r.source,
            SOURCES.join(", ")
        )));
    }
    if !r.url.is_empty() && !r.url.starts_with("http://") && !r.url.starts_with("https://") {
        return Err(unsupported(format!(
            "the link to the run must start with http:// or https://: '{}'",
            r.url
        )));
    }
    Ok(())
}

fn dir(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join("gates")
}

/// Keep a result as a new file beside the earlier ones. Written to a temporary file in the same
/// folder, then renamed, so a crash never leaves half a file.
pub fn record(store: &Store, id: &str, r: &GateResult) -> Result<PathBuf> {
    check(r)?;
    let folder = dir(store, id).join(&r.name);
    std::fs::create_dir_all(&folder)?;
    let stamp: String =
        r.at.chars()
            .filter(|c| c.is_ascii_digit())
            .take(14)
            .collect();
    let stamp = if stamp.len() == 14 {
        format!("{}T{}Z", &stamp[..8], &stamp[8..])
    } else {
        return Err(unsupported(format!("'{}' is not a time", r.at)));
    };
    let base = format!("{stamp}-{}", &r.commit[..7]);
    // Two results in one second for one commit keep both.
    let mut path = folder.join(format!("{base}.yaml"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{base}-{n}.yaml"));
        n += 1;
    }
    let tmp = folder.join(format!(".{base}.tmp"));
    std::fs::write(&tmp, crate::yaml::to_string(r)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every result, oldest first, optionally of one gate. A file that cannot be read is skipped.
pub fn results(store: &Store, id: &str, name: Option<&str>) -> Result<Vec<GateResult>> {
    Ok(read_all(store, id, name)?.0)
}

/// The results and the names of the files that could not be read.
pub fn read_all(
    store: &Store,
    id: &str,
    name: Option<&str>,
) -> Result<(Vec<GateResult>, Vec<String>)> {
    let mut found: Vec<(PathBuf, GateResult)> = Vec::new();
    let mut bad = Vec::new();
    let root = dir(store, id);
    let Ok(gates) = std::fs::read_dir(&root) else {
        return Ok((Vec::new(), bad));
    };
    for g in gates.flatten() {
        if name.is_some_and(|n| g.file_name().to_string_lossy() != n) || !g.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(g.path()) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            match std::fs::read_to_string(&p)
                .ok()
                .and_then(|s| crate::yaml::from_str::<GateResult>(&s).ok())
            {
                Some(r) => found.push((p, r)),
                None => bad.push(p.display().to_string()),
            }
        }
    }
    found.sort_by(|(pa, a), (pb, b)| crate::cmp_instants(&a.at, &b.at).then_with(|| pa.cmp(pb)));
    Ok((found.into_iter().map(|(_, r)| r).collect(), bad))
}

/// The newest result of `name` for exactly `commit`. A result for another commit says nothing.
pub fn newest<'a>(results: &'a [GateResult], name: &str, commit: &str) -> Option<&'a GateResult> {
    results
        .iter()
        .rev()
        .find(|r| r.name == name && r.commit == commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn scratch(name: &str) -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-gates-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects").join("demo")).unwrap();
        (Store::new(&dir), dir)
    }

    pub fn result(name: &str, commit: &str, passed: bool, at: &str) -> GateResult {
        GateResult {
            name: name.into(),
            commit: commit.into(),
            passed,
            by: "Ada L".into(),
            at: at.into(),
            source: "person".into(),
            ..GateResult::default()
        }
    }

    #[test]
    fn a_result_is_kept_beside_earlier_ones() {
        let (store, dir) = scratch("kept");
        let first = record(
            &store,
            "demo",
            &result("ci", A, false, "2026-11-04T10:00:00Z"),
        )
        .unwrap();
        let before = std::fs::read_to_string(&first).unwrap();
        let second = record(
            &store,
            "demo",
            &result("ci", A, true, "2026-11-04T11:00:00Z"),
        )
        .unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), before);
        let all = results(&store, "demo", Some("ci")).unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all[0].passed && all[1].passed);
        // the same second for the same commit keeps both files
        let third = record(
            &store,
            "demo",
            &result("ci", A, true, "2026-11-04T11:00:00Z"),
        )
        .unwrap();
        assert_ne!(second, third);
        // bad input names the rule
        let mut bad = result("CI", A, true, "2026-11-04T11:00:00Z");
        assert!(record(&store, "demo", &bad).is_err());
        bad = result("ci", "abc", true, "2026-11-04T11:00:00Z");
        assert!(record(&store, "demo", &bad).is_err());
        bad = result("ci", A, true, "2026-11-04T11:00:00Z");
        bad.url = "ftp://x".into();
        assert!(record(&store, "demo", &bad).is_err());
        bad.url.clear();
        bad.source = "server".into();
        assert!(record(&store, "demo", &bad).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn newest_result_for_the_commit_decides() {
        let rs = vec![
            result("ci", A, false, "2026-11-04T10:00:00Z"),
            result("ci", A, true, "2026-11-04T11:00:00Z"),
            result("ci", B, false, "2026-11-04T12:00:00Z"),
            result("local", A, false, "2026-11-04T13:00:00Z"),
        ];
        assert!(newest(&rs, "ci", A).unwrap().passed);
        assert!(!newest(&rs, "ci", B).unwrap().passed);
        assert!(!newest(&rs, "local", A).unwrap().passed);
        assert!(newest(&rs, "security", A).is_none());
        assert!(newest(&rs, "ci", &"c".repeat(40)).is_none());
    }
}
