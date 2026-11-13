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
    /// Approved, then the definition gained parts that now need a decision (FTR-018): the item is
    /// at or past its build stage, or its next stage would refuse the additions.
    Added,
    /// The spec a sign-off pinned changed after it was given, and the item has passed the gate
    /// that asks for it (FTR-023).
    #[serde(rename = "spec_changed")]
    SpecChanged,
}

/// How a line stands in a [`line_diff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DiffOp {
    Same,
    Added,
    Removed,
}

/// One line of a [`line_diff`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DiffLine {
    pub op: DiffOp,
    pub text: String,
}

/// The spec a sign-off pins, as the person is shown it (FTR-023).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SpecBrief {
    /// The sign-off that pins it.
    pub signoff: String,
    /// The spec's current text, normalised as it is pinned.
    pub text: String,
    /// False when an earlier sign-off pinned the spec but its text is missing from the board.
    /// True otherwise, including a first sign-off with nothing earlier to keep.
    pub previous_kept: bool,
    /// Who gave the earlier sign-off, and when; empty on a first sign-off.
    pub signed_by: String,
    pub signed_at: String,
    /// The current text against the kept one; empty on a first sign-off, when the earlier text
    /// was not kept, or when nothing differs.
    pub diff: Vec<DiffLine>,
}

/// Above this many cells (`old lines x new lines`) [`line_diff`] stops looking for common lines.
const DIFF_CELL_CAP: usize = 2_000_000;

/// A line diff of `new` against `old`: the longest common run of lines kept as `Same`. Past
/// [`DIFF_CELL_CAP`] cells it gives up and returns every old line removed and every new one added,
/// which bounds the work for a pathological spec.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let line = |op, text: &str| DiffLine {
        op,
        text: text.to_string(),
    };
    if a.len().saturating_mul(b.len()) > DIFF_CELL_CAP {
        return a
            .iter()
            .map(|l| line(DiffOp::Removed, l))
            .chain(b.iter().map(|l| line(DiffOp::Added, l)))
            .collect();
    }
    // lcs[i][j]: the common length of a[i..] and b[j..], row-major.
    let w = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * w];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * w + j] = if a[i] == b[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len().max(b.len()));
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(line(DiffOp::Same, a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            out.push(line(DiffOp::Removed, a[i]));
            i += 1;
        } else {
            out.push(line(DiffOp::Added, b[j]));
            j += 1;
        }
    }
    out
}

/// What the Review entry says when no code repository is attached (FTR-013 R-9).
pub const DRIFT_SKIPPED: &str =
    "the drift check needs the code repository: start the monitor from the project folder";

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
    /// `docs-folded` is among them and the item lists no documents: the sign-off confirms that it
    /// changes none (DEF-033).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changes_no_documents: bool,
    /// `docs-folded` is among them: the texts that name something the item changed since they
    /// last changed, for the person to confirm or correct (FTR-013). `None` when the check was
    /// skipped (see `drift_skipped`) or the sign-off is not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_check: Option<Vec<crate::docdrift::Flag>>,
    /// Why the drift check did not run: no code repository is attached. The sign-off is offered
    /// all the same, and this is never read as "no drift" (FTR-013).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_skipped: Option<String>,
    pub started_unapproved: String,
    pub definition: FeatureDefinition,
    /// What a verdict given from this brief is pinned to (FEAT-159).
    pub rev: String,
    /// What the ordering keys on, and useful to a client besides.
    pub in_progress: bool,
    /// The item is finished work (DEF-013): listed only because its approval lapsed.
    pub finished: bool,
    /// The parts added, changed and removed since the last decision (FTR-018); present on
    /// `lapsed`, `added` and `signoff` entries that have a baseline to compare with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<crate::models::PartChanges>,
    /// The planned releases it could ship in, in file order (FEAT-185). Present whenever the
    /// release question is due, so a surface asking another question first can ask this one too.
    /// Empty: only a new release to choose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_choices: Option<Vec<String>>,
    /// The hash of the spec's current text, when the sign-off asked pins the spec (FTR-023): what
    /// the verdict is pinned to, sent back with it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spec_rev: String,
    /// The spec the sign-off asked pins, with what changed since the last sign-off (FTR-023).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<SpecBrief>,
    /// Claude's overrides on the item that wait for the person to ratify (DEF-043), oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<OverrideView>,
}

/// One override Claude made, as the Review page shows it (DEF-043).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OverrideView {
    pub at: String,
    pub from: String,
    pub to: String,
    pub reason: String,
    pub origin: String,
}

/// The spec a sign-off `name` pins, for the brief: its current text, and the diff against the text
/// kept at the last sign-off that pinned it (FTR-023).
fn spec_brief(
    store: &Store,
    p: &str,
    f: &crate::models::FeatureItem,
    definition: &FeatureDefinition,
    name: &str,
) -> (String, SpecBrief) {
    let (text, rev) = crate::readiness::spec_pin(&f.specification);
    let previous = definition
        .signoffs
        .iter()
        .rev()
        .find(|s| s.id == name && s.verdict.is_given() && !s.spec_rev.is_empty());
    let mut brief = SpecBrief {
        signoff: name.to_string(),
        text: text.clone(),
        previous_kept: true,
        signed_by: String::new(),
        signed_at: String::new(),
        diff: Vec::new(),
    };
    if let Some(prev) = previous {
        brief.signed_by = prev.by.clone();
        brief.signed_at = prev.at.clone();
        match store.spec_pin_text(p, &f.code, &prev.spec_rev) {
            Some(old) => {
                let diff = line_diff(&old, &text);
                if diff.iter().any(|l| l.op != DiffOp::Same) {
                    brief.diff = diff;
                }
            }
            None => brief.previous_kept = false,
        }
    }
    (rev, brief)
}

/// The review queue for project `p`.
///
/// `repo` is the project's code repository when the caller has one: the repository docs an item
/// lists are judged against it before `docs-folded` is offered (DEF-033).
pub fn pending(store: &Store, p: &str, repo: Option<&std::path::Path>) -> Result<Vec<ReviewEntry>> {
    let project = store.load_for_gates(p)?;
    let charter = crate::charter::load(store, p)?;
    let ctx = crate::readiness::Context::load_in(store, p, &project.config, repo);
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
            // An override Claude made waits too, live or finished (DEF-043).
            let open_overrides = crate::readiness::open_overrides(f);
            let unratified =
                crate::doctor::awaiting_ratification(f) && (!live || !open_overrides.is_empty());
            let finished_lapsed = !live
                && !unratified
                && crate::graph::is_finished_work(&project.config, &f.status)
                && matches!(
                    definition.approval_state(),
                    ApprovalState::Lapsed | ApprovalState::Extended
                );
            // A sign-off that pinned the spec, which then changed, for an item that has passed the
            // gate asking for it (FTR-023). Finished work is admitted as DEF-013 admits a lapsed
            // approval: nothing else on the board shows the change.
            let spec_changed: Option<String> = {
                let position = |s: &str| project.config.statuses.iter().position(|x| x == s);
                let here = position(&f.status);
                definition
                    .signoffs
                    .iter()
                    .map(|s| s.id.as_str())
                    .filter(|name| {
                        gates.iter().any(|(status, g)| {
                            g.signoffs.iter().any(|s| s == name)
                                && matches!((position(status), here), (Some(t), Some(h)) if t <= h)
                        })
                    })
                    .find(|name| f.signoff_spec_changed(name).is_some())
                    .map(String::from)
            };
            let finished_spec_changed = !live
                && !unratified
                && spec_changed.is_some()
                && crate::graph::is_finished_work(&project.config, &f.status);
            if !live && !unratified && !finished_lapsed && !finished_spec_changed {
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
            // Text added to an approval needs a decision at once from the build stage on (a
            // finished item is always past it), or when the next stage would refuse it
            // (FTR-018). Before that the additions wait inside the sign-off that stage asks for.
            let extended = definition.approval_state() == ApprovalState::Extended;
            let additions_due = extended
                && (!live
                    || ctx.builds(&f.status)
                    || next.as_ref().is_some_and(|n| {
                        n.requires_gaps
                            .iter()
                            .any(|g| g.label == "added since approval")
                    }));
            let next = next.filter(|n| n.requires_gaps.is_empty());
            let changes_no_documents = next.as_ref().is_some_and(|n| {
                n.signoffs_needed
                    .iter()
                    .any(|s| s == crate::method::DOCS_FOLDED)
                    && n.docs_built
                        .as_ref()
                        .is_some_and(|d| d.changes_no_documents)
            });
            let signoffs_needed: Vec<String> = next.map(|n| n.signoffs_needed).unwrap_or_default();
            let approval = match definition.approval_state() {
                _ if unratified => ApprovalKind::Unratified,
                // Agreed is agreed, however late — a ratified item is not still a
                // decision waiting to be made, so it leaves the queue (FEAT-080) — unless
                // its next stage is waiting on a sign-off.
                ApprovalState::Current | ApprovalState::Ratified | ApprovalState::Extended
                    if !signoffs_needed.is_empty() =>
                {
                    ApprovalKind::Signoff
                }
                // The design was signed off, then the spec moved on (FTR-023). Asked after the
                // next stage's sign-off and before the definition's additions: one signature on
                // the page settles both.
                ApprovalState::Current | ApprovalState::Ratified | ApprovalState::Extended
                    if spec_changed.is_some() =>
                {
                    ApprovalKind::SpecChanged
                }
                ApprovalState::Extended if additions_due => ApprovalKind::Added,
                // Agreed, and nothing to sign: the one question left is the release.
                ApprovalState::Current | ApprovalState::Ratified | ApprovalState::Extended
                    if asks_release =>
                {
                    ApprovalKind::Release
                }
                ApprovalState::Current | ApprovalState::Ratified | ApprovalState::Extended => {
                    return None;
                }
                ApprovalState::Missing => ApprovalKind::Missing,
                ApprovalState::Lapsed => ApprovalKind::Lapsed,
            };
            // The first sign-off asked that pins the spec brings the spec's text (FTR-023).
            let (spec_rev, spec) = match approval {
                ApprovalKind::Signoff => signoffs_needed
                    .iter()
                    .find(|n| project.config.signoff_pins_spec(n))
                    .map(|n| spec_brief(store, p, f, definition, n)),
                ApprovalKind::SpecChanged => spec_changed
                    .as_deref()
                    .map(|n| spec_brief(store, p, f, definition, n)),
                _ => None,
            }
            .map_or((String::new(), None), |(rev, brief)| (rev, Some(brief)));
            let signoffs_needed = if approval == ApprovalKind::SpecChanged {
                spec_changed.clone().into_iter().collect()
            } else {
                signoffs_needed
            };
            let changes = matches!(
                approval,
                ApprovalKind::Lapsed | ApprovalKind::Added | ApprovalKind::Signoff
            )
            .then(|| definition.part_changes())
            .flatten();
            let (to_check, drift_skipped) = if signoffs_needed
                .iter()
                .any(|s| s == crate::method::DOCS_FOLDED)
            {
                match ctx.code_repo.as_deref() {
                    None => (None, Some(DRIFT_SKIPPED.to_string())),
                    Some(repo) => match crate::docdrift::flags_for_item(store, p, &f.code, repo) {
                        Ok(flags) => (Some(flags), None),
                        Err(e) => (None, Some(format!("the drift check failed: {e}"))),
                    },
                }
            } else {
                (None, None)
            };
            Some(ReviewEntry {
                to_check,
                drift_skipped,
                changes,
                code: f.code.clone(),
                title: f.title.clone(),
                status: f.status.clone(),
                approval,
                changes_no_documents: changes_no_documents
                    && signoffs_needed
                        .iter()
                        .any(|s| s == crate::method::DOCS_FOLDED),
                signoffs_needed,
                started_unapproved: definition.started_unapproved.clone(),
                definition: definition.clone(),
                rev: definition.content_rev(),
                in_progress: f.status != project.config.default_state,
                finished: !live,
                release_choices: asks_release.then(|| release_choices.clone()),
                spec_rev,
                spec,
                overrides: open_overrides
                    .iter()
                    .map(|t| OverrideView {
                        at: t.at.clone(),
                        from: t.from.clone(),
                        to: t.to.clone(),
                        reason: t.override_reason.clone().unwrap_or_default(),
                        origin: t.origin.clone(),
                    })
                    .collect(),
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
mod tests {
    use super::*;

    fn ops(d: &[DiffLine]) -> String {
        d.iter()
            .map(|l| match l.op {
                DiffOp::Same => format!(" {}", l.text),
                DiffOp::Added => format!("+{}", l.text),
                DiffOp::Removed => format!("-{}", l.text),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// FTR-023 R-5: a changed line is one removal and one addition, the rest unchanged.
    #[test]
    fn line_diff_marks_added_and_removed_lines() {
        assert_eq!(ops(&line_diff("a\nb\nc", "a\nb\nc")), " a| b| c");
        assert_eq!(ops(&line_diff("a\nb\nc", "a\nB\nc")), " a|-b|+B| c");
        assert_eq!(ops(&line_diff("a\nc", "a\nb\nc\nd")), " a|+b| c|+d");
        assert_eq!(ops(&line_diff("a\nb", "")), "-a|-b");
        assert_eq!(ops(&line_diff("", "a")), "+a");
        // Past the cell cap: every old line removed, every new line added.
        let old = (0..1500)
            .map(|i| format!("o{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..1500)
            .map(|i| format!("n{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = line_diff(&old, &new);
        assert_eq!(d.len(), 3000);
        assert!(d[..1500].iter().all(|l| l.op == DiffOp::Removed));
        assert!(d[1500..].iter().all(|l| l.op == DiffOp::Added));
    }
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
        let queue = pending(&store, "demo", None).unwrap();
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
