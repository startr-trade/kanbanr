//! A manual test is a judgement test (FTR-039): the one check on an item that only a person can
//! make, such as whether a page reads clearly or a result answers its question.
//!
//! The stored kind stays `manual`, so older binaries, capture and the stale-test sweep behave as
//! before; every text calls it a judgement test. An item has at most one, its name starts with
//! [`PREFIX`] and says what the person judges, and it never names something a run can observe (a
//! file, a list, a checksum, a push, a page's text): that is a unit, integration or walkthrough
//! test. [`check_write`] refuses a definition write that adds a test breaking the rule; tests that
//! were already there are left until they are touched. The vocabularies are constants, decided by
//! the person (FTR-039 Q1): a misjudged wording is fixed by changing them, and there is no flag to
//! let a refused write through.

use crate::models::{FeatureDefinition, TestRef};
use crate::{CoreError, Result};
use serde::Serialize;

/// How a judgement test's name starts.
pub const PREFIX: &str = "manual:";

/// Whole-word phrases (lower case) that only a run should check, each with the kind to name.
pub const OBSERVABLE: &[(&str, &str)] = &[
    ("checksum", "integration"),
    ("checksums", "integration"),
    ("sha256", "integration"),
    ("sha256sum", "integration"),
    ("byte-for-byte", "integration"),
    ("exit code", "integration"),
    ("exits", "integration"),
    ("prints", "unit or integration"),
    ("returns", "unit or integration"),
    ("exists", "unit or integration"),
    ("is listed", "unit or integration"),
    ("lists", "unit or integration"),
    ("is pushed", "walkthrough"),
    ("pushed", "walkthrough"),
    ("pushes", "walkthrough"),
    ("is tagged", "walkthrough"),
    ("page shows", "a web check or walkthrough"),
    ("page lists", "a web check or walkthrough"),
    ("button", "a web check or walkthrough"),
];

/// Whole-word phrases (lower case) a judgement names: what the person judges.
pub const JUDGED: &[&str] = &[
    "reads",
    "read",
    "wording",
    "clear",
    "clearly",
    "understand",
    "understands",
    "understandable",
    "confusing",
    "usable",
    "usability",
    "judges",
    "judge",
    "answers",
    "makes sense",
    "helpful",
    "convincing",
    "trust",
    "natural",
    "prefer",
    "feels",
    "readable",
    "looks right",
];

/// Is this test a judgement test? Its stored kind is `manual` (FTR-039 Q2).
pub fn is_judgement(t: &TestRef) -> bool {
    t.kind.trim().eq_ignore_ascii_case("manual")
}

/// What breaks the judgement rule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Finding {
    /// More than one judgement test on one item.
    TooMany { tests: Vec<String> },
    /// A judgement test whose name does not start with [`PREFIX`].
    NoPrefix { test: String },
    /// A judgement test naming something a run can observe, with the kind to use instead.
    Observable {
        test: String,
        word: String,
        kind: String,
    },
    /// A judgement test that does not say what the person judges.
    NoJudgement { test: String },
}

impl Finding {
    /// The finding in a sentence, naming the way out.
    pub fn message(&self) -> String {
        match self {
            Self::TooMany { tests } => {
                let names: Vec<String> = tests.iter().map(|t| format!("'{t}'")).collect();
                format!(
                    "{} manual tests ({}): an item has at most one judgement test",
                    tests.len(),
                    names.join(", ")
                )
            }
            Self::NoPrefix { test } => format!(
                "'{test}' is a manual test, so its name must start with '{PREFIX}' and say what the person judges"
            ),
            Self::Observable { test, word, kind } => format!(
                "'{test}' names '{word}', which a run can check: make it {}",
                kind_phrase(kind)
            ),
            Self::NoJudgement { test } => format!(
                "'{test}' does not say what the person judges (its wording, whether it reads clearly, is usable or answers its question): say it, or make it a unit, integration or walkthrough test"
            ),
        }
    }
}

/// "an integration test", "a unit or integration test", "a walkthrough test", or a kind that
/// already reads as a phrase ("a web check or walkthrough").
fn kind_phrase(kind: &str) -> String {
    if kind.starts_with("a ") || kind.starts_with("an ") {
        kind.to_string()
    } else if kind.starts_with(['a', 'e', 'i', 'o']) {
        format!("an {kind} test")
    } else {
        format!("a {kind} test")
    }
}

/// A character that is part of a word: alphanumeric, or `-` (L-23).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-'
}

/// Does `text` hold `phrase` as whole words? Both are lower case.
fn holds(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + phrase.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

/// Check one judgement test's name, in order: the prefix, observable wording, what is judged.
pub fn assess(name: &str) -> Option<Finding> {
    let trimmed = name.trim();
    let prefixed = trimmed
        .get(..PREFIX.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX));
    if !prefixed {
        return Some(Finding::NoPrefix {
            test: name.to_string(),
        });
    }
    let text = trimmed[PREFIX.len()..].to_lowercase();
    if let Some((word, kind)) = OBSERVABLE.iter().find(|(w, _)| holds(&text, w)) {
        return Some(Finding::Observable {
            test: name.to_string(),
            word: word.to_string(),
            kind: kind.to_string(),
        });
    }
    if !JUDGED.iter().any(|w| holds(&text, w)) {
        return Some(Finding::NoJudgement {
            test: name.to_string(),
        });
    }
    None
}

/// The judgement tests of a definition, in order.
fn judgement_tests(def: &FeatureDefinition) -> impl Iterator<Item = &TestRef> {
    def.requirements
        .iter()
        .flat_map(|r| r.tests.iter())
        .filter(|t| is_judgement(t))
}

/// Everything in `def` that breaks the rule: [`Finding::TooMany`] first when it has more than one
/// judgement test, then each judgement test's [`assess`] finding.
pub fn findings(def: &FeatureDefinition) -> Vec<Finding> {
    let tests: Vec<&TestRef> = judgement_tests(def).collect();
    let mut out = Vec::new();
    if tests.len() > 1 {
        out.push(Finding::TooMany {
            tests: tests.iter().map(|t| t.name.clone()).collect(),
        });
    }
    out.extend(tests.iter().filter_map(|t| assess(&t.name)));
    out
}

/// Refuse a definition write that adds a judgement test breaking the rule. A test is new when no
/// judgement test anywhere in `previous` has its name, which covers a renamed test and one whose
/// kind became manual. With none new the write passes: tests already there are left until touched.
pub fn check_write(previous: Option<&FeatureDefinition>, def: &FeatureDefinition) -> Result<()> {
    let known: Vec<&str> = previous
        .map(|p| judgement_tests(p).map(|t| t.name.as_str()).collect())
        .unwrap_or_default();
    let all: Vec<&TestRef> = judgement_tests(def).collect();
    let new: Vec<&TestRef> = all
        .iter()
        .copied()
        .filter(|t| !known.contains(&t.name.as_str()))
        .collect();
    if new.is_empty() {
        return Ok(());
    }
    if all.len() > 1 {
        let finding = Finding::TooMany {
            tests: all.iter().map(|t| t.name.clone()).collect(),
        };
        return Err(CoreError::Unsupported(format!(
            "{}; keep one judgement test; make the others unit, integration or walkthrough tests",
            finding.message()
        )));
    }
    let messages: Vec<String> = new
        .iter()
        .filter_map(|t| assess(&t.name))
        .map(|f| f.message())
        .collect();
    if messages.is_empty() {
        Ok(())
    } else {
        Err(CoreError::Unsupported(messages.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Requirement;

    fn test(name: &str, kind: &str) -> TestRef {
        TestRef {
            name: name.into(),
            kind: kind.into(),
            ..Default::default()
        }
    }

    fn def(tests: &[TestRef]) -> FeatureDefinition {
        FeatureDefinition {
            requirements: tests
                .iter()
                .enumerate()
                .map(|(i, t)| Requirement {
                    id: format!("R-{}", i + 1),
                    text: "THE SYSTEM SHALL do it".into(),
                    tests: vec![t.clone()],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    const GOOD: &str =
        "manual: the person judges whether the Releases page reads clearly to a newcomer";
    const OTHER: &str = "manual: the wording of the release notes makes sense to a user";

    #[test]
    fn a_second_manual_test_is_refused() {
        let previous = def(&[test(GOOD, "manual"), test("core::tests::it", "unit")]);
        let next = def(&[test(GOOD, "manual"), test(OTHER, "manual")]);
        let err = check_write(Some(&previous), &next).unwrap_err().to_string();
        assert!(err.contains(GOOD) && err.contains(OTHER), "{err}");
        assert!(err.contains("keep one judgement test"), "{err}");
        // With no previous definition at all, both are new.
        assert!(check_write(None, &next).is_err());
        // A test whose kind became manual is new too.
        let previous = def(&[test(GOOD, "manual"), test(OTHER, "unit")]);
        assert!(check_write(Some(&previous), &next).is_err());
        // One judgement test is fine.
        assert!(check_write(None, &def(&[test(GOOD, "manual")])).is_ok());
    }

    #[test]
    fn an_item_that_already_had_two_keeps_them_until_one_is_added_or_renamed() {
        let two = def(&[
            test("release notes exist", "manual"),
            test("manual: sha256sum -c passes", "manual"),
        ]);
        // Rewritten with both unchanged: passes, findings and all.
        assert!(check_write(Some(&two), &two).is_ok());
        // Other edits leave them alone too.
        let mut edited = two.clone();
        edited.statement = "a new statement".into();
        edited.requirements[0]
            .tests
            .push(test("core::tests::new", "unit"));
        assert!(check_write(Some(&two), &edited).is_ok());
        // Renaming one of them is refused.
        let renamed = def(&[test("release notes exist", "manual"), test(GOOD, "manual")]);
        let err = check_write(Some(&two), &renamed).unwrap_err().to_string();
        assert!(err.contains("at most one judgement test"), "{err}");
        // Adding a third is refused.
        let mut added = two.clone();
        added.requirements[0].tests.push(test(GOOD, "Manual"));
        assert!(check_write(Some(&two), &added).is_err());
    }

    #[test]
    fn observable_wording_is_refused_with_the_kind_to_use() {
        let name = "manual: sha256sum -c passes over the manifest";
        assert_eq!(
            assess(name),
            Some(Finding::Observable {
                test: name.into(),
                word: "sha256sum".into(),
                kind: "integration".into(),
            })
        );
        assert_eq!(
            assess(name).unwrap().message(),
            "'manual: sha256sum -c passes over the manifest' names 'sha256sum', which a run can check: make it an integration test"
        );
        match assess("manual: the tag is pushed") {
            Some(Finding::Observable { kind, .. }) => assert_eq!(kind, "walkthrough"),
            other => panic!("{other:?}"),
        }
        match assess("Manual: the button reads clearly") {
            Some(Finding::Observable { word, kind, .. }) => {
                assert_eq!(word, "button");
                assert_eq!(kind, "a web check or walkthrough");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            assess("manual: the command prints the plan")
                .unwrap()
                .message()
                .ends_with("make it a unit or integration test")
        );
        // A refused write says the same.
        let err = check_write(None, &def(&[test("manual: the tag is pushed", "manual")]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("make it a walkthrough test"), "{err}");
    }

    #[test]
    fn a_manual_test_must_say_what_the_person_judges() {
        assert_eq!(
            assess("release notes exist"),
            Some(Finding::NoPrefix {
                test: "release notes exist".into()
            })
        );
        assert_eq!(
            assess("manual: the page is fine"),
            Some(Finding::NoJudgement {
                test: "manual: the page is fine".into()
            })
        );
        let err = check_write(None, &def(&[test("manual: the page is fine", "manual")]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not say what the person judges"), "{err}");
        // Whole words only: "readme" is not "read", "pushedness" is not "pushed".
        assert!(matches!(
            assess("manual: the readme is fine"),
            Some(Finding::NoJudgement { .. })
        ));
    }

    #[test]
    fn a_test_that_says_what_a_person_judges_is_accepted() {
        assert_eq!(assess(GOOD), None);
        assert_eq!(
            assess(&format!("  {}  ", GOOD.replace("manual:", "MANUAL:"))),
            None
        );
        assert_eq!(assess(OTHER), None);
        // Whole words only: "pushedness" is not "pushed".
        assert_eq!(
            assess("manual: the pushedness of the plan reads clearly"),
            None
        );
        assert!(check_write(None, &def(&[test(GOOD, "manual")])).is_ok());
        // A test that is not manual is never assessed.
        assert!(check_write(None, &def(&[test("the tag is pushed", "walkthrough")])).is_ok());
        assert!(is_judgement(&test(GOOD, " Manual ")));
        assert!(!is_judgement(&test(GOOD, "unit")));
    }

    #[test]
    fn findings_name_an_item_with_two_manual_tests_or_observable_wording() {
        let clean = def(&[test(GOOD, "manual"), test("core::tests::it", "unit")]);
        assert!(findings(&clean).is_empty());
        let two = def(&[
            test(GOOD, "manual"),
            test("manual: sha256sum -c passes", "manual"),
        ]);
        let found = findings(&two);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(
            found[0],
            Finding::TooMany {
                tests: vec![GOOD.into(), "manual: sha256sum -c passes".into()]
            }
        );
        assert!(matches!(&found[1], Finding::Observable { word, .. } if word == "sha256sum"));
        let one = def(&[test("manual: the checksum matches", "manual")]);
        assert!(matches!(
            findings(&one).as_slice(),
            [Finding::Observable { word, .. }] if word == "checksum"
        ));
    }
}
