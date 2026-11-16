//! The content catalog (FTR-005): what a project documents, by question and perspective.
//!
//! A row says what a content answers (the Zachman questions), for whom (five perspectives), at
//! which level, in which notation, from which source (written, generated or enforced) and with
//! which protection against drift. Which rows apply follows the project's facts
//! ([`crate::facts`]); a row that does not apply says why.
//!
//! Stored as a per-project side file (`catalog.yaml`) beside `config.yaml`. Absent means "no
//! catalog": nothing reads or reports one (R-8). `kanbanr catalog init` copies the starter
//! ([`STARTER`]) and records the starter version it came from.

use crate::Store;
use crate::config::ProjectConfig;
use crate::error::{CoreError, Result};
use crate::facts::{Facts, VOCABULARY};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Per-project catalog file, next to `config.yaml`.
pub const CATALOG_FILE: &str = "catalog.yaml";
/// The catalog file's own schema version.
pub const SCHEMA: u32 = 1;
/// The starter catalog `catalog init` copies.
pub const STARTER: &str = include_str!("../presets/catalog.yaml");
/// The version of [`STARTER`]; a catalog records the one it began from.
pub const STARTER_VERSION: u32 = 1;

/// The questions a content answers: `trace::COLUMNS` in lower case, in the same order.
pub const QUESTIONS: [&str; 6] = ["what", "how", "where", "when", "who", "why"];

/// A reader of the system, each standing in for a Zachman row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perspective {
    pub id: &'static str,
    pub zachman: &'static str,
    pub means: &'static str,
}

/// The five perspectives, in plain words.
pub const PERSPECTIVES: [Perspective; 5] = [
    Perspective {
        id: "sponsor",
        zachman: "Executive (scope)",
        means: "why it exists, what it is for, its boundary",
    },
    Perspective {
        id: "user",
        zachman: "Business management",
        means: "what it does for whom, in which processes",
    },
    Perspective {
        id: "architect",
        zachman: "Architect (system logic)",
        means: "its structure: context, parts, data, interfaces, decisions",
    },
    Perspective {
        id: "builder",
        zachman: "Engineer and Technician (technology, components)",
        means: "its code: modules, dependencies, tests",
    },
    Perspective {
        id: "operator",
        zachman: "Enterprise (operations)",
        means: "where and how it runs: deployment, configuration, releases",
    },
];

/// The views kanbanr derives, as a generated source names them.
pub const GENERATORS: &[&str] = &[
    "kanbanr:charter",
    "kanbanr:zachman",
    "kanbanr:adr-index",
    "kanbanr:trace",
    "kanbanr:requirements",
    "kanbanr:releases",
    "kanbanr:gantt",
    "kanbanr:evidence",
    "kanbanr:component-list",
];

/// The protections a row may name. A name ending in `:` takes an argument (`coverage:<id>`,
/// `test:<name>`).
pub const PROTECTIONS: &[&str] = &[
    "docs_updated",
    "coverage:",
    "drift",
    "spec_sections",
    "design-review",
    "view-id",
    "test:",
    "none",
];

/// The prefix of a written source read from the code repository.
pub const REPO_PREFIX: &str = "repo:";
const KATALOGR_PREFIX: &str = "katalogr:";
/// The `when` key read from the config rather than the facts record.
const CONFIG_KEY: &str = "config";

/// What a row documents: the whole system, each component, or each item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    System,
    Component,
    Item,
}

/// The three kinds a realized design becomes (ADR-0014 point 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    Written,
    Generated,
    Enforced,
}

/// Where a row's content comes from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Source {
    #[serde(default)]
    pub kind: SourceKind,
    /// Written: a board doc, a board folder ending `/`, or `repo:<path>`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Generated: `kanbanr:<view>` or `katalogr:<op>`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub generator: String,
    /// Enforced: the test or rule that keeps the content true.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test: String,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// One content the project documents.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Row {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The questions it answers ([`QUESTIONS`]).
    #[serde(default)]
    pub answers: Vec<String>,
    /// The perspectives it answers them for ([`PERSPECTIVES`]).
    #[serde(default, rename = "for")]
    pub for_: Vec<String>,
    #[serde(default)]
    pub level: Level,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notation: String,
    #[serde(default)]
    pub source: Source,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protection: Vec<String>,
    /// When the row applies: each key must meet the facts record's list for it (`config` reads
    /// the config's cadence). Absent: always.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub when: BTreeMap<String, Vec<String>>,
    /// An override of `when`, with `why`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

fn default_schema() -> u32 {
    SCHEMA
}

/// A project's catalog: its rows and the starter version they began from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Catalog {
    #[serde(default = "default_schema")]
    pub schema: u32,
    /// The starter version the rows were taken from; 0 when they were not.
    #[serde(default)]
    pub starter: u32,
    #[serde(default)]
    pub rows: Vec<Row>,
    /// Keys this version does not know, kept and written back (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl Catalog {
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// Whether a row applies, why not, and what decided it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Applicability {
    pub applies: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// `always` (no `when`), `facts` (the `when` clause decided) or `override` (`applies:`).
    pub by: &'static str,
}

impl Applicability {
    fn new(applies: bool, why: impl Into<String>, by: &'static str) -> Self {
        Applicability {
            applies,
            why: why.into(),
            by,
        }
    }
}

/// The text a row with a `when` reads when the project has no facts record.
pub const NO_FACTS: &str = "no project facts: `kanbanr facts set …`";

impl Row {
    /// Whether the row applies to this project: an explicit `applies` wins; else every key of
    /// `when` must meet the facts record (or, for `config`, the config's cadence); no `when`
    /// always applies.
    pub fn applicability(&self, facts: Option<&Facts>, config: &ProjectConfig) -> Applicability {
        match self.applies {
            Some(false) => return Applicability::new(false, self.why.clone(), "override"),
            Some(true) => return Applicability::new(true, self.why.clone(), "override"),
            None => {}
        }
        if self.when.is_empty() {
            return Applicability::new(true, "", "always");
        }
        for (key, wanted) in &self.when {
            if key == CONFIG_KEY {
                let on = wanted.iter().any(|v| match v.trim() {
                    "releases" => config.cadence.releases,
                    "sprints" => config.cadence.sprints,
                    _ => false,
                });
                if !on {
                    return Applicability::new(
                        false,
                        format!("the config does not switch on {}", wanted.join(" or ")),
                        "facts",
                    );
                }
                continue;
            }
            let Some(facts) = facts else {
                return Applicability::new(false, NO_FACTS, "facts");
            };
            let have = facts.list(key);
            if have.is_empty() {
                return Applicability::new(
                    false,
                    format!("the facts record does not say its {key}"),
                    "facts",
                );
            }
            if !wanted.iter().any(|v| facts.has(key, v)) {
                return Applicability::new(
                    false,
                    format!(
                        "its {key} is {}; the row applies to {}",
                        have.join(", "),
                        wanted.join(", ")
                    ),
                    "facts",
                );
            }
        }
        Applicability::new(true, "", "facts")
    }

    /// The (question, perspective) cells the row fills: each of `answers` with each of `for`.
    pub fn cells(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for q in &self.answers {
            for p in &self.for_ {
                out.push((q.clone(), p.clone()));
            }
        }
        out
    }

    /// Where the monitor shows the row's source: a board doc's page, a board folder on the Docs
    /// page, or the page of a generated view that has one. A repository doc, an enforced source
    /// and a view with no page have none.
    pub fn href(&self, project: &str) -> Option<String> {
        match self.source.kind {
            SourceKind::Written => {
                let path = self.source.path.trim();
                if path.is_empty() || path.starts_with(REPO_PREFIX) {
                    None
                } else if let Some(folder) = path.strip_suffix('/') {
                    Some(format!("/p/{project}/docs/folder/{folder}"))
                } else {
                    Some(format!("/p/{project}/docs/file/{path}"))
                }
            }
            SourceKind::Generated => {
                let page = match self.source.generator.trim() {
                    // The charter page shows the goals and the work behind each: the trace.
                    "kanbanr:charter" | "kanbanr:trace" => "charter",
                    "kanbanr:releases" => "releases",
                    "kanbanr:gantt" => "gantt",
                    _ => return None,
                };
                Some(format!("/p/{project}/{page}"))
            }
            SourceKind::Enforced => None,
        }
    }
}

/// The starter catalog, parsed. A unit test keeps it valid.
pub fn starter() -> Catalog {
    parse(STARTER).expect("the starter catalog parses")
}

/// Read a catalog from YAML: a mapping with `rows`, or a bare list of rows.
pub fn parse(text: &str) -> Result<Catalog> {
    match crate::yaml::parse::<Catalog>(text) {
        Ok(c) => Ok(c),
        Err(mapping_err) => match crate::yaml::parse::<Vec<Row>>(text) {
            Ok(rows) => Ok(Catalog {
                schema: SCHEMA,
                rows,
                ..Catalog::default()
            }),
            Err(_) => Err(mapping_err.into()),
        },
    }
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_op(op: &str) -> bool {
    let mut chars = op.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c == '-')
}

/// Why a relative path cannot be read, if it cannot: empty, absolute, or climbing with `..`.
fn path_problem(path: &str) -> Option<&'static str> {
    if path.trim().is_empty() {
        return Some("an empty path");
    }
    let absolute = path.starts_with('/')
        || path.starts_with('\\')
        || std::path::Path::new(path).is_absolute()
        || path.chars().nth(1) == Some(':');
    if absolute {
        return Some("an absolute path");
    }
    if path.split(['/', '\\']).any(|part| part == "..") {
        return Some("a path with '..'");
    }
    None
}

fn known_protection(name: &str) -> bool {
    PROTECTIONS.iter().any(|p| match p.strip_suffix(':') {
        Some(_) => name.starts_with(p),
        None => name == *p,
    })
}

/// What is wrong with a catalog, each naming its row.
pub fn problems(c: &Catalog) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let facts_keys: Vec<&str> = VOCABULARY.iter().map(|v| v.key).collect();
    for (i, row) in c.rows.iter().enumerate() {
        let name = if row.id.trim().is_empty() {
            format!("row {}", i + 1)
        } else {
            format!("row \"{}\"", row.id)
        };
        if row.id.trim().is_empty() {
            out.push(format!("{name}: an empty id"));
        } else if !valid_id(&row.id) {
            out.push(format!(
                "{name}: the id must be lower-case letters, digits and '-', starting with a letter or digit"
            ));
        } else if !seen.insert(row.id.clone()) {
            out.push(format!("{name}: a duplicate id"));
        }
        if row.answers.is_empty() {
            out.push(format!("{name}: answers no question"));
        }
        for q in &row.answers {
            if !QUESTIONS.contains(&q.as_str()) {
                out.push(format!(
                    "{name}: \"{q}\" is not a question (use {})",
                    QUESTIONS.join(", ")
                ));
            }
        }
        if row.for_.is_empty() {
            out.push(format!("{name}: is for no perspective"));
        }
        for p in &row.for_ {
            if !PERSPECTIVES.iter().any(|x| x.id == p) {
                out.push(format!(
                    "{name}: \"{p}\" is not a perspective (use {})",
                    PERSPECTIVES.map(|x| x.id).join(", ")
                ));
            }
        }
        match row.source.kind {
            SourceKind::Written => {
                let path = row.source.path.trim();
                let path = path.strip_prefix(REPO_PREFIX).unwrap_or(path);
                // An item-level row's source is each item's own spec `## Design`: it names no
                // path (coordinator, 17 Nov 2026). Any other level needs one.
                let exempt = row.level == Level::Item && row.source.path.trim().is_empty();
                if !exempt && let Some(why) = path_problem(path) {
                    out.push(format!("{name}: a written source with {why}"));
                }
            }
            SourceKind::Generated => {
                let g = row.source.generator.trim();
                if g.is_empty() {
                    out.push(format!("{name}: a generated source with no generator"));
                } else if !GENERATORS.contains(&g)
                    && !g.strip_prefix(KATALOGR_PREFIX).is_some_and(valid_op)
                {
                    out.push(format!(
                        "{name}: \"{g}\" is neither a kanbanr view ({}) nor katalogr:<op>",
                        GENERATORS.join(", ")
                    ));
                }
            }
            SourceKind::Enforced => {
                if row.source.test.trim().is_empty() {
                    out.push(format!("{name}: an enforced source with no test"));
                }
            }
        }
        for p in &row.protection {
            if !known_protection(p) {
                out.push(format!(
                    "{name}: \"{p}\" is not a protection (use {})",
                    PROTECTIONS.join(", ")
                ));
            } else if p
                .strip_prefix("coverage:")
                .is_some_and(|id| id.trim().is_empty())
            {
                out.push(format!("{name}: \"coverage:\" names no coverage rule"));
            }
        }
        for key in row.when.keys() {
            if key != CONFIG_KEY && !facts_keys.contains(&key.as_str()) {
                out.push(format!(
                    "{name}: \"{key}\" is not a facts key or config (use {}, config)",
                    facts_keys.join(", ")
                ));
            }
        }
        if row.applies == Some(false) && row.why.trim().is_empty() {
            out.push(format!("{name}: switched off with no reason"));
        }
    }
    out
}

fn catalog_path(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(CATALOG_FILE)
}

/// Load a project's catalog. Absent or blank is `None`; a file that does not parse is an error.
pub fn load(store: &Store, id: &str) -> Result<Option<Catalog>> {
    match std::fs::read_to_string(catalog_path(store, id)) {
        Ok(s) if s.trim().is_empty() => Ok(None),
        Ok(s) => Ok(Some(parse(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Save a project's catalog, refused with every problem named when it has any.
pub fn save(store: &Store, id: &str, c: &Catalog) -> Result<Catalog> {
    let found = problems(c);
    if !found.is_empty() {
        return Err(CoreError::Unsupported(found.join("; ")));
    }
    let mut c = c.clone();
    if c.schema == 0 {
        c.schema = SCHEMA;
    }
    crate::atomic::write(catalog_path(store, id), crate::yaml::to_string(&c)?)?;
    Ok(c)
}

/// Write the starter catalog. Refused when the project has a catalog file, unless `force`.
pub fn init(store: &Store, id: &str, force: bool) -> Result<Catalog> {
    if !force && catalog_path(store, id).exists() {
        return Err(CoreError::Unsupported(
            "the project already has a catalog: `kanbanr catalog init --force` replaces it".into(),
        ));
    }
    save(store, id, &starter())
}

fn load_existing(store: &Store, id: &str) -> Result<Catalog> {
    load(store, id)?.ok_or_else(|| {
        CoreError::Unsupported("the project has no catalog: `kanbanr catalog init`".into())
    })
}

/// Copy one starter row into the project's catalog. Refused for a row it already has or one the
/// starter does not hold.
pub fn add_row(store: &Store, id: &str, row_id: &str) -> Result<Catalog> {
    let mut c = load_existing(store, id)?;
    if c.row(row_id).is_some() {
        return Err(CoreError::Unsupported(format!(
            "the catalog already has row \"{row_id}\""
        )));
    }
    let starter = starter();
    let Some(row) = starter.row(row_id) else {
        let ids: Vec<&str> = starter.rows.iter().map(|r| r.id.as_str()).collect();
        return Err(CoreError::Unsupported(format!(
            "the starter catalog has no row \"{row_id}\" (it has {})",
            ids.join(", ")
        )));
    };
    c.rows.push(row.clone());
    save(store, id, &c)
}

/// Switch a row off (with a reason) or back on. On clears the override, so the row's `when`
/// decides again.
pub fn set_row(store: &Store, id: &str, row_id: &str, applies: bool, why: &str) -> Result<Catalog> {
    let mut c = load_existing(store, id)?;
    let Some(row) = c.rows.iter_mut().find(|r| r.id == row_id) else {
        return Err(CoreError::Unsupported(format!(
            "the catalog has no row \"{row_id}\""
        )));
    };
    if applies {
        row.applies = None;
        row.why = String::new();
    } else {
        if why.trim().is_empty() {
            return Err(CoreError::Unsupported(format!(
                "switching row \"{row_id}\" off needs a reason: --why \"…\""
            )));
        }
        row.applies = Some(false);
        row.why = why.trim().to_string();
    }
    save(store, id, &c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-catalog-{}-{}",
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

    fn row(id: &str) -> Row {
        Row {
            id: id.into(),
            title: id.into(),
            answers: vec!["what".into()],
            for_: vec!["architect".into()],
            source: Source {
                kind: SourceKind::Written,
                path: format!("architecture/{id}.md"),
                ..Source::default()
            },
            protection: vec!["docs_updated".into()],
            ..Row::default()
        }
    }

    fn one(r: Row) -> Vec<String> {
        problems(&Catalog {
            schema: SCHEMA,
            rows: vec![r],
            ..Catalog::default()
        })
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// R-4: both YAML forms parse; each `problems` rule fires with its row named.
    #[test]
    fn rows_parse_and_validate() {
        let mapping = "schema: 1\nstarter: 1\nrows:\n  - id: context\n    title: System context\n    answers: [what, where]\n    for: [sponsor, architect]\n    level: system\n    notation: mermaid\n    source: {kind: written, path: architecture/context.md}\n    protection: [docs_updated]\n    later: kept\n";
        let c = parse(mapping).unwrap();
        assert_eq!(c.starter, 1);
        assert_eq!(c.rows.len(), 1);
        assert_eq!(c.rows[0].for_, strings(&["sponsor", "architect"]));
        assert_eq!(c.rows[0].extra["later"], serde_json::json!("kept"));
        assert!(problems(&c).is_empty(), "{:?}", problems(&c));
        let text = crate::yaml::to_string(&c).unwrap();
        assert!(
            text.contains("for:") && text.contains("later: kept"),
            "{text}"
        );

        let list = "- id: tests\n  answers: [how]\n  for: [builder]\n  source: {kind: generated, generator: \"kanbanr:evidence\"}\n";
        let c = parse(list).unwrap();
        assert_eq!(c.schema, SCHEMA);
        assert_eq!(c.starter, 0);
        assert_eq!(c.rows[0].level, Level::System);
        assert!(problems(&c).is_empty(), "{:?}", problems(&c));
        assert!(parse("rows: [unclosed").is_err());

        let fires = |r: Row, needle: &str| {
            let found = one(r);
            assert!(
                found.iter().any(|p| p.contains(needle)),
                "{needle} not in {found:?}"
            );
        };
        // ids
        fires(
            Row {
                id: String::new(),
                ..row("x")
            },
            "row 1: an empty id",
        );
        fires(
            Row {
                id: "Bad id".into(),
                ..row("x")
            },
            "row \"Bad id\": the id must be",
        );
        let dup = problems(&Catalog {
            rows: vec![row("a"), row("a")],
            ..Catalog::default()
        });
        assert_eq!(dup, vec!["row \"a\": a duplicate id"]);
        // questions and perspectives
        fires(
            Row {
                answers: strings(&["which"]),
                ..row("q")
            },
            "row \"q\": \"which\" is not a question",
        );
        fires(
            Row {
                answers: vec![],
                ..row("q")
            },
            "row \"q\": answers no question",
        );
        fires(
            Row {
                for_: strings(&["manager"]),
                ..row("p")
            },
            "row \"p\": \"manager\" is not a perspective",
        );
        fires(
            Row {
                for_: vec![],
                ..row("p")
            },
            "row \"p\": is for no perspective",
        );
        // written sources
        let written = |path: &str, level: Level| Row {
            level,
            source: Source {
                kind: SourceKind::Written,
                path: path.into(),
                ..Source::default()
            },
            ..row("w")
        };
        fires(
            written("", Level::System),
            "row \"w\": a written source with an empty path",
        );
        fires(
            written("", Level::Component),
            "a written source with an empty path",
        );
        fires(written("/etc/passwd", Level::System), "an absolute path");
        fires(
            written("repo:/etc/passwd", Level::System),
            "an absolute path",
        );
        fires(written("../outside.md", Level::System), "a path with '..'");
        fires(
            written("repo:docs/../../x.md", Level::System),
            "a path with '..'",
        );
        assert!(one(written("repo:docs/system/context.md", Level::System)).is_empty());
        assert!(one(written("architecture/components/", Level::Component)).is_empty());
        // An item-level written row names no path: each item's spec is its source.
        assert!(one(written("", Level::Item)).is_empty());
        fires(written("../x.md", Level::Item), "a path with '..'");
        // generated and enforced sources
        let generated = |g: &str| Row {
            source: Source {
                kind: SourceKind::Generated,
                generator: g.into(),
                ..Source::default()
            },
            ..row("g")
        };
        fires(
            generated(""),
            "row \"g\": a generated source with no generator",
        );
        fires(generated("kanbanr:nope"), "\"kanbanr:nope\" is neither");
        fires(
            generated("katalogr:Bad_Op"),
            "\"katalogr:Bad_Op\" is neither",
        );
        fires(generated("katalogr:"), "is neither");
        assert!(one(generated("katalogr:structure")).is_empty());
        assert!(one(generated("katalogr:some-future-op")).is_empty());
        assert!(one(generated("kanbanr:zachman")).is_empty());
        fires(
            Row {
                source: Source {
                    kind: SourceKind::Enforced,
                    ..Source::default()
                },
                ..row("e")
            },
            "row \"e\": an enforced source with no test",
        );
        // protections
        fires(
            Row {
                protection: strings(&["vibes"]),
                ..row("pr")
            },
            "\"vibes\" is not a protection",
        );
        fires(
            Row {
                protection: strings(&["coverage:"]),
                ..row("pr")
            },
            "\"coverage:\" names no coverage rule",
        );
        assert!(
            one(Row {
                protection: strings(&[
                    "coverage:catalog",
                    "test:a::b",
                    "none",
                    "drift",
                    "spec_sections",
                    "design-review",
                    "view-id"
                ]),
                ..row("pr")
            })
            .is_empty()
        );
        // when keys and overrides
        let mut when = BTreeMap::new();
        when.insert("colour".to_string(), strings(&["blue"]));
        fires(
            Row { when, ..row("wh") },
            "\"colour\" is not a facts key or config",
        );
        let mut when = BTreeMap::new();
        when.insert("config".to_string(), strings(&["releases"]));
        when.insert("parts".to_string(), strings(&["api"]));
        assert!(one(Row { when, ..row("wh") }).is_empty());
        fires(
            Row {
                applies: Some(false),
                ..row("off")
            },
            "row \"off\": switched off with no reason",
        );
        assert!(
            one(Row {
                applies: Some(false),
                why: "not ours".into(),
                ..row("off")
            })
            .is_empty()
        );
    }

    /// R-5: the starter has no problems, its ids are unique, and every question has a row that
    /// always applies.
    #[test]
    fn the_starter_catalog_is_valid_and_every_question_has_a_row() {
        let c = starter();
        assert_eq!(c.starter, STARTER_VERSION);
        assert_eq!(c.schema, SCHEMA);
        assert!(problems(&c).is_empty(), "{:?}", problems(&c));
        assert_eq!(c.rows.len(), 21);
        let ids: BTreeSet<&str> = c.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), c.rows.len(), "ids are unique");
        let config = ProjectConfig::default_for("demo");
        for q in QUESTIONS {
            assert!(
                c.rows.iter().any(|r| r.answers.iter().any(|a| a == q)
                    && r.applicability(None, &config).by == "always"),
                "no always-applying row answers {q}"
            );
        }
        assert_eq!(
            c.row("code-structure").unwrap().source.generator,
            "katalogr:structure"
        );
        // The questions are trace's columns, lower-cased, in order.
        let columns: Vec<String> = crate::trace::COLUMNS
            .iter()
            .map(|c| c.to_lowercase())
            .collect();
        assert_eq!(columns, QUESTIONS.to_vec());
        // Every perspective is read by some row.
        for p in PERSPECTIVES {
            assert!(
                c.rows.iter().any(|r| r.for_.iter().any(|f| f == p.id)),
                "{}",
                p.id
            );
        }
    }

    /// R-6: applicability follows the facts record's lists, `config:` keys and overrides each way.
    #[test]
    fn applicability_follows_the_facts_and_overrides() {
        let c = starter();
        let mut config = ProjectConfig::default_for("demo");
        let deployment = c.row("deployment").unwrap();
        let interfaces = c.row("interfaces").unwrap();
        let context = c.row("context").unwrap();

        // No `when`: always, facts or not.
        assert_eq!(
            context.applicability(None, &config),
            Applicability::new(true, "", "always")
        );
        // No facts record: a `when` row reads as not applying, naming the gap.
        assert_eq!(
            deployment.applicability(None, &config),
            Applicability::new(false, NO_FACTS, "facts")
        );
        // A matching facts record applies; a non-matching one says why.
        let server = Facts {
            production: vec!["Server".into()],
            ..Facts::default()
        };
        assert_eq!(
            deployment.applicability(Some(&server), &config),
            Applicability::new(true, "", "facts")
        );
        let library = Facts {
            production: vec!["library".into()],
            parts: vec!["library".into()],
            ..Facts::default()
        };
        let a = deployment.applicability(Some(&library), &config);
        assert!(!a.applies && a.by == "facts", "{a:?}");
        assert!(a.why.contains("production is library"), "{a:?}");
        assert!(interfaces.applicability(Some(&library), &config).applies);
        // A key the facts record does not answer reads as not applying.
        assert_eq!(
            interfaces.applicability(Some(&server), &config),
            Applicability::new(false, "the facts record does not say its parts", "facts")
        );
        // `config:` keys read the cadence, not the facts record.
        let releases = c.row("releases").unwrap();
        let off = releases.applicability(Some(&server), &config);
        assert!(!off.applies && off.why.contains("releases"), "{off:?}");
        config.cadence.releases = true;
        assert_eq!(
            releases.applicability(Some(&server), &config),
            Applicability::new(true, "", "facts")
        );
        let mut sprint_row = releases.clone();
        sprint_row
            .when
            .insert("config".into(), vec!["sprints".into()]);
        assert!(!sprint_row.applicability(Some(&server), &config).applies);
        config.cadence.sprints = true;
        assert!(sprint_row.applicability(Some(&server), &config).applies);
        // Overrides win each way.
        let mut forced_off = context.clone();
        forced_off.applies = Some(false);
        forced_off.why = "the README is the context".into();
        assert_eq!(
            forced_off.applicability(Some(&server), &config),
            Applicability::new(false, "the README is the context", "override")
        );
        let mut forced_on = deployment.clone();
        forced_on.applies = Some(true);
        assert_eq!(
            forced_on.applicability(Some(&library), &config),
            Applicability::new(true, "", "override")
        );
        // Cells and links.
        assert_eq!(
            context.cells(),
            vec![
                ("what".to_string(), "sponsor".to_string()),
                ("what".to_string(), "architect".to_string()),
                ("where".to_string(), "sponsor".to_string()),
                ("where".to_string(), "architect".to_string()),
            ]
        );
        assert_eq!(
            context.href("p").as_deref(),
            Some("/p/p/docs/file/architecture/context.md")
        );
        assert_eq!(
            c.row("component-docs").unwrap().href("p").as_deref(),
            Some("/p/p/docs/folder/architecture/components")
        );
        assert_eq!(
            c.row("timeline").unwrap().href("p").as_deref(),
            Some("/p/p/gantt")
        );
        assert_eq!(c.row("code-structure").unwrap().href("p"), None);
        assert_eq!(c.row("item-design").unwrap().href("p"), None);
        let mut repo = context.clone();
        repo.source.path = "repo:docs/system/context.md".into();
        assert_eq!(repo.href("p"), None);
    }

    /// R-6: switching a row off needs a reason; switching it on clears the override. Also `init`
    /// and `add_row`.
    #[test]
    fn switching_a_row_off_needs_a_reason() {
        let (store, dir) = temp_store();
        assert_eq!(load(&store, "demo").unwrap(), None);
        assert!(
            set_row(&store, "demo", "context", false, "x").is_err(),
            "no catalog yet"
        );
        let written = init(&store, "demo", false).unwrap();
        assert_eq!(written, starter());
        assert_eq!(load(&store, "demo").unwrap(), Some(starter()));
        assert!(
            init(&store, "demo", false).is_err(),
            "a second init refuses"
        );

        let err = set_row(&store, "demo", "deployment", false, "  ").unwrap_err();
        assert!(err.to_string().contains("needs a reason"), "{err}");
        assert!(
            load(&store, "demo")
                .unwrap()
                .unwrap()
                .row("deployment")
                .unwrap()
                .applies
                .is_none()
        );
        let c = set_row(
            &store,
            "demo",
            "deployment",
            false,
            " a library has no deployment ",
        )
        .unwrap();
        let r = c.row("deployment").unwrap();
        assert_eq!(r.applies, Some(false));
        assert_eq!(r.why, "a library has no deployment");
        assert_eq!(load(&store, "demo").unwrap().unwrap(), c);
        let c = set_row(&store, "demo", "deployment", true, "ignored").unwrap();
        let r = c.row("deployment").unwrap();
        assert_eq!(r.applies, None);
        assert_eq!(r.why, "");
        assert!(set_row(&store, "demo", "nope", true, "").is_err());

        // add_row copies a starter row; refuses a duplicate or an unknown id.
        let mut fewer = starter();
        fewer.rows.retain(|r| r.id != "security");
        save(&store, "demo", &fewer).unwrap();
        let c = add_row(&store, "demo", "security").unwrap();
        assert_eq!(c.row("security"), starter().row("security"));
        assert!(add_row(&store, "demo", "security").is_err());
        assert!(add_row(&store, "demo", "nope").is_err());

        // save refuses an invalid catalog and leaves the file alone; init --force rewrites.
        let before = std::fs::read_to_string(catalog_path(&store, "demo")).unwrap();
        let mut bad = c.clone();
        bad.rows[0].answers = vec!["which".into()];
        assert!(save(&store, "demo", &bad).is_err());
        assert_eq!(
            std::fs::read_to_string(catalog_path(&store, "demo")).unwrap(),
            before
        );
        assert_eq!(init(&store, "demo", true).unwrap(), starter());
        let _ = std::fs::remove_dir_all(dir);
    }
}
