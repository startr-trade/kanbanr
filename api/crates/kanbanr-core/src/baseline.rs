//! A project's engineering baseline (FTR-033): the project-wide answers to its practice packs'
//! questions, kept beside the charter as `projects/<id>/baseline.yaml`.
//!
//! Every list is append-only. An answer is proposed as a revision; it is in force only once the
//! person accepts that revision on the monitor or in their terminal. Reading never writes
//! (ADR-0005). This file holds the record's types and its reader; the writes come with P3.

use crate::decisions::Stamp;
use crate::models::Extra;
use crate::profile::{Adds, Kind, ProfileSource};
use crate::{Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Stamp {
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
}
