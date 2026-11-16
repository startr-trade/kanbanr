//! The known vocabularies for tags, and their one spelling (MNT-009).
//!
//! A requirement's ISO/IEC 25010 tags, an ADR's Zachman columns, qualities and layer are free
//! strings on disk. Two spellings of one value ("Performance efficiency", "Performance
//! Efficiency") split every count that reads them, so each vocabulary has a canonical form, and
//! every count goes through it.
//!
//! On requirements the canonical form is applied when a tag is added or changed: a tag carried
//! unchanged from the stored definition is kept as stored, because a tag is part of the
//! definition's content hash and rewriting it would lapse the item's approval for a change of
//! case (ADR-0015). On ADRs nothing hangs on the bytes, so they are read in canonical form.

use crate::adr::Adr;
use crate::error::{CoreError, Result};
use crate::models::FeatureDefinition;
use std::collections::BTreeMap;

/// The Zachman framework's six interrogatives, as `Zachman::columns()` names them.
pub const ZACHMAN_COLUMNS: [&str; 6] = ["What", "How", "Where", "Who", "When", "Why"];

/// The TOGAF architecture layers an ADR sits in.
pub const LAYERS: [&str; 3] = ["conceptual", "logical", "physical"];

/// The canonical ISO/IEC 25010:2023 characteristic, ignoring case, spaces and hyphens.
pub fn iso(tag: &str) -> Option<&'static str> {
    crate::ears::normalize_iso(tag)
}

/// The Zachman column `column` names, ignoring surrounding space and ASCII case.
pub fn zachman(column: &str) -> Option<&'static str> {
    let column = column.trim();
    ZACHMAN_COLUMNS
        .into_iter()
        .find(|c| c.eq_ignore_ascii_case(column))
}

/// The layer `name` names, ignoring surrounding space and ASCII case.
pub fn layer(name: &str) -> Option<&'static str> {
    let name = name.trim();
    LAYERS.into_iter().find(|l| l.eq_ignore_ascii_case(name))
}

/// The refusal for a tag outside ISO/IEC 25010, naming the nine. "Portability" is the 2011 name,
/// so it is told what the 2023 revision calls it.
pub fn unknown_iso(tag: &str) -> String {
    let key: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let hint = if key == "portability" {
        " (ISO/IEC 25010:2023 calls Portability Flexibility)"
    } else {
        ""
    };
    format!(
        "'{tag}' is not an ISO/IEC 25010 characteristic: use one of {}{hint}",
        crate::ears::ISO25010.join(", ")
    )
}

fn unknown_zachman(column: &str) -> String {
    format!(
        "'{column}' is not a Zachman column: use one of {}",
        ZACHMAN_COLUMNS.join(", ")
    )
}

fn unknown_layer(name: &str) -> String {
    format!("'{name}' is not a layer: use one of {}", LAYERS.join(" | "))
}

/// Push `value` unless it is already there: duplicates after canonicalisation go, the first kept.
fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.contains(&value) {
        out.push(value);
    }
}

/// Store each requirement's ISO tags in their canonical spelling. A tag equal, byte for byte, to
/// one the same requirement already stores in `previous` is kept as it is (Q1, ADR-0015); every
/// other tag is canonicalised, and one with no canonical form is refused.
pub fn canonicalise_iso(
    previous: Option<&FeatureDefinition>,
    def: &mut FeatureDefinition,
) -> Result<()> {
    for requirement in &mut def.requirements {
        let stored: &[String] = previous
            .filter(|_| !requirement.id.trim().is_empty())
            .and_then(|p| p.requirements.iter().find(|r| r.id == requirement.id))
            .map(|r| r.iso.as_slice())
            .unwrap_or(&[]);
        let mut out = Vec::with_capacity(requirement.iso.len());
        for tag in &requirement.iso {
            if stored.contains(tag) {
                push_unique(&mut out, tag.clone());
                continue;
            }
            let canonical = iso(tag).ok_or_else(|| CoreError::Unsupported(unknown_iso(tag)))?;
            push_unique(&mut out, canonical.to_string());
        }
        requirement.iso = out;
    }
    Ok(())
}

/// Map a list through `canon`; an unknown value is refused when `strict`, kept when not.
fn canonical_list(
    values: &[String],
    strict: bool,
    canon: fn(&str) -> Option<&'static str>,
    refusal: fn(&str) -> String,
) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        match canon(value) {
            Some(c) => push_unique(&mut out, c.to_string()),
            None if strict => return Err(CoreError::Unsupported(refusal(value))),
            None => push_unique(&mut out, value.clone()),
        }
    }
    Ok(out)
}

/// Put an ADR's `zachman`, `quality` and `layer` in canonical form, removing duplicates. A value
/// with no canonical form is refused when `strict` (a new decision) and kept as written when not
/// (a board file being read: it still loads, and the doctor reports it).
pub fn canonicalise_adr(adr: &mut Adr, strict: bool) -> Result<()> {
    let zachman_cols = canonical_list(&adr.zachman, strict, zachman, unknown_zachman)?;
    let quality = canonical_list(&adr.quality, strict, iso, unknown_iso)?;
    let layer_name = if adr.layer.trim().is_empty() {
        adr.layer.clone()
    } else {
        match layer(&adr.layer) {
            Some(l) => l.to_string(),
            None if strict => return Err(CoreError::Unsupported(unknown_layer(&adr.layer))),
            None => adr.layer.clone(),
        }
    };
    adr.zachman = zachman_cols;
    adr.quality = quality;
    adr.layer = layer_name;
    Ok(())
}

fn tally<'a>(
    values: impl IntoIterator<Item = &'a str>,
    canon: fn(&str) -> Option<&'static str>,
) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for value in values {
        let key = canon(value).map_or_else(|| value.trim().to_string(), str::to_string);
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

/// Count ISO tags by their canonical form; an unknown tag is counted under its own (trimmed)
/// name, so it stays visible.
pub fn quality_tally<'a>(tags: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, usize> {
    tally(tags, iso)
}

/// Count Zachman columns by their canonical form; an unknown one under its own (trimmed) name.
pub fn zachman_tally<'a>(cols: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, usize> {
    tally(cols, zachman)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Requirement;

    fn nfr(id: &str, tags: &[&str]) -> Requirement {
        Requirement {
            id: id.to_string(),
            text: "THE SYSTEM SHALL answer in 100 ms.".to_string(),
            iso: tags.iter().map(|t| t.to_string()).collect(),
            ..Requirement::default()
        }
    }

    fn def(reqs: Vec<Requirement>) -> FeatureDefinition {
        FeatureDefinition {
            requirements: reqs,
            ..FeatureDefinition::default()
        }
    }

    #[test]
    fn iso_tags_are_stored_in_their_canonical_spelling() {
        let mut d = def(vec![nfr(
            "R-1",
            &[
                "performance-efficiency",
                "  SECURITY ",
                "Performance Efficiency",
            ],
        )]);
        canonicalise_iso(None, &mut d).unwrap();
        assert_eq!(
            d.requirements[0].iso,
            vec!["Performance Efficiency", "Security"]
        );
    }

    #[test]
    fn a_tag_carried_unchanged_is_kept_as_stored() {
        let previous = def(vec![
            nfr("R-1", &["Performance efficiency"]),
            nfr("R-2", &[]),
        ]);
        let mut d = def(vec![
            nfr("R-1", &["Performance efficiency", "reliability"]),
            // Another requirement's stored spelling is not this one's.
            nfr("R-2", &["Performance efficiency"]),
            // A requirement with no id yet has no stored counterpart.
            nfr("", &["Performance efficiency"]),
        ]);
        canonicalise_iso(Some(&previous), &mut d).unwrap();
        assert_eq!(
            d.requirements[0].iso,
            vec!["Performance efficiency", "Reliability"]
        );
        assert_eq!(d.requirements[1].iso, vec!["Performance Efficiency"]);
        assert_eq!(d.requirements[2].iso, vec!["Performance Efficiency"]);
    }

    #[test]
    fn an_unknown_iso_tag_is_refused_with_the_nine_and_portability_names_flexibility() {
        let mut d = def(vec![nfr("R-1", &["Speediness"])]);
        let err = canonicalise_iso(None, &mut d).unwrap_err().to_string();
        assert!(err.contains("'Speediness' is not an ISO/IEC 25010 characteristic"));
        for name in crate::ears::ISO25010 {
            assert!(err.contains(name), "{err} names {name}");
        }
        assert!(!err.contains("calls Portability Flexibility"));

        let mut d = def(vec![nfr("R-1", &["Portability"])]);
        let err = canonicalise_iso(None, &mut d).unwrap_err().to_string();
        assert!(
            err.contains("(ISO/IEC 25010:2023 calls Portability Flexibility)"),
            "{err}"
        );

        // A stored "Portability" carried unchanged is kept (Q3): it becomes Flexibility when the
        // item is next redefined with the tag changed.
        let previous = def(vec![nfr("R-1", &["Portability"])]);
        let mut d = def(vec![nfr("R-1", &["Portability"])]);
        canonicalise_iso(Some(&previous), &mut d).unwrap();
        assert_eq!(d.requirements[0].iso, vec!["Portability"]);
    }

    #[test]
    fn the_tallies_count_each_value_once_whatever_its_spelling() {
        let q = quality_tally([
            "Performance efficiency",
            "Performance Efficiency",
            "Reliability",
            " Speed ",
        ]);
        assert_eq!(q.get("Performance Efficiency"), Some(&2));
        assert_eq!(q.get("Reliability"), Some(&1));
        assert_eq!(q.get("Speed"), Some(&1));
        assert_eq!(q.len(), 3);

        let z = zachman_tally(["How", "how", "Where", "where", "who", "Who", "why", "Data"]);
        assert_eq!(z.get("How"), Some(&2));
        assert_eq!(z.get("Where"), Some(&2));
        assert_eq!(z.get("Who"), Some(&2));
        assert_eq!(z.get("Why"), Some(&1));
        assert_eq!(z.get("Data"), Some(&1));
        assert_eq!(z.len(), 5);
    }
}
