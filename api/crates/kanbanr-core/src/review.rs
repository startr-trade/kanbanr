//! The review queue (FEAT-067): what is waiting for a person's agreement, in the order to ask it.
//! `GET /projects/{p}/review` answers with [`pending`]; its entries are [`ReviewEntry`] (FEAT-177).

use crate::Store;
use crate::error::Result;
use crate::models::{ApprovalState, FeatureDefinition};
use serde::Serialize;

/// The question a review entry asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ApprovalKind {
    /// Finished under a recorded bypass and never agreed to (FEAT-109): ratify it.
    Unratified,
    /// Agreed; the stage it reaches next waits on a sign-off (FEAT-117).
    Signoff,
    /// Agreed, and nothing to sign: which release it ships in (FEAT-185).
    Release,
    /// Defined, never approved.
    Missing,
    /// Approved, then the definition changed.
    Lapsed,
}

/// One item waiting for agreement.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReviewEntry {
    pub code: String,
    pub title: String,
    pub status: String,
    /// What is being asked.
    pub approval: ApprovalKind,
    /// The sign-offs the next stage still needs, by name; empty unless `approval` is `signoff`.
    pub signoffs_needed: Vec<String>,
    pub started_unapproved: String,
    pub definition: FeatureDefinition,
    /// What a verdict given from this brief is pinned to (FEAT-159).
    pub rev: String,
    /// What the ordering keys on, and useful to a client besides.
    pub in_progress: bool,
    /// The item is finished work (DEF-013): listed only because its approval lapsed.
    pub finished: bool,
    /// The planned releases it could ship in, in file order (FEAT-185). Present whenever the
    /// release question is due, so a surface asking another question first can ask this one too.
    /// Empty: only a new release to choose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_choices: Option<Vec<String>>,
}

/// The review queue for project `p`.
pub fn pending(store: &Store, p: &str) -> Result<Vec<ReviewEntry>> {
    let project = store.load_for_gates(p)?;
    let charter = crate::charter::load(store, p)?;
    let ctx = crate::readiness::Context::load(store, p, &project.config);
    // The planned releases, in file order: the choices of the which-release question
    // (FEAT-185). Only loaded when the project works in releases.
    let release_choices: Vec<String> = if project.config.cadence.releases {
        crate::releases::load(store, p)?
            .into_iter()
            .filter(|r| r.state == crate::releases::ReleaseState::Planned)
            .map(|r| r.version)
            .collect()
    } else {
        Vec::new()
    };
    let gates = project.config.effective_gates();
    // Whether an approval is current or has lapsed depends on a hash of the definition's
    // content, so it is decided here rather than in each caller: the CLI, the monitor and
    // any future client all get the same answer to "is this agreed?".
    let mut pending: Vec<ReviewEntry> = project
        .features
        .iter()
        // Only where agreement can still change what happens (FEAT-078). Approving a
        // Completed item that was never agreed to records a signature, pinned to a definition
        // hash, for work already merged; a deliberately Deferred one is not going to start.
        // The same gate the doctor applies, so the two surfaces cannot give different answers
        // to one question. The exception is finished work whose approval lapsed (DEF-013): it
        // was agreed once and the text has changed since, which nothing else on the board shows.
        .filter_map(|f| {
            let definition = f.definition.as_ref()?;
            let live = crate::graph::is_live_work(&project.config, &f.status);
            // Work finished under a recorded bypass and never agreed to is still a
            // question for a person, however done it is — the one the doctor reports.
            // It is asked here too (FEAT-109), with the predicate the doctor uses, so the
            // two surfaces cannot disagree about which items need a ratification.
            let unratified = !live && crate::doctor::unreconciled_bypass(definition);
            let finished_lapsed = !live
                && !unratified
                && crate::graph::is_finished_work(&project.config, &f.status)
                && definition.approval_state() == ApprovalState::Lapsed;
            if !live && !unratified && !finished_lapsed {
                return None;
            }
            // Sign-offs the next stage asks for are the same kind of question: a person's
            // agreement, recorded before the item can move on (FEAT-117). Only the stage
            // the item reaches next, not every stage it could skip to, and only once that
            // stage's other requirements are met (FEAT-182): a sign-off offered before the
            // work exists records agreement to work that has not happened.
            let next = if live {
                crate::readiness::immediate_next_gate(&project, Some(&charter), f, &ctx)
            } else {
                None
            };
            // Which release it ships in (FEAT-185): asked of live work not yet planned
            // into one, once the stage it reaches next is the end, or asks for a release.
            let asks_release = project.config.cadence.releases
                && f.release.as_deref().is_none_or(|r| r.trim().is_empty())
                && next.as_ref().is_some_and(|n| {
                    project.config.is_terminal(&n.status)
                        || gates.get(&n.status).is_some_and(|g| {
                            g.requires.contains(&crate::readiness::Condition::Check(
                                crate::readiness::Check::InRelease,
                            ))
                        })
                });
            let signoffs_needed: Vec<String> = next
                .filter(|n| n.requires_gaps.is_empty())
                .map(|n| n.signoffs_needed)
                .unwrap_or_default();
            let approval = match definition.approval_state() {
                _ if unratified => ApprovalKind::Unratified,
                // Agreed is agreed, however late — a ratified item is not still a
                // decision waiting to be made, so it leaves the queue (FEAT-080) — unless
                // its next stage is waiting on a sign-off.
                ApprovalState::Current | ApprovalState::Ratified if !signoffs_needed.is_empty() => {
                    ApprovalKind::Signoff
                }
                // Agreed, and nothing to sign: the one question left is the release.
                ApprovalState::Current | ApprovalState::Ratified if asks_release => {
                    ApprovalKind::Release
                }
                ApprovalState::Current | ApprovalState::Ratified => return None,
                ApprovalState::Missing => ApprovalKind::Missing,
                ApprovalState::Lapsed => ApprovalKind::Lapsed,
            };
            Some(ReviewEntry {
                code: f.code.clone(),
                title: f.title.clone(),
                status: f.status.clone(),
                approval,
                signoffs_needed,
                started_unapproved: definition.started_unapproved.clone(),
                definition: definition.clone(),
                rev: definition.content_rev(),
                in_progress: f.status != project.config.default_state,
                finished: !live,
                release_choices: asks_release.then(|| release_choices.clone()),
            })
        })
        .collect();
    // Work already built without agreement comes first (FEAT-109), then work being built
    // without it, then work that has not started (FEAT-078 R-2). Finished work whose approval
    // lapsed follows all live decisions (DEF-013). The release question comes after every
    // approval and sign-off (FEAT-185).
    pending.sort_by_key(|e| {
        (
            e.approval == ApprovalKind::Release,
            e.approval != ApprovalKind::Unratified,
            e.finished,
            !e.in_progress,
            e.code.clone(),
        )
    });
    Ok(pending)
}

#[cfg(test)]
mod perf {
    //! DEF-022: the review queue on a board whose history and doc references have grown.
    use super::*;
    use crate::models::FeatureDefinition;
    use crate::readiness::{Check, Condition};

    /// The first commit's time and the gap between commits, in seconds.
    const T0: i64 = 1_700_000_000;
    const STEP: i64 = 60;
    const DOCS: usize = 40;

    fn rfc3339(secs: i64) -> String {
        use time::format_description::well_known::Rfc3339;
        time::OffsetDateTime::from_unix_timestamp(secs)
            .unwrap()
            .format(&Rfc3339)
            .unwrap()
    }

    /// A board in a temp git repository: `items` items waiting at the first stage, each declaring
    /// `refs_per_item` of `DOCS` docs, with a gate on the next stage that asks for `docs_updated`,
    /// and `commits` commits to the docs, made with git2 at increasing times. Each item's entry time
    /// is spread across that history.
    fn generated_board(
        commits: usize,
        items: usize,
        refs_per_item: usize,
    ) -> (crate::Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kanbanr-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Scheduled".to_string(),
            crate::config::Gate {
                warns: vec![Condition::Check(Check::DocsUpdated)],
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        let docs = store.project_dir("demo").join("docs").join("architecture");
        std::fs::create_dir_all(&docs).unwrap();
        for d in 0..DOCS {
            std::fs::write(docs.join(format!("d{d}.md")), "v0").unwrap();
        }
        let span = (commits as i64 * STEP).max(1);
        for i in 0..items {
            let item = store
                .add_feature("demo", &format!("Item {i}"), "", "M", None)
                .unwrap();
            let affects_docs = (0..refs_per_item)
                .map(|j| format!("architecture/d{}.md", (i * refs_per_item + j) % DOCS))
                .collect();
            store
                .set_feature_definition(
                    "demo",
                    &item.code,
                    Some(FeatureDefinition {
                        statement: format!("Item {i}"),
                        affects_docs,
                        ..Default::default()
                    }),
                )
                .unwrap();
            // Entry times from before the first commit to the last, spread evenly.
            let entered = rfc3339(T0 - STEP + span * i as i64 / items as i64);
            let path = store.feature_path("demo", &item.status, &item.code);
            let text = std::fs::read_to_string(&path).unwrap();
            let text: Vec<String> = text
                .lines()
                .map(|l| {
                    if l.starts_with("created_at:") {
                        format!("created_at: {entered}")
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            std::fs::write(&path, text.join("\n") + "\n").unwrap();
        }

        let repo = git2::Repository::open(&dir).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        // The board's own first commit stays the root; the generated history grows from it.
        let mut parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        for i in 0..commits {
            let rel = store
                .project_dir("demo")
                .join("docs")
                .join(format!("architecture/d{}.md", i % DOCS));
            std::fs::write(&rel, format!("v{}", i + 1)).unwrap();
            let rel = rel
                .strip_prefix(repo.workdir().unwrap())
                .unwrap()
                .to_owned();
            index.add_path(&rel).unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::new(
                "Ada",
                "ada@example.com",
                &git2::Time::new(T0 + i as i64 * STEP, 0),
            )
            .unwrap();
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, "docs", &tree, &parents)
                .unwrap();
            parent = Some(repo.find_commit(oid).unwrap());
        }
        index.write().unwrap();
        (store, dir)
    }

    /// The most the queue may take on the generated board: under a third of the 15.0 s measured
    /// before DEF-022, and under a second.
    const BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

    /// DEF-022 R-4: the review queue on 2,500 commits, 70 items and 490 doc references.
    /// Run: `cargo test -p kanbanr-core --release review::perf -- --ignored --nocapture`
    #[test]
    #[ignore = "a timing measure; run it on its own in release"]
    fn review_on_a_board_with_many_commits_and_doc_refs() {
        let (store, dir) = generated_board(2_500, 70, 7);
        let started = std::time::Instant::now();
        let queue = pending(&store, "demo").unwrap();
        let elapsed = started.elapsed();
        eprintln!("review::pending: {} entries in {elapsed:?}", queue.len());
        assert_eq!(queue.len(), 70);
        assert!(
            elapsed <= BUDGET,
            "{elapsed:?} is over the budget {BUDGET:?}"
        );

        // The shared history answers as the walk of each doc alone does.
        let project = store.load("demo").unwrap();
        let docs = store.project_dir("demo").join("docs");
        let history = crate::git::DocHistory::default();
        for f in &project.features {
            let since = f.entered_status_at();
            for doc in &f.definition.as_ref().unwrap().affects_docs {
                let path = docs.join(crate::readiness::affects_doc_key(doc));
                assert_eq!(
                    history.changed_since(&path, since),
                    crate::git::doc_changed_since_single_walk(&path, since),
                    "{} {doc}",
                    f.code
                );
            }
        }
        eprintln!("history: {:?}", history.stats());
        assert_eq!(
            history.stats(),
            crate::git::HistoryStats { opens: 1, walks: 1 }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
