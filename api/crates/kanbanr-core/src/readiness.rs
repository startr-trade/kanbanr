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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Planned into the project's active sprint (FEAT-119).
    InSprint,
    /// Planned into a release (FEAT-120).
    InRelease,
    /// A named sign-off is recorded against the current definition. Asked for through a gate's
    /// `signoffs: [name]`, never listed on its own — it needs the name.
    Signoff,
    /// The spec has each named section, with content (FEAT-179). A design stage asks for the
    /// design itself, not a promise of one: `{spec_sections: [Design]}`.
    SpecSections,
    /// Every doc the item declares in `affects_docs` has changed since it entered its current
    /// stage (FEAT-180) — the design was folded into the docs it said it would change.
    DocsUpdated,
    /// Every Mermaid diagram in the spec and in each `affects_docs` doc passes the diagram lint
    /// (FEAT-200), judged when the item moves, never cached.
    DiagramsValid,
    /// The item is covered by the project's coverage rules (FEAT-197), judged as if it were already
    /// in the stage it is entering; `{docs_cover: [id]}` asks for only those rules.
    DocsCover,
}

impl Check {
    /// Every check a gate can name, in the order the guide lists them (FEAT-168). `signoff` is not
    /// one: a gate asks for a sign-off by its name, under `signoffs`. A new check must be added
    /// here, and [`Check::passes_when`] will not compile until it says what the check means.
    pub const GATEABLE: [Check; 20] = [
        Check::Definition,
        Check::Statement,
        Check::Goals,
        Check::Zachman,
        Check::Requirements,
        Check::Ears,
        Check::TestsNamed,
        Check::TestsGreen,
        Check::Quality,
        Check::Approved,
        Check::Bypass,
        Check::GoalsKnown,
        Check::Small,
        Check::Estimated,
        Check::InSprint,
        Check::InRelease,
        Check::SpecSections,
        Check::DocsUpdated,
        Check::DiagramsValid,
        Check::DocsCover,
    ];

    /// The name a gate uses for this check.
    pub fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }

    /// When this check passes, in a phrase — what `kanbanr process checks` prints and the guide's
    /// table repeats, so Claude can offer it to the user in their own terms.
    pub fn passes_when(self) -> &'static str {
        match self {
            Check::Definition => "the item has a definition at all",
            Check::Statement => "the one-sentence statement is written",
            Check::Goals => "it links at least one charter goal",
            Check::Zachman => {
                "all six dimensions are answered; `{zachman: [what, why]}` asks for only those"
            }
            Check::Requirements => "it has at least one requirement",
            Check::Ears => "every requirement is in EARS form",
            Check::TestsNamed => "every requirement names a test",
            Check::TestsGreen => "every requirement has a green test",
            Check::Quality => {
                "quality requirements carry an ISO 25010 tag and a measured scenario that names \
                 its test"
            }
            Check::Approved => "the definition is agreed: approved, or ratified after the fact",
            Check::Bypass => "a recorded override has been answered",
            Check::GoalsKnown => "every linked goal exists in the charter",
            Check::Small => "not estimated above three days",
            Check::Estimated => {
                "estimated in the project's unit: story points or days \
                 (`kanbanr config cadence --unit points`)"
            }
            Check::InSprint => {
                "planned into the active sprint, one that has started (asked only when sprints are on)"
            }
            Check::InRelease => "planned into a release (projects with releases switched on)",
            Check::Signoff => "a named sign-off is recorded; a gate names it under `signoffs`",
            Check::SpecSections => {
                "the spec has each named section, with content; plain `spec_sections` asks for \
                 `Design`"
            }
            Check::DocsUpdated => {
                "every board doc the item names in `affects_docs` has changed since it entered its \
                 current stage"
            }
            Check::DiagramsValid => {
                "every Mermaid diagram in the spec and in each doc it names in `affects_docs` \
                 passes the diagram lint"
            }
            Check::DocsCover => {
                "the item is covered by every project coverage rule it falls under; \
                 `{docs_cover: [id]}` asks for only those rules"
            }
        }
    }

    /// What the check can be narrowed by, if anything.
    pub fn parameters(self) -> Option<&'static str> {
        match self {
            Check::Zachman => Some("zachman: [what, how, where, when, who, why] (any of the six)"),
            Check::SpecSections => {
                Some("spec_sections: [Design, …] (the spec's headings, any level)")
            }
            Check::DocsCover => Some(
                "docs_cover: [rule-id, …] (the project's coverage rules, `kanbanr config coverage`)",
            ),
            _ => None,
        }
    }

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Condition {
    Check(Check),
    Zachman {
        zachman: Vec<String>,
    },
    SpecSections {
        spec_sections: Vec<String>,
    },
    /// The coverage check narrowed to some of the project's rules (FEAT-197).
    DocsCover {
        docs_cover: Vec<String>,
    },
}

/// Read by hand so a misspelt check is named, with the ones there are (FEAT-168): the derived
/// reader said only "data did not match any variant of untagged enum Condition".
impl<'de> Deserialize<'de> for Condition {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Zachman { zachman: Vec<String> },
            SpecSections { spec_sections: Vec<String> },
            DocsCover { docs_cover: Vec<String> },
        }
        match Raw::deserialize(d)? {
            Raw::Zachman { zachman } => Ok(Condition::Zachman { zachman }),
            Raw::DocsCover { docs_cover } => Ok(Condition::DocsCover { docs_cover }),
            Raw::SpecSections { spec_sections } => Ok(Condition::SpecSections { spec_sections }),
            Raw::Name(name) => serde_json::from_value(serde_json::Value::String(name.clone()))
                .map(Condition::Check)
                .map_err(|_| {
                    let known: Vec<String> = Check::GATEABLE.iter().map(|c| c.name()).collect();
                    serde::de::Error::custom(format!(
                        "'{name}' is not a check a gate can ask for — the checks are: {} (or \
                         {{zachman: [what, why]}}; see `kanbanr process checks`)",
                        known.join(", ")
                    ))
                }),
        }
    }
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
            Condition::Check(_) | Condition::SpecSections { .. } | Condition::DocsCover { .. } => {
                None
            }
        }
    }

    /// Whether this condition asks for coverage (FEAT-197), plain or narrowed.
    pub fn is_docs_cover(&self) -> bool {
        matches!(
            self,
            Condition::Check(Check::DocsCover) | Condition::DocsCover { .. }
        )
    }

    /// Whether evaluating this condition reads the spec body, which a metadata-only load leaves
    /// empty.
    pub fn reads_spec(&self) -> bool {
        matches!(
            self,
            Condition::SpecSections { .. }
                | Condition::Check(Check::SpecSections)
                | Condition::Check(Check::DiagramsValid)
        )
    }
}

/// The spec as a sign-off pins it, and its hash (FTR-023). Line endings become LF, trailing
/// whitespace is stripped from each line, and blank lines at the start and end are dropped, so a
/// file rewritten without any change of words pins the same. An empty spec pins `""`.
pub fn spec_pin(spec: &str) -> (String, String) {
    let lines: Vec<&str> = spec.lines().map(str::trim_end).collect();
    let text = lines.join("\n").trim_matches('\n').to_string();
    if text.is_empty() {
        return (String::new(), String::new());
    }
    let rev = crate::hash::stable_hash(&text);
    (text, rev)
}

/// The sign-offs among `names` that are not recorded against the definition as it stands.
pub fn signoff_gaps(feature: &FeatureItem, names: &[String]) -> Vec<Gap> {
    let def = feature.definition.as_ref();
    names
        .iter()
        .filter_map(|name| {
            let latest = def.and_then(|d| d.signoff_latest(name));
            let current = feature.signoff_current(name);
            match (latest, current) {
                (_, Some(_)) => None,
                (Some(_), None) if feature.signoff_spec_changed(name).is_some() => Some(gap(
                    Check::Signoff,
                    None,
                    &format!("sign-off '{name}': spec changed"),
                    format!(
                        "the spec changed after sign-off '{name}' was given, so it no longer \
                         covers what will be built: the Review page shows what changed"
                    ),
                )),
                // Taken back (FEAT-182): say so, with the reason, rather than "lapsed".
                (Some(w), None) if !w.verdict.is_given() => Some(gap(
                    Check::Signoff,
                    None,
                    &format!("sign-off '{name}' withdrawn"),
                    format!(
                        "sign-off '{name}' was withdrawn: {} — `kanbanr signoff {} {name}` to give \
                         it again",
                        w.reason, feature.code
                    ),
                )),
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
/// What some checks need to know about the project beyond the item: the unit it estimates in, and
/// its cadence (FEAT-121, FEAT-119).
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub unit: crate::config::EstimateUnit,
    /// The sprint currently active, if the project uses sprints and one is.
    pub active_sprint: Option<String>,
    /// The project has no sprints switched on, so `in_sprint` asks nothing (DEF-041).
    pub sprints_off: bool,
    /// The code and first day of each sprint that is neither closed nor running as of today.
    pub sprint_starts: std::collections::BTreeMap<String, String>,
    /// The project's doc folder, for [`Check::DocsUpdated`] (FEAT-180). Its history is read only
    /// when that check is evaluated, and only for items that declare docs.
    pub docs_dir: Option<std::path::PathBuf>,
    /// What the coverage rules' source docs reference (FEAT-197), read only when the project has
    /// rules and some gate asks for `docs_cover`.
    pub coverage: Option<std::sync::Arc<crate::coverage::Snapshot>>,
    /// The stages at or after the one holding the `build` role, in workflow order (FTR-018): from
    /// there on, text added to an approved definition must be decided at once. `None` when no
    /// stage holds the role, which counts every stage as build onward.
    pub build_onward: Option<std::collections::BTreeSet<String>>,
    /// The versions of the releases not yet cut, when the project works in releases (DEF-019).
    /// An item planned into one waits for the cut before it reaches its end status.
    pub unshipped_releases: std::collections::BTreeSet<String>,
    /// The board's git history, shared by every doc judged in this evaluation (DEF-022): opened
    /// and walked once, on the first doc. Make a new `Context` per evaluation; nothing outlives it.
    pub history: std::sync::Arc<crate::git::DocHistory>,
    /// The code repository, when this surface has one (DEF-033): the repository docs an item lists
    /// are judged against it. `None` holds `docs-folded` for items that list any.
    pub code_repo: Option<std::path::PathBuf>,
    /// The sprints and releases the line an item merges into is derived from; loaded only with a
    /// code repository on a project that automates its git model.
    pub sprints: Vec<crate::sprints::Sprint>,
    pub releases: Vec<crate::releases::Release>,
    /// The repository's branches, read once per evaluation on first use.
    pub(crate) branches: std::sync::OnceLock<Option<crate::gitmodel::BranchFacts>>,
    /// `cadence.main_branch`, when set.
    pub(crate) main_branch: Option<String>,
}

impl Context {
    /// The code repository's branches, read once; `None` without a repository or when it cannot
    /// be read.
    pub fn branch_facts(&self) -> Option<&crate::gitmodel::BranchFacts> {
        self.branches
            .get_or_init(|| {
                let root = self.code_repo.as_deref()?;
                let main = self
                    .main_branch
                    .clone()
                    .unwrap_or_else(|| crate::coderepo::default_branch(root));
                crate::coderepo::branch_facts(root, &main).ok()
            })
            .as_ref()
    }

    /// Is `status` the build stage or a later one? With no build role declared, every stage is.
    pub fn builds(&self, status: &str) -> bool {
        self.build_onward
            .as_ref()
            .is_none_or(|stages| stages.contains(status))
    }

    /// The context of a project, read from its config and side files.
    pub fn load(store: &crate::Store, id: &str, config: &crate::ProjectConfig) -> Context {
        Self::load_in(store, id, config, None)
    }

    /// [`Context::load`] with the project's code repository, when the surface has one (DEF-033).
    pub fn load_in(
        store: &crate::Store,
        id: &str,
        config: &crate::ProjectConfig,
        repo: Option<&std::path::Path>,
    ) -> Context {
        let line_inputs = repo.is_some() && config.cadence.git_model;
        let today = crate::sprints::today();
        let all_sprints = if config.cadence.sprints {
            crate::sprints::load(store, id).unwrap_or_default()
        } else {
            Vec::new()
        };
        Context {
            sprints_off: !config.cadence.sprints,
            sprint_starts: all_sprints
                .iter()
                .filter(|s| s.state_on(today) == crate::sprints::SprintState::Planned)
                .map(|s| (s.code.clone(), s.start.clone()))
                .collect(),
            code_repo: repo.map(std::path::Path::to_path_buf),
            main_branch: config
                .cadence
                .main_branch
                .clone()
                .filter(|m| !m.trim().is_empty()),
            sprints: if line_inputs {
                crate::sprints::load(store, id).unwrap_or_default()
            } else {
                Vec::new()
            },
            releases: if line_inputs {
                crate::releases::load(store, id).unwrap_or_default()
            } else {
                Vec::new()
            },
            branches: Default::default(),
            unit: config.estimate_unit,
            active_sprint: crate::sprints::active_on(&all_sprints, today).map(|a| a.code.clone()),
            docs_dir: Some(store.project_dir(id).join("docs")),
            coverage: Self::coverage(store, id, config),
            build_onward: config.role_status(crate::config::Role::Build).map(|build| {
                config
                    .statuses
                    .iter()
                    .skip_while(|s| **s != build)
                    .filter(|s| !config.is_no_op(s))
                    .cloned()
                    .collect()
            }),
            unshipped_releases: if config.cadence.releases {
                crate::releases::load(store, id)
                    .map(|rs| {
                        rs.into_iter()
                            .filter(|r| r.state != crate::releases::ReleaseState::Shipped)
                            .map(|r| r.version)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Default::default()
            },
            history: Default::default(),
        }
    }

    /// The coverage snapshot, when a gate will ask for it; one that names why, when it cannot be
    /// built.
    fn coverage(
        store: &crate::Store,
        id: &str,
        config: &crate::ProjectConfig,
    ) -> Option<std::sync::Arc<crate::coverage::Snapshot>> {
        let asked = config.effective_gates().values().any(|g| {
            g.requires
                .iter()
                .chain(&g.warns)
                .any(Condition::is_docs_cover)
        });
        if config.coverage.is_empty() || !asked {
            return None;
        }
        let snap = store
            .load_meta(id)
            .and_then(|p| crate::coverage::Snapshot::load(store, &p))
            .unwrap_or_else(|e| crate::coverage::Snapshot::failed(config, e.to_string()));
        Some(std::sync::Arc::new(snap))
    }
}

/// The gaps of an item against a gate's conditions, judged where it stands.
pub fn evaluate_conditions(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    conditions: &[Condition],
    ctx: &Context,
) -> Vec<Gap> {
    evaluate_entering(feature, goal_ids, conditions, ctx, &feature.status)
}

/// The gaps of an item against a gate's conditions, as if it were already in `entering` — the
/// stage whose gate is being judged (FEAT-197). Only `docs_cover` depends on where the item is.
pub fn evaluate_entering(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    conditions: &[Condition],
    ctx: &Context,
    entering: &str,
) -> Vec<Gap> {
    let checks: Vec<Check> = conditions
        .iter()
        .filter_map(|c| match c {
            Condition::Check(c) => Some(*c),
            Condition::Zachman { .. }
            | Condition::SpecSections { .. }
            | Condition::DocsCover { .. } => None,
        })
        .collect();
    let mut gaps = evaluate_in(feature, goal_ids, &checks, ctx);
    let sections: Vec<String> = conditions
        .iter()
        .flat_map(|c| match c {
            Condition::SpecSections { spec_sections } => spec_sections.clone(),
            _ => vec![],
        })
        .collect();
    if !sections.is_empty() && !is_exempt(feature) {
        gaps.extend(spec_section_gaps(&feature.specification, &sections));
    }
    let columns: Vec<String> = conditions
        .iter()
        .flat_map(|c| match c {
            Condition::Zachman { zachman } => zachman.clone(),
            Condition::Check(_) | Condition::SpecSections { .. } | Condition::DocsCover { .. } => {
                vec![]
            }
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
    gaps.extend(docs_cover_gaps(feature, conditions, ctx, entering));
    gaps
}

/// What `docs_cover` lacks (FEAT-197): plain asks for every rule, narrowed for the union of the ids
/// named. Judged at `entering`. A named rule the project does not configure is an Error.
fn docs_cover_gaps(
    feature: &FeatureItem,
    conditions: &[Condition],
    ctx: &Context,
    entering: &str,
) -> Vec<Gap> {
    let asked: Vec<&Condition> = conditions.iter().filter(|c| c.is_docs_cover()).collect();
    if asked.is_empty() || is_exempt(feature) {
        return Vec::new();
    }
    let plain = asked
        .iter()
        .any(|c| matches!(c, Condition::Check(Check::DocsCover)));
    let mut named: Vec<String> = Vec::new();
    for c in &asked {
        if let Condition::DocsCover { docs_cover } = c {
            for id in docs_cover.iter().map(|i| i.trim().to_string()) {
                if !id.is_empty() && !named.contains(&id) {
                    named.push(id);
                }
            }
        }
    }
    let configured = |id: &str| {
        ctx.coverage
            .as_ref()
            .is_some_and(|s| s.rules.iter().any(|r| r.id == id))
    };
    let mut gaps = Vec::new();
    for id in named.iter().filter(|id| !configured(id)) {
        gaps.push(Gap {
            level: Level::Error,
            ..gap(
                Check::DocsCover,
                None,
                &format!("unknown rule {id}"),
                format!("docs_cover names rule '{id}', which this project does not configure"),
            )
        });
    }
    let Some(snap) = ctx.coverage.as_deref() else {
        return gaps;
    };
    let only: Option<Vec<String>> =
        (!plain).then(|| named.iter().filter(|id| configured(id)).cloned().collect());
    if only.as_ref().is_some_and(|o| o.is_empty()) {
        return gaps;
    }
    for f in snap.item_findings(&snap.config, feature, entering, only.as_deref()) {
        gaps.push(Gap {
            check: Check::DocsCover,
            requirement: None,
            level: f.level,
            label: format!("[{}] not covered", f.rule),
            message: f.message,
        });
    }
    gaps
}

/// What moving an item on to one next status would ask, and what it still lacks (FEAT-117).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NextGate {
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    pub enforce: crate::config::Enforce,
    /// What `requires` and `signoffs` still lack.
    pub gaps: Vec<Gap>,
    /// What `requires` alone still lacks, without the sign-offs (FEAT-182): a sign-off is offered
    /// only once this is empty, so nobody signs off work that has not happened yet.
    pub requires_gaps: Vec<Gap>,
    /// What `warns` lacks — said, never enforced.
    pub warnings: Vec<Gap>,
    /// The sign-offs this gate still needs, by name, so a surface can offer to record them.
    pub signoffs_needed: Vec<String>,
    /// Sign-offs withheld because the work they confirm is not built (DEF-033): one gap per
    /// unbuilt document. Only Claude can build them, so they are not "for you".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signoffs_held: Vec<Gap>,
    /// What `docs-folded` was judged against, when it was asked (not serialized).
    #[serde(skip)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    pub docs_built: Option<crate::docsbuilt::DocsBuilt>,
}

/// What one item is missing, and what its next stage asks (FEAT-112, FEAT-117):
/// `GET /projects/{p}/features/{code}/readiness`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReadinessView {
    pub code: String,
    /// The definition's revision: a verdict given with `rev` is refused if it moved on
    /// (FEAT-159). Null when the item has no definition.
    pub rev: Option<String>,
    /// What `kanbanr check` reports.
    pub gaps: Vec<Gap>,
    /// What each next stage asks, and what it still lacks.
    pub next: Vec<NextGate>,
}

impl ReadinessView {
    pub fn load(store: &crate::Store, p: &str, code: &str) -> crate::Result<Self> {
        Self::load_in(store, p, code, None)
    }

    /// [`ReadinessView::load`] with the code repository, when the surface has one (DEF-033).
    pub fn load_in(
        store: &crate::Store,
        p: &str,
        code: &str,
        repo: Option<&std::path::Path>,
    ) -> crate::Result<Self> {
        let project = store.load_for_gates(p)?;
        let feature = project.feature(code)?;
        let charter = crate::charter::load(store, p)?;
        let goal_ids = charter.goal_ids();
        Ok(Self {
            code: feature.code.clone(),
            rev: feature.definition.as_ref().map(|d| d.content_rev()),
            gaps: evaluate(feature, Some(&goal_ids), CHECK),
            next: next_gates(
                &project,
                Some(&charter),
                feature,
                &Context::load_in(store, p, &project.config, repo),
            ),
        })
    }
}

/// A next stage on a card: its status, how many things it still lacks, which of those are
/// sign-offs, and how many of the item's tasks are open (FEAT-205) — so a card never reads as
/// ready while a sign-off or work is outstanding.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NextStage {
    pub status: String,
    /// Everything the stage still lacks, its sign-offs included (as before FEAT-205).
    pub missing: usize,
    /// The sign-offs among `missing`.
    pub signoffs: Vec<String>,
    pub tasks_open: usize,
    /// The stage ends the work (an end status that is not a no-op): "ready to complete".
    pub ends: bool,
    /// The stage the item reaches next in the workflow's order, the one the Review page asks
    /// sign-offs for (DEF-020). A card speaks only of this one.
    pub immediate: bool,
    /// How many of `missing` the user settles: sign-offs and the `approved` check (DEF-020).
    pub for_you: usize,
}

/// One card's readiness: its gaps, and its next stages.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CardReadiness {
    pub gaps: Vec<Gap>,
    pub next: Vec<NextStage>,
}

/// The board's cards in one answer, keyed by item code, in board order:
/// `GET /projects/{p}/readiness`. Finished and parked items are left out, as doctor leaves them
/// out — they are history, not work.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectReadiness(pub Vec<(String, CardReadiness)>);

impl ProjectReadiness {
    pub fn load(store: &crate::Store, p: &str) -> crate::Result<Self> {
        Self::load_in(store, p, None)
    }

    /// [`ProjectReadiness::load`] with the code repository, when the surface has one (DEF-033).
    pub fn load_in(
        store: &crate::Store,
        p: &str,
        repo: Option<&std::path::Path>,
    ) -> crate::Result<Self> {
        let project = store.load_for_gates(p)?;
        let charter = crate::charter::load(store, p)?;
        let ctx = Context::load_in(store, p, &project.config, repo);
        Ok(Self(
            project
                .features
                .iter()
                .filter(|f| crate::graph::is_live_work(&project.config, &f.status))
                .map(|f| {
                    let card = CardReadiness {
                        gaps: evaluate(f, None, CARD),
                        // The next stage, with how much it still lacks (FEAT-117).
                        next: {
                            let gates = next_gates(&project, Some(&charter), f, &ctx);
                            let first = immediate_status(&project, &gates).map(str::to_owned);
                            gates
                                .into_iter()
                                .map(|n| NextStage {
                                    ends: crate::graph::is_terminal_status(
                                        &project.config,
                                        &n.status,
                                    ) && !project.config.is_no_op(&n.status),
                                    immediate: first.as_deref() == Some(n.status.as_str()),
                                    for_you: n
                                        .gaps
                                        .iter()
                                        .filter(|g| {
                                            matches!(g.check, Check::Signoff | Check::Approved)
                                        })
                                        .count(),
                                    status: n.status,
                                    missing: n.gaps.len(),
                                    signoffs: n.signoffs_needed,
                                    tasks_open: f
                                        .all_tasks()
                                        .filter(|t| t.state != crate::models::TaskState::Completed)
                                        .count(),
                                })
                                .collect()
                        },
                    };
                    (f.code.clone(), card)
                })
                .collect(),
        ))
    }
}

impl Serialize for ProjectReadiness {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (code, card) in &self.0 {
            map.serialize_entry(code, card)?;
        }
        map.end()
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for ProjectReadiness {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProjectReadiness".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "description": "The board's live cards, keyed by item code, in board order.",
            "additionalProperties": generator.subschema_for::<CardReadiness>(),
        })
    }
}

/// The gates an item meets next: the statuses it can move on to in one step — forward in the
/// workflow's own order, never a parking or no-op status — that have a gate. This is the answer to
/// "what does the next stage need?", which a definition built stage by stage asks at every step.
pub fn next_gates(
    project: &crate::store::Project,
    charter: Option<&crate::Charter>,
    feature: &FeatureItem,
    ctx: &Context,
) -> Vec<NextGate> {
    let config = &project.config;
    let gates = config.effective_gates();
    let position = |s: &str| config.statuses.iter().position(|x| x == s);
    let here = position(&feature.status);
    let goal_ids = charter.map(|c| c.goal_ids());
    let kind = feature.kind.as_deref();
    // An item on a kind's path (FTR-019) meets the next stage on it, not the stages it skips.
    let path = config.path_for(kind);
    let reachable: Vec<&String> = match path {
        Some(_) => config
            .statuses
            .iter()
            .filter(|to| config.transition_allowed_for(&feature.status, to, kind))
            .collect(),
        None => config
            .transitions
            .get(&feature.status)
            .into_iter()
            .flatten()
            .collect(),
    };
    let first_move = feature.status == config.default_state;
    reachable
        .into_iter()
        .filter(|to| !config.is_no_op(to))
        .filter(|to| match (here, position(to)) {
            (Some(h), Some(t)) => t > h,
            _ => false,
        })
        .filter_map(|to| {
            let first_requires = if first_move {
                config.first_move_requires(kind, to)
            } else {
                Vec::new()
            };
            let mut gate = gates
                .get(to)
                .filter(|g| g.applies_to(kind))
                .cloned()
                .unwrap_or_default();
            if let Some(path) = path {
                crate::method::strip_gate(&mut gate, &path.method_off);
            }
            if first_move {
                for c in &first_requires {
                    if !gate.requires.contains(c) {
                        gate.requires.push(c.clone());
                    }
                }
            }
            // A stage with nothing to say about this kind is not listed, as before.
            if gates.get(to).filter(|g| g.applies_to(kind)).is_none() && first_requires.is_empty() {
                return None;
            }
            let mut requires_gaps =
                evaluate_entering(feature, goal_ids.as_ref(), &gate.requires, ctx, to);
            requires_gaps.extend(additions_gap(
                feature,
                &gate.requires,
                &gate.signoffs,
                ctx,
                to,
            ));
            let mut gaps = requires_gaps.clone();
            let mut signoffs = signoff_gaps(feature, &gate.signoffs);
            let mut signoffs_needed: Vec<String> = gate
                .signoffs
                .iter()
                .filter(|name| feature.signoff_current(name).is_none())
                .cloned()
                .collect();
            // `docs-folded` is offered only once the documents it confirms are built (DEF-033).
            let mut signoffs_held = Vec::new();
            let mut docs_built = None;
            if signoffs_needed
                .iter()
                .any(|n| n == crate::method::DOCS_FOLDED)
            {
                let built = crate::docsbuilt::evaluate(project, feature, ctx);
                if !built.is_built() {
                    signoffs_needed.retain(|n| n != crate::method::DOCS_FOLDED);
                    let named = format!("'{}'", crate::method::DOCS_FOLDED);
                    signoffs.retain(|g| !(g.check == Check::Signoff && g.label.contains(&named)));
                    signoffs_held = built.gaps();
                }
                docs_built = Some(built);
            }
            gaps.extend(signoffs);
            gaps.extend(signoffs_held.iter().cloned());
            Some(NextGate {
                status: to.clone(),
                purpose: gate.purpose.clone(),
                enforce: gate.enforce,
                gaps,
                requires_gaps,
                warnings: evaluate_entering(feature, goal_ids.as_ref(), &gate.warns, ctx, to),
                signoffs_needed,
                signoffs_held,
                docs_built,
            })
        })
        .collect()
}

/// Why `name` is held back from the person, if it is (DEF-033): `Some(sentence)` naming the
/// unbuilt documents when `name` is `docs-folded`, the item's immediate next gate asks for it and
/// the item's documents are not built. Any other name, a gate that does not ask for it and an
/// item not at such a stage are never refused.
pub fn signoff_hold(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
    name: &str,
) -> Option<String> {
    if name != crate::method::DOCS_FOLDED {
        return None;
    }
    let next = immediate_next_gate(project, None, feature, ctx)?;
    if next.signoffs_held.is_empty() {
        return None;
    }
    Some(format!(
        "{} cannot be signed off '{name}' yet: its documents are not built ({})",
        feature.code,
        next.signoffs_held
            .iter()
            .map(|g| g.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// The gate of the stage an item reaches next in the workflow's own order (FEAT-182): of the
/// stages it can move to in one step, the earliest. A workflow that lets an item skip ahead
/// (a stage that skips ahead) still has one next stage, and only its sign-offs are due.
pub fn immediate_next_gate(
    project: &crate::store::Project,
    charter: Option<&crate::Charter>,
    feature: &FeatureItem,
    ctx: &Context,
) -> Option<NextGate> {
    let gates = next_gates(project, charter, feature, ctx);
    let first = immediate_status(project, &gates)?.to_owned();
    gates.into_iter().find(|g| g.status == first)
}

/// The status of the earliest gate in the workflow's order: the one stage an item reaches next.
/// Review and the cards both ask this, so they cannot differ (DEF-020).
pub fn immediate_status<'a>(
    project: &crate::store::Project,
    gates: &'a [NextGate],
) -> Option<&'a str> {
    let position = |s: &str| project.config.statuses.iter().position(|x| x == s);
    gates
        .iter()
        .min_by_key(|g| position(&g.status).unwrap_or(usize::MAX))
        .map(|g| g.status.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Warning,
    /// A broken reference, not a judgement: someone can fix it by editing a name.
    Error,
}

/// One thing an item is missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    evaluate_in(feature, goal_ids, checks, &Context::default())
}

/// As [`evaluate`], judging the project-dependent checks ([`Check::Estimated`],
/// [`Check::InSprint`]) against `ctx`.
/// Claude's overrides on `f` that no ratification after them answers (DEF-043): each history
/// entry with an `override` reason and origin `conversation` or `delegated`, made after the
/// newest approval event whose verdict is `ratified` (`crate::before`). Oldest first.
///
/// A move with no origin (every move before this version, and every move the person made) is
/// never open, so old boards gain no queue.
pub fn open_overrides(f: &FeatureItem) -> Vec<&crate::models::Transition> {
    let since = f.definition.as_ref().and_then(|d| {
        d.approvals
            .iter()
            .filter(|e| e.verdict == "ratified")
            .map(|e| e.at.as_str())
            .max_by(|a, b| crate::cmp_instants(a, b))
    });
    f.history
        .iter()
        .filter(|t| {
            t.override_reason
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
                && matches!(t.origin.as_str(), "conversation" | "delegated")
                && since.is_none_or(|s| crate::before(s, &t.at))
        })
        .collect()
}

pub fn evaluate_in(
    feature: &FeatureItem,
    goal_ids: Option<&std::collections::BTreeSet<String>>,
    checks: &[Check],
    ctx: &Context,
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
        let (has, what) = match ctx.unit {
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
    if checks.contains(&Check::SpecSections) && !exempt {
        gaps.extend(spec_section_gaps(
            &feature.specification,
            &["Design".to_string()],
        ));
    }
    if checks.contains(&Check::DocsUpdated)
        && !exempt
        && let Some(docs) = ctx.docs_dir.as_deref()
    {
        gaps.extend(docs_updated_gaps(feature, docs, &ctx.history));
    }
    // An override Claude made waits for the person to ratify it, even on an agreed item
    // (DEF-043). Unagreed, the "started without approval" gap already asks.
    if checks.contains(&Check::Bypass)
        && feature
            .definition
            .as_ref()
            .is_some_and(|d| d.approval_state().is_agreed())
    {
        for t in open_overrides(feature) {
            gaps.push(gap(
                Check::Bypass,
                None,
                "override not ratified",
                format!(
                    "moved {} → {} on {} past its gate by Claude: {} — the person ratifies it on \
                     the Review page",
                    t.from,
                    t.to,
                    t.at.get(..10).unwrap_or(&t.at),
                    t.override_reason.as_deref().unwrap_or("").trim()
                ),
            ));
        }
    }
    if checks.contains(&Check::DiagramsValid) && !exempt {
        gaps.extend(diagram_gaps(feature, ctx.docs_dir.as_deref()));
    }
    if checks.contains(&Check::InRelease) && !exempt && feature.release.is_none() {
        gaps.push(gap(
            Check::InRelease,
            None,
            "not in a release",
            format!(
                "not planned into a release — `kanbanr release plan <VERSION> {}`",
                feature.code
            ),
        ));
    }
    if checks.contains(&Check::InSprint) && !exempt && !ctx.sprints_off {
        let in_active = ctx
            .active_sprint
            .as_deref()
            .is_some_and(|a| feature.sprint.as_deref() == Some(a));
        let waiting = feature
            .sprint
            .as_deref()
            .and_then(|sp| ctx.sprint_starts.get(sp).map(|start| (sp, start)));
        if in_active {
            // Planned into the sprint that is running: nothing to say.
        } else if let Some((sp, start)) = waiting {
            gaps.push(gap(
                Check::InSprint,
                None,
                "sprint not started",
                format!(
                    "planned into {sp}, which has not started (its first day is {start}) — start it with `kanbanr sprint start {sp}`, which starts it today"
                ),
            ));
        } else {
            match ctx.active_sprint.as_deref() {
                Some(active) => gaps.push(gap(
                    Check::InSprint,
                    None,
                    "not in the sprint",
                    format!(
                        "not in the active sprint {active} — `kanbanr sprint plan {active} {}`",
                        feature.code
                    ),
                )),
                None => gaps.push(gap(
                    Check::InSprint,
                    None,
                    "no active sprint",
                    "no sprint is active — `kanbanr sprint start <SP-…>`".to_string(),
                )),
            }
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
        // Anything that reads the definition fails without one — a gate asking for a statement
        // must not wave through an item that has no definition at all. It is said once, as the
        // missing definition, not as every field it would have held.
        let reads_definition = checks.iter().any(|c| {
            !matches!(
                c,
                Check::Small
                    | Check::Estimated
                    | Check::InSprint
                    | Check::InRelease
                    | Check::Signoff
                    | Check::SpecSections
                    | Check::DocsUpdated
                    | Check::DiagramsValid
                    | Check::DocsCover
            )
        });
        if reads_definition {
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

/// An approval that has only grown passes `approved` before the build stage, because the next
/// decision comes before anything is built. From the build stage on it passes only where the gate
/// also asks for a sign-off, which is pinned to the current text; otherwise this is the gap
/// (FTR-018). `at` is the stage being entered.
pub fn additions_gap(
    feature: &FeatureItem,
    requires: &[Condition],
    signoffs: &[String],
    ctx: &Context,
    at: &str,
) -> Option<Gap> {
    let def = feature.definition.as_ref()?;
    if !requires.contains(&Condition::Check(Check::Approved))
        || def.approval_state() != ApprovalState::Extended
        || !ctx.builds(at)
        || !signoffs.is_empty()
    {
        return None;
    }
    let added = def.part_changes().map(|c| c.added).unwrap_or_default();
    Some(gap(
        Check::Approved,
        None,
        "added since approval",
        format!(
            "text was added to the definition after it was approved ({}): approve it again (the \
             Review page shows only the additions), or have this stage ask for a sign-off",
            added.join(", ")
        ),
    ))
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
    let agreed = def.approval_state().is_agreed();
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
            // An extended approval passes here; whether the additions may wait is a question of the
            // stage, answered by `additions_gap` (FTR-018).
            ApprovalState::Current | ApprovalState::Ratified | ApprovalState::Extended => {}
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
        | Check::InSprint
        | Check::InRelease
        | Check::Signoff
        | Check::SpecSections
        | Check::DocsUpdated
        | Check::DiagramsValid
        | Check::DocsCover => {}
    }
}

/// A declared doc as the board stores it: under `docs/`, `.md` appended when missing.
pub fn affects_doc_key(path: &str) -> String {
    crate::refs::doc_key(path)
}

/// The docs an item declares that have not changed since it entered its stage (FEAT-180). An item
/// that declares none passes — not every item touches an architecture doc; doctor says so when the
/// next stage asks. A board with no history to judge by passes too: nothing can be shown either way.
pub(crate) fn docs_updated_gaps(
    feature: &FeatureItem,
    docs: &std::path::Path,
    history: &crate::git::DocHistory,
) -> Vec<Gap> {
    let declared = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.as_slice())
        .unwrap_or(&[]);
    let since = feature.entered_status_at();
    let mut gaps = Vec::new();
    for doc in declared {
        let key = affects_doc_key(doc);
        let path = docs.join(&key);
        let changed = history.changed_since(&path, since);
        if !path.is_file() && changed != Some(true) {
            gaps.push(Gap {
                level: Level::Error,
                ..gap(
                    Check::DocsUpdated,
                    None,
                    &format!("unknown doc {key}"),
                    format!("{key} is declared in affects_docs but is not on the board"),
                )
            });
        } else if changed == Some(false) {
            gaps.push(gap(
                Check::DocsUpdated,
                None,
                &format!("{key} not updated"),
                format!(
                    "{key} has not changed since {} entered {} — fold the design into it",
                    feature.code, feature.status
                ),
            ));
        }
    }
    gaps
}

/// One gap per diagram lint finding in the item's spec and its `affects_docs` docs (FEAT-200), read
/// from disk now, so a fixed diagram unblocks the very next move.
fn diagram_gaps(feature: &FeatureItem, docs: Option<&std::path::Path>) -> Vec<Gap> {
    crate::diagrams::item_findings(feature, docs)
        .into_iter()
        .map(|f| {
            gap(
                Check::DiagramsValid,
                None,
                &format!("diagram #{} in {}", f.block, f.file),
                f.describe(),
            )
        })
        .collect()
}

fn is_exempt(feature: &FeatureItem) -> bool {
    feature
        .definition
        .as_ref()
        .is_some_and(|d| !d.exempt.trim().is_empty())
}

/// The spec's headings, as (level, text, has content). A section's content runs to the next
/// heading of the same or a higher level, so a `## Design` holding only `### Data model` and its
/// text has content. Headings inside fenced code are not headings.
fn spec_headings(spec: &str) -> Vec<(usize, String, bool)> {
    let mut headings: Vec<(usize, String, bool)> = Vec::new();
    let mut fenced = false;
    for line in spec.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        let heading = !fenced && (1..=6).contains(&level) && t[level..].starts_with(' ');
        if heading {
            let text = t[level..].trim().trim_end_matches('#').trim().to_string();
            headings.push((level, text, false));
            continue;
        }
        let content = !t.is_empty() && !(t.starts_with("<!--") && t.ends_with("-->"));
        if content {
            // Content belongs to every open section it sits under: the nearest heading and each
            // enclosing one of a lower level.
            let mut floor = usize::MAX;
            for h in headings.iter_mut().rev() {
                if h.0 < floor {
                    h.2 = true;
                    floor = h.0;
                }
            }
        }
    }
    headings
}

/// What `{spec_sections: […]}` lacks: each named section that is absent or empty.
fn spec_section_gaps(spec: &str, names: &[String]) -> Vec<Gap> {
    let headings = spec_headings(spec);
    names
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .filter_map(
            |name| match headings.iter().find(|h| h.1.eq_ignore_ascii_case(name)) {
                Some((_, _, true)) => None,
                Some((level, text, false)) => Some(gap(
                    Check::SpecSections,
                    None,
                    &format!("empty section '{name}'"),
                    format!(
                        "the spec's '{} {text}' section is empty — write it before moving on",
                        "#".repeat(*level)
                    ),
                )),
                None => Some(gap(
                    Check::SpecSections,
                    None,
                    &format!("no section '{name}'"),
                    format!(
                        "the spec has no '{name}' section — add it as a heading, with its content"
                    ),
                )),
            },
        )
        .collect()
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
            // A test checked later (DEF-043: after main is pushed, at the cut or at a release
            // stage) holds that stage, not the item's gate.
            if !r.tests.is_empty()
                && !r.tests.iter().any(|t| t.state == TestState::Green)
                && r.tests.iter().any(|t| t.at.trim().is_empty())
            {
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

/// One item as `kanbanr check` reports it (FEAT-051, FEAT-117): what it has not said, what its
/// next stage asks, and the definition revision a verdict would be pinned to (FEAT-159).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemCheck {
    pub code: String,
    pub title: String,
    pub status: String,
    /// The CHECK list's gaps, as messages.
    pub gaps: Vec<String>,
    /// What each next stage asks; empty when the item's readiness could not be read.
    pub next: Vec<NextGate>,
    pub rev: Option<String>,
    /// Manual tests not yet green, as `R-4: <name>` (DEF-003). A note, never a gap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_manual: Vec<String>,
    /// The documents still unbuilt while the next stage holds `docs-folded` (DEF-033).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs_unbuilt: Vec<crate::docsbuilt::Unbuilt>,
    /// The flagged texts Claude has not settled, as `doc:<path>`, `adr:<id>` or `item:<CODE>`,
    /// while the next stage holds `docs-folded` and a code repository is attached (DEF-039).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts_to_settle: Vec<String>,
}

/// The manual tests of an item that are not green, as `R-4: <name>` (DEF-003): a check nobody
/// ran is named, though it does not count against readiness.
pub fn open_manual_tests(feature: &FeatureItem) -> Vec<String> {
    let Some(def) = &feature.definition else {
        return Vec::new();
    };
    def.requirements
        .iter()
        .flat_map(|r| {
            r.tests
                .iter()
                .filter(|t| t.kind.eq_ignore_ascii_case("manual") && t.state != TestState::Green)
                .map(move |t| format!("{}: {}", r.id, t.name))
        })
        .collect()
}

/// What `kanbanr check` reports over its scope (FEAT-176).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CheckReport {
    pub items: Vec<ItemCheck>,
}

impl CheckReport {
    /// How many items have no gaps left.
    pub fn ready(&self) -> usize {
        self.items.iter().filter(|i| i.gaps.is_empty()).count()
    }
}

/// `kanbanr check [<code>]` (FEAT-051): the named item, or with none every item in a displayed,
/// non-terminal status — the work in flight. Nothing in scope is refused.
pub fn check_board(
    store: &crate::Store,
    p: &str,
    scope: Option<&str>,
    repo: Option<&std::path::Path>,
) -> crate::Result<CheckReport> {
    let project = store.load(p)?;
    let features: Vec<&FeatureItem> = match scope {
        Some(code) => project.features.iter().filter(|f| f.code == code).collect(),
        None => project
            .features
            .iter()
            .filter(|f| project.config.displayed_states.contains(&f.status))
            .filter(|f| !crate::graph::is_terminal_status(&project.config, &f.status))
            .collect(),
    };
    if features.is_empty() {
        return Err(crate::CoreError::Refused(
            "no such item, or nothing in scope".into(),
        ));
    }
    let items = features
        .into_iter()
        .map(|f| {
            // What each item's next stage asks, with the project's unit and active sprint in hand
            // (FEAT-117, FEAT-119).
            let view = ReadinessView::load_in(store, p, &f.code, repo).ok();
            let docs_unbuilt = view
                .as_ref()
                .and_then(|v| {
                    let first = immediate_status(&project, &v.next)?;
                    v.next.iter().find(|n| n.status == first)
                })
                .filter(|n| !n.signoffs_held.is_empty())
                .and_then(|n| n.docs_built.as_ref())
                .map(|d| d.unbuilt.clone())
                .unwrap_or_default();
            let at_docs_folded = view
                .as_ref()
                .and_then(|v| {
                    let first = immediate_status(&project, &v.next)?;
                    v.next.iter().find(|n| n.status == first)
                })
                .is_some_and(|n| {
                    !n.signoffs_held.is_empty()
                        || n.signoffs_needed
                            .iter()
                            .any(|s| s == crate::method::DOCS_FOLDED)
                });
            let texts_to_settle = match repo {
                Some(repo) if at_docs_folded => {
                    crate::docdrift::flags_for_item(store, p, &f.code, repo)
                        .map(|flags| crate::review::split_settled(store, p, &f.code, flags).0)
                        .unwrap_or_default()
                        .iter()
                        .map(|fl| fl.target.key())
                        .collect()
                }
                _ => Vec::new(),
            };
            ItemCheck {
                docs_unbuilt,
                texts_to_settle,
                code: f.code.clone(),
                title: f.title.clone(),
                status: f.status.clone(),
                gaps: evaluate(f, None, CHECK)
                    .into_iter()
                    .map(|g| g.message)
                    .collect(),
                next: view.as_ref().map(|v| v.next.clone()).unwrap_or_default(),
                rev: view.and_then(|v| v.rev),
                open_manual: open_manual_tests(f),
            }
        })
        .collect();
    Ok(CheckReport { items })
}

/// What `kanbanr check --file` found in a definition that is not on a board (FEAT-052).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DefinitionCheck {
    /// Something to tell a maintainer that is not counted against the author: a goal link
    /// cannot be judged without the charter.
    pub note: Option<String>,
    pub gaps: Vec<String>,
}

/// Check a definition's text (YAML or JSON) by the rules the board applies (FEAT-052, FEAT-112),
/// minus what needs a board to judge. Text that does not parse is refused, saying so.
pub fn check_definition_text(text: &str) -> crate::Result<DefinitionCheck> {
    let definition: FeatureDefinition = crate::yaml::from_str(text).map_err(|e| {
        crate::CoreError::Refused(format!("invalid definition (YAML or JSON expected): {e}"))
    })?;
    // Not a gap that can be judged here: without the charter there is nothing to resolve an id
    // against. Reported so a maintainer knows to check it, not counted against the contributor.
    let note = definition
        .goals
        .is_empty()
        .then(|| "no goal link — the maintainer will check this against the charter".to_string());
    let gaps = evaluate_definition(Some(&definition), None, None, FILE)
        .into_iter()
        .map(|g| g.message)
        .collect();
    Ok(DefinitionCheck { note, gaps })
}

#[cfg(test)]
mod check_tests {
    use super::*;

    /// FEAT-176: a definition that does not parse is refused in the words `check --file` used;
    /// one with no goal carries the maintainer's note.
    #[test]
    fn check_definition_text_notes_and_refuses() {
        let err = check_definition_text("statement: [")
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("invalid definition (YAML or JSON expected): "),
            "{err}"
        );
        let checked = check_definition_text("statement: \"x for y so that z\"\n").unwrap();
        assert!(checked.note.is_some());
        assert!(!checked.gaps.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Requirement, TestRef};

    /// An item whose design-review pinned `spec`, then holding `now` as its spec (FTR-023).
    fn pinned_item(spec: &str, now: &str) -> FeatureItem {
        let mut def = crate::models::FeatureDefinition {
            statement: "Keep a cart".into(),
            ..Default::default()
        };
        def.signoffs.push(crate::models::Signoff {
            id: "design-review".into(),
            by: "Ada".into(),
            rev: def.content_rev(),
            spec_rev: spec_pin(spec).1,
            ..Default::default()
        });
        let mut item: FeatureItem = crate::yaml::from_str(
            "code: FTR-1\ntitle: T\nspecification: ''\nstatus: Build\nmilestone: ''\n\
             created_at: '2026-11-06T00:00:00Z'\nupdated_at: '2026-11-06T00:00:00Z'\n",
        )
        .unwrap();
        item.definition = Some(def);
        item.specification = now.into();
        item
    }

    /// FTR-023 R-3: whitespace at line ends, line endings and outer blank lines are not changes.
    #[test]
    fn spec_pin_ignores_trailing_whitespace() {
        let base = "## Why\nA cart.\n\n## Design\nUse a map.";
        let rev = spec_pin(base).1;
        assert!(!rev.is_empty());
        assert_eq!(
            spec_pin("\n\n## Why  \r\nA cart.\t\r\n\r\n## Design\r\nUse a map.\r\n\n").1,
            rev
        );
        assert_ne!(spec_pin("## Why\nA cart!\n\n## Design\nUse a map.").1, rev);
        assert_ne!(spec_pin(&format!("{base}\nMore.")).1, rev);
        assert_ne!(spec_pin(&format!("{base}\n\n## Notes\nx")).1, rev);
        assert_eq!(spec_pin("  \n\n"), (String::new(), String::new()));
    }

    /// FTR-023 R-2: a change anywhere in the spec lapses a sign-off that pinned it.
    #[test]
    fn a_changed_spec_lapses_its_signoff() {
        let names = vec!["design-review".to_string()];
        let spec = "## Why\nA cart.\n\n## Design\nUse a map.";
        assert!(signoff_gaps(&pinned_item(spec, spec), &names).is_empty());
        for edited in [
            "## Why\nA basket.\n\n## Design\nUse a map.",
            "## Why\nA cart.\n\n## Design\nUse a vec.",
            "## Why\nA cart.\n\n## Design\nUse a map.\n\n## Notes\nnew",
        ] {
            let gaps = signoff_gaps(&pinned_item(spec, edited), &names);
            assert_eq!(gaps.len(), 1, "{edited}");
            assert_eq!(gaps[0].label, "sign-off 'design-review': spec changed");
            assert!(gaps[0].message.contains("the spec changed after sign-off"));
        }
    }

    /// FTR-023 R-12: a sign-off with no spec pin is judged by the definition alone.
    #[test]
    fn a_signoff_without_a_spec_pin_is_judged_as_before() {
        let names = vec!["design-review".to_string()];
        let mut item = pinned_item("", "anything at all");
        item.definition.as_mut().unwrap().signoffs[0].spec_rev = String::new();
        assert!(signoff_gaps(&item, &names).is_empty());
        item.specification = "something else".into();
        assert!(signoff_gaps(&item, &names).is_empty());
        item.definition.as_mut().unwrap().statement = "Keep a cart longer".into();
        assert_eq!(
            signoff_gaps(&item, &names)[0].label,
            "sign-off 'design-review' lapsed"
        );
    }

    /// A board in a temp git repository with one item declaring `architecture/data.md` (FEAT-180).
    /// The doc is committed before the item is created: stage entry must not fall in an earlier
    /// second than the commit that holds the doc (MNT-008).
    fn docs_board(tag: &str) -> (crate::Store, std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("kanbanr-docs-{tag}-{}", std::process::id()));
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
        store
            .write_doc("demo", "architecture/data.md", "# Data\n\nv1\n")
            .unwrap();
        assert!(crate::git::commit(&dir, "Ada", "ada@example.com", "docs"));
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart".into(),
                    affects_docs: vec!["architecture/data".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        assert!(crate::git::commit(&dir, "Ada", "ada@example.com", "board"));
        (store, dir, item.code)
    }

    fn docs_gaps(store: &crate::Store, code: &str) -> Vec<Gap> {
        let project = store.load("demo").unwrap();
        let ctx = Context::load(store, "demo", &project.config);
        evaluate_conditions(
            project.feature(code).unwrap(),
            None,
            &[Condition::Check(Check::DocsUpdated)],
            &ctx,
        )
    }

    /// Commit times are whole seconds: keep "entered the stage" and "changed the doc" apart.
    fn next_second() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    /// DEF-033 R-4: the next gate holds `docs-folded` while a listed document is unbuilt, names
    /// it, counts nothing "for you", and leaves the other sign-offs alone.
    #[test]
    fn next_gate_holds_docs_folded_and_names_the_unbuilt_documents() {
        let (store, dir, code) = docs_board("held");
        let gate = crate::config::Gate {
            signoffs: vec!["design-review".into(), "docs-folded".into()],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        let next = |store: &crate::Store| {
            let project = store.load_for_gates("demo").unwrap();
            let ctx = Context::load(store, "demo", &project.config);
            let f = project.feature(&code).unwrap();
            (
                next_gates(&project, None, f, &ctx),
                signoff_hold(&project, f, &ctx, "docs-folded"),
                signoff_hold(&project, f, &ctx, "design-review"),
            )
        };
        let (gates, hold, other) = next(&store);
        assert_eq!(gates[0].signoffs_needed, vec!["design-review".to_string()]);
        assert_eq!(gates[0].signoffs_held.len(), 1, "{gates:?}");
        assert!(
            gates[0].signoffs_held[0]
                .message
                .contains("architecture/data.md")
        );
        assert!(gates[0].gaps.iter().any(|g| g.label == "docs not built"));
        assert!(hold.unwrap().contains("architecture/data.md"));
        assert!(other.is_none());
        let card = ProjectReadiness::load(&store, "demo").unwrap();
        let n = &card.0.iter().find(|(c, _)| c == &code).unwrap().1.next[0];
        assert_eq!(n.for_you, 1, "only design-review is for the person");

        // Folded in: the sign-off is offered.
        store
            .write_doc("demo", "architecture/data.md", "# Data\n\nv2\n")
            .unwrap();
        let (gates, hold, _) = next(&store);
        assert_eq!(
            gates[0].signoffs_needed,
            vec!["design-review".to_string(), "docs-folded".to_string()]
        );
        assert!(gates[0].signoffs_held.is_empty() && hold.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-205 R-1: a card's next stage names the sign-offs it still needs and the tasks still
    /// open, apart from its unmet conditions — so "ready" can only mean ready.
    #[test]
    fn next_stage_names_signoffs_and_open_tasks() {
        let (store, dir, code) = docs_board("next-stage");
        let gate = crate::config::Gate {
            requires: vec![Condition::Check(Check::Definition)],
            signoffs: vec!["docs-folded".into()],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        let list = store.add_todo_list("demo", &code, "work", None).unwrap();
        store
            .add_task("demo", &code, &list.code, "one", None)
            .unwrap();
        store
            .add_task("demo", &code, &list.code, "two", None)
            .unwrap();
        store
            .set_task_state(
                "demo",
                &code,
                &list.code,
                "T1",
                crate::models::TaskState::Completed,
            )
            .unwrap();
        let card = |store: &crate::Store| {
            ProjectReadiness::load(store, "demo")
                .unwrap()
                .0
                .into_iter()
                .find(|(c, _)| c == &code)
                .unwrap()
                .1
        };
        // The doc it declares is folded in, so `docs-folded` is offered (DEF-033).
        next_second();
        store
            .write_doc("demo", "architecture/data.md", "# Data\n\nv2\n")
            .unwrap();
        let next = &card(&store).next;
        assert_eq!(next.len(), 1, "{next:?}");
        assert_eq!(next[0].status, "Scheduled");
        assert_eq!(
            next[0].missing, 1,
            "the definition is there; the sign-off is not"
        );
        assert_eq!(next[0].signoffs, vec!["docs-folded".to_string()]);
        assert_eq!(next[0].tasks_open, 1);

        store
            .signoff_feature("demo", &code, "docs-folded", "Ada", "", "")
            .unwrap();
        store
            .set_task_state(
                "demo",
                &code,
                &list.code,
                "T2",
                crate::models::TaskState::Completed,
            )
            .unwrap();
        let next = &card(&store).next;
        assert!(
            next.iter()
                .all(|n| n.signoffs.is_empty() && n.tasks_open == 0),
            "{next:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A TOGAF board with one item at Implementation: a red test, a statement and goal, and a
    /// Migration gate that asks for green tests, agreement and the `docs-folded` sign-off (DEF-020).
    fn togaf_item_at_implementation(tag: &str) -> (crate::Store, std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("kanbanr-card-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/workflow",
            Some(&serde_json::json!({"preset": "togaf"})),
        )
        .unwrap();
        // The preset has no move from Implementation to Operations (DEF-019); these cards are
        // about a workflow that lets an item skip ahead, so add that move back.
        let cfg = store.load("demo").unwrap().config;
        let mut transitions = cfg.transitions.clone();
        transitions
            .get_mut("Implementation")
            .unwrap()
            .push("Operations".into());
        store
            .set_workflow(
                "demo",
                cfg.statuses.clone(),
                transitions,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let item = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    statement: "Keep a cart".into(),
                    requirements: vec![Requirement {
                        id: "R-1".into(),
                        text: "WHEN a cart is idle, THE SYSTEM SHALL keep it.".into(),
                        tests: vec![TestRef {
                            name: "t1".into(),
                            state: crate::models::TestState::Red,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        // Finished work, so `docs-folded` is offered (DEF-033).
        let list = store
            .add_todo_list("demo", &item.code, "work", None)
            .unwrap();
        let task = store
            .add_task("demo", &item.code, &list.code, "build it", None)
            .unwrap();
        store
            .set_task_state(
                "demo",
                &item.code,
                &list.code,
                &task.key,
                crate::models::TaskState::Completed,
            )
            .unwrap();
        // Walk to Implementation with no gates in the way, then set the gates under test.
        store
            .set_gates("demo", std::collections::BTreeMap::new())
            .unwrap();
        for to in ["Business Arch", "System Design", "Implementation"] {
            store.move_feature("demo", &item.code, to).unwrap();
        }
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Migration".to_string(),
            crate::config::Gate {
                requires: vec![
                    Condition::Check(Check::TestsGreen),
                    Condition::Check(Check::Approved),
                ],
                signoffs: vec!["docs-folded".into()],
                ..Default::default()
            },
        );
        gates.insert(
            "Operations".to_string(),
            crate::config::Gate {
                requires: vec![Condition::Check(Check::TestsGreen)],
                signoffs: vec!["release".into()],
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        (store, dir, item.code)
    }

    fn card_of(store: &crate::Store, code: &str) -> CardReadiness {
        ProjectReadiness::load(store, "demo")
            .unwrap()
            .0
            .into_iter()
            .find(|(c, _)| c == code)
            .unwrap()
            .1
    }

    /// DEF-020 R-1: a card counts every gap `kanbanr check` lists for the stage, sign-offs
    /// included.
    #[test]
    fn card_next_stage_counts_match_check() {
        let (store, dir, code) = togaf_item_at_implementation("counts");
        let report = check_board(&store, "demo", Some(&code), None).unwrap();
        let checked = &report.items[0].next;
        let card = card_of(&store, &code);
        assert!(!card.next.is_empty());
        for stage in &card.next {
            let listed = checked
                .iter()
                .find(|n| n.status == stage.status)
                .unwrap_or_else(|| panic!("check lists {}", stage.status));
            assert_eq!(
                stage.missing,
                listed.gaps.len(),
                "{}: the card and check count the same",
                stage.status
            );
        }
        let migration = card.next.iter().find(|n| n.status == "Migration").unwrap();
        assert_eq!(migration.missing, 3, "green tests, agreement, one sign-off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-020 R-2: of the stages an item can reach, the card marks the one reached next, the one
    /// the Review page asks sign-offs for.
    #[test]
    fn card_marks_the_immediate_next_stage() {
        let (store, dir, code) = togaf_item_at_implementation("immediate");
        let card = card_of(&store, &code);
        let flag = |s: &str| card.next.iter().find(|n| n.status == s).unwrap().immediate;
        assert!(flag("Migration"));
        assert!(!flag("Operations"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-020 R-3: `for_you` counts sign-offs and the `approved` check, nothing else.
    #[test]
    fn card_counts_what_is_the_users() {
        let (store, dir, code) = togaf_item_at_implementation("foryou");
        let card = card_of(&store, &code);
        let migration = card.next.iter().find(|n| n.status == "Migration").unwrap();
        assert_eq!(migration.missing, 3);
        assert_eq!(
            migration.for_you, 2,
            "approval and the sign-off; not the red test"
        );
        let operations = card.next.iter().find(|n| n.status == "Operations").unwrap();
        assert_eq!(operations.for_you, 1, "the release sign-off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-180 R-2: `affects_docs` is kept, does not reopen the agreement, and reads back the same
    /// through an older binary that keeps it as an unknown key.
    #[test]
    fn affects_docs_round_trips() {
        let (store, dir, code) = docs_board("roundtrip");
        let def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        // Stored as the board path, `.md` appended.
        assert_eq!(def.affects_docs, vec!["architecture/data.md".to_string()]);

        let mut bare = def.clone();
        bare.affects_docs.clear();
        assert_eq!(
            def.content_rev(),
            bare.content_rev(),
            "naming the docs does not lapse the approval (Q1)"
        );

        // An older binary keeps the key in `extra`, which is written last: the same bytes.
        let mut older = bare.clone();
        older.extra.insert(
            "affects_docs".into(),
            serde_json::to_value(vec!["architecture/data.md"]).unwrap(),
        );
        assert_eq!(
            crate::yaml::to_string(&def).unwrap(),
            crate::yaml::to_string(&older).unwrap()
        );
        let back: FeatureDefinition =
            crate::yaml::from_str(&crate::yaml::to_string(&older).unwrap()).unwrap();
        assert_eq!(back.affects_docs, def.affects_docs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-180 R-3: a declared doc must change after the item entered its stage. Unchanged
    /// blocks; changed before entry still blocks; an uncommitted change and a committed one after
    /// entry pass.
    #[test]
    fn docs_updated_gate() {
        let (store, dir, code) = docs_board("gate");
        next_second();
        store.move_feature("demo", &code, "Scheduled").unwrap();
        assert!(crate::git::commit(&dir, "Ada", "ada@example.com", "move"));
        next_second();

        let gaps = docs_gaps(&store, &code);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(
            gaps[0]
                .message
                .contains("architecture/data.md has not changed since"),
            "{}",
            gaps[0].message
        );
        assert_eq!(gaps[0].level, Level::Warning);

        // Written, not yet committed: counts.
        store
            .write_doc("demo", "architecture/data.md", "# Data\n\nv2\n")
            .unwrap();
        assert!(docs_gaps(&store, &code).is_empty());
        // Committed after entry: still counts.
        assert!(crate::git::commit(
            &dir,
            "Ada",
            "ada@example.com",
            "fold design"
        ));
        assert!(docs_gaps(&store, &code).is_empty());

        // Entering the next stage resets the clock: the change was before it.
        next_second();
        store.move_feature("demo", &code, "Planned").unwrap();
        assert!(crate::git::commit(&dir, "Ada", "ada@example.com", "move"));
        assert_eq!(docs_gaps(&store, &code).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A board of `items` items, each declaring `docs` of the docs it holds, with a gate on
    /// `Scheduled` that asks for `docs_updated` (DEF-022).
    fn many_docs_board(
        tag: &str,
        items: usize,
        docs: usize,
        gate: crate::config::Gate,
    ) -> (crate::Store, std::path::PathBuf) {
        let (store, dir, _) = docs_board(tag);
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();
        for d in 0..6 {
            store
                .write_doc("demo", &format!("architecture/d{d}.md"), "# D\n")
                .unwrap();
        }
        for i in 0..items {
            let item = store
                .add_feature("demo", &format!("Item {i}"), "", "M", None)
                .unwrap();
            let affects_docs = (0..docs)
                .map(|j| format!("architecture/d{}.md", (i + j) % 6))
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
        }
        assert!(crate::git::commit(&dir, "Ada", "ada@example.com", "items"));
        (store, dir)
    }

    fn docs_gate() -> crate::config::Gate {
        crate::config::Gate {
            warns: vec![Condition::Check(Check::DocsUpdated)],
            ..Default::default()
        }
    }

    /// DEF-022 R-2: one evaluation over many items and docs opens the repository once and walks
    /// its history once.
    #[test]
    fn next_gates_for_many_items_share_one_history() {
        let (store, dir) = many_docs_board("share", 20, 3, docs_gate());
        let project = store.load_for_gates("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        let mut judged = 0;
        for f in project
            .features
            .iter()
            .filter(|f| f.title.starts_with("Item"))
        {
            let next = next_gates(&project, None, f, &ctx);
            judged += next.iter().filter(|n| n.status == "Scheduled").count();
        }
        assert_eq!(judged, 20);
        assert_eq!(
            ctx.history.stats(),
            crate::git::HistoryStats { opens: 1, walks: 1 }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-022 R-3: nothing to judge, nothing read. A gate that does not ask for `docs_updated`,
    /// or items that declare no docs, leave the history alone.
    #[test]
    fn no_declared_docs_reads_no_history() {
        let none = crate::git::HistoryStats { opens: 0, walks: 0 };
        // Items declare docs; the gate does not ask.
        let gate = crate::config::Gate {
            warns: vec![Condition::Check(Check::Definition)],
            ..Default::default()
        };
        let (store, dir) = many_docs_board("lazy-gate", 5, 3, gate);
        let project = store.load_for_gates("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        for f in &project.features {
            next_gates(&project, None, f, &ctx);
        }
        assert_eq!(ctx.history.stats(), none);
        let _ = std::fs::remove_dir_all(&dir);

        // The gate asks; no item declares a doc.
        let (store, dir) = many_docs_board("lazy-docs", 5, 0, docs_gate());
        let project = store.load_for_gates("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        for f in project
            .features
            .iter()
            .filter(|f| f.title.starts_with("Item"))
        {
            next_gates(&project, None, f, &ctx);
        }
        assert_eq!(ctx.history.stats(), none);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-180 R-4: a declared doc the board does not have is an error, not a pending change.
    #[test]
    fn affects_docs_unknown_is_error() {
        let (store, dir, code) = docs_board("unknown");
        let mut def = store
            .load("demo")
            .unwrap()
            .feature(&code)
            .unwrap()
            .definition
            .clone()
            .unwrap();
        def.affects_docs.push("architecture/nope.md".into());
        store
            .set_feature_definition("demo", &code, Some(def))
            .unwrap();
        let gaps = docs_gaps(&store, &code);
        let unknown: Vec<&Gap> = gaps.iter().filter(|g| g.level == Level::Error).collect();
        assert_eq!(unknown.len(), 1, "{gaps:?}");
        assert_eq!(
            unknown[0].message,
            "architecture/nope.md is declared in affects_docs but is not on the board"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// DEF-043 R-2: an override Claude makes on an approved item waits for the person to ratify
    /// it, is a `Bypass` gap, is asked on the Review page and keeps the item from shipping; once
    /// ratified it ships. The person's own override opens nothing.
    #[test]
    fn an_override_from_claude_waits_for_ratification_and_holds_the_cut() {
        let (store, dir, repo) = crate::releases::tests::board_and_repo("def043-override");
        let approved = |title: &str| {
            let code = store
                .add_feature("shop", title, "", "M", None)
                .unwrap()
                .code;
            store
                .set_feature_definition(
                    "shop",
                    &code,
                    Some(FeatureDefinition {
                        statement: "Run CI".into(),
                        ..Default::default()
                    }),
                )
                .unwrap();
            store.approve_feature("shop", &code, "Ada").unwrap();
            crate::releases::plan(&store, "shop", "v1", std::slice::from_ref(&code)).unwrap();
            store
                .move_feature_approved("shop", &code, "Scheduled", None)
                .unwrap();
            code
        };
        let code = approved("CI");
        store
            .move_feature_gated_as(
                "shop",
                &code,
                "Completed",
                Some("CI after push"),
                "conversation",
            )
            .unwrap();
        let f = store.load("shop").unwrap().feature(&code).unwrap().clone();
        assert_eq!(open_overrides(&f).len(), 1);
        let gaps = evaluate(&f, None, &[Check::Bypass]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].label, "override not ratified");
        assert!(
            gaps[0].message.contains("CI after push"),
            "{}",
            gaps[0].message
        );
        let queue = crate::review::pending(&store, "shop", None).unwrap();
        let entry = queue.iter().find(|e| e.code == code).expect("asked");
        assert_eq!(entry.approval, crate::review::ApprovalKind::Unratified);
        assert_eq!(entry.overrides.len(), 1);
        let preview = crate::releases::cut_preview(&store, "shop", "v1").unwrap();
        let carried = preview.iter().find(|p| p.code == code).unwrap();
        assert!(!carried.ships);
        assert!(
            carried.why.contains("waits for the person to ratify"),
            "{}",
            carried.why
        );

        store
            .ratify_feature_as("shop", &code, "Ada", "agreed", "monitor")
            .unwrap();
        let f = store.load("shop").unwrap().feature(&code).unwrap().clone();
        assert!(open_overrides(&f).is_empty());
        let preview = crate::releases::cut_preview(&store, "shop", "v1").unwrap();
        assert!(preview.iter().find(|p| p.code == code).unwrap().ships);
        let queue = crate::review::pending(&store, "shop", None).unwrap();
        assert!(!queue.iter().any(|e| e.code == code), "{queue:?}");

        let mine = approved("Mine");
        store
            .move_feature_gated_as("shop", &mine, "Completed", Some("mine"), "terminal")
            .unwrap();
        let f = store.load("shop").unwrap().feature(&mine).unwrap().clone();
        assert!(open_overrides(&f).is_empty());
        crate::releases::tests::clean(&dir, &repo);
    }

    /// DEF-043 R-1: a requirement whose only not-green test is checked later passes the item's
    /// `TestsGreen`; with one more plain planned test it does not.
    #[test]
    fn a_test_checked_later_does_not_hold_the_items_gate() {
        let mut later = requirement("R-2", "ci", &[("ci::runs", TestState::Planned)]);
        later.tests[0].at = "pushed".into();
        let f = item(Some(FeatureDefinition {
            requirements: vec![later.clone()],
            ..Default::default()
        }));
        let gaps = evaluate(&f, None, &[Check::TestsGreen]);
        assert!(gaps.is_empty(), "{gaps:?}");
        later.tests.push(TestRef {
            name: "plain".into(),
            ..Default::default()
        });
        let f = item(Some(FeatureDefinition {
            requirements: vec![later],
            ..Default::default()
        }));
        let gaps = evaluate(&f, None, &[Check::TestsGreen]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].label, "not proven");
    }

    /// DEF-003 R-1: a manual test not green is named with its requirement; a green one, or one of
    /// another kind, is not.
    #[test]
    fn open_manual_tests_are_named() {
        let mut open = requirement("R-4", "edit", &[("Edit and Save", TestState::Planned)]);
        open.tests[0].kind = "manual".into();
        let mut done = requirement("R-5", "view", &[("Look", TestState::Green)]);
        done.tests[0].kind = "manual".into();
        let auto = requirement("R-6", "auto", &[("t", TestState::Red)]);
        let f = item(Some(FeatureDefinition {
            requirements: vec![open, done, auto],
            ..Default::default()
        }));
        assert_eq!(
            open_manual_tests(&f),
            vec!["R-4: Edit and Save".to_string()]
        );
        assert!(open_manual_tests(&item(None)).is_empty());
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
        // Any list that reads the definition says, once, that there is none — even one that does
        // not name `definition` itself. A gate asking for a statement once waved an undefined item
        // through, because the missing definition had nothing to be reported as.
        let file = evaluate(&bare, None, FILE);
        assert_eq!(file.len(), 1);
        assert_eq!(file[0].check, Check::Definition);
        assert!(
            !evaluate(&bare, None, &[Check::Estimated, Check::Small])
                .iter()
                .any(|g| g.check == Check::Definition),
            "checks that do not read the definition do not ask for one"
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
            extra: Default::default(),
            delegation: String::new(),
            by: "A".into(),
            at: "2026-09-29T00:00:00Z".into(),
            rev: def.content_rev(),
            parts: Default::default(),
            origin: String::new(),
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

    /// An item whose approval has only grown since it was given (FTR-018).
    fn grown_item() -> FeatureItem {
        use crate::models::Approval;
        let mut def = FeatureDefinition {
            statement: "s".into(),
            ..Default::default()
        };
        def.approval = Some(Approval {
            by: "A".into(),
            at: "2026-09-29T00:00:00Z".into(),
            rev: def.content_rev(),
            parts: def.part_revs(),
            ..Default::default()
        });
        def.zachman.how = "by hashing".into();
        assert_eq!(def.approval_state(), ApprovalState::Extended);
        item(Some(def))
    }

    fn approved_only() -> Vec<Condition> {
        vec![Condition::Check(Check::Approved)]
    }

    /// FTR-018 R-5: before the build stage the additions wait, so `approved` is met.
    #[test]
    fn an_extended_approval_passes_a_gate_before_build() {
        let f = grown_item();
        let ctx = Context {
            build_onward: Some(["Build".to_string(), "Done".to_string()].into()),
            ..Default::default()
        };
        assert!(additions_gap(&f, &approved_only(), &[], &ctx, "Design").is_none());
        let gaps = evaluate_entering(&f, None, &approved_only(), &ctx, "Design");
        assert!(gaps.is_empty(), "{gaps:?}");
        // The item-level check does not report it either.
        assert!(evaluate(&f, None, &[Check::Approved]).is_empty());
    }

    /// FTR-018 R-6: from the build stage on, a gate with no sign-off refuses the additions and
    /// names them; one that asks for a sign-off leaves that to the sign-off.
    #[test]
    fn additions_need_a_decision_before_build() {
        let f = grown_item();
        let ctx = Context {
            build_onward: Some(["Build".to_string(), "Done".to_string()].into()),
            ..Default::default()
        };
        let gap = additions_gap(&f, &approved_only(), &[], &ctx, "Build").unwrap();
        assert_eq!(gap.label, "added since approval");
        assert!(gap.message.contains("zachman/how"), "{}", gap.message);
        let with_signoff = ["design-review".to_string()];
        assert!(additions_gap(&f, &approved_only(), &with_signoff, &ctx, "Build").is_none());
        // A gate that does not ask for approval has nothing to refuse.
        assert!(additions_gap(&f, &[], &[], &ctx, "Build").is_none());
        // No build role declared: every stage counts, as before.
        assert!(additions_gap(&f, &approved_only(), &[], &Context::default(), "Design").is_some());
        assert!(Context::default().builds("x"));
    }

    #[test]
    fn every_check_is_listed_with_a_description() {
        let mut names: Vec<String> = Check::GATEABLE.iter().map(|c| c.name()).collect();
        for check in Check::GATEABLE {
            let name = check.name();
            assert!(!name.is_empty(), "{check:?} has no name");
            assert!(!check.passes_when().is_empty(), "{name} says nothing");
            let back: Check = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(back, check, "{name} does not read back");
        }
        assert!(
            !names.contains(&Check::Signoff.name()),
            "signoff is named, not listed"
        );
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            Check::GATEABLE.len(),
            "a check is listed twice"
        );
    }

    /// FEAT-168: the guide's table of checks says what `process checks` says, row for row.
    #[test]
    fn the_guides_check_table_matches_process_checks() {
        let guide = include_str!("../../../../docs/src/using/processes.md");
        let section = guide
            .split("### The checks")
            .nth(1)
            .expect("the guide has a checks section");
        let rows: Vec<(String, String)> = section
            .lines()
            .skip_while(|l| !l.starts_with("| Check"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split(" | ").collect();
                (
                    cells[0].trim().trim_matches('`').to_string(),
                    cells[1].trim().to_string(),
                )
            })
            .collect();
        let want: Vec<(String, String)> = Check::GATEABLE
            .iter()
            .map(|c| (c.name(), c.passes_when().to_string()))
            .collect();
        assert_eq!(rows, want);
    }

    /// FEAT-179 R-2: a design stage asks for the design itself. A missing section and an empty
    /// one are both gaps; nested content counts; a heading in a code fence is not a heading; an
    /// exempt item has no gaps.
    #[test]
    fn spec_sections_gate() {
        let with_spec = |spec: &str| {
            let mut f = item(Some(FeatureDefinition {
                statement: "s".into(),
                ..Default::default()
            }));
            f.specification = spec.into();
            f
        };
        let ask = |names: &[&str]| {
            vec![Condition::SpecSections {
                spec_sections: names.iter().map(|n| n.to_string()).collect(),
            }]
        };
        let ctx = Context::default();
        let gaps =
            |f: &FeatureItem, names: &[&str]| evaluate_conditions(f, None, &ask(names), &ctx);

        let none = with_spec("# Title\n\nWhy it exists.\n");
        let g = gaps(&none, &["Design"]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].check, Check::SpecSections);
        assert_eq!(g[0].label, "no section 'Design'");

        let empty = with_spec("# Title\n\n## Design\n\n<!-- to do -->\n\n## Tests\n\nt\n");
        assert_eq!(gaps(&empty, &["Design"])[0].label, "empty section 'Design'");

        let nested = with_spec("# T\n\n## design\n\n### Data model\n\nstruct Shard { .. }\n");
        assert!(
            gaps(&nested, &["Design"]).is_empty(),
            "case-insensitive, nested content counts"
        );

        let fenced = with_spec("# T\n\n```md\n## Design\ntext\n```\n");
        assert_eq!(
            gaps(&fenced, &["Design"]).len(),
            1,
            "a fenced heading is not a section"
        );

        let two = with_spec("## Design\n\nx\n");
        let g = gaps(&two, &["Design", "Test plan"]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].label, "no section 'Test plan'");

        // Plain `spec_sections` asks for Design.
        let plain = vec![Condition::Check(Check::SpecSections)];
        assert_eq!(evaluate_conditions(&none, None, &plain, &ctx).len(), 1);
        assert!(evaluate_conditions(&nested, None, &plain, &ctx).is_empty());

        let mut exempt = with_spec("");
        exempt.definition.as_mut().unwrap().exempt = "predates the method".into();
        assert!(gaps(&exempt, &["Design"]).is_empty());
    }

    /// FEAT-179 R-3: `spec_sections` is in the gate vocabulary, reads from a gate both ways, and
    /// shows in the working agreement. A gate that reads specs says so, so readers load them.
    #[test]
    fn spec_sections_in_vocabulary() {
        assert!(Check::GATEABLE.contains(&Check::SpecSections));
        assert_eq!(Check::SpecSections.name(), "spec_sections");
        assert!(Check::SpecSections.parameters().is_some());
        let gate: crate::config::Gate = crate::yaml::from_str(
            "requires:\n  - spec_sections\n  - spec_sections: [Design, Test plan]\n",
        )
        .expect("both forms read");
        assert_eq!(gate.requires[0], Condition::Check(Check::SpecSections));
        assert_eq!(
            gate.requires[1],
            Condition::SpecSections {
                spec_sections: vec!["Design".into(), "Test plan".into()]
            }
        );
        assert!(gate.reads_spec());
        let mut config = crate::ProjectConfig::default_for("demo");
        config.gates.insert("Scheduled".into(), gate);
        let agreement = crate::config::working_agreement("demo", &config);
        assert!(
            agreement.contains("spec sections: Design, Test plan"),
            "{agreement}"
        );
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

    /// FEAT-200 R-12: `diagrams_valid` is a check a gate can name — in `requires` or `warns` — and
    /// `process checks` lists it with what it means. It needs no definition, and it reads the spec.
    #[test]
    fn diagrams_valid_in_vocabulary() {
        assert!(Check::GATEABLE.contains(&Check::DiagramsValid));
        assert_eq!(Check::DiagramsValid.name(), "diagrams_valid");
        assert!(Check::DiagramsValid.passes_when().contains("diagram lint"));
        assert_eq!(Check::DiagramsValid.parameters(), None);
        let gate: crate::config::Gate =
            crate::yaml::from_str("requires: [diagrams_valid]\nwarns: [diagrams_valid]\n").unwrap();
        assert_eq!(gate.requires, vec![Condition::Check(Check::DiagramsValid)]);
        assert_eq!(gate.warns, vec![Condition::Check(Check::DiagramsValid)]);
        assert!(
            gate.requires[0].reads_spec(),
            "a metadata-only load would miss it"
        );

        let mut f = item(None);
        assert!(
            evaluate(&f, None, &[Check::DiagramsValid]).is_empty(),
            "no definition is not a diagram problem"
        );
        f.specification = "```mermaid\nflowchart LR\n  A --> end\n```\n".into();
        let gaps = evaluate(&f, None, &[Check::DiagramsValid]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].check, Check::DiagramsValid);
        assert_eq!(gaps[0].label, "diagram #1 in FEAT-001 spec");
        assert!(
            gaps[0]
                .message
                .contains("FEAT-001 spec, diagram #1, line 3")
        );
        assert!(gaps[0].message.ends_with("(end_as_id)"));
        // An exempt item has no gaps, as with every other check.
        f.definition = Some(FeatureDefinition {
            exempt: "a spike".into(),
            ..Default::default()
        });
        assert!(evaluate(&f, None, &[Check::DiagramsValid]).is_empty());
    }

    /// FEAT-200 R-13: a gate asking for `diagrams_valid` refuses the move while the spec, or a doc
    /// in `affects_docs`, has a diagram the lint rejects — naming file, block, line and rule — and a
    /// fix unblocks the very next move: the lint runs at move time, never cached.
    #[test]
    fn diagrams_valid_gate_blocks_and_names_findings() {
        let dir = std::env::temp_dir().join(format!("kanbanr-diagram-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let charter = crate::Charter {
            purpose: "Draw diagrams that draw".into(),
            adopted_at: "2000-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        crate::charter::save(&store, "demo", &charter).unwrap();
        let gate = crate::config::Gate {
            requires: vec![Condition::Check(Check::DiagramsValid)],
            ..Default::default()
        };
        let mut gates = std::collections::BTreeMap::new();
        gates.insert("Scheduled".to_string(), gate);
        store.set_gates("demo", gates).unwrap();

        let broken = "```mermaid\nsequenceDiagram\n  A->>B: one; two\n```\n";
        let fixed = "```mermaid\nsequenceDiagram\n  A->>B: one, two\n```\n";
        let item = store
            .add_feature("demo", "Flow", broken, "M", None)
            .unwrap();
        let code = item.code.clone();
        let refused = |store: &crate::Store| {
            store
                .move_feature_gated("demo", &code, "Scheduled", None)
                .unwrap_err()
                .to_string()
        };
        let err = refused(&store);
        assert!(
            err.contains(&format!("{code} spec, diagram #1, line 3"))
                && err.contains("semicolon_in_text"),
            "{err}"
        );
        store
            .edit_feature("demo", &code, None, Some(fixed.into()), None, None)
            .unwrap();
        store
            .move_feature_gated("demo", &code, "Scheduled", None)
            .unwrap();

        // The same for a doc the item names in `affects_docs`.
        store.move_feature("demo", &code, "Planned").unwrap();
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(FeatureDefinition {
                    statement: "Draw the flow".into(),
                    affects_docs: vec!["architecture/flow".into(), "architecture/missing".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        let doc = "# Flow\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```mermaid\nflowchart LR\n  A[f(x)] --> B\n```\n";
        store
            .write_doc("demo", "architecture/flow.md", doc)
            .unwrap();
        let err = refused(&store);
        assert!(
            err.contains("docs/architecture/flow.md, diagram #2, line 10")
                && err.contains("label_punctuation"),
            "{err}"
        );
        assert!(
            !err.contains("missing"),
            "a missing doc is docs_updated's to report: {err}"
        );
        store
            .write_doc(
                "demo",
                "architecture/flow.md",
                &doc.replace("A[f(x)]", "A[\"f(x)\"]"),
            )
            .unwrap();
        store
            .move_feature_gated("demo", &code, "Scheduled", None)
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A board with coverage rules, for FEAT-197's gate tests: `zachman` asks every item at
    /// Scheduled or later to be referenced from architecture/zachman.md, `listed` asks items in
    /// Scheduled to list it; the doc names FEAT-002 only.
    fn coverage_board(tag: &str) -> (crate::Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kanbanr-cover-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let charter = crate::Charter {
            purpose: "Keep the views whole".into(),
            adopted_at: "2000-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        crate::charter::save(&store, "demo", &charter).unwrap();
        for t in ["One", "Two"] {
            store.add_feature("demo", t, "", "M", None).unwrap();
        }
        store
            .write_doc("demo", "architecture/zachman.md", "| FEAT-002 |\n")
            .unwrap();
        let rules = crate::coverage::parse_rules(
            "- id: zachman\n  from: Scheduled\n  referenced_from: architecture/zachman.md\n\
             - id: listed\n  in: [Scheduled]\n  lists: architecture/zachman.md\n",
        )
        .unwrap();
        store.set_coverage("demo", rules).unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Scheduled".to_string(),
            crate::config::Gate {
                requires: vec![Condition::Check(Check::DocsCover)],
                ..Default::default()
            },
        );
        store.set_gates("demo", gates).unwrap();
        (store, dir)
    }

    /// FEAT-197 R-8: `docs_cover` is judged as if the item were already in the stage it enters —
    /// an item at Planned is not yet a subject of a rule from Scheduled, and is one on entering it;
    /// a narrowed condition asks only for its rules; an exempt item has no gaps.
    #[test]
    fn docs_cover_gate() {
        let (store, dir) = coverage_board("gate");
        let project = store.load_meta("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        assert!(
            ctx.coverage.is_some(),
            "a gate asks, and the project has rules"
        );
        let one = project.feature("FEAT-001").unwrap();
        let two = project.feature("FEAT-002").unwrap();
        let plain = vec![Condition::Check(Check::DocsCover)];
        assert!(
            evaluate_conditions(one, None, &plain, &ctx).is_empty(),
            "at Planned"
        );
        let gaps = evaluate_entering(one, None, &plain, &ctx, "Scheduled");
        let labels: Vec<&str> = gaps.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["[zachman] not covered", "[listed] not covered"]
        );
        assert!(gaps.iter().all(|g| g.check == Check::DocsCover));
        assert_eq!(
            gaps[0].message,
            "[zachman] FEAT-001 is not referenced from architecture/zachman.md — add a link to \
             it there"
        );
        let narrowed = vec![Condition::DocsCover {
            docs_cover: vec!["zachman".into()],
        }];
        let gaps = evaluate_entering(two, None, &narrowed, &ctx, "Scheduled");
        assert!(gaps.is_empty(), "FEAT-002 is in the matrix: {gaps:?}");
        let gaps = evaluate_entering(two, None, &plain, &ctx, "Scheduled");
        assert_eq!(gaps.len(), 1, "it does not list the doc: {gaps:?}");
        assert_eq!(
            gaps[0].message,
            "[listed] FEAT-002 does not list architecture/zachman.md in affects_docs"
        );
        // Past Scheduled, `listed` no longer applies; `zachman` still does.
        assert_eq!(
            evaluate_entering(one, None, &plain, &ctx, "Completed").len(),
            1
        );
        let mut exempt = one.clone();
        exempt.definition = Some(FeatureDefinition {
            exempt: "a spike".into(),
            ..Default::default()
        });
        assert!(evaluate_entering(&exempt, None, &plain, &ctx, "Scheduled").is_empty());
        // The gate refuses the move, naming the rule; a link in the doc lets it through.
        let err = store
            .move_feature_gated("demo", "FEAT-001", "Scheduled", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("[zachman] FEAT-001 is not referenced"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-197 R-14: a gate narrowing `docs_cover` to a rule the project does not configure is an
    /// Error gap naming it — with rules configured and without.
    #[test]
    fn docs_cover_unknown_rule_is_error() {
        let (store, dir) = coverage_board("unknown");
        let project = store.load_meta("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        let one = project.feature("FEAT-001").unwrap();
        let narrowed = vec![Condition::DocsCover {
            docs_cover: vec!["zachman".into(), "catalgo".into()],
        }];
        let gaps = evaluate_entering(one, None, &narrowed, &ctx, "Scheduled");
        let unknown: Vec<&Gap> = gaps.iter().filter(|g| g.level == Level::Error).collect();
        assert_eq!(unknown.len(), 1, "{gaps:?}");
        assert_eq!(
            unknown[0].message,
            "docs_cover names rule 'catalgo', which this project does not configure"
        );
        assert!(gaps.iter().any(|g| g.label == "[zachman] not covered"));
        // No rules at all: plain passes, a named rule is unknown.
        let none = Context::default();
        assert!(
            evaluate_entering(
                one,
                None,
                &[Condition::Check(Check::DocsCover)],
                &none,
                "Scheduled"
            )
            .is_empty()
        );
        let gaps = evaluate_entering(one, None, &narrowed, &none, "Scheduled");
        assert_eq!(gaps.len(), 2);
        assert!(gaps.iter().all(|g| g.level == Level::Error));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-197 R-12: `docs_cover` reads plain and narrowed, in `requires` and `warns`, says what
    /// it means and what narrows it, and reads no spec body.
    #[test]
    fn docs_cover_reads_and_narrows() {
        assert!(Check::GATEABLE.contains(&Check::DocsCover));
        assert_eq!(Check::DocsCover.name(), "docs_cover");
        assert!(Check::DocsCover.passes_when().contains("coverage rule"));
        assert!(
            Check::DocsCover
                .parameters()
                .is_some_and(|p| p.starts_with("docs_cover: [rule-id"))
        );
        let gate: crate::config::Gate = crate::yaml::from_str(
            "requires: [docs_cover, {docs_cover: [zachman, catalog]}]\nwarns: [docs_cover]\n",
        )
        .unwrap();
        assert_eq!(
            gate.requires,
            vec![
                Condition::Check(Check::DocsCover),
                Condition::DocsCover {
                    docs_cover: vec!["zachman".into(), "catalog".into()]
                }
            ]
        );
        assert_eq!(gate.warns, vec![Condition::Check(Check::DocsCover)]);
        assert!(gate.requires.iter().all(Condition::is_docs_cover));
        assert!(!gate.reads_spec());
        assert_eq!(gate.requires[1].invalid_column(), None);
        let back: crate::config::Gate =
            crate::yaml::from_str(&crate::yaml::to_string(&gate).unwrap()).unwrap();
        assert_eq!(back, gate, "round-trips");
        // No definition is not a coverage gap of its own.
        let f = item(None);
        assert!(evaluate(&f, None, &[Check::DocsCover]).is_empty());
    }

    /// FTR-019 R-5: a chore at Vision is told about Implementation and what its first move asks, not
    /// the stages its path skips; a feature still meets Business Arch.
    #[test]
    fn next_gates_for_a_chore_follow_its_path() {
        let dir = std::env::temp_dir().join(format!("kanbanr-chore-next-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        let config = crate::config::preset("togaf").unwrap().into_config("demo");
        store.init_project("demo", config).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let chore = store
            .add_feature_of_kind("demo", "Pin", "", "M", None, Some("chore"))
            .unwrap();
        let feature = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        for code in [&chore.code, &feature.code] {
            store
                .set_feature_definition("demo", code, Some(FeatureDefinition::default()))
                .unwrap();
        }
        let project = store.load("demo").unwrap();
        let ctx = Context::load(&store, "demo", &project.config);
        let next = |code: &str| next_gates(&project, None, project.feature(code).unwrap(), &ctx);

        let chore_next = next(&chore.code);
        let stages: Vec<&str> = chore_next.iter().map(|g| g.status.as_str()).collect();
        assert_eq!(stages, ["Implementation"]);
        let gaps: Vec<&str> = chore_next[0]
            .gaps
            .iter()
            .map(|g| g.message.as_str())
            .collect();
        let said = gaps.join(" | ");
        // The path's own questions, and the entered stage's.
        assert!(said.contains("statement"), "{said}");
        assert!(said.contains("requirement"), "{said}");
        assert!(said.contains("approv"), "{said}");
        // No Design section or design review for a chore.
        assert!(!said.contains("Design"), "{said}");
        assert!(
            chore_next[0].signoffs_needed.is_empty(),
            "{:?}",
            chore_next[0]
        );

        let stages: Vec<String> = next(&feature.code).into_iter().map(|g| g.status).collect();
        assert_eq!(stages, ["Business Arch"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-041 R-1: the first move into a working stage asks for a sprint that has started.
    #[test]
    fn a_working_stage_waits_for_the_sprint_to_start() {
        let in_sprint = Condition::Check(Check::InSprint);
        let config_of = |name: &str, sprints: bool| {
            let mut c = crate::config::preset_bare(name).unwrap().into_config("p");
            c.cadence.sprints = sprints;
            c
        };
        let togaf = config_of("togaf", true);
        assert!(
            togaf
                .first_move_requires(None, "Business Arch")
                .contains(&in_sprint)
        );
        assert!(
            !togaf
                .first_move_requires(None, "Operations")
                .contains(&in_sprint)
        );
        let mut with_chore = togaf.clone();
        with_chore.paths.insert(
            "chore".into(),
            crate::config::KindPath {
                stages: ["Vision", "Implementation", "Migration", "Operations"]
                    .map(String::from)
                    .to_vec(),
                requires: vec![Condition::Check(Check::Statement)],
                method_off: vec![],
            },
        );
        let chore = with_chore.first_move_requires(Some("chore"), "Implementation");
        assert!(chore.contains(&Condition::Check(Check::Statement)) && chore.contains(&in_sprint));
        // Scrum asks for it at In Progress already; Ready stays refinement before the sprint.
        let scrum = config_of("scrum", true);
        assert!(
            !scrum
                .first_move_requires(None, "Ready")
                .contains(&in_sprint)
        );
        assert!(
            config_of("togaf", false)
                .first_move_requires(None, "Business Arch")
                .is_empty()
        );

        let item = pinned_item("", "");
        let mut item = item;
        item.sprint = Some("SP-001".into());
        let conditions = [Check::InSprint];
        let mut ctx = Context {
            sprint_starts: [("SP-001".to_string(), "2026-11-16".to_string())].into(),
            ..Context::default()
        };
        let gaps = evaluate_in(&item, None, &conditions, &ctx);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].label, "sprint not started");
        assert!(gaps[0].message.contains("2026-11-16") && gaps[0].message.contains("SP-001"));
        ctx.active_sprint = Some("SP-001".into());
        ctx.sprint_starts.clear();
        assert!(evaluate_in(&item, None, &conditions, &ctx).is_empty());
        let off = Context {
            sprints_off: true,
            ..Context::default()
        };
        assert!(evaluate_in(&item, None, &conditions, &off).is_empty());
    }
}
