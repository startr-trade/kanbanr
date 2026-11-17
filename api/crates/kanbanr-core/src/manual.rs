//! Manual test results (DEF-040): a manual test says a person checked something, so its result is
//! kept as raw data beside the test's state — who, when, from where, and any note.
//!
//! Each result is a file of its own, `manual/<YYYYMMDDTHHMMSSZ>-<CODE>-<R-n>.yaml` in the project
//! folder, written once and never rewritten, as `gates::record` does. The test's `state` and
//! `checked_rev` on the item are set as `kanbanr test` sets them; nothing new goes on `TestRef`.

use crate::config::ProjectConfig;
use crate::error::{CoreError, Result};
use crate::models::TestState;
use crate::store::{Project, Store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The longest note a result keeps.
pub const NOTE_MAX: usize = 4000;

/// One kept result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ManualResult {
    pub code: String,
    pub requirement: String,
    pub test: String,
    /// `green` or `red`.
    pub state: String,
    pub by: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// `monitor` or `terminal`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// The code commit it was checked at, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked_rev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The prepared record's id, when confirmed from one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prepared: String,
    /// Keys this version does not know are kept and written back unchanged.
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// A manual test that is not green, with what a person needs to make the check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ManualWaiting {
    pub code: String,
    pub title: String,
    pub status: String,
    pub requirement: String,
    pub requirement_text: String,
    pub test: String,
    /// `planned` or `red`.
    pub state: String,
    pub checked_rev: String,
    /// The newest kept result for this test, so a failed check shows who failed it.
    pub last: Option<ManualResult>,
    /// The build stage the check opens at (DEF-076); empty when the check is due.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub opens_at: String,
    /// The item's release (FTR-057); empty when it has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release: String,
    /// The button this check holds (FTR-057), for example "Promote v0.7.0 to main": set when the
    /// item's release is one of [`holding`]'s and its next forward promotion waits for this test
    /// ([`crate::releases::tests_due`]). Derived, never stored (ADR-0002).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub holds: String,
}

/// The releases the Review page puts first (FTR-057): the versions of releases in flight (at a
/// stage, not shipped, not superseded), newest first by version; when there are none, the one
/// planned release with items and the lowest version, the next to ship. Empty when the project
/// does not work in releases.
pub fn holding(store: &Store, id: &str) -> Result<Vec<String>> {
    let project = store.load_meta(id)?;
    let releases = crate::releases::load(store, id)?;
    Ok(holding_of(&project, &releases))
}

fn holding_of(project: &Project, releases: &[crate::releases::Release]) -> Vec<String> {
    use crate::releases::{ReleaseState, version_key};
    if !project.config.cadence.releases {
        return Vec::new();
    }
    let open = |r: &&crate::releases::Release| {
        r.state != ReleaseState::Shipped && r.superseded_by.is_empty()
    };
    let mut flight: Vec<&crate::releases::Release> = releases
        .iter()
        .filter(open)
        .filter(|r| !r.stage.is_empty())
        .collect();
    if !flight.is_empty() {
        flight.sort_by_cached_key(|r| std::cmp::Reverse(version_key(&r.version)));
        return flight.into_iter().map(|r| r.version.clone()).collect();
    }
    releases
        .iter()
        .filter(open)
        .filter(|r| r.stage.is_empty())
        .filter(|r| {
            project
                .features
                .iter()
                .any(|f| f.release.as_deref() == Some(r.version.as_str()))
        })
        .min_by_key(|r| version_key(&r.version))
        .map(|r| vec![r.version.clone()])
        .unwrap_or_default()
}

/// What one holding release's next forward promotion waits for.
struct Held {
    version: String,
    /// That promotion's button, e.g. "Promote v0.8.0 to main".
    button: String,
    /// The due tests, as (code, requirement, test).
    due: Vec<(String, String, String)>,
}

/// The tests each holding release's next forward promotion waits for, with that promotion's
/// button. One [`crate::releases::tests_due`] per holding release.
fn held_by(
    project: &Project,
    releases: &[crate::releases::Release],
    holding: &[String],
) -> Vec<Held> {
    let cadence = &project.config.cadence;
    let stages = cadence.stages();
    if stages.is_empty() {
        return Vec::new();
    }
    let last = stages.len() - 1;
    let cut = cadence.cut_index();
    let mut out = Vec::new();
    for version in holding {
        let Some(r) = releases.iter().find(|r| r.version == *version) else {
            continue;
        };
        // One step forward, as `work_out` computes it: the stage after the release's, or the
        // first for a release at no stage.
        let next = if r.stage.is_empty() {
            Some(0)
        } else {
            stages.iter().position(|s| *s == r.stage).map(|c| c + 1)
        };
        let Some(next) = next.filter(|&n| n <= last) else {
            continue;
        };
        let due = crate::releases::tests_due(project, cadence, version, &[next], cut)
            .into_iter()
            .map(|d| (d.code, d.requirement, d.test))
            .collect();
        let button = crate::releases::button_text(&stages, cut, last, version, next);
        out.push(Held {
            version: version.clone(),
            button,
            due,
        });
    }
    out
}

/// The build stage and the stages at or after it, in workflow order, no-ops excluded (FTR-018,
/// DEF-076). `None` when no stage holds the build role, which counts every stage as build onward.
/// [`crate::readiness::Context`] reads its `build_onward` from here, so the two cannot disagree.
pub fn build_onward(config: &ProjectConfig) -> Option<std::collections::BTreeSet<String>> {
    let build = config.role_status(crate::config::Role::Build)?;
    Some(
        config
            .statuses
            .iter()
            .skip_while(|s| **s != build)
            .filter(|s| !config.is_no_op(s))
            .cloned()
            .collect(),
    )
}

/// Where a manual check of an item at `status` opens (DEF-076): `Some(build)` when the workflow
/// has a build role and `status` comes before it in the workflow's order (no-ops skipped); `None`
/// when the check is due, or the workflow has no build role.
pub fn opens_at(config: &ProjectConfig, status: &str) -> Option<String> {
    let build = config.role_status(crate::config::Role::Build)?;
    if config.is_no_op(status) {
        return None;
    }
    let onward = build_onward(config)?;
    let at = config.statuses.iter().position(|s| s == status)?;
    let build_at = config.statuses.iter().position(|s| *s == build)?;
    (at < build_at && !onward.contains(status)).then_some(build)
}

/// Refuse a manual result for an item that has not reached its build stage (DEF-076): before
/// then nothing exists to check. Acts only on a manual test set green or red; setting a test back
/// to planned, or any other kind of test, is never refused here.
pub fn refuse_before_build(
    project: &Project,
    code: &str,
    requirement: &str,
    test: &str,
    state: TestState,
) -> Result<()> {
    if state == TestState::Planned {
        return Ok(());
    }
    let Some(f) = crate::aliases::find(project, code) else {
        return Ok(());
    };
    let manual = f
        .definition
        .as_ref()
        .and_then(|d| d.requirements.iter().find(|r| r.id == requirement))
        .and_then(|r| r.tests.iter().find(|t| t.name == test))
        .is_some_and(|t| t.kind.eq_ignore_ascii_case("manual"));
    if !manual {
        return Ok(());
    }
    match opens_at(&project.config, &f.status) {
        Some(build) => Err(unsupported(format!(
            "{code} is at {}: its manual check opens at {build}, once the code it checks exists. \
             Record it then.",
            f.status
        ))),
        None => Ok(()),
    }
}

fn unsupported(msg: String) -> CoreError {
    CoreError::Unsupported(msg)
}

/// The project folder that holds recorded manual test results.
pub const FOLDER: &str = "manual";

fn dir(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(FOLDER)
}

fn state_name(s: TestState) -> &'static str {
    match s {
        TestState::Planned => "planned",
        TestState::Red => "red",
        TestState::Green => "green",
    }
}

fn check(r: &ManualResult) -> Result<()> {
    if r.state != "green" && r.state != "red" {
        return Err(unsupported(format!(
            "a manual result is green or red, not '{}'",
            r.state
        )));
    }
    if r.by.trim().is_empty() {
        return Err(unsupported(
            "a manual result must name the person who checked it".into(),
        ));
    }
    if r.note.chars().count() > NOTE_MAX {
        return Err(unsupported(format!(
            "the note is longer than {NOTE_MAX} characters"
        )));
    }
    Ok(())
}

/// Keep a result as a new file beside the earlier ones. Written to a temporary file in the same
/// folder, then renamed, so a crash never leaves half a file.
pub fn record(store: &Store, id: &str, r: &ManualResult) -> Result<PathBuf> {
    check(r)?;
    let folder = dir(store, id);
    std::fs::create_dir_all(&folder)?;
    let stamp: String =
        r.at.chars()
            .filter(|c| c.is_ascii_digit())
            .take(14)
            .collect();
    let stamp = if stamp.len() == 14 {
        format!("{}T{}Z", &stamp[..8], &stamp[8..])
    } else {
        return Err(unsupported(format!("'{}' is not a time", r.at)));
    };
    let base = format!("{stamp}-{}-{}", r.code, r.requirement);
    // Two results in one second for one test keep both.
    let mut path = folder.join(format!("{base}.yaml"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{base}-{n}.yaml"));
        n += 1;
    }
    crate::atomic::write(&path, crate::yaml::to_string(r)?)?;
    Ok(path)
}

/// Every kept result, oldest first, optionally of one item. A file that cannot be read is skipped.
pub fn results(store: &Store, id: &str, code: Option<&str>) -> Result<Vec<ManualResult>> {
    let Ok(files) = std::fs::read_dir(dir(store, id)) else {
        return Ok(Vec::new());
    };
    // FTR-056: a result kept under an earlier code of the item is the item's.
    let codes: Option<Vec<String>> = code.map(|c| {
        store
            .load_meta(id)
            .ok()
            .and_then(|p| {
                crate::aliases::find(&p, c).map(|f| f.codes().map(str::to_string).collect())
            })
            .unwrap_or_else(|| vec![c.to_string()])
    });
    let mut found: Vec<((String, u32), ManualResult)> = Vec::new();
    for f in files.flatten() {
        let p = f.path();
        if p.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(r) = crate::yaml::from_str::<ManualResult>(&text) else {
            continue;
        };
        if codes.as_ref().is_some_and(|cs| !cs.contains(&r.code)) {
            continue;
        }
        // Oldest first: by the stamp in the name, then by the counter a same-second result gets
        // (`-2`), which text order would put before the first.
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let stamp = stem.get(..16).unwrap_or(stem).to_string();
        let n = stem
            .rsplit_once('-')
            .and_then(|(head, tail)| {
                let own = format!("-{}-{}", r.code, r.requirement);
                (head.ends_with(&own))
                    .then(|| tail.parse::<u32>().ok())
                    .flatten()
            })
            .unwrap_or(1);
        found.push(((stamp, n), r));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found.into_iter().map(|(_, r)| r).collect())
}

/// The manual tests waiting for a person (DEF-040 decision 1): every item not in a no-op status,
/// live work first, then finished work, each in board order, then requirement order. Checks not
/// yet due (DEF-076: the item is before its build stage) come after every due one, with
/// `opens_at` set.
pub fn waiting(store: &Store, id: &str) -> Result<Vec<ManualWaiting>> {
    let project = store.load_meta(id)?;
    let kept = results(store, id, None)?;
    let releases = crate::releases::load(store, id)?;
    let held = held_by(&project, &releases, &holding_of(&project, &releases));
    let mut live = Vec::new();
    let mut done = Vec::new();
    let mut not_due = Vec::new();
    for f in &project.features {
        if project.config.is_no_op(&f.status) {
            continue;
        }
        let Some(def) = &f.definition else { continue };
        let opens = opens_at(&project.config, &f.status).unwrap_or_default();
        let out = if !opens.is_empty() {
            &mut not_due
        } else if project.config.counts_as_done(&f.status) {
            &mut done
        } else {
            &mut live
        };
        for r in &def.requirements {
            for t in &r.tests {
                if !t.kind.eq_ignore_ascii_case("manual") || t.state == TestState::Green {
                    continue;
                }
                out.push(ManualWaiting {
                    code: f.code.clone(),
                    title: f.title.clone(),
                    status: f.status.clone(),
                    requirement: r.id.clone(),
                    requirement_text: r.text.clone(),
                    test: t.name.clone(),
                    state: state_name(t.state).into(),
                    checked_rev: t.checked_rev.clone(),
                    last: kept
                        .iter()
                        .rev()
                        .find(|k| {
                            f.answers_to(&k.code) && k.requirement == r.id && k.test == t.name
                        })
                        .cloned(),
                    opens_at: opens.clone(),
                    release: f.release.clone().unwrap_or_default(),
                    holds: held
                        .iter()
                        .find(|h| {
                            f.release.as_deref() == Some(h.version.as_str())
                                && h.due.iter().any(|(c, req, test)| {
                                    f.answers_to(c) && *req == r.id && *test == t.name
                                })
                        })
                        .map(|h| h.button.clone())
                        .unwrap_or_default(),
                });
            }
        }
    }
    live.extend(done);
    live.extend(not_due);
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef};

    fn fixture() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-manual-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "Wave", "", vec![], Some("MS-1".into()))
            .unwrap();
        (store, dir)
    }

    fn test(name: &str, kind: &str) -> TestRef {
        TestRef {
            name: name.into(),
            kind: kind.into(),
            ..Default::default()
        }
    }

    /// FTR-057 R-2: the releases the Review page puts first are those in flight (at a stage),
    /// newest first; with none in flight, the planned release with items and the lowest version;
    /// with releases off, none.
    #[test]
    fn holding_releases_are_those_in_flight_else_the_next_planned() {
        let (store, dir) = fixture();
        assert_eq!(holding(&store, "demo").unwrap(), Vec::<String>::new());
        let mut c = store.load_meta("demo").unwrap().config.cadence;
        c.releases = true;
        c.release_stages = vec!["candidate".into(), "main".into(), "production".into()];
        store.set_cadence("demo", None, c).unwrap();
        assert_eq!(holding(&store, "demo").unwrap(), Vec::<String>::new());
        for v in ["v0.9.0", "v0.7.1", "v0.8.0", "v0.6.0"] {
            crate::releases::add(&store, "demo", v, "", "").unwrap();
        }
        // v0.6.0 has no items, so it is not the next to ship.
        for (title, v) in [("A", "v0.9.0"), ("B", "v0.8.0"), ("C", "v0.7.1")] {
            let code = store
                .add_feature("demo", title, "", "MS-1", None)
                .unwrap()
                .code;
            crate::releases::plan(&store, "demo", v, &[code]).unwrap();
        }
        assert_eq!(holding(&store, "demo").unwrap(), ["v0.7.1"]);
        let file = store.project_dir("demo").join("releases.yaml");
        let mut all = crate::releases::load(&store, "demo").unwrap();
        for r in &mut all {
            match r.version.as_str() {
                "v0.8.0" => r.stage = "candidate".into(),
                "v0.7.1" => r.stage = "main".into(),
                _ => {}
            }
        }
        std::fs::write(&file, crate::yaml::to_string(&all).unwrap()).unwrap();
        assert_eq!(holding(&store, "demo").unwrap(), ["v0.8.0", "v0.7.1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-040 R-2: the kept result says who checked, when, from where, and the note.
    #[test]
    fn a_manual_result_is_kept_with_who_when_and_the_note() {
        let (store, dir) = fixture();
        let mut r = ManualResult {
            code: "FEAT-002".into(),
            requirement: "R-17".into(),
            test: "manual: the cart keeps its items".into(),
            state: "green".into(),
            by: "Ada L".into(),
            at: "2026-11-12T11:02:00Z".into(),
            origin: "monitor".into(),
            checked_rev: "74aa8cc".into(),
            note: "on staging".into(),
            ..Default::default()
        };
        let first = record(&store, "demo", &r).unwrap();
        // By components, so Windows' `\\` separator compares too (DEF-062).
        assert!(
            first.ends_with(
                std::path::Path::new("manual").join("20261112T110200Z-FEAT-002-R-17.yaml")
            ),
            "{first:?}"
        );
        // The same second keeps both files, never rewriting the first.
        r.state = "red".into();
        let second = record(&store, "demo", &r).unwrap();
        assert_ne!(first, second);
        let kept = results(&store, "demo", Some("FEAT-002")).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(
            (
                kept[0].by.as_str(),
                kept[0].at.as_str(),
                kept[0].note.as_str()
            ),
            ("Ada L", "2026-11-12T11:02:00Z", "on staging")
        );
        assert_eq!(
            (kept[0].origin.as_str(), kept[0].state.as_str()),
            ("monitor", "green")
        );
        assert!(
            results(&store, "demo", Some("FEAT-009"))
                .unwrap()
                .is_empty()
        );
        // No person named, no record.
        r.by = " ".into();
        assert!(record(&store, "demo", &r).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-040 R-1: each manual test not green is listed with its requirement and the check.
    #[test]
    fn waiting_lists_manual_tests_not_green_with_their_requirement() {
        let (store, dir) = fixture();
        let f = store.add_feature("demo", "Cart", "", "MS-1", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(FeatureDefinition {
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "THE SYSTEM SHALL hold the cart.".into(),
                        tests: vec![
                            test("cart::holds", "unit"),
                            test("manual: looks right", "manual"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // An item has at most one judgement test (FTR-039): the green one is on a second item.
        let g = store.add_feature("demo", "Till", "", "MS-1", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &g.code,
                Some(FeatureDefinition {
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "THE SYSTEM SHALL open the till.".into(),
                        tests: vec![test("manual: feels right", "manual")],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // A manual check opens at the build stage, Scheduled here (DEF-076).
        store.move_feature("demo", &f.code, "Scheduled").unwrap();
        store.move_feature("demo", &g.code, "Scheduled").unwrap();
        store
            .set_test_state(
                "demo",
                &g.code,
                "R-1",
                "manual: feels right",
                TestState::Green,
                None,
            )
            .unwrap();
        store
            .set_test_state(
                "demo",
                &f.code,
                "R-1",
                "manual: looks right",
                TestState::Red,
                None,
            )
            .unwrap();
        record(
            &store,
            "demo",
            &ManualResult {
                code: f.code.clone(),
                requirement: "R-1".into(),
                test: "manual: looks right".into(),
                state: "red".into(),
                by: "Ada L".into(),
                at: "2026-11-12T11:02:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let w = waiting(&store, "demo").unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].code, f.code);
        assert_eq!(w[0].requirement, "R-1");
        assert_eq!(w[0].requirement_text, "THE SYSTEM SHALL hold the cart.");
        assert_eq!(w[0].test, "manual: looks right");
        assert_eq!(w[0].state, "red");
        assert_eq!(w[0].last.as_ref().map(|l| l.by.as_str()), Some("Ada L"));
        let _ = std::fs::remove_dir_all(dir);
    }
    /// A workflow whose Build gate makes the branch: Backlog → Design → Build → Review → Done,
    /// with Dropped as a no-op.
    fn build_workflow() -> ProjectConfig {
        let mut c = ProjectConfig::default_for("demo");
        let all: Vec<String> = ["Backlog", "Design", "Build", "Review", "Done", "Dropped"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        c.statuses = all.clone();
        c.default_state = "Backlog".into();
        c.displayed_states = all[..5].to_vec();
        c.no_op_states = vec!["Dropped".into()];
        c.terminal_states = vec!["Done".into()];
        c.transitions = all
            .iter()
            .map(|from| {
                (
                    from.clone(),
                    all.iter().filter(|to| *to != from).cloned().collect(),
                )
            })
            .collect();
        c.gates.clear();
        c.gates.insert(
            "Build".into(),
            crate::config::Gate {
                on_enter: vec![crate::config::Action::Branch],
                ..Default::default()
            },
        );
        c
    }

    /// DEF-076 R-1: a manual check opens at the item's build stage and stays open after it; with
    /// no build role it is always open; checks not yet due come after the due ones.
    #[test]
    fn a_manual_check_is_not_due_before_its_item_reaches_its_build_stage() {
        let config = build_workflow();
        assert_eq!(opens_at(&config, "Backlog").as_deref(), Some("Build"));
        assert_eq!(opens_at(&config, "Design").as_deref(), Some("Build"));
        for due in ["Build", "Review", "Done", "Dropped"] {
            assert_eq!(opens_at(&config, due), None, "{due}");
        }
        // The same order as the readiness context's build stage, so the two cannot disagree.
        let onward = build_onward(&config).unwrap();
        for s in &config.statuses {
            if !config.is_no_op(s) {
                assert_eq!(opens_at(&config, s).is_none(), onward.contains(s), "{s}");
            }
        }
        // No build role: a declared gate that makes no branch.
        let mut none = build_workflow();
        none.gates.get_mut("Build").unwrap().on_enter.clear();
        assert_eq!(none.role_status(crate::config::Role::Build), None);
        for s in &none.statuses {
            assert_eq!(opens_at(&none, s), None, "{s}");
        }

        let (store, dir) = fixture();
        store.save_config("demo", &config).unwrap();
        let mut codes = Vec::new();
        for (title, status) in [("Early", "Design"), ("Built", "Build"), ("Later", "Review")] {
            let f = store.add_feature("demo", title, "", "MS-1", None).unwrap();
            store
                .set_feature_definition(
                    "demo",
                    &f.code,
                    Some(FeatureDefinition {
                        requirements: vec![Requirement {
                            id: "R-1".into(),
                            text: "THE SYSTEM SHALL work.".into(),
                            tests: vec![test(
                                "manual: the person judges whether it works for them",
                                "manual",
                            )],
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )
                .unwrap();
            store.move_feature("demo", &f.code, status).unwrap();
            codes.push(f.code);
        }
        let w = waiting(&store, "demo").unwrap();
        let rows: Vec<(&str, &str)> = w
            .iter()
            .map(|r| (r.code.as_str(), r.opens_at.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                (codes[1].as_str(), ""),
                (codes[2].as_str(), ""),
                (codes[0].as_str(), "Build"),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
