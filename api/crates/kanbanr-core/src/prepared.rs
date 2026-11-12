//! Prepared records (FTR-034): the evidence for a decision, written by Claude with the CLI and
//! confirmed by the person with one click.
//!
//! A prepared record is evidence, never a decision: it changes no gate, no readiness and no
//! verdict record. Only a confirmation makes the decision, and it makes it through the verdict's
//! own route, with every check that route makes. Each record is a file of its own,
//! `prepared/<id>.yaml` in the project folder, written once; the person's answer is a second file,
//! `prepared/<id>.resolved.yaml`, written once. Neither is ever rewritten, and reading never
//! writes (ADR-0005): the state of a record is derived when it is listed.

use crate::error::{CoreError, Result};
use crate::models::Extra;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// The longest label a source may have, in characters.
pub const MAX_LABEL: usize = 200;
/// The most sources one record may name.
pub const MAX_SOURCES: usize = 20;
/// The longest text field, in characters.
pub const MAX_TEXT: usize = 4000;

/// Evidence for one decision, prepared for the person to confirm.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Prepared {
    /// `<YYYYMMDDTHHMMSSZ>-<kind>`, with `-2`, `-3` … when taken.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `signoff`, `promotion`, `gate-result`, `ratification`, … A string, so a kind a newer
    /// kanbanr adds is listed by an older one rather than failing to parse.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// What the decision is about; keys per kind.
    #[serde(default)]
    pub subject: Subject,
    /// What the verdict will carry; keys per kind.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Value>,
    /// Where the evidence came from: at least one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// `Claude` when prepared in the conversation; else the preparer's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    /// Stamped by the front end, never the request (DEF-028).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// RFC 3339, UTC.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub at: String,
    /// What it was prepared against: the definition's revision, the release tip or the commit.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rev: String,
    /// Keys this version does not know, kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What a decision is about. Which keys a kind needs is in [`KINDS`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(rename = "PreparedSubject")
)]
pub struct Subject {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signoff: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub requirement: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gate: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// Keys this version does not know, kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl Subject {
    /// The value of a known key; "" for an unknown or empty one.
    pub fn get(&self, key: &str) -> &str {
        match key {
            "item" => &self.item,
            "signoff" => &self.signoff,
            "requirement" => &self.requirement,
            "test" => &self.test,
            "release" => &self.release,
            "stage" => &self.stage,
            "gate" => &self.gate,
            "commit" => &self.commit,
            "target" => &self.target,
            _ => "",
        }
    }

    /// Set a known key; false for a key a subject does not have.
    pub fn set(&mut self, key: &str, value: String) -> bool {
        let slot = match key {
            "item" => &mut self.item,
            "signoff" => &mut self.signoff,
            "requirement" => &mut self.requirement,
            "test" => &mut self.test,
            "release" => &mut self.release,
            "stage" => &mut self.stage,
            "gate" => &mut self.gate,
            "commit" => &mut self.commit,
            "target" => &mut self.target,
            _ => return false,
        };
        *slot = value;
        true
    }
}

/// Where a piece of evidence came from: a link or a board reference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(rename = "PreparedSource")
)]
pub struct Source {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Empty, `http(s)://…` with no user info, `doc:<path>`, `item:<CODE>` or `commit:<sha>`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Keys this version does not know, kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// The person's answer to a prepared record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Resolution {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub verdict: Answer,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub at: String,
    /// Each field the person edited; empty when confirmed as prepared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<FieldChange>,
    /// A rejection's reason, optional.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// The verdict route it made, `POST /projects/…`; empty on a rejection.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub made: String,
    /// Keys this version does not know, kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Confirmed,
    Rejected,
}

/// One field the person changed before confirming: both values are kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FieldChange {
    pub field: String,
    pub prepared: Value,
    pub confirmed: Value,
    /// Keys this version does not know, kept and written back unchanged (FEAT-151).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// Where a prepared record stands, derived when it is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(rename = "PreparedState")
)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting for the person.
    Open,
    Confirmed,
    Rejected,
    /// A newer record for the same decision replaces it.
    Superseded,
    /// The decision no longer waits, or moved on since it was prepared.
    Stale,
    /// A kind this version of kanbanr does not know.
    UnknownKind,
}

// ---- kinds ---------------------------------------------------------------------------------

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Bool,
    /// A test state: `planned`, `red` or `green`.
    State,
}

/// One field a kind carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub ty: FieldType,
    pub required: bool,
    /// The person may change it before confirming.
    pub editable: bool,
}

/// What a record's `rev` is taken from when it is prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevOf {
    /// The item's `definition.content_rev()`.
    Definition,
    /// The release branch's tip, read with the repository; empty without one.
    ReleaseTip,
    /// The subject's commit.
    Commit,
}

/// A kind of prepared record: its subject's required keys, its fields and its `rev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSpec {
    pub kind: &'static str,
    pub subject: &'static [&'static str],
    pub fields: &'static [FieldSpec],
    pub rev: RevOf,
}

const fn text(name: &'static str, required: bool) -> FieldSpec {
    FieldSpec {
        name,
        ty: FieldType::Text,
        required,
        editable: true,
    }
}

/// Every kind this version knows.
pub const KINDS: &[KindSpec] = &[
    KindSpec {
        kind: "signoff",
        subject: &["item", "signoff"],
        fields: &[text("note", false), text("doc", false)],
        rev: RevOf::Definition,
    },
    KindSpec {
        kind: "promotion",
        subject: &["release", "stage"],
        fields: &[text("checked", true)],
        rev: RevOf::ReleaseTip,
    },
    KindSpec {
        kind: "gate-result",
        subject: &["gate", "commit"],
        fields: &[
            FieldSpec {
                name: "passed",
                ty: FieldType::Bool,
                required: true,
                editable: true,
            },
            text("url", false),
            text("note", false),
        ],
        rev: RevOf::Commit,
    },
    KindSpec {
        kind: "ratification",
        subject: &["item"],
        fields: &[text("reason", true)],
        rev: RevOf::Definition,
    },
];

/// The kind named `kind`, if this version knows it.
pub fn kind(kind: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| k.kind == kind)
}

fn unsupported(msg: String) -> CoreError {
    CoreError::Unsupported(msg)
}

// ---- sources -------------------------------------------------------------------------------

fn is_board_ref(url: &str) -> bool {
    ["doc:", "item:", "commit:"]
        .iter()
        .any(|p| url.starts_with(p))
}

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Is `url` one a source may hold? Returns why not.
pub fn check_url(url: &str) -> std::result::Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    if is_web(url) {
        let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() {
            return Err(format!("'{url}' names no host"));
        }
        if authority.contains('@') {
            return Err(format!(
                "'{url}' carries user info before its host: a source never stores a credential"
            ));
        }
        if url.chars().any(char::is_whitespace) {
            return Err(format!("'{url}' holds a space"));
        }
        return Ok(());
    }
    if let Some(path) = url.strip_prefix("doc:") {
        if path.trim().is_empty() || path.split('/').any(|s| s == "..") {
            return Err(format!("'{url}' does not name a board doc"));
        }
        return Ok(());
    }
    if let Some(code) = url.strip_prefix("item:") {
        let ok = !code.is_empty()
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        return if ok {
            Ok(())
        } else {
            Err(format!("'{url}' does not name an item"))
        };
    }
    if let Some(sha) = url.strip_prefix("commit:") {
        let ok = (7..=40).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit());
        return if ok {
            Ok(())
        } else {
            Err(format!(
                "'{url}' does not name a commit (7 to 40 hex digits)"
            ))
        };
    }
    Err(format!(
        "'{url}' is not a source: use an http(s) link, doc:<path>, item:<CODE> or commit:<sha>"
    ))
}

/// Is this source one a record may hold?
pub fn check_source(s: &Source) -> Result<()> {
    let n = s.label.trim().chars().count();
    if n == 0 || n > MAX_LABEL {
        return Err(unsupported(format!(
            "a source's label is 1 to {MAX_LABEL} characters: '{}'",
            s.label
        )));
    }
    check_url(&s.url).map_err(unsupported)
}

/// Read a source as the CLI takes it: `LABEL=URL`, a bare URL (labelled `link`, `commit`, or the
/// path or code of a board reference), or a bare label.
pub fn parse_source(s: &str) -> Result<Source> {
    let s = s.trim();
    let source = if is_web(s) || is_board_ref(s) {
        let label = if is_web(s) {
            "link".to_string()
        } else if s.starts_with("commit:") {
            "commit".to_string()
        } else {
            s.split_once(':').map(|(_, r)| r).unwrap_or(s).to_string()
        };
        Source {
            label,
            url: s.to_string(),
            ..Default::default()
        }
    } else if let Some((label, url)) = s.split_once('=')
        && !url.trim().is_empty()
    {
        Source {
            label: label.trim().to_string(),
            url: url.trim().to_string(),
            ..Default::default()
        }
    } else {
        Source {
            label: s.to_string(),
            ..Default::default()
        }
    };
    check_source(&source)?;
    Ok(source)
}

// ---- storage -------------------------------------------------------------------------------

fn dir(store: &Store, p: &str) -> PathBuf {
    store.project_dir(p).join("prepared")
}

const RESOLVED: &str = ".resolved.yaml";

/// Keep a prepared record as a new file. Its id is `<YYYYMMDDTHHMMSSZ>-<kind>` from `at` (or the
/// record's own id when it has one), with `-2`, `-3` … when taken. Written atomically; an existing
/// file is never replaced. Returns the path; the file's stem is the record's id.
pub fn write_record(store: &Store, p: &str, r: &Prepared) -> Result<PathBuf> {
    let folder = dir(store, p);
    std::fs::create_dir_all(&folder)?;
    let base = if r.id.is_empty() {
        let digits: String =
            r.at.chars()
                .filter(|c| c.is_ascii_digit())
                .take(14)
                .collect();
        if digits.len() != 14 {
            return Err(unsupported(format!("'{}' is not a time", r.at)));
        }
        format!("{}T{}Z-{}", &digits[..8], &digits[8..], r.kind)
    } else {
        r.id.clone()
    };
    let taken = |id: &str| {
        folder.join(format!("{id}.yaml")).exists()
            || folder.join(format!("{id}{RESOLVED}")).exists()
    };
    let mut id = base.clone();
    let mut n = 2;
    while taken(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let mut kept = r.clone();
    kept.id = id.clone();
    let path = folder.join(format!("{id}.yaml"));
    crate::atomic::write(&path, crate::yaml::to_string(&kept)?)?;
    Ok(path)
}

/// Every record with its answer, oldest first, and the names of the files that could not be read.
pub type Records = (Vec<(Prepared, Option<Resolution>)>, Vec<String>);

/// Every prepared record and its answer, oldest first. A file that cannot be read is skipped and
/// named in the second list.
pub fn read_all(store: &Store, p: &str) -> Result<Records> {
    let mut records: Vec<(PathBuf, Prepared)> = Vec::new();
    let mut answers: std::collections::BTreeMap<String, Resolution> = Default::default();
    let mut bad = Vec::new();
    let Ok(files) = std::fs::read_dir(dir(store, p)) else {
        return Ok((Vec::new(), bad));
    };
    for f in files.flatten() {
        let path = f.path();
        let name = f.file_name().to_string_lossy().to_string();
        if !name.ends_with(".yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).ok();
        if let Some(id) = name.strip_suffix(RESOLVED) {
            match text.and_then(|s| crate::yaml::from_str::<Resolution>(&s).ok()) {
                Some(r) => {
                    answers.insert(id.to_string(), r);
                }
                None => bad.push(path.display().to_string()),
            }
            continue;
        }
        let id = name.trim_end_matches(".yaml").to_string();
        match text.and_then(|s| crate::yaml::from_str::<Prepared>(&s).ok()) {
            Some(mut r) => {
                // The file's name is the id: a record is found by it.
                r.id = id;
                records.push((path, r));
            }
            None => bad.push(path.display().to_string()),
        }
    }
    // Within one second, `<id>` before `<id>-2` before `<id>-10`.
    records.sort_by(|(_, a), (_, b)| {
        crate::cmp_instants(&a.at, &b.at)
            .then_with(|| a.id.len().cmp(&b.id.len()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok((
        records
            .into_iter()
            .map(|(_, r)| {
                let answer = answers.remove(&r.id);
                (r, answer)
            })
            .collect(),
        bad,
    ))
}

/// Keep the person's answer to record `r.id`, once. Refused when the record does not exist or was
/// already answered.
pub fn resolve(store: &Store, p: &str, r: &Resolution) -> Result<()> {
    let folder = dir(store, p);
    if r.id.is_empty() || !folder.join(format!("{}.yaml", r.id)).exists() {
        return Err(unsupported(format!("there is no prepared record {}", r.id)));
    }
    let path = folder.join(format!("{}{RESOLVED}", r.id));
    if path.exists() {
        return Err(unsupported(format!("{} was already answered", r.id)));
    }
    crate::atomic::write(&path, crate::yaml::to_string(r)?)?;
    Ok(())
}

/// Where a record stands (pure): answered; a kind this version does not know; replaced by a newer
/// open record for the same decision; no longer waiting (`waiting` says why); or open. The second
/// value says why when it is not open or answered.
pub fn state_of(
    record: &Prepared,
    resolution: Option<&Resolution>,
    newer_open: bool,
    waiting: std::result::Result<(), String>,
) -> (State, String) {
    if let Some(r) = resolution {
        return match r.verdict {
            Answer::Confirmed => (State::Confirmed, String::new()),
            Answer::Rejected => (State::Rejected, String::new()),
        };
    }
    if kind(&record.kind).is_none() {
        return (
            State::UnknownKind,
            format!(
                "'{}' is a kind this version of kanbanr does not know",
                record.kind
            ),
        );
    }
    if newer_open {
        return (
            State::Superseded,
            "a newer prepared record for the same decision replaces it".into(),
        );
    }
    match waiting {
        Err(why) => (State::Stale, why),
        Ok(()) => (State::Open, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "9f078bd0c1e2aaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    pub(crate) fn scratch() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-prepared-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::config::ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn gate_record(at: &str) -> Prepared {
        let mut fields = Map::new();
        fields.insert("passed".into(), Value::Bool(true));
        Prepared {
            kind: "gate-result".into(),
            subject: Subject {
                gate: "ci".into(),
                commit: SHA.into(),
                ..Default::default()
            },
            fields,
            sources: vec![Source {
                label: "CI run 4512".into(),
                url: "https://ci.example/run/4512".into(),
                ..Default::default()
            }],
            by: "Claude".into(),
            origin: "conversation".into(),
            at: at.into(),
            rev: SHA.into(),
            ..Default::default()
        }
    }

    /// FTR-034 R-1: a source is a link with no user info or a board reference, and nothing else.
    #[test]
    fn sources_are_links_or_board_references_only() {
        let s = parse_source("CI run 4512=https://ci.example/run/4512").unwrap();
        assert_eq!(
            (s.label.as_str(), s.url.as_str()),
            ("CI run 4512", "https://ci.example/run/4512")
        );
        // A bare link keeps its query, `=` and all.
        let s = parse_source("https://ci.example/run?id=4512").unwrap();
        assert_eq!(
            (s.label.as_str(), s.url.as_str()),
            ("link", "https://ci.example/run?id=4512")
        );
        let s = parse_source(&format!("commit:{SHA}")).unwrap();
        assert_eq!(s.label, "commit");
        let s = parse_source("doc:design/data-model.md").unwrap();
        assert_eq!(s.label, "design/data-model.md");
        let s = parse_source("item:FTR-034").unwrap();
        assert_eq!(s.label, "FTR-034");
        let s = parse_source("what the person said in the conversation").unwrap();
        assert_eq!(s.url, "");
        for refused in [
            "run=javascript:alert(1)",
            "run=https://user:token@ci.example/run/1",
            "https://token@ci.example/",
            "run=ftp://x/y",
            "doc=doc:../secrets",
            "c=commit:xyz",
            "",
        ] {
            assert!(parse_source(refused).is_err(), "accepted: {refused}");
        }
        assert!(parse_source(&"x".repeat(MAX_LABEL + 1)).is_err());
        assert!(parse_source(&"x".repeat(MAX_LABEL)).is_ok());
    }

    /// FTR-034 R-3: a newer open record for the same decision supersedes an older one; an answered
    /// record reads as its answer, and one whose decision moved on reads stale with why.
    #[test]
    fn a_newer_record_supersedes_an_open_one_and_a_decided_one_reads_stale() {
        let (store, _dir) = scratch();
        let first = write_record(&store, "demo", &gate_record("2026-11-12T10:15:00Z")).unwrap();
        let second = write_record(&store, "demo", &gate_record("2026-11-12T10:15:00Z")).unwrap();
        let stem = |p: &PathBuf| p.file_stem().unwrap().to_string_lossy().to_string();
        assert_eq!(stem(&first), "20261112T101500Z-gate-result");
        assert_eq!(stem(&second), "20261112T101500Z-gate-result-2");
        let (all, bad) = read_all(&store, "demo").unwrap();
        assert!(bad.is_empty());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0.id, stem(&first));
        let (older, newer) = (&all[0].0, &all[1].0);
        assert_eq!(state_of(older, None, true, Ok(())).0, State::Superseded);
        assert_eq!(state_of(newer, None, false, Ok(())).0, State::Open);
        let (state, why) = state_of(
            newer,
            None,
            false,
            Err("a result was recorded since".into()),
        );
        assert_eq!(
            (state, why.as_str()),
            (State::Stale, "a result was recorded since")
        );
        // An answer wins over everything else, and is written once.
        let answer = Resolution {
            id: newer.id.clone(),
            verdict: Answer::Rejected,
            by: "Ada".into(),
            origin: "monitor".into(),
            at: "2026-11-12T10:20:00Z".into(),
            changed: vec![],
            reason: String::new(),
            made: String::new(),
            extra: Default::default(),
        };
        resolve(&store, "demo", &answer).unwrap();
        assert!(resolve(&store, "demo", &answer).is_err());
        let (all, _) = read_all(&store, "demo").unwrap();
        let (r, a) = &all[1];
        assert_eq!(
            state_of(r, a.as_ref(), false, Err("x".into())).0,
            State::Rejected
        );
        // A kind a newer kanbanr added is listed, not refused.
        let mut other = gate_record("2026-11-12T10:16:00Z");
        other.kind = "from-the-future".into();
        assert_eq!(state_of(&other, None, false, Ok(())).0, State::UnknownKind);
        // An unreadable file is skipped and named.
        std::fs::write(
            store.project_dir("demo").join("prepared/broken.yaml"),
            "id: [",
        )
        .unwrap();
        let (all, bad) = read_all(&store, "demo").unwrap();
        assert_eq!((all.len(), bad.len()), (2, 1));
    }
}
