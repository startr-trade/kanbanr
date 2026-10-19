//! The method, on by default (FEAT-193): design before build, and living documentation.
//!
//! Stated once, as gate fragments addressed by stage role (FEAT-188), so the same rule lands on
//! whichever stage holds the role in any process:
//! - at the **build** role: `spec_sections: [Design]` in `requires`, `design-review` in `signoffs`;
//! - at the **integrate** role: `docs_updated` in `requires` (FEAT-180), `docs-folded` in
//!   `signoffs`;
//! - everywhere: doctor's broken-reference scan (FEAT-181), the `links` part.
//!
//! The fragments live in two role-addressed gates, `"@build"` and `"@integrate"`. Those add to a
//! stage's own gate rather than replacing the built-in rule, so a process that declares nothing
//! else keeps its start rule. A project chooses which parts are off ([`MethodChoice`]); the
//! built-in processes have them all on.

use crate::config::{Gate, ROLE_KEY_PREFIX, Role, WorkflowFile};
use crate::readiness::{Check, Condition};
use serde::{Deserialize, Serialize};

/// One part of the method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Part {
    /// The spec has a Design section before build.
    Design,
    /// A person signs off the design before build.
    DesignReview,
    /// The docs the item names were updated before it integrates.
    Docs,
    /// A person signs off that the design was folded into the docs.
    DocsFolded,
    /// Doctor reports references to things the board does not have.
    Links,
}

/// The sign-off names the method uses.
pub const DESIGN_REVIEW: &str = "design-review";
pub const DOCS_FOLDED: &str = "docs-folded";
const DESIGN: &str = "Design";

impl Part {
    pub const ALL: [Part; 5] = [
        Part::Design,
        Part::DesignReview,
        Part::Docs,
        Part::DocsFolded,
        Part::Links,
    ];
    /// The parts that are gate fragments: all but `links`.
    pub const GATES: [Part; 4] = [
        Part::Design,
        Part::DesignReview,
        Part::Docs,
        Part::DocsFolded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Part::Design => "design",
            Part::DesignReview => "design-review",
            Part::Docs => "docs",
            Part::DocsFolded => "docs-folded",
            Part::Links => "links",
        }
    }

    /// A part by name, or an error naming the five there are.
    pub fn parse(s: &str) -> crate::Result<Part> {
        let key = s.trim().to_ascii_lowercase();
        Part::ALL
            .into_iter()
            .find(|p| p.as_str() == key)
            .ok_or_else(|| {
                let names: Vec<&str> = Part::ALL.iter().map(|p| p.as_str()).collect();
                crate::CoreError::Unsupported(format!(
                    "'{s}' is not a part of the method — the parts are: {}",
                    names.join(", ")
                ))
            })
    }

    /// The stage role whose gate holds this part; `links` is not a gate.
    pub fn role(self) -> Option<Role> {
        match self {
            Part::Design | Part::DesignReview => Some(Role::Build),
            Part::Docs | Part::DocsFolded => Some(Role::Integrate),
            Part::Links => None,
        }
    }

    /// What this part asks, in words.
    pub fn means(self) -> &'static str {
        match self {
            Part::Design => "requires spec_sections [Design]",
            Part::DesignReview => "needs the design-review sign-off",
            Part::Docs => "requires docs_updated",
            Part::DocsFolded => "needs the docs-folded sign-off",
            Part::Links => "doctor reports broken references",
        }
    }
}

/// What a project chose. Absent on disk means never chosen: everything on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodChoice {
    /// The parts switched off, sorted and unique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<Part>,
    /// When the project last chose (RFC 3339); empty when it never has.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chosen_at: String,
}

impl MethodChoice {
    pub fn is_on(&self, p: Part) -> bool {
        !self.off.contains(&p)
    }

    pub fn is_default(&self) -> bool {
        self.off.is_empty() && self.chosen_at.is_empty()
    }
}

fn role_gate_key(role: Role) -> String {
    format!("{ROLE_KEY_PREFIX}{}", role.name())
}

/// Does a requires list ask for the Design section? Plain `spec_sections` means Design.
fn asks_design(requires: &[Condition]) -> bool {
    requires.iter().any(|c| match c {
        Condition::Check(Check::SpecSections) => true,
        Condition::SpecSections { spec_sections } => spec_sections
            .iter()
            .any(|s| s.trim().eq_ignore_ascii_case(DESIGN)),
        _ => false,
    })
}

/// Merge the parts' fragments into the role-addressed gates. Idempotent: a condition or sign-off
/// already there is not added twice. One note per part that could not be placed, because no status
/// holds its role.
pub fn add(wf: &mut WorkflowFile, parts: &[Part]) -> Vec<String> {
    let roles = wf.clone().into_config("method").roles();
    let mut notes = Vec::new();
    for &part in parts {
        let Some(role) = part.role() else { continue };
        if roles.status(role).is_none() {
            notes.push(format!(
                "no status holds the {} role: {} not added",
                role.name(),
                part.as_str()
            ));
            continue;
        }
        let gate = wf.gates.entry(role_gate_key(role)).or_default();
        match part {
            Part::Design => {
                if !asks_design(&gate.requires) {
                    gate.requires.push(Condition::SpecSections {
                        spec_sections: vec![DESIGN.to_string()],
                    });
                }
            }
            Part::Docs => {
                let c = Condition::Check(Check::DocsUpdated);
                if !gate.requires.contains(&c) {
                    gate.requires.push(c);
                }
            }
            Part::DesignReview | Part::DocsFolded => {
                let name = if part == Part::DesignReview {
                    DESIGN_REVIEW
                } else {
                    DOCS_FOLDED
                };
                if !gate.signoffs.iter().any(|s| s == name) {
                    gate.signoffs.push(name.to_string());
                }
            }
            Part::Links => {}
        }
    }
    notes
}

/// Take the parts' fragments out of the role-addressed gates — never out of a stage's own gate,
/// which is the project's rule, not the method's. A gate left with nothing in it is dropped.
pub fn remove(wf: &mut WorkflowFile, parts: &[Part]) {
    for &part in parts {
        let Some(role) = part.role() else { continue };
        let key = role_gate_key(role);
        let Some(gate) = wf.gates.get_mut(&key) else {
            continue;
        };
        match part {
            Part::Design => {
                gate.requires.retain_mut(|c| match c {
                    Condition::Check(Check::SpecSections) => false,
                    Condition::SpecSections { spec_sections } => {
                        spec_sections.retain(|s| !s.trim().eq_ignore_ascii_case(DESIGN));
                        !spec_sections.is_empty()
                    }
                    _ => true,
                });
            }
            Part::Docs => gate
                .requires
                .retain(|c| *c != Condition::Check(Check::DocsUpdated)),
            Part::DesignReview => gate.signoffs.retain(|s| s != DESIGN_REVIEW),
            Part::DocsFolded => gate.signoffs.retain(|s| s != DOCS_FOLDED),
            Part::Links => {}
        }
        if *gate == Gate::default() {
            wf.gates.remove(&key);
        }
    }
}

/// `wf` with every part the choice has off taken out.
pub fn shaped(mut wf: WorkflowFile, choice: &MethodChoice) -> WorkflowFile {
    remove(&mut wf, &choice.off);
    wf
}

/// The status where `p` is in force in `wf`, or None. `links` is never in a gate.
pub fn present(wf: &WorkflowFile, p: Part) -> Option<String> {
    let role = p.role()?;
    let gate = wf.gates.get(&role_gate_key(role))?;
    let here = match p {
        Part::Design => asks_design(&gate.requires),
        Part::Docs => gate
            .requires
            .contains(&Condition::Check(Check::DocsUpdated)),
        Part::DesignReview => gate.signoffs.iter().any(|s| s == DESIGN_REVIEW),
        Part::DocsFolded => gate.signoffs.iter().any(|s| s == DOCS_FOLDED),
        Part::Links => false,
    };
    if !here {
        return None;
    }
    wf.clone().into_config("method").role_status(role)
}

/// One part as a project has it: chosen on or off, and where it is in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartStatus {
    pub part: Part,
    pub on: bool,
    /// The stage it is in force at; None when it is not in this workflow (or is `links`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub means: String,
}

/// Every part, as the project has it — what `kanbanr config method` shows.
pub fn status(config: &crate::config::ProjectConfig) -> Vec<PartStatus> {
    let wf = WorkflowFile::from_config(config);
    Part::ALL
        .into_iter()
        .map(|part| PartStatus {
            part,
            on: config.method.is_on(part),
            status: present(&wf, part),
            means: part.means().to_string(),
        })
        .collect()
}

/// The parts named in a request: all five when none are named.
pub fn parse_parts(names: &[String]) -> crate::Result<Vec<Part>> {
    if names.is_empty() {
        return Ok(Part::ALL.to_vec());
    }
    names.iter().map(|n| Part::parse(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_is_idempotent() {
        let mut once = crate::config::preset_bare("togaf").unwrap();
        assert!(add(&mut once, &Part::GATES).is_empty());
        let mut twice = once.clone();
        assert!(add(&mut twice, &Part::GATES).is_empty());
        assert_eq!(once, twice);
        for p in Part::GATES {
            assert!(present(&once, p).is_some(), "{p:?}");
        }
        assert_eq!(
            present(&once, Part::Design).as_deref(),
            Some("Implementation")
        );
        assert_eq!(present(&once, Part::Docs).as_deref(), Some("Migration"));
    }

    /// Taking every part out of a built-in process gives back the process as written: the default
    /// process's built-in rule is untouched, and it is the same board as before the method.
    #[test]
    fn remove_all_restores_the_bare_preset() {
        for (name, _) in crate::config::presets() {
            let bare = crate::config::preset_bare(name).unwrap();
            let mut with = crate::config::preset(name).unwrap();
            assert_ne!(with, bare, "{name} carries the method");
            remove(&mut with, &Part::ALL);
            assert_eq!(with, bare, "{name}");
        }
        let default = crate::config::preset_bare("default")
            .unwrap()
            .into_config("p");
        assert_eq!(default.required_schema_version(), 4);
    }

    #[test]
    fn missing_role_adds_nothing_and_says_so() {
        let mut wf = crate::config::preset_bare("togaf").unwrap();
        for gate in wf.gates.values_mut() {
            gate.roles.retain(|r| r != "integrate");
        }
        let before = wf.clone();
        let notes = add(&mut wf, &[Part::Docs, Part::DocsFolded]);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].contains("no status holds the integrate role"),
            "{notes:?}"
        );
        assert_eq!(wf, before);
    }

    #[test]
    fn parts_parse_by_name() {
        assert_eq!(Part::parse("docs-folded").unwrap(), Part::DocsFolded);
        let err = Part::parse("nope").unwrap_err().to_string();
        assert!(
            err.contains("design, design-review, docs, docs-folded, links"),
            "{err}"
        );
    }
}
