//! Portfolio / program hierarchy + cross-project rollups & board (FEAT-030).
//!
//! A root-level `workspace.yaml` (under the data dir root, NOT inside any project) declares
//! **programs** and which **projects** belong to each, plus light portfolio metadata. Projects are
//! left untouched — this is purely an *index* layered over the existing single-project store, not a
//! move. When no `workspace.yaml` is present everything falls into an implicit **default program**
//! that contains every project the store can list, so single-project setups keep working unchanged.
//!
//! Everything here is a **read** over the public `Store` API (`list_projects`/`load`); it never
//! touches the store internals, and only `add_program` writes (and it writes only `workspace.yaml`).

use crate::error::{CoreError, Result};
use crate::{Project, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The on-disk root index. Absent file ⇒ `Workspace::default()` (an empty program list, which the
/// rollup/board logic interprets as "one implicit default program over every project").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workspace {
    /// Optional display name for the whole portfolio.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The declared programs. Each lists the project ids it groups.
    #[serde(default)]
    pub programs: Vec<Program>,
}

/// A program: a named grouping of projects within the portfolio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Project ids belonging to this program (must exist in the store to contribute to rollups).
    #[serde(default)]
    pub projects: Vec<String>,
}

/// The id used for the implicit catch-all program when no `workspace.yaml` declares any program.
pub const DEFAULT_PROGRAM_ID: &str = "default";

/// The data-dir root, derived from the public `projects_dir()` (`<root>/projects`) so this module
/// never needs a private accessor on `Store`.
fn data_root(store: &Store) -> PathBuf {
    store
        .projects_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.projects_dir())
}

fn workspace_path(root: &Path) -> PathBuf {
    root.join("workspace.yaml")
}

/// Load `workspace.yaml` from the store's data-dir root, or `Workspace::default()` when absent.
pub fn load(store: &Store) -> Result<Workspace> {
    let path = workspace_path(&data_root(store));
    if !path.is_file() {
        return Ok(Workspace::default());
    }
    let text = std::fs::read_to_string(&path)?;
    let ws: Workspace = serde_yaml::from_str(&text)?;
    Ok(ws)
}

/// Persist `workspace.yaml` at the data-dir root. This is the only write in this module.
pub fn save(store: &Store, ws: &Workspace) -> Result<()> {
    let path = workspace_path(&data_root(store));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_yaml::to_string(ws)?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// Add (or replace) a program in `workspace.yaml`, then save. A program with the same id is
/// overwritten so the command is idempotent. Returns the updated workspace.
pub fn add_program(
    store: &Store,
    id: &str,
    name: Option<String>,
    description: Option<String>,
    projects: Vec<String>,
) -> Result<Workspace> {
    let id = id.trim();
    if id.is_empty() {
        return Err(CoreError::InvalidName(id.to_string()));
    }
    let mut ws = load(store)?;
    let program = Program {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        description: description.unwrap_or_default(),
        projects: projects
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    };
    if let Some(existing) = ws.programs.iter_mut().find(|p| p.id == id) {
        *existing = program;
    } else {
        ws.programs.push(program);
    }
    save(store, &ws)?;
    Ok(ws)
}

// ---- normalized disposition (cross-project lanes) ------------------------------------------

/// A project-independent lane a feature falls into, since statuses are per-project (no shared
/// vocabulary). Computed from the project config + the feature's task activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    /// Terminal: a "Completed"-named status OR a no-op disposition.
    Done,
    /// Active and has some task activity (at least one task, not all done).
    InProgress,
    /// Active but no task activity yet.
    NotStarted,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Disposition::Done => "done",
            Disposition::InProgress => "in-progress",
            Disposition::NotStarted => "not-started",
        }
    }
}

/// Is a status terminal for rollup/board purposes? A "Completed"-named status (case-insensitive) or
/// any configured no-op (inert disposition) state counts as terminal.
fn is_terminal(project: &Project, status: &str) -> bool {
    status.eq_ignore_ascii_case("Completed") || project.config.is_no_op(status)
}

/// Would this status appear on the project's own board? (FEAT-097)
///
/// The cross-project board is a view ACROSS project boards, so it shows what they show: the
/// statuses in `displayed_states`, or every status when that list is empty (the web board's rule).
/// It used to take every feature, so an item a project had deliberately parked off its board —
/// `Deferred` — reappeared here as "Not started", and anything in a no-op disposition (`Out-of-
/// Scope`, `No Action`) was counted as "Done", which reads as delivered.
fn on_board(project: &Project, status: &str) -> bool {
    let shown = &project.config.displayed_states;
    shown.is_empty() || shown.iter().any(|s| s == status)
}

/// Classify a feature into a normalized cross-project lane.
fn disposition(project: &Project, f: &crate::FeatureItem) -> Disposition {
    if is_terminal(project, &f.status) {
        Disposition::Done
    } else if f.task_count() > 0 {
        Disposition::InProgress
    } else {
        Disposition::NotStarted
    }
}

// ---- rollups -------------------------------------------------------------------------------

/// Feature-completion counts + a derived percentage. Completion is **status-aware**: a feature in a
/// terminal status (Completed / no-op) counts as fully done even if it tracks no tasks (many
/// features are completed by a status move, not by checking off tasks); a non-terminal feature with
/// tasks gets partial credit (`done/total`); otherwise it's 0. `percent` is the mean per-feature
/// completion across the group (rounded), or `0` when there are no features. `tasks_*` are retained
/// for display (how many checklist items exist/are done) but no longer drive the percentage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    pub features: usize,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub percent: u32,
    /// Sum of per-feature completion fractions (0.0–1.0 each); `percent = progress/features*100`.
    #[serde(skip)]
    progress: f64,
}

impl Counts {
    fn finalize(mut self) -> Self {
        self.percent = if self.features == 0 {
            0
        } else {
            ((self.progress / self.features as f64) * 100.0).round() as u32
        };
        self
    }
    fn add(&mut self, other: &Counts) {
        self.features += other.features;
        self.tasks_total += other.tasks_total;
        self.tasks_done += other.tasks_done;
        self.progress += other.progress;
    }
    /// Fold one feature into the counts: full credit if terminal, else task progress, else 0.
    fn add_feature(&mut self, project: &Project, f: &crate::FeatureItem) {
        self.features += 1;
        self.tasks_total += f.task_count();
        self.tasks_done += f.done_count();
        self.progress += if is_terminal(project, &f.status) {
            1.0
        } else if f.task_count() > 0 {
            f.done_count() as f64 / f.task_count() as f64
        } else {
            0.0
        };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneRollup {
    pub code: String,
    pub name: String,
    pub counts: Counts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRollup {
    pub id: String,
    pub name: String,
    pub counts: Counts,
    pub milestones: Vec<MilestoneRollup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramRollup {
    pub id: String,
    pub name: String,
    pub counts: Counts,
    pub projects: Vec<ProjectRollup>,
}

/// The full rollup tree: milestone% → project% → program% → portfolio%.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollupReport {
    pub portfolio: String,
    pub counts: Counts,
    pub programs: Vec<ProgramRollup>,
}

/// Compute completion rollups across the portfolio. The percentage is the mean per-feature
/// completion (status-aware: a terminal feature is 100% even with no tasks; a non-terminal feature
/// with tasks gets `done/total`), rolled up milestone → project → program → portfolio. A project
/// listed in a program but missing from the store is skipped (it contributes nothing).
pub fn rollups(store: &Store) -> Result<RollupReport> {
    let ws = load(store)?;
    let programs = resolve_programs(store, &ws)?;

    let mut report = RollupReport {
        portfolio: if ws.name.is_empty() {
            "Portfolio".to_string()
        } else {
            ws.name.clone()
        },
        counts: Counts::default(),
        programs: Vec::new(),
    };

    for prog in &programs {
        let mut prog_counts = Counts::default();
        let mut project_rollups = Vec::new();
        for pid in &prog.projects {
            let Ok(project) = store.load(pid) else {
                continue;
            };
            let pr = project_rollup(&project);
            prog_counts.add(&pr.counts);
            project_rollups.push(pr);
        }
        let prog_counts = prog_counts.finalize();
        report.counts.add(&prog_counts);
        report.programs.push(ProgramRollup {
            id: prog.id.clone(),
            name: prog.name.clone(),
            counts: prog_counts,
            projects: project_rollups,
        });
    }
    report.counts = report.counts.finalize();
    Ok(report)
}

fn project_rollup(project: &Project) -> ProjectRollup {
    let mut milestones = Vec::new();
    let mut proj_counts = Counts::default();

    // Per-milestone counts (a feature contributes to its declared milestone).
    for ms in &project.milestones {
        let mut c = Counts::default();
        for f in project.features.iter().filter(|f| f.milestone == ms.code) {
            c.add_feature(project, f);
        }
        let c = c.finalize();
        proj_counts.add(&c);
        milestones.push(MilestoneRollup {
            code: ms.code.clone(),
            name: ms.name.clone(),
            counts: c,
        });
    }
    // Features whose milestone doesn't resolve still count toward the project total.
    let ms_codes: Vec<&str> = project.milestones.iter().map(|m| m.code.as_str()).collect();
    for f in project
        .features
        .iter()
        .filter(|f| !ms_codes.contains(&f.milestone.as_str()))
    {
        proj_counts.add_feature(project, f);
    }

    let name = if project.config.name.is_empty() {
        project.id.clone()
    } else {
        project.config.name.clone()
    };
    ProjectRollup {
        id: project.id.clone(),
        name,
        counts: proj_counts.finalize(),
        milestones,
    }
}

// ---- cross-project board -------------------------------------------------------------------

/// A single feature as it appears on the cross-project board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardCard {
    pub project: String,
    pub code: String,
    pub title: String,
    /// The feature's native (per-project) status — kept for display, since lanes are normalized.
    pub status: String,
    pub milestone: String,
    pub assignee: Option<String>,
    pub team: Option<String>,
    pub tasks_done: usize,
    pub tasks_total: usize,
    pub disposition: Disposition,
}

/// One normalized lane plus its cards (across all projects).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardLane {
    pub disposition: Disposition,
    pub cards: Vec<BoardCard>,
}

/// The cross-project board: every feature in the portfolio grouped into the normalized lanes
/// {not-started, in-progress, done}. Robust to differing per-project workflows because grouping is
/// derived from each project's own config (Completed-name / no-op) + task activity, not a shared
/// status vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardReport {
    pub lanes: Vec<BoardLane>,
}

pub fn cross_project_board(store: &Store) -> Result<BoardReport> {
    let ws = load(store)?;
    let programs = resolve_programs(store, &ws)?;

    // De-duplicate project ids (a project could appear in more than one program).
    let mut seen = std::collections::BTreeSet::new();
    let mut not_started = Vec::new();
    let mut in_progress = Vec::new();
    let mut done = Vec::new();

    for prog in &programs {
        for pid in &prog.projects {
            if !seen.insert(pid.clone()) {
                continue;
            }
            let Ok(project) = store.load(pid) else {
                continue;
            };
            for f in project
                .features
                .iter()
                .filter(|f| on_board(&project, &f.status))
            {
                let disp = disposition(&project, f);
                let card = BoardCard {
                    project: project.id.clone(),
                    code: f.code.clone(),
                    title: f.title.clone(),
                    status: f.status.clone(),
                    milestone: f.milestone.clone(),
                    assignee: f.assignee.clone(),
                    team: f.team.clone(),
                    tasks_done: f.done_count(),
                    tasks_total: f.task_count(),
                    disposition: disp,
                };
                match disp {
                    Disposition::NotStarted => not_started.push(card),
                    Disposition::InProgress => in_progress.push(card),
                    Disposition::Done => done.push(card),
                }
            }
        }
    }

    Ok(BoardReport {
        lanes: vec![
            BoardLane {
                disposition: Disposition::NotStarted,
                cards: not_started,
            },
            BoardLane {
                disposition: Disposition::InProgress,
                cards: in_progress,
            },
            BoardLane {
                disposition: Disposition::Done,
                cards: done,
            },
        ],
    })
}

// ---- program resolution --------------------------------------------------------------------

/// A resolved program: real programs from `workspace.yaml`, or a single implicit default program
/// over every store project when none are declared. Used by both rollups and the board.
struct ResolvedProgram {
    id: String,
    name: String,
    projects: Vec<String>,
}

fn resolve_programs(store: &Store, ws: &Workspace) -> Result<Vec<ResolvedProgram>> {
    if ws.programs.is_empty() {
        // Implicit default: one program containing every listable project.
        return Ok(vec![ResolvedProgram {
            id: DEFAULT_PROGRAM_ID.to_string(),
            name: "Default".to_string(),
            projects: store.list_projects()?,
        }]);
    }
    Ok(ws
        .programs
        .iter()
        .map(|p| ResolvedProgram {
            id: p.id.clone(),
            name: if p.name.is_empty() {
                p.id.clone()
            } else {
                p.name.clone()
            },
            projects: p.projects.clone(),
        })
        .collect())
}

/// The portfolio index as a serializable view: the workspace metadata + programs, with the implicit
/// default program materialized when none are declared (so callers always see a consistent shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioView {
    pub name: String,
    pub description: String,
    pub programs: Vec<ProgramView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub projects: Vec<String>,
    /// True when this is the synthesized implicit default program (no workspace.yaml programs).
    pub implicit: bool,
}

/// Build the portfolio index view (workspace + programs), materializing the implicit default
/// program over every project when `workspace.yaml` declares none.
pub fn view(store: &Store) -> Result<PortfolioView> {
    let ws = load(store)?;
    let implicit = ws.programs.is_empty();
    let programs = if implicit {
        vec![ProgramView {
            id: DEFAULT_PROGRAM_ID.to_string(),
            name: "Default".to_string(),
            description: "All projects (no workspace.yaml programs declared).".to_string(),
            projects: store.list_projects()?,
            implicit: true,
        }]
    } else {
        ws.programs
            .iter()
            .map(|p| ProgramView {
                id: p.id.clone(),
                name: if p.name.is_empty() {
                    p.id.clone()
                } else {
                    p.name.clone()
                },
                description: p.description.clone(),
                projects: p.projects.clone(),
                implicit: false,
            })
            .collect()
    };
    Ok(PortfolioView {
        name: if ws.name.is_empty() {
            "Portfolio".to_string()
        } else {
            ws.name.clone()
        },
        description: ws.description.clone(),
        programs,
    })
}

/// A per-program/per-project tally rolled into a `BTreeMap` keyed by disposition string — a small
/// helper exposed for callers/tests that want lane totals without the full card list.
pub fn lane_totals(board: &BoardReport) -> BTreeMap<String, usize> {
    board
        .lanes
        .iter()
        .map(|l| (l.disposition.as_str().to_string(), l.cards.len()))
        .collect()
}
