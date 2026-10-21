//! The board's YAML (FEAT-187): every board file is read and written here, so the library's
//! options are set in exactly one place.
//!
//! serde-saphyr is the library: maintained, and it and its parser forbid `unsafe`. It replaced
//! serde_yaml, whose C-to-Rust core (unsafe-libyaml) is unmaintained (RUSTSEC-2024-0449).
//!
//! Reading matches serde_yaml's: only `true`/`false` are booleans, and a repeated key keeps its last
//! value. Writing gives no `---`, `key:` followed by `- a` sequences at the same indent, `|`/`|-`
//! blocks for multi-line text, and never folds a long line. Its output is not byte-identical to
//! serde_yaml's (double quotes where serde_yaml used single), which is why a content revision is a
//! hash of canonical JSON rather than of YAML text ([`crate::hash::canonical_json`]).

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A value of any shape: what a key this version does not know holds (FEAT-151). A YAML value a
/// board holds always has a JSON form, so one type serves the board, the API and revisions.
pub type Value = serde_json::Value;

/// The parser's own error, for a caller that words it (a file a user handed in).
pub type Error = serde_saphyr::Error;

fn read_options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        strict_booleans: true,
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins,
        // One line, no source snippet: errors end up in CLI messages and HTTP bodies.
        with_snippet: false,
    }
}

fn write_options() -> serde_saphyr::SerializerOptions {
    serde_saphyr::ser_options! {
        yaml_12: true,
        no_lang_directive: true,
        folded_wrap_chars: usize::MAX,
    }
}

/// Parse board YAML, with the parser's error.
pub fn parse<T: DeserializeOwned>(s: &str) -> std::result::Result<T, Error> {
    serde_saphyr::from_str_with_options(s, read_options())
}

/// Read a board record from YAML (or JSON: YAML is a superset).
pub fn from_str<T: DeserializeOwned>(s: &str) -> crate::Result<T> {
    Ok(parse(s)?)
}

/// Write a board record as YAML, the way every board file is written.
pub fn to_string<T: Serialize + ?Sized>(v: &T) -> crate::Result<String> {
    Ok(serde_saphyr::to_string_with_options(&v, write_options())?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// The board kanbanr 0.1.7 made (FEAT-187 task 1), as committed.
    pub(crate) fn v017_board() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/v017/board")
    }

    fn yaml_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "dot-git") {
                    yaml_files(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                out.push(path);
            }
        }
    }

    /// FEAT-187 R-2: every board file, read and written again, holds what it held — unknown keys
    /// included. Compared as canonical JSON, so a value the parser reads differently from the way
    /// it writes (an octal-looking number, say) fails here, by file.
    #[test]
    fn board_files_round_trip() {
        let mut files = Vec::new();
        yaml_files(&v017_board(), &mut files);
        assert!(files.len() > 20, "the fixture board is missing: {files:?}");
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            let read: Value = from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let written = to_string(&read).unwrap();
            let again: Value = from_str(&written).unwrap();
            assert_eq!(
                crate::hash::canonical_json(&read),
                crate::hash::canonical_json(&again),
                "{} changed in a round trip:\n{written}",
                path.display()
            );
        }
        // The typed records too: an item read, written and read again is the same item.
        let mut features = 0;
        for path in files.iter().filter(|p| {
            p.parent()
                .and_then(|d| d.parent())
                .is_some_and(|d| d.ends_with("features"))
        }) {
            let meta: crate::models::FeatureMeta =
                from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            let back: crate::models::FeatureMeta = from_str(&to_string(&meta).unwrap()).unwrap();
            assert_eq!(
                crate::hash::canonical_json(&meta),
                crate::hash::canonical_json(&back),
                "{}",
                path.display()
            );
            features += 1;
        }
        assert_eq!(features, 14);
    }

    /// Reading is serde_yaml's: `yes` is a string, the last of two equal keys wins, and an error
    /// is one line.
    #[test]
    fn reading_matches_the_old_reader() {
        let v: Value = from_str("a: yes\nb: 1\nb: 2\nc: ~\n").unwrap();
        assert_eq!(v, serde_json::json!({"a": "yes", "b": 2, "c": null}));
        let err = from_str::<Value>("a: [1, 2\n").unwrap_err().to_string();
        assert!(!err.trim().contains('\n'), "{err}");
    }

    /// Writing: no document marker, sequences at the key's indent, block text, no folding.
    #[test]
    fn writing_is_plain_block_yaml() {
        let long = "word ".repeat(40);
        let text = to_string(&serde_json::json!({
            "list": ["a", "b"],
            "text": "one\ntwo",
            "long": long.trim(),
        }))
        .unwrap();
        assert!(!text.starts_with("---") && !text.starts_with('%'), "{text}");
        assert!(text.contains("list:\n- a\n- b\n"), "{text}");
        assert!(text.contains("text: |-\n  one\n  two\n"), "{text}");
        assert!(text.contains(&format!("long: {}\n", long.trim())), "{text}");
    }
}
