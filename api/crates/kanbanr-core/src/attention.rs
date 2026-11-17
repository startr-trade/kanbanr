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
}

/// One kind of thing waiting, counted, with where to see it or what to ask Claude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AttentionLine {
    /// A review group's key, as the Review page names it, or `release`, `red-tests`,
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

    match crate::reviewgroups::summary(store, p, repo) {
        Ok(groups) => {
            decisions.extend(
                groups
                    .into_iter()
                    .filter(|g| g.count > 0)
                    .map(|g| AttentionLine {
                        to: Some(format!("/p/{p}/review?group={}", g.key)),
                        kind: g.key,
                        section: Section::Decisions,
                        label: g.label,
                        count: g.count,
                        ask: None,
                        tone: INFO.into(),
                        yours: true,
                    }),
            )
        }
        Err(e) => health.push(could_not_read("review groups", &e)),
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
}
