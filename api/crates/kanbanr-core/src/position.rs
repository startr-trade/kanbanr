//! A repository's position: a commit plus a hash of what is uncommitted (SPK-001; chain decisions
//! CD-2 and CD-3 of katalogr FEAT-043, shared byte for byte with katalogr), and files read at a
//! position with nothing checked out.
//!
//! A spike's code: nothing here is reachable from the CLI, the daemon or the monitor yet. FTR-003
//! takes it over.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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
}
