//! An item's earlier codes (FTR-056). A rename keeps the old code in the item's `aliases`, so a
//! reference made under it — a commit trailer, a link, a dependency, a record — still reaches the
//! item, and no other item is ever given it. These are the lookups every reader shares.

use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::store::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The item `code` names, current code or alias. A current code wins over an alias, should a
/// broken board have both (doctor reports that).
pub fn find<'a>(project: &'a Project, code: &str) -> Option<&'a FeatureItem> {
    project
        .features
        .iter()
        .find(|f| f.code == code)
        .or_else(|| {
            project
                .features
                .iter()
                .find(|f| f.aliases.iter().any(|a| a == code))
        })
}

/// The current code `code` names, if any item answers to it.
pub fn current<'a>(project: &'a Project, code: &str) -> Option<&'a str> {
    find(project, code).map(|f| f.code.as_str())
}

/// Every code taken in the project: current codes and aliases.
pub fn taken(project: &Project) -> BTreeSet<String> {
    project
        .features
        .iter()
        .flat_map(|f| f.codes().map(str::to_string))
        .collect()
}

/// Refuse a code a new item or a rename would take when any item answers to it.
/// `except` is the item being renamed, so renaming back to its own alias is allowed.
pub fn check_free(project: &Project, code: &str, except: Option<&str>) -> Result<()> {
    if let Some(f) = project.features.iter().find(|f| f.code == code) {
        return Err(CoreError::CodeTaken(format!("{code} is {}'s code", f.code)));
    }
    if let Some(f) = project
        .features
        .iter()
        .find(|f| f.aliases.iter().any(|a| a == code))
        && except != Some(f.code.as_str())
    {
        return Err(CoreError::CodeTaken(format!(
            "{code} was an earlier code of {}, and a code is never given to another item",
            f.code
        )));
    }
    Ok(())
}

/// One rename of a bulk recode: the item `from` becomes `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Recode {
    pub from: String,
    pub to: String,
    pub kind: Option<String>,
    pub title: String,
}

/// What a bulk recode would do, and the revision that names it: confirming applies exactly this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RecodePlan {
    pub renames: Vec<Recode>,
    pub rev: String,
}

/// The letters before a code's last `-`.
fn prefix_of(code: &str) -> &str {
    code.rsplit_once('-').map_or(code, |(p, _)| p)
}

/// Every item whose code's prefix is not `codes.prefix_for(kind)`, each with its new code. Items
/// are taken in `created_at` order, then by code; each new code continues its prefix's counter
/// past every code and earlier code taken, so no number is reused. Reads only; an empty plan is
/// valid.
pub fn recode_plan(project: &Project) -> RecodePlan {
    let codes = &project.config.codes;
    let mut items: Vec<&FeatureItem> = project.features.iter().collect();
    items.sort_by(|a, b| (&a.created_at, &a.code).cmp(&(&b.created_at, &b.code)));
    let mut taken = taken(project);
    let mut renames = Vec::new();
    for f in items {
        let prefix = codes.prefix_for(f.kind.as_deref());
        if prefix_of(&f.code) == prefix {
            continue;
        }
        let existing: Vec<String> = taken.iter().cloned().collect();
        let to = crate::validate::next_code(prefix, &existing);
        taken.insert(to.clone());
        renames.push(Recode {
            from: f.code.clone(),
            to,
            kind: f.kind.clone(),
            title: f.title.clone(),
        });
    }
    let rev = crate::hash::stable_hash(&crate::hash::canonical_json(&renames));
    RecodePlan { renames, rev }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn item(code: &str, aliases: &[&str]) -> FeatureItem {
        let yaml = format!(
            "code: {code}\ntitle: {code}\nstatus: Backlog\nmilestone: MS-001\n\
             created_at: '2026-11-17T00:00:00Z'\nupdated_at: '2026-11-17T00:00:00Z'\n"
        );
        let mut f: FeatureItem = crate::yaml::from_str(&yaml).expect("item");
        f.aliases = aliases.iter().map(|a| a.to_string()).collect();
        f
    }

    fn project(features: Vec<FeatureItem>) -> Project {
        Project {
            id: "demo".into(),
            config: ProjectConfig::default_for("demo"),
            features,
            milestones: Vec::new(),
        }
    }

    #[test]
    fn an_alias_resolves_to_the_item_that_carries_it() {
        let p = project(vec![
            item("DEF-001", &["FEAT-001", "BUG-001"]),
            item("FTR-002", &[]),
        ]);
        for code in ["DEF-001", "FEAT-001", "BUG-001"] {
            assert_eq!(find(&p, code).map(|f| f.code.as_str()), Some("DEF-001"));
            assert_eq!(current(&p, code), Some("DEF-001"));
            assert_eq!(p.feature(code).expect(code).code, "DEF-001");
        }
        assert_eq!(current(&p, "FTR-002"), Some("FTR-002"));
        assert!(find(&p, "FEAT-009").is_none());
        assert!(current(&p, "FEAT-009").is_none());
        assert!(matches!(
            p.feature("FEAT-009"),
            Err(CoreError::FeatureNotFound(c)) if c == "FEAT-009"
        ));

        let f = p.feature("FEAT-001").unwrap();
        assert!(f.answers_to("DEF-001") && f.answers_to("BUG-001"));
        assert!(!f.answers_to("FTR-002"));
        assert_eq!(
            f.codes().collect::<Vec<_>>(),
            ["DEF-001", "FEAT-001", "BUG-001"]
        );

        assert_eq!(
            taken(&p).into_iter().collect::<Vec<_>>(),
            ["BUG-001", "DEF-001", "FEAT-001", "FTR-002"]
        );
        assert!(check_free(&p, "FEAT-003", None).is_ok());
        let is_code = check_free(&p, "FTR-002", None).unwrap_err().to_string();
        assert!(is_code.contains("FTR-002 is FTR-002's code"), "{is_code}");
        assert!(!is_code.contains("already exists"), "{is_code}");
        let was = check_free(&p, "FEAT-001", None).unwrap_err().to_string();
        assert!(was.contains("was an earlier code of DEF-001"), "{was}");
        assert!(check_free(&p, "FEAT-001", Some("DEF-001")).is_ok());
        assert!(check_free(&p, "FEAT-001", Some("FTR-002")).is_err());
    }
    /// R-4: what was written under an earlier code counts for the item, and is left as written.
    #[test]
    fn records_naming_an_earlier_code_count_for_the_item() {
        use crate::store::Store;
        let dir =
            std::env::temp_dir().join(format!("kanbanr-aliases-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        store.add_feature("demo", "A", "", "M", None).unwrap();
        store.add_feature("demo", "B", "", "M", None).unwrap();

        // Records made under FEAT-001, before the rename.
        crate::activity::append(&dir, "demo", "Ada", "did a thing", Some("FEAT-001"));
        crate::activity::append(&dir, "demo", "Ada", "did another", Some("FEAT-002"));
        let manual = crate::manual::ManualResult {
            code: "FEAT-001".into(),
            requirement: "R-1".into(),
            test: "t".into(),
            state: "green".into(),
            by: "Ada".into(),
            at: "2026-11-17T10:00:00Z".into(),
            ..Default::default()
        };
        crate::manual::record(&store, "demo", &manual).unwrap();
        let prepared = crate::prepared::Prepared {
            kind: "signoff".into(),
            subject: crate::prepared::Subject {
                item: "FEAT-001".into(),
                ..Default::default()
            },
            at: "2026-11-17T10:00:00Z".into(),
            ..Default::default()
        };
        crate::prepared::write_record(&store, "demo", &prepared).unwrap();
        let snapshot = |sub: &str| -> Vec<(String, Vec<u8>)> {
            let mut out = Vec::new();
            let mut stack = vec![dir.join("projects/demo").join(sub)];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        out.push((p.display().to_string(), std::fs::read(&p).unwrap()));
                    }
                }
            }
            out.sort();
            out
        };
        let (a0, m0, p0) = (
            snapshot("activity"),
            snapshot("manual"),
            snapshot("prepared"),
        );
        assert!(!a0.is_empty() && !m0.is_empty() && !p0.is_empty());

        store
            .edit_feature("demo", "FEAT-001", None, None, None, Some("DEF-001".into()))
            .unwrap();
        assert_eq!(snapshot("activity"), a0, "activity is kept as written");
        assert_eq!(snapshot("manual"), m0);
        assert_eq!(snapshot("prepared"), p0);

        // Activity: either code asks for the item's entries, and not the other item's.
        for code in ["DEF-001", "FEAT-001"] {
            let got = crate::activity::read_filtered(
                &store,
                "demo",
                &crate::activity::Filter {
                    item: Some(code.into()),
                    ..Default::default()
                },
            );
            assert_eq!(got.len(), 1, "{code}: {got:?}");
            assert_eq!(got[0].message, "did a thing");
        }
        // Manual results.
        for code in ["DEF-001", "FEAT-001"] {
            assert_eq!(
                crate::manual::results(&store, "demo", Some(code))
                    .unwrap()
                    .len(),
                1
            );
        }
        assert!(
            crate::manual::results(&store, "demo", Some("FEAT-002"))
                .unwrap()
                .is_empty()
        );
        // Prepared records.
        for code in ["DEF-001", "FEAT-001"] {
            let q = crate::prepared::ListQuery {
                all: true,
                item: Some(code.into()),
                release: None,
            };
            assert_eq!(
                crate::prepared::list(&store, "demo", &q, None)
                    .unwrap()
                    .len(),
                1,
                "{code}"
            );
        }

        let mut cadence = store.load("demo").unwrap().config.cadence;
        cadence.releases = true;
        store.set_cadence("demo", None, cadence).unwrap();
        let project = store.load("demo").unwrap();
        let item = project.feature("DEF-001").unwrap();
        // A shipped list written under the old code.
        let release = crate::releases::Release {
            version: "v1.0.0".into(),
            shipped: vec!["FEAT-001".into()],
            ..Default::default()
        };
        assert_eq!(
            crate::releases::release_line(&project, &[release], item).as_deref(),
            Some("Released in v1.0.0")
        );
        assert!(
            item.answers_to("FEAT-001")
                && !project.feature("FEAT-002").unwrap().answers_to("FEAT-001")
        );
        // A scope issued under FEAT-001 covers DEF-001.
        let mut scope = crate::agentscope::Scope {
            id: "SC-001".into(),
            project: "demo".into(),
            role: crate::agentscope::Role::Build,
            item: "FEAT-001".into(),
            operations: crate::agentscope::Role::Build.operations(),
            branch: None,
            base: None,
            task: None,
            issued_by: "Ada".into(),
            issued_at: "2026-11-17T09:00:00Z".into(),
            expires_at: "2026-11-17T17:00:00Z".into(),
            note: String::new(),
            worktree: None,
            agent_branch: None,
            claimed_at: None,
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            extra: Default::default(),
        };
        assert!(!scope.allows(crate::agentscope::Operation::TaskState, "DEF-001"));
        scope = scope.resolved(&project);
        assert_eq!(scope.item, "DEF-001");
        assert!(scope.allows(crate::agentscope::Operation::TaskState, "DEF-001"));
        // A delegation that names FEAT-001 covers DEF-001.
        let d = crate::decisions::Delegation {
            id: "DLG-001".into(),
            verdicts: vec!["approve".into()],
            items: vec!["FEAT-001".into()],
            until: "2099-01-01T00:00:00Z".into(),
            granted: Some(Default::default()),
            ..Default::default()
        };
        let verdict = crate::decisions::Verdict {
            kind: crate::decisions::VerdictKind::Approve,
            code: "DEF-001".into(),
            signoff: String::new(),
        };
        assert!(
            d.covers(&verdict, item, "2026-11-17T10:00:00Z"),
            "the grant's item list holds an earlier code of the item"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A project whose kinds map as this board's do: bug and defect `DEF`, chore `MNT`, feature
    /// `FTR`.
    fn kinded(items: Vec<(&str, Option<&str>, &str)>) -> Project {
        let mut p = project(Vec::new());
        p.config.codes = p.config.codes.apply(&crate::config::CodesChange {
            set: [("feature", "FTR"), ("bug", "DEF"), ("chore", "MNT")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        });
        for (code, kind, at) in items {
            let mut f = item(code, &[]);
            f.kind = kind.map(str::to_string);
            f.created_at = at.to_string();
            p.features.push(f);
        }
        p
    }

    /// R-6: each mismatched item gets its kind's next code, in creation order, skipping every
    /// number taken (current or earlier); the rev repeats for the same board.
    #[test]
    fn the_recode_plan_gives_each_mismatched_item_its_kinds_next_code() {
        let mut p = kinded(vec![
            ("FEAT-003", Some("bug"), "2026-11-03T00:00:00Z"),
            ("FEAT-001", Some("chore"), "2026-11-01T00:00:00Z"),
            ("FEAT-002", Some("bug"), "2026-11-02T00:00:00Z"),
            ("FEAT-004", Some("feature"), "2026-11-04T00:00:00Z"),
            ("DEF-001", Some("bug"), "2026-11-05T00:00:00Z"),
            ("FTR-007", None, "2026-11-06T00:00:00Z"),
            ("MNT-002", Some("chore"), "2026-11-07T00:00:00Z"),
        ]);
        // DEF-002 was given up by an item that is now an FTR: its number stays retired.
        p.features
            .iter_mut()
            .find(|f| f.code == "FTR-007")
            .unwrap()
            .aliases
            .push("DEF-002".into());
        let plan = recode_plan(&p);
        let pairs: Vec<(&str, &str)> = plan
            .renames
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("FEAT-001", "MNT-003"),
                ("FEAT-002", "DEF-003"),
                ("FEAT-003", "DEF-004"),
                ("FEAT-004", "FTR-008"),
            ]
        );
        assert_eq!(plan.renames[0].kind.as_deref(), Some("chore"));
        assert_eq!(plan.renames[0].title, "FEAT-001");
        assert_eq!(
            recode_plan(&p).rev,
            plan.rev,
            "the same board, the same rev"
        );
        // A different board has a different rev.
        p.features.push({
            let mut f = item("FEAT-005", &[]);
            f.kind = Some("bug".into());
            f
        });
        assert_ne!(recode_plan(&p).rev, plan.rev);
    }

    #[test]
    fn the_recode_plan_is_empty_when_every_code_fits() {
        let p = kinded(vec![
            ("DEF-001", Some("bug"), "2026-11-01T00:00:00Z"),
            ("MNT-001", Some("chore"), "2026-11-02T00:00:00Z"),
            ("FTR-001", None, "2026-11-03T00:00:00Z"),
        ]);
        let plan = recode_plan(&p);
        assert!(plan.renames.is_empty());
        assert_eq!(plan.rev, recode_plan(&p).rev);
        assert!(recode_plan(&project(Vec::new())).renames.is_empty());
    }

    /// Copy the shipped example board into a scratch folder under the temp dir (L-41), so loading
    /// it can never touch a real board.
    fn example_board(name: &str, strip_aliases: bool) -> std::path::PathBuf {
        fn copy(from: &std::path::Path, to: &std::path::Path, strip: bool) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let (src, dst) = (e.path(), to.join(e.file_name()));
                if src.is_dir() {
                    copy(&src, &dst, strip);
                } else if src.extension().is_some_and(|x| x == "yaml") && strip {
                    let text = std::fs::read_to_string(&src).unwrap();
                    let mut out = String::new();
                    let mut in_aliases = false;
                    for line in text.lines() {
                        if line == "aliases:" {
                            in_aliases = true;
                        } else if in_aliases && line.starts_with("- ") {
                        } else {
                            in_aliases = false;
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                    std::fs::write(&dst, out).unwrap();
                } else {
                    std::fs::copy(&src, &dst).unwrap();
                }
            }
        }
        let dir =
            std::env::temp_dir().join(format!("kanbanr-aliases-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/aliases-v1");
        copy(&src, &dir, strip_aliases);
        dir
    }

    /// R-8: each reference in the shipped example resolves as the documented rules say. A
    /// reference is `kanbanr:<CODE>[/R-n | /TL-n/Tn]`, `<CODE> R-n`, `/p/<project>/feature/<CODE>`
    /// or `<project>:<CODE>`; the code in it may be an earlier code.
    #[test]
    fn the_contract_example_resolves_as_documented() {
        let dir = example_board("contract", false);
        let store = crate::store::Store::new(dir.clone());
        let project = store.load("demo").unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/testdata/aliases-v1/expected.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let code_of = |text: &str| -> String {
            let t = text
                .strip_prefix("kanbanr:")
                .or_else(|| text.rsplit_once("/feature/").map(|(_, c)| c))
                .map(str::to_string)
                .unwrap_or_else(|| text.to_string());
            let t = t.rsplit_once(':').map_or(t.as_str(), |(_, c)| c);
            t.split(['/', ' ']).next().unwrap().to_string()
        };
        let refs = expected["references"].as_object().unwrap();
        assert!(refs.len() >= 10, "the example covers every form");
        for (text, want) in refs {
            let got = current(&project, &code_of(text));
            assert_eq!(got, want.as_str(), "{text}");
        }
        // Rule 1: every code and earlier code is distinct.
        let all: Vec<String> = project
            .features
            .iter()
            .flat_map(|f| f.codes().map(str::to_string))
            .collect();
        assert_eq!(all.len(), taken(&project).len());
        // Rule 5: the spec file is named by the current code.
        assert!(
            dir.join("projects/demo/features/Planned/features-spec/DEF-095.md")
                .is_file()
        );
        assert_eq!(
            project.feature("BUG-001").unwrap().aliases,
            ["FEAT-001", "BUG-001"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-10: no schema bump. The example is stamped with the version a board without aliases has,
    /// and loads; so does a copy with every `aliases` removed, as a 0.7.0 board reads.
    #[test]
    fn an_aliased_board_needs_no_new_schema_version() {
        assert_eq!(
            ProjectConfig::default_for("demo").required_schema_version(),
            5
        );
        for (strip, name) in [(false, "schema-with"), (true, "schema-without")] {
            let dir = example_board(name, strip);
            let store = crate::store::Store::new(dir.clone());
            let project = store.load("demo").unwrap();
            assert_eq!(project.config.schema_version, 5);
            assert_eq!(
                project.config.schema_version,
                project.config.required_schema_version()
            );
            assert_eq!(project.features.len(), 2);
            let with = project.features.iter().any(|f| !f.aliases.is_empty());
            assert_eq!(with, !strip, "{name}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
