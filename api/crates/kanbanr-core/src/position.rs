//! A repository's position: a commit plus a hash of what is uncommitted (SPK-001; chain decisions
//! CD-2 and CD-3 of katalogr FEAT-043, shared byte for byte with katalogr), and files read at a
//! position with nothing checked out.
//!
//! A spike's code: nothing here is reachable from the CLI, the daemon or the monitor yet. FTR-003
//! takes it over.

use crate::coderepo::{git_raw, rev_parse};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One repository's position (CD-2). Serialises as `{"repo","commit","dirty"}`, absent parts null.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PositionRef {
    pub repo: String,
    /// 40 lower-case hex, or None for a directory with no commit.
    pub commit: Option<String>,
    /// 64 lower-case hex, or None when the checkout is clean.
    pub dirty: Option<String>,
}

/// A position text that does not parse. Callers report it as a usage error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionError(pub String);

impl std::fmt::Display for PositionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad position: {}", self.0)
    }
}

impl std::error::Error for PositionError {}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl PositionRef {
    /// `<commit>`, `<commit>+<dirty>` or `none+<dirty>`. A position with neither part (which
    /// `parse` never yields) is `none`.
    pub fn text(&self) -> String {
        match (&self.commit, &self.dirty) {
            (Some(c), None) => c.clone(),
            (Some(c), Some(d)) => format!("{c}+{d}"),
            (None, Some(d)) => format!("none+{d}"),
            (None, None) => "none".to_string(),
        }
    }

    /// The text form for `repo`; 40 and 64 lower-case hex only, `none` needs a dirty part.
    pub fn parse(repo: &str, text: &str) -> std::result::Result<PositionRef, PositionError> {
        let bad = |why: &str| PositionError(format!("{why}: {text:?}"));
        let (head, dirty) = match text.split_once('+') {
            Some((h, d)) => (h, Some(d)),
            None => (text, None),
        };
        let commit = if head == "none" {
            if dirty.is_none() {
                return Err(bad("`none` needs a `+<dirty>` part"));
            }
            None
        } else if is_hex(head, 40) {
            Some(head.to_string())
        } else {
            return Err(bad("commit must be 40 lower-case hex digits or `none`"));
        };
        let dirty = match dirty {
            Some(d) if is_hex(d, 64) => Some(d.to_string()),
            Some(_) => return Err(bad("dirty part must be 64 lower-case hex digits")),
            None => None,
        };
        Ok(PositionRef {
            repo: repo.to_string(),
            commit,
            dirty,
        })
    }

    /// A clean position at `commit` (dirty None). FTR-004 resolves every pick but the working
    /// tree to one.
    pub fn at_commit(repo: &str, commit: &str) -> PositionRef {
        PositionRef {
            repo: repo.to_string(),
            commit: Some(commit.to_string()),
            dirty: None,
        }
    }

    /// `dirty.is_none()`.
    pub fn is_clean(&self) -> bool {
        self.dirty.is_none()
    }

    /// 7 characters of the commit, then `+` and 4 of the dirty hash when present (`none+83eb`
    /// with no commit).
    pub fn short(&self) -> String {
        let head: String = self
            .commit
            .as_deref()
            .map_or("none".to_string(), |c| c.chars().take(7).collect());
        match &self.dirty {
            Some(d) => format!("{head}+{}", d.chars().take(4).collect::<String>()),
            None => head,
        }
    }
}

/// CD-3's shape: `v1-` followed by 32 lower-case hex digits. FTR-016 checks katalogr's IDs with it.
pub fn is_view_id(s: &str) -> bool {
    s.strip_prefix("v1-").is_some_and(|h| is_hex(h, 32))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The git blob id of raw bytes (SHA-1 of `blob <len>\0` + bytes), 40 lower-case hex.
pub fn blob_id(bytes: &[u8]) -> String {
    git2::Oid::hash_object(git2::ObjectType::Blob, bytes)
        .map(|o| o.to_string())
        .unwrap_or_default()
}

/// CD-2's hash of (path, blob id or None) pairs: 64 lower-case hex, None for an empty set. Each
/// line is `<path>\0<blob id or ->\n`; the lines are sorted as bytes.
pub fn dirty_hash<'a>(
    entries: impl IntoIterator<Item = (&'a [u8], Option<&'a str>)>,
) -> Option<String> {
    let mut lines: Vec<Vec<u8>> = entries
        .into_iter()
        .map(|(path, id)| {
            let mut l = path.to_vec();
            l.push(0);
            l.extend_from_slice(id.unwrap_or("-").as_bytes());
            l.push(b'\n');
            l
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    lines.sort();
    let mut h = Sha256::new();
    for l in &lines {
        h.update(l);
    }
    Some(hex(&h.finalize()))
}

/// A question, for its ID: only what changes the answer, never output flags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewQuery {
    pub op: String,
    pub args: BTreeMap<String, String>,
}

/// The format version in every view ID; a change of the ID's input bumps it.
pub const VIEW_FORMAT: u32 = 1;

/// CD-3: `v1-` plus the first 32 hex of the SHA-256 of the compact, key-sorted JSON
/// `{"facts","positions","query":{"args","op"},"view_format":1}`. Pure and total; `positions`
/// must be sorted by repo.
pub fn view_id(positions: &[PositionRef], facts: &str, q: &ViewQuery) -> String {
    let body = serde_json::json!({
        "facts": facts,
        "positions": positions,
        "query": {"args": &q.args, "op": &q.op},
        "view_format": VIEW_FORMAT,
    });
    let text = crate::hash::canonical_json(&body);
    let digest = Sha256::digest(text.as_bytes());
    format!("v1-{}", &hex(&digest)[..32])
}

/// A checkout's position and the paths that make its dirty part, sorted by path; the blob id is
/// None for a deleted path. Paths are the repo-relative, '/'-separated bytes git prints, lossy
/// UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingState {
    pub position: PositionRef,
    pub changed: Vec<(String, Option<String>)>,
}

#[cfg(unix)]
fn path_of(root: &Path, path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    root.join(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
fn path_of(root: &Path, path: &[u8]) -> PathBuf {
    root.join(String::from_utf8_lossy(path).as_ref())
}

/// What git would hash for `path` with no filter applied: a symbolic link's target, a regular
/// file's bytes; None for anything else (missing, a directory).
fn raw_bytes(root: &Path, path: &[u8]) -> Option<Vec<u8>> {
    let full = path_of(root, path);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(&full).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            return Some(target.as_os_str().as_bytes().to_vec());
        }
        #[cfg(not(unix))]
        return Some(target.to_string_lossy().replace('\\', "/").into_bytes());
    }
    if meta.is_file() {
        return std::fs::read(&full).ok();
    }
    None
}

/// The blobs of `ls-tree -r -z <commit> [-- prefix]`: path bytes to blob id; gitlinks skipped.
fn head_blobs(root: &Path, commit: &str, prefix: &str) -> Result<BTreeMap<Vec<u8>, String>> {
    let mut args = vec!["ls-tree", "-r", "-z", commit];
    if !prefix.is_empty() {
        args.push("--");
        args.push(prefix);
    }
    let out = git_raw(root, &args)?;
    let mut map = BTreeMap::new();
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = rec.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let meta = String::from_utf8_lossy(&rec[..tab]);
        let mut parts = meta.split(' ');
        let (_mode, kind, oid) = (parts.next(), parts.next(), parts.next());
        if kind == Some("blob")
            && let Some(oid) = oid
        {
            map.insert(rec[tab + 1..].to_vec(), oid.to_string());
        }
    }
    Ok(map)
}

fn nul_list(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// HEAD plus the CD-2 hash of the checkout at `root`, which must be the top of a git work tree.
/// An unborn HEAD gives commit None with every untracked, not ignored file counted. Reads only;
/// no cache (katalogr's stat cache is not copied: see the SPK-001 result).
pub fn read_position(root: &Path, repo: &str) -> Result<WorkingState> {
    let commit = rev_parse(root, "HEAD").ok();
    let head = match &commit {
        Some(c) => head_blobs(root, c, "")?,
        None => BTreeMap::new(),
    };
    let mut lines: BTreeMap<Vec<u8>, Option<String>> = BTreeMap::new();
    for (path, oid) in &head {
        match raw_bytes(root, path) {
            Some(bytes) => {
                let id = blob_id(&bytes);
                if &id != oid {
                    lines.insert(path.clone(), Some(id));
                }
            }
            None => {
                lines.insert(path.clone(), None);
            }
        }
    }
    let others = nul_list(git_raw(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    let staged_new = match &commit {
        Some(c) => nul_list(git_raw(
            root,
            &[
                "diff-index",
                "--cached",
                "--diff-filter=A",
                "--name-only",
                "-z",
                c,
            ],
        )?),
        None => nul_list(git_raw(root, &["ls-files", "--cached", "-z"])?),
    };
    for path in others.into_iter().chain(staged_new) {
        if head.contains_key(&path) || lines.contains_key(&path) {
            continue;
        }
        if let Some(bytes) = raw_bytes(root, &path) {
            lines.insert(path, Some(blob_id(&bytes)));
        }
    }
    let dirty = dirty_hash(lines.iter().map(|(p, id)| (p.as_slice(), id.as_deref())));
    let changed = lines
        .into_iter()
        .map(|(p, id)| (String::from_utf8_lossy(&p).into_owned(), id))
        .collect();
    Ok(WorkingState {
        position: PositionRef {
            repo: repo.to_string(),
            commit,
            dirty,
        },
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: &str = "e410760849abb8cd9fa8fbba305f5dd5d96ba954";
    const D: &str = "83ebb47e03fee94773edcd621a87f37980b3b408b96467062fcc87368f3260c7";

    /// R-3: the three text forms and the JSON round-trip.
    #[test]
    fn text_roundtrip() {
        for (text, commit, dirty) in [
            (C.to_string(), Some(C), None),
            (format!("{C}+{D}"), Some(C), Some(D)),
            (format!("none+{D}"), None, Some(D)),
        ] {
            let p = PositionRef::parse("r", &text).unwrap();
            assert_eq!(p.repo, "r");
            assert_eq!(p.commit.as_deref(), commit);
            assert_eq!(p.dirty.as_deref(), dirty);
            assert_eq!(p.text(), text);
        }
        let p = PositionRef::parse("r", &format!("{C}+{D}")).unwrap();
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            format!(r#"{{"repo":"r","commit":"{C}","dirty":"{D}"}}"#)
        );
        let none = PositionRef {
            repo: "r".into(),
            commit: None,
            dirty: Some(D.into()),
        };
        assert!(
            serde_json::to_string(&none)
                .unwrap()
                .contains(r#""commit":null"#)
        );
        assert_eq!(none.short(), "none+83eb");
        let c = PositionRef::at_commit("r", C);
        assert!(c.is_clean());
        assert_eq!(c.short(), "e410760");
        assert_eq!(p.short(), "e410760+83eb");
        assert!(!p.is_clean());
        assert!(is_view_id("v1-096ec1a42f3e9b74ae0f0348152caeb2"));
        assert!(!is_view_id("v1-096ec1a42f3e9b74ae0f0348152caeb"));
        assert!(!is_view_id("v2-096ec1a42f3e9b74ae0f0348152caeb2"));
        assert!(!is_view_id("v1-096EC1a42f3e9b74ae0f0348152caeb2"));
    }

    /// R-3: katalogr's twelve bad texts are refused.
    #[test]
    fn bad_text_is_error() {
        let upper = C.to_uppercase();
        for bad in [
            "",
            "none",
            "abc",
            &C[..39],
            &format!("{C}0"),
            &upper,
            &format!("{C}+"),
            &format!("{C}+{}", &D[..63]),
            &format!("{C}+{}", D.to_uppercase()),
            &format!("none+{}", &D[..10]),
            &format!("{C}+{D}+{D}"),
            &format!("zz{}+{D}", &C[2..]),
        ] {
            assert!(PositionRef::parse("r", bad).is_err(), "accepted {bad:?}");
        }
    }

    /// R-1: katalogr's vectors for the dirty hash and the blob id.
    #[test]
    fn hash_matches_spec_vector() {
        let blob = blob_id(b"changed\n");
        assert_eq!(blob, "5ea2ed416fbd4a4cbe227b75fe255dd7fa6bd4d6");
        assert_eq!(
            dirty_hash([(b"a.txt".as_slice(), Some(blob.as_str()))]).as_deref(),
            Some(D)
        );
        assert_eq!(
            dirty_hash([(b"src/b.txt".as_slice(), None)]).as_deref(),
            Some("c09b5a87b6536d1a72e30c97cfd47a03df6b36c40af129dabe9947c4f99b0ce0")
        );
        assert_eq!(dirty_hash(std::iter::empty()), None);
        let a = dirty_hash([(b"a".as_slice(), None), (b"b".as_slice(), Some("x"))]);
        let b = dirty_hash([(b"b".as_slice(), Some("x")), (b"a".as_slice(), None)]);
        assert_eq!(a, b);
    }

    /// R-3: katalogr's spec vector gives the same ID, and every input matters.
    #[test]
    fn view_id_matches_the_shared_vector() {
        let app = PositionRef::at_commit("app", C);
        let zed = PositionRef::parse("zed", &format!("none+{D}")).unwrap();
        let q = ViewQuery {
            op: "find".into(),
            args: BTreeMap::from([
                ("budget".to_string(), "400".to_string()),
                ("text".to_string(), "call".to_string()),
            ]),
        };
        let ps = [app.clone(), zed.clone()];
        let id = view_id(&ps, "0123abcd", &q);
        assert_eq!(id, "v1-096ec1a42f3e9b74ae0f0348152caeb2");
        assert!(is_view_id(&id));
        let mut q2 = q.clone();
        q2.args.insert("budget".into(), "401".into());
        assert_ne!(view_id(&ps, "0123abcd", &q2), id);
        assert_ne!(view_id(&ps, "0123abce", &q), id);
        assert_ne!(view_id(&ps[..1], "0123abcd", &q), id);
        let mut q3 = q.clone();
        q3.op = "other".into();
        assert_ne!(view_id(&ps, "0123abcd", &q3), id);
    }

    // ---- scratch repositories (integration) ----

    use crate::coderepo::{git, tests::commit_file, tests::scratch_repo};

    /// A scratch repository that no global git setting can bend: no line-ending conversion and an
    /// empty global ignore file. Removes itself with `gone`.
    fn repo(name: &str) -> PathBuf {
        let dir = scratch_repo(&format!("pos-{name}"));
        let empty = dir.join(".git/empty-excludes");
        std::fs::write(&empty, "").unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        git(&dir, &["config", "core.excludesFile", empty.to_str().unwrap()]).unwrap();
        dir
    }

    fn gone(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn pos(dir: &Path) -> WorkingState {
        read_position(dir, "r").unwrap()
    }

    /// A clean checkout has a commit and no dirty hash.
    #[test]
    fn clean_has_no_dirty_hash() {
        let d = repo("clean");
        let st = pos(&d);
        assert!(st.position.commit.is_some());
        assert!(st.position.is_clean());
        assert!(st.changed.is_empty());
        gone(&d);
    }

    #[test]
    fn edit_changes_hash() {
        let d = repo("edit");
        std::fs::write(d.join("README"), "edited\n").unwrap();
        let a = pos(&d);
        assert!(a.position.dirty.is_some());
        assert_eq!(a.changed.len(), 1);
        std::fs::write(d.join("README"), "edited again\n").unwrap();
        assert_ne!(pos(&d).position.dirty, a.position.dirty);
        gone(&d);
    }

    #[test]
    fn revert_to_head_is_clean() {
        let d = repo("revert");
        std::fs::write(d.join("README"), "edited\n").unwrap();
        assert!(!pos(&d).position.is_clean());
        std::fs::write(d.join("README"), "base\n").unwrap();
        assert!(pos(&d).position.is_clean());
        gone(&d);
    }

    /// R-1: staging a change alone moves nothing; the bytes on disk decide.
    #[test]
    fn staging_alone_changes_nothing() {
        let d = repo("staging");
        let before = pos(&d).position;
        std::fs::write(d.join("README"), "edited\n").unwrap();
        let edited = pos(&d).position;
        git(&d, &["add", "README"]).unwrap();
        assert_eq!(pos(&d).position, edited, "staging the edit changed it");
        // Stage something other than the disk holds: the disk still decides.
        std::fs::write(d.join("README"), "base\n").unwrap();
        assert_eq!(pos(&d).position, before);
        gone(&d);
    }

    /// R-1: an untracked file counts, an ignored one does not.
    #[test]
    fn untracked_counts_ignored_does_not() {
        let d = repo("untracked");
        commit_file(&d, ".gitignore", "*.log\n", "ignore");
        let clean = pos(&d).position;
        std::fs::write(d.join("x.log"), "noise\n").unwrap();
        assert_eq!(pos(&d).position, clean);
        std::fs::write(d.join("new.txt"), "new\n").unwrap();
        let st = pos(&d);
        assert_ne!(st.position, clean);
        assert_eq!(st.changed.len(), 1);
        assert_eq!(st.changed[0].0, "new.txt");
        gone(&d);
    }

    /// R-1: a deleted tracked path is a line with `-`, not a blob id.
    #[test]
    fn deleted_uses_dash() {
        let d = repo("deleted");
        std::fs::remove_file(d.join("README")).unwrap();
        let st = pos(&d);
        assert_eq!(st.changed, vec![("README".to_string(), None)]);
        assert_eq!(
            st.position.dirty,
            dirty_hash([(b"README".as_slice(), None)])
        );
        gone(&d);
    }

    /// R-1: a mode change with the same bytes is not a difference.
    #[cfg(unix)]
    #[test]
    fn mode_change_does_not_count() {
        use std::os::unix::fs::PermissionsExt;
        let d = repo("mode");
        let p = d.join("README");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(pos(&d).position.is_clean());
        gone(&d);
    }

    /// R-1: before the first commit there is no commit, and untracked files count.
    #[test]
    fn unborn_head_has_no_commit() {
        let dir = std::env::temp_dir().join(format!("kanbanr-pos-{}-unborn", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "--initial-branch=main"]).unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        let empty = pos(&dir);
        assert_eq!(empty.position.commit, None);
        assert_eq!(empty.position.dirty, None);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let st = pos(&dir);
        assert_eq!(st.position.commit, None);
        assert!(st.position.dirty.is_some());
        assert!(st.position.text().starts_with("none+"));
        // A path staged then removed from disk is simply absent.
        git(&dir, &["add", "a.txt"]).unwrap();
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        assert_eq!(pos(&dir).position.dirty, None);
        gone(&dir);
    }

    // ---- the shared file (R-1) ----

    fn contract() -> serde_json::Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/contract/position-v1.json");
        serde_json::from_slice(&std::fs::read(path).expect("read position-v1.json"))
            .expect("parse position-v1.json")
    }

    fn files(v: &serde_json::Value) -> BTreeMap<String, String> {
        v.as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn put(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// `git` with stdin, in a clean environment, as katalogr's test runs it.
    fn git_in(
        root: &Path,
        args: &[&str],
        stdin: &[u8],
        env: &[(&str, &str)],
    ) -> std::process::Output {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut cmd = Command::new("git");
        cmd.current_dir(root)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut ch = cmd.spawn().unwrap();
        ch.stdin.take().unwrap().write_all(stdin).unwrap();
        ch.wait_with_output().unwrap()
    }

    /// R-1: the file names the six required cases, each `expected` parses and round-trips.
    #[test]
    fn shared_file_covers_required_cases() {
        let v = contract();
        assert_eq!(v["format"], 1);
        let names: Vec<&str> = v["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        for need in [
            "clean",
            "edited",
            "deleted",
            "untracked",
            "staged_only",
            "line_endings",
        ] {
            assert!(names.contains(&need), "missing case {need}: {names:?}");
        }
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "duplicate case names");
        for c in v["cases"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            assert!(c["head"].is_object() && c["worktree"].is_object(), "{name}");
            let expected = c["expected"].as_str().unwrap();
            let parsed = PositionRef::parse("r", expected).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(parsed.text(), expected, "{name}");
        }
    }

    /// R-1: every case of the shared file, built with real git as katalogr builds it, gives its
    /// expected text, cold and on a second read.
    #[test]
    fn shared_file_cases_match() {
        let v = contract();
        let author = v["commit"]["author"].as_str().unwrap();
        let (name, email) = author.split_once(" <").unwrap();
        let email = email.trim_end_matches('>');
        let time = format!("@{}", v["commit"]["time"].as_str().unwrap());
        let message = v["commit"]["message"].as_str().unwrap();
        let mut failures = Vec::new();
        for c in v["cases"].as_array().unwrap() {
            let case = c["name"].as_str().unwrap();
            let r = std::env::temp_dir().join(format!(
                "kanbanr-pos-{}-shared-{case}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&r);
            std::fs::create_dir_all(&r).unwrap();
            git(&r, &["init", "-q"]).unwrap();
            let empty = r.join(".git/empty-config");
            std::fs::write(&empty, "").unwrap();
            let empty_s = empty.to_str().unwrap();
            git(&r, &["config", "core.autocrlf", "false"]).unwrap();
            git(&r, &["config", "core.excludesFile", empty_s]).unwrap();
            for (path, text) in files(&c["head"]) {
                put(&r, &path, &text);
            }
            git(&r, &["add", "-A"]).unwrap();
            let out = git_in(
                &r,
                &["commit", "-q", "--no-gpg-sign", "-F", "-"],
                message.as_bytes(),
                &[
                    ("GIT_CONFIG_GLOBAL", empty_s),
                    ("GIT_CONFIG_SYSTEM", empty_s),
                    ("GIT_AUTHOR_NAME", name),
                    ("GIT_AUTHOR_EMAIL", email),
                    ("GIT_AUTHOR_DATE", &time),
                    ("GIT_COMMITTER_NAME", name),
                    ("GIT_COMMITTER_EMAIL", email),
                    ("GIT_COMMITTER_DATE", &time),
                ],
            );
            assert!(out.status.success(), "{case}: commit failed");
            if let Some(cfg) = c["config"].as_object() {
                for (k, val) in cfg {
                    git(&r, &["config", k, val.as_str().unwrap()]).unwrap();
                }
            }
            let want = files(&c["worktree"]);
            for (path, _) in files(&c["head"]) {
                if !want.contains_key(&path) {
                    std::fs::remove_file(r.join(&path)).unwrap();
                }
            }
            for (path, text) in &want {
                put(&r, path, text);
            }
            for (path, text) in files(&c["index"]) {
                let blob = git_in(&r, &["hash-object", "-w", "--stdin"], text.as_bytes(), &[]);
                let id = String::from_utf8(blob.stdout).unwrap().trim().to_string();
                git(
                    &r,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{id},{path}"),
                    ],
                )
                .unwrap();
            }
            let expected = c["expected"].as_str().unwrap();
            for how in ["cold", "second read"] {
                let got = pos(&r).position.text();
                if got != expected {
                    failures.push(format!("{case} ({how}): got {got}, expected {expected}"));
                }
            }
            gone(&r);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
