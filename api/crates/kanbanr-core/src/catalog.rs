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

// ---- the derived status (T3) ------------------------------------------------------------------

/// A doc body (after its front matter) with fewer non-blank characters than this reads as a stub:
/// a doc holding only front matter and a heading is one.
pub const STUB_THRESHOLD: usize = 80;

/// What a row's source holds right now, derived from the board, the config and the code
/// repository the surface has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// A written doc with a body.
    Present,
    /// A written doc whose body is under [`STUB_THRESHOLD`] non-blank characters.
    Stub,
    /// A written doc (or folder) that is not there.
    Missing,
    /// A written folder and the docs under it (0 counts as missing for the gaps).
    Folder { docs: usize },
    /// A generated kanbanr view: always derivable.
    Available,
    /// A `katalogr:` view: it needs katalogr (FTR-016 reports the capability).
    NeedsKatalogr,
    /// An enforced source, or an item-level written row with no path: the row declares it.
    Declared,
    /// A `repo:` source on a surface with no code repository: it cannot tell (not a gap).
    NoRepository,
}

impl SourceState {
    /// Whether the source fills the cells its row answers: a missing source, an empty folder and a
    /// stub do not.
    fn fills(&self) -> bool {
        !matches!(
            self,
            SourceState::Missing | SourceState::Stub | SourceState::Folder { docs: 0 }
        )
    }
}

/// One protection of a row and whether the project has it in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProtectionStatus {
    pub name: String,
    pub in_force: bool,
    /// What decides it, or what is missing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// One row, with its derived state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RowStatus {
    pub id: String,
    pub title: String,
    pub level: Level,
    pub answers: Vec<String>,
    #[serde(rename = "for")]
    pub for_: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notation: String,
    pub source: Source,
    pub state: SourceState,
    pub applies: Applicability,
    pub protections: Vec<ProtectionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

/// A (question, perspective) pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Cell {
    pub question: String,
    pub perspective: String,
}

/// The catalog as it stands: every row's state, the cells no applying row fills, and what is
/// wrong with the catalog file. Derived, never stored (ADR-0002).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CatalogStatus {
    /// The facts record's content revision; empty with no facts record.
    pub facts_rev: String,
    /// The starter version the catalog began from; 0 with none.
    pub starter: u32,
    pub rows: Vec<RowStatus>,
    pub gaps: Vec<Cell>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// How many source docs were read (each written source once).
    pub docs_read: usize,
}

/// A board folder source as a docs-relative prefix ending `/`.
fn folder_prefix(path: &str) -> String {
    let p = path.trim().trim_start_matches("./").trim_start_matches('/');
    let p = p.strip_prefix("docs/").unwrap_or(p);
    let p = p.trim_end_matches('/');
    format!("{p}/")
}

fn body_state(text: &str) -> SourceState {
    let (_, body) = crate::docs::split_front_matter(text);
    let chars = body.chars().filter(|c| !c.is_whitespace()).count();
    if chars < STUB_THRESHOLD {
        SourceState::Stub
    } else {
        SourceState::Present
    }
}

/// Reads each source once, in the order rows ask for them.
struct Reader<'a> {
    store: &'a Store,
    project: &'a str,
    repo: Option<&'a std::path::Path>,
    /// Board docs (by key) and repository docs (by `repo:<path>`) already judged.
    seen: BTreeMap<String, SourceState>,
    listing: Option<Vec<String>>,
    reads: usize,
}

impl Reader<'_> {
    fn state(&mut self, row: &Row) -> SourceState {
        match row.source.kind {
            SourceKind::Generated => {
                if row.source.generator.trim().starts_with(KATALOGR_PREFIX) {
                    SourceState::NeedsKatalogr
                } else {
                    SourceState::Available
                }
            }
            SourceKind::Enforced => SourceState::Declared,
            SourceKind::Written => {
                let path = row.source.path.trim();
                if path.is_empty() {
                    // An item-level row's source is each item's spec (coordinator, 17 Nov 2026).
                    return if row.level == Level::Item {
                        SourceState::Declared
                    } else {
                        SourceState::Missing
                    };
                }
                if let Some(rel) = path.strip_prefix(REPO_PREFIX) {
                    return self.repo_doc(rel);
                }
                if path.ends_with('/') {
                    let prefix = folder_prefix(path);
                    let listing = self.listing.get_or_insert_with(|| {
                        self.store.list_docs(self.project).unwrap_or_default()
                    });
                    let docs = listing.iter().filter(|d| d.starts_with(&prefix)).count();
                    return SourceState::Folder { docs };
                }
                self.board_doc(path)
            }
        }
    }

    fn board_doc(&mut self, path: &str) -> SourceState {
        if path_problem(path).is_some() {
            return SourceState::Missing;
        }
        let key = crate::refs::doc_key(path);
        if let Some(s) = self.seen.get(&key) {
            return s.clone();
        }
        self.reads += 1;
        let state = match self.store.read_doc(self.project, &key) {
            Ok(text) => body_state(&text),
            Err(_) => SourceState::Missing,
        };
        self.seen.insert(key, state.clone());
        state
    }

    fn repo_doc(&mut self, rel: &str) -> SourceState {
        let Some(root) = self.repo else {
            return SourceState::NoRepository;
        };
        if path_problem(rel).is_some() {
            return SourceState::Missing;
        }
        let key = format!("{REPO_PREFIX}{rel}");
        if let Some(s) = self.seen.get(&key) {
            return s.clone();
        }
        self.reads += 1;
        let state = repo_file(root, rel)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| body_state(&text))
            .unwrap_or(SourceState::Missing);
        self.seen.insert(key, state.clone());
        state
    }
}

/// `rel` under `root`, only when it resolves inside it (a symlink out of the repository is refused).
fn repo_file(root: &std::path::Path, rel: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = root.join(rel).canonicalize().ok()?;
    (path.starts_with(&root) && path.is_file()).then_some(path)
}

fn protection_status(
    name: &str,
    row: &Row,
    project: &crate::store::Project,
    tracked: &BTreeSet<&str>,
    repo: bool,
) -> ProtectionStatus {
    use crate::method::Part;
    let config = &project.config;
    let part = |p: Part, what: &str| {
        if config.method.is_on(p) {
            (true, String::new())
        } else {
            (
                false,
                format!("the method's {} part is off: {what}", p.as_str()),
            )
        }
    };
    let (in_force, why) = match name {
        "docs_updated" => part(Part::Docs, "nothing checks the doc changed with the item"),
        "spec_sections" => part(Part::Design, "no item spec needs a Design"),
        "design-review" => part(Part::DesignReview, "no one signs off a design"),
        "drift" => {
            if repo {
                (true, String::new())
            } else {
                (
                    false,
                    "no code repository here, so drift cannot be checked".to_string(),
                )
            }
        }
        "view-id" => (
            false,
            "not yet: no kanbanr view carries an ID for this source".to_string(),
        ),
        "none" => (true, "drift is accepted".to_string()),
        other => {
            if let Some(id) = other.strip_prefix("coverage:") {
                if config.coverage.iter().any(|r| r.id == id) {
                    (true, String::new())
                } else {
                    (false, format!("the config has no coverage rule \"{id}\""))
                }
            } else if let Some(test) = other.strip_prefix("test:") {
                let declared =
                    row.source.kind == SourceKind::Enforced && row.source.test.trim() == test;
                if tracked.contains(test) || declared {
                    (true, String::new())
                } else {
                    (false, format!("no item tracks a test named \"{test}\""))
                }
            } else {
                (false, format!("\"{other}\" is not a protection"))
            }
        }
    };
    ProtectionStatus {
        name: name.to_string(),
        in_force,
        why,
    }
}

/// The catalog's status for `project`: each row's applicability, source state and protections, and
/// the gaps. Reads the catalog and facts files and each written source once, in row order; writes
/// nothing and holds no clock, host or absolute path, so the same board gives the same bytes. With
/// no catalog it reads no doc and returns no rows. An invalid catalog comes back under `problems`
/// with no rows. `repo` is the code repository the surface has, for `repo:` sources.
pub fn status(
    store: &Store,
    project: &crate::store::Project,
    repo: Option<&std::path::Path>,
) -> Result<CatalogStatus> {
    let id = project.id.as_str();
    let mut out = CatalogStatus::default();
    let facts = match crate::facts::load(store, id) {
        Ok(f) => f,
        Err(e) => {
            out.problems
                .push(format!("{}: {e}", crate::facts::FACTS_FILE));
            None
        }
    };
    out.facts_rev = facts.as_ref().map(|f| f.content_rev()).unwrap_or_default();
    let catalog = match load(store, id) {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(out),
        Err(e) => {
            out.problems.push(format!("{CATALOG_FILE}: {e}"));
            return Ok(out);
        }
    };
    out.starter = catalog.starter;
    let found = problems(&catalog);
    if !found.is_empty() {
        out.problems.extend(found);
        return Ok(out);
    }
    let tracked: BTreeSet<&str> = project
        .features
        .iter()
        .filter_map(|f| f.definition.as_ref())
        .flat_map(|d| d.requirements.iter())
        .flat_map(|r| r.tests.iter())
        .map(|t| t.name.as_str())
        .collect();
    let mut reader = Reader {
        store,
        project: id,
        repo,
        seen: BTreeMap::new(),
        listing: None,
        reads: 0,
    };
    let mut filled: BTreeSet<(String, String)> = BTreeSet::new();
    for row in &catalog.rows {
        let applies = row.applicability(facts.as_ref(), &project.config);
        let state = reader.state(row);
        if applies.applies && state.fills() {
            filled.extend(row.cells());
        }
        let protections = row
            .protection
            .iter()
            .map(|p| protection_status(p, row, project, &tracked, repo.is_some()))
            .collect();
        out.rows.push(RowStatus {
            id: row.id.clone(),
            title: row.title.clone(),
            level: row.level,
            answers: row.answers.clone(),
            for_: row.for_.clone(),
            notation: row.notation.clone(),
            source: row.source.clone(),
            state,
            applies,
            protections,
            href: row.href(id),
        });
    }
    out.docs_read = reader.reads;
    for q in QUESTIONS {
        for p in PERSPECTIVES {
            if !filled.contains(&(q.to_string(), p.id.to_string())) {
                out.gaps.push(Cell {
                    question: q.to_string(),
                    perspective: p.id.to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// The rows a change touches (FTR-006 reads this): written rows whose source is among `docs`
/// (board doc keys; a folder row matches any doc under it; a `repo:` row matches its own
/// `repo:<path>`), plus rows whose id is in `named`. Sorted, deduplicated. Pure.
pub fn views_changed(catalog: &Catalog, docs: &[String], named: &[String]) -> Vec<String> {
    let keys: BTreeSet<String> = docs
        .iter()
        .filter(|d| !d.trim().starts_with(REPO_PREFIX))
        .map(|d| crate::refs::doc_key(d))
        .collect();
    let repo: BTreeSet<&str> = docs
        .iter()
        .map(|d| d.trim())
        .filter(|d| d.starts_with(REPO_PREFIX))
        .collect();
    let mut out = BTreeSet::new();
    for row in &catalog.rows {
        if named.iter().any(|n| n.trim() == row.id) {
            out.insert(row.id.clone());
            continue;
        }
        if row.source.kind != SourceKind::Written {
            continue;
        }
        let path = row.source.path.trim();
        let hit = if path.is_empty() {
            false
        } else if path.starts_with(REPO_PREFIX) {
            repo.contains(path)
        } else if path.ends_with('/') {
            let prefix = folder_prefix(path);
            keys.iter().any(|k| k.starts_with(&prefix))
        } else {
            keys.contains(&crate::refs::doc_key(path))
        };
        if hit {
            out.insert(row.id.clone());
        }
    }
    out.into_iter().collect()
}

/// The rows a design changes (FTR-006): [`views_changed`] over the docs the definition declares in
/// `affects_docs`, the board docs linked under the spec's Affects docs heading, and the rows
/// `affects_views` names. Pure.
pub fn views_changed_of(
    catalog: &Catalog,
    project: &str,
    spec: &str,
    def: &crate::models::FeatureDefinition,
) -> Vec<String> {
    let mut docs = def.affects_docs.clone();
    docs.extend(crate::docsbuilt::linked_board_docs(spec, project));
    views_changed(catalog, &docs, &def.affects_views)
}

/// What a `kanbanr:` generator derives for `project`, as JSON: the value the system baseline
/// (FTR-006) hashes through [`crate::hash::canonical_json`] to pin a generated row. Each is the
/// read the view already serves: the charter, the Zachman matrix, the ADR index, the trace of
/// each charter goal, every item's requirements, the release reports, the Gantt chart's text,
/// every item's tracked tests, and the component docs under `architecture/components/`. A
/// `katalogr:` source or a name kanbanr does not derive is refused: it cannot be pinned here.
pub fn generate(
    store: &Store,
    project: &crate::store::Project,
    generator: &str,
) -> Result<serde_json::Value> {
    use serde_json::{Value, json};
    fn ser<T: Serialize>(v: &T) -> Result<Value> {
        serde_json::to_value(v).map_err(|e| CoreError::Unsupported(e.to_string()))
    }
    let id = project.id.as_str();
    let mut features: Vec<&crate::FeatureItem> = project.features.iter().collect();
    features.sort_by(|a, b| a.code.cmp(&b.code));
    match generator.trim() {
        "kanbanr:charter" => ser(&crate::charter::load(store, id)?),
        // A project with no items has no matrix: the matrix refuses an empty scope.
        "kanbanr:zachman" if project.features.is_empty() => Ok(Value::Null),
        "kanbanr:zachman" => ser(&crate::trace::zachman(store, id, None)?),
        "kanbanr:adr-index" => ser(&crate::adr::list(store, id)?),
        "kanbanr:trace" => {
            let charter = crate::charter::load(store, id)?;
            let mut out = Vec::new();
            for goal in &charter.goals {
                let subject = crate::trace::Subject::Goal {
                    id: goal.id.clone(),
                };
                out.push(json!({"goal": goal.id, "trace": ser(&crate::trace::run(store, id, &subject)?)?}));
            }
            Ok(Value::Array(out))
        }
        "kanbanr:requirements" => Ok(Value::Array(
            features
                .iter()
                .filter_map(|f| {
                    let d = f.definition.as_ref()?;
                    Some(json!({"code": f.code, "requirements": d.requirements}))
                })
                .collect(),
        )),
        "kanbanr:releases" => ser(&crate::releases::report_all(store, id)?),
        // The chart's inputs, not its text: the rendered chart tags a sprint active or done by
        // today's date, which would read as a change with nothing changed (FTR-006 T2 review).
        "kanbanr:gantt" => {
            let mut milestones: Vec<&crate::models::Milestone> =
                project.milestones.iter().collect();
            milestones.sort_by(|a, b| a.code.cmp(&b.code));
            let sprints = if project.config.cadence.sprints {
                let mut sprints = crate::sprints::load(store, id)?;
                sprints.sort_by(|a, b| a.code.cmp(&b.code));
                sprints
                    .iter()
                    .map(|s| json!({"code": s.code, "start": s.start, "end": s.end}))
                    .collect()
            } else {
                Vec::new()
            };
            Ok(json!({
                "milestones": milestones
                    .iter()
                    .map(|m| json!({"code": m.code, "name": m.name, "depends_on": m.depends_on}))
                    .collect::<Vec<_>>(),
                "sprints": sprints,
                "items": features
                    .iter()
                    .map(|f| json!({
                        "code": f.code,
                        "milestone": f.milestone,
                        "sprint": f.sprint,
                        "depends_on": f.depends_on,
                    }))
                    .collect::<Vec<_>>(),
            }))
        }
        "kanbanr:evidence" => Ok(Value::Array(
            features
                .iter()
                .filter_map(|f| {
                    let d = f.definition.as_ref()?;
                    let tests: Vec<Value> = d
                        .requirements
                        .iter()
                        .filter(|r| !r.tests.is_empty())
                        .map(|r| json!({"requirement": r.id, "tests": r.tests}))
                        .collect();
                    (!tests.is_empty()).then(|| json!({"code": f.code, "tests": tests}))
                })
                .collect(),
        )),
        "kanbanr:component-list" => {
            let prefix = "architecture/components/";
            let docs: Vec<String> = store
                .list_docs(id)?
                .into_iter()
                .filter(|d| d.starts_with(prefix))
                .collect();
            ser(&docs)
        }
        other => Err(CoreError::Unsupported(format!(
            "\"{other}\" is not a view kanbanr derives (use {})",
            GENERATORS.join(", ")
        ))),
    }
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

    /// FTR-006: every view kanbanr derives has a value to pin, the same twice; a katalogr view
    /// and an unknown name are refused.
    #[test]
    fn generate_derives_every_kanbanr_view() {
        let (store, dir) = temp_store();
        let project = store.load_meta("demo").unwrap();
        for g in GENERATORS {
            let once = generate(&store, &project, g).unwrap_or_else(|e| panic!("{g}: {e}"));
            let twice = generate(&store, &project, g).unwrap();
            assert_eq!(once, twice, "{g}");
        }
        assert!(generate(&store, &project, "katalogr:structure").is_err());
        assert!(generate(&store, &project, "kanbanr:nothing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-006 T2 review: `kanbanr:gantt` is pinned by the chart's inputs, not its text. A sprint
    /// that turns active by date changes the rendered chart ("active") but nothing it is drawn
    /// from, so the generated value — and its hash — does not move.
    #[test]
    fn gantt_is_generated_from_its_inputs_so_a_sprint_turning_active_by_date_is_no_change() {
        let (store, dir) = temp_store();
        let mut config = store.load_meta("demo").unwrap().config;
        config.cadence.sprints = true;
        store.save_config("demo", &config).unwrap();
        let today = crate::sprints::today();
        let start = today.checked_add(time::Duration::days(1)).unwrap();
        let mut sprint = crate::sprints::add(
            &store,
            "demo",
            &crate::gantt::fmt_date(start),
            Some(5),
            "",
            None,
            "",
            None,
            None,
        )
        .unwrap();
        // Started ahead of its first day: planned today, active from tomorrow, by date alone.
        sprint.state = crate::sprints::SprintState::Active;
        std::fs::write(
            store.project_dir("demo").join("sprints.yaml"),
            crate::yaml::to_string(&vec![sprint.clone()]).unwrap(),
        )
        .unwrap();
        assert_eq!(sprint.state_on(today), crate::sprints::SprintState::Planned);
        assert_eq!(sprint.state_on(start), crate::sprints::SprintState::Active);

        let project = store.load_meta("demo").unwrap();
        let value = generate(&store, &project, "kanbanr:gantt").unwrap();
        assert_eq!(
            value["sprints"],
            serde_json::json!([{"code": sprint.code, "start": sprint.start, "end": sprint.end}]),
            "a sprint is its code and dates, never its state on a day"
        );
        for key in ["milestones", "items"] {
            assert!(value[key].is_array(), "{key}: {value}");
        }
        let text = value.to_string();
        assert!(
            !text.contains("active") && !text.contains("done"),
            "nothing in the value is derived from today: {text}"
        );
        let hash =
            |v: &serde_json::Value| crate::hash::stable_hash(&crate::hash::canonical_json(v));
        let again = generate(&store, &store.load_meta("demo").unwrap(), "kanbanr:gantt").unwrap();
        assert_eq!(hash(&value), hash(&again));
        let _ = std::fs::remove_dir_all(&dir);
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

    /// Every file under `dir`, with its bytes: a board compared before and after a read.
    fn snapshot(dir: &std::path::Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(d: &std::path::Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.insert(p.clone(), std::fs::read(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, &mut out);
        out
    }

    fn long_body() -> String {
        format!("# Context\n\n{}\n", "The system reads a board. ".repeat(8))
    }

    fn written_row(id: &str, path: &str, answers: &[&str], for_: &[&str]) -> Row {
        Row {
            id: id.into(),
            title: id.into(),
            answers: strings(answers),
            for_: strings(for_),
            source: Source {
                kind: SourceKind::Written,
                path: path.into(),
                ..Source::default()
            },
            ..Row::default()
        }
    }

    fn state_of<'a>(s: &'a CatalogStatus, id: &str) -> &'a RowStatus {
        s.rows.iter().find(|r| r.id == id).unwrap()
    }

    /// R-7: every source state is derived from the board's files; protections follow the config;
    /// the gaps are the cells no applying row with a source fills; nothing is written.
    #[test]
    fn status_is_derived_from_board_files_and_names_the_gaps() {
        let (store, dir) = temp_store();
        store
            .write_doc("demo", "architecture/context.md", &long_body())
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/behaviour.md",
                "---\ntitle: Behaviour\n---\n# Behaviour\n\nTo do.\n",
            )
            .unwrap();
        store
            .write_doc("demo", "architecture/components/store.md", &long_body())
            .unwrap();
        store
            .write_doc("demo", "architecture/components/web.md", "# Web\n")
            .unwrap();
        let repo = dir.join("repo");
        std::fs::create_dir_all(repo.join("docs/system")).unwrap();
        std::fs::write(repo.join("docs/system/context.md"), long_body()).unwrap();

        let mut rows = vec![
            Row {
                protection: strings(&["docs_updated", "coverage:catalog", "drift"]),
                ..written_row(
                    "context",
                    "architecture/context.md",
                    &["what"],
                    &["sponsor"],
                )
            },
            // Read through the same key: once.
            written_row(
                "context-again",
                "docs/architecture/context.md",
                &["what"],
                &["user"],
            ),
            written_row(
                "behaviour",
                "architecture/behaviour.md",
                &["how"],
                &["user"],
            ),
            Row {
                protection: strings(&["spec_sections", "design-review", "coverage:nope"]),
                ..written_row("missing", "architecture/data.md", &["what"], &["architect"])
            },
            Row {
                level: Level::Component,
                ..written_row(
                    "components",
                    "architecture/components/",
                    &["what"],
                    &["builder"],
                )
            },
            Row {
                level: Level::Component,
                ..written_row("empty-folder", "architecture/none/", &["who"], &["builder"])
            },
            written_row(
                "repo-doc",
                "repo:docs/system/context.md",
                &["where"],
                &["operator"],
            ),
            written_row(
                "repo-missing",
                "repo:docs/system/gone.md",
                &["when"],
                &["operator"],
            ),
            Row {
                level: Level::Item,
                protection: strings(&["view-id", "none"]),
                ..written_row("item-design", "", &["how"], &["builder"])
            },
            Row {
                source: Source {
                    kind: SourceKind::Generated,
                    generator: "kanbanr:charter".into(),
                    ..Source::default()
                },
                ..written_row("purpose", "", &["why"], &["sponsor"])
            },
            Row {
                source: Source {
                    kind: SourceKind::Generated,
                    generator: "katalogr:structure".into(),
                    ..Source::default()
                },
                ..written_row("code", "", &["what"], &["builder"])
            },
            Row {
                source: Source {
                    kind: SourceKind::Enforced,
                    test: "architecture::tests::thin".into(),
                    ..Source::default()
                },
                protection: strings(&["test:architecture::tests::thin", "test:other::t"]),
                ..written_row("thin", "", &["how"], &["architect"])
            },
            // Off: it fills nothing.
            Row {
                applies: Some(false),
                why: "not ours".into(),
                ..written_row("off", "architecture/context.md", &["why"], &["operator"])
            },
        ];
        // A row with a `when` and no facts record does not apply.
        let mut when = BTreeMap::new();
        when.insert("parts".to_string(), strings(&["api"]));
        rows.push(Row {
            when,
            ..written_row("interfaces", "architecture/context.md", &["why"], &["user"])
        });
        save(
            &store,
            "demo",
            &Catalog {
                schema: SCHEMA,
                starter: 1,
                rows,
                ..Catalog::default()
            },
        )
        .unwrap();
        let mut project = store.load("demo").unwrap();
        project.config.coverage.push(crate::coverage::CoverageRule {
            id: "catalog".into(),
            ..Default::default()
        });
        project.config.method.off = vec![crate::method::Part::DesignReview];

        let before = snapshot(&dir);
        let s = status(&store, &project, Some(&repo)).unwrap();
        assert_eq!(snapshot(&dir), before, "reading the status writes nothing");

        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.starter, 1);
        assert_eq!(s.facts_rev, "");
        let st = |id: &str| state_of(&s, id).state.clone();
        assert_eq!(st("context"), SourceState::Present);
        assert_eq!(st("context-again"), SourceState::Present);
        assert_eq!(st("behaviour"), SourceState::Stub);
        assert_eq!(st("missing"), SourceState::Missing);
        assert_eq!(st("components"), SourceState::Folder { docs: 2 });
        assert_eq!(st("empty-folder"), SourceState::Folder { docs: 0 });
        assert_eq!(st("repo-doc"), SourceState::Present);
        assert_eq!(st("repo-missing"), SourceState::Missing);
        assert_eq!(st("item-design"), SourceState::Declared);
        assert_eq!(st("purpose"), SourceState::Available);
        assert_eq!(st("code"), SourceState::NeedsKatalogr);
        assert_eq!(st("thin"), SourceState::Declared);
        // context, behaviour, data.md, the repo doc and its missing sibling: each read once.
        assert_eq!(s.docs_read, 5);
        assert_eq!(
            state_of(&s, "context").href.as_deref(),
            Some("/p/demo/docs/file/architecture/context.md")
        );
        // Applicability rides along.
        assert!(!state_of(&s, "off").applies.applies);
        assert_eq!(state_of(&s, "interfaces").applies.why, NO_FACTS);

        // Protections, per config.
        let prot = |id: &str, name: &str| {
            state_of(&s, id)
                .protections
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .clone()
        };
        assert!(prot("context", "docs_updated").in_force);
        assert!(prot("context", "coverage:catalog").in_force);
        assert!(prot("context", "drift").in_force, "a repository is here");
        assert!(prot("missing", "spec_sections").in_force);
        let dr = prot("missing", "design-review");
        assert!(!dr.in_force && dr.why.contains("design-review"), "{dr:?}");
        assert!(!prot("missing", "coverage:nope").in_force);
        assert!(!prot("item-design", "view-id").in_force);
        assert!(prot("item-design", "none").in_force);
        assert!(prot("thin", "test:architecture::tests::thin").in_force);
        assert!(!prot("thin", "test:other::t").in_force);

        // Gaps: cells with no applying row whose source fills them, in question then perspective
        // order. Filled: what×sponsor, what×user (context), what×builder (components; and code),
        // where×operator (repo doc), how×builder (item-design), why×sponsor (purpose),
        // how×architect (thin).
        let filled = [
            ("what", "sponsor"),
            ("what", "user"),
            ("what", "builder"),
            ("how", "architect"),
            ("how", "builder"),
            ("where", "operator"),
            ("why", "sponsor"),
        ];
        let expected: Vec<Cell> = QUESTIONS
            .iter()
            .flat_map(|q| PERSPECTIVES.iter().map(move |p| (*q, p.id)))
            .filter(|c| !filled.contains(c))
            .map(|(q, p)| Cell {
                question: q.into(),
                perspective: p.into(),
            })
            .collect();
        assert_eq!(s.gaps, expected);
        assert_eq!(
            s.gaps[0],
            Cell {
                question: "what".into(),
                perspective: "architect".into()
            },
            "the missing doc fills no cell"
        );

        // With no repository, a repo source cannot tell and drift is not in force.
        let s = status(&store, &project, None).unwrap();
        assert_eq!(state_of(&s, "repo-doc").state, SourceState::NoRepository);
        assert!(
            !state_of(&s, "context")
                .protections
                .iter()
                .find(|p| p.name == "drift")
                .unwrap()
                .in_force
        );
        assert!(
            !s.gaps
                .iter()
                .any(|c| c.question == "where" && c.perspective == "operator"),
            "a source the surface cannot read is not a gap"
        );
        assert_eq!(s.docs_read, 3);

        // An invalid catalog comes back as problems with no rows.
        let mut bad = load(&store, "demo").unwrap().unwrap();
        bad.rows[0].answers = strings(&["which"]);
        crate::atomic::write(
            catalog_path(&store, "demo"),
            crate::yaml::to_string(&bad).unwrap(),
        )
        .unwrap();
        let s = status(&store, &project, None).unwrap();
        assert!(s.rows.is_empty() && s.docs_read == 0);
        assert!(
            s.problems.iter().any(|p| p.contains("row \"context\"")),
            "{:?}",
            s.problems
        );
        std::fs::write(catalog_path(&store, "demo"), "rows: [unclosed").unwrap();
        let s = status(&store, &project, None).unwrap();
        assert!(s.rows.is_empty());
        assert!(s.problems[0].starts_with(CATALOG_FILE), "{:?}", s.problems);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-7: two reads of the same board give byte-equal JSON.
    #[test]
    fn the_same_board_gives_the_same_status_bytes() {
        let (store, dir) = temp_store();
        init(&store, "demo", false).unwrap();
        store
            .write_doc("demo", "architecture/context.md", &long_body())
            .unwrap();
        crate::facts::save(
            &store,
            "demo",
            &Facts {
                production: vec!["server".into()],
                parts: vec!["api".into(), "cli".into()],
                ..Facts::default()
            },
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let a = serde_json::to_vec(&status(&store, &project, None).unwrap()).unwrap();
        let b = serde_json::to_vec(&status(&store, &project, None).unwrap()).unwrap();
        assert_eq!(a, b);
        let s = status(&store, &project, None).unwrap();
        assert_eq!(s.rows.len(), 21);
        assert_eq!(
            s.facts_rev,
            crate::facts::load(&store, "demo")
                .unwrap()
                .unwrap()
                .content_rev()
        );
        let text = String::from_utf8(a).unwrap();
        assert!(!text.contains(&*dir.to_string_lossy()), "no absolute path");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-8: with no files, the status reads no docs and has no rows.
    #[test]
    fn no_catalog_no_effect() {
        let (store, dir) = temp_store();
        store
            .write_doc("demo", "architecture/context.md", &long_body())
            .unwrap();
        assert_eq!(load(&store, "demo").unwrap(), None);
        assert_eq!(crate::facts::load(&store, "demo").unwrap(), None);
        let project = store.load("demo").unwrap();
        let before = snapshot(&dir);
        let s = status(&store, &project, Some(&dir)).unwrap();
        assert_eq!(snapshot(&dir), before);
        assert_eq!(s.docs_read, 0);
        assert!(s.rows.is_empty() && s.gaps.is_empty() && s.problems.is_empty());
        assert_eq!(s, CatalogStatus::default());
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!({"facts_rev": "", "starter": 0, "rows": [], "gaps": [], "docs_read": 0})
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-12: 40 rows over 40 docs of 20 KB and 2,000 items, each source read once, under 200 ms.
    #[test]
    fn status_reads_each_source_once_within_budget() {
        let (store, dir) = temp_store();
        let body = format!("# Doc\n\n{}\n", "x".repeat(20 * 1024));
        let mut rows = Vec::new();
        for i in 0..40 {
            let path = format!("architecture/doc-{i:02}.md");
            store.write_doc("demo", &path, &body).unwrap();
            rows.push(Row {
                protection: strings(&["docs_updated", &format!("test:t{i}")]),
                ..written_row(&format!("row-{i:02}"), &path, &["what"], &["builder"])
            });
        }
        save(
            &store,
            "demo",
            &Catalog {
                schema: SCHEMA,
                rows,
                ..Catalog::default()
            },
        )
        .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let seed = store
            .add_feature("demo", "seed", "", "M", Some("FEAT-001".into()))
            .unwrap();
        let mut project = store.load("demo").unwrap();
        project.features = (0..2000)
            .map(|i| {
                let mut f = seed.clone();
                f.code = format!("FEAT-{i:04}");
                f.definition = Some(crate::models::FeatureDefinition {
                    requirements: vec![crate::models::Requirement {
                        id: "R-1".into(),
                        text: "x".into(),
                        tests: vec![crate::models::TestRef {
                            name: format!("t{i}"),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                });
                f
            })
            .collect();
        let started = std::time::Instant::now();
        let s = status(&store, &project, None).unwrap();
        let took = started.elapsed();
        assert_eq!(s.docs_read, 40);
        assert_eq!(s.rows.len(), 40);
        assert!(
            s.rows
                .iter()
                .all(|r| r.state == SourceState::Present && r.protections[1].in_force)
        );
        assert!(
            took < std::time::Duration::from_millis(200),
            "status took {took:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `views_changed`: written rows by doc key or folder, repository rows by their own path, and
    /// rows named outright; sorted and deduplicated.
    #[test]
    fn views_changed_names_rows_by_doc_and_by_id() {
        let c = starter();
        let changed = views_changed(
            &c,
            &strings(&[
                "docs/architecture/context.md",
                "architecture/components/store.md",
                "architecture/unrelated.md",
            ]),
            &strings(&["decisions", "context"]),
        );
        assert_eq!(
            changed,
            strings(&["component-docs", "context", "decisions"])
        );
        let mut repo = c.clone();
        repo.rows[3].source.path = "repo:docs/system/context.md".into();
        assert_eq!(repo.rows[3].id, "context");
        assert_eq!(
            views_changed(&repo, &strings(&["repo:docs/system/context.md"]), &[]),
            strings(&["context"])
        );
        assert!(views_changed(&repo, &strings(&["docs/system/context.md"]), &[]).is_empty());
        assert!(views_changed(&c, &[], &[]).is_empty());
    }

    /// FTR-006 R-8: a doc under a folder row, a doc named in `affects_docs`, a doc linked under
    /// Affects docs, and a named generated row each give their row once.
    #[test]
    fn views_changed_are_derived_from_docs_and_named_rows() {
        let c = starter();
        let spec = "## Design\n\nx\n\n### Affects docs\n\n- [components](/p/demo/docs/file/architecture/components/store.md)\n- [context again](/p/demo/docs/file/architecture/context.md)\n- [elsewhere](/p/other/docs/file/architecture/security.md)\n";
        let def = crate::models::FeatureDefinition {
            affects_docs: strings(&["architecture/context.md"]),
            affects_views: strings(&["decisions"]),
            ..Default::default()
        };
        let got = views_changed_of(&c, "demo", spec, &def);
        assert_eq!(got, strings(&["component-docs", "context", "decisions"]));
        // Nothing declared, linked or named: nothing changes.
        let none = crate::models::FeatureDefinition::default();
        assert!(views_changed_of(&c, "demo", "## Design\n\nno links\n", &none).is_empty());
        // A generated row is changed only by being named.
        let only_named = crate::models::FeatureDefinition {
            affects_views: strings(&["decisions", "decisions"]),
            ..Default::default()
        };
        assert_eq!(
            views_changed_of(&c, "demo", "", &only_named),
            strings(&["decisions"])
        );
    }

    /// R-11: the skill's setup interview and the living-docs guide name the facts questions and
    /// every catalog command, so Claude asks the facts once and the person can read what they do.
    #[test]
    fn the_skill_and_the_guide_name_the_facts_questions_and_the_catalog_commands() {
        let skill = include_str!("../../../../skill/kanbanr/SKILL.md");
        let guide = include_str!("../../../../docs/guide/using/living-docs.md");
        for group in ["**d. Facts**", "**f. Catalog**"] {
            assert!(skill.contains(group), "the skill's setup has no {group}");
        }
        let facts = skill.find("**d. Facts**").unwrap();
        let baseline = skill.find("**e. Engineering baseline**").expect("group e");
        let catalog = skill.find("**f. Catalog**").unwrap();
        assert!(
            facts < baseline && baseline < catalog,
            "setup order is facts, baseline, catalog"
        );
        assert!(
            guide.contains("### What a project documents: the content catalog"),
            "the guide has no catalog section"
        );
        for command in [
            "kanbanr facts set",
            "kanbanr facts vocabulary",
            "kanbanr catalog init",
            "kanbanr catalog add",
            "kanbanr catalog off",
            "kanbanr catalog on",
        ] {
            assert!(
                skill.contains(command),
                "the skill does not name `{command}`"
            );
            assert!(
                guide.contains(command),
                "the guide does not name `{command}`"
            );
        }
        for v in crate::facts::VOCABULARY.iter() {
            let key = format!("`{}`", v.key);
            assert!(
                skill.contains(&key),
                "the skill does not name the facts key {key}"
            );
            assert!(
                guide.contains(&key),
                "the guide does not name the facts key {key}"
            );
        }
    }
}
