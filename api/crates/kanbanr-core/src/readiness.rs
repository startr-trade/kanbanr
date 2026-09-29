//! One answer to "what is this item missing?" (FEAT-112).
//!
//! The question used to be answered five times — `kanbanr check`, `doctor`, `check --file`,
//! `query --gap` and the monitor's cards each carried their own copy of the rules — and the copies
//! drifted: doctor never asked for a green test, check never looked at quality requirements, the
//! query counted a ratified item as unapproved. Here the rules live once, as a closed vocabulary of
//! [`Check`]s, and every surface asks for the checks it wants. Which checks a surface asks for can
//! differ; what a check *means* cannot.
//!
//! A gap carries a short `label` (for a one-line list) and a `message` that says what to do. The
//! wording is part of the rule, so it lives here too.

use crate::models::{
    ApprovalState, FeatureDefinition, FeatureItem, Requirement, RequirementKind, TestState,
};
use serde::{Deserialize, Serialize};

/// A condition an item can be checked against. The vocabulary is closed on purpose: a workflow can
/// only require what kanbanr knows how to evaluate from the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// The item has a definition at all.
    Definition,
    /// A one-sentence statement: what, for whom, why.
    Statement,
    /// All six Zachman dimensions answered.
    Zachman,
    /// Linked to at least one charter goal.
    Goals,
    /// Every linked goal exists in the charter (needs the charter; an error, not a warning).
    GoalsKnown,
    /// Agreement is current: approved, or ratified after the fact.
    Approved,
    /// A recorded bypass has been answered (approved or ratified).
    Bypass,
    /// At least one requirement.
    Requirements,
    /// Every requirement is in EARS form.
    Ears,
    /// Every requirement names a test.
    TestsNamed,
    /// Every requirement has a green test.
    TestsGreen,
    /// Quality requirements carry a valid ISO 25010 tag and a scenario whose measure names its test.
    Quality,
    /// INVEST "Small": not estimated above three days.
    Small,
    /// Estimated in the project's unit — story points or days (FEAT-121). A Definition of Ready
    /// usually asks for it.
    Estimated,
    /// A named sign-off is recorded against the current definition. Asked for through a gate's
    /// `signoffs: [name]`, never listed on its own — it needs the name.
    Signoff,
}

impl Check {
    /// Checks answered once per requirement, reported in requirement order.
    fn per_requirement(self) -> bool {
        matches!(
            self,
            Check::Ears | Check::TestsNamed | Check::TestsGreen | Check::Quality
        )
    }
}

/// One condition a gate can name: a check, or the Zachman check narrowed to some columns
/// (`{zachman: [what, who, why]}`), which is how a process asks for the six dimensions a stage at a
/// time rather than all at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    Check(Check),
    Zachman { zachman: Vec<String> },
}

impl Condition {
    /// A column list names only real Zachman columns (checked when a workflow is saved).
    pub fn invalid_column(&self) -> Option<String> {
        const COLUMNS: [&str; 6] = ["what", "how", "where", "when", "who", "why"];
        match self {
            Condition::Zachman { zachman } => zachman
                .iter()
                .find(|c| !COLUMNS.contains(&c.trim().to_ascii_lowercase().as_str()))
                .cloned(),
            Condition::Check(_) => None,
        }
    }
}

/// The sign-offs among `names` that are not recorded against the definition as it stands.
pub fn signoff_gaps(feature: &FeatureItem, names: &[String]) -> Vec<Gap> {
    let def = feature.definition.as_ref();
    names
        .iter()
        .filter_map(|name| {
            let latest = def.and_then(|d| d.signoffs.iter().rev().find(|s| &s.id == name));
            let current = def.and_then(|d| d.signoff_current(name));
            match (latest, current) {
                (_, Some(_)) => None,
                (Some(_), None) => Some(gap(
                    Check::Signoff,
                    None,
                    &format!("sign-off '{name}' lapsed"),
                    format!(
                        "sign-off '{name}' lapsed — the definition changed after it was given, so \
                         it no longer covers what is proposed"
                    ),
                )),
                (None, None) => Some(gap(
                    Check::Signoff,
                    None,
                    &format!("no sign-off '{name}'"),
                    format!(
                        "no sign-off '{name}' — `kanbanr signoff {} {name}`",
                        feature.code
                    ),
                )),
            }
        })
        .collect()
}

/// The gaps of an item against a gate's conditions.
pub fn evaluate_conditions(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    conditions: &[Condition],
    unit: crate::config::EstimateUnit,
) -> Vec<Gap> {
    let checks: Vec<Check> = conditions
        .iter()
        .filter_map(|c| match c {
            Condition::Check(c) => Some(*c),
            Condition::Zachman { .. } => None,
        })
        .collect();
    let mut gaps = evaluate_in_unit(feature, goal_ids, &checks, unit);
    let columns: Vec<String> = conditions
        .iter()
        .flat_map(|c| match c {
            Condition::Zachman { zachman } => zachman.clone(),
            Condition::Check(_) => vec![],
        })
        .map(|c| c.trim().to_ascii_lowercase())
        .collect();
    if !columns.is_empty()
        && let Some(def) = feature.definition.as_ref()
        && def.exempt.trim().is_empty()
    {
        for column in def.zachman.missing() {
            if columns.contains(&column.to_ascii_lowercase())
                && !gaps
                    .iter()
                    .any(|g| g.label == format!("[MISSING: {column}]"))
            {
                let label = format!("[MISSING: {column}]");
                gaps.push(gap(Check::Zachman, None, &label, label.clone()));
            }
        }
    }
    gaps
}

/// What moving an item on to one next status would ask, and what it still lacks (FEAT-117).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextGate {
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    pub enforce: crate::config::Enforce,
    /// What `requires` and `signoffs` still lack.
    pub gaps: Vec<Gap>,
    /// What `warns` lacks — said, never enforced.
    pub warnings: Vec<Gap>,
    /// The sign-offs this gate still needs, by name, so a surface can offer to record them.
    pub signoffs_needed: Vec<String>,
}

/// The gates an item meets next: the statuses it can move on to in one step — forward in the
/// workflow's own order, never a parking or no-op status — that have a gate. This is the answer to
/// "what does the next stage need?", which a definition built stage by stage asks at every step.
pub fn next_gates(
    project: &crate::store::Project,
    charter: Option<&crate::Charter>,
    feature: &FeatureItem,
) -> Vec<NextGate> {
    let config = &project.config;
    let gates = config.effective_gates();
    let position = |s: &str| config.statuses.iter().position(|x| x == s);
    let here = position(&feature.status);
    let goal_ids = charter.map(|c| c.goal_ids());
    config
        .transitions
        .get(&feature.status)
        .into_iter()
        .flatten()
        .filter(|to| !config.is_no_op(to))
        .filter(|to| match (here, position(to)) {
            (Some(h), Some(t)) => t > h,
            _ => false,
        })
        .filter_map(|to| {
            let gate = gates.get(to)?;
            if !gate.applies_to(feature.kind.as_deref()) {
                return None;
            }
            let unit = config.estimate_unit;
            let mut gaps = evaluate_conditions(feature, goal_ids.as_ref(), &gate.requires, unit);
            let signoffs = signoff_gaps(feature, &gate.signoffs);
            let signoffs_needed = gate
                .signoffs
                .iter()
                .filter(|name| {
                    feature
                        .definition
                        .as_ref()
                        .and_then(|d| d.signoff_current(name))
                        .is_none()
                })
                .cloned()
                .collect();
            gaps.extend(signoffs);
            Some(NextGate {
                status: to.clone(),
                purpose: gate.purpose.clone(),
                enforce: gate.enforce,
                gaps,
                warnings: evaluate_conditions(feature, goal_ids.as_ref(), &gate.warns, unit),
                signoffs_needed,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Warning,
    /// A broken reference, not a judgement: someone can fix it by editing a name.
    Error,
}

/// One thing an item is missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub check: Check,
    /// The requirement the gap is about, for per-requirement checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    pub level: Level,
    /// Short form, for a one-line list (`[MISSING: What]`, `no goal link`).
    pub label: String,
    /// What is missing and what to do about it.
    pub message: String,
}

/// What `kanbanr check` and `kanbanr finish` report: the definition, agreement, and evidence.
pub const CHECK: &[Check] = &[
    Check::Definition,
    Check::Statement,
    Check::Zachman,
    Check::Goals,
    Check::Approved,
    Check::Bypass,
    Check::Requirements,
    Check::Ears,
    Check::TestsNamed,
    Check::TestsGreen,
];

/// What `doctor` warns about on live work. Agreement is the review queue's question, and a green
/// test is not expected until the work is done, so neither is here.
pub const DOCTOR: &[Check] = &[
    Check::Definition,
    Check::Statement,
    Check::Zachman,
    Check::Goals,
    Check::GoalsKnown,
    Check::Bypass,
    Check::Requirements,
    Check::Ears,
    Check::TestsNamed,
    Check::Quality,
    Check::Small,
];

/// What `check --file` holds a contribution to. There is no board, so nothing about agreement or
/// goal ids can be judged.
pub const FILE: &[Check] = &[
    Check::Statement,
    Check::Zachman,
    Check::Requirements,
    Check::Ears,
    Check::TestsNamed,
    Check::TestsGreen,
    Check::Quality,
];

/// What a board card counts: whether the item has said what it is and how it will be verified.
pub const CARD: &[Check] = &[
    Check::Definition,
    Check::Statement,
    Check::Zachman,
    Check::Goals,
    Check::Requirements,
    Check::TestsNamed,
];

/// The groups `query --gap` filters by.
pub fn query_group(name: &str) -> Option<&'static [Check]> {
    match name.trim().to_ascii_lowercase().as_str() {
        "why" => Some(&[
            Check::Definition,
            Check::Statement,
            Check::Zachman,
            Check::Goals,
        ]),
        "test" => Some(&[
            Check::Definition,
            Check::Requirements,
            Check::TestsNamed,
            Check::TestsGreen,
        ]),
        "approval" => Some(&[Check::Definition, Check::Approved, Check::Bypass]),
        _ => None,
    }
}

/// The gaps of a board item against `checks`. `goal_ids` is the charter's goal ids, needed only
/// for [`Check::GoalsKnown`].
pub fn evaluate(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    checks: &[Check],
) -> Vec<Gap> {
    evaluate_in_unit(feature, goal_ids, checks, crate::config::EstimateUnit::Days)
}

/// As [`evaluate`], judging [`Check::Estimated`] in the project's unit.
pub fn evaluate_in_unit(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    checks: &[Check],
    unit: crate::config::EstimateUnit,
) -> Vec<Gap> {
    let mut gaps = evaluate_definition(
        feature.definition.as_ref(),
        feature.estimate_days,
        goal_ids,
        checks,
    );
    let exempt = feature
        .definition
        .as_ref()
        .is_some_and(|d| !d.exempt.trim().is_empty());
    if checks.contains(&Check::Estimated) && !exempt {
        let (has, what) = match unit {
            crate::config::EstimateUnit::Points => (feature.points.is_some(), "story points"),
            crate::config::EstimateUnit::Days => (feature.estimate_days.is_some(), "days"),
        };
        if !has {
            gaps.push(gap(
                Check::Estimated,
                None,
                "not estimated",
                format!("not estimated — this project estimates in {what}"),
            ));
        }
    }
    gaps
}

/// The gaps of a definition on its own, as `check --file` has it.
///
/// An exempt definition has no gaps: the recorded reason is the answer. No definition at all is
/// one gap (when asked about), and nothing else can be judged.
pub fn evaluate_definition(
    definition: Option<&FeatureDefinition>,
    estimate_days: Option<f64>,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    checks: &[Check],
) -> Vec<Gap> {
    let mut gaps = Vec::new();
    let Some(def) = definition else {
        if checks.contains(&Check::Definition) {
            gaps.push(gap(
                Check::Definition,
                None,
                "no definition",
                "no definition — why it exists, what must be true, how it is verified \
                 (`kanbanr feature define`)"
                    .into(),
            ));
        }
        return gaps;
    };
    if !def.exempt.trim().is_empty() {
        return gaps;
    }
    for check in checks
        .iter()
        .filter(|c| !c.per_requirement() && **c != Check::Small)
    {
        item_level(*check, def, goal_ids, &mut gaps);
    }
    let per_requirement: Vec<Check> = checks
        .iter()
        .copied()
        .filter(|c| c.per_requirement())
        .collect();
    for r in &def.requirements {
        for check in &per_requirement {
            requirement_level(*check, r, &mut gaps);
        }
    }
    if checks.contains(&Check::Small)
        && let Some(days) = estimate_days.filter(|d| *d > 3.0)
    {
        gaps.push(gap(
            Check::Small,
            None,
            "too big",
            format!("estimated at {days} days — slice it smaller (INVEST: Small)"),
        ));
    }
    gaps
}

fn gap(check: Check, requirement: Option<&str>, label: &str, message: String) -> Gap {
    Gap {
        check,
        requirement: requirement.map(str::to_string),
        level: Level::Warning,
        label: label.to_string(),
        message,
    }
}

fn item_level(
    check: Check,
    def: &FeatureDefinition,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    gaps: &mut Vec<Gap>,
) {
    let agreed = matches!(
        def.approval_state(),
        ApprovalState::Current | ApprovalState::Ratified
    );
    match check {
        Check::Definition => {}
        Check::Statement => {
            if def.statement.trim().is_empty() {
                gaps.push(gap(
                    check,
                    None,
                    "[MISSING: statement]",
                    "[MISSING: statement] — one sentence: what this gives whom, and why".into(),
                ));
            }
        }
        Check::Zachman => {
            for column in def.zachman.missing() {
                let label = format!("[MISSING: {column}]");
                gaps.push(gap(check, None, &label, label.clone()));
            }
        }
        Check::Goals => {
            if def.goals.is_empty() {
                gaps.push(gap(
                    check,
                    None,
                    "no goal link",
                    "no goal link — nothing says what this is for".into(),
                ));
            }
        }
        Check::GoalsKnown => {
            let Some(known) = goal_ids else { return };
            for goal in def.goals.iter().filter(|g| !known.contains(g.as_str())) {
                let mut g = gap(
                    check,
                    None,
                    "unknown goal",
                    format!("links goal '{goal}', which is not in the project charter"),
                );
                g.level = Level::Error;
                gaps.push(g);
            }
        }
        Check::Approved => match def.approval_state() {
            ApprovalState::Current | ApprovalState::Ratified => {}
            ApprovalState::Missing => {
                gaps.push(gap(check, None, "not approved", "not approved".into()))
            }
            ApprovalState::Lapsed => gaps.push(gap(
                check,
                None,
                "approval lapsed",
                "approval lapsed — the definition changed after it was approved".into(),
            )),
        },
        // Reported only while unanswered: agreeing to the definition is what it asks for, whether
        // the agreement came before the work or after it. FEAT-068, FEAT-080.
        Check::Bypass => {
            if !def.started_unapproved.trim().is_empty() && !agreed {
                gaps.push(gap(
                    check,
                    None,
                    "started without approval",
                    format!(
                        "started without approval: {} — review and approve what was actually built",
                        def.started_unapproved.trim()
                    ),
                ));
            }
        }
        Check::Requirements => {
            if def.requirements.is_empty() {
                gaps.push(gap(
                    check,
                    None,
                    "no requirements",
                    "no requirements — nothing states what must be true for this to be done".into(),
                ));
            }
        }
        Check::Ears
        | Check::TestsNamed
        | Check::TestsGreen
        | Check::Quality
        | Check::Small
        | Check::Estimated
        | Check::Signoff => {}
    }
}

/// A requirement's name for a message: its id, or the start of its text when it has none yet (a
/// contribution checked from a file may not have been numbered).
fn requirement_name(r: &Requirement) -> String {
    if r.id.trim().is_empty() {
        format!("\"{}\"", r.text.chars().take(40).collect::<String>())
    } else {
        r.id.clone()
    }
}

fn requirement_level(check: Check, r: &Requirement, gaps: &mut Vec<Gap>) {
    let id = requirement_name(r);
    let mut push = |label: &str, message: String| {
        gaps.push(gap(check, Some(&id), label, message));
    };
    match check {
        Check::Ears => {
            if crate::ears::classify(&r.text).is_none() {
                push(
                    "not EARS",
                    format!(
                        "{id} is not in EARS form (THE SYSTEM SHALL … / WHEN … / WHILE … / WHERE … / IF …)"
                    ),
                );
            }
        }
        Check::TestsNamed => {
            if r.tests.is_empty() {
                push(
                    "no test",
                    format!("{id} has no test — it cannot be shown to be met"),
                );
            }
        }
        Check::TestsGreen => {
            if !r.tests.is_empty() && !r.tests.iter().any(|t| t.state == TestState::Green) {
                push(
                    "not proven",
                    format!("{id} has tests but none is green — not yet proven"),
                );
            }
        }
        // The unsupported-claim checks matter most: a quality tag or a measured number that
        // nothing verifies reads as rigour while being decoration.
        Check::Quality => {
            for tag in &r.iso {
                if crate::ears::normalize_iso(tag).is_none() {
                    push(
                        "unknown quality",
                        format!(
                            "{id} is tagged '{tag}', which is not an ISO/IEC 25010 characteristic"
                        ),
                    );
                }
            }
            if matches!(r.kind, RequirementKind::Nfr) {
                if r.iso.is_empty() {
                    push(
                        "no quality tag",
                        format!("{id} is a quality requirement with no ISO 25010 tag"),
                    );
                }
                match r.scenario.as_ref() {
                    None => push(
                        "no scenario",
                        format!(
                            "{id} is a quality requirement with no scenario (stimulus, environment, response, measure)"
                        ),
                    ),
                    Some(s) if s.measure.trim().is_empty() => push(
                        "no measure",
                        format!(
                            "{id} has a quality scenario with no measure — an unmeasured quality is an opinion"
                        ),
                    ),
                    Some(s) => {
                        let measure = s.measure.to_lowercase();
                        let named = r.tests.iter().any(|t| {
                            !t.name.trim().is_empty() && measure.contains(&t.name.to_lowercase())
                        });
                        if !named {
                            push(
                                "unsupported claim",
                                format!(
                                    "{id}: the measure names no test or benchmark that checks it (unsupported claim)"
                                ),
                            );
                        }
                    }
                }
            } else if !r.iso.is_empty() {
                push(
                    "misplaced quality tag",
                    format!(
                        "{id} carries a quality tag but is not a quality requirement (kind: nfr)"
                    ),
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Requirement, TestRef};

    fn requirement(id: &str, text: &str, tests: &[(&str, TestState)]) -> Requirement {
        Requirement {
            id: id.into(),
            text: text.into(),
            tests: tests
                .iter()
                .map(|(n, s)| TestRef {
                    name: (*n).into(),
                    state: *s,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn item(def: Option<FeatureDefinition>) -> FeatureItem {
        let mut f: FeatureItem = serde_json::from_value(serde_json::json!({
            "code": "FEAT-001", "title": "t", "status": "Planned", "milestone": "MS-001",
            "created_at": "2026-09-29T00:00:00Z", "updated_at": "2026-09-29T00:00:00Z"
        }))
        .expect("a minimal item");
        f.definition = def;
        f
    }

    /// FEAT-112 R-2: with no gates, each surface's list reproduces what that surface reported
    /// before the engine existed — the same gaps, in the same order.
    #[test]
    fn readiness_golden_matches_legacy_reports() {
        let def = FeatureDefinition {
            statement: String::new(),
            goals: vec![],
            requirements: vec![
                requirement("R-1", "keep carts", &[("t1", TestState::Red)]),
                requirement("R-2", "WHEN x, THE SYSTEM SHALL y.", &[]),
            ],
            started_unapproved: "urgent".into(),
            ..Default::default()
        };
        let f = item(Some(def));
        let labels = |checks: &[Check]| -> Vec<String> {
            evaluate(&f, None, checks)
                .into_iter()
                .map(|g| g.message)
                .collect()
        };
        let check = labels(CHECK);
        assert_eq!(
            check[0],
            "[MISSING: statement] — one sentence: what this gives whom, and why"
        );
        assert!(
            check[1..7].iter().all(|m| m.starts_with("[MISSING: ")),
            "{check:?}"
        );
        assert_eq!(check[7], "no goal link — nothing says what this is for");
        assert_eq!(check[8], "not approved");
        assert!(check[9].starts_with("started without approval: urgent"));
        // Per requirement, in requirement order: R-1 EARS, R-1 not green, R-2 no test.
        assert!(check[10].starts_with("R-1 is not in EARS form"));
        assert!(check[11].starts_with("R-1 has tests but none is green"));
        assert!(check[12].starts_with("R-2 has no test"));
        assert_eq!(check.len(), 13);

        // Doctor never asked about approval or green tests, and still does not.
        let doctor = labels(DOCTOR);
        assert!(!doctor.iter().any(|m| m == "not approved"), "{doctor:?}");
        assert!(
            !doctor.iter().any(|m| m.contains("none is green")),
            "{doctor:?}"
        );
        assert!(doctor.iter().any(|m| m.starts_with("R-2 has no test")));
    }

    /// Exempt is an answer, and a missing definition is one gap, not a cascade.
    #[test]
    fn exempt_and_undefined_items() {
        let exempt = item(Some(FeatureDefinition {
            exempt: "predates the method".into(),
            ..Default::default()
        }));
        assert!(evaluate(&exempt, None, CHECK).is_empty());
        let bare = item(None);
        let gaps = evaluate(&bare, None, CHECK);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].check, Check::Definition);
        assert!(
            evaluate(&bare, None, FILE).is_empty(),
            "FILE never asks about a missing definition"
        );
    }

    /// A ratified item is agreed to: neither the approval nor the bypass check reports it. The
    /// query used to count it as unapproved.
    #[test]
    fn ratified_is_agreed() {
        use crate::models::Approval;
        let mut def = FeatureDefinition {
            statement: "s".into(),
            started_unapproved: "urgent".into(),
            ..Default::default()
        };
        def.approval = Some(Approval {
            by: "A".into(),
            at: "2026-09-29T00:00:00Z".into(),
            rev: def.content_rev(),
        });
        def.approvals.push(crate::models::ApprovalEvent {
            verdict: "ratified".into(),
            by: "A".into(),
            rev: def.content_rev(),
            ..Default::default()
        });
        let f = item(Some(def));
        let approval = query_group("approval").unwrap();
        assert!(evaluate(&f, None, approval).is_empty());
    }

    #[test]
    fn unknown_goals_are_errors() {
        let f = item(Some(FeatureDefinition {
            statement: "s".into(),
            goals: vec!["G-9".into()],
            ..Default::default()
        }));
        let known: std::collections::BTreeSet<String> = ["G-1".to_string()].into();
        let gaps = evaluate(&f, Some(&known), &[Check::GoalsKnown]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].level, Level::Error);
        assert!(
            evaluate(&f, None, &[Check::GoalsKnown]).is_empty(),
            "no charter, no judgement"
        );
    }
}
