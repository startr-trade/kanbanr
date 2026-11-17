//! What needs the person (FTR-048): the lines behind the Needs-you badge and drawer, and the
//! Overview's action cards.
//!
//! Derived from the board on each read and never stored (ADR-0002, ADR-0005). The Decisions lines
//! are [`crate::reviewgroups::summary`]'s groups, mapped generically by key and label so the badge
//! and the Review page's tabs cannot disagree; this module holds no list of group keys. Only the
//! person's own decisions (review groups and releases at a stage) are counted in
//! [`Attention::total`]; health and records are Claude's to fix, so they are shown, not counted.
//!
//! A board read error fails the whole read. Every other source fails soft: its line is left out
//! and a Health line says what could not be read, so nothing fails silently.

use crate::Store;
use crate::error::Result;
use crate::releases::ReleaseState;
use crate::report::item_evidence;
use serde::Serialize;
use std::path::Path;

/// Everything waiting, and how many of it are the person's own decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Attention {
    /// The sum of the counts of the lines that are the person's (`yours`).
    pub total: usize,
    /// In section order: decisions, releases, health, records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<AttentionLine>,
    /// What Claude holds and decided under a delegation, while one is in force or has just ended
    /// (DEF-104). Absent in `monitor` mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated: Option<DelegatedAttention>,
}

/// A delegation ends "soon" within this many hours.
pub const ENDING_SOON_HOURS: i64 = 72;
/// The most delegated decisions an answer lists; `decided_total` says how many there are.
pub const DECIDED_MAX: usize = 50;

/// Needs you in delegated (autonomous) mode: what Claude decided and holds (DEF-104).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DelegatedAttention {
    /// The delegations in force, in file order.
    pub in_force: Vec<String>,
    /// The verdicts Claude recorded that the person has not yet kept or undone, newest first, at
    /// most [`DECIDED_MAX`].
    pub decided: Vec<crate::decisions::DelegatedDecision>,
    pub decided_total: usize,
    /// The decisions each delegation in force still holds.
    pub holds: Vec<DelegationHold>,
    /// Delegations ending within [`ENDING_SOON_HOURS`], or ended or revoked within them.
    pub ending: Vec<DelegationEnding>,
}

/// What one delegation in force still holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DelegationHold {
    pub delegation: String,
    pub until: String,
    pub total: usize,
    /// In the delegation's own `verdicts` order, zero counts left out.
    pub verdicts: Vec<HeldCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HeldCount {
    /// `approve`, `ratify` or `signoff:<name>`.
    pub verdict: String,
    pub count: usize,
}

/// A delegation that ends soon, or just ended, and how many decisions return to the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DelegationEnding {
    pub delegation: String,
    /// `granted` (ends soon), `ended` or `revoked`.
    pub state: crate::decisions::DelegationState,
    /// When it ends, ended or was revoked (RFC 3339).
    pub at: String,
    pub returns: usize,
}

/// The delegated part of the answer; `None` unless the mode is `delegated` and a delegation is in
/// force or has an ending line (DEF-104).
pub(crate) fn delegated_attention(
    d: &crate::decisions::Decisions,
    project: &crate::Project,
    holds: &crate::reviewgroups::Holds,
    now: &str,
) -> Option<DelegatedAttention> {
    use crate::decisions::DelegationState as St;
    use time::format_description::well_known::Rfc3339;
    if d.mode() != crate::decisions::Mode::Delegated {
        return None;
    }
    let instant = |s: &str| time::OffsetDateTime::parse(s.trim(), &Rfc3339).ok();
    let now_t = instant(now)?;
    let window = time::Duration::hours(ENDING_SOON_HOURS);
    let mut in_force = Vec::new();
    let mut hold_lines = Vec::new();
    let mut ending: Vec<DelegationEnding> = Vec::new();
    for g in &d.delegations {
        let held_by_it: Vec<&crate::reviewgroups::HeldDecision> =
            holds.held.iter().filter(|h| h.delegation == g.id).collect();
        let returned = holds
            .returned
            .iter()
            .find(|(id, _)| *id == g.id)
            .map_or(0, |(_, n)| *n);
        match g.state(now) {
            St::Granted => {
                in_force.push(g.id.clone());
                let verdicts: Vec<HeldCount> = g
                    .verdicts
                    .iter()
                    .map(|v| v.trim())
                    .map(|v| HeldCount {
                        verdict: v.to_string(),
                        count: held_by_it.iter().filter(|h| h.verdict == v).count(),
                    })
                    .filter(|c| c.count > 0)
                    .collect();
                let total: usize = verdicts.iter().map(|c| c.count).sum();
                if total > 0 {
                    hold_lines.push(DelegationHold {
                        delegation: g.id.clone(),
                        until: g.until.clone(),
                        total,
                        verdicts,
                    });
                }
                if instant(&g.until).is_some_and(|u| u - now_t <= window) {
                    ending.push(DelegationEnding {
                        delegation: g.id.clone(),
                        state: St::Granted,
                        at: g.until.clone(),
                        returns: held_by_it.len(),
                    });
                }
            }
            St::Ended => {
                if instant(&g.until).is_some_and(|u| now_t - u <= window) {
                    ending.push(DelegationEnding {
                        delegation: g.id.clone(),
                        state: St::Ended,
                        at: g.until.clone(),
                        returns: returned,
                    });
                }
            }
            St::Revoked => {
                if let Some(r) = g
                    .revoked
                    .as_ref()
                    .filter(|r| instant(&r.at).is_some_and(|t| t <= now_t && now_t - t <= window))
                {
                    ending.push(DelegationEnding {
                        delegation: g.id.clone(),
                        state: St::Revoked,
                        at: r.at.clone(),
                        returns: returned,
                    });
                }
            }
            St::Proposed => {}
        }
    }
    if in_force.is_empty() && ending.is_empty() {
        return None;
    }
    ending.sort_by(|a, b| crate::cmp_instants(&a.at, &b.at));
    let mut decided = crate::decisions::to_review(project, d);
    decided.reverse();
    let decided_total = decided.len();
    decided.truncate(DECIDED_MAX);
    Some(DelegatedAttention {
        in_force,
        decided,
        decided_total,
        holds: hold_lines,
        ending,
    })
}

/// One kind of thing waiting, counted, with where to see it or what to ask Claude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AttentionLine {
    /// A review group's key, as the Review page names it, or `planning`, `release`, `red-tests`,
    /// `doctor-errors`, `git-doctor`, `retro-due`, or `read-error` for a source that could not be
    /// read.
    pub kind: String,
    pub section: Section,
    /// "Approve", "v0.8.0 at candidate", "Red tests", "Doctor errors", "Retro due: SP-003".
    pub label: String,
    pub count: usize,
    /// The monitor page that shows it; absent when no page does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The sentence to ask Claude with, when no page shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
    /// `info`, `warn` or `error`.
    pub tone: String,
    /// The person's own decision, counted in [`Attention::total`].
    pub yours: bool,
}

/// Where a line is shown in the drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Decisions,
    Releases,
    Health,
    Records,
}

const INFO: &str = "info";
const WARN: &str = "warn";
const ERROR: &str = "error";

/// The lines for project `p`. `repo` is the code repository, when the server has one: the review
/// groups that judge it and the git doctor need it, and are left out without it.
pub fn attention(store: &Store, p: &str, repo: Option<&Path>) -> Result<Attention> {
    // The one read that fails the whole answer: an unknown project, a newer board.
    let project = store.load_meta(p)?;

    let mut decisions = Vec::new();
    let mut releases = Vec::new();
    let mut health = Vec::new();
    let mut records = Vec::new();

    let now = crate::now_rfc3339();
    // An unreadable decisions.yaml reads as nothing delegated: every decision is the person's.
    let delegations = crate::decisions::load(store, p).unwrap_or_default();
    let mut holds = crate::reviewgroups::Holds::default();
    match crate::reviewgroups::summary_with_holds_shared(store, p, repo) {
        Ok((groups, held)) => {
            holds = held;
            let in_force = delegations.mode() == crate::decisions::Mode::Delegated
                && delegations
                    .delegations
                    .iter()
                    .any(|g| g.state(&now) == crate::decisions::DelegationState::Granted);
            decisions.extend(groups.into_iter().filter(|g| g.count > 0).map(|g| {
                AttentionLine {
                    to: Some(format!("/p/{p}/review?group={}", g.key)),
                    // Claude's verdicts are shown to check, not counted, while a delegation is in
                    // force (DEF-104).
                    yours: !(in_force && g.key == "delegated"),
                    kind: g.key,
                    section: Section::Decisions,
                    label: g.label,
                    count: g.count,
                    ask: None,
                    tone: INFO.into(),
                }
            }))
        }
        Err(e) => health.push(could_not_read("review groups", &e)),
    }

    // The person's planning decisions wait for Claude, not the person: shown, not counted (FTR-049).
    match crate::planning::list(store, p, Some("pending")) {
        Ok(pending) if !pending.is_empty() => decisions.push(AttentionLine {
            kind: "planning".into(),
            section: Section::Decisions,
            label: "Planning decisions waiting for Claude".into(),
            count: pending.len(),
            to: Some(format!("/p/{p}/plan")),
            ask: None,
            tone: INFO.into(),
            yours: false,
        }),
        Ok(_) => {}
        Err(e) => health.push(could_not_read("planning decisions", &e)),
    }

    match crate::releases::load(store, p) {
        Ok(all) => releases.extend(
            all.iter()
                .filter(|r| {
                    r.waits_on_tag()
                        || (!r.stage.is_empty()
                            && r.state == ReleaseState::Planned
                            && r.superseded_by.is_empty())
                })
                .map(|r| AttentionLine {
                    kind: "release".into(),
                    section: Section::Releases,
                    label: if r.waits_on_tag() {
                        format!("{} tagged, waiting for its workflow", r.version)
                    } else {
                        format!("{} at {}", r.version, r.stage)
                    },
                    count: 1,
                    to: Some(format!("/p/{p}/releases?tab=current#{}", r.version)),
                    ask: None,
                    tone: INFO.into(),
                    yours: true,
                }),
        ),
        Err(e) => health.push(could_not_read("releases", &e)),
    }

    let red: usize = project
        .features
        .iter()
        .map(|f| item_evidence(f, None).tests_red)
        .sum();
    if red > 0 {
        health.push(AttentionLine {
            kind: "red-tests".into(),
            section: Section::Health,
            label: "Red tests".into(),
            count: red,
            to: Some(format!("/p/{p}/items?evidence=red")),
            ask: None,
            tone: ERROR.into(),
            yours: false,
        });
    }

    match crate::doctor::run_project(store, p) {
        Ok(report) => {
            let errors = report
                .issues
                .iter()
                .filter(|i| i.severity == crate::doctor::Severity::Error)
                .count();
            if errors > 0 {
                health.push(AttentionLine {
                    kind: "doctor-errors".into(),
                    section: Section::Health,
                    label: "Doctor errors".into(),
                    count: errors,
                    to: None,
                    ask: Some(format!(
                        "Ask Claude to fix the {errors} doctor {}.",
                        if errors == 1 { "error" } else { "errors" }
                    )),
                    tone: ERROR.into(),
                    yours: false,
                });
            }
        }
        Err(e) => health.push(could_not_read("doctor", &e)),
    }

    if let Some(root) = repo.filter(|_| project.config.cadence.git_model) {
        match crate::gitdrift::scan(store, p, root) {
            Ok(findings) if !findings.is_empty() => health.push(AttentionLine {
                kind: "git-doctor".into(),
                section: Section::Health,
                label: "Git doctor findings".into(),
                count: findings.len(),
                to: Some(format!("/p/{p}/workflow#git")),
                ask: None,
                tone: WARN.into(),
                yours: false,
            }),
            Ok(_) => {}
            Err(e) => health.push(could_not_read("git doctor", &e)),
        }
    }

    match crate::retro::due(store, p) {
        Ok(due) => records.extend(due.into_iter().map(|m| AttentionLine {
            kind: "retro-due".into(),
            section: Section::Records,
            label: format!("Retro due: {m}"),
            count: 1,
            to: None,
            ask: Some(format!(
                "Ask Claude to write the retro for {m} from its facts."
            )),
            tone: INFO.into(),
            yours: false,
        })),
        Err(e) => health.push(could_not_read("retros", &e)),
    }

    let lines: Vec<AttentionLine> = decisions
        .into_iter()
        .chain(releases)
        .chain(health)
        .chain(records)
        .collect();
    Ok(Attention {
        total: lines.iter().filter(|l| l.yours).map(|l| l.count).sum(),
        lines,
        delegated: store
            .load(p)
            .ok()
            .and_then(|project| delegated_attention(&delegations, &project, &holds, &now)),
    })
}

/// The Health line that stands in for a source that failed (FEAT-084: nothing fails silently).
fn could_not_read(source: &str, e: &crate::CoreError) -> AttentionLine {
    AttentionLine {
        kind: "read-error".into(),
        section: Section::Health,
        label: format!("Could not read {source}: {e}"),
        count: 0,
        to: None,
        ask: None,
        tone: WARN.into(),
        yours: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef, TestState};
    use crate::releases::Release;

    /// A board in its own folder under the temp dir (L-41), checked before the first write, with
    /// releases on and one milestone.
    fn board() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-attention-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        assert!(
            root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()),
            "a test board lives in the temp dir: {}",
            root.display()
        );
        assert!(
            !dir.join(".kanbanr").exists(),
            "the board is the test's own"
        );
        let store = Store::new(dir.clone());
        let mut config = ProjectConfig::default_for("demo");
        config.cadence.releases = true;
        store.init_project("demo", config).unwrap();
        assert_eq!(std::fs::canonicalize(store.data_dir()).unwrap(), root);
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    /// An item awaiting approval, with one requirement and one test.
    fn awaiting_approval(store: &Store, title: &str) -> String {
        let f = store.add_feature("demo", title, "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    statement: title.into(),
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "THE SYSTEM SHALL hold.".into(),
                        tests: vec![TestRef {
                            name: "it_holds".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        f.code
    }

    /// Writes the releases file as given: a release promoted to `stage`, without a repository.
    fn release_at(store: &Store, version: &str, stage: &str) {
        let mut all = crate::releases::load(store, "demo").unwrap();
        all.push(Release {
            version: version.into(),
            stage: stage.into(),
            ..Default::default()
        });
        std::fs::write(
            store.project_dir("demo").join("releases.yaml"),
            crate::yaml::to_string(&all).unwrap(),
        )
        .unwrap();
    }

    /// A board with one waiting thing of each kind: an item awaiting approval, a release at
    /// candidate, a red test, a doctor error and a milestone due a retro.
    fn waiting_of_each_kind() -> (Store, std::path::PathBuf, String) {
        let (store, dir) = board();
        let code = awaiting_approval(&store, "Needs agreeing");
        store
            .set_test_state("demo", &code, "R-1", "it_holds", TestState::Red, None)
            .unwrap();
        // A dangling dependency is a doctor error.
        let mut item = store.load("demo").unwrap().feature(&code).unwrap().clone();
        item.depends_on = vec!["FTR-999".into()];
        store.persist_feature_for_test("demo", &item).unwrap();
        release_at(&store, "v0.8.0", "candidate");
        // A finished wave with no retro written.
        store
            .add_milestone("demo", "Done wave", "", vec![], Some("MS-2".into()))
            .unwrap();
        let done = store
            .add_feature("demo", "Finished", "", "MS-2", None)
            .unwrap();
        store.move_feature("demo", &done.code, "Scheduled").unwrap();
        store.move_feature("demo", &done.code, "Completed").unwrap();
        (store, dir, code)
    }

    /// FTR-048 R-1: every waiting kind is a line with its count; decisions and releases link to
    /// their page, doctor and retro carry a sentence to ask Claude and no link.
    #[test]
    fn lines_cover_every_waiting_kind_with_counts_and_links() {
        let (store, dir, _code) = waiting_of_each_kind();
        let a = attention(&store, "demo", None).unwrap();
        let line = |kind: &str| {
            a.lines
                .iter()
                .find(|l| l.kind == kind)
                .unwrap_or_else(|| panic!("no {kind} line: {a:#?}"))
        };

        // The decisions are the review groups, label and count copied, linked by key.
        let groups = crate::reviewgroups::summary(&store, "demo", None).unwrap();
        assert!(groups.iter().any(|g| g.key == "approve"), "{groups:#?}");
        for g in &groups {
            let l = line(&g.key);
            assert_eq!(l.section, Section::Decisions);
            assert_eq!(l.label, g.label);
            assert_eq!(l.count, g.count);
            assert_eq!(
                l.to.as_deref(),
                Some(format!("/p/demo/review?group={}", g.key).as_str())
            );
            assert_eq!(l.tone, "info");
            assert!(l.yours);
        }
        assert_eq!(line("approve").count, 1);

        let release = line("release");
        assert_eq!(release.section, Section::Releases);
        assert_eq!(release.label, "v0.8.0 at candidate");
        assert_eq!(release.count, 1);
        assert_eq!(
            release.to.as_deref(),
            Some("/p/demo/releases?tab=current#v0.8.0")
        );

        let red = line("red-tests");
        assert_eq!(red.section, Section::Health);
        assert_eq!(red.count, 1);
        assert_eq!(red.to.as_deref(), Some("/p/demo/items?evidence=red"));
        assert_eq!(red.tone, "error");

        let doctor = line("doctor-errors");
        let errors = crate::doctor::run_project(&store, "demo")
            .unwrap()
            .issues
            .iter()
            .filter(|i| i.severity == crate::doctor::Severity::Error)
            .count();
        assert!(errors >= 1);
        assert_eq!(doctor.section, Section::Health);
        assert_eq!(doctor.count, errors);
        assert_eq!(doctor.to, None);
        let ask = doctor.ask.as_deref().unwrap();
        assert!(
            ask.starts_with(&format!("Ask Claude to fix the {errors} doctor error")),
            "{ask}"
        );
        assert_eq!(doctor.tone, "error");

        let retro = line("retro-due");
        assert_eq!(retro.section, Section::Records);
        assert_eq!(retro.label, "Retro due: MS-2");
        assert_eq!(retro.count, 1);
        assert_eq!(retro.to, None);
        assert_eq!(
            retro.ask.as_deref(),
            Some("Ask Claude to write the retro for MS-2 from its facts.")
        );

        // No repository: no git doctor line, and nothing failed to read.
        assert!(a.lines.iter().all(|l| l.kind != "git-doctor"));
        assert!(a.lines.iter().all(|l| l.kind != "read-error"), "{a:#?}");

        // In section order.
        let sections: Vec<Section> = a.lines.iter().map(|l| l.section).collect();
        let mut sorted = sections.clone();
        sorted.sort();
        assert_eq!(sections, sorted);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-1: a board with no items needs nothing, and that is not an error (L-29).
    #[test]
    fn an_empty_board_needs_nothing() {
        let (store, dir) = board();
        let a = attention(&store, "demo", None).unwrap();
        assert_eq!(a.total, 0);
        assert_eq!(a.lines, Vec::new());
        assert!(
            attention(&store, "nope", None).is_err(),
            "an unknown project"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-048 R-2: the badge counts only the person's own decisions: the review groups and the
    /// releases at a stage, never red tests, doctor errors or retros.
    #[test]
    fn the_badge_counts_only_the_persons_decisions() {
        let (store, dir, _code) = waiting_of_each_kind();
        let a = attention(&store, "demo", None).unwrap();
        let decisions: usize = crate::reviewgroups::summary(&store, "demo", None)
            .unwrap()
            .iter()
            .map(|g| g.count)
            .sum();
        assert!(decisions >= 1);
        assert_eq!(a.total, decisions + 1, "the groups and the one release");
        for l in &a.lines {
            let counted = matches!(l.section, Section::Decisions | Section::Releases);
            assert_eq!(l.yours, counted, "{l:#?}");
        }
        assert!(a.lines.iter().any(|l| !l.yours && l.count > 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-049 R-3: a pending planning decision waits for Claude, not the person: it is a line in
    /// Decisions, linked to the Plan page, and not counted in the badge.
    #[test]
    fn pending_planning_decisions_are_listed_but_not_counted() {
        let (store, dir) = board();
        let item = store.add_feature("demo", "Plan me", "", "M", None).unwrap();
        release_at(&store, "v0.9.0", "");
        let before = attention(&store, "demo", None).unwrap();
        assert!(
            before.lines.iter().all(|l| l.kind != "planning"),
            "{before:#?}"
        );

        crate::planning::record(
            &store,
            "demo",
            &serde_json::json!({
                "action": "into-release",
                "target": "v0.9.0",
                "items": [item.code],
                "by": "Ada",
            }),
            "monitor",
        )
        .unwrap();
        let a = attention(&store, "demo", None).unwrap();
        let line = a
            .lines
            .iter()
            .find(|l| l.kind == "planning")
            .unwrap_or_else(|| panic!("no planning line: {a:#?}"));
        assert_eq!(line.section, Section::Decisions);
        assert_eq!(line.label, "Planning decisions waiting for Claude");
        assert_eq!(line.count, 1);
        assert_eq!(line.to.as_deref(), Some("/p/demo/plan"));
        assert_eq!(line.ask, None);
        assert_eq!(line.tone, "info");
        assert!(!line.yours);
        assert_eq!(a.total, before.total, "not counted in the badge");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A board with item A (held), item B (the person's), and C and D approved by Claude under
    /// DLG-001, in delegated mode. DLG-001 covers `approve` on A, C and D until `until`; `revoked_at`
    /// revokes it then.
    fn delegated_board(
        until: &str,
        revoked_at: Option<&str>,
    ) -> (Store, std::path::PathBuf, [String; 4]) {
        use crate::decisions::{Decisions, Delegation, Mode, ModeChange, Stamp};
        let (store, dir) = board();
        let a = awaiting_approval(&store, "Alpha");
        let b = awaiting_approval(&store, "Beta");
        let c = awaiting_approval(&store, "Gamma");
        let d = awaiting_approval(&store, "Delta");
        let stamp = |origin: &str, at: &str| Stamp {
            by: "Ada".into(),
            at: at.into(),
            origin: origin.into(),
            ..Default::default()
        };
        let decisions = Decisions {
            modes: vec![ModeChange {
                mode: Mode::Delegated,
                by: "Ada".into(),
                at: "2026-11-01T00:00:00+00:00".into(),
                origin: "monitor".into(),
                reason: String::new(),
                extra: Default::default(),
            }],
            delegations: vec![Delegation {
                id: "DLG-001".into(),
                verdicts: vec!["approve".into()],
                items: vec![a.clone(), c.clone(), d.clone()],
                until: until.into(),
                proposed: stamp("conversation", "2026-11-01T00:00:00+00:00"),
                granted: Some(stamp("monitor", "2026-11-01T00:00:00+00:00")),
                revoked: revoked_at.map(|at| stamp("monitor", at)),
                ..Default::default()
            }],
            ..Default::default()
        };
        // Claude approved C, then D, under DLG-001 (`at` set so the order is certain).
        for (code, at) in [
            (&c, "2026-11-10T09:00:00+00:00"),
            (&d, "2026-11-12T09:00:00+00:00"),
        ] {
            store
                .approve_feature_as("demo", code, "Claude", "delegated", "DLG-001", None)
                .unwrap();
            let mut item = store.load("demo").unwrap().feature(code).unwrap().clone();
            for e in &mut item.definition.as_mut().unwrap().approvals {
                e.at = at.into();
            }
            store.persist_feature_for_test("demo", &item).unwrap();
        }
        std::fs::write(
            store.project_dir("demo").join("decisions.yaml"),
            crate::yaml::to_string(&decisions).unwrap(),
        )
        .unwrap();
        (store, dir, [a, b, c, d])
    }

    /// DEF-104 (supports R-4, R-5): in delegated mode the badge counts only what no delegation
    /// holds; what Claude decided is listed newest first and not counted; each delegation lists
    /// what it holds.
    #[test]
    fn delegated_mode_counts_only_the_persons_decisions() {
        let (store, dir, [a, b, c, d]) = delegated_board("2099-12-04T00:00:00+00:00", None);
        let at = attention(&store, "demo", None).unwrap();
        let approve = at.lines.iter().find(|l| l.kind == "approve").unwrap();
        assert_eq!(
            (approve.count, approve.yours),
            (1, true),
            "only Beta is the person's"
        );
        let line = at
            .lines
            .iter()
            .find(|l| l.kind == "delegated")
            .unwrap_or_else(|| panic!("no delegated line: {at:#?}"));
        assert!(
            !line.yours,
            "Claude's verdicts are shown to check, not counted"
        );
        assert_eq!(line.count, 2);
        assert_eq!(at.total, 1, "{at:#?}");
        let dg = at.delegated.as_ref().expect("a delegated part");
        assert_eq!(dg.in_force, vec!["DLG-001".to_string()]);
        assert_eq!(
            dg.decided
                .iter()
                .map(|x| x.code.clone())
                .collect::<Vec<_>>(),
            vec![d, c],
            "newest first"
        );
        assert_eq!(dg.decided_total, 2);
        assert_eq!(dg.holds.len(), 1);
        assert_eq!(dg.holds[0].delegation, "DLG-001");
        assert_eq!(dg.holds[0].total, 1, "Alpha");
        assert_eq!(
            dg.holds[0].verdicts,
            vec![HeldCount {
                verdict: "approve".into(),
                count: 1
            }]
        );
        assert!(dg.holds[0].until.starts_with("2099-12-04"));
        assert!(dg.ending.is_empty());
        let _ = (a, b);
        // Monitor mode: nothing is Claude's, there is no delegated part and the line counts.
        std::fs::remove_file(store.project_dir("demo").join("decisions.yaml")).unwrap();
        let at = attention(&store, "demo", None).unwrap();
        assert!(at.delegated.is_none());
        assert_eq!(
            at.total, 4,
            "Alpha and Beta, and Gamma and Delta to keep or undo, as before"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-104 R-6 (core half): a delegation that ends within three days, or was revoked within
    /// them, is listed with how many decisions return to the person.
    #[test]
    fn a_delegation_ending_within_three_days_is_listed_with_what_returns() {
        let at = |hours: i64| {
            (time::OffsetDateTime::now_utc() + time::Duration::hours(hours))
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap()
        };
        // Ends in two days: it still holds Alpha; one decision returns.
        let (store, dir, _) = delegated_board(&at(48), None);
        let a = attention(&store, "demo", None).unwrap();
        let dg = a.delegated.unwrap();
        assert_eq!(dg.ending.len(), 1, "{:?}", dg.ending);
        let e = &dg.ending[0];
        assert_eq!(e.delegation, "DLG-001");
        assert_eq!(e.state, crate::decisions::DelegationState::Granted);
        assert_eq!(e.returns, 1);
        let _ = std::fs::remove_dir_all(dir);
        // Ends in five days: no ending line.
        let (store, dir, _) = delegated_board(&at(120), None);
        let dg = attention(&store, "demo", None).unwrap().delegated.unwrap();
        assert!(dg.ending.is_empty());
        let _ = std::fs::remove_dir_all(dir);
        // Revoked an hour ago: nothing is in force; Alpha, Gamma's and Delta's verdicts it held
        // return... only the queue's: Alpha.
        let (store, dir, _) = delegated_board(&at(500), Some(&at(-1)));
        let a = attention(&store, "demo", None).unwrap();
        let dg = a.delegated.expect("an ending line keeps the part");
        assert!(dg.in_force.is_empty());
        assert_eq!(dg.ending.len(), 1);
        assert_eq!(
            dg.ending[0].state,
            crate::decisions::DelegationState::Revoked
        );
        assert_eq!(dg.ending[0].returns, 1, "Alpha returns to the person");
        assert_eq!(
            a.total, 4,
            "Alpha and Beta are the person's again, and the two decided ones count again"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
