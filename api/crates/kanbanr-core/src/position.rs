//! A repository's position: a commit plus a hash of what is uncommitted (SPK-001; chain decisions
//! CD-2 and CD-3 of katalogr FEAT-043, shared byte for byte with katalogr), and files read at a
//! position with nothing checked out.
//!
//! FTR-003 took this over from SPK-001: `kanbanr position`, `kanbanr view` and the daemon's
//! position and view routes read it.
//!
//! Each working file is compared in its repository form, after git's to-git conversion
//! (`core.autocrlf`, `core.eol`, the `text`, `eol`, `crlf` and `ident` attributes), and the dirty
//! part carries repository-form blob ids: the ids `git add` would store (DEF-066, katalogr
//! FEAT-141). Driver filters are never run.

use crate::coderepo::{git_raw, rev_parse};
use crate::error::{CoreError, Result};
use crate::repoform::RepoForm;
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
/// the repository-form blob id (what `git add` would store), None for a deleted path. Paths are the repo-relative, '/'-separated bytes git prints, lossy
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

/// What is at `path` with no filter applied: a symbolic link's target, a regular file's bytes; None
/// for anything else (missing, a directory). The flag says whether the path is a symbolic link.
fn raw_bytes(root: &Path, path: &[u8]) -> Option<(Vec<u8>, bool)> {
    let full = path_of(root, path);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(&full).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            return Some((target.as_os_str().as_bytes().to_vec(), true));
        }
        #[cfg(not(unix))]
        return Some((
            target.to_string_lossy().replace('\\', "/").into_bytes(),
            true,
        ));
    }
    if meta.is_file() {
        return std::fs::read(&full).ok().map(|b| (b, false));
    }
    None
}

/// What git would store at `path` (DEF-066): None when nothing regular is there; else the blob id
/// and the bytes it hashes. `known` is the id to try first (the HEAD blob, or the id a position
/// recorded): raw bytes that hash to it are returned unconverted. A symbolic link's target and a
/// path that is not UTF-8 are taken raw; any other regular file is taken in its repository form.
fn stored_id(
    form: &mut RepoForm,
    root: &Path,
    path: &[u8],
    known: Option<&str>,
) -> Option<(String, Vec<u8>)> {
    let (raw, is_link) = raw_bytes(root, path)?;
    let raw_id = blob_id(&raw);
    if is_link || known == Some(raw_id.as_str()) {
        return Some((raw_id, raw));
    }
    let Ok(rela) = std::str::from_utf8(path) else {
        return Some((raw_id, raw));
    };
    match form.convert(rela, &raw) {
        std::borrow::Cow::Borrowed(_) => Some((raw_id, raw)),
        std::borrow::Cow::Owned(stored) => Some((blob_id(&stored), stored)),
    }
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
    read_state(root, repo, 1).map(|(s, _)| s)
}

/// read_position's body: the state, and how many files it converted. `workers` is the number of
/// threads that read the tracked files (1 reads on the calling thread).
pub(crate) fn read_state(root: &Path, repo: &str, workers: usize) -> Result<(WorkingState, usize)> {
    let _ = workers;
    let commit = rev_parse(root, "HEAD").ok();
    let head = match &commit {
        Some(c) => head_blobs(root, c, "")?,
        None => BTreeMap::new(),
    };
    let mut lines: BTreeMap<Vec<u8>, Option<String>> = BTreeMap::new();
    let mut form = RepoForm::new(root);
    for (path, oid) in &head {
        match stored_id(&mut form, root, path, Some(oid)) {
            Some((id, _)) => {
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
        if let Some((id, _)) = stored_id(&mut form, root, &path, None) {
            lines.insert(path, Some(id));
        }
    }
    let dirty = dirty_hash(lines.iter().map(|(p, id)| (p.as_slice(), id.as_deref())));
    let changed = lines
        .into_iter()
        .map(|(p, id)| (String::from_utf8_lossy(&p).into_owned(), id))
        .collect();
    Ok((
        WorkingState {
            position: PositionRef {
                repo: repo.to_string(),
                commit,
                dirty,
            },
            changed,
        },
        form.converted(),
    ))
}

/// The bytes of `path` at the position `state` describes, with nothing checked out. None when the
/// path is absent there. Err when an uncommitted path's bytes no longer hash to what the position
/// recorded ("position moved").
pub fn read_at(root: &Path, state: &WorkingState, path: &str) -> Result<Option<Vec<u8>>> {
    if let Ok(i) = state
        .changed
        .binary_search_by(|(p, _)| p.as_str().cmp(path))
    {
        let Some(want) = &state.changed[i].1 else {
            return Ok(None);
        };
        return match raw_bytes(root, path.as_bytes()) {
            Some((bytes, _)) if &blob_id(&bytes) == want => Ok(Some(bytes)),
            _ => Err(CoreError::Git(format!(
                "position moved: {path} changed since the position was read"
            ))),
        };
    }
    let Some(commit) = &state.position.commit else {
        return Ok(None);
    };
    match git_raw(root, &["cat-file", "blob", &format!("{commit}:{path}")]) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(CoreError::Git(why))
            if why.contains("does not exist") || why.contains("Not a valid object name") =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// The files under `prefix` ("" for all; "docs" means "docs/") at the position: (path, blob id),
/// sorted by path. Deleted paths are gone, uncommitted and untracked ones present.
pub fn tree_at(root: &Path, state: &WorkingState, prefix: &str) -> Result<Vec<(String, String)>> {
    let prefix = match prefix.trim_matches('/') {
        "" => String::new(),
        dir => format!("{dir}/"),
    };
    let mut files: BTreeMap<String, String> = match &state.position.commit {
        Some(c) => head_blobs(root, c, &prefix)?
            .into_iter()
            .map(|(p, o)| (String::from_utf8_lossy(&p).into_owned(), o))
            .collect(),
        None => BTreeMap::new(),
    };
    for (path, id) in &state.changed {
        if !path.starts_with(&prefix) {
            continue;
        }
        match id {
            None => {
                files.remove(path);
            }
            Some(id) => {
                files.insert(path.clone(), id.clone());
            }
        }
    }
    Ok(files.into_iter().collect())
}

/// What `kanbanr position --json` prints and `GET /projects/{p}/position` returns (FTR-003).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PositionAnswer {
    pub schema: u32,
    pub live: PositionRef,
    /// `live.text()`.
    pub text: String,
    /// The dirty part, sorted by path.
    pub changed: Vec<ChangedPath>,
}

/// One uncommitted path: its repository-form blob id (what `git add` would store), absent for a
/// deleted path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChangedPath {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// The live position of the work tree `root` (its top), named `repo`.
pub fn position_answer(root: &Path, repo: &str) -> Result<PositionAnswer> {
    let st = read_position(root, repo)?;
    Ok(PositionAnswer {
        schema: 1,
        text: st.position.text(),
        live: st.position,
        changed: st
            .changed
            .into_iter()
            .map(|(path, blob)| ChangedPath { path, blob })
            .collect(),
    })
}

/// What a read answers for: the live checkout, or a clean commit read from objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Live(WorkingState),
    Commit(WorkingState),
}

impl Resolved {
    pub fn state(&self) -> &WorkingState {
        match self {
            Resolved::Live(s) | Resolved::Commit(s) => s,
        }
    }
}

/// Resolve `at` against the checkout at `root`: None, or equal to the live position, gives the live
/// state; a clean position whose commit exists here gives that commit, read from objects with
/// nothing checked out; anything else is [`CoreError::PositionUnavailable`] naming the live
/// position. Text that is not a position is `CoreError::Unsupported` (a 400).
pub fn resolve_position(root: &Path, repo: &str, at: Option<&str>) -> Result<Resolved> {
    let live = read_position(root, repo)?;
    let Some(text) = at else {
        return Ok(Resolved::Live(live));
    };
    let wanted =
        PositionRef::parse(repo, text).map_err(|e| CoreError::Unsupported(e.to_string()))?;
    if wanted == live.position {
        return Ok(Resolved::Live(live));
    }
    if let (true, Some(commit)) = (wanted.is_clean(), &wanted.commit)
        && rev_parse(root, commit).is_ok()
    {
        return Ok(Resolved::Commit(WorkingState {
            position: wanted,
            changed: vec![],
        }));
    }
    Err(CoreError::PositionUnavailable {
        wanted: wanted.text(),
        available: vec![live.position],
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

    /// A scratch repository that no global git setting can bend: no line-ending conversion, an
    /// empty global ignore file and an empty global attributes file, pinned in its own config
    /// (libgit2 does not honour GIT_CONFIG_GLOBAL). Removes itself with `gone`.
    fn repo(name: &str) -> PathBuf {
        let dir = scratch_repo(&format!("pos-{name}"));
        let empty = dir.join(".git/empty-excludes");
        std::fs::write(&empty, "").unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        git(
            &dir,
            &["config", "core.excludesFile", empty.to_str().unwrap()],
        )
        .unwrap();
        let attrs = dir.join(".git/empty-attributes");
        std::fs::write(&attrs, "").unwrap();
        git(
            &dir,
            &["config", "core.attributesFile", attrs.to_str().unwrap()],
        )
        .unwrap();
        assert!(
            dir.canonicalize()
                .unwrap()
                .starts_with(std::env::temp_dir().canonicalize().unwrap()),
            "the scratch repository is the test's own"
        );
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

    fn contract_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/contract/position-v1.json");
        std::fs::read(path).expect("read position-v1.json")
    }

    fn contract() -> serde_json::Value {
        serde_json::from_slice(&contract_bytes()).expect("parse position-v1.json")
    }

    /// DEF-066 R-1: the shared file is katalogr's copy at 04fe2e7 (its v0.2.0) byte for byte, so a
    /// hand edit or a stale copy fails.
    #[test]
    fn shared_file_is_katalogrs_copy() {
        assert_eq!(
            blob_id(&contract_bytes()),
            "3cd6f5d073717aa8142533f53819459f9258e29d"
        );
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
            let parsed =
                PositionRef::parse("r", expected).unwrap_or_else(|e| panic!("{name}: {e}"));
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
            let r = std::env::temp_dir()
                .join(format!("kanbanr-pos-{}-shared-{case}", std::process::id()));
            let _ = std::fs::remove_dir_all(&r);
            std::fs::create_dir_all(&r).unwrap();
            git(&r, &["init", "-q"]).unwrap();
            let empty = r.join(".git/empty-config");
            std::fs::write(&empty, "").unwrap();
            let empty_s = empty.to_str().unwrap();
            git(&r, &["config", "core.autocrlf", "false"]).unwrap();
            git(&r, &["config", "core.excludesFile", empty_s]).unwrap();
            git(&r, &["config", "core.attributesFile", empty_s]).unwrap();
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

    // ---- reading at a position (R-2) ----

    fn head_of(d: &Path) -> (String, String, String) {
        (
            git(d, &["rev-parse", "HEAD"]).unwrap(),
            git(d, &["symbolic-ref", "HEAD"]).unwrap(),
            git(d, &["status", "--porcelain=v1", "--untracked-files=no"]).unwrap(),
        )
    }

    /// R-2: a clean path comes from the commit even when the disk differs later, an uncommitted
    /// path from disk, a deleted or absent path is None, and nothing is checked out.
    #[test]
    fn read_at_serves_commit_disk_and_deleted() {
        let d = repo("readat");
        commit_file(&d, "keep.txt", "keep\n", "keep");
        commit_file(&d, "gone.txt", "gone\n", "gone");
        std::fs::write(d.join("README"), "edited\n").unwrap();
        std::fs::remove_file(d.join("gone.txt")).unwrap();
        std::fs::write(d.join("new.txt"), "new\n").unwrap();
        let st = pos(&d);
        let before = head_of(&d);
        assert_eq!(
            read_at(&d, &st, "keep.txt").unwrap(),
            Some(b"keep\n".to_vec())
        );
        assert_eq!(
            read_at(&d, &st, "README").unwrap(),
            Some(b"edited\n".to_vec())
        );
        assert_eq!(
            read_at(&d, &st, "new.txt").unwrap(),
            Some(b"new\n".to_vec())
        );
        assert_eq!(read_at(&d, &st, "gone.txt").unwrap(), None);
        assert_eq!(read_at(&d, &st, "never.txt").unwrap(), None);
        // A clean path is the commit's bytes, whatever the disk holds later.
        std::fs::write(d.join("keep.txt"), "disk moved on\n").unwrap();
        assert_eq!(
            read_at(&d, &st, "keep.txt").unwrap(),
            Some(b"keep\n".to_vec())
        );
        assert_eq!(head_of(&d).0, before.0);
        assert_eq!(head_of(&d).1, before.1);
        // An unborn position holds nothing committed.
        let no_commit = WorkingState {
            position: PositionRef {
                repo: "r".into(),
                commit: None,
                dirty: None,
            },
            changed: vec![],
        };
        assert_eq!(read_at(&d, &no_commit, "README").unwrap(), None);
        gone(&d);
    }

    /// R-2: an uncommitted file edited after the position was read is an error, never new bytes.
    #[test]
    fn read_at_refuses_a_moved_file() {
        let d = repo("moved");
        std::fs::write(d.join("README"), "edited\n").unwrap();
        let st = pos(&d);
        std::fs::write(d.join("README"), "edited again\n").unwrap();
        let err = read_at(&d, &st, "README").unwrap_err().to_string();
        assert!(err.contains("position moved"), "{err}");
        std::fs::remove_file(d.join("README")).unwrap();
        assert!(read_at(&d, &st, "README").is_err());
        gone(&d);
    }

    /// R-2: the list is the position's files, not the disk's.
    #[test]
    fn tree_at_lists_the_position_not_the_disk() {
        let d = repo("tree");
        commit_file(&d, "keep.txt", "k\n", "k");
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::create_dir_all(d.join("docs2")).unwrap();
        commit_file(&d, "docs/a.md", "a\n", "a");
        commit_file(&d, "docs/b.md", "b\n", "b");
        commit_file(&d, "docs2/c.md", "c\n", "c");
        std::fs::remove_file(d.join("docs/b.md")).unwrap();
        std::fs::write(d.join("docs/a.md"), "a edited\n").unwrap();
        std::fs::write(d.join("docs/new.md"), "n\n").unwrap();
        let st = pos(&d);
        let paths = |prefix: &str| -> Vec<String> {
            tree_at(&d, &st, prefix)
                .unwrap()
                .into_iter()
                .map(|(p, _)| p)
                .collect()
        };
        assert_eq!(paths("docs"), vec!["docs/a.md", "docs/new.md"]);
        assert_eq!(paths("docs/"), paths("docs"));
        assert_eq!(paths("docs2"), vec!["docs2/c.md"]);
        assert_eq!(paths("empty"), Vec::<String>::new());
        assert_eq!(paths("").len(), 5);
        let a = tree_at(&d, &st, "docs").unwrap();
        assert_eq!(a[0].1, blob_id(b"a edited\n"));
        gone(&d);
    }

    // ---- the repository form (DEF-066) ----

    /// How a CRLF checkout gets its CRLF: `core.autocrlf=true`, or a committed `.gitattributes`.
    #[derive(Clone, Copy, Debug)]
    enum Eol {
        AutoCrlf,
        Attributes,
    }

    const BIN: &[u8] = b"\0\x01bin\r\nary\r\n\x02";

    /// DEF-066: a checkout whose text files git checked out with CRLF over LF blobs. Commits
    /// `a.txt`, `src/lib.rs`, `docs/guide.md` (LF) and `bin.dat` (binary with CRLF), configures the
    /// conversion, deletes the files and checks them out again.
    fn crlf_checkout(name: &str, eol: Eol) -> PathBuf {
        let d = repo(name);
        put(&d, "a.txt", "alpha\nbeta\n");
        put(&d, "src/lib.rs", "fn a() {}\nfn b() {}\n");
        put(&d, "docs/guide.md", "# Guide\n\nText.\n");
        std::fs::write(d.join("bin.dat"), BIN).unwrap();
        git(&d, &["add", "-A"]).unwrap();
        git(&d, &["commit", "-q", "-m", "lf"]).unwrap();
        match eol {
            Eol::AutoCrlf => {
                git(&d, &["config", "core.autocrlf", "true"]).unwrap();
            }
            Eol::Attributes => commit_file(&d, ".gitattributes", "* text=auto eol=crlf\n", "eol"),
        }
        for f in ["a.txt", "src/lib.rs", "docs/guide.md", "bin.dat"] {
            std::fs::remove_file(d.join(f)).unwrap();
        }
        git(&d, &["checkout", "--", "."]).unwrap();
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"alpha\r\nbeta\r\n");
        assert_eq!(
            std::fs::read(d.join("src/lib.rs")).unwrap(),
            b"fn a() {}\r\nfn b() {}\r\n"
        );
        assert_eq!(
            std::fs::read(d.join("docs/guide.md")).unwrap(),
            b"# Guide\r\n\r\nText.\r\n"
        );
        assert_eq!(std::fs::read(d.join("bin.dat")).unwrap(), BIN);
        d
    }

    /// What `git add <file>` then `git ls-files -s <file>` stores for `file`.
    fn added_id(d: &Path, file: &str) -> String {
        git(d, &["add", file]).unwrap();
        let line = git(d, &["ls-files", "-s", "--", file]).unwrap();
        line.split_whitespace().nth(1).unwrap().to_string()
    }

    fn id_of<'a>(st: &'a WorkingState, path: &str) -> Option<&'a str> {
        st.changed
            .iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, id)| id.as_deref())
    }

    /// DEF-066 R-2: a CRLF checkout of LF blobs reads clean, by either way of getting CRLF; the
    /// binary file counts as unchanged.
    #[test]
    fn crlf_checkout_is_clean() {
        for (name, eol) in [("clean-auto", Eol::AutoCrlf), ("clean-attr", Eol::Attributes)] {
            let d = crlf_checkout(name, eol);
            let head = git(&d, &["rev-parse", "HEAD"]).unwrap();
            for _ in 0..2 {
                let st = pos(&d);
                assert_eq!(st.position.dirty, None, "{eol:?}: {:?}", st.changed);
                assert_eq!(st.position.commit.as_deref(), Some(head.as_str()));
                assert!(st.changed.is_empty(), "{eol:?}: {:?}", st.changed);
            }
            gone(&d);
        }
    }

    /// DEF-066 R-2: a real edit in a CRLF checkout, and an untracked CRLF file, carry the ids
    /// `git add` stores, the LF form's.
    #[test]
    fn real_edit_in_a_crlf_checkout_has_the_stored_id() {
        let d = crlf_checkout("edit-crlf", Eol::AutoCrlf);
        std::fs::write(d.join("src/lib.rs"), "fn a() {}\r\nfn c() {}\r\n").unwrap();
        std::fs::write(d.join("new.txt"), "x\r\ny\r\n").unwrap();
        let st = pos(&d);
        let paths: Vec<&str> = st.changed.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["new.txt", "src/lib.rs"]);
        let lib = blob_id(b"fn a() {}\nfn c() {}\n");
        assert_eq!(id_of(&st, "src/lib.rs"), Some(lib.as_str()));
        let new = blob_id(b"x\ny\n");
        assert_eq!(id_of(&st, "new.txt"), Some(new.as_str()));
        assert_eq!(added_id(&d, "src/lib.rs"), lib);
        assert_eq!(added_id(&d, "new.txt"), new);
        assert_eq!(pos(&d), st, "staging changed the position");
        gone(&d);
    }

    /// DEF-066 R-2: a binary file with CRLF, tracked or untracked, keeps its raw-bytes id.
    #[test]
    fn binary_with_crlf_is_not_converted() {
        let d = crlf_checkout("bin-crlf", Eol::AutoCrlf);
        let edited: &[u8] = b"\0\x02bin\r\nedited\r\n";
        let new: &[u8] = b"\0new\r\nbin\r\n";
        std::fs::write(d.join("bin.dat"), edited).unwrap();
        std::fs::write(d.join("new.bin"), new).unwrap();
        let st = pos(&d);
        assert_eq!(id_of(&st, "bin.dat"), Some(blob_id(edited).as_str()));
        assert_eq!(id_of(&st, "new.bin"), Some(blob_id(new).as_str()));
        assert_eq!(st.changed.len(), 2, "{:?}", st.changed);
        gone(&d);
    }

    /// DEF-066 R-2: a blob committed with CRLF is not normalised later ("safer autocrlf"): a CRLF
    /// edit keeps its raw id.
    #[test]
    fn crlf_in_the_index_is_not_normalised() {
        let d = repo("crlf-index");
        std::fs::write(d.join("c.txt"), "a\r\nb\r\n").unwrap();
        git(&d, &["add", "c.txt"]).unwrap();
        git(&d, &["commit", "-q", "-m", "crlf blob"]).unwrap();
        commit_file(&d, ".gitattributes", "* text=auto\n", "auto");
        git(&d, &["config", "core.autocrlf", "true"]).unwrap();
        assert!(pos(&d).position.is_clean());
        let edit: &[u8] = b"a\r\nb\r\nc\r\n";
        std::fs::write(d.join("c.txt"), edit).unwrap();
        let st = pos(&d);
        assert_eq!(id_of(&st, "c.txt"), Some(blob_id(edit).as_str()));
        assert_eq!(added_id(&d, "c.txt"), blob_id(edit));
        gone(&d);
    }

    /// DEF-066 R-2: under each attribute that converts, an untracked file's id is what `git add`
    /// stores, and staging it changes no position.
    #[test]
    fn repository_form_ids_match_what_git_add_stores() {
        let d = repo("attrs");
        // A person's global core.safecrlf=true would make `git add` refuse a mixed file.
        git(&d, &["config", "core.safecrlf", "false"]).unwrap();
        commit_file(
            &d,
            ".gitattributes",
            "*.bat -text\n*.t text\n*.old crlf\n*.lf eol=lf\n*.c ident\n",
            "attributes",
        );
        let cases: [(&str, &[u8]); 5] = [
            ("r.bat", b"a\r\n"),
            ("m.t", b"a\rb\r\nc\r\n"),
            ("x.old", b"q\r\n"),
            ("y.lf", b"q\r\nw\n"),
            ("a.c", b"$Id: 1234567890 $\nint x;\n"),
        ];
        let second = repo("attrs-input");
        git(&second, &["config", "core.autocrlf", "input"]).unwrap();
        std::fs::write(second.join("a.txt"), b"a\r\n").unwrap();
        for (f, bytes) in cases {
            std::fs::write(d.join(f), bytes).unwrap();
        }
        for (root, names) in [
            (&d, cases.iter().map(|(f, _)| *f).collect::<Vec<_>>()),
            (&second, vec!["a.txt"]),
        ] {
            let st = pos(root);
            for f in &names {
                let id = id_of(&st, f).unwrap_or_else(|| panic!("{f} not changed")).to_string();
                assert_eq!(added_id(root, f), id, "{f}");
            }
            assert_eq!(pos(root), st, "staging changed the position");
        }
        // The conversions are real ones, not the identity.
        let st = pos(&d);
        assert_eq!(id_of(&st, "r.bat"), Some(blob_id(b"a\r\n").as_str()));
        assert_eq!(id_of(&st, "x.old"), Some(blob_id(b"q\n").as_str()));
        assert_eq!(id_of(&st, "a.c"), Some(blob_id(b"$Id$\nint x;\n").as_str()));
        let st = pos(&second);
        assert_eq!(id_of(&st, "a.txt"), Some(blob_id(b"a\n").as_str()));
        gone(&d);
        gone(&second);
    }

    /// DEF-066 R-2: `core.safecrlf=true` never fails a read; a mixed file converts.
    #[test]
    fn safecrlf_does_not_fail_a_read() {
        let d = repo("safecrlf");
        git(&d, &["config", "core.autocrlf", "true"]).unwrap();
        git(&d, &["config", "core.safecrlf", "true"]).unwrap();
        std::fs::write(d.join("m.txt"), b"a\r\nb\n").unwrap();
        let st = read_position(&d, "r").expect("the read succeeds");
        assert_eq!(id_of(&st, "m.txt"), Some(blob_id(b"a\nb\n").as_str()));
        gone(&d);
    }

    /// DEF-066 R-4: with no conversion configured, CR bytes stay in the ids.
    #[test]
    fn lf_checkout_ids_are_raw_bytes_ids() {
        let d = repo("lf-raw");
        let edit: &[u8] = b"one\r\nmore\r\n";
        std::fs::write(d.join("README"), edit).unwrap();
        std::fs::write(d.join("b.txt"), b"b\r\n").unwrap();
        let st = pos(&d);
        assert_eq!(id_of(&st, "README"), Some(blob_id(edit).as_str()));
        assert_eq!(id_of(&st, "b.txt"), Some(blob_id(b"b\r\n").as_str()));
        gone(&d);
    }

    /// DEF-066 R-4: a clean LF checkout converts nothing; one edit converts that file alone.
    #[test]
    fn an_lf_checkout_converts_nothing() {
        let d = repo("lf-none");
        commit_file(&d, "a.txt", "a\n", "a");
        commit_file(&d, "b.txt", "b\n", "b");
        let (st, converted) = read_state(&d, "r", 1).unwrap();
        assert!(st.position.is_clean());
        assert_eq!(converted, 0);
        std::fs::write(d.join("a.txt"), "a edited\n").unwrap();
        let (st, converted) = read_state(&d, "r", 1).unwrap();
        assert_eq!(st.changed.len(), 1);
        assert_eq!(converted, 1);
        gone(&d);
    }

    // ---- the measurement (R-6) ----

    fn median_ms(root: &Path) -> (f64, Vec<f64>) {
        let mut ms: Vec<f64> = (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                let st = pos(root);
                assert!(st.position.commit.is_some());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        (ms[1], ms)
    }

    /// The decided budget (SPK-001: 141.8 ms measured in a release build, 200 ms set) for one
    /// position read of a clean 10,000-file checkout, warm.
    const BUDGET_MS: f64 = 200.0;

    /// R-10: one position read of a clean 10,000-file checkout, warm, three times; the median is
    /// printed with the load average and held to the budget (twice the budget when the load
    /// average is above 4, so a busy machine does not fail it for being busy). A release-build
    /// figure: a debug build (224 ms measured) prints the number and asserts nothing; run it with
    /// `cargo test --release -p kanbanr-core --lib position::tests::read_position_10k -- --nocapture`.
    #[test]
    fn read_position_10k_files_within_budget() {
        let d = repo("tenk");
        let mut n = 0u32;
        for dir in 0..100 {
            let folder = d.join(format!("d{dir:03}"));
            std::fs::create_dir_all(&folder).unwrap();
            for f in 0..100 {
                // 200 to 2,000 bytes, deterministic.
                let len = 200 + (n as usize * 37) % 1801;
                let body: String = (0..len)
                    .map(|i| char::from(b'a' + ((i as u32 + n) % 26) as u8))
                    .collect();
                std::fs::write(folder.join(format!("f{f:03}.txt")), body).unwrap();
                n += 1;
            }
        }
        git(&d, &["add", "-A"]).unwrap();
        git(&d, &["commit", "-q", "-m", "10k"]).unwrap();
        // Warm the file cache, then time three reads.
        let first = pos(&d);
        assert!(first.position.is_clean());
        let (median, all) = median_ms(&d);
        let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| "n/a".into());
        let tracked = git(&d, &["ls-files"]).unwrap().lines().count();
        println!(
            "FTR-003 R-10: {tracked} tracked files, median {median:.1} ms of {all:?} ms, load {}",
            load.trim()
        );
        assert_eq!(tracked, 10_001);
        let load1: f64 = load
            .split_whitespace()
            .next()
            .and_then(|l| l.parse().ok())
            .unwrap_or(0.0);
        #[allow(unused_variables)]
        let budget = if load1 > 4.0 {
            BUDGET_MS * 2.0
        } else {
            BUDGET_MS
        };
        #[cfg(not(debug_assertions))]
        assert!(
            median <= budget,
            "median {median:.1} ms over the {budget} ms budget (load {load1})"
        );
        assert_eq!(pos(&d), first, "repeats");
        // This repository, for scale (informational).
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if let Ok(here) = here.canonicalize()
            && here.join(".git").exists()
        {
            let files = git(&here, &["ls-files"]).map_or(0, |s| s.lines().count());
            let t = std::time::Instant::now();
            let ok = read_position(&here, "kanbanr").is_ok();
            println!(
                "FTR-003 R-10: this repository, {files} tracked files: {:.1} ms (ok: {ok})",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        gone(&d);
    }

    /// R-10: a view ID is computed within 100 microseconds (median of 1,000 computations).
    #[test]
    fn view_id_under_100_microseconds() {
        let ps = [
            PositionRef::at_commit("app", C),
            PositionRef::parse("zed", &format!("none+{D}")).unwrap(),
        ];
        let q = ViewQuery {
            op: "docs".into(),
            args: BTreeMap::from([
                ("max-nodes".to_string(), "60".to_string()),
                ("root".to_string(), "docs".to_string()),
            ]),
        };
        let mut us: Vec<f64> = (0..1000)
            .map(|_| {
                let t = std::time::Instant::now();
                let id = view_id(&ps, "0123abcd", &q);
                let e = t.elapsed().as_secs_f64() * 1e6;
                assert!(is_view_id(&id));
                e
            })
            .collect();
        us.sort_by(f64::total_cmp);
        let median = us[500];
        println!("FTR-003 R-10: view ID median {median:.1} us of 1000");
        #[cfg(not(debug_assertions))]
        assert!(median < 100.0, "median {median:.1} us");
    }
}
