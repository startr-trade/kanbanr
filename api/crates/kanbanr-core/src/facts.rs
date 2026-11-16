//! The project's facts (FTR-005): what the project is — its languages, runtimes, where it runs,
//! the parts it is made of and the data it keeps.
//!
//! The facts decide which rows of the content catalog apply: a library has no deployment of its
//! own, a project with no data has no data model. They also give the facts part of a kanbanr view
//! ID its second input ([`Facts::content_rev`], read by `views::project_facts`).
//!
//! Stored as a per-project side file (`facts.yaml`) beside `config.yaml`, like the charter: absent
//! means "no facts record", never an error, so a board written before this item behaves as it did.

use crate::Store;
use crate::error::{CoreError, Result};
use crate::hash::{canonical_json, stable_hash};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Per-project facts file, next to `config.yaml`.
pub const FACTS_FILE: &str = "facts.yaml";
/// The facts record's own schema version.
pub const SCHEMA: u32 = 1;

/// The longest text an `other:<text>` answer may carry after the colon.
const OTHER_MAX: usize = 40;
const OTHER_PREFIX: &str = "other:";

/// One question of the facts record and the words that answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vocabulary {
    pub key: &'static str,
    pub values: &'static [&'static str],
    pub means: &'static str,
}

/// The five questions, in the order the record lists them. Any key also takes `other:<text>`.
pub const VOCABULARY: [Vocabulary; 5] = [
    Vocabulary {
        key: "languages",
        values: &[
            "rust",
            "go",
            "python",
            "typescript",
            "javascript",
            "java",
            "kotlin",
            "csharp",
            "cpp",
            "c",
            "ruby",
            "php",
            "swift",
            "shell",
            "sql",
        ],
        means: "the languages the code is written in",
    },
    Vocabulary {
        key: "runtimes",
        values: &[
            "native",
            "jvm",
            "node",
            "browser",
            "python",
            "dotnet",
            "wasm",
            "container",
        ],
        means: "what the built code runs on",
    },
    Vocabulary {
        key: "production",
        values: &[
            "developer-machine",
            "server",
            "container",
            "kubernetes",
            "serverless",
            "browser",
            "mobile",
            "embedded",
            "library",
        ],
        means: "where the built thing runs",
    },
    Vocabulary {
        key: "parts",
        values: &[
            "cli",
            "api",
            "web-ui",
            "desktop-ui",
            "mobile-app",
            "library",
            "worker",
            "daemon",
        ],
        means: "the parts the project is made of",
    },
    Vocabulary {
        key: "data",
        values: &[
            "none",
            "files",
            "sql",
            "nosql",
            "object-store",
            "queue",
            "cache",
        ],
        means: "the data it keeps, and where",
    },
];

fn default_schema() -> u32 {
    SCHEMA
}

/// What the project is: the answers to the five [`VOCABULARY`] questions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Facts {
    #[serde(default = "default_schema")]
    pub schema: u32,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub runtimes: Vec<String>,
    #[serde(default)]
    pub production: Vec<String>,
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub data: Vec<String>,
    /// When the record was last saved; stamped by [`save`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub set_at: String,
    /// Keys this version does not know — written by a newer kanbanr — kept and written back
    /// unchanged, so an older binary never deletes what a newer one recorded (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

fn normalize_list(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = list.iter().map(|v| v.trim().to_lowercase()).collect();
    out.sort();
    out.dedup();
    out
}

impl Facts {
    /// The answers to one question; an unknown or empty key answers nothing.
    pub fn list(&self, key: &str) -> &[String] {
        match key {
            "languages" => &self.languages,
            "runtimes" => &self.runtimes,
            "production" => &self.production,
            "parts" => &self.parts,
            "data" => &self.data,
            _ => &[],
        }
    }

    /// Whether `key` holds `value`, both compared after normalisation.
    pub fn has(&self, key: &str, value: &str) -> bool {
        let value = value.trim().to_lowercase();
        self.list(key)
            .iter()
            .any(|v| v.trim().to_lowercase() == value)
    }

    /// Every list empty: the record answers nothing.
    pub fn is_empty(&self) -> bool {
        VOCABULARY.iter().all(|v| self.list(v.key).is_empty())
    }

    /// The record with every list trimmed, lower-cased, sorted and deduplicated; `set_at` and
    /// `extra` kept. A record that never named its schema gets this version's.
    pub fn normalized(&self) -> Facts {
        Facts {
            schema: if self.schema == 0 {
                SCHEMA
            } else {
                self.schema
            },
            languages: normalize_list(&self.languages),
            runtimes: normalize_list(&self.runtimes),
            production: normalize_list(&self.production),
            parts: normalize_list(&self.parts),
            data: normalize_list(&self.data),
            set_at: self.set_at.clone(),
            extra: self.extra.clone(),
        }
    }

    /// The record's content revision: a stable hash of its canonical form, normalised, without
    /// `set_at` and `extra`. The same answers give the same revision whatever their order, case
    /// or spacing, and whenever they were saved (R-2).
    pub fn content_rev(&self) -> String {
        let mut n = self.normalized();
        n.set_at = String::new();
        n.extra = Default::default();
        stable_hash(&canonical_json(&n))
    }
}

/// What is wrong with a facts record, each naming the key and value: a value outside the
/// vocabulary, an `other:` with no text or too long a text, a record that answers nothing.
pub fn problems(p: &Facts) -> Vec<String> {
    let n = p.normalized();
    let mut out = Vec::new();
    for vocab in &VOCABULARY {
        for value in n.list(vocab.key) {
            if let Some(text) = value.strip_prefix(OTHER_PREFIX) {
                let len = text.trim().chars().count();
                if len == 0 || len > OTHER_MAX {
                    out.push(format!(
                        "{}: \"{value}\" needs 1 to {OTHER_MAX} characters after \"{OTHER_PREFIX}\"",
                        vocab.key
                    ));
                }
            } else if !vocab.values.contains(&value.as_str()) {
                out.push(format!(
                    "{}: unknown value \"{value}\" (known: {}; or {OTHER_PREFIX}<text>)",
                    vocab.key,
                    vocab.values.join(", ")
                ));
            }
        }
    }
    if n.is_empty() {
        out.push("a facts record answers at least one question".to_string());
    }
    out
}

fn facts_path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(FACTS_FILE)
}

/// Load a project's facts record. An absent or blank file is `None` (no facts record yet, a normal
/// state); a file that is not a facts record is an error.
pub fn load(store: &Store, id: &str) -> Result<Option<Facts>> {
    match std::fs::read_to_string(facts_path(store, id)) {
        Ok(s) if s.trim().is_empty() => Ok(None),
        Ok(s) => Ok(Some(crate::yaml::from_str(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Save a project's facts record, replacing the whole record: refused with every problem named
/// when it has any, else normalised, stamped with `set_at` and written.
pub fn save(store: &Store, id: &str, p: &Facts) -> Result<Facts> {
    let found = problems(p);
    if !found.is_empty() {
        return Err(CoreError::Unsupported(found.join("; ")));
    }
    let mut facts = p.normalized();
    facts.set_at = crate::now_rfc3339();
    crate::atomic::write(facts_path(store, id), crate::yaml::to_string(&facts)?)?;
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn temp_store() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-facts-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        // L-41: the board is this test's own, under the system temp folder.
        assert!(store.data_dir().starts_with(std::env::temp_dir()));
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// R-1: save and load round-trip, with `other:` values and an unknown key kept in `extra`; an
    /// unknown value and an empty facts record are refused, naming the problem.
    #[test]
    fn a_facts_record_round_trips_and_refuses_unknown_values() {
        let (store, dir) = temp_store();
        assert_eq!(load(&store, "demo").unwrap(), None, "absent is no record");
        std::fs::write(facts_path(&store, "demo"), "  \n").unwrap();
        assert_eq!(load(&store, "demo").unwrap(), None, "blank is no record");

        let mut facts = Facts {
            languages: strings(&[" Rust", "typescript", "rust", "other:Zig"]),
            runtimes: strings(&["native", "browser"]),
            production: strings(&["developer-machine"]),
            parts: strings(&["cli", "daemon", "web-ui", "library"]),
            data: strings(&["files"]),
            ..Facts::default()
        };
        facts
            .extra
            .insert("hosting".into(), serde_json::json!("on-prem"));
        let saved = save(&store, "demo", &facts).unwrap();
        assert_eq!(saved.schema, SCHEMA);
        assert_eq!(
            saved.languages,
            strings(&["other:zig", "rust", "typescript"])
        );
        assert!(!saved.set_at.is_empty(), "save stamps set_at");
        assert!(saved.has("languages", "RUST "));
        assert!(!saved.has("languages", "go"));
        assert!(saved.list("").is_empty());
        assert!(saved.list("nope").is_empty());

        let text = std::fs::read_to_string(facts_path(&store, "demo")).unwrap();
        assert!(text.contains("schema: 1"), "{text}");
        assert!(
            text.contains("hosting: on-prem"),
            "unknown key written back: {text}"
        );
        let loaded = load(&store, "demo").unwrap().unwrap();
        assert_eq!(loaded, saved, "round-trips through yaml unchanged");

        // An unknown value is refused, naming its key and value; the file is unchanged.
        let bad = Facts {
            languages: strings(&["cobol"]),
            ..Facts::default()
        };
        let err = save(&store, "demo", &bad).unwrap_err().to_string();
        assert!(
            err.contains("languages") && err.contains("\"cobol\""),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(facts_path(&store, "demo")).unwrap(),
            text
        );

        // `other:` needs 1 to 40 characters.
        let other = Facts {
            data: strings(&["other:", &format!("other:{}", "x".repeat(41))]),
            ..Facts::default()
        };
        let found = problems(&other);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|p| p.starts_with("data:")), "{found:?}");
        assert!(
            problems(&Facts {
                data: strings(&[&format!("other:{}", "x".repeat(40))]),
                ..Facts::default()
            })
            .is_empty()
        );

        // An empty facts record answers nothing and is refused.
        assert!(Facts::default().is_empty());
        let err = save(&store, "demo", &Facts::default())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a facts record answers at least one question"),
            "{err}"
        );

        // Unreadable YAML is an error, not "no record".
        std::fs::write(facts_path(&store, "demo"), "languages: [unclosed\n").unwrap();
        assert!(load(&store, "demo").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-2: order, case, whitespace and `set_at` do not change the revision; a changed answer does.
    #[test]
    fn the_same_answers_give_the_same_rev() {
        let a = Facts {
            languages: strings(&["rust", "typescript"]),
            runtimes: strings(&["native"]),
            set_at: "2026-11-20T09:12:00Z".into(),
            ..Facts::default()
        };
        let b = Facts {
            schema: SCHEMA,
            languages: strings(&[" TypeScript ", "rust", "rust"]),
            runtimes: strings(&["Native"]),
            set_at: "2027-01-01T00:00:00Z".into(),
            ..Facts::default()
        };
        assert_eq!(a.content_rev(), b.content_rev());
        assert_eq!(a.content_rev().len(), 16);
        let mut c = a.clone();
        c.extra.insert("later".into(), serde_json::json!(1));
        assert_eq!(a.content_rev(), c.content_rev(), "extra is not an answer");
        let d = Facts {
            runtimes: strings(&["node"]),
            ..a.clone()
        };
        assert_ne!(
            a.content_rev(),
            d.content_rev(),
            "a changed answer changes it"
        );
        assert_ne!(
            Facts {
                languages: strings(&["rust"]),
                ..Facts::default()
            }
            .content_rev(),
            Facts {
                production: strings(&["rust"]),
                ..Facts::default()
            }
            .content_rev(),
            "the key an answer sits under counts"
        );
    }
}
