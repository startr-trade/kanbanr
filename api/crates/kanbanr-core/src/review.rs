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
            let signoffs_needed: Vec<String> = next
                .filter(|n| n.requires_gaps.is_empty())
                .map(|n| n.signoffs_needed)
                .unwrap_or_default();
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
            Some(ReviewEntry {
                changes,
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
                spec_rev,
                spec,
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
