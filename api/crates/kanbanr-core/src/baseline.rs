//! A project's engineering baseline (FTR-033): the project-wide answers to its practice packs'
//! questions, kept beside the charter as `projects/<id>/baseline.yaml`.
//!
//! Every list is append-only. An answer is proposed as a revision; it is in force only once the
//! person accepts that revision on the monitor or in their terminal. Reading never writes
//! (ADR-0005). Accepting a revision writes the ADR of each constraint it records and the release
//! stage gates its entries promise; the checks read the in-force entries a Design touches.

use crate::adr::Adr;
use crate::config::Library;
use crate::decisions::Stamp;
use crate::models::Extra;
use crate::profile::{self, Adds, Kind, ProfileFile, ProfileSource};
use crate::{CoreError, Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The baseline file, beside `config.yaml`.
pub const BASELINE_FILE: &str = "baseline.yaml";

fn is_false(b: &bool) -> bool {
    !*b
}

/// The whole record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Baseline {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packs: Vec<PackRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub areas: Vec<AreaRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<Revision>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A pack the project follows, or followed: a removed pack is kept, and its entries stay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PackRecord {
    #[serde(flatten)]
    pub source: ProfileSource,
    pub adopted: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Stamp>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One entry of the baseline: a requirement, constraint, non-goal or practice an answer records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Entry {
    pub id: String,
    pub pack: String,
    pub area: String,
    pub option: String,
    pub kind: Kind,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iso25010: Vec<String>,
    /// Copied from the pack when proposed, so the checks never need the pack file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touches: Vec<String>,
    /// The area's other options' words, for the contradiction flag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mentions: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Adds::is_empty")]
    pub adds: Adds,
    /// The ADR written at acceptance, for a constraint.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adr: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub superseded_by: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// The standing answer of an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AreaState {
    Answered,
    Later,
    NotApplicable,
}

/// One chosen option of an answer, with the value it carries when the option asks for one
/// (`asks_value`). Written as the bare option id when it carries no value, so `options:
/// [postgresql]` reads as before; a value is written `{option: component-library, value: MUI}`.
/// In a `many` area every chosen option carries its own value (coordinator, 17 Nov 2026).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Chosen {
    pub option: String,
    pub value: String,
}

impl Chosen {
    pub fn new(option: impl Into<String>, value: impl Into<String>) -> Chosen {
        Chosen {
            option: option.into(),
            value: value.into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ChosenForm {
    Bare(String),
    Full {
        option: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        value: String,
    },
}

impl Serialize for Chosen {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        if self.value.is_empty() {
            ChosenForm::Bare(self.option.clone()).serialize(s)
        } else {
            ChosenForm::Full {
                option: self.option.clone(),
                value: self.value.clone(),
            }
            .serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Chosen {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(match ChosenForm::deserialize(d)? {
            ChosenForm::Bare(option) => Chosen {
                option,
                value: String::new(),
            },
            ChosenForm::Full { option, value } => Chosen { option, value },
        })
    }
}

/// The ids of the chosen options.
pub fn option_ids(chosen: &[Chosen]) -> Vec<String> {
    chosen.iter().map(|c| c.option.clone()).collect()
}

/// The standing answer per area; history is in the revisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AreaRecord {
    pub id: String,
    pub state: AreaState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Chosen>,
    /// Read from an older record whose value was one for the whole answer; new records carry the
    /// value on each chosen option.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One answer of a revision (or of an answers file).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Answer {
    pub area: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Chosen>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub later: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_applicable: Option<String>,
    /// The person picked after "Explain the options".
    #[serde(default, skip_serializing_if = "is_false")]
    pub explained: bool,
}

/// The forms an answer is read in: `options: [..]` (bare ids or `{option, value}`), and the older
/// `option: <id>` with one `value` for the whole answer.
#[derive(Deserialize)]
struct AnswerRaw {
    area: String,
    #[serde(default)]
    options: Vec<Chosen>,
    #[serde(default)]
    option: Option<String>,
    #[serde(default)]
    value: String,
    #[serde(default)]
    later: bool,
    #[serde(default)]
    not_applicable: Option<String>,
    #[serde(default)]
    explained: bool,
}

impl<'de> Deserialize<'de> for Answer {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = AnswerRaw::deserialize(d)?;
        let mut options = raw.options;
        if let Some(o) = raw.option
            && !options.iter().any(|c| c.option == o)
        {
            options.insert(0, Chosen::new(o, ""));
        }
        // One answer-wide value belongs to the one chosen option that carries none.
        if !raw.value.is_empty() {
            let bare: Vec<usize> = (0..options.len())
                .filter(|i| options[*i].value.is_empty())
                .collect();
            if bare.len() == 1 {
                options[bare[0]].value = raw.value;
            }
        }
        Ok(Answer {
            area: raw.area,
            options,
            later: raw.later,
            not_applicable: raw.not_applicable,
            explained: raw.explained,
        })
    }
}

/// A proposed change to the baseline, and what became of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Revision {
    pub id: String,
    /// The item whose design asked; empty at setup.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub answers: Vec<Answer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    pub proposed: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<Stamp>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// Where a revision stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RevisionState {
    Proposed,
    Accepted,
    Withdrawn,
}

impl Revision {
    pub fn state(&self) -> RevisionState {
        if self.withdrawn.is_some() {
            RevisionState::Withdrawn
        } else if self.accepted.is_some() {
            RevisionState::Accepted
        } else {
            RevisionState::Proposed
        }
    }
}

/// Where one area stands in the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaStatus {
    Open,
    Later,
    /// A proposed revision answers it: that revision's id.
    Proposed(String),
    /// The chosen option ids.
    Answered(Vec<String>),
    /// The reason it does not apply.
    NotApplicable(String),
}

fn baseline_path(store: &Store, p: &str) -> std::path::PathBuf {
    store.project_dir(p).join(BASELINE_FILE)
}

/// A project's baseline: the default when there is no file; an error, never a rewrite, when the
/// file does not parse.
pub fn load(store: &Store, p: &str) -> Result<Baseline> {
    match std::fs::read_to_string(baseline_path(store, p)) {
        Ok(s) if s.trim().is_empty() => Ok(Baseline::default()),
        Ok(s) => crate::yaml::from_str(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Baseline::default()),
        Err(e) => Err(e.into()),
    }
}

/// Where `area` stands: a proposed revision answering it, else its standing record, else open.
pub fn area_status(b: &Baseline, area: &str) -> AreaStatus {
    if let Some(r) = b
        .revisions
        .iter()
        .find(|r| r.state() == RevisionState::Proposed && r.answers.iter().any(|a| a.area == area))
    {
        return AreaStatus::Proposed(r.id.clone());
    }
    match b.areas.iter().rev().find(|a| a.id == area) {
        Some(rec) => match rec.state {
            AreaState::Answered => AreaStatus::Answered(option_ids(&rec.options)),
            AreaState::Later => AreaStatus::Later,
            AreaState::NotApplicable => AreaStatus::NotApplicable(rec.reason.clone()),
        },
        None => AreaStatus::Open,
    }
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T> {
    Err(CoreError::Unsupported(msg.into()))
}

/// Write the whole record (ADR-0005: only a file that parsed is ever rewritten).
fn save(store: &Store, p: &str, b: &Baseline) -> Result<()> {
    let path = baseline_path(store, p);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic::write(path, crate::yaml::to_string(b)?)?;
    Ok(())
}

/// `projects/<id>/packs/<name>.yaml`: the applied copy of a pack the project follows.
fn applied_path(store: &Store, p: &str, name: &str) -> std::path::PathBuf {
    store
        .project_dir(p)
        .join(profile::APPLIED_DIR)
        .join(format!("{name}.yaml"))
}

/// The applied copy of pack `name`, `None` when the file is gone; an error when it does not parse.
pub fn applied_pack(store: &Store, p: &str, name: &str) -> Result<Option<ProfileFile>> {
    match std::fs::read_to_string(applied_path(store, p, name)) {
        Ok(text) => ProfileFile::from_yaml(&text).map(Some).map_err(|e| {
            CoreError::Unsupported(format!(
                "projects/{p}/{}/{name}.yaml does not parse: {e}",
                profile::APPLIED_DIR
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The packs the project follows (not removed), in the baseline's order, each with its applied
/// copy: what `propose`, `ask` and the checks read. A pack whose copy is gone is left out.
pub fn followed_packs(
    store: &Store,
    p: &str,
    b: &Baseline,
) -> Result<Vec<(ProfileSource, ProfileFile)>> {
    let mut out = Vec::new();
    for rec in b.packs.iter().filter(|r| r.removed.is_none()) {
        if let Some(file) = applied_pack(store, p, &rec.source.name)? {
            out.push((rec.source.clone(), file));
        }
    }
    Ok(out)
}

/// Follow pack `name` (or update it to `file`): writes `projects/<id>/packs/<name>.yaml` as given
/// and adds or updates its record; adding a removed pack again clears `removed` and restores the
/// stage gates its entries in force promised. Refused for a pack with problems, one requiring a
/// pack the project does not follow, and one defining an area another followed pack defines.
/// Entries, areas and revisions are never touched, so every answer is kept (R-5).
pub fn add_pack(
    store: &Store,
    p: &str,
    name: &str,
    file: &ProfileFile,
    library: Library,
    stamp: Stamp,
) -> Result<Baseline> {
    let name = name.trim();
    if name.is_empty() {
        return unsupported("a pack needs a name");
    }
    let problems = file.problems();
    if !problems.is_empty() {
        return unsupported(format!(
            "pack '{name}' has problems, so it cannot be followed: {}",
            problems
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let mut b = load(store, p)?;
    let mut others: Vec<(String, ProfileFile)> = Vec::new();
    for rec in b.packs.iter().filter(|r| r.removed.is_none()) {
        if rec.source.name == name {
            continue;
        }
        let f = applied_pack(store, p, &rec.source.name)?.unwrap_or_default();
        others.push((rec.source.name.clone(), f));
    }
    let followed: Vec<(&str, &ProfileFile)> = others.iter().map(|(n, f)| (n.as_str(), f)).collect();
    profile::check_composition(&followed, name, file)?;
    let path = applied_path(store, p, name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic::write(&path, file.to_yaml()?)?;
    let source = file.source(name, library);
    let mut restored = false;
    match b.packs.iter_mut().find(|r| r.source.name == name) {
        Some(rec) => {
            restored = rec.removed.take().is_some();
            rec.source = source;
            rec.adopted = stamp;
        }
        None => b.packs.push(PackRecord {
            source,
            adopted: stamp,
            removed: None,
            extra: Extra::default(),
        }),
    }
    if restored {
        let mut project = store.load_meta(p)?;
        let mut cadence = project.config.cadence.clone();
        let mine: Vec<&Entry> = in_force(&b)
            .into_iter()
            .filter(|e| e.pack == name)
            .collect();
        // With releases off no stage holds a gate (as on acceptance).
        apply_stage_gates(&mut cadence, &mine)?;
        if cadence != project.config.cadence {
            project.config.cadence = cadence;
            store.save_config(p, &project.config)?;
        }
    }
    save(store, p, &b)?;
    Ok(b)
}

/// Stop following pack `name`: its record is kept with `removed` set, and its entries stay for the
/// history and the ADRs, but the stage gates they promised are dropped from the release stages.
/// Refused when another followed pack requires it.
pub fn remove_pack(store: &Store, p: &str, name: &str, stamp: Stamp) -> Result<Baseline> {
    let name = name.trim();
    let mut b = load(store, p)?;
    if !b
        .packs
        .iter()
        .any(|r| r.source.name == name && r.removed.is_none())
    {
        return unsupported(format!("this project does not follow pack '{name}'"));
    }
    for rec in b.packs.iter().filter(|r| r.removed.is_none()) {
        if rec.source.name == name {
            continue;
        }
        if let Some(f) = applied_pack(store, p, &rec.source.name)?
            && f.requires().iter().any(|r| r == name)
        {
            return unsupported(format!(
                "pack '{}' requires pack '{name}': remove '{}' first",
                rec.source.name, rec.source.name
            ));
        }
    }
    if let Some(rec) = b.packs.iter_mut().find(|r| r.source.name == name) {
        rec.removed = Some(stamp);
    }
    let ids: Vec<String> = b
        .entries
        .iter()
        .filter(|e| e.pack == name)
        .map(|e| e.id.clone())
        .collect();
    let mut project = store.load_meta(p)?;
    if project.config.cadence.remove_stage_gates_from(&ids) > 0 {
        store.save_config(p, &project.config)?;
    }
    save(store, p, &b)?;
    Ok(b)
}

impl Revision {
    /// What the person accepts: a hash of the revision's content, from canonical JSON.
    pub fn rev(&self) -> String {
        #[derive(Serialize)]
        struct Content<'a> {
            item: &'a str,
            note: &'a str,
            answers: Vec<&'a Answer>,
            adds: &'a [String],
            replaces: &'a [String],
        }
        let mut answers: Vec<&Answer> = self.answers.iter().collect();
        answers.sort_by(|a, b| a.area.cmp(&b.area));
        crate::hash::stable_hash(&crate::hash::canonical_json(&Content {
            item: &self.item,
            note: &self.note,
            answers,
            adds: &self.adds,
            replaces: &self.replaces,
        }))
    }
}

/// The ids of accepted revisions.
fn accepted_ids(b: &Baseline) -> BTreeSet<&str> {
    b.revisions
        .iter()
        .filter(|r| r.state() == RevisionState::Accepted)
        .map(|r| r.id.as_str())
        .collect()
}

/// The entries in force: their revision is accepted and nothing has superseded them. Whether the
/// acceptance came through kanbanr is [`accepted_through_kanbanr`]'s, which the view reports.
pub fn in_force(b: &Baseline) -> Vec<&Entry> {
    let accepted = accepted_ids(b);
    b.entries
        .iter()
        .filter(|e| accepted.contains(e.revision.as_str()) && e.superseded_by.is_empty())
        .collect()
}

/// The (stage, gate, entry) each option of `entries` promises, once per option: the first entry
/// an option records carries its stage gates for all of them.
pub(crate) fn promised_gates(entries: &[&Entry]) -> Vec<(String, String, String)> {
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut out = Vec::new();
    for e in entries {
        for g in &e.adds.release_gates {
            let key = (
                format!("{}/{}/{}", e.revision, e.area, e.option),
                g.stage.clone(),
                g.gate.clone(),
            );
            if seen.insert(key) {
                out.push((g.stage.clone(), g.gate.clone(), e.id.clone()));
            }
        }
    }
    out
}

/// Write the stage gates `entries` promise into `cadence` (FTR-033 P4 Q3): the one place they are
/// applied, by an acceptance, by following a removed pack again and by turning releases on. With
/// releases off nothing is written and each gate comes back as a warning saying when it applies
/// (P3 Q2); with releases on, a stage the project does not have refuses, naming the stages it has.
pub fn apply_stage_gates(
    cadence: &mut crate::config::Cadence,
    entries: &[&Entry],
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for (stage, gate, from) in promised_gates(entries) {
        if !cadence.releases {
            warnings.push(format!(
                "{from} requires the `{gate}` gate at release stage {stage}; this project has \
                 releases off, so it applies once releases are on: stage {stage}"
            ));
            continue;
        }
        cadence
            .add_stage_gate(&stage, &gate, &from)
            .map_err(CoreError::Unsupported)?;
    }
    Ok(warnings)
}

/// The entries in force of the packs the project still follows: the ones whose stage gates the
/// release stages hold.
pub fn followed_in_force(b: &Baseline) -> Vec<&Entry> {
    let removed = removed_packs(b);
    in_force(b)
        .into_iter()
        .filter(|e| !removed.contains(e.pack.as_str()))
        .collect()
}

/// What Claude proposes: the answers, and the item whose design asked (empty at setup).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Proposal {
    #[serde(default)]
    pub item: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub answers: Vec<Answer>,
}

/// `{label}`, `{value}` and `{project}` in a pack's text.
fn template(text: &str, label: &str, value: &str, project: &str) -> String {
    text.replace("{label}", label)
        .replace("{value}", value)
        .replace("{project}", project)
}

/// The highest number of ids shaped `<prefix><n>`.
fn highest(ids: impl Iterator<Item = String>, prefix: &str) -> u32 {
    ids.filter_map(|id| id.strip_prefix(prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

/// Record a proposed revision (R-3): inert until the person accepts it. Refused for an empty
/// proposal, an unknown area or option, a `one` area not answered with exactly one option, an
/// answer giving options and also `later` or `not_applicable`, an area a proposed revision already
/// answers, a value missing or out of place (`profile::value_problems`), and an item not on the
/// board. A `when` that does not hold is a warning on the view. Each chosen option makes one entry
/// per `records` row, except a non-goal row that another option chosen in the same area is the
/// thing it excludes (its `excludes`, else its words; left out, with a warning naming that
/// choice); `replaces` names the in-force entries of every area answered.
pub fn propose(
    store: &Store,
    p: &str,
    proposal: Proposal,
    packs: &[(ProfileSource, ProfileFile)],
    stamp: Stamp,
) -> Result<RevisionView> {
    if proposal.answers.is_empty() {
        return unsupported("a baseline revision needs at least one answer");
    }
    let item = proposal.item.trim().to_string();
    let project = store.load_meta(p)?;
    if !item.is_empty() {
        project.feature(&item)?;
    }
    let project_name = if project.config.name.trim().is_empty() {
        p.to_string()
    } else {
        project.config.name.clone()
    };
    let mut b = load(store, p)?;
    let id = format!(
        "REV-{:03}",
        highest(b.revisions.iter().map(|r| r.id.clone()), "REV-") + 1
    );
    let mut next = highest(b.entries.iter().map(|e| e.id.clone()), "B-");
    let set = profile::answer_set(&b, &proposal.answers);
    let mut entries: Vec<Entry> = Vec::new();
    let mut replaces: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for a in &proposal.answers {
        let Some((source, area)) = packs
            .iter()
            .find_map(|(s, f)| f.area(&a.area).map(|area| (s, area)))
        else {
            return unsupported(format!(
                "no pack this project follows asks area '{}'",
                a.area
            ));
        };
        if !seen.insert(a.area.as_str()) {
            return unsupported(format!("area '{}' is answered twice", a.area));
        }
        if let AreaStatus::Proposed(other) = area_status(&b, &a.area) {
            return unsupported(format!(
                "{other} already proposes '{}': accept or withdraw it first",
                a.area
            ));
        }
        let decided = a.later || a.not_applicable.is_some();
        if a.later && a.not_applicable.is_some() {
            return unsupported(format!(
                "area '{}': an answer is either 'decide later' or 'not applicable', not both",
                a.area
            ));
        }
        if decided && !a.options.is_empty() {
            return unsupported(format!(
                "area '{}': an answer gives options or says 'decide later' or 'not applicable', \
                 not both",
                a.area
            ));
        }
        if !decided && a.options.is_empty() {
            return unsupported(format!(
                "area '{}': the answer names no option (give options, later or not_applicable)",
                a.area
            ));
        }
        if a.not_applicable
            .as_deref()
            .is_some_and(|r| r.trim().is_empty())
        {
            return unsupported(format!(
                "area '{}': 'not applicable' needs its reason",
                a.area
            ));
        }
        for c in &a.options {
            if area.option(&c.option).is_none() {
                return unsupported(format!(
                    "area '{}' has no option '{}' — its options are {}",
                    a.area,
                    c.option,
                    area.options
                        .iter()
                        .map(|o| o.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if !decided && area.select == profile::Select::One && a.options.len() != 1 {
            return unsupported(format!(
                "area '{}' takes exactly one option; {} were given",
                a.area,
                a.options.len()
            ));
        }
        let pairs: Vec<(String, String)> = a
            .options
            .iter()
            .map(|c| (c.option.clone(), c.value.clone()))
            .collect();
        if let Some(e) = profile::value_problems(area, &pairs).into_iter().next() {
            return Err(e);
        }
        if !profile::when_holds(area, &set)
            && let Some(w) = &area.when
        {
            warnings.push(format!(
                "area '{}' is asked only when '{}' is one of {}, which the baseline does not say",
                a.area,
                w.area,
                w.options.join(", ")
            ));
        }
        replaces.extend(
            in_force(&b)
                .into_iter()
                .filter(|e| e.area == a.area)
                .map(|e| e.id.clone()),
        );
        for c in &a.options {
            let opt = area.option(&c.option).expect("checked above");
            let mentions: BTreeMap<String, Vec<String>> = area
                .options
                .iter()
                .filter(|o| o.id != opt.id && !o.mentions.is_empty())
                .map(|o| (o.id.clone(), o.mentions.clone()))
                .collect();
            for r in &opt.records {
                // A non-goal gives way to another option chosen in the same area that is the
                // thing it excludes (containers' "No Kubernetes" with kubernetes also chosen):
                // the row is left out and the view says which choice it gave way to (FTR-033 P3
                // Q1). The other option is one its `excludes` names (P4 Q1, P12 Q1/Q2); a row without
                // `excludes` knows it by its words: its mentions and its label.
                let gave_way = if r.kind != Kind::NonGoal {
                    None
                } else if !r.excludes.is_empty() {
                    a.options
                        .iter()
                        .find(|o| {
                            o.option != opt.id && r.excludes.iter().any(|e| e.trim() == o.option)
                        })
                        .and_then(|o| area.option(&o.option))
                        .map(|other| (other, format!("it excludes '{}'", other.id)))
                } else {
                    a.options.iter().find_map(|o| {
                        if o.option == opt.id {
                            return None;
                        }
                        let other = area.option(&o.option)?;
                        let mut words = other.mentions.clone();
                        words.push(other.label.clone());
                        profile::tokens_in(&words, &r.text)
                            .into_iter()
                            .next()
                            .map(|w| (other, format!("it says '{w}'")))
                    })
                };
                if let Some((other, because)) = gave_way {
                    warnings.push(format!(
                        "area '{}': the non-goal '{}' of '{}' is left out: it gave way to '{}', \
                         also chosen ({because})",
                        area.id,
                        template(&r.text, &opt.label, &c.value, &project_name),
                        opt.label,
                        other.label
                    ));
                    continue;
                }
                next += 1;
                entries.push(Entry {
                    id: format!("B-{next:03}"),
                    pack: source.name.clone(),
                    area: area.id.clone(),
                    option: opt.id.clone(),
                    kind: r.kind,
                    text: template(&r.text, &opt.label, &c.value, &project_name),
                    why: template(
                        if r.why.is_empty() { &opt.why } else { &r.why },
                        &opt.label,
                        &c.value,
                        &project_name,
                    ),
                    iso25010: r.iso25010.clone(),
                    touches: area.touches.clone(),
                    mentions: mentions.clone(),
                    adds: opt.adds.clone(),
                    adr: String::new(),
                    revision: id.clone(),
                    superseded_by: String::new(),
                    extra: Extra::default(),
                });
            }
        }
    }
    replaces.sort();
    replaces.dedup();
    let revision = Revision {
        id: id.clone(),
        item,
        note: proposal.note.trim().to_string(),
        answers: proposal.answers,
        adds: entries.iter().map(|e| e.id.clone()).collect(),
        replaces,
        proposed: stamp,
        accepted: None,
        withdrawn: None,
        extra: Extra::default(),
    };
    b.entries.extend(entries);
    b.revisions.push(revision);
    save(store, p, &b)?;
    let mut view = revision_view(&b, &id, &BTreeMap::new(), &BTreeMap::new());
    view.warnings = warnings;
    Ok(view)
}

/// The record's packs: every pack it lists, removed ones included, with its applied copy.
fn every_pack(store: &Store, p: &str, b: &Baseline) -> Vec<(String, Option<ProfileFile>)> {
    b.packs
        .iter()
        .map(|r| {
            (
                r.source.name.clone(),
                applied_pack(store, p, &r.source.name).ok().flatten(),
            )
        })
        .collect()
}

/// The person accepts revision `id`, pinned to the `rev` they saw (R-3). Refused to any origin but
/// the monitor and the terminal, for a revision not proposed, and for a stale `rev`. Each constraint
/// gets its ADR (one already holding `baseline: B-n` is reused); each replaced entry is superseded
/// by the new entry of its area (and its ADR by the new ADR); each answered area's standing record
/// is rewritten; the stage gates the entries promise are written into `cadence.release_gates` and
/// those of replaced entries dropped. ADRs are written first, then the config, then the baseline,
/// so the write recipe commits the baseline and the config together.
pub fn accept(
    store: &Store,
    p: &str,
    id: &str,
    rev: &str,
    mut stamp: Stamp,
) -> Result<RevisionView> {
    crate::decisions::person_stamp(&stamp, "accepting a baseline revision")?;
    let mut b = load(store, p)?;
    let Some(index) = b.revisions.iter().position(|r| r.id == id.trim()) else {
        return unsupported(format!("there is no baseline revision {id}"));
    };
    let revision = b.revisions[index].clone();
    match revision.state() {
        RevisionState::Proposed => {}
        RevisionState::Accepted => return unsupported(format!("{id} is already accepted")),
        RevisionState::Withdrawn => {
            return unsupported(format!("{id} was withdrawn, so it cannot be accepted"));
        }
    }
    if revision.rev() != rev.trim() {
        return unsupported(format!(
            "{id} changed since you saw it: read it again before accepting"
        ));
    }
    // The release stages first, in memory, so an unknown stage refuses before anything is written.
    let mut project = store.load_meta(p)?;
    let mut cadence = project.config.cadence.clone();
    cadence.remove_stage_gates_from(&revision.replaces);
    let added: Vec<&Entry> = b
        .entries
        .iter()
        .filter(|e| revision.adds.contains(&e.id))
        .collect();
    // With releases off there are no stages to hold a gate: the entry is recorded, no stage gate
    // is added, and the view says when it applies (FTR-033 P3 Q2); turning releases on writes it
    // then (P4 Q3). With releases on, a stage the project does not have refuses the acceptance,
    // naming the stages it has.
    let warnings = apply_stage_gates(&mut cadence, &added)?;
    let cadence_changed = cadence != project.config.cadence;
    if cadence_changed {
        cadence
            .validate_release_gates()
            .map_err(CoreError::Unsupported)?;
    }
    // The ADRs of the constraints.
    let today = stamp.at.get(..10).map(str::to_string).unwrap_or_else(|| {
        crate::now_rfc3339()
            .get(..10)
            .unwrap_or_default()
            .to_string()
    });
    let packs = every_pack(store, p, &b);
    let existing = crate::adr::list(store, p)?;
    let mut adr_of_entry: BTreeMap<String, String> = BTreeMap::new();
    for e in &added {
        if e.kind != Kind::Constraint || !e.adr.is_empty() {
            continue;
        }
        if let Some(found) = existing
            .iter()
            .find(|a| a.baseline.eq_ignore_ascii_case(&e.id))
        {
            adr_of_entry.insert(e.id.clone(), found.id.clone());
            continue;
        }
        let area = packs
            .iter()
            .find(|(n, _)| *n == e.pack)
            .and_then(|(_, f)| f.as_ref())
            .and_then(|f| f.area(&e.area));
        let chosen: Vec<&str> = revision
            .answers
            .iter()
            .filter(|a| a.area == e.area)
            .flat_map(|a| a.options.iter().map(|c| c.option.as_str()))
            .collect();
        let others: Vec<(String, String)> = area
            .map(|area| {
                area.options
                    .iter()
                    .filter(|o| !chosen.contains(&o.id.as_str()))
                    .map(|o| {
                        let said = o
                            .records
                            .iter()
                            .find(|r| r.kind == Kind::Constraint)
                            .map(|r| template(&r.text, &o.label, "", p))
                            .unwrap_or_else(|| o.why.clone());
                        (o.label.clone(), said)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let others: Vec<(&str, &str)> = others
            .iter()
            .map(|(l, t)| (l.as_str(), t.as_str()))
            .collect();
        let with: Vec<&Entry> = added
            .iter()
            .copied()
            .filter(|x| {
                x.area == e.area
                    && x.option == e.option
                    && matches!(x.kind, Kind::Practice | Kind::Nfr)
            })
            .collect();
        let (title, adr, body) = adr_of(
            e,
            area.map_or(e.area.as_str(), |a| a.title.as_str()),
            area.map_or("", |a| a.question.as_str()),
            &others,
            &with,
            &revision.item,
            &stamp.by,
            &stamp.origin,
            &today,
        );
        let written = crate::adr::create_with_body(store, p, &title, adr, body)?;
        adr_of_entry.insert(e.id.clone(), written.id);
    }
    for e in b.entries.iter_mut() {
        if let Some(a) = adr_of_entry.get(&e.id) {
            e.adr = a.clone();
        }
    }
    // What the revision replaces is superseded by the new entry of its area.
    for old in &revision.replaces {
        let Some(old_entry) = b.entries.iter().find(|e| e.id == *old).cloned() else {
            continue;
        };
        let new = b
            .entries
            .iter()
            .find(|e| revision.adds.contains(&e.id) && e.area == old_entry.area)
            .cloned();
        if let Some(n) = &new
            && !n.adr.is_empty()
            && !old_entry.adr.is_empty()
            && !n.adr.eq_ignore_ascii_case(&old_entry.adr)
        {
            crate::adr::supersede(store, p, &n.adr, &old_entry.adr)?;
        }
        let by = new.map_or_else(|| revision.id.clone(), |n| n.id);
        if let Some(e) = b.entries.iter_mut().find(|e| e.id == *old) {
            e.superseded_by = by;
        }
    }
    // The standing answer of each area the revision answers.
    for a in &revision.answers {
        let record = AreaRecord {
            id: a.area.clone(),
            state: if a.later {
                AreaState::Later
            } else if a.not_applicable.is_some() {
                AreaState::NotApplicable
            } else {
                AreaState::Answered
            },
            options: a.options.clone(),
            value: String::new(),
            reason: a.not_applicable.clone().unwrap_or_default(),
            item: revision.item.clone(),
            revision: revision.id.clone(),
            extra: Extra::default(),
        };
        match b.areas.iter_mut().find(|r| r.id == a.area) {
            Some(r) => *r = record,
            None => b.areas.push(record),
        }
    }
    stamp.rev = revision.rev();
    b.revisions[index].accepted = Some(stamp);
    if cadence_changed {
        project.config.cadence = cadence;
        store.save_config(p, &project.config)?;
    }
    save(store, p, &b)?;
    let mut view = revision_view(&b, id.trim(), &BTreeMap::new(), &BTreeMap::new());
    view.warnings = warnings;
    Ok(view)
}

/// Withdraw a proposed revision, with a reason; anyone may. The record stays.
pub fn withdraw(
    store: &Store,
    p: &str,
    id: &str,
    reason: &str,
    mut stamp: Stamp,
) -> Result<RevisionView> {
    if reason.trim().is_empty() {
        return unsupported("withdrawing a baseline revision needs a reason");
    }
    let mut b = load(store, p)?;
    let Some(r) = b.revisions.iter_mut().find(|r| r.id == id.trim()) else {
        return unsupported(format!("there is no baseline revision {id}"));
    };
    if r.state() != RevisionState::Proposed {
        return unsupported(format!(
            "{id} is {}: only a proposed revision is withdrawn",
            match r.state() {
                RevisionState::Accepted => "accepted",
                _ => "already withdrawn",
            }
        ));
    }
    stamp.reason = reason.trim().to_string();
    r.withdrawn = Some(stamp);
    save(store, p, &b)?;
    Ok(revision_view(
        &b,
        id.trim(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    ))
}

/// Was revision `id` accepted through kanbanr? As `decisions::granted_through_kanbanr`, over
/// `projects/<p>/baseline.yaml`: no uncommitted change to the file, and the first commit showing
/// `id` accepted carries `Kanbanr-Origin: monitor` or `terminal`. A history that cannot be read
/// is an error, never "not accepted" (DEF-080: fail closed, and say why).
pub fn accepted_through_kanbanr(
    store: &Store,
    p: &str,
    id: &str,
) -> std::result::Result<bool, crate::git::HistoryError> {
    let rel = format!("projects/{p}/{BASELINE_FILE}");
    let dir = store.data_dir();
    if let Err(dirty) = crate::git::is_clean(dir)
        && dirty.iter().any(|f| f == &rel)
    {
        return Ok(false);
    }
    let history = crate::git::file_history(dir, &rel)?;
    let Some(first) = history.iter().rev().find(|v| {
        crate::yaml::from_str::<Baseline>(&v.content)
            .ok()
            .is_some_and(|b| {
                b.revisions
                    .iter()
                    .any(|r| r.id == id && r.accepted.is_some())
            })
    }) else {
        return Ok(false);
    };
    Ok(first.message.lines().any(|l| {
        matches!(
            l.trim(),
            "Kanbanr-Origin: monitor" | "Kanbanr-Origin: terminal"
        )
    }))
}

/// The ADR of an accepted constraint: (title, front matter, body). Every section is filled, so
/// it reads as accepted. `others` are the area's other options (label, what it would have said),
/// `with` the practices and nfrs the same option recorded.
#[allow(clippy::too_many_arguments)]
fn adr_of(
    entry: &Entry,
    area_title: &str,
    question: &str,
    others: &[(&str, &str)],
    with: &[&Entry],
    item: &str,
    by: &str,
    origin: &str,
    today: &str,
) -> (String, Adr, String) {
    let title = entry.text.trim().trim_end_matches('.').to_string();
    let adr = Adr {
        status: "accepted".to_string(),
        date: today.to_string(),
        deciders: vec![by.to_string()],
        affects: if item.is_empty() {
            Vec::new()
        } else {
            vec![item.to_string()]
        },
        quality: entry.iso25010.clone(),
        decided: today.to_string(),
        decided_origin: origin.to_string(),
        baseline: entry.id.clone(),
        ..Adr::default()
    };
    let when = if item.is_empty() {
        "It was answered at setup".to_string()
    } else {
        format!("It was answered in {item}'s design")
    };
    let question = if question.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", question.trim())
    };
    let alternatives = if others.is_empty() {
        "- No other option was offered.".to_string()
    } else {
        others
            .iter()
            .map(|(label, said)| {
                if said.trim().is_empty() {
                    format!("- {label}")
                } else {
                    format!("- {label}: {said}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let touches = if entry.touches.is_empty() {
        format!("area '{}'", entry.area)
    } else {
        entry
            .touches
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut consequences: Vec<String> = with
        .iter()
        .map(|x| format!("- {}: {}", x.kind.as_str(), x.text))
        .collect();
    consequences.push(format!(
        "- Every design that touches {touches} names {} with a test or a reason.",
        entry.id
    ));
    let why = if entry.why.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n{}", entry.why.trim())
    };
    let body = format!(
        "# {title}\n\n\
         ## Context\n\n\
         The engineering baseline asks about {area_title}.{question} {when}, as baseline entry {id} \
         (area `{area}`, option `{option}`).\n\n\
         ## Decision\n\n\
         We will hold to this: {text}.{why}\n\n\
         ## Alternatives considered\n\n\
         {alternatives}\n\n\
         ## Consequences\n\n\
         {consequences}\n\n\
         ## Compliance\n\n\
         `kanbanr check` names a design that touches {touches} without naming {id} and flags one \
         that names another option; the Baseline page lists the items that name it.\n",
        id = entry.id,
        area = entry.area,
        option = entry.option,
        text = entry.text.trim().trim_end_matches('.'),
        consequences = consequences.join("\n"),
    );
    (title, adr, body)
}

/// The headings of a spec outside fenced blocks: (byte offset of the line, level, text).
fn headings(spec: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut at = 0;
    for line in spec.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            let level = t.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&level) && t[level..].starts_with(' ') {
                let text = t[level..].trim().trim_end_matches('#').trim().to_string();
                out.push((at, level, text));
            }
        }
        at += line.len();
    }
    out
}

/// The section of `text` under the first heading named `name` (case-insensitive), to the next
/// heading of the same or a higher level; with the heading's level.
fn section<'a>(text: &'a str, name: &str) -> Option<(&'a str, usize)> {
    let hs = headings(text);
    let i = hs
        .iter()
        .position(|(_, _, t)| t.eq_ignore_ascii_case(name))?;
    let (start, level, _) = hs[i];
    let end = hs[i + 1..]
        .iter()
        .find(|(_, l, _)| *l <= level)
        .map_or(text.len(), |(at, _, _)| *at);
    Some((&text[start..end], level))
}

/// The Design of a spec: from the heading named `Design` (any level) to the next heading of the
/// same or a higher level; "" when there is none. Pure.
pub fn design_text(spec: &str) -> &str {
    section(spec, "Design").map_or("", |(s, _)| s)
}

/// One `- B-003: text` line, as (id, text).
fn named_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let t = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))?
        .trim_start();
    let (t, bold) = match t.strip_prefix("**") {
        Some(r) => (r, true),
        None => (t, false),
    };
    if t.len() < 3 || !t[..2].eq_ignore_ascii_case("b-") {
        return None;
    }
    let digits: String = t[2..].chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let mut rest = &t[2 + digits.len()..];
    let mut sep = false;
    if bold {
        if let Some(r) = rest.strip_prefix(":**") {
            rest = r;
            sep = true;
        } else {
            rest = rest.strip_prefix("**")?;
        }
    }
    if !sep {
        if let Some(r) = rest.strip_prefix(':') {
            rest = r;
        } else {
            rest = rest
                .strip_prefix(" — ")
                .or_else(|| rest.strip_prefix(" —"))?;
        }
    }
    Some((format!("B-{digits}"), rest.trim().to_string()))
}

/// The entries a Design names under its `### Baseline` heading, each `- B-003: <test or reason>`
/// (`-` or `*`, optional bold around the id, `:` or ` — `), the id upper-cased. An id with no
/// text is not named; a `Baseline` heading outside the Design is not read. Pure.
pub fn baseline_section(spec: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some((body, _)) = section(design_text(spec), "Baseline") else {
        return out;
    };
    for line in body.lines().skip(1) {
        if let Some((id, text)) = named_line(line)
            && !text.is_empty()
        {
            out.entry(id).or_insert(text);
        }
    }
    out
}

/// The names of removed packs.
fn removed_packs(b: &Baseline) -> BTreeSet<&str> {
    b.packs
        .iter()
        .filter(|r| r.removed.is_some())
        .map(|r| r.source.name.as_str())
        .collect()
}

/// Each in-force entry of a followed (not removed) pack that `design` touches, with the tokens.
pub fn touched<'a>(b: &'a Baseline, design: &str) -> Vec<(&'a Entry, Vec<String>)> {
    let removed = removed_packs(b);
    in_force(b)
        .into_iter()
        .filter(|e| !removed.contains(e.pack.as_str()))
        .filter_map(|e| {
            let tokens = profile::tokens_in(&e.touches, design);
            (!tokens.is_empty()).then_some((e, tokens))
        })
        .collect()
}

/// A design naming an option of an answered area outside its answer: a flag to read, never a
/// judgment (as drift is, FTR-013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Contradiction {
    pub entry: String,
    pub option: String,
    pub word: String,
    /// From 1, within the text given.
    pub line: usize,
}

/// For each touched entry of an answered area, each option outside the area's answer whose words
/// the design holds: one per entry and option, at its first line.
pub fn contradictions(b: &Baseline, design: &str) -> Vec<Contradiction> {
    let mut out = Vec::new();
    for (e, _) in touched(b, design) {
        let AreaStatus::Answered(chosen) = standing_status(b, &e.area) else {
            continue;
        };
        for (option, words) in &e.mentions {
            if chosen.contains(option) {
                continue;
            }
            let hit = design.lines().enumerate().find_map(|(n, line)| {
                profile::tokens_in(words, line)
                    .into_iter()
                    .next()
                    .map(|w| (n + 1, w))
            });
            if let Some((line, word)) = hit {
                out.push(Contradiction {
                    entry: e.id.clone(),
                    option: option.clone(),
                    word,
                    line,
                });
            }
        }
    }
    out
}

/// An area's standing record alone, ignoring proposed revisions.
fn standing_status(b: &Baseline, area: &str) -> AreaStatus {
    match b.areas.iter().rev().find(|a| a.id == area) {
        Some(rec) => match rec.state {
            AreaState::Answered => AreaStatus::Answered(option_ids(&rec.options)),
            AreaState::Later => AreaStatus::Later,
            AreaState::NotApplicable => AreaStatus::NotApplicable(rec.reason.clone()),
        },
        None => AreaStatus::Open,
    }
}

/// An area a design touches that the baseline has not answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OpenArea {
    pub area: String,
    pub pack: String,
    pub title: String,
    pub tokens: Vec<String>,
}

/// The areas of the followed packs whose status is open or deferred, whose `when` holds, and which
/// `design` touches.
pub fn open_areas(
    packs: &[(ProfileSource, ProfileFile)],
    b: &Baseline,
    design: &str,
) -> Vec<OpenArea> {
    let set = profile::answer_set(b, &[]);
    let mut out = Vec::new();
    for (source, file) in profile::followed(packs, b) {
        for area in &file.areas {
            if !matches!(
                area_status(b, &area.id),
                AreaStatus::Open | AreaStatus::Later
            ) || !profile::when_holds(area, &set)
            {
                continue;
            }
            let tokens = profile::touches(area, design);
            if !tokens.is_empty() {
                out.push(OpenArea {
                    area: area.id.clone(),
                    pack: source.name.clone(),
                    title: area.title.clone(),
                    tokens,
                });
            }
        }
    }
    out
}

// ---- the packs' status (`GET /projects/{p}/profile`) ------------------------------------------

/// One pack the project follows or followed, against its library's current copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PackStatus {
    pub name: String,
    pub library: Library,
    pub version: u32,
    /// The project stopped following it; its entries stay.
    pub removed: bool,
    /// How it stands against its library: `None` for a removed pack, and for a personal pack
    /// when the caller cannot see the personal library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<profile::Drift>,
    /// The drift as sentences, each saying what to run.
    pub messages: Vec<String>,
}

/// The packs of a project with their drift (FTR-033 R-5).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProfileStatus {
    pub packs: Vec<PackStatus>,
}

/// Every pack of `p`'s baseline, in its order, with its drift from [`Store::profile_drift`].
pub fn profile_status(
    store: &Store,
    p: &str,
    personal_dir: Option<&std::path::Path>,
) -> Result<ProfileStatus> {
    let b = load(store, p)?;
    let drift = store.profile_drift(p, personal_dir)?;
    let packs = b
        .packs
        .iter()
        .map(|r| {
            let d = (r.removed.is_none())
                .then(|| drift.iter().find(|d| d.source.name == r.source.name))
                .flatten()
                .cloned();
            PackStatus {
                name: r.source.name.clone(),
                library: r.source.library,
                version: r.source.version,
                removed: r.removed.is_some(),
                messages: d.as_ref().map(|d| d.messages()).unwrap_or_default(),
                drift: d,
            }
        })
        .collect();
    Ok(ProfileStatus { packs })
}

// ---- the view ---------------------------------------------------------------------------------

/// A pack the project follows or followed, with what its library's copy says now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PackView {
    #[serde(flatten)]
    pub record: PackRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
}

/// One entry as the Baseline page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EntryView {
    #[serde(flatten)]
    pub entry: Entry,
    pub in_force: bool,
    pub pack_removed: bool,
    /// Live items whose Design names it.
    pub named_by: Vec<String>,
    /// Its acceptance is in the file but did not come through kanbanr, so it is not honoured.
    pub not_accepted_through_kanbanr: bool,
}

/// One area as the Baseline page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AreaView {
    pub id: String,
    pub pack: String,
    pub title: String,
    pub question: String,
    /// open | later | proposed | answered | not_applicable
    pub status: String,
    pub options: Vec<Chosen>,
    pub value: String,
    pub reason: String,
    pub item: String,
    pub revision: String,
    pub entries: Vec<EntryView>,
}

/// One revision with what accepting it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RevisionView {
    #[serde(flatten)]
    pub revision: Revision,
    pub state: RevisionState,
    /// What the person pins when accepting.
    pub rev: String,
    /// The entries it adds.
    pub entries: Vec<Entry>,
    /// The entries it replaces.
    pub replaces_entries: Vec<Entry>,
    /// (stage, gate) acceptance will require.
    pub stage_gates: Vec<(String, String)>,
    /// Items naming a replaced entry, their ADRs, and docs holding its id.
    pub depends_on: Vec<String>,
    /// `kanbanr baseline accept REV-n`.
    pub command: String,
    /// What the proposal or the acceptance noticed and did not refuse: an area whose `when` does
    /// not hold, a non-goal that gave way to another choice, a stage gate waiting for releases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Accepted, but the board's history cannot be read, so whether the acceptance came through
    /// kanbanr cannot be checked: why, naming the repository (FTR-033 P4 Q4, as DEF-080's
    /// `grant_unverifiable`). The rest of the view still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_unverifiable: Option<String>,
}

/// The Baseline page's whole read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BaselineView {
    pub packs: Vec<PackView>,
    pub areas: Vec<AreaView>,
    pub revisions: Vec<RevisionView>,
    /// The charter's constraints, shown read-only beside the baseline.
    pub charter_constraints: Vec<String>,
}

/// Revision `id` of `b` as viewed. `named_by` maps an entry id to the live items naming it,
/// `in_docs` to the docs holding it.
fn revision_view(
    b: &Baseline,
    id: &str,
    named_by: &BTreeMap<String, Vec<String>>,
    in_docs: &BTreeMap<String, Vec<String>>,
) -> RevisionView {
    let revision = b
        .revisions
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .expect("the revision was just found or written");
    let pick = |ids: &[String]| -> Vec<Entry> {
        b.entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect()
    };
    let entries = pick(&revision.adds);
    let replaces_entries = pick(&revision.replaces);
    let stage_gates = promised_gates(&entries.iter().collect::<Vec<_>>())
        .into_iter()
        .map(|(stage, gate, _)| (stage, gate))
        .collect();
    let mut depends_on: Vec<String> = Vec::new();
    for e in &replaces_entries {
        depends_on.extend(named_by.get(&e.id).cloned().unwrap_or_default());
        if !e.adr.is_empty() {
            depends_on.push(e.adr.clone());
        }
        depends_on.extend(in_docs.get(&e.id).cloned().unwrap_or_default());
    }
    let mut seen = BTreeSet::new();
    depends_on.retain(|d| seen.insert(d.clone()));
    RevisionView {
        state: revision.state(),
        rev: revision.rev(),
        command: format!("kanbanr baseline accept {}", revision.id),
        revision,
        entries,
        replaces_entries,
        stage_gates,
        depends_on,
        warnings: Vec::new(),
        acceptance_unverifiable: None,
    }
}

/// The Baseline page's read (`GET /projects/{p}/baseline`): the packs with their drift, every area
/// of every pack the record lists with its status and entries, every revision with what accepting
/// it does, and the charter's constraints. Live items' specs are read once for the entries their
/// Designs name, and the board's docs once for `B-n` in code spans. Changes nothing.
pub fn view(store: &Store, p: &str, _review_url: Option<&str>) -> Result<BaselineView> {
    let b = load(store, p)?;
    let project = store.load(p)?;
    let ids: BTreeSet<&str> = b.entries.iter().map(|e| e.id.as_str()).collect();
    let mut named_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &project.features {
        if !crate::graph::is_live_work(&project.config, &f.status) {
            continue;
        }
        for id in baseline_section(&f.specification).into_keys() {
            if ids.contains(id.as_str()) {
                named_by.entry(id).or_default().push(f.code.clone());
            }
        }
    }
    let mut in_docs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if !ids.is_empty() {
        let names: BTreeSet<crate::docdrift::Name> = ids
            .iter()
            .map(|id| crate::docdrift::Name {
                kind: crate::docdrift::NameKind::Name,
                text: id.to_string(),
            })
            .collect();
        for doc in store.list_docs(p).unwrap_or_default() {
            if !doc.ends_with(".md") {
                continue;
            }
            let Ok(text) = store.read_doc(p, &doc) else {
                continue;
            };
            for hit in crate::docdrift::scan(&text, &names) {
                in_docs.entry(hit.name.text).or_default().push(doc.clone());
            }
        }
    }
    let drift = store.profile_drift(p, None).unwrap_or_default();
    let packs: Vec<PackView> = b
        .packs
        .iter()
        .map(|r| PackView {
            record: r.clone(),
            messages: drift
                .iter()
                .filter(|d| d.source.name == r.source.name)
                .flat_map(|d| d.messages())
                .collect(),
        })
        .collect();
    let files = every_pack(store, p, &b);
    let removed = removed_packs(&b);
    let accepted = accepted_ids(&b);
    // A history that cannot be read marks the revision, never the whole view (P4 Q4): its
    // entries are not called "not accepted through kanbanr", and the page says why.
    let mut through: BTreeMap<&str, bool> = BTreeMap::new();
    let mut unverifiable: BTreeMap<&str, String> = BTreeMap::new();
    for r in b
        .revisions
        .iter()
        .filter(|r| accepted.contains(r.id.as_str()))
    {
        match accepted_through_kanbanr(store, p, &r.id) {
            Ok(ok) => {
                through.insert(r.id.as_str(), ok);
            }
            Err(e) => {
                unverifiable.insert(
                    r.id.as_str(),
                    format!(
                        "cannot tell whether {} was accepted through kanbanr: {e}",
                        r.id
                    ),
                );
            }
        }
    }
    let force: BTreeSet<&str> = in_force(&b).into_iter().map(|e| e.id.as_str()).collect();
    let entry_view = |e: &Entry| EntryView {
        in_force: force.contains(e.id.as_str()),
        pack_removed: removed.contains(e.pack.as_str()),
        named_by: named_by.get(&e.id).cloned().unwrap_or_default(),
        not_accepted_through_kanbanr: !through.get(e.revision.as_str()).copied().unwrap_or(true),
        entry: e.clone(),
    };
    let area_view = |id: &str, pack: &str, title: &str, question: &str| {
        let status = match area_status(&b, id) {
            AreaStatus::Open => "open",
            AreaStatus::Later => "later",
            AreaStatus::Proposed(_) => "proposed",
            AreaStatus::Answered(_) => "answered",
            AreaStatus::NotApplicable(_) => "not_applicable",
        };
        let rec = b.areas.iter().rev().find(|a| a.id == id);
        AreaView {
            id: id.to_string(),
            pack: pack.to_string(),
            title: title.to_string(),
            question: question.to_string(),
            status: status.to_string(),
            options: rec.map(|r| r.options.clone()).unwrap_or_default(),
            value: rec.map(|r| r.value.clone()).unwrap_or_default(),
            reason: rec.map(|r| r.reason.clone()).unwrap_or_default(),
            item: rec.map(|r| r.item.clone()).unwrap_or_default(),
            revision: rec.map(|r| r.revision.clone()).unwrap_or_default(),
            entries: b
                .entries
                .iter()
                .filter(|e| e.area == id && accepted.contains(e.revision.as_str()))
                .map(entry_view)
                .collect(),
        }
    };
    let mut areas: Vec<AreaView> = Vec::new();
    for (name, file) in &files {
        if let Some(file) = file {
            for a in &file.areas {
                if !areas.iter().any(|x| x.id == a.id) {
                    areas.push(area_view(&a.id, name, &a.title, &a.question));
                }
            }
        }
    }
    // Areas the record holds that no pack copy describes (a gone pack, a charter constraint).
    let mut loose: Vec<(String, String)> = Vec::new();
    for (id, pack) in b
        .areas
        .iter()
        .map(|a| {
            let pack = b
                .entries
                .iter()
                .find(|e| e.area == a.id)
                .map(|e| e.pack.clone())
                .unwrap_or_default();
            (a.id.clone(), pack)
        })
        .chain(b.entries.iter().map(|e| (e.area.clone(), e.pack.clone())))
    {
        if !areas.iter().any(|x| x.id == id) && !loose.iter().any(|(x, _)| *x == id) {
            loose.push((id, pack));
        }
    }
    for (id, pack) in &loose {
        areas.push(area_view(id, pack, id, ""));
    }
    let revisions = b
        .revisions
        .iter()
        .map(|r| RevisionView {
            acceptance_unverifiable: unverifiable.get(r.id.as_str()).cloned(),
            ..revision_view(&b, &r.id, &named_by, &in_docs)
        })
        .collect();
    let charter_constraints = crate::charter::load(store, p)
        .map(|c| c.constraints)
        .unwrap_or_default();
    Ok(BaselineView {
        packs,
        areas,
        revisions,
        charter_constraints,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn stamp() -> Stamp {
        Stamp {
            by: "Claude".into(),
            at: "2026-11-17T10:00:00Z".into(),
            origin: "conversation".into(),
            rev: String::new(),
            reason: String::new(),
            extra: Extra::default(),
        }
    }

    /// A many area with two asks_value options keeps both values through a write and a read; a
    /// bare id and the older `option:` form still read (coordinator, 17 Nov 2026).
    #[test]
    fn a_many_answer_keeps_a_value_per_chosen_option() {
        let a = Answer {
            area: "ui-style".into(),
            options: vec![
                Chosen::new("component-library", "MUI"),
                Chosen::new("design-system", "docs/design-system.md"),
                Chosen::new("utility-classes", ""),
            ],
            ..Answer::default()
        };
        let yaml = crate::yaml::to_string(&a).unwrap();
        assert!(yaml.contains("- utility-classes"), "{yaml}");
        let back: Answer = crate::yaml::from_str(&yaml).unwrap();
        assert_eq!(back, a);

        let old: Answer =
            crate::yaml::from_str("area: edge\noption: proxy-ends-tls\nvalue: nginx\n").unwrap();
        assert_eq!(old.options, vec![Chosen::new("proxy-ends-tls", "nginx")]);
        let bare: Answer =
            crate::yaml::from_str("area: database\noptions: [postgresql]\n").unwrap();
        assert_eq!(bare.options, vec![Chosen::new("postgresql", "")]);
    }

    #[test]
    fn area_status_reads_proposed_then_the_standing_record() {
        let mut b = Baseline::default();
        assert_eq!(area_status(&b, "database"), AreaStatus::Open);
        b.areas.push(AreaRecord {
            id: "database".into(),
            state: AreaState::Answered,
            options: vec![Chosen::new("postgresql", "")],
            value: String::new(),
            reason: String::new(),
            item: String::new(),
            revision: "REV-001".into(),
            extra: Extra::default(),
        });
        assert_eq!(
            area_status(&b, "database"),
            AreaStatus::Answered(vec!["postgresql".into()])
        );
        b.revisions.push(Revision {
            id: "REV-002".into(),
            item: String::new(),
            note: String::new(),
            answers: vec![Answer {
                area: "database".into(),
                later: true,
                ..Answer::default()
            }],
            adds: vec![],
            replaces: vec![],
            proposed: stamp(),
            accepted: None,
            withdrawn: None,
            extra: Extra::default(),
        });
        assert_eq!(
            area_status(&b, "database"),
            AreaStatus::Proposed("REV-002".into())
        );
        b.revisions[0].withdrawn = Some(stamp());
        assert_eq!(b.revisions[0].state(), RevisionState::Withdrawn);
        assert!(matches!(
            area_status(&b, "database"),
            AreaStatus::Answered(_)
        ));
    }

    // ---- P3: the record, its acceptance and its stage gates ----------------------------------

    /// A test pack with no `requires`: a `many` database area whose PostgreSQL option records a
    /// constraint and a practice and promises a `migrations-check` gate at stage `main`.
    pub(crate) const STORE_PACK: &str = r#"
profile:
  name: store
  description: The database, for the tests
  version: 1
areas:
  - id: database
    title: The database
    asks: first_need
    select: many
    touches: [migrations/, SQL, database]
    question: Which database does the project use?
    options:
      - id: postgresql
        label: PostgreSQL
        recommended: true
        why: One relational database
        fits_when: More than one writer
        costs: A server to run
        mentions: [PostgreSQL, Postgres]
        records:
          - {kind: constraint, text: "{label} is the database", why: "The store the team runs"}
          - {kind: practice, text: "A design that adds a table names its migration"}
        adds:
          release_gates: [{stage: main, gate: migrations-check}]
      - id: sqlite
        label: SQLite
        why: A file beside the binary
        fits_when: One writer
        costs: No concurrent writers
        mentions: [SQLite]
        records:
          - {kind: constraint, text: "{label} is the database"}
"#;

    /// A second test pack: a scanning area whose option promises `sast` at `main`.
    pub(crate) const SCAN_PACK: &str = r#"
profile:
  name: scan
  description: Scanning, for the tests
  version: 1
areas:
  - id: scanning
    title: Static analysis
    asks: first_need
    touches: [SAST, scan]
    question: What scans the code?
    options:
      - id: sast
        label: SAST
        recommended: true
        why: Finds unsafe patterns
        fits_when: Anything on a network
        costs: Run time
        records:
          - {kind: practice, text: "SAST runs before every release"}
        adds:
          release_gates: [{stage: main, gate: sast}]
      - id: none
        label: None
        why: Nothing scans
        fits_when: A prototype
        costs: Findings reach production
        records:
          - {kind: non_goal, text: "No static analysis"}
"#;

    pub(crate) fn person() -> Stamp {
        Stamp {
            by: "Ada".into(),
            at: "2026-11-17T11:00:00Z".into(),
            origin: "monitor".into(),
            ..Stamp::default()
        }
    }

    /// A board in a fresh temp folder (L-41: never under the repository) with one feature, and,
    /// with `stages`, the recommended release stages candidate, main, tag declared.
    pub(crate) fn board(stages: bool) -> (Store, std::path::PathBuf, String) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-baseline-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.join(".kanbanr").exists());
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::config::ProjectConfig::default_for("demo"))
            .unwrap();
        if stages {
            let mut c = store.load_meta("demo").unwrap().config.cadence;
            let (names, gates, _) = crate::config::Cadence::recommended(false);
            c.releases = true;
            c.release_stages = names;
            c.release_gates = gates;
            store.set_cadence("demo", None, c).unwrap();
        }
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let code = store
            .add_feature("demo", "Orders", "spec", "M", None)
            .unwrap()
            .code;
        (store, dir, code)
    }

    pub(crate) fn follow(store: &Store, yaml: &str) {
        let file = ProfileFile::from_yaml(yaml).unwrap();
        let name = file.profile.as_ref().unwrap().name.clone();
        add_pack(store, "demo", &name, &file, Library::Board, stamp()).unwrap();
    }

    pub(crate) fn answer(area: &str, options: &[&str]) -> Answer {
        Answer {
            area: area.into(),
            options: options.iter().map(|o| Chosen::new(*o, "")).collect(),
            ..Answer::default()
        }
    }

    pub(crate) fn propose_answers(store: &Store, item: &str, answers: Vec<Answer>) -> RevisionView {
        let b = load(store, "demo").unwrap();
        let packs = followed_packs(store, "demo", &b).unwrap();
        propose(
            store,
            "demo",
            Proposal {
                item: item.into(),
                note: String::new(),
                answers,
            },
            &packs,
            stamp(),
        )
        .unwrap()
    }

    pub(crate) fn accept_it(store: &Store, v: &RevisionView) -> RevisionView {
        accept(store, "demo", &v.revision.id, &v.rev, person()).unwrap()
    }

    fn main_gates(store: &Store) -> Vec<(String, String)> {
        let c = store.load_meta("demo").unwrap().config.cadence;
        c.release_gates
            .get("main")
            .map(|g| {
                g.gates
                    .iter()
                    .map(|s| (s.gate.clone(), s.from.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// R-3: an answer is a proposed revision, inert until the person accepts it, pinned.
    #[test]
    fn an_answer_becomes_a_proposed_revision_in_effect_only_once_the_person_accepts_it() {
        let (store, dir, code) = board(true);
        follow(&store, STORE_PACK);
        let spec_before = store.feature_spec("demo", &code).unwrap();
        let v = propose_answers(&store, &code, vec![answer("database", &["postgresql"])]);
        assert_eq!(v.revision.id, "REV-001");
        assert_eq!(v.state, RevisionState::Proposed);
        assert_eq!(v.command, "kanbanr baseline accept REV-001");
        let kinds: Vec<Kind> = v.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Kind::Constraint, Kind::Practice]);
        assert_eq!(v.entries[0].text, "PostgreSQL is the database");
        assert_eq!(v.entries[0].touches, ["migrations/", "SQL", "database"]);
        assert_eq!(v.entries[0].mentions["sqlite"], ["SQLite"]);
        assert_eq!(
            v.stage_gates,
            vec![("main".to_string(), "migrations-check".to_string())]
        );
        let b = load(&store, "demo").unwrap();
        assert!(b.areas.is_empty(), "no standing answer before acceptance");
        assert!(in_force(&b).is_empty());
        assert_eq!(
            area_status(&b, "database"),
            AreaStatus::Proposed("REV-001".into())
        );
        assert!(main_gates(&store).is_empty());

        // A second proposal for the same area waits on the first.
        let packs = followed_packs(&store, "demo", &b).unwrap();
        let again = propose(
            &store,
            "demo",
            Proposal {
                answers: vec![answer("database", &["sqlite"])],
                ..Proposal::default()
            },
            &packs,
            stamp(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            again.contains("REV-001 already proposes 'database'"),
            "{again}"
        );

        let wrong = accept(&store, "demo", "REV-001", "stale", person())
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("changed since you saw it"), "{wrong}");
        let claude = accept(&store, "demo", "REV-001", &v.rev, stamp())
            .unwrap_err()
            .to_string();
        assert!(claude.contains("the person's own"), "{claude}");
        assert!(in_force(&load(&store, "demo").unwrap()).is_empty());

        let done = accept_it(&store, &v);
        assert_eq!(done.state, RevisionState::Accepted);
        let b = load(&store, "demo").unwrap();
        assert_eq!(in_force(&b).len(), 2);
        assert_eq!(
            area_status(&b, "database"),
            AreaStatus::Answered(vec!["postgresql".into()])
        );
        assert_eq!(b.areas[0].item, code);
        assert_eq!(b.revisions[0].accepted.as_ref().unwrap().rev, v.rev);
        // Nothing is written on the item.
        assert_eq!(store.feature_spec("demo", &code).unwrap(), spec_before);
        let twice = accept(&store, "demo", "REV-001", &v.rev, person())
            .unwrap_err()
            .to_string();
        assert!(twice.contains("already accepted"), "{twice}");

        // Withdrawing: only a proposed revision, with a reason; the record stays.
        let w = propose_answers(&store, "", vec![answer("database", &["sqlite"])]);
        assert!(withdraw(&store, "demo", &w.revision.id, " ", stamp()).is_err());
        let gone = withdraw(&store, "demo", &w.revision.id, "not now", stamp()).unwrap();
        assert_eq!(gone.state, RevisionState::Withdrawn);
        assert!(accept(&store, "demo", &w.revision.id, &w.rev, person()).is_err());
        assert!(withdraw(&store, "demo", "REV-001", "late", stamp()).is_err());
        let b = load(&store, "demo").unwrap();
        assert_eq!(b.revisions.len(), 2);
        assert!(matches!(
            area_status(&b, "database"),
            AreaStatus::Answered(_)
        ));

        // The older `option:` form reads as `options`.
        let old: Answer = crate::yaml::from_str("area: database\noption: postgresql\n").unwrap();
        assert_eq!(old.options, vec![Chosen::new("postgresql", "")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// propose's refusals, each naming the fix.
    #[test]
    fn a_proposal_is_refused_with_the_reason() {
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        let b = load(&store, "demo").unwrap();
        let packs = followed_packs(&store, "demo", &b).unwrap();
        let refused = |answers: Vec<Answer>, item: &str| {
            propose(
                &store,
                "demo",
                Proposal {
                    item: item.into(),
                    note: String::new(),
                    answers,
                },
                &packs,
                stamp(),
            )
            .unwrap_err()
            .to_string()
        };
        assert!(refused(vec![], "").contains("at least one answer"));
        assert!(refused(vec![answer("nope", &["x"])], "").contains("area 'nope'"));
        assert!(refused(vec![answer("database", &["oracle"])], "").contains("no option 'oracle'"));
        let both = Answer {
            later: true,
            ..answer("database", &["sqlite"])
        };
        assert!(refused(vec![both], "").contains("not both"));
        assert!(refused(vec![answer("database", &["sqlite"])], "FTR-999").contains("FTR-999"));
        let valued = Answer {
            area: "database".into(),
            options: vec![Chosen::new("sqlite", "v3")],
            ..Answer::default()
        };
        assert!(refused(vec![valued], "").contains("takes no value"));
        assert!(load(&store, "demo").unwrap().revisions.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-3: each constraint gets an accepted ADR with every section; a later answer supersedes the
    /// old entry and its ADR; an ADR already naming the entry is reused.
    #[test]
    fn accepting_a_revision_writes_each_constraints_adr_and_supersedes_what_it_replaces() {
        let (store, dir, code) = board(true);
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, &code, vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        let adrs = crate::adr::list(&store, "demo").unwrap();
        assert_eq!(
            adrs.len(),
            1,
            "one ADR per constraint, none for the practice"
        );
        let first = &adrs[0];
        assert_eq!(first.status, "accepted");
        assert_eq!(first.baseline, "B-001");
        assert_eq!(first.decided_origin, "monitor");
        assert_eq!(first.deciders, ["Ada"]);
        assert_eq!(first.affects, std::slice::from_ref(&code));
        assert!(first.missing_sections().is_empty(), "{}", first.body);
        assert!(first.body.contains("- SQLite: SQLite is the database"));
        assert!(first.body.contains("practice: A design that adds a table"));
        assert!(first.body.contains("names B-001"));
        let b = load(&store, "demo").unwrap();
        assert_eq!(b.entries[0].adr, first.id);
        assert!(b.entries[1].adr.is_empty());

        // Changing the answer supersedes both old entries and the ADR.
        let change = propose_answers(&store, "", vec![answer("database", &["sqlite"])]);
        assert_eq!(change.revision.replaces, ["B-001", "B-002"]);
        assert_eq!(change.replaces_entries.len(), 2);
        assert!(
            change.depends_on.contains(&first.id),
            "{:?}",
            change.depends_on
        );
        accept_it(&store, &change);
        let b = load(&store, "demo").unwrap();
        let ids: Vec<&str> = in_force(&b).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["B-003"]);
        assert_eq!(b.entries[0].superseded_by, "B-003");
        assert_eq!(b.entries[1].superseded_by, "B-003");
        let adrs = crate::adr::list(&store, "demo").unwrap();
        let old = adrs.iter().find(|a| a.id == first.id).unwrap();
        let new = adrs.iter().find(|a| a.baseline == "B-003").unwrap();
        assert_eq!(old.status, "superseded");
        assert_eq!(old.superseded_by, new.id);
        assert_eq!(new.supersedes, std::slice::from_ref(&old.id));
        // History keeps the old answer.
        assert_eq!(b.revisions.len(), 2);
        assert_eq!(b.revisions[0].answers[0].options[0].option, "postgresql");
        assert_eq!(
            area_status(&b, "database"),
            AreaStatus::Answered(vec!["sqlite".into()])
        );

        // An ADR already naming the entry (a retry after a failed write) is reused.
        let third = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        let pending = third.entries[0].id.clone();
        let made = crate::adr::create_with_body(
            &store,
            "demo",
            "PostgreSQL is the database",
            Adr {
                status: "accepted".into(),
                baseline: pending.clone(),
                ..Adr::default()
            },
            "# PostgreSQL is the database\n".into(),
        )
        .unwrap();
        let before = crate::adr::list(&store, "demo").unwrap().len();
        accept_it(&store, &third);
        assert_eq!(crate::adr::list(&store, "demo").unwrap().len(), before);
        let b = load(&store, "demo").unwrap();
        assert_eq!(
            b.entries.iter().find(|e| e.id == pending).unwrap().adr,
            made.id
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-3: acceptance writes the promised gate into the stage, and an unknown stage refuses the
    /// acceptance before anything is written, naming the stages there are.
    #[test]
    fn an_accepted_entry_with_a_release_gate_requires_it_at_the_stage() {
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        let before = store.load_meta("demo").unwrap().config.cadence;
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        assert_eq!(
            main_gates(&store),
            vec![("migrations-check".to_string(), "B-001".to_string())]
        );
        let after = store.load_meta("demo").unwrap().config.cadence;
        // Only the gate was added: the stage's own rules stay.
        assert_eq!(
            after.release_gates["main"].on_enter,
            before.release_gates["main"].on_enter
        );
        assert_eq!(
            after.release_gates["main"].requires,
            before.release_gates["main"].requires
        );
        assert!(
            load(&store, "demo").unwrap().revisions[0]
                .accepted
                .is_some()
        );
        let _ = std::fs::remove_dir_all(dir);

        // A stage declared by default only gets an entry carrying its default rules and the gate.
        let (store, dir, _) = board(false);
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = true;
        c.release_stages = vec!["candidate".into(), "main".into(), "tag".into()];
        store.set_cadence("demo", None, c).unwrap();
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        let c = store.load_meta("demo").unwrap().config.cadence;
        assert_eq!(c.release_gates.len(), 1);
        assert!(c.release_gates["main"].on_enter.is_empty());
        let effective = c.effective_release_gates();
        assert_eq!(
            effective[2].1.on_enter.len(),
            2,
            "tag still moves main and tags"
        );
        let _ = std::fs::remove_dir_all(dir);

        // With releases on, stages without `main`: refused, naming them, and nothing written.
        let (store, dir, _) = board(false);
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = true;
        c.release_stages = ["candidate", "test", "staging", "production"]
            .map(String::from)
            .to_vec();
        store.set_cadence("demo", None, c).unwrap();
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        let err = accept(&store, "demo", "REV-001", &v.rev, person())
            .unwrap_err()
            .to_string();
        assert!(err.contains("'main' is not a release stage"), "{err}");
        assert!(
            err.contains("candidate, test, staging, production"),
            "{err}"
        );
        assert!(crate::adr::list(&store, "demo").unwrap().is_empty());
        let b = load(&store, "demo").unwrap();
        assert_eq!(b.revisions[0].state(), RevisionState::Proposed);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-033 P3 Q2 (R-3): with releases off, accepting an entry whose option promises a stage
    /// gate records the entry, adds no stage gate, and says when it applies.
    #[test]
    fn with_releases_off_an_accepted_release_gate_is_recorded_and_waits_for_releases() {
        let (store, dir, _) = board(false);
        assert!(!store.load_meta("demo").unwrap().config.cadence.releases);
        follow(&store, STORE_PACK);
        let before = store.load_meta("demo").unwrap().config.cadence;
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        let accepted = accept_it(&store, &v);
        assert_eq!(accepted.state, RevisionState::Accepted);
        assert_eq!(
            accepted.warnings,
            vec![
                "B-001 requires the `migrations-check` gate at release stage main; this project \
                 has releases off, so it applies once releases are on: stage main"
                    .to_string()
            ]
        );
        let b = load(&store, "demo").unwrap();
        assert_eq!(in_force(&b).len(), 2, "both entries are recorded");
        assert_eq!(store.load_meta("demo").unwrap().config.cadence, before);
        assert!(main_gates(&store).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-033 P3 Q1 (R-3): a chosen option's non-goal is recorded unless another option chosen
    /// in the same area is the thing it excludes; then it is left out, and the view says which
    /// choice it gave way to.
    #[test]
    fn a_non_goal_gives_way_to_the_option_it_excludes_chosen_in_the_same_area() {
        const RUNS: &str = r#"
profile:
  name: runs
  description: Where it runs, for the tests
  version: 1
areas:
  - id: runs-where
    title: Where it runs
    asks: setup
    select: many
    touches: [deploy]
    question: Where does the product run?
    options:
      - id: containers
        label: Containers on a host
        recommended: true
        why: One host
        fits_when: Small
        costs: One host
        mentions: [Docker, Compose]
        records:
          - {kind: constraint, text: Containers on a host run the product}
          - {kind: non_goal, text: "No Kubernetes: containers run on a host with Compose"}
      - id: kubernetes
        label: Kubernetes
        why: A cluster
        fits_when: Many services
        costs: A cluster to run
        mentions: [Kubernetes, k8s]
        records:
          - {kind: constraint, text: Kubernetes runs the product}
      - id: host
        label: A host or VM
        why: No containers
        fits_when: One binary
        costs: Hand-run
        mentions: [systemd]
        records:
          - {kind: constraint, text: The product runs directly on a host or VM}
"#;
        let (store, dir, _) = board(false);
        follow(&store, RUNS);
        // Containers alone, or beside a host: the non-goal is recorded.
        let v = propose_answers(
            &store,
            "",
            vec![answer("runs-where", &["containers", "host"])],
        );
        assert!(
            v.entries.iter().any(|e| e.kind == Kind::NonGoal),
            "{:?}",
            v.entries
        );
        assert!(v.warnings.is_empty(), "{:?}", v.warnings);
        withdraw(&store, "demo", &v.revision.id, "try again", person()).unwrap();

        // Containers and Kubernetes: "No Kubernetes" gives way, and the view says why.
        let v = propose_answers(
            &store,
            "",
            vec![answer("runs-where", &["containers", "kubernetes"])],
        );
        let kinds: Vec<(Kind, &str)> = v
            .entries
            .iter()
            .map(|e| (e.kind, e.option.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Constraint, "containers"),
                (Kind::Constraint, "kubernetes")
            ]
        );
        assert_eq!(v.warnings.len(), 1, "{:?}", v.warnings);
        let w = &v.warnings[0];
        assert!(
            w.contains("No Kubernetes")
                && w.contains("gave way to 'Kubernetes'")
                && w.contains("runs-where"),
            "{w}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-033 P4 Q1 (R-3): a non-goal row's `excludes` names the option it gives way to; the
    /// words of the other chosen options are read only when it names none. `excludes` belongs to
    /// a non-goal and names another option of its area, or the pack has a problem.
    #[test]
    fn a_non_goal_gives_way_to_the_option_its_excludes_names_else_to_its_words() {
        const RUNS: &str = r#"
profile:
  name: runs
  description: Where it runs, for the tests
  version: 1
areas:
  - id: runs-where
    title: Where it runs
    asks: setup
    select: many
    touches: [deploy]
    question: Where does the product run?
    options:
      - id: containers
        label: Containers on a host
        recommended: true
        why: One host
        fits_when: Small
        costs: One host
        mentions: [Docker, Compose]
        records:
          - {kind: constraint, text: Containers on a host run the product}
          - {kind: non_goal, text: "No orchestrator: Compose on one host", excludes: kubernetes}
      - id: kubernetes
        label: Kubernetes
        why: A cluster
        fits_when: Many services
        costs: A cluster to run
        mentions: [Kubernetes, k8s]
        records:
          - {kind: constraint, text: Kubernetes runs the product}
      - id: host
        label: A host or VM
        why: No containers
        fits_when: One binary
        costs: Hand-run
        mentions: [systemd]
        records:
          - {kind: constraint, text: The product runs directly on a host or VM}
          - {kind: non_goal, text: "No Kubernetes and no Docker on the host", excludes: containers}
"#;
        let (store, dir, _) = board(false);
        follow(&store, RUNS);
        // The text names no word of Kubernetes, but `excludes` does: it gives way.
        let v = propose_answers(
            &store,
            "",
            vec![answer("runs-where", &["containers", "kubernetes"])],
        );
        assert!(
            !v.entries
                .iter()
                .any(|e| e.kind == Kind::NonGoal && e.option == "containers"),
            "{:?}",
            v.entries
        );
        assert_eq!(v.warnings.len(), 1, "{:?}", v.warnings);
        assert!(
            v.warnings[0].contains("gave way to 'Kubernetes'")
                && v.warnings[0].contains("it excludes 'kubernetes'"),
            "{}",
            v.warnings[0]
        );
        withdraw(&store, "demo", &v.revision.id, "try again", person()).unwrap();
        // The text says Kubernetes, but `excludes` names containers, which is not chosen: kept.
        let v = propose_answers(
            &store,
            "",
            vec![answer("runs-where", &["host", "kubernetes"])],
        );
        assert!(
            v.entries
                .iter()
                .any(|e| e.kind == Kind::NonGoal && e.option == "host"),
            "{:?}",
            v.entries
        );
        assert!(v.warnings.is_empty(), "{:?}", v.warnings);
        let _ = std::fs::remove_dir_all(dir);

        // `excludes` on a constraint, or naming no other option of the area: a problem each.
        let bad = RUNS
            .replace(
                "{kind: constraint, text: Kubernetes runs the product}",
                "{kind: constraint, text: Kubernetes runs the product, excludes: host}",
            )
            .replace("excludes: containers}", "excludes: lambda}");
        let problems: Vec<String> = ProfileFile::from_yaml(&bad)
            .unwrap()
            .problems()
            .iter()
            .map(|e| e.to_string())
            .collect();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`excludes` is for a non_goal, not a constraint")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`excludes` names 'lambda'")),
            "{problems:?}"
        );
    }

    /// FTR-033 P4 Q3 (R-3): a gate accepted while releases were off lands when releases are
    /// turned on, by the function acceptance uses; a removed pack's gate does not.
    #[test]
    fn turning_releases_on_writes_the_stage_gates_an_accepted_revision_waited_for() {
        let (store, dir, _) = board(false);
        follow(&store, STORE_PACK);
        follow(&store, SCAN_PACK);
        let v = propose_answers(
            &store,
            "",
            vec![
                answer("database", &["postgresql"]),
                answer("scanning", &["sast"]),
            ],
        );
        let accepted = accept_it(&store, &v);
        assert_eq!(accepted.warnings.len(), 2, "{:?}", accepted.warnings);
        assert!(main_gates(&store).is_empty());
        remove_pack(&store, "demo", "scan", stamp()).unwrap();
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = true;
        store.set_cadence("demo", None, c).unwrap();
        assert_eq!(
            main_gates(&store),
            vec![("migrations-check".to_string(), "B-001".to_string())]
        );
        // Turning releases off and on again adds nothing twice.
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = false;
        store.set_cadence("demo", None, c).unwrap();
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = true;
        store.set_cadence("demo", None, c).unwrap();
        assert_eq!(main_gates(&store).len(), 1, "{:?}", main_gates(&store));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-033 P4 Q4 (R-3): when the board's history cannot be read, the view still reads; the
    /// accepted revision says why its acceptance cannot be checked, and its entries are not
    /// called "not accepted through kanbanr".
    #[test]
    fn a_view_whose_history_cannot_be_read_marks_the_revision_and_still_reads() {
        let (store, dir, _) = board(false);
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        if dir.join(".git").exists() {
            std::fs::rename(dir.join(".git"), dir.join(".git-away")).unwrap();
        }
        let seen = view(&store, "demo", None).expect("the view reads");
        let json = serde_json::to_value(&seen).unwrap();
        let why = json["revisions"][0]["acceptance_unverifiable"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            why.contains("cannot tell whether REV-001 was accepted through kanbanr"),
            "{json}"
        );
        let area = json["areas"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "database")
            .unwrap()
            .clone();
        assert!(!area["entries"].as_array().unwrap().is_empty(), "{area}");
        for e in area["entries"].as_array().unwrap() {
            assert_eq!(e["not_accepted_through_kanbanr"], false, "{e}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-3: a superseding revision, a `later` answer or removing the pack drops only the stage
    /// gates of its own entries; adding the pack again restores them.
    #[test]
    fn withdrawing_the_entry_removes_the_stage_requirement_it_added() {
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        follow(&store, SCAN_PACK);
        let v = propose_answers(
            &store,
            "",
            vec![
                answer("database", &["postgresql"]),
                answer("scanning", &["sast"]),
            ],
        );
        accept_it(&store, &v);
        let gate = |g: &str, from: &str| (g.to_string(), from.to_string());
        assert_eq!(
            main_gates(&store),
            vec![gate("migrations-check", "B-001"), gate("sast", "B-003")]
        );
        // Superseded by an answer that promises nothing.
        let v = propose_answers(&store, "", vec![answer("database", &["sqlite"])]);
        accept_it(&store, &v);
        assert_eq!(main_gates(&store), vec![gate("sast", "B-003")]);
        // Answered again, then deferred: the deferral withdraws the entry and its gate.
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        assert_eq!(
            main_gates(&store),
            vec![gate("sast", "B-003"), gate("migrations-check", "B-005")]
        );
        let later = Answer {
            area: "database".into(),
            later: true,
            ..Answer::default()
        };
        let v = propose_answers(&store, "", vec![later]);
        assert_eq!(v.revision.replaces, ["B-005", "B-006"]);
        accept_it(&store, &v);
        assert_eq!(main_gates(&store), vec![gate("sast", "B-003")]);
        let b = load(&store, "demo").unwrap();
        assert_eq!(area_status(&b, "database"), AreaStatus::Later);
        assert_eq!(b.entries[4].superseded_by, "REV-004");
        // Removing the pack drops its entries' gates; adding it again restores them.
        remove_pack(&store, "demo", "scan", person()).unwrap();
        assert!(main_gates(&store).is_empty());
        let b = load(&store, "demo").unwrap();
        assert!(b.entries.iter().any(|e| e.id == "B-003"), "entries stay");
        follow(&store, SCAN_PACK);
        assert_eq!(main_gates(&store), vec![gate("sast", "B-003")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// R-4: the Design's `### Baseline` subsection, parsed.
    #[test]
    fn the_baseline_section_names_entries_with_a_test_or_a_reason() {
        let spec = "# Orders\n\n## Why\n\n### Baseline\n\n- B-009: outside the Design\n\n\
                    ## Design\n\nIt adds `migrations/0002.sql`.\n\n```\n## not a heading\n```\n\n\
                    ### Baseline\n\n- B-003: migrations::tests::applies_twice\n\
                    * **B-004** — no table is added\n- b-5: lower-case id\n- B-006:\n\
                    - **B-007:** bold with the colon inside\n- not an entry\n\n\
                    ### Tasks\n\n- B-008: after the subsection\n\n## Test plan\n\n- B-010: later\n";
        let named = baseline_section(spec);
        let ids: Vec<&str> = named.keys().map(String::as_str).collect();
        assert_eq!(ids, ["B-003", "B-004", "B-007", "B-5"]);
        assert_eq!(named["B-003"], "migrations::tests::applies_twice");
        assert_eq!(named["B-004"], "no table is added");
        assert_eq!(named["B-5"], "lower-case id");
        let design = design_text(spec);
        assert!(design.starts_with("## Design"));
        assert!(design.contains("## not a heading") && !design.contains("Test plan"));
        assert!(baseline_section("# T\n\n### Baseline\n\n- B-001: x\n").is_empty());
        assert_eq!(design_text("# T\n\nno design here\n"), "");
        assert!(baseline_section("").is_empty());
    }

    /// R-2: an accepted area is not asked again; a deferred one is asked at the next design that
    /// touches it, marked deferred.
    #[test]
    fn an_answered_area_is_never_asked_again_and_a_deferred_one_is_asked_at_the_next_design_that_touches_it()
     {
        let design = "## Design\n\nAdds `migrations/0002_orders.sql`.\n";
        let other = "## Design\n\nA query on the database.\n";
        let asked = |store: &Store, d: &str| {
            let b = load(store, "demo").unwrap();
            let packs = followed_packs(store, "demo", &b).unwrap();
            profile::ask(
                &packs,
                &b,
                &[],
                profile::Asks::FirstNeed,
                None,
                None,
                Some(d),
                "",
            )
        };
        let (store, dir, code) = board(true);
        follow(&store, STORE_PACK);
        let a = asked(&store, design);
        let ids: Vec<&str> = a.ask.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["database"]);
        assert!(!a.ask[0].deferred);
        let open = open_areas(
            &followed_packs(&store, "demo", &load(&store, "demo").unwrap()).unwrap(),
            &load(&store, "demo").unwrap(),
            design,
        );
        assert_eq!(open.len(), 1);
        // `.sql` holds the word SQL, as P2 found.
        assert_eq!(open[0].tokens, ["SQL", "migrations/"]);
        let v = propose_answers(&store, &code, vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        for d in [design, other] {
            let a = asked(&store, d);
            assert!(a.ask.is_empty(), "{:?}", a.ask);
            assert_eq!(a.answered, ["database"]);
        }
        let b = load(&store, "demo").unwrap();
        assert!(open_areas(&followed_packs(&store, "demo", &b).unwrap(), &b, design).is_empty());
        let t = touched(&b, design);
        assert_eq!(t.len(), 2);
        assert!(contradictions(&b, design).is_empty());
        let c = contradictions(&b, "## Design\n\nline one\nIt keeps SQLite for tests.\n");
        assert_eq!(c.len(), 0, "SQLite alone does not touch the database area");
        let c = contradictions(&b, "## Design\n\nThe database\nkeeps SQLite for tests.\n");
        assert_eq!(c[0].option, "sqlite");
        assert_eq!(c[0].line, 4);
        let _ = std::fs::remove_dir_all(dir);

        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        let later = Answer {
            area: "database".into(),
            later: true,
            ..Answer::default()
        };
        let v = propose_answers(&store, "", vec![later]);
        assert!(v.entries.is_empty());
        accept_it(&store, &v);
        let a = asked(&store, other);
        assert_eq!(a.ask.len(), 1);
        assert!(a.ask[0].deferred);
        assert_eq!(a.deferred, ["database"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Compat: keys this version does not know survive a rewrite on every record type.
    #[test]
    fn an_older_record_keeps_unknown_keys_through_a_rewrite() {
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        let path = store.project_dir("demo").join(BASELINE_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut b: Baseline = crate::yaml::from_str(&text).unwrap();
        let key = |k: &str| (k.to_string(), serde_json::json!(format!("kept {k}")));
        b.extra.extend([key("future_top")]);
        b.packs[0].extra.extend([key("future_pack")]);
        b.entries[0].extra.extend([key("future_entry")]);
        b.revisions[0].extra.extend([key("future_revision")]);
        std::fs::write(&path, crate::yaml::to_string(&b).unwrap()).unwrap();
        // A rewrite through a write of this version keeps them all.
        withdraw(&store, "demo", "REV-001", "rewrite", stamp()).unwrap();
        let back = load(&store, "demo").unwrap();
        assert_eq!(back.extra["future_top"], "kept future_top");
        assert_eq!(back.packs[0].extra["future_pack"], "kept future_pack");
        assert_eq!(back.entries[0].extra["future_entry"], "kept future_entry");
        assert_eq!(
            back.revisions[0].extra["future_revision"],
            "kept future_revision"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod perf {
    //! FTR-033: the Baseline page's read on a board with many live items.
    use super::tests::*;
    use super::*;

    const BUDGET: std::time::Duration = std::time::Duration::from_millis(150);

    #[test]
    #[ignore = "a timing budget: run with --ignored, as review::perf is"]
    fn view_on_a_board_with_many_items() {
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        // 60 in-force entries, as the budget names.
        let mut b = load(&store, "demo").unwrap();
        let template = b.entries[1].clone();
        for n in 3..=60 {
            let mut e = template.clone();
            e.id = format!("B-{n:03}");
            e.area = format!("area-{n}");
            b.entries.push(e);
        }
        save(&store, "demo", &b).unwrap();
        let spec = format!(
            "# Item\n\n## Design\n\n{}\n### Baseline\n\n- B-001: migrations::tests::applies\n",
            "It adds a table under `migrations/`.\n".repeat(400)
        );
        for i in 0..200 {
            store
                .add_feature("demo", &format!("Item {i}"), &spec, "M", None)
                .unwrap();
        }
        let started = std::time::Instant::now();
        let view = view(&store, "demo", None).unwrap();
        let elapsed = started.elapsed();
        eprintln!(
            "baseline::view: {} areas, {} revisions, 200 items in {elapsed:?}",
            view.areas.len(),
            view.revisions.len()
        );
        assert_eq!(
            view.areas[0].entries[0].named_by.len(),
            200,
            "every item names B-001"
        );
        assert!(
            elapsed <= BUDGET,
            "{elapsed:?} is over the budget {BUDGET:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
