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
        // Completed item records a signature, pinned to a definition hash, for work already
        // merged; a deliberately Deferred one is not going to start. The same gate the
        // doctor applies, so the two surfaces cannot give different answers to one question.
        .filter_map(|f| {
            let definition = f.definition.as_ref()?;
            let live = crate::graph::is_live_work(&project.config, &f.status);
            // Work finished under a recorded bypass and never agreed to is still a
            // question for a person, however done it is — the one the doctor reports.
            // It is asked here too (FEAT-109), with the predicate the doctor uses, so the
            // two surfaces cannot disagree about which items need a ratification.
            let unratified = !live && crate::doctor::unreconciled_bypass(definition);
            if !live && !unratified {
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
                release_choices: asks_release.then(|| release_choices.clone()),
            })
        })
        .collect();
    // Work already built without agreement comes first (FEAT-109), then work being built
    // without it, then work that has not started (FEAT-078 R-2). The release question
    // comes after every approval and sign-off (FEAT-185).
    pending.sort_by_key(|e| {
        (
            e.approval == ApprovalKind::Release,
            e.approval != ApprovalKind::Unratified,
            !e.in_progress,
            e.code.clone(),
        )
    });
    Ok(pending)
}
