//! Portfolio-wide integrity scan (FEAT-037).
//!
//! `doctor` walks every project (or one project) and reports structural problems that the
//! mutating ops can't always prevent after the fact — e.g. a feature whose milestone was renamed
//! out from under it, a `depends_on` ref that no longer resolves, or a config written by an older
//! schema version. It is a **read**: it never mutates the store.
//!
//! Severities: an `Error` is a broken reference that should be fixed; a `Warning` is advisory
//! (currently only an outdated `schema_version`).

use crate::config::CURRENT_SCHEMA_VERSION;
use crate::error::Result;
use crate::graph::{parse_ref, qualify};
use crate::{Project, Store};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub project: String,
    /// The feature/entity code the issue concerns, when applicable.
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub issues: Vec<Issue>,
    /// What later stages will ask, counted per project and stage (DEF-021). Not warnings: they
    /// never change `has_errors` and are left out of the JSON when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ahead: Vec<Ahead>,
    /// Items already counted in `ahead`, so an item is one item per stage however many scans add to it.
    #[serde(skip)]
    ahead_seen: BTreeSet<(String, String, String)>,
}

/// How many items still lack something at the stage they reach next, and how many things in all.
#[derive(Debug, Clone, Serialize)]
pub struct Ahead {
    pub project: String,
    pub stage: String,
    pub items: usize,
    pub things: usize,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Count `things` an item lacks at `stage`, which it reaches next. The entries stay grouped by
    /// project and in the workflow's order of stages.
    fn count_ahead(&mut self, project: &Project, code: &str, stage: &str, things: usize) {
        if things == 0 {
            return;
        }
        let new_item =
            self.ahead_seen
                .insert((project.id.clone(), stage.to_owned(), code.to_owned()));
        if let Some(a) = self
            .ahead
            .iter_mut()
            .find(|a| a.project == project.id && a.stage == stage)
        {
            a.items += usize::from(new_item);
            a.things += things;
            return;
        }
        let position = |s: &str| project.config.statuses.iter().position(|x| x == s);
        let at = self
            .ahead
            .iter()
            .rposition(|a| a.project == project.id && position(&a.stage) <= position(stage))
            .map(|i| i + 1)
            .or_else(|| self.ahead.iter().position(|a| a.project == project.id))
            .unwrap_or(self.ahead.len());
        self.ahead.insert(
            at,
            Ahead {
                project: project.id.clone(),
                stage: stage.to_owned(),
                items: 1,
                things,
            },
        );
    }
}

/// Scan every project in the store.
pub fn run(store: &Store) -> Result<Report> {
    // The set of fully-qualified feature ids that exist anywhere, used to resolve cross-project
    // `depends_on` refs without re-loading per dependency.
    let mut report = Report::default();
    let ids = store.list_projects()?;

    // Build the universe of existing qualified feature ids first (one load each).
    let mut projects: Vec<Project> = Vec::new();
    for id in &ids {
        match store.load(id) {
            Ok(p) => projects.push(p),
            // A board from before canonical revisions cannot be scanned until it is migrated, and
            // saying nothing would read as healthy (FEAT-187).
            Err(e @ crate::CoreError::NeedsMigration { .. }) => report.issues.push(Issue {
                severity: Severity::Error,
                project: id.clone(),
                code: None,
                message: e.to_string(),
            }),
            Err(_) => {}
        }
    }
    let existing: BTreeSet<String> = projects
        .iter()
        .flat_map(|p| {
            // An earlier code still names its item (FTR-056).
            p.features
                .iter()
                .flat_map(|f| f.codes())
                .map(move |c| qualify(&p.id, c))
        })
        .collect();

    for project in &projects {
        scan_project(project, &existing, &mut report);
        let charter = crate::charter::load(store, &project.id)?;
        scan_charter(&charter, &project.id, &mut report);
        let ctx = crate::readiness::Context::load(store, &project.id, &project.config);
        scan_definitions(project, &charter, &ctx, &mut report);
        scan_decisions(store, project, &mut report);
        scan_stray_folders(store, project, &mut report);
        scan_leftover_writes(store, &project.id, &mut report);
        scan_process_drift(store, project, &mut report);
        scan_shipped_release_items(store, project, &charter, &ctx, &mut report);
        scan_release_items_outside_sprint(store, project, &mut report);
        scan_sprints_started_early(store, project, &mut report);
        if project.config.method.is_on(crate::method::Part::Links) {
            scan_references(store, project, &mut report);
        }
        scan_declared_docs(project, &charter, &ctx, &mut report);
        scan_coverage(store, project, &mut report);
        scan_catalog(store, project, &mut report);
        scan_diagrams(store, project, &mut report);
        scan_query_blocks(store, project, &mut report);
        scan_method_choice(project, &mut report);
        scan_repository(store, project, &mut report);
        // A task ref whose task is gone (FTR-060).
        report
            .issues
            .extend(crate::taskgraph::issues(store, &project.id));
    }
    scan_leftover_writes(store, "board", &mut report);
    scan_identity(store, "board", &mut report);
    scan_readme(store, "board", &mut report);
    scan_remote_lag(store, "board", &mut report);
    Ok(report)
}

/// Scan a single project (cross-project dependency resolution still consults the whole store, so
/// a dangling ref into another project is still detected).
pub fn run_project(store: &Store, id: &str) -> Result<Report> {
    run_project_in(store, id, None)
}

/// [`run_project`], with the project's code repository when the caller has one (FTR-004): the
/// specs' repository doc links are then checked at each item's position.
pub fn run_project_in(store: &Store, id: &str, repo: Option<&std::path::Path>) -> Result<Report> {
    let mut report = Report::default();
    let project = store.load(id)?;
    let mut existing: BTreeSet<String> = BTreeSet::new();
    for pid in store.list_projects()? {
        if let Ok(p) = store.load(&pid) {
            for f in &p.features {
                for c in f.codes() {
                    existing.insert(qualify(&p.id, c));
                }
            }
        }
    }
    scan_project(&project, &existing, &mut report);
    let charter = crate::charter::load(store, id)?;
    scan_charter(&charter, id, &mut report);
    let ctx = crate::readiness::Context::load(store, id, &project.config);
    scan_definitions(&project, &charter, &ctx, &mut report);
    scan_decisions(store, &project, &mut report);
    let adrs = crate::adr::list(store, id).unwrap_or_default();
    report.issues.extend(
        unknown_zachman_columns(&adrs)
            .into_iter()
            .map(|message| Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: None,
                message,
            }),
    );
    scan_stray_folders(store, &project, &mut report);
    scan_leftover_writes(store, id, &mut report);
    scan_process_drift(store, &project, &mut report);
    scan_shipped_release_items(store, &project, &charter, &ctx, &mut report);
    scan_release_items_outside_sprint(store, &project, &mut report);
    scan_sprints_started_early(store, &project, &mut report);
    if project.config.method.is_on(crate::method::Part::Links) {
        scan_references(store, &project, &mut report);
    }
    scan_declared_docs(&project, &charter, &ctx, &mut report);
    scan_profile_drift(store, &project, &mut report);
    scan_baseline(store, &project, &mut report);
    scan_coverage(store, &project, &mut report);
    scan_catalog(store, &project, &mut report);
    scan_affects_views(store, &project, &mut report);
    scan_diagrams(store, &project, &mut report);
    scan_query_blocks(store, &project, &mut report);
    scan_method_choice(&project, &mut report);
    scan_codes(&project, &mut report);
    scan_repository(store, &project, &mut report);
    // A task ref whose task is gone (FTR-060).
    report.issues.extend(crate::taskgraph::issues(store, id));
    scan_identity(store, id, &mut report);
    scan_readme(store, id, &mut report);
    scan_remote_lag(store, id, &mut report);
    scan_judgement(&project, &mut report);
    if project.config.method.is_on(crate::method::Part::Links) {
        scan_repository_links(store, &project, repo, &mut report);
    }
    Ok(report)
}

/// Report a spec's link to a repository doc that is not in the code repository at the item's
/// position (FTR-004, ADR-0014): the item's merge commit, else its branch, else main (an item with
/// no branch yet is judged against main, and the message says so). Without a repository, one
/// warning says how many links were not checked; nothing is said when no spec links one (L-4). A
/// position or tree that cannot be read steps aside (ADR-0004).
fn scan_repository_links(
    store: &Store,
    project: &Project,
    repo: Option<&std::path::Path>,
    report: &mut Report,
) {
    use crate::picks::{Pick, resolve};
    let Ok(index) = crate::refs::BoardIndex::of(store, project) else {
        return;
    };
    let mut links: Vec<(&str, Vec<(usize, String)>)> = Vec::new();
    for f in &project.features {
        let found: Vec<(usize, String)> = crate::refs::find(&f.specification, &index)
            .into_iter()
            .filter_map(|found| match found.reference {
                crate::refs::Ref::RepoDoc { path } => Some((found.line, path)),
                _ => None,
            })
            .collect();
        if !found.is_empty() {
            links.push((f.code.as_str(), found));
        }
    }
    let n: usize = links.iter().map(|(_, l)| l.len()).sum();
    if n == 0 {
        return;
    }
    let Some(repo) = repo else {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message: format!(
                "{n} repository doc links were not checked: run doctor from the project folder"
            ),
        });
        return;
    };
    let mut trees: std::collections::BTreeMap<String, BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (code, found) in links {
        let at = match resolve(store, &project.id, repo, &Pick::Item(code.to_string())) {
            Ok(at) => at,
            Err(_) => match resolve(store, &project.id, repo, &Pick::Main) {
                Ok(mut main) => {
                    main.label = format!("{} (no branch yet)", main.label);
                    main
                }
                Err(_) => continue,
            },
        };
        let key = at.state.position.text();
        if !trees.contains_key(&key) {
            let Ok(files) = crate::position::tree_at(repo, &at.state, "docs/") else {
                continue;
            };
            trees.insert(key.clone(), files.into_iter().map(|(p, _)| p).collect());
        }
        let files = &trees[&key];
        for (line, path) in found {
            if !files.contains(&path) {
                report.issues.push(Issue {
                    severity: Severity::Error,
                    project: project.id.clone(),
                    code: Some(code.to_string()),
                    message: format!(
                        "{code} spec:{line} links {path}, which is not in the code repository at {}",
                        at.label
                    ),
                });
            }
        }
    }
}

/// Report an item whose manual tests break the judgement rule (FTR-039): more than one, or one
/// that names what a run can check. A warning for items in any status, never an error: the rule
/// refuses only a new or renamed manual test, so an item written before it keeps its tests until
/// they are touched.
fn scan_judgement(project: &Project, report: &mut Report) {
    for f in &project.features {
        let Some(line) = judgement_warning(f) else {
            continue;
        };
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: Some(f.code.clone()),
            message: line,
        });
    }
}

/// "<CODE> has manual tests that break the judgement rule: <messages>", or `None` for an item
/// whose manual tests keep the rule (FTR-039). `kanbanr check` prints the same line as a note.
pub fn judgement_warning(f: &crate::models::FeatureItem) -> Option<String> {
    let def = f.definition.as_ref()?;
    let findings = crate::judgement::findings(def);
    if findings.is_empty() {
        return None;
    }
    let messages: Vec<String> = findings.iter().map(|x| x.message()).collect();
    Some(format!(
        "{} has manual tests that break the judgement rule: {}",
        f.code,
        messages.join("; ")
    ))
}

/// Report a board further behind its remote than its push policy allows (FEAT-142). kanbanr's own
/// public board drifted 58 commits behind GitHub with nothing saying so: the CLI's batched push
/// never fired, and when it was asked to, it could not reach GitHub at all.
fn scan_remote_lag(store: &Store, project: &str, report: &mut Report) {
    use crate::git::PushPolicy;
    let dir = store.data_dir();
    let Some(ahead) = crate::git::ahead_of_remotes(dir) else {
        return;
    };
    let (policy, _) = PushPolicy::for_board(dir);
    let allowed = match policy {
        PushPolicy::Auto => 0,
        PushPolicy::Debounce { every } => every as usize,
        // Pushing only by hand is a choice, not a fault; still say so once it is a real backlog.
        PushPolicy::Off => 10,
    };
    if ahead > allowed {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: format!(
                "this board is {ahead} commit(s) ahead of its remote (push policy: {policy}) — \
                 `kanbanr sync` pushes them; `kanbanr remote push-policy` shows why they have not \
                 gone"
            ),
        });
    }
}

/// Report a board that commits as nobody (FEAT-128): the placeholder identity older versions wrote
/// into every new data repository, or no identity at all — in which case kanbanr now refuses to
/// commit, and this says why before a write does.
fn scan_identity(store: &Store, project: &str, report: &mut Report) {
    let dir = store.data_dir();
    if !dir.join(".git").exists() {
        return;
    }
    let message = if crate::git::has_placeholder_identity(dir) {
        "this board's git config names the placeholder `kanbanr <kanbanr@local>` as its commit \
         identity, which authors commits as nobody. Set yours with `kanbanr identity --name \
         \"Your Name\" --email you@example.com`"
    } else if crate::git::identity(dir).is_none() {
        "this board has no commit identity, so writes are refused. Set one with `kanbanr identity \
         --name \"Your Name\" --email you@example.com`"
    } else {
        return;
    };
    report.issues.push(Issue {
        severity: Severity::Warning,
        project: project.to_string(),
        code: None,
        message: message.to_string(),
    });
}

/// Report what a work item has not said yet (FEAT-049).
///
/// Scope is everything here. A naive "warn when undefined" fires on every item a board has ever
/// held — 45 on kanbanr's own board, 40 of them finished — and a report that long is wallpaper
/// nobody reads. An item is in scope only when it is live work under the method: not terminal,
/// displayed on the board, and created after the charter was adopted.
fn scan_definitions(
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    report: &mut Report,
) {
    if charter.adopted_at.trim().is_empty() {
        return; // the method has not been adopted here; nothing to hold anyone to
    }
    let goal_ids = charter.goal_ids();
    let mut goals_served: BTreeSet<String> = BTreeSet::new();

    for feature in &project.features {
        // Goal coverage counts EVERY item, finished ones included: a goal delivered last month is
        // served, even though that item is out of scope for gap reporting.
        if let Some(def) = &feature.definition {
            goals_served.extend(def.goals.iter().cloned());
        }
        // An unreconciled bypass outlives the terminal gate (FEAT-080). Every other gap check
        // rightly stops caring once work is finished — you cannot fix a missing requirement on a
        // shipped item. But "built without agreement, and still never agreed to" is a question
        // that finishing does not answer; letting it age out silently meant a clean report read as
        // "nothing needs attention" when it meant "the things that did, completed first".
        if !in_scope(project, charter, feature) {
            if let Some(def) = &feature.definition
                && awaiting_ratification(feature)
                && !crate::before(&feature.created_at, &charter.adopted_at)
                && def.exempt.trim().is_empty()
            {
                push(
                    report,
                    project,
                    feature,
                    Severity::Warning,
                    format!(
                        "finished without agreement: {} — `kanbanr ratify {}` to agree to it \
                         after the fact, or record why it does not need one",
                        def.started_unapproved.trim(),
                        feature.code
                    ),
                );
            }
            continue;
        }

        // The same rules every surface uses (FEAT-112). Where the workflow declares its gates,
        // doctor warns only about what the gate of the item's CURRENT stage asks, plus its warnings
        // — a definition built stage by stage is not incomplete for lacking what a later stage
        // asks (FEAT-117, DEF-021). What the stage reached next still lacks is counted, not
        // warned. Agreement and sign-offs are the review queue's questions, not doctor's.
        let gaps = if !project.config.declares_gates() {
            crate::readiness::evaluate(feature, Some(&goal_ids), crate::readiness::DOCTOR)
        } else {
            use crate::readiness::Check;
            let mut gaps = crate::readiness::evaluate(
                feature,
                Some(&goal_ids),
                &[Check::Definition, Check::GoalsKnown, Check::Bypass],
            );
            for gap in current_stage_gaps(project, charter, feature, ctx) {
                if !gaps.contains(&gap) {
                    gaps.push(gap);
                }
            }
            if let Some((stage, ahead)) = next_stage_gaps(project, charter, feature, ctx) {
                report.count_ahead(project, &feature.code, &stage, ahead.len());
            }
            gaps
        };
        // One combined message for what the definition has not said: a separate issue per column
        // would bury the report.
        use crate::readiness::Check;
        let (unsaid, rest): (Vec<_>, Vec<_>) = gaps
            .into_iter()
            .partition(|g| matches!(g.check, Check::Statement | Check::Zachman | Check::Goals));
        if !unsaid.is_empty() {
            let labels: Vec<&str> = unsaid.iter().map(|g| g.label.as_str()).collect();
            push(
                report,
                project,
                feature,
                Severity::Warning,
                format!("definition gaps: {}", labels.join(" ")),
            );
        }
        for gap in rest {
            // A declared doc the board lacks is a broken reference, said once by
            // scan_references, gate or no gate (FEAT-180).
            if gap.check == Check::DocsUpdated && gap.level == crate::readiness::Level::Error {
                continue;
            }
            let severity = match gap.level {
                crate::readiness::Level::Error => Severity::Error,
                crate::readiness::Level::Warning => Severity::Warning,
            };
            push(report, project, feature, severity, gap.message);
        }
    }

    for goal in &charter.goals {
        if !goals_served.contains(&goal.id) {
            report.issues.push(Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: None,
                message: format!(
                    "charter goal {} has no work linked to it: \"{}\"",
                    goal.id,
                    goal.statement.trim()
                ),
            });
        }
    }
}

/// Whether doctor says a gap at all: agreement and sign-offs are the Review queue's questions, and
/// the definition's own presence is checked on every item regardless of stage.
pub(crate) fn doctor_relevant(gap: &crate::readiness::Gap) -> bool {
    use crate::readiness::Check;
    !matches!(
        gap.check,
        Check::Approved | Check::Signoff | Check::Definition
    )
}

/// What the gate of the item's current status asks and the item no longer meets: its `requires`
/// then its `warns`. The item met this gate to be here, so a gap is drift (DEF-021).
fn current_stage_gaps(
    project: &Project,
    charter: &crate::Charter,
    feature: &crate::FeatureItem,
    ctx: &crate::readiness::Context,
) -> Vec<crate::readiness::Gap> {
    let Some(gate) = project
        .config
        .gate_for_kind(&feature.status, feature.kind.as_deref())
    else {
        return Vec::new();
    };
    let goal_ids = charter.goal_ids();
    // Judged as the move judged it: `docs_updated` over the window the move used (DEF-074).
    let mut gaps = crate::readiness::evaluate_passed(feature, Some(&goal_ids), &gate.requires, ctx);
    gaps.extend(crate::readiness::evaluate_passed(
        feature,
        Some(&goal_ids),
        &gate.warns,
        ctx,
    ));
    gaps.retain(doctor_relevant);
    gaps
}

/// The stage the item reaches next and what it still lacks there, by the same rules as
/// `current_stage_gaps`; counted, never warned (DEF-021). `None` when there is no next stage.
fn next_stage_gaps(
    project: &Project,
    charter: &crate::Charter,
    feature: &crate::FeatureItem,
    ctx: &crate::readiness::Context,
) -> Option<(String, Vec<crate::readiness::Gap>)> {
    let next = crate::readiness::immediate_next_gate(project, Some(charter), feature, ctx)?;
    let mut gaps = next.gaps;
    gaps.extend(next.warnings);
    gaps.retain(doctor_relevant);
    Some((next.status, gaps))
}

/// An ADR naming a Zachman column outside the six (MNT-009, Q4). `parse` keeps such a value as
/// written so the board still loads; this is where somebody sees it. A warning, never a hold.
fn unknown_zachman_columns(adrs: &[crate::adr::Adr]) -> Vec<String> {
    adrs.iter()
        .flat_map(|adr| {
            adr.zachman
                .iter()
                .filter(|v| crate::vocab::zachman(v).is_none())
                .map(move |v| {
                    format!(
                        "{} names '{v}', which is not a Zachman column (What, How, Where, Who, \
                         When, Why)",
                        adr.id
                    )
                })
        })
        .collect()
}

/// Architecture decisions in the graph (FEAT-057).
///
/// The severities follow the existing rule: a **broken reference** is an error, because someone
/// can fix it by editing a name; everything else is a warning, because it is a judgement about
/// whether the reasoning is complete, and a judgement that blocks a write gets worked around.
fn scan_decisions(store: &Store, project: &Project, report: &mut Report) {
    let adrs = crate::adr::list(store, &project.id).unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();
    let mut warn = |message: String| warnings.push(message);

    for problem in crate::adr::dangling(&adrs, project) {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: project.id.clone(),
            code: None,
            message: problem,
        });
    }

    for adr in &adrs {
        let missing = adr.missing_sections();
        // An ADR without its Decision or Consequences is a note about a meeting. Consequences is
        // the section a later reader needs most, because it says what the decision cost.
        for section in ["Decision", "Consequences"] {
            if missing.contains(&section) {
                warn(format!(
                    "{} has no {} section — the part a later reader needs before overturning it",
                    adr.id,
                    section.to_lowercase()
                ));
            }
        }
        if adr.is_accepted() && adr.affects.is_empty() && !adr.is_superseded() {
            warn(format!(
                "{} is accepted but claims to affect nothing, so nothing rests on it",
                adr.id
            ));
        }
    }

    // A quality a requirement measures, that no decision serves: the number is an aspiration.
    let mut measured: BTreeSet<String> = BTreeSet::new();
    for feature in &project.features {
        for requirement in feature
            .definition
            .iter()
            .flat_map(|d| d.requirements.iter())
            .filter(|r| matches!(r.kind, crate::models::RequirementKind::Nfr))
        {
            // Counted by the canonical form, whatever the stored spelling (MNT-009).
            measured.extend(requirement.iso.iter().map(|t| {
                crate::vocab::iso(t).map_or_else(|| t.trim().to_string(), str::to_string)
            }));
        }
    }
    let served: BTreeSet<String> = adrs
        .iter()
        .filter(|a| !a.is_superseded())
        .flat_map(|a| {
            a.quality
                .iter()
                .map(|t| crate::vocab::iso(t).map_or_else(|| t.trim().to_string(), str::to_string))
        })
        .collect();
    for quality in measured.difference(&served) {
        warn(format!(
            "{quality} is measured by a requirement but no decision claims to serve it"
        ));
    }

    report
        .issues
        .extend(warnings.into_iter().map(|message| Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message,
        }));
}

fn push(
    report: &mut Report,
    project: &Project,
    feature: &crate::FeatureItem,
    severity: Severity,
    message: String,
) {
    report.issues.push(Issue {
        severity,
        project: project.id.clone(),
        code: Some(feature.code.clone()),
        message,
    });
}

/// Did this item start under a recorded bypass that nobody has answered yet?
///
/// Answered means agreed to — before the work (`approved`) or after it (`ratified`). A lapsed
/// approval is not an answer: the definition changed, so the agreement no longer covers what was
/// built, and the question is open again.
pub(crate) fn unreconciled_bypass(def: &crate::models::FeatureDefinition) -> bool {
    !def.started_unapproved.trim().is_empty() && !def.approval_state().is_agreed()
}

/// Does `f` wait for a ratification? The old rule (`unreconciled_bypass` of its definition),
/// or an open override (DEF-043). The one predicate the doctor, the review queue, the cut and
/// FTR-034's `ratification` prepared record use.
pub fn awaiting_ratification(f: &crate::FeatureItem) -> bool {
    f.definition.as_ref().is_some_and(unreconciled_bypass)
        || !crate::readiness::open_overrides(f).is_empty()
}

/// Live work under the method: not finished, on the board, and created since adoption. The first two
/// are `graph::is_live_work`, shared with the review queue so the two cannot drift (FEAT-078); the
/// adoption cutoff is the doctor's own, keeping pre-method items out of the report.
fn in_scope(project: &Project, charter: &crate::Charter, feature: &crate::FeatureItem) -> bool {
    crate::graph::is_live_work(&project.config, &feature.status)
        && !crate::before(&feature.created_at, &charter.adopted_at)
}

/// Check a project's charter — the root of its reasoning (FEAT-046). Without a purpose there is
/// nothing to judge work against; without goals, items have nothing to link to. Both are warnings:
/// a project with no charter is incomplete, not broken.
fn scan_charter(charter: &crate::Charter, project: &str, report: &mut Report) {
    if charter.purpose.trim().is_empty() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: "no charter purpose — nothing states why this project exists \
                      (write one with `kanbanr charter set --file charter.yaml`)"
                .to_string(),
        });
    } else if charter.goals.is_empty() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message:
                "charter states a purpose but declares no goals — work items have no goal ids \
                      to link to"
                    .to_string(),
        });
    }
}

/// Run the three checks against one loaded project, given the universe of existing qualified
/// feature ids (`existing`) for cross-project dependency resolution.
/// A folder that looks like a status but is not one any more (FEAT-071). Renaming a status used to
/// leave its directory behind, and an empty orphan can sit in a board for months — this one did.
/// Reported, never removed: deleting a directory the tool does not understand is not doctor's job.
/// A project whose saved process has moved on, or whose workflow was edited after it was applied
/// (FEAT-170). Reported, never applied: a process change reaches a project when its user says so.
fn scan_process_drift(store: &Store, project: &Project, report: &mut Report) {
    let Ok(Some(drift)) = store.process_drift(project, None) else {
        return;
    };
    for message in drift.messages() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message,
        });
    }
}

/// The practice packs a project follows whose library copy has changed since it applied them, or
/// that are gone from their library (FTR-033 R-5): a warning per message, each naming what to run.
/// Only reported, never applied: `kanbanr profile update` is the person's choice. A personal pack
/// is skipped, as the daemon cannot see that library.
fn scan_profile_drift(store: &Store, project: &Project, report: &mut Report) {
    // A baseline that does not parse is `scan_baseline`'s to report, once.
    if crate::baseline::load(store, &project.id).is_err() {
        return;
    }
    let drift = match store.profile_drift(&project.id, None) {
        Ok(d) => d,
        Err(e) => {
            report.issues.push(Issue {
                severity: Severity::Error,
                project: project.id.clone(),
                code: None,
                message: format!("the practice packs this project follows cannot be read: {e}"),
            });
            return;
        }
    };
    for message in drift.iter().flat_map(|d| d.messages()) {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message,
        });
    }
}

/// The baseline's own consistency (FTR-033 R-5): a proposed revision still waiting for the
/// person, a constraint in force without its ADR, an ADR recording an entry that is not in force,
/// and a release stage edited away from a gate an entry in force promised.
fn scan_baseline(store: &Store, project: &Project, report: &mut Report) {
    use crate::baseline::RevisionState;
    let mut push = |severity: Severity, message: String| {
        report.issues.push(Issue {
            severity,
            project: project.id.clone(),
            code: None,
            message,
        });
    };
    let b = match crate::baseline::load(store, &project.id) {
        Ok(b) => b,
        Err(e) => {
            push(
                Severity::Error,
                format!(
                    "{} does not parse: {e}; kanbanr never rewrites it, so fix it by hand",
                    crate::baseline::BASELINE_FILE
                ),
            );
            return;
        }
    };
    for r in b
        .revisions
        .iter()
        .filter(|r| r.state() == RevisionState::Proposed)
    {
        let since = r.proposed.at.get(..10).unwrap_or(&r.proposed.at);
        push(
            Severity::Warning,
            format!(
                "{} has waited since {since} for the person to accept it on the Baseline page; \
                 the gates read no baseline from it",
                r.id
            ),
        );
    }
    let adrs = crate::adr::list(store, &project.id).unwrap_or_default();
    for e in crate::baseline::in_force(&b)
        .into_iter()
        .filter(|e| e.kind == crate::profile::Kind::Constraint)
    {
        let found = !e.adr.is_empty() && adrs.iter().any(|a| a.id.eq_ignore_ascii_case(&e.adr));
        if !found {
            let why = if e.adr.is_empty() {
                "baseline.yaml names none".to_string()
            } else {
                format!("{} is not among the ADRs", e.adr)
            };
            push(
                Severity::Error,
                format!(
                    "baseline entry {} is an accepted constraint with no ADR ('{}'): {why}",
                    e.id, e.text
                ),
            );
        }
    }
    let accepted: BTreeSet<&str> = b
        .revisions
        .iter()
        .filter(|r| r.state() == RevisionState::Accepted)
        .map(|r| r.id.as_str())
        .collect();
    for a in adrs
        .iter()
        .filter(|a| a.status == "accepted" && !a.baseline.is_empty())
    {
        match b
            .entries
            .iter()
            .find(|e| e.id.eq_ignore_ascii_case(&a.baseline))
        {
            Some(e) if accepted.contains(e.revision.as_str()) => {}
            Some(e) => push(
                Severity::Error,
                format!(
                    "{} records baseline entry {}, but {}, which adds it, is not accepted: \
                     accept {} again",
                    a.id, e.id, e.revision, e.revision
                ),
            ),
            None => push(
                Severity::Error,
                format!(
                    "{} records baseline entry {}, which the baseline does not hold",
                    a.id, a.baseline
                ),
            ),
        }
    }
    // With releases off no stage gate is written (it applies once releases are on).
    let cadence = &project.config.cadence;
    if cadence.releases {
        let followed = crate::baseline::followed_in_force(&b);
        for (stage, gate, from) in crate::baseline::promised_gates(&followed) {
            let held = cadence
                .release_gates
                .get(&stage)
                .is_some_and(|g| g.gates.iter().any(|s| s.gate == gate && s.from == from));
            if !held {
                push(
                    Severity::Warning,
                    format!(
                        "stage {stage} no longer requires `{gate}`, which {from} promised; a \
                         release can enter {stage} without it"
                    ),
                );
            }
        }
    }
}

/// An item planned into a release that has shipped and is not at an end status (DEF-029): the
/// board says it shipped, but the item still stands where it was. Reported, never moved: the way
/// out is the person's own sign-off or move.
fn scan_shipped_release_items(
    store: &Store,
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    report: &mut Report,
) {
    let Ok(releases) = crate::releases::load(store, &project.id) else {
        return;
    };
    for s in crate::releases::stranded(project, charter, ctx, &releases) {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            message: format!(
                "{} is planned into {}, which shipped on {}, but is at {}: {}. Give what it lacks, \
                 or plan it into a coming release",
                s.code, s.release, s.shipped_at, s.status, s.why
            ),
            code: Some(s.code),
        });
    }
}

/// A live item planned into a release that has not shipped, and into no sprint, while a sprint
/// runs (DEF-029). Quiet when no sprint runs, so it never fires on everything.
fn scan_release_items_outside_sprint(store: &Store, project: &Project, report: &mut Report) {
    let (Ok(sprints), Ok(releases)) = (
        crate::sprints::load(store, &project.id),
        crate::releases::load(store, &project.id),
    ) else {
        return;
    };
    for o in crate::sprints::outside_running_sprint(project, &sprints, &releases, None) {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            message: format!(
                "{} is planned into {} but into no sprint, and {} is running: kanbanr sprint plan {} {}",
                o.code, o.release, o.sprint, o.sprint, o.code
            ),
            code: Some(o.code),
        });
    }
}

/// A sprint stored active whose first day is still ahead (DEF-041): it reads as planned until
/// then, and no item can move into work in it. Starting it again starts it today.
fn scan_sprints_started_early(store: &Store, project: &Project, report: &mut Report) {
    let Ok(sprints) = crate::sprints::load(store, &project.id) else {
        return;
    };
    let today = crate::sprints::today();
    for s in sprints.iter().filter(|s| {
        s.state == crate::sprints::SprintState::Active
            && s.state_on(today) == crate::sprints::SprintState::Planned
    }) {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message: format!(
                "{code} was started before its first day ({start}): it reads as planned until then, and no item can move into work in it. Starting it again (`kanbanr sprint start {code}`) starts it today and keeps its length.",
                code = s.code,
                start = s.start
            ),
        });
    }
}

/// References in specs and docs that point at nothing on this board (FEAT-181). A link someone
/// wrote as a link is an error — it is meant to be followed; a code named in prose that has this
/// board's shape is a warning, since prose may name an item that was never created. A design doc an
/// item points at must exist.
fn scan_references(store: &Store, project: &Project, report: &mut Report) {
    let Ok(index) = crate::refs::BoardIndex::load(store, &project.id) else {
        return;
    };
    let mut push = |code: Option<String>, source: &str, found: &crate::refs::Found| {
        let severity = if found.explicit {
            Severity::Error
        } else {
            Severity::Warning
        };
        report.issues.push(Issue {
            severity,
            project: project.id.clone(),
            code,
            // A field (design_doc, affects_docs) has no line to point at.
            message: if found.line == 0 {
                format!(
                    "{source} refers to {}, which this board does not have",
                    describe(&found.reference)
                )
            } else {
                format!(
                    "{source}:{} refers to {}, which this board does not have",
                    found.line,
                    describe(&found.reference)
                )
            },
        });
    };
    for f in &project.features {
        // The docs an item says its design changes (FEAT-180): a name the board does not have is
        // a broken link, and `docs_updated` could never pass on it.
        for doc in f.definition.iter().flat_map(|d| d.affects_docs.iter()) {
            let reference = crate::refs::Ref::Doc { path: doc.clone() };
            if !index.resolves(&reference) {
                let found = crate::refs::Found {
                    reference,
                    line: 0,
                    explicit: true,
                };
                push(
                    Some(f.code.clone()),
                    &format!("{} affects_docs", f.code),
                    &found,
                );
            }
        }
        let source = format!("{} spec", f.code);
        for found in crate::refs::find(&f.specification, &index) {
            if index.is_broken(&found) {
                push(Some(f.code.clone()), &source, &found);
            }
        }
        if let Some(design) = f.definition.as_ref().map(|d| d.design_doc.trim())
            && !design.is_empty()
        {
            let found = crate::refs::Found {
                reference: crate::refs::Ref::Doc {
                    path: design.to_string(),
                },
                line: 0,
                explicit: true,
            };
            let resolved = crate::refs::Ref::Doc {
                path: crate::refs::doc_key(design),
            };
            if !index.resolves(&resolved) {
                push(
                    Some(f.code.clone()),
                    &format!("{} design_doc", f.code),
                    &found,
                );
            }
        }
    }
    for path in &index.docs {
        let Ok(text) = store.read_doc(&project.id, path) else {
            continue;
        };
        let shipped = path.starts_with("releases/");
        for found in crate::refs::find(&text, &index) {
            // A release's document is the notes as they shipped: its document links are the book's.
            if shipped && matches!(found.reference, crate::refs::Ref::Doc { .. }) {
                continue;
            }
            if index.is_broken(&found) {
                push(None, &format!("docs/{path}"), &found);
            }
        }
    }
}

/// An item whose current stage asks for `docs_updated` but that names no docs (FEAT-180, Q2). The
/// gate passes it — not every item touches an architecture doc — so this is where it is said; when
/// only the next stage asks, it is counted (DEF-021).
fn scan_declared_docs(
    project: &Project,
    charter: &crate::Charter,
    ctx: &crate::readiness::Context,
    report: &mut Report,
) {
    // Judged by the gate the item's kind meets, its path's parts left out (DEF-070, DEF-069), by
    // the same rule `kanbanr check` and the move use (DEF-075).
    let warning = |f: &crate::models::FeatureItem, status: &str| {
        project
            .config
            .gate_for_kind(status, f.kind.as_deref())
            .and_then(|g| crate::readiness::undeclared_docs_warning(&project.id, f, status, &g))
    };
    for f in &project.features {
        let Some(def) = f.definition.as_ref() else {
            continue;
        };
        // Named docs win over `affects_no_docs`; saying both is a contradiction to clear up.
        if !def.affects_docs.is_empty() && !def.affects_no_docs.trim().is_empty() {
            report.issues.push(Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: Some(f.code.clone()),
                message: format!(
                    "{} names docs and also records affects_no_docs; remove one",
                    f.code
                ),
            });
        }
        if !crate::graph::is_live_work(&project.config, &f.status)
            || !def.affects_docs.is_empty()
            || !def.exempt.trim().is_empty()
            || !def.affects_no_docs.trim().is_empty()
            // The gate reads the docs the Design links (DEF-047).
            || !crate::docsbuilt::docs_to_change(&f.specification, &project.id).is_empty()
        {
            continue;
        }
        if let Some(gap) = warning(f, &f.status) {
            report.issues.push(Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: Some(f.code.clone()),
                message: gap.message,
            });
        } else if let Some(next) =
            crate::readiness::immediate_next_gate(project, Some(charter), f, ctx)
            && warning(f, &next.status).is_some()
        {
            // Only the stage reached next asks: said as a count (DEF-021).
            report.count_ahead(project, &f.code, &next.status, 1);
        }
    }
}

/// The project's coverage rules (FEAT-197): one issue per rule that leaves subjects uncovered,
/// naming the first ten; an invalid rule, a rule that cannot be judged, a gate narrowing
/// `docs_cover` to a rule the project does not configure, and a gate's default doc that is not on
/// the board are errors. Silent for a project with no rules and no gate asking for coverage.
fn scan_coverage(store: &Store, project: &Project, report: &mut Report) {
    let config = &project.config;
    let asks = config
        .gates
        .values()
        .any(|g| g.requires.iter().chain(&g.warns).any(|c| c.is_docs_cover()));
    let defaults = config.gates.values().any(|g| !g.affects_docs.is_empty());
    if config.coverage.is_empty() && !asks && !defaults {
        return;
    }
    let mut issue = |severity: Severity, message: String| {
        report.issues.push(Issue {
            severity,
            project: project.id.clone(),
            code: None,
            message,
        });
    };
    let invalid: Vec<String> = crate::coverage::rule_problems(&config.coverage, config);
    for p in &invalid {
        issue(Severity::Error, p.clone());
    }
    let ids: BTreeSet<&str> = config.coverage.iter().map(|r| r.id.as_str()).collect();
    let docs: BTreeSet<String> = store
        .list_docs(&project.id)
        .unwrap_or_default()
        .into_iter()
        .collect();
    for (key, gate) in &config.gates {
        for c in gate.requires.iter().chain(&gate.warns) {
            if let crate::readiness::Condition::DocsCover { docs_cover } = c {
                for id in docs_cover.iter().filter(|id| !ids.contains(id.trim())) {
                    issue(
                        Severity::Error,
                        format!(
                            "the gate on '{key}' asks for docs_cover rule '{id}', which this \
                             project does not configure"
                        ),
                    );
                }
            }
        }
        for doc in &gate.affects_docs {
            let path = crate::refs::doc_key(doc);
            if !docs.contains(&path) {
                issue(
                    Severity::Error,
                    format!(
                        "the gate on '{key}' lists {path} in affects_docs, which is not on the \
                         board"
                    ),
                );
            }
        }
    }
    if config.coverage.is_empty() {
        return;
    }
    let snap = match crate::coverage::Snapshot::load(store, project) {
        Ok(s) => s,
        Err(e) => {
            issue(
                Severity::Error,
                format!("coverage rules could not be judged: {e}"),
            );
            return;
        }
    };
    for (rule, r) in snap.rules.iter().zip(snap.report(project).rules) {
        if let Some(problem) = &r.problem {
            // An invalid rule was named above, from the rules as written.
            if !invalid
                .iter()
                .any(|p| p.starts_with(&format!("coverage rule '{}'", r.id)))
            {
                let problem = problem
                    .strip_prefix(&format!("[{}] ", r.id))
                    .unwrap_or(problem);
                issue(
                    Severity::Error,
                    format!("coverage rule '{}': {problem}", r.id),
                );
            }
            continue;
        }
        if r.uncovered.is_empty() {
            continue;
        }
        let about = if rule.description.trim().is_empty() {
            rule.describe()
        } else {
            rule.description.trim().to_string()
        };
        let first: Vec<&str> = r.uncovered.iter().take(10).map(|s| s.id.as_str()).collect();
        let more = if r.uncovered.len() > 10 {
            ", … (first 10)"
        } else {
            ""
        };
        issue(
            Severity::Warning,
            format!(
                "coverage rule '{}' ({about}): {} of {} uncovered — {}{more}",
                r.id,
                r.uncovered.len(),
                r.subjects,
                first.join(", ")
            ),
        );
    }
}

/// The project's facts and content catalog (FTR-005): one hint when the project has no facts
/// record, an error per problem of an invalid catalog (naming the row), and a warning per applying
/// row whose source is missing. Nothing else without a catalog.
/// The catalog rows an item's design says it changes (`affects_views`, FTR-006): an id the catalog
/// does not have is an error, like an unknown `affects_docs` path; without a catalog, naming any
/// row at all is a warning.
fn scan_affects_views(store: &Store, project: &Project, report: &mut Report) {
    let catalog = crate::catalog::load(store, &project.id).ok().flatten();
    for f in &project.features {
        let Some(def) = f.definition.as_ref() else {
            continue;
        };
        if def.affects_views.is_empty() {
            continue;
        }
        let mut issue = |severity: Severity, message: String| {
            report.issues.push(Issue {
                severity,
                project: project.id.clone(),
                code: Some(f.code.clone()),
                message,
            });
        };
        match &catalog {
            None => issue(
                Severity::Warning,
                format!(
                    "{} affects_views names catalog rows but the project has no catalog: \
                     `kanbanr catalog init`",
                    f.code
                ),
            ),
            Some(c) => {
                for id in &def.affects_views {
                    if c.row(id.trim()).is_none() {
                        issue(
                            Severity::Error,
                            format!(
                                "{} affects_views refers to {id}, which the catalog does not have \
                                 (`kanbanr catalog` lists the rows)",
                                f.code
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn scan_catalog(store: &Store, project: &Project, report: &mut Report) {
    let mut issue = |severity: Severity, message: String| {
        report.issues.push(Issue {
            severity,
            project: project.id.clone(),
            code: None,
            message,
        });
    };
    match crate::facts::load(store, &project.id) {
        Ok(Some(_)) => {}
        Ok(None) => issue(
            Severity::Warning,
            "no project facts — `kanbanr facts set …`".to_string(),
        ),
        Err(e) => issue(
            Severity::Error,
            format!("{} cannot be read: {e}", crate::facts::FACTS_FILE),
        ),
    }
    let Ok(status) = crate::catalog::status(store, project, None) else {
        return;
    };
    for p in &status.problems {
        // The facts file's own problem is reported above.
        if !p.starts_with(crate::facts::FACTS_FILE) {
            issue(Severity::Error, format!("catalog: {p}"));
        }
    }
    for row in &status.rows {
        let missing = matches!(
            row.state,
            crate::catalog::SourceState::Missing | crate::catalog::SourceState::Folder { docs: 0 }
        );
        if row.applies.applies && missing {
            issue(
                Severity::Warning,
                format!(
                    "catalog row {} ({}) has no source: write {}",
                    row.id, row.title, row.source.path
                ),
            );
        }
    }
}

/// A project from before the method that never chose (FEAT-193, Q3): its workflow has none of the
/// method's gates and it has not said whether it wants them. Said once per project; either choice
/// silences it.
/// Every diagram lint finding on the board (FEAT-200): in every item's spec, finished items too,
/// and in every markdown doc. Warnings — what a `diagrams_valid` gate or a strict project would
/// refuse, found before then.
fn scan_diagrams(store: &Store, project: &Project, report: &mut Report) {
    let mut push = |code: Option<String>, f: crate::diagrams::DiagramFinding| {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code,
            message: f.describe(),
        });
    };
    for feature in &project.features {
        let file = format!("{} spec", feature.code);
        for f in crate::diagrams::lint_markdown(&file, &feature.specification) {
            push(Some(feature.code.clone()), f);
        }
    }
    for path in store.list_docs(&project.id).unwrap_or_default() {
        if !crate::diagrams::is_markdown_path(&path) {
            continue;
        }
        let Ok(text) = store.read_doc(&project.id, &path) else {
            continue;
        };
        for f in crate::diagrams::lint_markdown(&format!("docs/{path}"), &text) {
            push(None, f);
        }
    }
}

/// Every `katalogr-query` block in a markdown doc that does not parse (FTR-016): a warning naming
/// the doc, the block's line and the reason. A block that parses says nothing, and nothing here
/// needs katalogr to be installed.
fn scan_query_blocks(store: &Store, project: &Project, report: &mut Report) {
    for path in store.list_docs(&project.id).unwrap_or_default() {
        if !crate::diagrams::is_markdown_path(&path) {
            continue;
        }
        let Ok(text) = store.read_doc(&project.id, &path) else {
            continue;
        };
        for p in crate::katalogr::query::problems(&format!("docs/{path}"), &text) {
            report.issues.push(Issue {
                severity: Severity::Warning,
                project: project.id.clone(),
                code: None,
                message: format!("{}, query block, line {}: {}", p.file, p.line, p.reason),
            });
        }
    }
}

/// Codes a hand edit broke (FEAT-217): no item can be coded until they are fixed, so each problem
/// is an error.
fn scan_codes(project: &Project, report: &mut Report) {
    let milestones =
        crate::config::milestone_prefixes(project.milestones.iter().map(|m| m.code.as_str()));
    for problem in project.config.codes.problems(&milestones) {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: project.id.clone(),
            code: None,
            message: format!(
                "{problem} — new items cannot be coded; fix with `kanbanr config codes`"
            ),
        });
    }
}

fn scan_method_choice(project: &Project, report: &mut Report) {
    if !project.config.method.chosen_at.is_empty() {
        return;
    }
    let wf = crate::config::WorkflowFile::from_config(&project.config);
    if crate::method::Part::GATES
        .iter()
        .any(|p| crate::method::present(&wf, *p).is_some())
    {
        return;
    }
    report.issues.push(Issue {
        severity: Severity::Warning,
        project: project.id.clone(),
        code: None,
        message: "this project predates the method (design before build, docs kept current): \
                  `kanbanr config method --on` to take it, or `--off` to keep it off"
            .to_string(),
    });
}

/// A shared board whose project records no code repository (DEF-053): its README cannot link the
/// code it tracks. Silent on a board nobody else sees.
fn scan_repository(store: &Store, project: &Project, report: &mut Report) {
    if project.config.repository.is_some() || crate::git::list_remotes(store.data_dir()).is_empty()
    {
        return;
    }
    let id = &project.id;
    report.issues.push(Issue {
        severity: Severity::Warning,
        project: id.clone(),
        code: None,
        message: format!(
            "{id} records no code repository, so the board README cannot link the code it tracks \
             — `kanbanr project edit {id} --repository <URL>` records it (`--repository-private on` \
             names it without a link)"
        ),
    });
}

/// A board without its generated README (FEAT-201): someone who clones it is not told how to see
/// it. Said until `kanbanr readme` writes one.
fn scan_readme(store: &Store, project: &str, report: &mut Report) {
    if !crate::boardreadme::present(store) {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: "this board has no README telling a reader how to view it — `kanbanr readme` \
                      writes one"
                .to_string(),
        });
    } else if !crate::git::list_remotes(store.data_dir()).is_empty()
        && crate::boardreadme::stale(store)
    {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.to_string(),
            code: None,
            message: "the board README's generated block is out of date — `kanbanr readme` or \
                      `kanbanr claude sync` refreshes it"
                .to_string(),
        });
    }
}

/// How a reference reads in a message.
fn describe(r: &crate::refs::Ref) -> String {
    use crate::refs::Ref;
    match r {
        Ref::Item {
            code,
            requirement: Some(req),
        } => format!("{code}/{req}"),
        Ref::Item { code, .. } | Ref::Milestone { code } => code.clone(),
        Ref::Adr { id } | Ref::Goal { id } => id.clone(),
        Ref::Doc { path } | Ref::RepoDoc { path } => path.clone(),
    }
}

/// How old a temporary file of the atomic write must be before doctor names it: long enough that a
/// write in flight, on a slow disk or under a stopped debugger, is never reported (DEF-025).
const LEFTOVER_AGE: std::time::Duration = std::time::Duration::from_secs(3600);
const LEFTOVER_DEPTH: usize = 6;

/// Temporary files left by an interrupted write (DEF-025): one warning each. `id` is a project
/// id, or `"board"` for the board's own folders outside `projects/`. Deletes nothing: a repair is
/// the person's explicit act (ADR-0005, factor 12).
fn scan_leftover_writes(store: &Store, id: &str, report: &mut Report) {
    fn walk(dir: &Path, depth: usize, skip: &[&str], out: &mut Vec<PathBuf>) {
        out.extend(crate::atomic::leftovers(dir, LEFTOVER_AGE));
        if depth >= LEFTOVER_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !skip.contains(&e.file_name().to_string_lossy().as_ref()))
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for d in dirs {
            walk(&d, depth + 1, &[".git"], out);
        }
    }
    let mut found = Vec::new();
    if id == "board" {
        walk(store.data_dir(), 0, &[".git", "projects"], &mut found);
    } else {
        walk(&store.project_dir(id), 0, &[".git"], &mut found);
    }
    for path in found {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: id.to_string(),
            code: None,
            message: format!(
                "{} is left from a write that was interrupted; it is not part of the board and is safe to delete",
                path.display()
            ),
        });
    }
}

fn scan_stray_folders(store: &Store, project: &Project, report: &mut Report) {
    let known: BTreeSet<&str> = project.config.statuses.iter().map(String::as_str).collect();
    // Every folder kanbanr writes at a project's root. A new one goes here too, or doctor calls it
    // stray.
    const EXPECTED: [&str; 11] = [
        "features",
        "milestones",
        "docs",
        "activity",
        "events",
        "pins",
        crate::gates::FOLDER,
        crate::prepared::FOLDER,
        crate::manual::FOLDER,
        crate::timings::FOLDER,
        crate::tasklog::FOLDER,
    ];
    let Ok(entries) = std::fs::read_dir(store.project_dir(&project.id)) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if EXPECTED.contains(&name.as_str()) || name.starts_with('.') {
            continue;
        }
        let items = std::fs::read_dir(entry.path())
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        let note = if known.contains(name.as_str()) {
            "a status folder at the project root — it belongs under features/ and will move on the next write"
        } else if items == 0 {
            "an empty folder matching no status and no part of the layout — most likely left by a status rename"
        } else {
            "a folder matching no status and no part of the layout, and it is not empty"
        };
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: None,
            message: format!("{name}/ is {note}"),
        });
    }
}

/// Verdicts the schema-5 migration could not verify from the board's history (FEAT-187 R-5): each
/// reads as lapsed, and is named here so that someone gives it again.
fn scan_unverified(project: &Project, report: &mut Report) {
    use crate::migrate::UNVERIFIED;
    let mut warn = |code: Option<&str>, message: String| {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: project.id.clone(),
            code: code.map(String::from),
            message,
        })
    };
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        if def.approval.as_ref().is_some_and(|a| a.rev == UNVERIFIED) {
            warn(
                Some(&f.code),
                format!(
                    "approval of {} could not be verified during the schema-5 migration; approve \
                     again",
                    f.code
                ),
            );
        }
        let names: BTreeSet<&str> = def.signoffs.iter().map(|s| s.id.as_str()).collect();
        for name in names {
            if def
                .signoff_latest(name)
                .is_some_and(|s| s.verdict.is_given() && s.rev == UNVERIFIED)
            {
                warn(
                    Some(&f.code),
                    format!(
                        "sign-off '{name}' of {} could not be verified during the schema-5 \
                         migration; sign off again",
                        f.code
                    ),
                );
            }
        }
    }
    if project
        .config
        .process
        .as_ref()
        .is_some_and(|s| s.rev == UNVERIFIED)
    {
        warn(
            None,
            "the process this project's workflow came from could not be verified during the \
             schema-5 migration; apply it again with `kanbanr process update`"
                .to_string(),
        );
    }
}

/// Is `s` shaped as an item code: `^[A-Z][A-Z0-9]*-[0-9]+$`?
fn code_shaped(s: &str) -> bool {
    let Some((prefix, number)) = s.rsplit_once('-') else {
        return false;
    };
    let mut chars = prefix.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// The alias contract's rule 1 (FTR-056): within a project every item's code and every earlier code
/// is distinct, and an earlier code is shaped as a code. One error per break.
fn scan_aliases(project: &Project, report: &mut Report) {
    let mut error = |code: &str, message: String| {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: project.id.clone(),
            code: Some(code.to_string()),
            message,
        });
    };
    for (i, a) in project.features.iter().enumerate() {
        for x in &a.aliases {
            if !code_shaped(x) {
                error(
                    &a.code,
                    format!("{}'s earlier code '{x}' is not shaped as a code", a.code),
                );
                continue;
            }
            for b in &project.features {
                if b.code == *x {
                    error(
                        &a.code,
                        format!("{}'s earlier code {x} is {}'s code", a.code, b.code),
                    );
                }
            }
            // Both items carry it: reported once, from the first of the two.
            for b in &project.features[i + 1..] {
                if b.aliases.contains(x) {
                    error(
                        &a.code,
                        format!("{x} is an earlier code of both {} and {}", a.code, b.code),
                    );
                }
            }
        }
    }
}

fn scan_project(project: &Project, existing: &BTreeSet<String>, report: &mut Report) {
    let pid = &project.id;

    // A board newer than this binary cannot be read correctly at all — but `run` only gets here
    // when the load succeeded, so this is the belt to the load guard's braces: it catches a project
    // whose config was stamped forward while the process was running (FEAT-072).
    if project.config.schema_version > CURRENT_SCHEMA_VERSION {
        report.issues.push(Issue {
            severity: Severity::Error,
            project: pid.clone(),
            code: None,
            message: format!(
                "config schema_version {} is NEWER than this build understands ({}) — upgrade the \
                 binary and restart anything long-running, such as `kanbanr serve`",
                project.config.schema_version, CURRENT_SCHEMA_VERSION
            ),
        });
    }

    // Outdated schema_version (warning).
    if project.config.schema_version < project.config.required_schema_version() {
        report.issues.push(Issue {
            severity: Severity::Warning,
            project: pid.clone(),
            code: None,
            message: format!(
                "config schema_version {} is older than current {} (re-save to migrate)",
                project.config.schema_version,
                project.config.required_schema_version()
            ),
        });
    }

    scan_unverified(project, report);
    scan_aliases(project, report);

    let milestones: BTreeSet<&str> = project.milestones.iter().map(|m| m.code.as_str()).collect();

    for f in &project.features {
        // Unknown milestone (error).
        if !milestones.contains(f.milestone.as_str()) {
            report.issues.push(Issue {
                severity: Severity::Error,
                project: pid.clone(),
                code: Some(f.code.clone()),
                message: format!("references unknown milestone '{}'", f.milestone),
            });
        }

        // Dangling dependency (error): each `depends_on` ref must resolve to an existing feature.
        for dep in &f.depends_on {
            let (dp, dc) = parse_ref(dep, pid);
            if !existing.contains(&qualify(&dp, &dc)) {
                report.issues.push(Issue {
                    severity: Severity::Error,
                    project: pid.clone(),
                    code: Some(f.code.clone()),
                    message: format!(
                        "dangling dependency '{dep}' (resolves to {dp}:{dc}, which does not exist)"
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, TestRef, Zachman};

    fn leftover_in(store: &Store, age_secs: u64) -> std::path::PathBuf {
        let path = store
            .project_dir("demo")
            .join("features")
            .join(".f.yaml.1.1.tmp");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "half").unwrap();
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
        path
    }

    /// DEF-025 R-7.
    #[test]
    fn a_stale_temporary_file_is_reported_and_left_alone() {
        let (store, dir) = fixture();
        let path = leftover_in(&store, 2 * 3600);
        let report = run(&store).unwrap();
        let named: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Warning && i.message.contains(".f.yaml.1.1.tmp"))
            .collect();
        assert_eq!(named.len(), 1, "{:?}", report.issues);
        assert!(named[0].message.contains("safe to delete"));
        assert!(path.exists(), "doctor deletes nothing");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "half");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-025 R-7.
    #[test]
    fn a_fresh_temporary_file_is_not_reported() {
        let (store, dir) = fixture();
        leftover_in(&store, 0);
        let report = run(&store).unwrap();
        assert!(
            !report.issues.iter().any(|i| i.message.contains(".tmp")),
            "{:?}",
            report.issues
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-056 R-9: each way to break "a code names one item for ever" is one error; a renamed
    /// board and a reference through an earlier code are clean.
    #[test]
    fn an_alias_that_breaks_one_code_one_item_is_reported() {
        let (store, dir) = fixture();
        for t in ["A", "B", "C"] {
            store.add_feature("demo", t, "", "M", None).unwrap();
        }
        store
            .edit_feature("demo", "FEAT-001", None, None, None, Some("DEF-001".into()))
            .unwrap();
        let issues = |project: &Project| {
            let mut r = Report::default();
            scan_aliases(project, &mut r);
            r.issues.into_iter().map(|i| i.message).collect::<Vec<_>>()
        };
        let mut project = store.load("demo").unwrap();
        assert!(issues(&project).is_empty(), "{:?}", issues(&project));
        // A dependency written under the earlier code is not dangling.
        let mut dep = project.clone();
        dep.features
            .iter_mut()
            .find(|f| f.code == "FEAT-002")
            .unwrap()
            .depends_on = vec!["FEAT-001".into()];
        let existing: BTreeSet<String> = dep
            .features
            .iter()
            .flat_map(|f| f.codes())
            .map(|c| qualify("demo", c))
            .collect();
        let mut r = Report::default();
        scan_project(&dep, &existing, &mut r);
        assert!(
            !r.issues.iter().any(|i| i.message.contains("dangling")),
            "{:?}",
            r.issues
        );

        let set = |p: &mut Project, code: &str, aliases: &[&str]| {
            p.features
                .iter_mut()
                .find(|f| f.code == code)
                .unwrap()
                .aliases = aliases.iter().map(|a| a.to_string()).collect();
        };
        // An earlier code that is another item's code.
        set(&mut project, "FEAT-002", &["FEAT-003"]);
        assert_eq!(
            issues(&project),
            ["FEAT-002's earlier code FEAT-003 is FEAT-003's code"]
        );
        // The same earlier code on two items.
        set(&mut project, "FEAT-002", &["FEAT-009"]);
        set(&mut project, "FEAT-003", &["FEAT-009"]);
        assert_eq!(
            issues(&project),
            ["FEAT-009 is an earlier code of both FEAT-002 and FEAT-003"]
        );
        // An earlier code that is not a code.
        set(&mut project, "FEAT-003", &[]);
        set(&mut project, "FEAT-002", &["feat 9"]);
        assert_eq!(
            issues(&project),
            ["FEAT-002's earlier code 'feat 9' is not shaped as a code"]
        );
        let mut r = Report::default();
        scan_aliases(&project, &mut r);
        assert!(r.has_errors(), "rule 1 broken is an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-doctor-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Nothing is built on reasoning nobody agreed to".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        (store, dir)
    }

    /// A fresh board on a built-in process, with the method, a milestone and the fixture's charter;
    /// under the system temp folder (L-41).
    fn preset_board(p: &str) -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-doctor-path-{p}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "the board is the test's own"
        );
        let store = Store::new(dir.clone());
        store
            .init_project(
                "demo",
                crate::config::preset(p).unwrap().into_config("demo"),
            )
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Nothing is built on reasoning nobody agreed to".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        (store, dir)
    }

    /// An item of `kind` with a statement, goal G-1 and one EARS requirement naming no test, and a
    /// spec with no Design heading.
    fn item_without_design(store: &Store, kind: Option<&str>) -> String {
        let code = store
            .add_feature_of_kind(
                "demo",
                "Pin the toolchain",
                "# Pin the toolchain\n\n## Why\n\nBuilds drift.\n",
                "M",
                None,
                kind,
            )
            .unwrap()
            .code;
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "A pinned toolchain, for the builder, so that builds agree".into(),
                    goals: vec!["G-1".into()],
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "WHEN the project builds, THE SYSTEM SHALL use the pinned toolchain"
                            .into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        code
    }

    fn said_of(store: &Store, code: &str) -> Vec<String> {
        run_project(store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| i.code.as_deref() == Some(code))
            .map(|i| i.message)
            .collect()
    }

    /// What the item's current stage's FULL effective gate (unstripped, as doctor judged before
    /// DEF-069) asks and the item lacks, by the rules `current_stage_gaps` uses.
    fn full_gate_gaps(
        project: &Project,
        charter: &crate::Charter,
        feature: &crate::FeatureItem,
        ctx: &crate::readiness::Context,
    ) -> Vec<crate::readiness::Gap> {
        let gates = project.config.effective_gates();
        let Some(gate) = gates
            .get(&feature.status)
            .filter(|g| g.applies_to(feature.kind.as_deref()))
        else {
            return Vec::new();
        };
        let goal_ids = charter.goal_ids();
        let mut gaps = crate::readiness::evaluate_entering(
            feature,
            Some(&goal_ids),
            &gate.requires,
            ctx,
            &feature.status,
        );
        gaps.extend(crate::readiness::evaluate_entering(
            feature,
            Some(&goal_ids),
            &gate.warns,
            ctx,
            &feature.status,
        ));
        gaps.retain(doctor_relevant);
        gaps
    }

    /// DEF-069 R-1, R-2: on every built-in process, at the build stage, a feature (no path) is
    /// judged by the full gate as before; a chore is judged by the full gate less exactly what its
    /// path leaves out (the Design section), so every other condition the stage asks still warns.
    #[test]
    fn a_chore_is_judged_by_its_own_path() {
        use crate::readiness::{Check, Condition};
        for p in ["togaf", "agile", "scrum", "design-control"] {
            let (store, dir) = preset_board(p);
            let chore = item_without_design(&store, Some("chore"));
            let feature = item_without_design(&store, None);
            let config = store.load("demo").unwrap().config;
            let build = config
                .role_status(crate::config::Role::Build)
                .unwrap_or_else(|| panic!("{p} has a build stage"));

            let stages = config.path_for(Some("chore")).unwrap().stages.clone();
            let at = stages
                .iter()
                .position(|s| *s == build)
                .unwrap_or_else(|| panic!("{p}: the chore path {stages:?} passes {build}"));
            for stage in &stages[1..=at] {
                store.move_feature("demo", &chore, stage).unwrap();
            }
            let shown = &config.displayed_states;
            let at = shown.iter().position(|s| *s == build).unwrap();
            for stage in &shown[1..=at] {
                store.move_feature("demo", &feature, stage).unwrap();
            }

            let project = store.load("demo").unwrap();
            let charter = crate::charter::load(&store, "demo").unwrap();
            let ctx = crate::readiness::Context::load(&store, "demo", &project.config);
            let judged = |code: &str| {
                let f = project.feature(code).unwrap();
                (
                    current_stage_gaps(&project, &charter, f, &ctx),
                    full_gate_gaps(&project, &charter, f, &ctx),
                )
            };

            // A feature has no path: judged exactly as before, the Design section included.
            let (now, before) = judged(&feature);
            assert_eq!(now, before, "{p}: a feature is judged as before");
            assert!(
                now.iter().any(|g| g.check == Check::SpecSections),
                "{p}: {now:?}"
            );

            // A chore: the full gate's gaps less those its path leaves out, and nothing else.
            let (now, before) = judged(&chore);
            assert!(
                before.iter().any(|g| g.check == Check::SpecSections),
                "{p}: the full gate asks a chore for the Design section (the DEF-069 warning)"
            );
            // The Design's subsections (FTR-006) go with the Design section the path leaves out.
            let expected: Vec<_> = before
                .into_iter()
                .filter(|g| g.check != Check::SpecSections && g.check != Check::DesignSections)
                .collect();
            assert_eq!(
                now, expected,
                "{p}: a chore loses only what its path leaves out"
            );
            assert!(
                !now.iter().any(|g| g.message.contains("'Design' section")),
                "{p}: {now:?}"
            );
            // The stage's own conditions all still stand for the chore (approved, in_sprint,
            // zachman, tests_named, … as the preset asks), only the Design section is gone.
            let full = config.effective_gates()[&build].clone();
            let mut kept = full.requires.clone();
            kept.retain_mut(|c| match c {
                Condition::Check(Check::SpecSections) => false,
                Condition::SpecSections { spec_sections } => {
                    spec_sections.retain(|s| !s.trim().eq_ignore_ascii_case("Design"));
                    !spec_sections.is_empty()
                }
                _ => true,
            });
            let gate = config.gate_for_kind(&build, Some("chore")).unwrap();
            assert_eq!(gate.requires, kept, "{p}");
            assert_eq!(gate.warns, full.warns, "{p}: warns are not stripped (Q2)");
            assert!(!kept.is_empty(), "{p}: the stage still asks something");

            // And doctor's report says the same for the chore.
            let said = said_of(&store, &chore);
            assert!(
                !said.iter().any(|m| m.contains("'Design' section")),
                "{p}: {said:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// DEF-069 R-2: a feature, which has no path, is still warned at the build stage that its spec
    /// has no Design section.
    #[test]
    fn a_feature_at_the_build_stage_still_needs_its_design() {
        let (store, dir) = preset_board("togaf");
        let code = item_without_design(&store, None);
        for stage in ["Business Arch", "System Design", "Implementation"] {
            store.move_feature("demo", &code, stage).unwrap();
        }
        let said = said_of(&store, &code);
        assert!(
            said.iter()
                .any(|m| m.contains("the spec has no 'Design' section")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-070 R-1: on the togaf preset with a chore path that leaves docs out (set on the config's
    /// `paths` in Rust before the board is made, B-28), a chore that names no docs is not warned at
    /// the stage whose gate asks for docs_updated, nor counted one stage before it; a feature, which
    /// has no path, still is.
    #[test]
    fn a_path_that_leaves_docs_out_gets_no_docs_warning() {
        use crate::method::Part;
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-doctor-docs-off-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "the board is the test's own"
        );
        let mut config = crate::config::preset("togaf").unwrap().into_config("demo");
        let mut path = config
            .paths
            .get("chore")
            .cloned()
            .expect("togaf has a chore path");
        if !path.method_off.contains(&Part::Docs) {
            path.method_off.push(Part::Docs);
        }
        config.paths.insert("chore".into(), path);
        let store = Store::new(dir.clone());
        store.init_project("demo", config).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Nothing is built on reasoning nobody agreed to".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let chore = item_without_design(&store, Some("chore"));
        let feature = item_without_design(&store, None);
        let config = store.load("demo").unwrap().config;
        assert!(
            config
                .gate_for_kind("Migration", None)
                .unwrap()
                .requires
                .contains(&crate::readiness::Condition::Check(
                    crate::readiness::Check::DocsUpdated
                )),
            "Migration asks for docs_updated"
        );

        // One stage before Migration: the feature is counted, the chore is not.
        store
            .move_feature("demo", &chore, "Implementation")
            .unwrap();
        for stage in ["Business Arch", "System Design", "Implementation"] {
            store.move_feature("demo", &feature, stage).unwrap();
        }
        let project = store.load("demo").unwrap();
        let charter = crate::charter::load(&store, "demo").unwrap();
        let ctx = crate::readiness::Context::load(&store, "demo", &project.config);
        let mut report = Report::default();
        scan_declared_docs(&project, &charter, &ctx, &mut report);
        let counted = |code: &str| {
            report
                .ahead_seen
                .contains(&("demo".into(), "Migration".into(), code.to_string()))
        };
        assert!(counted(&feature), "{:?}", report.ahead);
        assert!(!counted(&chore), "{:?}", report.ahead);

        // At Migration: doctor warns for the feature only.
        store.move_feature("demo", &chore, "Migration").unwrap();
        store.move_feature("demo", &feature, "Migration").unwrap();
        let warned = |code: &str| {
            said_of(&store, code)
                .iter()
                .filter(|m| m.contains("declares no docs"))
                .count()
        };
        assert_eq!(warned(&feature), 1, "{:?}", said_of(&store, &feature));
        assert_eq!(warned(&chore), 0, "{:?}", said_of(&store, &chore));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A board with releases and sprints on, and v1 added.
    fn release_board() -> (Store, std::path::PathBuf) {
        let (store, dir) = fixture();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&serde_json::json!({"releases": true, "sprints": true})),
        )
        .unwrap();
        crate::releases::add(&store, "demo", "v1", "", "").unwrap();
        (store, dir)
    }

    fn item_in(store: &Store, release: &str, finished: bool) -> String {
        let code = store
            .add_feature("demo", "Item", "", "M", None)
            .unwrap()
            .code;
        if finished {
            for s in ["Scheduled", "Completed"] {
                store.move_feature("demo", &code, s).unwrap();
            }
        }
        crate::releases::plan(store, "demo", release, std::slice::from_ref(&code)).unwrap();
        code
    }

    fn said(store: &Store, needle: &str) -> Vec<String> {
        run_project(store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| i.message.contains(needle))
            .map(|i| i.message)
            .collect()
    }

    /// DEF-029 R-3: an item planned into a shipped release that is not at an end status is named,
    /// with the release and its date, its status and what holds it.
    #[test]
    fn doctor_names_an_item_a_shipped_release_left_unfinished() {
        let (store, dir) = release_board();
        let code = item_in(&store, "v1", true);
        crate::releases::cut(&store, "demo", "v1").unwrap();
        assert!(
            said(&store, "shipped on").is_empty(),
            "shipped and finished"
        );
        // It slips back after the cut, as three items did in the walkthrough.
        store.move_feature("demo", &code, "Scheduled").unwrap();
        let found = said(&store, "shipped on");
        assert_eq!(found.len(), 1, "{found:?}");
        let m = &found[0];
        assert!(
            m.contains(&format!("{code} is planned into v1, which shipped on 20"))
                && m.contains("but is at Scheduled")
                && m.contains("plan it into a coming release"),
            "{m}"
        );
        let issue = run_project(&store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .find(|i| i.message.contains("shipped on"))
            .unwrap();
        assert_eq!(issue.code.as_deref(), Some(code.as_str()));
        assert!(matches!(issue.severity, Severity::Warning));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-029 R-3: an item the cut carried into another release is not named.
    #[test]
    fn doctor_is_quiet_about_a_carried_item() {
        let (store, dir) = release_board();
        crate::releases::add(&store, "demo", "v2", "", "").unwrap();
        let carried = item_in(&store, "v1", false);
        crate::releases::cut(&store, "demo", "v1").unwrap();
        let board = store.load("demo").unwrap();
        assert_eq!(
            board.feature(&carried).unwrap().release.as_deref(),
            Some("v2")
        );
        assert!(said(&store, "shipped on").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-029 R-4: while a sprint runs, a live item planned into an unshipped release and into no
    /// sprint is named, with the command that plans it in.
    #[test]
    fn doctor_names_a_release_item_missing_from_the_running_sprint() {
        let (store, dir) = release_board();
        let code = item_in(&store, "v1", false);
        crate::sprints::add(
            &store,
            "demo",
            "2026-10-05",
            Some(5),
            "",
            None,
            "",
            None,
            None,
        )
        .unwrap();
        crate::sprints::start(&store, "demo", "SP-001").unwrap();
        let found = said(&store, "into no sprint");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains(&format!(
                "{code} is planned into v1 but into no sprint, and SP-001 is running: kanbanr sprint plan SP-001 {code}"
            )),
            "{}",
            found[0]
        );
        crate::sprints::plan(&store, "demo", "SP-001", std::slice::from_ref(&code)).unwrap();
        assert!(
            said(&store, "into no sprint").is_empty(),
            "planned in, so quiet"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-041 R-2: a sprint stored active but dated ahead is named, with the way out.
    #[test]
    fn a_sprint_started_before_its_first_day_is_reported() {
        let (store, dir) = release_board();
        std::fs::write(
            store.project_dir("demo").join("sprints.yaml"),
            "- code: SP-001\n  start: 2099-01-05\n  end: 2099-01-18\n  state: active\n",
        )
        .unwrap();
        let found = said(&store, "started before its first day");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("SP-001") && found[0].contains("2099-01-05"),
            "{}",
            found[0]
        );
        assert!(
            found[0].contains("kanbanr sprint start SP-001"),
            "{}",
            found[0]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-029 R-4: with no sprint running there is nothing to say.
    #[test]
    fn doctor_is_quiet_when_no_sprint_runs() {
        let (store, dir) = release_board();
        item_in(&store, "v1", false);
        assert!(said(&store, "into no sprint").is_empty());
        crate::sprints::add(
            &store,
            "demo",
            "2026-10-05",
            Some(5),
            "",
            None,
            "",
            None,
            None,
        )
        .unwrap();
        assert!(
            said(&store, "into no sprint").is_empty(),
            "planned, not started"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-217 R-11: codes a hand edit broke are a doctor error naming the entry, and allocation
    /// is refused naming the same problem; valid codes say nothing.
    #[test]
    fn doctor_reports_invalid_codes() {
        let (store, dir) = fixture();
        assert!(
            std::fs::canonicalize(&dir)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        let about_codes = |r: &Report| -> Vec<Issue> {
            r.issues
                .iter()
                .filter(|i| i.message.starts_with("codes:"))
                .cloned()
                .collect()
        };
        store
            .set_codes(
                "demo",
                crate::config::CodesChange {
                    set: [("defect".to_string(), "DEF".to_string())].into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(about_codes(&run_project(&store, "demo").unwrap()).is_empty());

        let mut cfg = store.read_project_config("demo").unwrap();
        cfg.codes.kinds.insert("spike".into(), "ADR".into());
        store.save_config("demo", &cfg).unwrap();
        let issues = about_codes(&run_project(&store, "demo").unwrap());
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("kind 'spike'"), "{issues:?}");
        let err = store
            .add_feature_of_kind("demo", "x", "", "M", None, Some("defect"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("kind 'spike'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-200 R-11: doctor lists every diagram lint finding on the board — in each item's spec
    /// and in each markdown doc — naming file, block, line and rule; clean diagrams say nothing.
    #[test]
    fn doctor_lists_diagram_findings() {
        let (store, dir) = fixture();
        let spec = "# Spec\n\n```mermaid\nsequenceDiagram\n  A->>B: one; two\n```\n";
        let item = store.add_feature("demo", "Flow", spec, "M", None).unwrap();
        let other = store
            .add_feature("demo", "Other", "```mermaid\nflowchrt\n```\n", "M", None)
            .unwrap();
        store
            .add_feature("demo", "Clean", "```mermaid\npie\n```\n", "M", None)
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/flow.md",
                "# Flow\n\n```mermaid\nflowchart LR\n  A --> end\n```\n",
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "notes.md",
                "```mermaid\nflowchart LR\n  A --> B\n```\n",
            )
            .unwrap();

        for report in [run(&store).unwrap(), run_project(&store, "demo").unwrap()] {
            let found: Vec<&Issue> = report
                .issues
                .iter()
                .filter(|i| i.message.contains("diagram #"))
                .collect();
            assert_eq!(found.len(), 3, "{found:#?}");
            assert!(found.iter().all(|i| i.severity == Severity::Warning));
            let spec_issue = found
                .iter()
                .find(|i| i.code.as_deref() == Some(item.code.as_str()))
                .unwrap();
            assert_eq!(
                spec_issue.message,
                format!(
                    "{} spec, diagram #1, line 5: `;` ends the statement early, so the rest of the \
                     message is lost — use a comma or words (semicolon_in_text)",
                    item.code
                )
            );
            assert!(
                found
                    .iter()
                    .any(|i| i.code.as_deref() == Some(other.code.as_str())
                        && i.message.ends_with("(unknown_type)"))
            );
            assert!(found.iter().any(|i| {
                i.code.is_none()
                    && i.message
                        .starts_with("docs/architecture/flow.md, diagram #1, line 5:")
                    && i.message.ends_with("(end_as_id)")
            }));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-016 R-2: doctor lists a query block that does not parse, with its doc, line and
    /// reason; a good block is not listed.
    #[test]
    fn doctor_lists_bad_query_blocks() {
        let (store, dir) = fixture();
        store
            .write_doc(
                "demo",
                "arch/views.md",
                "# V\n\n```katalogr-query\nstructure\n```\n\n```katalogr-query\nnonsense\n```\n",
            )
            .unwrap();
        for report in [run(&store).unwrap(), run_project(&store, "demo").unwrap()] {
            let found: Vec<&Issue> = report
                .issues
                .iter()
                .filter(|i| i.message.contains("query block"))
                .collect();
            assert_eq!(found.len(), 1, "{found:#?}");
            assert_eq!(found[0].severity, Severity::Warning);
            assert!(
                found[0]
                    .message
                    .starts_with("docs/arch/views.md, query block, line 7:"),
                "{}",
                found[0].message
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The repository doc link messages a report holds (FTR-004).
    fn repo_link_messages(report: &Report) -> Vec<(Severity, String)> {
        report
            .issues
            .iter()
            .filter(|i| i.message.contains("in the code repository at"))
            .map(|i| (i.severity, i.message.clone()))
            .collect()
    }

    /// FTR-004 R-6: a spec's repository doc link is judged at the item's position: a doc on its
    /// branch and not on main is no error; a doc absent from its branch is, naming the position;
    /// an item with no branch is judged at main, and the message says so.
    #[test]
    fn a_repository_link_is_checked_at_the_items_position() {
        use crate::coderepo::git;
        use crate::coderepo::tests::{commit_file, scratch_repo};
        let (store, dir) = fixture();
        assert!(store.data_dir().starts_with(std::env::temp_dir()));
        let repo = scratch_repo("doctor-repo-links");
        assert!(repo.starts_with(std::env::temp_dir()));
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        commit_file(&repo, "docs/guide/main.md", "# on main\n", "main doc");
        let a = store
            .add_feature("demo", "On a branch", "", "M", None)
            .unwrap();
        let b = store
            .add_feature("demo", "Not started", "", "M", None)
            .unwrap();
        let branch = format!("feat/{}-on-a-branch", a.code);
        git(&repo, &["checkout", "-q", "-b", &branch, "main"]).unwrap();
        commit_file(
            &repo,
            "docs/guide/new.md",
            "# on the branch\n",
            "branch doc",
        );
        let tip = crate::coderepo::rev_parse(&repo, "HEAD").unwrap();
        git(&repo, &["checkout", "-q", "main"]).unwrap();
        store
            .edit_feature(
                "demo",
                &a.code,
                None,
                Some("See [new](docs/guide/new.md).\nAnd [gone](docs/guide/gone.md).".into()),
                None,
                None,
            )
            .unwrap();
        store
            .edit_feature(
                "demo",
                &b.code,
                None,
                Some("See [main](docs/guide/main.md) and [new](./docs/guide/new.md).".into()),
                None,
                None,
            )
            .unwrap();

        let report = run_project_in(&store, "demo", Some(&repo)).unwrap();
        let main = crate::coderepo::rev_parse(&repo, "main").unwrap();
        assert_eq!(
            repo_link_messages(&report),
            vec![
                (
                    Severity::Error,
                    format!(
                        "{a} spec:2 links docs/guide/gone.md, which is not in the code repository \
                         at {a} · {branch} @ {}",
                        &tip[..7],
                        a = a.code
                    )
                ),
                (
                    Severity::Error,
                    format!(
                        "{b} spec:1 links docs/guide/new.md, which is not in the code repository \
                         at main @ {} (no branch yet)",
                        &main[..7],
                        b = b.code
                    )
                ),
            ]
        );
        // None of them is a broken board doc.
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.message.contains("which this board does not have")),
            "{:?}",
            report.issues
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-6: without the code repository, one warning says how many repository doc links
    /// were not checked; none when no spec links one (L-29).
    #[test]
    fn repository_links_are_not_checked_without_a_repository_and_doctor_says_so_once() {
        let (store, dir) = fixture();
        let warned = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("repository doc links were not checked"))
                .map(|i| {
                    assert_eq!(i.severity, Severity::Warning);
                    assert_eq!(i.code, None);
                    i.message
                })
                .collect()
        };
        let a = store.add_feature("demo", "Links", "", "M", None).unwrap();
        store
            .edit_feature(
                "demo",
                &a.code,
                None,
                Some(
                    "No repository doc here: [d](architecture/data.md), `docs/guide/x.md`.".into(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(warned(&store), Vec::<String>::new());
        let b = store.add_feature("demo", "More", "", "M", None).unwrap();
        store
            .edit_feature(
                "demo",
                &a.code,
                None,
                Some("[x](docs/guide/x.md)\n[y](docs/guide/y.md)".into()),
                None,
                None,
            )
            .unwrap();
        store
            .edit_feature(
                "demo",
                &b.code,
                None,
                Some("[z](docs/system/z.md)".into()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            warned(&store),
            vec![
                "3 repository doc links were not checked: run doctor from the project folder"
                    .to_string()
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-181: a link to an item, requirement or doc the board does not have is reported, with
    /// where it is; a link that resolves, and a token that is not this board's, are not.
    #[test]
    fn broken_board_references_reported() {
        let (store, _dir) = fixture();
        let a = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let spec = format!(
            "Builds on {code}. See [gone](/p/demo/feature/FEAT-999) and docs/missing.md.\n\
             Also FEAT-998, UTF-8 and [ok](/p/demo/feature/{code}).",
            code = a.code
        );
        store
            .edit_feature("demo", &a.code, None, Some(spec), None, None)
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/data.md",
                "Serves G-1 and G-7; see [x](nowhere.md).",
            )
            .unwrap();

        let issues: Vec<(Severity, String)> = run_project(&store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| i.message.contains("which this board does not have"))
            .map(|i| (i.severity, i.message))
            .collect();
        let spec = format!("{} spec", a.code);
        assert_eq!(
            issues,
            vec![
                (Severity::Error, format!("{spec}:1 refers to FEAT-999, which this board does not have")),
                (Severity::Warning, format!("{spec}:2 refers to FEAT-998, which this board does not have")),
                (Severity::Warning, "docs/architecture/data.md:1 refers to G-7, which this board does not have".to_string()),
                (Severity::Error, "docs/architecture/data.md:1 refers to nowhere.md, which this board does not have".to_string()),
            ]
        );
    }

    fn broken_doc_messages(store: &Store) -> Vec<(Severity, String)> {
        run_project(store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| i.message.contains("which this board does not have"))
            .map(|i| (i.severity, i.message))
            .collect()
    }

    /// DEF-016 R-1: a shipped release's relative book links are not reported as broken.
    #[test]
    fn shipped_notes_book_links_are_not_broken() {
        let (store, _dir) = fixture();
        store
            .write_doc(
                "demo",
                "releases/v1.0.0.md",
                "See [p](../using/processes.md).",
            )
            .unwrap();
        assert_eq!(broken_doc_messages(&store), vec![]);
    }

    /// DEF-016 R-2: an item a shipped release names that the board lacks is still reported.
    #[test]
    fn shipped_notes_still_report_an_unknown_item() {
        let (store, _dir) = fixture();
        store
            .write_doc(
                "demo",
                "releases/v1.0.0.md",
                "Fixes [x](/p/demo/feature/FEAT-999) and [p](../using/processes.md).",
            )
            .unwrap();
        assert_eq!(
            broken_doc_messages(&store),
            vec![(
                Severity::Error,
                "docs/releases/v1.0.0.md:1 refers to FEAT-999, which this board does not have"
                    .to_string()
            )]
        );
    }

    /// DEF-016 R-3: a doc outside releases/ still reports a broken link.
    #[test]
    fn other_docs_still_report_a_broken_link() {
        let (store, _dir) = fixture();
        store
            .write_doc(
                "demo",
                "architecture/x.md",
                "See [p](../using/processes.md).",
            )
            .unwrap();
        let got = broken_doc_messages(&store);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, Severity::Error);
    }

    /// FTR-006 R-8: an `affects_views` id the catalog does not have is an Error naming the item;
    /// on a project without a catalog any `affects_views` is a Warning.
    #[test]
    fn unknown_affects_views_reported() {
        use crate::catalog::{Catalog, Level, Row, Source, SourceKind};
        let (store, _dir) = fixture();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let set = |views: &[&str]| {
            store
                .set_feature_definition(
                    "demo",
                    &item.code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart".into(),
                        affects_views: views.iter().map(|v| v.to_string()).collect(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        };
        let about = |store: &Store| {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| {
                    i.code.as_deref() == Some(item.code.as_str())
                        && i.message.contains("affects_views")
                })
                .map(|i| (i.severity, i.message))
                .collect::<Vec<_>>()
        };
        // Nothing named: nothing said, with or without a catalog.
        set(&[]);
        assert!(about(&store).is_empty());
        // Named, no catalog: a warning.
        set(&["context"]);
        let got = about(&store);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, Severity::Warning);
        assert!(got[0].1.contains("no catalog"), "{}", got[0].1);
        // A catalog with `context`: a known row is quiet, an unknown one is an error.
        crate::catalog::save(
            &store,
            "demo",
            &Catalog {
                schema: crate::catalog::SCHEMA,
                rows: vec![Row {
                    id: "context".into(),
                    title: "Context".into(),
                    answers: vec!["what".into()],
                    for_: vec!["architect".into()],
                    level: Level::System,
                    source: Source {
                        kind: SourceKind::Written,
                        path: "architecture/context.md".into(),
                        ..Source::default()
                    },
                    ..Row::default()
                }],
                ..Catalog::default()
            },
        )
        .unwrap();
        assert!(about(&store).is_empty(), "{:?}", about(&store));
        set(&["context", "nope"]);
        let got = about(&store);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, Severity::Error);
        assert!(
            got[0].1.contains(&item.code) && got[0].1.contains("nope"),
            "{}",
            got[0].1
        );
    }

    /// FEAT-180 R-4 (doctor): a declared doc the board does not have is an error; an item whose
    /// current stage asks for docs_updated but names no docs is warned about, and one that names
    /// its docs is not. While only the next stage asks (DEF-021), it is counted, not warned.
    #[test]
    fn affects_docs_unknown_and_undeclared_reported() {
        let (store, _dir) = fixture();
        let named = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let silent = store
            .add_feature("demo", "Checkout", "", "M", None)
            .unwrap();
        store
            .write_doc("demo", "architecture/data.md", "# Data")
            .unwrap();
        for (code, docs) in [
            (
                &named.code,
                vec!["architecture/data.md".to_string(), "nope.md".to_string()],
            ),
            (&silent.code, vec![]),
        ] {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart".into(),
                        affects_docs: docs,
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        let gate = crate::config::Gate {
            requires: vec![crate::readiness::Condition::Check(
                crate::readiness::Check::DocsUpdated,
            )],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();

        let issues = run_project(&store, "demo").unwrap().issues;
        let about = |code: &str, needle: &str| {
            issues
                .iter()
                .filter(|i| i.code.as_deref() == Some(code) && i.message.contains(needle))
                .map(|i| (i.severity, i.message.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            about(&named.code, "affects_docs"),
            vec![(
                Severity::Error,
                format!(
                    "{} affects_docs refers to nope.md, which this board does not have",
                    named.code
                )
            )]
        );
        assert_eq!(about(&named.code, "declares no docs"), vec![]);
        // Scheduled is only the next stage so far: a count, not a warning.
        assert_eq!(about(&silent.code, "declares no docs"), vec![]);
        let report = run_project(&store, "demo").unwrap();
        assert!(
            report
                .ahead
                .iter()
                .any(|a| a.stage == "Scheduled" && a.items >= 1),
            "{:?}",
            report.ahead
        );
        // Once the item is at Scheduled, the stage asks now.
        store
            .move_feature("demo", &silent.code, "Scheduled")
            .unwrap();
        let issues = run_project(&store, "demo").unwrap().issues;
        let warned: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.code.as_deref() == Some(silent.code.as_str())
                    && i.message.contains("declares no docs")
            })
            .collect();
        assert_eq!(warned.len(), 1, "{issues:?}");
        assert_eq!(warned[0].severity, Severity::Warning);
    }

    /// DEF-075 R-3: on the board `affects_docs_unknown_and_undeclared_reported` builds, doctor's
    /// 'declares no docs' message names `affects_docs`, `affects_no_docs` and the command that
    /// records it; `affects_no_docs` silences it; an item recording both docs and
    /// `affects_no_docs` is told to remove one.
    #[test]
    fn undeclared_docs_warning_names_affects_no_docs() {
        let (store, _dir) = fixture();
        let both = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let silent = store
            .add_feature("demo", "Checkout", "", "M", None)
            .unwrap();
        let quiet = store.add_feature("demo", "Pin", "", "M", None).unwrap();
        store
            .write_doc("demo", "architecture/data.md", "# Data")
            .unwrap();
        for (code, docs, none) in [
            (
                &both.code,
                vec!["architecture/data.md".to_string()],
                "a pin",
            ),
            (&silent.code, vec![], ""),
            (&quiet.code, vec![], "test-only"),
        ] {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart".into(),
                        affects_docs: docs,
                        affects_no_docs: none.into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        let gate = crate::config::Gate {
            requires: vec![crate::readiness::Condition::Check(
                crate::readiness::Check::DocsUpdated,
            )],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        // The contradiction is said wherever the item stands; the others stand at Scheduled.
        for code in [&silent.code, &quiet.code] {
            store.move_feature("demo", code, "Scheduled").unwrap();
        }

        let issues = run_project(&store, "demo").unwrap().issues;
        let about = |code: &str, needle: &str| {
            issues
                .iter()
                .filter(|i| i.code.as_deref() == Some(code) && i.message.contains(needle))
                .map(|i| (i.severity, i.message.clone()))
                .collect::<Vec<_>>()
        };
        let warned = about(&silent.code, "declares no docs");
        assert_eq!(warned.len(), 1, "{issues:?}");
        assert_eq!(warned[0].0, Severity::Warning);
        let message = &warned[0].1;
        assert!(message.contains("affects_docs"), "{message}");
        assert!(message.contains("affects_no_docs"), "{message}");
        assert!(
            message.contains(&format!("kanbanr feature define {}", silent.code)),
            "{message}"
        );
        assert_eq!(about(&quiet.code, "declares no docs"), vec![]);
        assert_eq!(about(&both.code, "declares no docs"), vec![]);
        assert_eq!(
            about(&both.code, "also records affects_no_docs"),
            vec![(
                Severity::Warning,
                format!(
                    "{} names docs and also records affects_no_docs; remove one",
                    both.code
                )
            )]
        );
        assert_eq!(about(&quiet.code, "also records"), vec![]);
    }

    /// DEF-047 R-1: an item that names no `affects_docs` but links board docs under its Design's
    /// `Affects docs` is not told it names none, since the gate reads those links; one with
    /// neither still is (L-29).
    #[test]
    fn an_item_that_links_docs_is_not_told_it_names_none() {
        let (store, _dir) = fixture();
        let spec = "## Design\n\n### Affects docs\n\
            - [data](/p/demo/docs/file/architecture/data.md)\n";
        let linked = store.add_feature("demo", "Cart", spec, "M", None).unwrap();
        let bare = store
            .add_feature("demo", "Checkout", "## Design\n\nNo docs.\n", "M", None)
            .unwrap();
        store
            .write_doc("demo", "architecture/data.md", "# Data")
            .unwrap();
        for code in [&linked.code, &bare.code] {
            store
                .set_feature_definition(
                    "demo",
                    code,
                    Some(FeatureDefinition {
                        statement: "Keep a cart".into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
            store.move_feature("demo", code, "Scheduled").unwrap();
        }
        let gate = crate::config::Gate {
            requires: vec![crate::readiness::Condition::Check(
                crate::readiness::Check::DocsUpdated,
            )],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        let issues = run_project(&store, "demo").unwrap().issues;
        let told = |code: &str| {
            issues
                .iter()
                .filter(|i| {
                    i.code.as_deref() == Some(code) && i.message.contains("declares no docs")
                })
                .count()
        };
        assert_eq!(told(&linked.code), 0, "{issues:?}");
        assert_eq!(told(&bare.code), 1, "{issues:?}");
    }

    /// Commit times are whole seconds: keep "entered the stage" and "changed the doc" apart.
    fn next_second() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    /// A git board with one item that declares `declared` and links `linked` under its Design's
    /// `Affects docs` (DEF-074). It moves Planned → Scheduled, the docs in `folded` are written
    /// again and committed, and it moves back to Planned, a live stage, whose gate then asks for
    /// `docs_updated` (set after the moves, so no move is held). Each commit comes before the
    /// comparison that depends on it (L-45). Returns the store, the folder and the item's code.
    fn moved_back_board(
        declared: &[&str],
        linked: &[&str],
        folded: &[&str],
    ) -> (Store, std::path::PathBuf, String) {
        let (store, dir) = fixture();
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "the test's own board"
        );
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let commit = |msg: &str| assert!(crate::git::commit(&dir, "Ada", "ada@example.com", msg));
        for doc in declared.iter().chain(linked) {
            store
                .write_doc("demo", &format!("architecture/{doc}.md"), "# Doc\n\nv1\n")
                .unwrap();
        }
        commit("docs");
        let links: String = linked
            .iter()
            .map(|d| format!("- [{d}](/p/demo/docs/file/architecture/{d}.md)\n"))
            .collect();
        let spec = format!("## Design\n\n### Affects docs\n{links}");
        let item = store.add_feature("demo", "Cart", &spec, "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart".into(),
                    goals: vec!["G-1".into()],
                    affects_docs: declared
                        .iter()
                        .map(|d| format!("architecture/{d}"))
                        .collect(),
                    ..Default::default()
                }),
            )
            .unwrap();
        commit("board");
        next_second();
        store.move_feature("demo", &item.code, "Scheduled").unwrap();
        commit("move");
        next_second();
        for doc in folded {
            store
                .write_doc("demo", &format!("architecture/{doc}.md"), "# Doc\n\nv2\n")
                .unwrap();
        }
        commit("fold");
        next_second();
        store.move_feature("demo", &item.code, "Planned").unwrap();
        commit("move");
        let gate = crate::config::Gate {
            requires: vec![crate::readiness::Condition::Check(
                crate::readiness::Check::DocsUpdated,
            )],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Planned".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        (store, dir, item.code)
    }

    /// The messages doctor gives `code` that say a doc has not changed.
    fn unchanged_said(store: &Store, code: &str) -> Vec<String> {
        run_project(store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| {
                i.code.as_deref() == Some(code) && i.message.contains("has not changed since")
            })
            .map(|i| i.message)
            .collect()
    }

    /// DEF-074 R-1: a declared doc and a linked doc folded before the move are not reported as
    /// unchanged after it — doctor re-judges the passed gate over the window the move used.
    #[test]
    fn a_doc_folded_before_the_move_does_not_warn_after_it() {
        let (store, dir, code) =
            moved_back_board(&["ci-pipeline"], &["data"], &["ci-pipeline", "data"]);
        let said = unchanged_said(&store, &code);
        assert!(said.is_empty(), "{said:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-074 R-1 (DEF-021 R-4, DEF-047): a declared doc and a linked doc not folded in the
    /// move's window still warn after the move, naming the stage the window began in.
    #[test]
    fn a_doc_not_folded_before_the_move_still_warns_after_it() {
        let (store, dir, code) = moved_back_board(
            &["ci-pipeline", "api"],
            &["data", "queue"],
            &["ci-pipeline", "data"],
        );
        let mut said = unchanged_said(&store, &code);
        said.sort();
        let tail =
            format!("has not changed since {code} entered Scheduled — fold the design into it");
        assert_eq!(
            said,
            vec![
                format!("architecture/api.md {tail}"),
                format!("architecture/queue.md {tail}"),
            ],
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-193 Q3: a project from before the method is asked once to choose, and choosing either
    /// way silences it; with `links` off, doctor stops reporting broken references.
    #[test]
    fn method_choice_nudge_and_links_part() {
        let (store, _dir) = fixture();
        let a = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .edit_feature(
                "demo",
                &a.code,
                None,
                Some("See FEAT-999.".into()),
                None,
                None,
            )
            .unwrap();
        let messages = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .map(|i| i.message)
                .collect()
        };
        let before = messages(&store);
        assert!(
            before.iter().any(|m| m.contains("predates the method")),
            "{before:?}"
        );
        assert!(before.iter().any(|m| m.contains("FEAT-999")), "{before:?}");

        store
            .set_method("demo", false, &[crate::method::Part::Links])
            .unwrap();
        let after = messages(&store);
        assert!(
            !after.iter().any(|m| m.contains("predates the method")),
            "{after:?}"
        );
        assert!(
            !after.iter().any(|m| m.contains("FEAT-999")),
            "links off: {after:?}"
        );
    }

    /// FEAT-080: every other gap check rightly stops caring once work is finished — you cannot fix
    /// a missing requirement on a shipped item. "Built without agreement, and still never agreed
    /// to" is different: finishing does not answer it. Six items on this project's own board had
    /// aged out of every report, and producing the list took a script.
    #[test]
    fn an_unreconciled_bypass_outlives_the_terminal_gate() {
        let (store, _dir) = fixture();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart for 7 days".into(),
                    goals: vec!["G-1".into()],
                    started_unapproved: "shipped under a deadline".into(),
                    ..Default::default()
                }),
            )
            .unwrap();

        let bypass = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .map(|i| i.message)
                .filter(|m| m.contains("without approval") || m.contains("without agreement"))
                .collect()
        };

        store.move_feature("demo", &item.code, "Scheduled").unwrap();
        assert_eq!(
            bypass(&store).len(),
            1,
            "in flight and unanswered: it warns"
        );

        // Finishing it used to be how the question stopped being asked.
        store.move_feature("demo", &item.code, "Completed").unwrap();
        let after = bypass(&store);
        assert_eq!(
            after.len(),
            1,
            "a finished item with an unanswered bypass must STILL be reported: {after:?}"
        );
        assert!(
            after[0].contains("finished without agreement"),
            "and it should read as the question it is: {}",
            after[0]
        );
        assert!(
            after[0].contains("kanbanr ratify"),
            "a warning that names no remedy is wallpaper: {}",
            after[0]
        );

        // Ratifying answers it, and the report goes quiet — for the right reason this time.
        store
            .ratify_feature("demo", &item.code, "the user", "reviewed as a set")
            .unwrap();
        assert!(
            bypass(&store).is_empty(),
            "answered after the fact is still answered"
        );
    }

    /// FEAT-068: the warning asked the reader to "review and approve what was actually built". The
    /// user did, for twelve items, and it kept asking — which is how a warning becomes wallpaper.
    #[test]
    fn an_answered_escape_stops_warning() {
        let (store, dir) = fixture();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let definition = FeatureDefinition {
            statement: "Keep a cart for 7 days".into(),
            goals: vec!["G-1".into()],
            zachman: Zachman {
                extra: Default::default(),
                what: "cart persistence".into(),
                how: "server-side".into(),
                where_: "checkout".into(),
                when: "on mutation".into(),
                who: "returning shoppers".into(),
                why: "carts vanish".into(),
            },
            requirements: vec![Requirement {
                id: "R-1".into(),
                text: "THE SYSTEM SHALL retain the cart.".into(),
                tests: vec![TestRef {
                    name: "cart::retains".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            started_unapproved: "shipped under a deadline".into(),
            ..Default::default()
        };
        store
            .set_feature_definition("demo", &item.code, Some(definition.clone()))
            .unwrap();
        store.move_feature("demo", &item.code, "Scheduled").unwrap();

        let unapproved = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("started without approval"))
                .map(|i| i.message)
                .collect()
        };
        assert_eq!(unapproved(&store).len(), 1, "unanswered, so it warns");

        // Approving is the answer the warning asked for.
        store
            .approve_feature("demo", &item.code, "the user")
            .unwrap();
        assert!(
            unapproved(&store).is_empty(),
            "answered, so it stops: a warning that cannot be resolved is wallpaper"
        );

        // The record is kept — work did start before agreement, and that is history.
        let after = store.load("demo").unwrap();
        let after = after.feature(&item.code).unwrap();
        assert_eq!(
            after.definition.as_ref().unwrap().started_unapproved,
            "shipped under a deadline"
        );

        // Changing the definition lapses the approval, and the warning returns with it: the
        // agreement no longer covers what was built.
        let widened = FeatureDefinition {
            statement: "Keep a cart for 30 days".into(),
            ..definition
        };
        store
            .set_feature_definition("demo", &item.code, Some(widened))
            .unwrap();
        assert_eq!(
            unapproved(&store).len(),
            1,
            "scope changed after the yes, so the escape is unanswered again"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FEAT-128: a board whose git config still carries the placeholder identity commits as
    /// nobody; doctor says so, with the command that fixes it, and stops once it is fixed.
    #[test]
    fn a_placeholder_identity_is_reported() {
        let (store, dir) = fixture();
        crate::git::ensure_repo_as(&dir, Some(("kanbanr", crate::git::PLACEHOLDER_EMAIL)));
        let identity_issues = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("commit identity"))
                .map(|i| i.message)
                .collect()
        };
        let found = identity_issues(&store);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("placeholder") && found[0].contains("kanbanr identity"));

        crate::git::set_identity(&dir, "Ada", "ada@example.com").unwrap();
        assert!(identity_issues(&store).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-142 R-6: a board further ahead of its remote than its policy allows is reported, with
    /// the command that pushes it; one within it is not.
    #[test]
    fn a_board_far_ahead_of_its_remote_is_reported() {
        let (store, dir) = fixture();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let remote =
            std::env::temp_dir().join(format!("kanbanr-doctor-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remote);
        git2::Repository::init_bare(&remote).unwrap();
        crate::git::add_remote(&dir, "origin", remote.to_str().unwrap()).unwrap();
        crate::git::set_push_setting(&dir, crate::git::PushPolicy::Debounce { every: 2 }).unwrap();
        let lag = |store: &Store| -> Vec<String> {
            run_project(store, "demo")
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("ahead of its remote"))
                .map(|i| i.message)
                .collect()
        };
        for n in 0..3 {
            std::fs::write(dir.join(format!("note-{n}.txt")), "x").unwrap();
            assert!(crate::git::commit_local(&dir, "a change"));
        }
        // Never pushed: every commit is ahead — more than two.
        let found = lag(&store);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("kanbanr sync"), "{}", found[0]);
        let pushed = crate::git::push_pending(&dir);
        assert!(pushed.failures.is_empty(), "{pushed:?}");
        assert!(lag(&store).is_empty(), "within the policy once pushed");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&remote);
    }

    /// FEAT-197 R-10: doctor lists each coverage rule that leaves subjects uncovered, with the
    /// first ten; a rule whose source is not on the board, a hand-edited invalid rule, a gate
    /// narrowing `docs_cover` to an unknown rule and a gate's unknown default doc are errors; with
    /// no rules and no such gate it says nothing about coverage.
    #[test]
    fn doctor_lists_uncovered_rules() {
        let (store, dir) = fixture();
        let coverage = |report: &Report| -> Vec<(Severity, String)> {
            report
                .issues
                .iter()
                .filter(|i| i.message.contains("coverage") || i.message.contains("gate on"))
                .map(|i| (i.severity, i.message.clone()))
                .collect()
        };
        for n in 0..12 {
            store
                .add_feature("demo", &format!("Item {n}"), "", "M", None)
                .unwrap();
        }
        assert!(coverage(&run_project(&store, "demo").unwrap()).is_empty());

        store
            .write_doc("demo", "architecture/zachman.md", "FEAT-001 and FEAT-002\n")
            .unwrap();
        store
            .set_coverage(
                "demo",
                crate::coverage::parse_rules(
                    "- id: zachman\n  referenced_from: architecture/zachman.md\n\
                     - id: gone\n  referenced_from: architecture/gone.md\n\
                     - id: adrs\n  every: adr\n  referenced_from: architecture/\n",
                )
                .unwrap(),
            )
            .unwrap();
        // Hand-edited after: an invalid rule the store would have refused.
        let mut config = store.load_meta("demo").unwrap().config;
        config.coverage.push(crate::coverage::CoverageRule {
            id: "broken".into(),
            from: Some("Nowhere".into()),
            referenced_from: Some("architecture/zachman.md".into()),
            ..Default::default()
        });
        config.gates.insert(
            "Scheduled".into(),
            crate::config::Gate {
                requires: vec![crate::readiness::Condition::DocsCover {
                    docs_cover: vec!["zachman".into(), "catalgo".into()],
                }],
                affects_docs: vec!["architecture/missing.md".into()],
                ..Default::default()
            },
        );
        store.save_config("demo", &config).unwrap();

        let found = coverage(&run_project(&store, "demo").unwrap());
        let has = |severity: Severity, text: &str| {
            assert!(
                found
                    .iter()
                    .any(|(s, m)| *s == severity && m.contains(text)),
                "no {severity:?} '{text}' in {found:#?}"
            )
        };
        has(
            Severity::Warning,
            "coverage rule 'zachman' (every item is referenced from architecture/zachman.md): \
             10 of 12 uncovered — FEAT-003, FEAT-004, FEAT-005, FEAT-006, FEAT-007, FEAT-008, \
             FEAT-009, FEAT-010, FEAT-011, FEAT-012",
        );
        has(
            Severity::Error,
            "coverage rule 'gone': architecture/gone.md is not on the board",
        );
        has(
            Severity::Error,
            "coverage rule 'broken': 'Nowhere' is not a status of the workflow",
        );
        has(
            Severity::Error,
            "the gate on 'Scheduled' asks for docs_cover rule 'catalgo', which this project does \
             not configure",
        );
        has(
            Severity::Error,
            "the gate on 'Scheduled' lists architecture/missing.md in affects_docs, which is not on \
             the board",
        );
        assert!(
            !found.iter().any(|(_, m)| m.contains("'adrs'")),
            "no accepted ADRs, nothing uncovered: {found:#?}"
        );
        assert!(
            found.iter().filter(|(_, m)| m.contains("'broken'")).count() == 1,
            "said once: {found:#?}"
        );
        // The portfolio-wide run says the same.
        let all = coverage(&run(&store).unwrap());
        assert_eq!(all.len(), found.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-038 R-1: the folders kanbanr writes at a project's root (recorded gate results,
    /// prepared evidence, manual results) are part of the layout, not stray; others still are.
    #[test]
    fn a_recorded_gate_result_is_not_a_stray_folder() {
        let (store, dir) = fixture();
        crate::gates::record(
            &store,
            "demo",
            &crate::gates::GateResult {
                name: "ci".into(),
                commit: "a".repeat(40),
                passed: true,
                by: "Tester".into(),
                at: "2026-11-12T10:00:00Z".into(),
                source: "person".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let root = store.project_dir("demo");
        for folder in [crate::prepared::FOLDER, crate::manual::FOLDER] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("x.yaml"), "a: 1\n").unwrap();
        }
        let report = run_project(&store, "demo").unwrap();
        let strays: Vec<&str> = report
            .issues
            .iter()
            .map(|i| i.message.as_str())
            .filter(|m| m.contains("matching no status"))
            .collect();
        assert!(strays.is_empty(), "{strays:?}");
        // The scan still runs.
        std::fs::create_dir_all(root.join("stray")).unwrap();
        std::fs::write(root.join("stray").join("x.txt"), "x").unwrap();
        let report = run_project(&store, "demo").unwrap();
        let strays: Vec<&str> = report
            .issues
            .iter()
            .map(|i| i.message.as_str())
            .filter(|m| m.contains("matching no status"))
            .collect();
        assert_eq!(strays.len(), 1, "{strays:?}");
        assert!(strays[0].starts_with("stray/ is a folder matching no status"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-053 R-3: a shared board names a project with no recorded repository.
    #[test]
    fn doctor_names_a_project_with_no_repository() {
        let (store, dir) = fixture();
        let hits = |store: &Store| {
            run(store)
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("records no code repository"))
                .collect::<Vec<_>>()
        };
        assert!(hits(&store).is_empty(), "no remote: nothing to say");
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::add_remote(&dir, "origin", "git@git.example.invalid:o/board.git").unwrap();
        let found = hits(&store);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].project, "demo");
        assert!(found[0].message.contains("project edit demo --repository"));
        store
            .set_project_meta(
                "demo",
                None,
                None,
                Some("https://git.example.invalid/o/code.git".into()),
                None,
            )
            .unwrap();
        assert!(hits(&store).is_empty());
        store
            .init_project("ops", ProjectConfig::default_for("ops"))
            .unwrap();
        let err = store
            .set_project_meta("ops", None, None, None, Some(true))
            .unwrap_err();
        assert!(err.to_string().contains("no code repository is recorded"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-053 R-3: a README with the old block is reported on a shared board, and only there.
    #[test]
    fn a_readme_whose_generated_block_is_out_of_date_is_reported() {
        let (store, dir) = fixture();
        let stale_hits = |store: &Store| {
            run(store)
                .unwrap()
                .issues
                .into_iter()
                .filter(|i| i.message.contains("out of date"))
                .count()
        };
        let old = format!(
            "{}\n# A kanbanr board\n{}\n",
            crate::boardreadme::START,
            crate::boardreadme::END
        );
        std::fs::write(dir.join(crate::boardreadme::FILE), &old).unwrap();
        assert_eq!(stale_hits(&store), 0, "no remote: silent");
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        crate::git::add_remote(&dir, "origin", "git@git.example.invalid:o/board.git").unwrap();
        assert_eq!(stale_hits(&store), 1);
        crate::boardreadme::write(&store).unwrap();
        assert_eq!(stale_hits(&store), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-005 R-9: one hint for no facts record; an error per problem of an invalid catalog,
    /// naming the row; a warning per applying row whose source is missing; nothing else with no
    /// catalog.
    #[test]
    fn doctor_reports_missing_facts_and_unsourced_rows() {
        let (store, dir) = fixture();
        let catalog_issues = |r: &Report| -> Vec<Issue> {
            r.issues
                .iter()
                .filter(|i| i.message.contains("catalog") || i.message.contains("project facts"))
                .cloned()
                .collect()
        };
        // No facts record and no catalog: the one hint, and nothing else.
        for report in [run(&store).unwrap(), run_project(&store, "demo").unwrap()] {
            let found = catalog_issues(&report);
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].severity, Severity::Warning);
            assert_eq!(found[0].message, "no project facts — `kanbanr facts set …`");
        }

        // A facts record silences the hint; the starter catalog names each applying written row
        // whose source is missing.
        crate::facts::save(
            &store,
            "demo",
            &crate::facts::Facts {
                production: vec!["library".into()],
                parts: vec!["library".into()],
                data: vec!["none".into()],
                ..Default::default()
            },
        )
        .unwrap();
        crate::catalog::init(&store, "demo", false).unwrap();
        store
            .write_doc(
                "demo",
                "architecture/context.md",
                &format!("# Context\n\n{}\n", "The system reads a board. ".repeat(8)),
            )
            .unwrap();
        let report = run_project(&store, "demo").unwrap();
        let found = catalog_issues(&report);
        assert!(
            found.iter().all(|i| i.severity == Severity::Warning),
            "{found:?}"
        );
        let mut rows: Vec<String> = found
            .iter()
            .map(|i| i.message.split_whitespace().nth(2).unwrap().to_string())
            .collect();
        rows.sort();
        // Library facts: interfaces applies; data-model, deployment, configuration and
        // observability do not. Context has its doc.
        assert_eq!(
            rows,
            vec![
                "behaviour",
                "component-docs",
                "components",
                "interfaces",
                "security"
            ],
            "{found:?}"
        );
        assert!(
            found.iter().any(|i| i.message
                == "catalog row behaviour (Behaviour and processes) has no source: write architecture/behaviour.md"),
            "{found:?}"
        );
        assert_eq!(
            catalog_issues(&run(&store).unwrap()).len(),
            found.len(),
            "run and run_project agree"
        );

        // A switched-off row is not reported.
        crate::catalog::set_row(&store, "demo", "security", false, "covered by the README")
            .unwrap();
        let found = catalog_issues(&run_project(&store, "demo").unwrap());
        assert!(
            !found.iter().any(|i| i.message.contains("row security")),
            "{found:?}"
        );

        // An invalid catalog is an error per problem, naming the row, and nothing else.
        // Hand-edited: written past `save`, which would refuse it.
        let mut bad = crate::catalog::load(&store, "demo").unwrap().unwrap();
        bad.rows
            .iter_mut()
            .find(|r| r.id == "context")
            .unwrap()
            .answers = vec!["which".into()];
        let path = store.project_dir("demo").join(crate::catalog::CATALOG_FILE);
        std::fs::write(&path, crate::yaml::to_string(&bad).unwrap()).unwrap();
        let found = catalog_issues(&run_project(&store, "demo").unwrap());
        assert!(!found.is_empty());
        assert!(
            found.iter().all(|i| i.severity == Severity::Error),
            "{found:?}"
        );
        assert!(
            found.iter().any(|i| i.message.contains("row \"context\"")),
            "{found:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    fn warnings_of(store: &Store) -> Vec<String> {
        run_project(store, "demo")
            .unwrap()
            .issues
            .into_iter()
            .filter(|i| i.severity == Severity::Warning)
            .map(|i| i.message)
            .collect()
    }

    /// MNT-009 R-5: an NFR whose stored tag is "Performance efficiency" is served by an ADR naming
    /// "Performance Efficiency": one quality, so no "no decision claims to serve it".
    #[test]
    fn a_quality_spelled_two_ways_counts_as_one() {
        let (store, dir) = fixture();
        let code = store
            .add_feature("demo", "Answer fast", "# Answer fast\n", "M", None)
            .unwrap()
            .code;
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Fast answers, for the shopper, so that carts are kept".into(),
                    goals: vec!["G-1".into()],
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        kind: crate::models::RequirementKind::Nfr,
                        text: "THE SYSTEM SHALL answer in 100 ms".into(),
                        iso: vec!["Performance Efficiency".into()],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // The spelling a board stored before MNT-009, kept as stored (Q1).
        let status = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .status
            .clone();
        let path = store.feature_path("demo", &status, &code);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Performance Efficiency"), "{text}");
        std::fs::write(
            &path,
            text.replace("Performance Efficiency", "Performance efficiency"),
        )
        .unwrap();
        store
            .write_doc(
                "demo",
                "decisions/0001-cache.md",
                "---\nid: ADR-0001\nstatus: accepted\naffects: [FEAT-001]\n\
                 quality: [Performance Efficiency]\n---\n# Cache\n\n## Decision\n\nd\n\n\
                 ## Consequences\n\nq\n",
            )
            .unwrap();
        let said = warnings_of(&store);
        assert!(
            !said
                .iter()
                .any(|m| m.contains("no decision claims to serve it")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// MNT-009 R-6: an ADR naming a column outside the six still loads, and the doctor names it.
    #[test]
    fn doctor_warns_of_an_adr_naming_an_unknown_zachman_column() {
        let (store, dir) = fixture();
        let adr = |id: &str, cols: &str| {
            format!(
                "---\nid: {id}\nstatus: proposed\nzachman: [{cols}]\n---\n# {id}\n\n\
                 ## Decision\n\nd\n\n## Consequences\n\nq\n"
            )
        };
        store
            .write_doc(
                "demo",
                "decisions/0001-data.md",
                &adr("ADR-0001", "Data, How"),
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "decisions/0002-known.md",
                &adr("ADR-0002", "how, Where"),
            )
            .unwrap();
        let listed = crate::adr::list(&store, "demo").unwrap();
        assert_eq!(listed.len(), 2, "both load");
        let said = warnings_of(&store);
        assert!(
            said.iter()
                .any(|m| m.contains("ADR-0001 names 'Data', which is not a Zachman column")),
            "{said:?}"
        );
        assert!(
            !said
                .iter()
                .any(|m| m.contains("ADR-0002") && m.contains("not a Zachman column")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-039 R-6: one doctor warning per item whose manual tests break the judgement rule (two
    /// of them, or wording a run can check), none for a clean item; a warning, never an error.
    #[test]
    fn doctor_reports_manual_tests_that_break_the_judgement_rule() {
        let (store, dir) = fixture();
        let define = |title: &str, names: &[&str]| -> String {
            let code = store
                .add_feature("demo", title, "", "M", None)
                .unwrap()
                .code;
            store
                .set_feature_definition(
                    "demo",
                    &code,
                    Some(FeatureDefinition {
                        statement: "Notes for the shopper, so that a release is understood".into(),
                        requirements: vec![Requirement {
                            id: "R-1".into(),
                            text: "THE SYSTEM SHALL write release notes".into(),
                            tests: names
                                .iter()
                                .map(|n| TestRef {
                                    name: n.to_string(),
                                    kind: "unit".into(),
                                    ..Default::default()
                                })
                                .collect(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )
                .unwrap();
            code
        };
        // Items written before the rule: their tests become manual by hand, past the store, which
        // would refuse them now.
        let to_manual = |code: &str| {
            let status = store
                .load("demo")
                .unwrap()
                .feature(code)
                .unwrap()
                .status
                .clone();
            let path = store.feature_path("demo", &status, code);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("kind: unit"), "{text}");
            std::fs::write(&path, text.replace("kind: unit", "kind: manual")).unwrap();
        };
        let clean = define(
            "Clean",
            &["manual: the person judges whether the notes read clearly to a newcomer"],
        );
        let two = define(
            "Two",
            &[
                "manual: the person judges whether the notes read clearly",
                "manual: a newcomer judges whether the wording is clear",
            ],
        );
        let observable = define(
            "Observable",
            &["manual: sha256sum -c passes over the manifest"],
        );
        for code in [&clean, &two, &observable] {
            to_manual(code);
        }
        let report = run_project(&store, "demo").unwrap();
        let rule: Vec<&Issue> = report
            .issues
            .iter()
            .filter(|i| i.message.contains("break the judgement rule"))
            .collect();
        assert_eq!(rule.len(), 2, "{rule:?}");
        assert!(
            rule.iter().all(|i| i.severity == Severity::Warning),
            "{rule:?}"
        );
        let of = |code: &str| {
            rule.iter()
                .find(|i| i.code.as_deref() == Some(code))
                .map(|i| i.message.clone())
        };
        let two_said = of(&two).expect("the item with two manual tests is named");
        assert!(
            two_said.starts_with(&format!(
                "{two} has manual tests that break the judgement rule: "
            )),
            "{two_said}"
        );
        assert!(
            two_said.contains("a newcomer judges whether the wording is clear"),
            "{two_said}"
        );
        let observable_said = of(&observable).expect("the observable wording is named");
        assert!(observable_said.contains("'sha256sum'"), "{observable_said}");
        assert!(observable_said.contains("integration"), "{observable_said}");
        assert_eq!(of(&clean), None, "a clean item is not reported");
        let _ = std::fs::remove_dir_all(dir);
    }

    // ---- FTR-033 P6: the practice packs and the baseline ------------------------------------

    fn issues_of(f: impl Fn(&mut Report)) -> Vec<Issue> {
        let mut report = Report::default();
        f(&mut report);
        report.issues
    }

    fn write_baseline(store: &Store, b: &crate::baseline::Baseline) {
        std::fs::write(
            store
                .project_dir("demo")
                .join(crate::baseline::BASELINE_FILE),
            crate::yaml::to_string(b).unwrap(),
        )
        .unwrap();
    }

    /// FTR-033 R-5: a pack whose board copy moved on is reported with its new areas and changed
    /// recommendation, a current one is not; a revision waiting for the person is a warning and
    /// a constraint in force without its ADR an error.
    #[test]
    fn doctor_reports_a_newer_profile_and_its_new_areas() {
        use crate::baseline::tests::{
            SCAN_PACK, STORE_PACK, accept_it, answer, board, propose_answers, stamp,
        };
        use crate::profile::ProfileFile;
        let (store, dir, _) = board(true);
        let v1 = store
            .save_profile("store", ProfileFile::from_yaml(STORE_PACK).unwrap(), None)
            .unwrap();
        crate::baseline::add_pack(
            &store,
            "demo",
            "store",
            &v1,
            crate::config::Library::Board,
            stamp(),
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let current = issues_of(|r| scan_profile_drift(&store, &project, r));
        assert!(current.is_empty(), "a current pack: {current:?}");

        let mut v2 = v1.clone();
        v2.areas[0].options[0].recommended = false;
        v2.areas[0].options[1].recommended = true;
        v2.areas
            .push(ProfileFile::from_yaml(SCAN_PACK).unwrap().areas[0].clone());
        store.save_profile("store", v2, None).unwrap();
        let drift = issues_of(|r| scan_profile_drift(&store, &project, r));
        assert_eq!(drift.len(), 1, "{drift:?}");
        let said = &drift[0].message;
        assert_eq!(drift[0].severity, Severity::Warning);
        assert!(said.contains("'store'"), "{said}");
        assert!(said.contains("1 new area to answer (scanning)"), "{said}");
        assert!(
            said.contains("1 recommendation changed (database)"),
            "{said}"
        );
        assert!(said.contains("kanbanr profile update store"), "{said}");

        // A proposed revision waits on the person; once accepted, nothing is said.
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        let waiting = issues_of(|r| scan_baseline(&store, &project, r));
        assert_eq!(waiting.len(), 1, "{waiting:?}");
        assert_eq!(waiting[0].severity, Severity::Warning);
        assert!(
            waiting[0].message.contains(
                "REV-001 has waited since 2026-11-17 for the person to accept it on the Baseline page"
            ),
            "{waiting:?}"
        );
        accept_it(&store, &v);
        // The acceptance wrote the stage gate into the config: read the project again.
        let project = store.load("demo").unwrap();
        let accepted = issues_of(|r| scan_baseline(&store, &project, r));
        assert!(accepted.is_empty(), "{accepted:?}");

        // The constraint's ADR link lost by hand: an error naming the entry.
        let mut b = crate::baseline::load(&store, "demo").unwrap();
        let adr = std::mem::take(&mut b.entries[0].adr);
        assert!(!adr.is_empty());
        write_baseline(&store, &b);
        let lost = issues_of(|r| scan_baseline(&store, &project, r));
        assert_eq!(lost.len(), 1, "{lost:?}");
        assert_eq!(lost[0].severity, Severity::Error);
        assert!(lost[0].message.contains("B-001"), "{lost:?}");
        assert!(lost[0].message.contains("no ADR"), "{lost:?}");

        // Its revision's acceptance removed by hand: the ADR records an entry not in force.
        b.entries[0].adr = adr.clone();
        b.revisions[0].accepted = None;
        write_baseline(&store, &b);
        let unaccepted = issues_of(|r| scan_baseline(&store, &project, r));
        let errors: Vec<_> = unaccepted
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert_eq!(errors.len(), 1, "{unaccepted:?}");
        assert!(errors[0].message.contains(&adr), "{errors:?}");
        assert!(
            errors[0].message.contains("accept REV-001 again"),
            "{errors:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-033 R-5: a stage gate an entry in force promised, removed from the stage by hand, is
    /// reported naming the stage, the gate and the entry.
    #[test]
    fn a_stage_edited_away_from_the_baseline_is_reported() {
        use crate::baseline::tests::{
            STORE_PACK, accept_it, answer, board, follow, propose_answers,
        };
        let (store, dir, _) = board(true);
        follow(&store, STORE_PACK);
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        let project = store.load("demo").unwrap();
        let held = issues_of(|r| scan_baseline(&store, &project, r));
        assert!(held.is_empty(), "{held:?}");

        let mut config = project.config.clone();
        config
            .cadence
            .release_gates
            .get_mut("main")
            .unwrap()
            .gates
            .clear();
        store.save_config("demo", &config).unwrap();
        let project = store.load("demo").unwrap();
        let edited = issues_of(|r| scan_baseline(&store, &project, r));
        assert_eq!(edited.len(), 1, "{edited:?}");
        assert_eq!(edited[0].severity, Severity::Warning);
        assert!(
            edited[0]
                .message
                .contains("stage main no longer requires `migrations-check`, which B-001 promised"),
            "{edited:?}"
        );

        // Releases turned off by hand: nothing is held, so nothing is reported.
        let mut config = project.config.clone();
        config.cadence.releases = false;
        store.save_config("demo", &config).unwrap();
        let project = store.load("demo").unwrap();
        let off = issues_of(|r| scan_baseline(&store, &project, r));
        assert!(off.is_empty(), "{off:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
