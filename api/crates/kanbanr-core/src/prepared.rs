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

// ---- preparing -----------------------------------------------------------------------------

fn text_of(v: &Value) -> Option<&str> {
    v.as_str()
}

/// Is `v` a value field `f` may hold? Returns why not.
fn check_field(kind: &str, f: &FieldSpec, v: &Value) -> Result<()> {
    let ok = match f.ty {
        FieldType::Text => text_of(v).is_some_and(|t| t.chars().count() <= MAX_TEXT),
        FieldType::Bool => v.is_boolean(),
        FieldType::State => {
            text_of(v).is_some_and(|t| crate::models::TestState::parse(t).is_some())
        }
    };
    if ok {
        return Ok(());
    }
    let what = match f.ty {
        FieldType::Text => format!("text of at most {MAX_TEXT} characters"),
        FieldType::Bool => "true or false".to_string(),
        FieldType::State => "planned, red or green".to_string(),
    };
    Err(unsupported(format!(
        "a prepared {kind}'s '{}' is {what}",
        f.name
    )))
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Record the evidence for a decision, as `POST /projects/{p}/prepared` takes it:
/// `{kind, subject: {…}, fields: {…}, sources: [{label, url}], by?}`, with `origin` stamped by the
/// front end. The kind must be known, its subject's keys present, its fields known and typed, and
/// at least one source named (a gate result's `url` counts as one). The decision must be one that
/// waits: the item and its definition exist; the release exists, is not shipped and its next move
/// is a confirmation after the tag; the gate's name and the full commit are well formed; a
/// ratification has a bypass to reconcile. `by` is `Claude` when the origin is the conversation.
/// Nothing a gate reads changes.
pub fn prepare(
    store: &Store,
    p: &str,
    body: &Value,
    repo: Option<&std::path::Path>,
) -> Result<Prepared> {
    let kind_name = body
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let spec = kind(&kind_name).ok_or_else(|| {
        unsupported(format!(
            "'{kind_name}' is not a kind of prepared record: it is one of {}",
            KINDS.iter().map(|k| k.kind).collect::<Vec<_>>().join(", ")
        ))
    })?;
    // The subject: only the keys the kind has, trimmed.
    let mut subject = Subject::default();
    if let Some(given) = body.get("subject") {
        let Some(given) = given.as_object() else {
            return Err(unsupported(
                "a prepared record's subject is an object".into(),
            ));
        };
        for (k, v) in given {
            if !spec.subject.contains(&k.as_str()) {
                return Err(unsupported(format!(
                    "a prepared {kind_name} has no subject '{k}': it names {}",
                    spec.subject.join(", ")
                )));
            }
            let v = v.as_str().unwrap_or_default().trim().to_string();
            subject.set(k, v);
        }
    }
    // A promotion with no stage is the plan's next one.
    let project = store.load(p)?;
    let mut plan = None;
    if kind_name == "promotion" && !subject.release.is_empty() {
        let releases = crate::releases::load(store, p)?;
        let release = releases
            .iter()
            .find(|r| r.version == subject.release)
            .ok_or_else(|| unsupported(format!("there is no release {}", subject.release)))?;
        if release.state == crate::releases::ReleaseState::Shipped {
            return Err(unsupported(format!(
                "{} has shipped: there is no promotion left to prepare",
                release.version
            )));
        }
        let to = (!subject.stage.is_empty()).then_some(subject.stage.as_str());
        let worked = crate::releases::plan_promotion(
            store,
            p,
            &subject.release,
            to,
            "",
            repo,
            crate::releases::Fetch::No,
        )?;
        if subject.stage.is_empty() {
            subject.stage = worked.to.clone();
        }
        plan = Some(worked);
    }
    let missing: Vec<&str> = spec
        .subject
        .iter()
        .copied()
        .filter(|k| subject.get(k).is_empty())
        .collect();
    if !missing.is_empty() {
        return Err(unsupported(format!(
            "a prepared {kind_name} names its {}",
            missing.join(" and ")
        )));
    }
    // The fields: known, typed, the required ones present.
    let mut fields = Map::new();
    if let Some(given) = body.get("fields") {
        let Some(given) = given.as_object() else {
            return Err(unsupported(
                "a prepared record's fields are an object".into(),
            ));
        };
        for (k, v) in given {
            let f = spec.fields.iter().find(|f| f.name == k).ok_or_else(|| {
                unsupported(format!(
                    "a prepared {kind_name} has no field '{k}': it carries {}",
                    spec.fields
                        .iter()
                        .map(|f| f.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            let v = match v {
                Value::String(t) => Value::String(t.trim().to_string()),
                other => other.clone(),
            };
            check_field(&kind_name, f, &v)?;
            if v.as_str().is_some_and(str::is_empty) {
                continue;
            }
            fields.insert(k.clone(), v);
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        if !fields.contains_key(f.name) {
            return Err(unsupported(format!(
                "a prepared {kind_name} needs '{}'",
                f.name
            )));
        }
    }
    // The sources: at least one, each a link or a board reference.
    let mut sources: Vec<Source> = match body.get("sources") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => {
            serde_json::from_value(v.clone()).map_err(|e| unsupported(format!("sources: {e}")))?
        }
    };
    for s in &mut sources {
        s.label = s.label.trim().to_string();
        s.url = s.url.trim().to_string();
        check_source(s)?;
    }
    if kind_name == "gate-result"
        && let Some(url) = fields.get("url").and_then(Value::as_str)
    {
        if !is_web(url) {
            return Err(unsupported(format!(
                "the link to the run must start with http:// or https://: '{url}'"
            )));
        }
        check_url(url).map_err(unsupported)?;
        if !sources.iter().any(|s| s.url == url) {
            sources.push(Source {
                label: "the run".into(),
                url: url.to_string(),
                ..Default::default()
            });
        }
    }
    if sources.is_empty() {
        return Err(unsupported(format!(
            "a prepared {kind_name} names at least one source: the run's link, the commit, the \
             doc or what the person said"
        )));
    }
    if sources.len() > MAX_SOURCES {
        return Err(unsupported(format!(
            "a prepared record names at most {MAX_SOURCES} sources"
        )));
    }
    // The decision must be one that waits, and the record is pinned to what it was prepared on.
    let definition = |code: &str| -> Result<crate::models::FeatureDefinition> {
        project.feature(code)?.definition.clone().ok_or_else(|| {
            unsupported(format!(
                "{code} has no definition: there is no decision to prepare"
            ))
        })
    };
    let rev = match kind_name.as_str() {
        "signoff" => definition(&subject.item)?.content_rev(),
        "ratification" => {
            let def = definition(&subject.item)?;
            if def.started_unapproved.trim().is_empty() || def.approval_state().is_agreed() {
                return Err(unsupported(format!(
                    "{} has no unreconciled bypass: there is nothing to ratify",
                    subject.item
                )));
            }
            def.content_rev()
        }
        "gate-result" => {
            crate::gates::check_name(&subject.gate).map_err(unsupported)?;
            if !is_full_sha(&subject.commit) {
                return Err(unsupported(format!(
                    "'{}' is not a full commit: a gate result names the 40-digit hash",
                    subject.commit
                )));
            }
            subject.commit.clone()
        }
        "promotion" => {
            let plan = plan
                .as_ref()
                .expect("a promotion's plan is worked out above");
            if !plan.confirm {
                return Err(unsupported(format!(
                    "promoting {} to {} is not a confirmation after the tag: it has one button \
                     and nothing to fill in, so there is nothing to prepare",
                    subject.release, subject.stage
                )));
            }
            plan.release_tip.clone()
        }
        _ => String::new(),
    };
    let origin = crate::origin::from_body(body);
    let by = if origin == crate::origin::Origin::Conversation.as_str() {
        "Claude".to_string()
    } else {
        body.get("by")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    if by.is_empty() {
        return Err(unsupported(
            "a prepared record names who prepared it: pass `by`".into(),
        ));
    }
    let record = Prepared {
        id: String::new(),
        kind: kind_name,
        subject,
        fields,
        sources,
        by,
        origin,
        at: crate::now_rfc3339(),
        rev,
        extra: Default::default(),
    };
    let path = write_record(store, p, &record)?;
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Prepared { id, ..record })
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

// ---- listing -------------------------------------------------------------------------------

/// A prepared record as the monitor lists it: the record, where it stands, the answer if any, and
/// what the person would type in their own terminal instead.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PreparedView {
    #[serde(flatten)]
    pub record: Prepared,
    pub state: State,
    pub resolution: Option<Resolution>,
    /// Why it no longer waits, when stale, superseded or of an unknown kind.
    pub stale_why: String,
    /// The monitor page its decision is taken on: `review` or `releases`.
    pub page: String,
    /// The equivalent verdict command, for a person's own terminal.
    pub command: String,
}

/// Which records [`list`] answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Every record, not only the open ones.
    pub all: bool,
    /// Only records about this item.
    pub item: Option<String>,
    /// Only records about this release.
    pub release: Option<String>,
}

/// The monitor page a kind's decision is taken on.
pub fn page_of(kind: &str) -> &'static str {
    match kind {
        "promotion" | "gate-result" => "releases",
        _ => "review",
    }
}

/// `s` as one shell word.
fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/:@=+-,".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn field_text<'a>(r: &'a Prepared, name: &str) -> &'a str {
    r.fields.get(name).and_then(Value::as_str).unwrap_or("")
}

/// The verdict command a person would run in their own terminal to take the decision `r`
/// prepares, with what it prepared. Empty for a kind with no command.
pub fn equivalent_command(r: &Prepared) -> String {
    let mut words: Vec<String> = vec!["kanbanr".into()];
    let flag = |words: &mut Vec<String>, name: &str, value: &str| {
        if !value.is_empty() {
            words.push(format!("--{name}"));
            words.push(quote(value));
        }
    };
    match r.kind.as_str() {
        "signoff" => {
            words.extend([
                "signoff".into(),
                quote(&r.subject.item),
                quote(&r.subject.signoff),
            ]);
            flag(&mut words, "note", field_text(r, "note"));
            flag(&mut words, "doc", field_text(r, "doc"));
            flag(&mut words, "rev", &r.rev);
        }
        "ratification" => {
            words.extend(["ratify".into(), quote(&r.subject.item)]);
            flag(&mut words, "reason", field_text(r, "reason"));
            flag(&mut words, "rev", &r.rev);
        }
        "gate-result" => {
            let passed = r
                .fields
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            words.extend([
                "gate".into(),
                "record".into(),
                quote(&r.subject.gate),
                if passed { "--passed" } else { "--failed" }.into(),
            ]);
            flag(&mut words, "commit", &r.subject.commit);
            flag(&mut words, "url", field_text(r, "url"));
            flag(&mut words, "note", field_text(r, "note"));
        }
        "promotion" => {
            words.extend([
                "release".into(),
                "promote".into(),
                quote(&r.subject.release),
                quote(&r.subject.stage),
            ]);
            flag(&mut words, "note", field_text(r, "checked"));
        }
        _ => return String::new(),
    }
    words.join(" ")
}

/// Does the decision `r` prepares still wait, as prepared? `Err` says why not. `project` is the
/// project loaded once for the request, with its specs.
pub fn still_waiting(
    store: &Store,
    p: &str,
    project: &crate::store::Project,
    r: &Prepared,
    repo: Option<&std::path::Path>,
) -> std::result::Result<(), String> {
    let changed = "the definition changed after it was prepared";
    match r.kind.as_str() {
        "signoff" => {
            let item = project
                .feature(&r.subject.item)
                .map_err(|_| format!("{} no longer exists", r.subject.item))?;
            if let Some(s) = item.signoff_current(&r.subject.signoff) {
                return Err(format!(
                    "{} was signed off by {} on {}",
                    r.subject.signoff,
                    s.by,
                    s.at.chars().take(10).collect::<String>()
                ));
            }
            let now = item.definition.as_ref().map(|d| d.content_rev());
            if now.as_deref() != Some(r.rev.as_str()) {
                return Err(changed.into());
            }
            Ok(())
        }
        "ratification" => {
            let item = project
                .feature(&r.subject.item)
                .map_err(|_| format!("{} no longer exists", r.subject.item))?;
            let Some(def) = item.definition.as_ref() else {
                return Err(changed.into());
            };
            if def.started_unapproved.trim().is_empty() || def.approval_state().is_agreed() {
                return Err(format!("{} has no bypass left to ratify", r.subject.item));
            }
            if def.content_rev() != r.rev {
                return Err(changed.into());
            }
            Ok(())
        }
        "gate-result" => {
            let results = crate::gates::results(store, p, Some(&r.subject.gate))
                .map_err(|e| e.to_string())?;
            let since = results.iter().find(|g| {
                g.commit == r.subject.commit
                    && crate::cmp_instants(&g.at, &r.at) == std::cmp::Ordering::Greater
            });
            match since {
                Some(g) => Err(format!(
                    "{} {} was recorded for {} by {} since",
                    g.name,
                    if g.passed { "passed" } else { "failed" },
                    g.commit.chars().take(7).collect::<String>(),
                    g.by
                )),
                None => Ok(()),
            }
        }
        "promotion" => {
            let releases = crate::releases::load(store, p).map_err(|e| e.to_string())?;
            let Some(release) = releases.iter().find(|x| x.version == r.subject.release) else {
                return Err(format!("{} no longer exists", r.subject.release));
            };
            if release.state == crate::releases::ReleaseState::Shipped {
                return Err(format!("{} has shipped", release.version));
            }
            let stages = project.config.cadence.stages();
            let at = stages.iter().position(|s| *s == release.stage);
            let to = stages.iter().position(|s| *s == r.subject.stage);
            if let (Some(at), Some(to)) = (at, to)
                && at >= to
            {
                return Err(format!(
                    "{} is already at {}",
                    release.version, release.stage
                ));
            }
            if let Some(repo) = repo
                && !r.rev.is_empty()
            {
                let branch = if release.branch.is_empty() {
                    format!("release/{}", release.version)
                } else {
                    release.branch.clone()
                };
                let tip = crate::coderepo::rev_parse(repo, &branch).unwrap_or_default();
                if tip != r.rev {
                    return Err("the release branch moved after it was prepared".into());
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Every record with where it stands, oldest first.
fn standing(store: &Store, p: &str, repo: Option<&std::path::Path>) -> Result<Vec<PreparedView>> {
    let (all, _) = read_all(store, p)?;
    let project = store.load(p)?;
    let mut out = Vec::with_capacity(all.len());
    for (i, (r, answer)) in all.iter().enumerate() {
        let newer_open = all[i + 1..]
            .iter()
            .any(|(n, a)| a.is_none() && n.kind == r.kind && n.subject == r.subject);
        // Only an unanswered record of a known kind needs the decision looked up.
        let waiting = if answer.is_none() && !newer_open && kind(&r.kind).is_some() {
            still_waiting(store, p, &project, r, repo)
        } else {
            Ok(())
        };
        let (state, stale_why) = state_of(r, answer.as_ref(), newer_open, waiting);
        out.push(PreparedView {
            record: r.clone(),
            state,
            resolution: answer.clone(),
            stale_why,
            page: page_of(&r.kind).to_string(),
            command: equivalent_command(r),
        });
    }
    Ok(out)
}

/// The prepared records `query` asks for, newest first (`GET /projects/{p}/prepared`).
pub fn list(
    store: &Store,
    p: &str,
    query: &ListQuery,
    repo: Option<&std::path::Path>,
) -> Result<Vec<PreparedView>> {
    let mut views = standing(store, p, repo)?;
    views.retain(|v| {
        (query.all || v.state == State::Open)
            && query
                .item
                .as_ref()
                .is_none_or(|i| &v.record.subject.item == i)
            && query
                .release
                .as_ref()
                .is_none_or(|x| &v.record.subject.release == x)
    });
    views.reverse();
    Ok(views)
}

// ---- confirming and rejecting -----------------------------------------------------------------

/// `(project, id, "confirm" | "reject")` for a route that answers a prepared record.
pub fn answer_route(method: &str, path: &str) -> Option<(String, String, String)> {
    if !method.eq_ignore_ascii_case("POST") {
        return None;
    }
    let seg: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match seg.as_slice() {
        [
            "projects",
            p,
            "prepared",
            id,
            action @ ("confirm" | "reject"),
        ] => Some((p.to_string(), id.to_string(), action.to_string())),
        _ => None,
    }
}

/// The verdict a confirmation makes, and the answer to keep once it is made.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub method: String,
    pub path: String,
    pub body: Value,
    pub resolution: Resolution,
}

/// The open record `id`, or why it cannot be answered.
fn open_record(
    store: &Store,
    p: &str,
    id: &str,
    repo: Option<&std::path::Path>,
) -> Result<Prepared> {
    let views = standing(store, p, repo)?;
    let Some(v) = views.into_iter().find(|v| v.record.id == id) else {
        return Err(unsupported(format!("there is no prepared record {id}")));
    };
    match v.state {
        State::Open => Ok(v.record),
        State::Confirmed | State::Rejected => {
            let a = v.resolution.unwrap_or_else(|| unreachable!("answered"));
            Err(unsupported(format!(
                "{id} was already {} by {} on {}",
                if a.verdict == Answer::Confirmed {
                    "confirmed"
                } else {
                    "rejected"
                },
                a.by,
                a.at.chars().take(19).collect::<String>().replace('T', " ")
            )))
        }
        _ => Err(unsupported(format!(
            "{id} no longer waits: {}. Ask Claude to prepare it again",
            v.stale_why
        ))),
    }
}

/// What a confirmation makes (FTR-034): the guard first, then the record must be open and of a
/// kind confirmed alone; `edits` may change only the kind's editable fields, each to a value of its
/// type, and each change is kept; the verdict body is built from the record, with `by` and the
/// pins (`rev`, `spec_rev`, `expect`) from the confirmation. Called by the write path before the
/// origin is stamped, so the verdict then goes through its own route exactly as a direct one does.
pub fn expand(
    store: &Store,
    p: &str,
    id: &str,
    body: &Value,
    origin: crate::origin::Origin,
) -> Result<Expanded> {
    use crate::origin::{Taker, Verdict, verdict_refusal};
    if let Some(why) = verdict_refusal(Verdict::ConfirmPrepared, Taker::of_origin(origin)) {
        return Err(CoreError::Refused(why));
    }
    let record = open_record(store, p, id, None)?;
    if record.kind == "text-check" {
        return Err(unsupported(
            "a text check is confirmed by the docs-folded sign-off".into(),
        ));
    }
    let spec = kind(&record.kind)
        .ok_or_else(|| unsupported(format!("{id} is a kind this version does not know")))?;
    let by = body
        .get("by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| {
            unsupported(
                "a confirmation must name who gave it: pass `by` (the monitor sends the board's \
                 commit identity)"
                    .into(),
            )
        })?
        .to_string();
    let mut fields = record.fields.clone();
    let mut changed = Vec::new();
    if let Some(edits) = body.get("edits").filter(|e| !e.is_null()) {
        let Some(edits) = edits.as_object() else {
            return Err(unsupported("edits are an object of field: value".into()));
        };
        for (k, v) in edits {
            let f = spec
                .fields
                .iter()
                .find(|f| f.name == k && f.editable)
                .ok_or_else(|| {
                    unsupported(format!(
                        "'{k}' is not a field the person can change on a prepared {}: they are {}",
                        record.kind,
                        spec.fields
                            .iter()
                            .filter(|f| f.editable)
                            .map(|f| f.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?;
            let v = match v {
                Value::String(t) => Value::String(t.trim().to_string()),
                other => other.clone(),
            };
            check_field(&record.kind, f, &v)?;
            let before = fields.get(k.as_str()).cloned().unwrap_or(Value::Null);
            let same = before == v || (before.is_null() && v.as_str() == Some(""));
            if same {
                continue;
            }
            if f.required && v.as_str() == Some("") {
                return Err(unsupported(format!(
                    "a {}'s '{k}' cannot be emptied",
                    record.kind
                )));
            }
            changed.push(FieldChange {
                field: k.clone(),
                prepared: before,
                confirmed: v.clone(),
                extra: Default::default(),
            });
            fields.insert(k.clone(), v);
        }
    }
    let text = |k: &str| {
        fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let pin = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    let s = &record.subject;
    let (path, mut verdict) = match record.kind.as_str() {
        "signoff" => (
            format!("/projects/{p}/features/{}/signoff/{}", s.item, s.signoff),
            serde_json::json!({"by": by, "note": text("note"), "doc": text("doc")}),
        ),
        "ratification" => (
            format!("/projects/{p}/features/{}/ratify", s.item),
            serde_json::json!({"by": by, "reason": text("reason")}),
        ),
        "gate-result" => {
            let mut v = serde_json::json!({
                "name": s.gate, "commit": s.commit,
                "passed": fields.get("passed").and_then(Value::as_bool).unwrap_or(false),
                "by": by,
            });
            for k in ["url", "note"] {
                if !text(k).is_empty() {
                    v[k] = Value::String(text(k));
                }
            }
            (format!("/projects/{p}/gate-results"), v)
        }
        "promotion" => (
            format!("/projects/{p}/releases/{}/promote", s.release),
            serde_json::json!({"by": by, "stage": s.stage, "reason": text("checked")}),
        ),
        other => {
            return Err(unsupported(format!(
                "a prepared {other} is not confirmed on its own"
            )));
        }
    };
    for k in ["rev", "spec_rev", "expect"] {
        if let Some(v) = pin(k) {
            verdict[k] = Value::String(v.to_string());
        }
    }
    let resolution = Resolution {
        id: id.to_string(),
        verdict: Answer::Confirmed,
        by,
        origin: origin.as_str().to_string(),
        at: crate::now_rfc3339(),
        changed,
        reason: String::new(),
        made: format!("POST {path}"),
        extra: Default::default(),
    };
    Ok(Expanded {
        method: "POST".into(),
        path,
        body: verdict,
        resolution,
    })
}

/// Reject an open record (`POST /projects/{p}/prepared/{id}/reject`, `{by, reason?}`): the guard,
/// then the record must be open; the answer is kept and nothing else changes. `body` carries the
/// origin the front end stamped; a body with none is not taken as the person's.
pub fn reject(store: &Store, p: &str, id: &str, body: &Value) -> Result<Resolution> {
    use crate::origin::{Origin, Taker, Verdict, verdict_refusal};
    let origin = Origin::parse(&crate::origin::from_body(body));
    let taker = origin.map(Taker::of_origin).unwrap_or(Taker::Claude);
    if let Some(why) = verdict_refusal(Verdict::RejectPrepared, taker) {
        return Err(CoreError::Refused(why));
    }
    let by = body
        .get("by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| unsupported("a rejection must name who gave it: pass `by`".into()))?
        .to_string();
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if reason.chars().count() > MAX_TEXT {
        return Err(unsupported(format!(
            "a rejection's reason is at most {MAX_TEXT} characters"
        )));
    }
    open_record(store, p, id, None)?;
    let resolution = Resolution {
        id: id.to_string(),
        verdict: Answer::Rejected,
        by,
        origin: origin.map(|o| o.as_str().to_string()).unwrap_or_default(),
        at: crate::now_rfc3339(),
        changed: Vec::new(),
        reason,
        made: String::new(),
        extra: Default::default(),
    };
    resolve(store, p, &resolution)?;
    Ok(resolution)
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

    fn apply(
        store: &Store,
        dir: &std::path::Path,
        method: &str,
        path: &str,
        body: Value,
        origin: crate::origin::Origin,
    ) -> std::result::Result<Value, crate::write::WriteError> {
        crate::write::apply_as(
            store,
            dir,
            crate::git::PushPolicy::Off,
            method,
            path,
            Some(&body),
            None,
            &crate::eventing::NullSender,
            &crate::agentscope::ProcessScope::Unscoped,
            origin,
        )
        .map(|a| serde_json::from_str(&a.body).unwrap())
    }

    /// A board under git with an author, and an item with a definition (with `bypass`, a recorded
    /// unapproved start).
    pub(crate) fn board_with_item(bypass: bool) -> (Store, PathBuf, String) {
        let (store, dir) = scratch();
        crate::git::ensure_repo_as(&dir, Some(("Ada L", "ada@example.com")));
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let f = store
            .add_feature("demo", "Cart", "spec", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    started_unapproved: if bypass {
                        "urgent".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                }),
            )
            .unwrap();
        (store, dir, f.code)
    }

    /// FTR-034 R-1: Claude records a prepared CI result with its run link, commit and conclusion;
    /// the file says it is Claude's, from the conversation, and names the run as a source.
    #[test]
    fn claude_records_a_prepared_ci_result_with_its_run_link_commit_and_conclusion() {
        let (store, dir, _) = board_with_item(false);
        let out = apply(
            &store,
            &dir,
            "POST",
            "/projects/demo/prepared",
            serde_json::json!({
                "kind": "gate-result",
                "subject": {"gate": "ci", "commit": SHA},
                "fields": {"passed": true, "url": "https://ci.example/run/4512",
                           "note": "all 412 tests passed"},
                "by": "Ada L",
                "origin": "monitor",
            }),
            crate::origin::Origin::Conversation,
        )
        .unwrap();
        let id = out["id"].as_str().unwrap().to_string();
        assert!(id.ends_with("-gate-result"), "{id}");
        let file = store
            .project_dir("demo")
            .join(format!("prepared/{id}.yaml"));
        let kept: Prepared =
            crate::yaml::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(kept.by, "Claude");
        assert_eq!(kept.origin, "conversation");
        assert_eq!(kept.subject.commit, SHA);
        assert_eq!(kept.rev, SHA);
        assert_eq!(kept.fields["passed"], Value::Bool(true));
        assert!(
            kept.sources
                .iter()
                .any(|s| s.url == "https://ci.example/run/4512"),
            "{:?}",
            kept.sources
        );
        // Preparing is not a verdict: no gate result was recorded.
        assert!(
            crate::gates::results(&store, "demo", None)
                .unwrap()
                .is_empty()
        );
    }

    /// FTR-034 R-1: a prepared record needs a source, a known kind, its subject and only the
    /// fields its kind carries, each of its type.
    #[test]
    fn a_prepared_record_needs_a_source_and_known_fields() {
        let (store, _dir, code) = board_with_item(true);
        let go = |body: Value| {
            let mut b = body;
            b["origin"] = Value::String("conversation".into());
            prepare(&store, "demo", &b, None)
        };
        let src = serde_json::json!([{"label": "the conversation"}]);
        let refused = |body: Value, words: &str| {
            let e = go(body).unwrap_err().to_string();
            assert!(e.contains(words), "{e}");
        };
        refused(
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "design-review"}}),
            "at least one source",
        );
        refused(
            serde_json::json!({"kind": "verdict", "subject": {}, "sources": src}),
            "not a kind",
        );
        refused(
            serde_json::json!({"kind": "signoff", "subject": {"item": code}, "sources": src}),
            "names its signoff",
        );
        refused(
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "x"},
                               "fields": {"passed": true}, "sources": src}),
            "no field 'passed'",
        );
        refused(
            serde_json::json!({"kind": "gate-result", "subject": {"gate": "ci", "commit": SHA},
                               "fields": {"passed": "yes"}, "sources": src}),
            "true or false",
        );
        refused(
            serde_json::json!({"kind": "gate-result", "subject": {"gate": "ci", "commit": "9f078bd"},
                               "fields": {"passed": true}, "sources": src}),
            "full commit",
        );
        refused(
            serde_json::json!({"kind": "ratification", "subject": {"item": code}, "sources": src}),
            "needs 'reason'",
        );
        refused(
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "x"},
                               "fields": {"note": "x".repeat(MAX_TEXT + 1)}, "sources": src}),
            "at most",
        );
        refused(
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "x"},
                               "sources": [{"label": "run", "url": "javascript:alert(1)"}]}),
            "not a source",
        );
        // A good one is kept, pinned to the definition it was prepared on.
        let r = go(
            serde_json::json!({"kind": "ratification", "subject": {"item": code},
                                      "fields": {"reason": "built under the outage"},
                                      "sources": src}),
        )
        .unwrap();
        let def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        assert_eq!(r.rev, def.content_rev());
        assert_eq!(r.by, "Claude");
        // Preparing changed nothing the gates read.
        assert!(!def.approval_state().is_agreed());
    }

    /// A prepared CI result for `SHA`, written as Claude would; its id.
    fn prepared_gate(store: &Store, dir: &std::path::Path) -> String {
        let out = apply(
            store,
            dir,
            "POST",
            "/projects/demo/prepared",
            serde_json::json!({
                "kind": "gate-result",
                "subject": {"gate": "ci", "commit": SHA},
                "fields": {"passed": true, "url": "https://ci.example/run/4512",
                           "note": "all 412 tests passed"},
            }),
            crate::origin::Origin::Conversation,
        )
        .unwrap();
        out["id"].as_str().unwrap().to_string()
    }

    /// Every file under `dir`, with its bytes.
    fn files(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if !p.to_string_lossy().contains(".lock") {
                    out.push((p.clone(), std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    /// FTR-034 R-4: Claude can neither confirm nor reject a prepared record; nothing is written.
    #[test]
    fn claudes_confirmation_of_a_prepared_record_is_refused() {
        let (store, dir, _) = board_with_item(false);
        let id = prepared_gate(&store, &dir);
        let before = files(&dir);
        for (action, body) in [
            ("confirm", serde_json::json!({"by": "Ada L"})),
            ("reject", serde_json::json!({"by": "Ada L", "reason": "no"})),
        ] {
            let path = format!("/projects/demo/prepared/{id}/{action}");
            match apply(
                &store,
                &dir,
                "POST",
                &path,
                body,
                crate::origin::Origin::Conversation,
            ) {
                Err(crate::write::WriteError::Refused(why)) => {
                    assert!(why.contains("never by Claude"), "{why}")
                }
                other => panic!("{action} was not refused: {other:?}"),
            }
        }
        assert_eq!(
            files(&dir),
            before,
            "a refused confirmation wrote something"
        );
        assert!(
            crate::gates::results(&store, "demo", None)
                .unwrap()
                .is_empty()
        );
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

        // Listed against the board: a decision taken some other way leaves the record stale.
        let (store, dir, code) = board_with_item(true);
        let gate = prepared_gate(&store, &dir);
        let signoff = prepare_as_claude(
            &store,
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "design-review"},
                               "fields": {"note": "held on Tuesday"},
                               "sources": [{"label": "minutes", "url": "doc:minutes/tuesday.md"}]}),
        );
        let ratify = prepare_as_claude(
            &store,
            serde_json::json!({"kind": "ratification", "subject": {"item": code},
                               "fields": {"reason": "built in the outage"},
                               "sources": [{"label": "what the person said"}]}),
        );
        let open = list(&store, "demo", &ListQuery::default(), None).unwrap();
        assert_eq!(open.len(), 3, "{open:#?}");
        // Newest first, each naming its page and its command.
        assert_eq!(open[0].record.id, ratify);
        assert_eq!(open[2].page, "releases");
        assert!(
            open[2]
                .command
                .starts_with(&format!("kanbanr gate record ci --passed --commit {SHA}")),
            "{}",
            open[2].command
        );
        // The person records the result in their own terminal; the sign-off is given; the
        // definition changes, so the ratification was prepared on other text.
        let mut r = crate::gates::GateResult {
            name: "ci".into(),
            commit: SHA.into(),
            passed: true,
            by: "Ada L".into(),
            at: "2999-01-01T00:00:00Z".into(),
            source: "person".into(),
            ..Default::default()
        };
        crate::gates::record(&store, "demo", &r).unwrap();
        r.at = String::new();
        store
            .signoff_feature_checked_as(
                "demo",
                &code,
                "design-review",
                "Ada L",
                "",
                "",
                "terminal",
                &[],
            )
            .unwrap();
        let stale = |id: &str| {
            let all = list(
                &store,
                "demo",
                &ListQuery {
                    all: true,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
            let v = all.iter().find(|v| v.record.id == id).unwrap().clone();
            assert_eq!(v.state, State::Stale, "{v:#?}");
            v.stale_why
        };
        assert!(stale(&gate).contains("recorded"), "{}", stale(&gate));
        assert!(
            stale(&signoff).contains("signed off by Ada L"),
            "{}",
            stale(&signoff)
        );
        let mut def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        def.statement = "Keep a cart for 30 days".into();
        store
            .set_feature_definition("demo", &code, Some(def))
            .unwrap();
        assert!(
            list(&store, "demo", &ListQuery::default(), None)
                .unwrap()
                .is_empty()
        );
        assert!(
            stale(&ratify).contains("definition changed"),
            "{}",
            stale(&ratify)
        );
        // Filters.
        let q = ListQuery {
            all: true,
            item: Some(code.clone()),
            release: None,
        };
        assert_eq!(list(&store, "demo", &q, None).unwrap().len(), 2);
    }

    /// Prepare `body` as Claude would, through the route; the record's id.
    fn prepare_as_claude(store: &Store, mut body: Value) -> String {
        body["origin"] = Value::String("conversation".into());
        prepare(store, "demo", &body, None).unwrap().id
    }

    fn confirm(
        store: &Store,
        dir: &std::path::Path,
        id: &str,
        body: Value,
    ) -> std::result::Result<Value, crate::write::WriteError> {
        apply(
            store,
            dir,
            "POST",
            &format!("/projects/demo/prepared/{id}/confirm"),
            body,
            crate::origin::Origin::Monitor,
        )
    }

    fn resolution_of(store: &Store, id: &str) -> Resolution {
        let path = store
            .project_dir("demo")
            .join(format!("prepared/{id}.resolved.yaml"));
        crate::yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// FTR-034 R-3: confirming a prepared sign-off, ratification and gate result makes the same
    /// record its verdict route makes, as the person, from the monitor.
    #[test]
    fn confirming_each_kind_makes_the_same_record_as_its_verdict_route() {
        let (store, dir, code) = board_with_item(true);
        let signoff = prepare_as_claude(
            &store,
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "design-review"},
                               "fields": {"note": "held on Tuesday", "doc": "minutes/tuesday.md"},
                               "sources": [{"label": "minutes", "url": "doc:minutes/tuesday.md"}]}),
        );
        let ratify = prepare_as_claude(
            &store,
            serde_json::json!({"kind": "ratification", "subject": {"item": code},
                               "fields": {"reason": "built in the outage"},
                               "sources": [{"label": "what the person said"}]}),
        );
        let gate = prepared_gate(&store, &dir);
        let rev = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap()
            .content_rev();

        let out = confirm(
            &store,
            &dir,
            &signoff,
            serde_json::json!({"by": "Ada L", "rev": rev}),
        )
        .unwrap();
        assert_eq!(out["prepared"], Value::String(signoff.clone()));
        let item = store.load("demo").unwrap().feature(&code).unwrap().clone();
        let s = item.signoff_current("design-review").unwrap();
        assert_eq!(
            (
                s.by.as_str(),
                s.note.as_str(),
                s.doc.as_str(),
                s.origin.as_str()
            ),
            ("Ada L", "held on Tuesday", "minutes/tuesday.md", "monitor")
        );
        let a = resolution_of(&store, &signoff);
        assert_eq!(
            (a.verdict, a.made.as_str()),
            (
                Answer::Confirmed,
                format!("POST /projects/demo/features/{code}/signoff/design-review").as_str()
            )
        );

        confirm(
            &store,
            &dir,
            &ratify,
            serde_json::json!({"by": "Ada L", "rev": rev}),
        )
        .unwrap();
        let def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        let last = def.approvals.last().unwrap();
        assert_eq!(
            (
                last.verdict.as_str(),
                last.by.as_str(),
                last.origin.as_str()
            ),
            ("ratified", "Ada L", "monitor")
        );

        confirm(&store, &dir, &gate, serde_json::json!({"by": "Ada L"})).unwrap();
        let results = crate::gates::results(&store, "demo", None).unwrap();
        assert_eq!(results.len(), 1);
        let g = &results[0];
        assert_eq!(
            (
                g.name.as_str(),
                g.commit.as_str(),
                g.passed,
                g.by.as_str(),
                g.url.as_str(),
                g.origin.as_str()
            ),
            (
                "ci",
                SHA,
                true,
                "Ada L",
                "https://ci.example/run/4512",
                "monitor"
            )
        );
        assert_eq!(g.note, "all 412 tests passed");
        // Each is answered now, and asked for no more.
        assert!(
            list(&store, "demo", &ListQuery::default(), None)
                .unwrap()
                .is_empty()
        );
        let again = confirm(&store, &dir, &gate, serde_json::json!({"by": "Ada L"})).unwrap_err();
        assert!(
            again.to_string().contains("already confirmed by Ada L"),
            "{again}"
        );
        // A pin the verdict route checks still holds: a stale `rev` is that route's refusal.
        let (store, dir, code) = board_with_item(false);
        let s2 = prepare_as_claude(
            &store,
            serde_json::json!({"kind": "signoff", "subject": {"item": code, "signoff": "design-review"},
                               "sources": [{"label": "the conversation"}]}),
        );
        let e = confirm(
            &store,
            &dir,
            &s2,
            serde_json::json!({"by": "Ada L", "rev": "not-it"}),
        )
        .unwrap_err();
        assert!(e.to_string().contains("changed after it was shown"), "{e}");
        assert!(
            !store
                .project_dir("demo")
                .join(format!("prepared/{s2}.resolved.yaml"))
                .exists()
        );
    }

    /// FTR-034 R-3: only the kind's editable fields can be changed, each to a value of its type,
    /// and each change is kept beside what was prepared; the prepared file is never rewritten.
    #[test]
    fn only_a_kinds_editable_fields_can_be_changed_and_each_change_is_kept() {
        let (store, dir, _) = board_with_item(false);
        let id = prepared_gate(&store, &dir);
        let file = store
            .project_dir("demo")
            .join(format!("prepared/{id}.yaml"));
        let before = std::fs::read(&file).unwrap();
        for (edits, words) in [
            (
                serde_json::json!({"commit": SHA}),
                "'commit' is not a field",
            ),
            (
                serde_json::json!({"gate": "smoke"}),
                "'gate' is not a field",
            ),
            (serde_json::json!({"passed": "no"}), "true or false"),
        ] {
            let e = confirm(
                &store,
                &dir,
                &id,
                serde_json::json!({"by": "Ada L", "edits": edits}),
            )
            .unwrap_err();
            assert!(e.to_string().contains(words), "{e}");
        }
        assert!(
            crate::gates::results(&store, "demo", None)
                .unwrap()
                .is_empty()
        );
        confirm(
            &store,
            &dir,
            &id,
            serde_json::json!({"by": "Ada L", "edits": {
                "note": "all 412 tests passed; checked the smoke run too",
                "url": "https://ci.example/run/4512",
            }}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "the prepared file was rewritten"
        );
        let a = resolution_of(&store, &id);
        assert_eq!(a.changed.len(), 1, "{:?}", a.changed);
        assert_eq!(a.changed[0].field, "note");
        assert_eq!(
            a.changed[0].prepared,
            Value::String("all 412 tests passed".into())
        );
        assert_eq!(
            a.changed[0].confirmed,
            Value::String("all 412 tests passed; checked the smoke run too".into())
        );
        let g = &crate::gates::results(&store, "demo", None).unwrap()[0];
        assert_eq!(g.note, "all 412 tests passed; checked the smoke run too");
    }

    /// FTR-034 R-3: rejecting keeps the record and the reason, and makes no verdict.
    #[test]
    fn rejecting_keeps_the_record_and_makes_no_verdict() {
        let (store, dir, _) = board_with_item(false);
        let id = prepared_gate(&store, &dir);
        let out = apply(
            &store,
            &dir,
            "POST",
            &format!("/projects/demo/prepared/{id}/reject"),
            serde_json::json!({"by": "Ada L", "reason": "that run was on another branch"}),
            crate::origin::Origin::Monitor,
        )
        .unwrap();
        assert_eq!(out["verdict"], "rejected");
        let a = resolution_of(&store, &id);
        assert_eq!(
            (
                a.by.as_str(),
                a.origin.as_str(),
                a.reason.as_str(),
                a.made.as_str()
            ),
            ("Ada L", "monitor", "that run was on another branch", "")
        );
        assert!(
            store
                .project_dir("demo")
                .join(format!("prepared/{id}.yaml"))
                .exists()
        );
        assert!(
            crate::gates::results(&store, "demo", None)
                .unwrap()
                .is_empty()
        );
        let all = list(
            &store,
            "demo",
            &ListQuery {
                all: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(all[0].state, State::Rejected);
        // A reason is optional; an answered record is not answered twice.
        let e = apply(
            &store,
            &dir,
            "POST",
            &format!("/projects/demo/prepared/{id}/reject"),
            serde_json::json!({"by": "Ada L"}),
            crate::origin::Origin::Monitor,
        )
        .unwrap_err();
        assert!(e.to_string().contains("already rejected"), "{e}");
        let id2 = prepared_gate(&store, &dir);
        apply(
            &store,
            &dir,
            "POST",
            &format!("/projects/demo/prepared/{id2}/reject"),
            serde_json::json!({"by": "Ada L"}),
            crate::origin::Origin::Terminal,
        )
        .unwrap();
    }
}
