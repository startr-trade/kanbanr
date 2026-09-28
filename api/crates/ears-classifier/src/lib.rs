//! EARS requirement classification and the ISO/IEC 25010 quality vocabulary (FEAT-049).
//!
//! EARS (Easy Approach to Requirements Syntax) gives a requirement a shape that can be tested:
//! one behaviour, one trigger, no compound sentences. This module *classifies* text — it never
//! rejects it. Stored requirements keep whatever the author wrote; `doctor` reports what doesn't
//! conform. That is deliberate: a write that fails because of prose style would push people to
//! stop recording requirements at all, and the pattern is derived on demand so stored data can
//! never disagree with the classifier.
//!
//! What is deliberately NOT checked: compound responses ("SHALL validate and reject"). A regex for
//! that fires on legitimate single behaviours, and a check people learn to ignore is worse than no
//! check.

/// The five EARS patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    /// `THE SYSTEM SHALL <response>` — always true.
    Ubiquitous,
    /// `WHEN <trigger>, THE SYSTEM SHALL <response>` — event driven.
    Event,
    /// `WHILE <state>, THE SYSTEM SHALL <response>` — state driven.
    State,
    /// `WHERE <feature is included>, THE SYSTEM SHALL <response>` — optional feature.
    Optional,
    /// `IF <undesired condition>, THE SYSTEM SHALL <response>` — unwanted behaviour.
    Unwanted,
}

impl Pattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Pattern::Ubiquitous => "ubiquitous",
            Pattern::Event => "event",
            Pattern::State => "state",
            Pattern::Optional => "optional",
            Pattern::Unwanted => "unwanted",
        }
    }
}

/// The marker every EARS requirement contains.
const SHALL: &str = "THE SYSTEM SHALL ";

/// Classify a requirement. `None` means it does not conform — a warning, never an error.
///
/// Case-insensitive (the skill asks for uppercase, but rejecting lowercase would only lose
/// information), and each conditional pattern must carry a non-empty condition: `WHEN, THE SYSTEM
/// SHALL …` states no trigger and so isn't testable.
pub fn classify(text: &str) -> Option<Pattern> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = normalized.to_uppercase();
    let marker = upper.find(SHALL)?;

    // Ubiquitous: the requirement opens with the marker itself.
    if marker == 0 {
        return has_response(&upper, marker).then_some(Pattern::Ubiquitous);
    }

    let (keyword, pattern) = [
        ("WHEN ", Pattern::Event),
        ("WHILE ", Pattern::State),
        ("WHERE ", Pattern::Optional),
        ("IF ", Pattern::Unwanted),
    ]
    .into_iter()
    .find(|(keyword, _)| upper.starts_with(keyword))?;

    // The condition sits between the keyword and the marker, and must say something. A trailing
    // `THEN` before the marker is the classic unwanted-behaviour spelling.
    let condition = upper[keyword.len()..marker]
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_end_matches("THEN")
        .trim()
        .trim_end_matches(',')
        .trim()
        .to_string();
    (!condition.is_empty() && has_response(&upper, marker)).then_some(pattern)
}

/// A requirement must say what the system does, not just that it shall.
fn has_response(upper: &str, marker: usize) -> bool {
    !upper[marker + SHALL.len()..].trim().is_empty()
}

/// The nine ISO/IEC 25010 (2023) quality characteristics.
pub const ISO25010: [&str; 9] = [
    "Functional Suitability",
    "Performance Efficiency",
    "Compatibility",
    "Interaction Capability",
    "Reliability",
    "Security",
    "Maintainability",
    "Flexibility",
    "Safety",
];

/// Canonical spelling of a quality tag, ignoring case, spacing and hyphens. `None` for a tag
/// outside the standard — reported, never rejected, because a hard failure here would break a
/// board load over a typo.
pub fn normalize_iso(tag: &str) -> Option<&'static str> {
    let key = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    let wanted = key(tag);
    ISO25010.into_iter().find(|c| key(c) == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_five_patterns() {
        let cases = [
            (
                "THE SYSTEM SHALL retain a cart for 7 days.",
                Pattern::Ubiquitous,
            ),
            (
                "WHEN a cart is abandoned, THE SYSTEM SHALL retain it for 7 days.",
                Pattern::Event,
            ),
            (
                "WHILE a sync is running, THE SYSTEM SHALL reject a second sync.",
                Pattern::State,
            ),
            (
                "WHERE the mirror is enabled, THE SYSTEM SHALL update the issue.",
                Pattern::Optional,
            ),
            (
                "IF gh is not installed, THEN THE SYSTEM SHALL complete the write.",
                Pattern::Unwanted,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(classify(text), Some(expected), "{text}");
        }
        // Case and spacing are tolerated; the skill asks for uppercase but losing a lowercase
        // requirement to a style rule helps nobody.
        assert_eq!(
            classify("when a cart is abandoned,  the system shall  retain it."),
            Some(Pattern::Event)
        );
    }

    #[test]
    fn rejects_text_that_cannot_be_tested() {
        // Prose, however sincere.
        assert_eq!(classify("The cart should be user-friendly."), None);
        // The marker, but no response.
        assert_eq!(classify("THE SYSTEM SHALL"), None);
        assert_eq!(
            classify("WHEN a cart is abandoned, THE SYSTEM SHALL   "),
            None
        );
        // A conditional with no condition states no trigger.
        assert_eq!(classify("WHEN, THE SYSTEM SHALL retain the cart."), None);
        assert_eq!(classify("IF THEN THE SYSTEM SHALL retain the cart."), None);
        // A keyword that isn't an EARS opener.
        assert_eq!(
            classify("AFTER a cart is abandoned, THE SYSTEM SHALL retain it."),
            None
        );
    }

    #[test]
    fn iso_tags_are_canonicalized_and_unknown_ones_are_reported() {
        assert_eq!(
            normalize_iso("performance-efficiency"),
            Some("Performance Efficiency")
        );
        assert_eq!(normalize_iso("  SECURITY "), Some("Security"));
        assert_eq!(normalize_iso("Reliability"), Some("Reliability"));
        // Usability was renamed Interaction Capability in the 2023 revision; an old or invented
        // tag is reported rather than silently accepted.
        assert_eq!(normalize_iso("Usability"), None);
        assert_eq!(normalize_iso("Fastness"), None);
    }
}

/// The README's example is the crate's whole surface, so it is compiled and run rather than
/// trusted — a README that does not work is worse than none, because it is believed.
#[cfg(test)]
mod readme {
    use super::*;

    #[test]
    fn the_readme_example_is_true() {
        assert_eq!(
            classify("WHEN the archive is downloaded, THE SYSTEM SHALL verify its checksum"),
            Some(Pattern::Event)
        );
        assert_eq!(classify("it should probably be fast"), None);
        assert_eq!(
            normalize_iso("performance-efficiency"),
            Some("Performance Efficiency")
        );
        assert_eq!(normalize_iso("Speed"), None);
        assert_eq!(ISO25010.len(), 9);
    }
}
