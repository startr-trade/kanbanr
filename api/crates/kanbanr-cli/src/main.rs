//! kanbanr CLI — driven by the Claude skill. It talks to a kanbanr **server** (the default), or in
//! **local mode** operates on the data folder directly via `kanbanr-core` with no server running
//! (auto-detected when no server is configured but a data dir is present; forced with `--local`).

mod backend;
mod hooks;
mod mirror;
mod scm;
mod self_update;

use backend::{Backend, Method};
use clap::{Args, Parser, Subcommand};
use kanbanr_core::docs::DocFolder;
use kanbanr_core::project::{DataDirSource, Marker};
use kanbanr_core::{Project, project};
use serde_json::{Map, Value, json};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What `--version` prints: the release, and the build that release was made from (FEAT-088).
///
/// The tag alone cannot distinguish two builds of the same release, which is precisely what happens
/// when an asset is re-uploaded under one tag. `self-update` answers that by checksum; a person
/// reading `--version` answers it by this.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("KANBANR_GIT_SHA"),
    ", built ",
    env!("KANBANR_BUILD_DATE"),
    ")"
);

#[derive(Parser)]
#[command(
    name = "kanbanr",
    version = VERSION,
    about = "Kanban task manager for Claude development (HTTP client)"
)]
struct Cli {
    /// Project to operate on (default: $KANBANR_PROJECT, .kanbanr marker, or cwd name).
    #[arg(long, global = true)]
    project: Option<String>,
    /// Data folder (default: $KANBANR_DATA_DIR, the nearest .kanbanr marker's data_dir, or ./data).
    #[arg(long, global = true)]
    data_dir: Option<String>,
    /// Emit machine-readable JSON instead of human text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the commit identity (name + email) of this data folder.
    Whoami,
    /// One-shot setup: create the local data dir + git repo, set the commit identity, scaffold a
    /// first project, and select it here. Gets you to a working board in one command.
    ///
    /// Without --data-dir it asks where to keep the board (in a terminal), recommending a folder
    /// next to the project's git repo named `<repo>.kanbanr`; non-interactively it uses that
    /// recommendation. The choice is recorded in the `.kanbanr` marker.
    Init {
        /// Project name to scaffold (default: the current directory's name).
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Commit identity name (e.g. "Ada Lovelace").
        #[arg(long)]
        author: Option<String>,
        /// Commit identity email.
        #[arg(long)]
        email: Option<String>,
        /// Don't register kanbanr's Claude Code hooks (they're added to the global Claude Code
        /// settings once, and act only in kanbanr-tracked folders).
        #[arg(long)]
        no_hooks: bool,
        /// Repoint a folder that already names a different board. Without it, init refuses rather
        /// than overwriting the only pointer this project has to its board (FEAT-085).
        #[arg(long)]
        force: bool,
    },
    /// Replace this binary with the one the release publishes.
    ///
    /// Notices two different things: a newer version, and the SAME version rebuilt after this copy
    /// was installed — the second is invisible to a version comparison and is how an interim fix
    /// ships. Never runs on its own; every download is checksum-verified before anything is
    /// replaced. (FEAT-088)
    SelfUpdate {
        /// Report whether an update is available and exit — download nothing, change nothing.
        #[arg(long)]
        check: bool,
        /// Install this tag instead of the newest release. Also how you roll back.
        #[arg(long = "version", value_name = "TAG")]
        tag: Option<String>,
    },
    /// Set the commit identity (name + email) on this data repo.
    Identity {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
    },
    /// Run the view daemon over this data folder. Read-only by default (a live web monitor); pass
    /// `--allow-writes` to make it a single-writer daemon that also accepts mutations. (FEAT-034)
    Serve {
        /// Address to bind (default: $KANBANR_BIND or 127.0.0.1:8080).
        #[arg(long)]
        bind: Option<String>,
        /// Built SPA directory to serve (default: $KANBANR_UI_DIR; omit to serve /api only).
        #[arg(long)]
        ui_dir: Option<String>,
        /// Opt-in: expose write routes and serialize all mutations through this one process
        /// (single-writer daemon). OFF by default — the default daemon only reads. (FEAT-034)
        #[arg(long)]
        allow_writes: bool,
    },
    /// Push local commits to the configured git remotes now (FEAT-034). Writes commit locally and
    /// push is debounced off the hot path by default; this flushes anything pending immediately.
    Sync,
    /// Open the live monitor in your browser (warns if it isn't running).
    Open,
    /// Print the data folder (board) this directory uses. With --json, also where that came from,
    /// the recommended `<repo>.kanbanr` folder, and existing kanbanr folders next to the project.
    Where,
    /// Show recent activity for the project (from its changelog).
    Activity,
    /// Show / test the notification events log for the project (FEAT-036). Events are emitted
    /// best-effort on state changes (feature added/moved/completed, dependents becoming ready).
    #[command(subcommand)]
    Events(EventsCmd),
    /// Manage git remotes for the data repo (sharing/centralization is via the remote).
    #[command(subcommand)]
    Remote(RemoteCmd),
    /// Create / configure / list projects.
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Manage feature items.
    #[command(subcommand)]
    Feature(FeatureCmd),
    /// Move a feature to a new status (validated against the workflow).
    Move {
        code: String,
        status: String,
        /// Pass a status's gate that is not met, recording why (FEAT-113). The reason is stored in
        /// the item's history — a bypass that leaves a trace beats one that is silent.
        /// `--unapproved` is the older name.
        #[arg(long = "override", alias = "unapproved", value_name = "REASON")]
        unapproved: Option<String>,
    },
    /// Move one test along the TDD lifecycle: planned | red | green. Flip it when the test
    /// actually runs, not when you intend to write it. (FEAT-051)
    Test {
        code: String,
        requirement: String,
        test: String,
        state: String,
        /// The project revision the result was observed at; a green older than HEAD is stale.
        #[arg(long)]
        rev: Option<String>,
    },
    /// What hangs off a goal, an item or a requirement — and what is missing from it. (FEAT-057)
    Trace {
        /// `G-2`, `FEAT-046`, `FEAT-046/R-2`, or `R-2` when the branch says which item.
        subject: Option<String>,
        /// Render the derived Zachman view for a milestone or item instead.
        #[arg(long)]
        zachman: bool,
    },
    /// Why does this code exist? Answers from the annotation on the line, else the trailer of the
    /// commit that wrote it, printing requirement → goal → purpose. (FEAT-057)
    Why {
        /// `src/thing.rs` or `src/thing.rs:42`.
        target: String,
    },
    /// Put the board's reasoning in front of the agent working on this project (FEAT-065):
    /// writes a generated, regenerable block into the project's CLAUDE.md.
    #[command(subcommand)]
    Claude(ClaudeCmd),
    /// Architecture decisions: documents that join the graph. (FEAT-057)
    #[command(subcommand)]
    Adr(AdrCmd),
    /// Record a lesson, or judge one that is already recorded (FEAT-055).
    #[command(subcommand)]
    Lesson(LessonCmd),
    /// The lessons worth reading right now, most believed first. Read these BEFORE starting work.
    Lessons {
        /// Only those that bear on this item (its labels, kind and goals).
        #[arg(long = "for")]
        for_item: Option<String>,
        /// Include retired ones — kept as a record, normally not surfaced.
        #[arg(long)]
        all: bool,
    },
    /// How a wave actually went (FEAT-054): scope growth by recorded cause, defects and escapes,
    /// cycle time, rework, evidence at completion, estimate against actual. Facts only — the
    /// narrative is yours to write, and `--write` leaves a section for it.
    Retro {
        /// A milestone code. Omit to use `--since` / `--label`, or neither for the whole board.
        milestone: Option<String>,
        /// Window, e.g. `14d` or an RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        label: Option<String>,
        /// One sprint's retro: its items (including those it carried out) and its burndown.
        #[arg(long)]
        sprint: Option<String>,
        /// Store the facts as a document in the board's retros/ folder.
        #[arg(long)]
        write: bool,
        /// List finished milestones whose retro has not been written.
        #[arg(long)]
        due: bool,
    },
    /// Sprints: timeboxes with a goal, a capacity and a burndown (FEAT-119). Only for projects that
    /// switch them on: `kanbanr config cadence --sprints on`.
    #[command(subcommand)]
    Sprint(SprintCmd),
    /// Releases: planned up front, cut from finished work (FEAT-120). Only for projects that switch
    /// them on: `kanbanr config cadence --releases on`.
    #[command(subcommand)]
    Release(ReleaseCmd),
    /// Record that an item was sliced out of another, so a wave's growth can be accounted for.
    SplitFrom {
        code: String,
        /// The item it came from; omit to clear.
        parent: Option<String>,
    },
    /// Record what a defect cost and where it came from (FEAT-053). Whether it *escaped* is
    /// derived — it escaped if the work that introduced it had already been called done.
    Defect {
        code: String,
        /// low | medium | high | critical.
        #[arg(long)]
        severity: Option<String>,
        /// The item whose work introduced it — a fix that caused this points at that fix.
        #[arg(long)]
        introduced_by: Option<String>,
        /// Where it was found: a status, an environment, or "production".
        #[arg(long)]
        found_in: Option<String>,
        #[arg(long)]
        root_cause: Option<String>,
        /// Found after the work was called done. Derived from `--introduced-by` when omitted.
        #[arg(long)]
        escaped: bool,
        /// The commit or test that proves it is fixed.
        #[arg(long)]
        fixed_by: Option<String>,
        /// Remove the defect record.
        #[arg(long)]
        clear: bool,
    },
    /// Flow and quality, derived from what the board already records: throughput, cycle time,
    /// rework, defect escape rate and requirement coverage. Nothing here is self-reported.
    Report {
        /// Window, e.g. `14d` or an RFC3339 timestamp. Items are counted by when they finished.
        #[arg(long)]
        since: Option<String>,
    },
    /// Record test results from a real run (FEAT-053). Reads a Claude Code PostToolUse payload on
    /// stdin, finds the test names the board is tracking, and flips their state to match what
    /// actually happened — so evidence is measured, never claimed.
    Capture,
    /// Is this item actually ready? Reports what it has not said and what it cannot yet show:
    /// missing dimensions, requirements with no test, unproven requirements, approval state.
    Check {
        /// One item; omit for every item currently in scope.
        code: Option<String>,
        /// Check a definition in a file instead of on the board — what CI runs on a contribution
        /// from someone who has no board access. `-` reads stdin.
        #[arg(long)]
        file: Option<String>,
    },
    /// Print the one-screen decision brief for an item: what is proposed, why, and how it will be
    /// verified. Read this BEFORE the work, not after. (FEAT-048)
    Review {
        /// One item; omit with `--pending` for every item whose definition is not yet agreed.
        code: Option<String>,
        /// Every item awaiting agreement, in one pass. Approval has to be cheap or it becomes
        /// theatre, and eleven invocations is not cheap.
        #[arg(long)]
        pending: bool,
        /// Review in the browser instead: starts the monitor with writes enabled and opens the
        /// review page, where the brief is rendered and the approve button is beside it.
        #[arg(long)]
        ui: bool,
    },
    /// Agree to an item that was built under a recorded `--unapproved` start (FEAT-080).
    ///
    /// Recorded as its own verdict, not as an ordinary approval: the gate exists to tell "we
    /// agreed, then built" from "we built, then agreed", and one verdict cannot. Only for items
    /// that actually took the bypass — it is not a shortcut around review.
    Ratify {
        code: String,
        /// Anything worth saying about agreeing after the fact.
        #[arg(long, default_value = "")]
        reason: String,
        /// Who is agreeing (default: this data folder's commit identity).
        #[arg(long)]
        by: Option<String>,
    },
    /// Take back an approval (FEAT-069). The agreement goes; the record of having given it stays,
    /// and the item returns to what is awaiting review.
    Unapprove {
        code: String,
        /// Why it is being withdrawn. An agreement needs no reason; taking one back does.
        #[arg(long)]
        reason: String,
        /// Who is withdrawing it (default: this data folder's commit identity).
        #[arg(long)]
        by: Option<String>,
    },
    /// Record agreement to an item's definition as it currently stands. Editing the definition
    /// afterwards lapses the approval. (FEAT-048)
    Approve {
        code: String,
        /// Who is approving (defaults to the data repo's commit identity).
        #[arg(long)]
        by: Option<String>,
    },
    /// Record a named sign-off a stage can require — "design-review", "release" (FEAT-114). It covers
    /// the definition as it stands: change the definition and the sign-off lapses.
    Signoff {
        code: String,
        /// The sign-off's name, as the workflow's gate asks for it.
        name: String,
        /// What was agreed, or where it was agreed.
        #[arg(long)]
        note: Option<String>,
        /// A board doc holding the record (minutes, a checklist).
        #[arg(long)]
        doc: Option<String>,
        /// Who is signing off (defaults to the data repo's commit identity).
        #[arg(long)]
        by: Option<String>,
    },
    /// Manage a feature's persistent todo-lists (an epic can hold many).
    #[command(subcommand)]
    Todo(TodoCmd),
    /// Manage tasks within a feature's todo-list.
    #[command(subcommand)]
    Task(TaskCmd),
    /// Manage milestones.
    #[command(subcommand)]
    Milestone(MilestoneCmd),
    /// View / edit the project workflow configuration.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Manage project documentation (a tree of markdown files under docs/).
    #[command(subcommand)]
    Doc(DocCmd),
    /// Apply a bundle of operations in ONE call (new/edited FIs, moves, todo-lists + items,
    /// task states, doc changes). Reads a JSON bundle from --file or stdin.
    Batch {
        /// Path to a JSON file; if omitted, reads the bundle from stdin.
        #[arg(long)]
        file: Option<String>,
        /// Commit message for this bundle (server frames one if omitted).
        #[arg(long)]
        message: Option<String>,
        /// Validate the bundle and report what it would create or skip, writing nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// The project charter: why this project exists, its goals, stakeholders and non-goals.
    /// Work items link goals by id, so the board can show what each item serves. (FEAT-046)
    #[command(subcommand)]
    Charter(CharterCmd),
    /// Register kanbanr's Claude Code hooks (SessionStart: recover the board; Stop: nudge to record
    /// work) in the global Claude Code settings. `kanbanr init` does this automatically.
    #[command(subcommand)]
    Hooks(HooksCmd),
    /// Mirror this project's features to GitHub issues via `gh` (kanbanr → GitHub, one way).
    #[command(subcommand)]
    Mirror(MirrorCmd),
    /// Tie the code repo to the board: hooks that require a reference, and the checks they run.
    #[command(subcommand)]
    Git(GitCmd),
    /// Begin work on an item: branch for it, and move it into an active status. The branch is how
    /// everything else knows what you are working on. (FEAT-056)
    Start {
        code: String,
        /// The status to move it to (default: the first active status on the board).
        #[arg(long)]
        to: Option<String>,
        /// Move the item without creating or switching a branch.
        #[arg(long)]
        no_branch: bool,
        /// Start past a gate that is not met, recording why. The reason stays on the item.
        /// `--unapproved` is the older name.
        #[arg(long = "override", alias = "unapproved", value_name = "REASON")]
        unapproved: Option<String>,
    },
    /// Finish an item: refuse while tasks are open or requirements are unproven, then move it to
    /// a terminal status. Merging stays yours. (FEAT-056)
    Finish {
        /// Defaults to the item this branch belongs to.
        code: Option<String>,
    },
    /// Commit with the board reference filled in from the current branch. (FEAT-056)
    Commit {
        #[arg(short, long)]
        message: String,
        /// Stage every tracked change first (`git commit -a`).
        #[arg(short, long)]
        all: bool,
        /// Reference something more precise than the item: `R-2`, `TL-001/T3`, or another item's
        /// code. Repeatable.
        #[arg(long = "ref")]
        refs: Vec<String>,
    },
    /// List the tests the board is tracking and check each name still exists in the project (run
    /// it in the project folder). `--write` returns a green whose test has vanished to `planned`,
    /// because evidence from a test nobody can run is not evidence. (FEAT-053)
    Tests {
        #[arg(long)]
        write: bool,
    },
    /// List imported features' sources and check whether file sources still exist in the project
    /// (run it in the project folder). `--write` records `missing_since` on sources that are gone
    /// (and clears it when they're back), so the monitor can label them. (FEAT-042)
    Sources {
        #[arg(long)]
        write: bool,
    },
    /// Export a feature item as markdown or JSON (Claude-ready).
    Export {
        code: String,
        #[arg(long, default_value = "md")]
        format: String,
    },
    /// Print a text kanban board for the project.
    Board,
    /// List features that are ready to work (all dependencies terminal). (FEAT-027)
    Ready {
        /// Span every project (portfolio-wide) instead of just the current one.
        #[arg(long)]
        all_projects: bool,
    },
    /// List features that are blocked (a dependency is not yet terminal). (FEAT-027)
    Blocked {
        /// Span every project (portfolio-wide) instead of just the current one.
        #[arg(long)]
        all_projects: bool,
    },
    /// Print the dependency graph as Graphviz DOT or JSON. (FEAT-027)
    Graph {
        /// Output format: `json` (default) or `dot`.
        #[arg(long, default_value = "json")]
        format: String,
        /// Span every project (portfolio-wide) instead of just the current one.
        #[arg(long)]
        all_projects: bool,
    },
    /// List features downstream of (impacted by) a feature — its transitive dependents. (FEAT-027)
    Impact { code: String },
    /// Search features with rich filters + full-text, in one project or across all. (FEAT-032)
    Query(QueryArgs),
    /// Print a Mermaid `gantt` diagram of the schedule (dates/sequencing + critical path). (FEAT-035)
    Gantt {
        /// Span every project (portfolio-wide, sectioned by project) instead of just the current one.
        #[arg(long)]
        all_projects: bool,
    },
    /// Print the critical path (longest dependency chain) and per-task schedule offsets. (FEAT-035)
    CriticalPath {
        /// Span every project (portfolio-wide) instead of just the current one.
        #[arg(long)]
        all_projects: bool,
    },
    /// Scan for integrity problems (dangling deps, unknown milestones, outdated schema). (FEAT-037)
    /// Defaults to the current/selected project; pass --all-projects for the whole portfolio.
    Doctor {
        /// Scan every project in the portfolio.
        #[arg(long)]
        all_projects: bool,
    },
    /// Rebuild the per-project index cache (index.yaml) from the source-of-truth files. (FEAT-033)
    /// A maintenance command; defaults to the current/selected project, --all-projects for all.
    Index {
        /// Rebuild the index for every project in the portfolio.
        #[arg(long)]
        all_projects: bool,
    },
    /// Portfolio / program hierarchy: cross-project rollups & board. (FEAT-030)
    #[command(subcommand)]
    Portfolio(PortfolioCmd),
}

#[derive(Subcommand)]
enum PortfolioCmd {
    /// Show the portfolio index (workspace metadata + programs and their projects).
    Show,
    /// Show task-based rollups: milestone% → project% → program% → portfolio%.
    Rollups,
    /// Show the cross-project board (features grouped into normalized lanes).
    Board,
    /// Declare/replace a program in workspace.yaml (a write).
    AddProgram {
        /// Program id.
        id: String,
        /// Display name (defaults to the id).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Comma-separated project ids belonging to this program.
        #[arg(long)]
        projects: Option<String>,
    },
}

#[derive(Subcommand)]
enum EventsCmd {
    /// List recent notification events for the project (newest first).
    List {
        /// Maximum number of events to show (default 25).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Emit a sample event to verify the log + any configured webhook delivery (FEAT-036). Writes a
    /// test event to the project's events log and POSTs it to every configured webhook.
    Test,
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// Add a git remote (commits are pulled+pushed to it after each write).
    Add { name: String, url: String },
    /// List configured remotes.
    List,
    /// Remove a git remote.
    Remove { name: String },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Create and configure a project.
    Init {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        statuses: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        displayed_states: Option<Vec<String>>,
        #[arg(long)]
        default_state: Option<String>,
        /// Statuses that are functionally inert (no-op) dispositions; always non-displayed.
        #[arg(long, value_delimiter = ',')]
        no_op_states: Option<Vec<String>>,
        /// Use a named workflow preset instead of the default (FEAT-116): scheduled, togaf, pdca or
        /// design-control. `kanbanr config workflow --preset list` describes them.
        #[arg(long)]
        workflow: Option<String>,
    },
    /// Update a project's display name / description.
    Edit {
        name: String,
        #[arg(long = "name")]
        new_name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// List all projects.
    List,
    /// Delete a project (only allowed when it has no feature items and no milestones).
    Delete { name: String },
    /// Write a .kanbanr marker in the current directory selecting this project.
    Use { name: String },
    /// Export the WHOLE project as a portable markdown bundle (board + milestones + every feature).
    Export {
        #[arg(long, default_value = "md")]
        format: String,
    },
}

#[derive(Subcommand)]
enum FeatureCmd {
    /// Record why an item exists, what must be true, and how it is verified (FEAT-047).
    /// Reads YAML or JSON from --file (or stdin). `--template` prints a skeleton for a kind
    /// instead of writing; `--clear` removes the block.
    Define {
        code: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        clear: bool,
        /// Print a skeleton for this kind and exit (feature | defect | chore | docs | recurring).
        #[arg(long)]
        template: bool,
        /// Which shape the template should take (default: feature, the strictest).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Add a feature item (a milestone is required; code auto-generates if omitted).
    Add {
        #[arg(long)]
        title: String,
        /// The milestone this feature belongs to (required).
        #[arg(long)]
        milestone: String,
        #[arg(long)]
        spec: Option<String>,
        #[arg(long)]
        spec_file: Option<String>,
        #[arg(long)]
        code: Option<String>,
        /// Work kind (feature / chore / bug / refactor / docs / recurring …).
        #[arg(long)]
        kind: Option<String>,
        /// Priority (low / medium / high …).
        #[arg(long)]
        priority: Option<String>,
        /// Planned start date (ISO date, e.g. 2026-07-01) for scheduling/Gantt.
        #[arg(long)]
        start: Option<String>,
        /// Due date (free text, e.g. an ISO date).
        #[arg(long)]
        due: Option<String>,
        /// Estimated effort in days (used as the Gantt/scheduling duration; default 1).
        #[arg(long)]
        estimate: Option<f64>,
        /// Estimated size in story points (FEAT-121; a value <= 0 clears it).
        #[arg(long)]
        points: Option<f64>,
        /// Feedback on a shipped release (FEAT-120): records the version it was found in, the
        /// same field a defect record uses.
        #[arg(long, value_name = "VERSION")]
        found_in: Option<String>,
        /// Assignee (the person/agent owning this feature).
        #[arg(long)]
        assignee: Option<String>,
        /// Owning team.
        #[arg(long)]
        team: Option<String>,
        /// Labels/tags (comma-separated).
        #[arg(long, value_delimiter = ',')]
        labels: Option<Vec<String>>,
        /// Feature codes this one is blocked by (comma-separated; must exist, no cycles).
        #[arg(long, value_delimiter = ',')]
        depends_on: Option<Vec<String>>,
    },
    /// List feature items (optionally filtered).
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
        /// Filter to features with this assignee.
        #[arg(long)]
        assignee: Option<String>,
        /// Filter to features with this team.
        #[arg(long)]
        team: Option<String>,
    },
    /// Show a single feature item (rendered markdown).
    Show { code: String },
    /// Edit a feature item.
    Edit {
        code: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        spec: Option<String>,
        #[arg(long)]
        spec_file: Option<String>,
        #[arg(long = "code")]
        new_code: Option<String>,
        /// Move the feature to a different (existing) milestone.
        #[arg(long)]
        milestone: Option<String>,
        /// Work kind (empty string clears).
        #[arg(long)]
        kind: Option<String>,
        /// Priority (empty string clears).
        #[arg(long)]
        priority: Option<String>,
        /// Planned start date (ISO date; empty string clears).
        #[arg(long)]
        start: Option<String>,
        /// Due date (empty string clears).
        #[arg(long)]
        due: Option<String>,
        /// Estimated effort in days (a value <= 0 clears it).
        #[arg(long)]
        estimate: Option<f64>,
        /// Estimated size in story points (FEAT-121; a value <= 0 clears it).
        #[arg(long)]
        points: Option<f64>,
        /// Assignee (empty string clears).
        #[arg(long)]
        assignee: Option<String>,
        /// Owning team (empty string clears).
        #[arg(long)]
        team: Option<String>,
        /// Replace labels (comma-separated).
        #[arg(long, value_delimiter = ',')]
        labels: Option<Vec<String>>,
        /// Replace blocked-by feature codes (comma-separated; must exist, no cycles).
        #[arg(long, value_delimiter = ',')]
        depends_on: Option<Vec<String>>,
    },
    // (Feature items are permanent — there is intentionally no `feature delete`.)
}

#[derive(Subcommand)]
enum TodoCmd {
    /// Add a todo-list to a feature (code auto-generates as TL-001, …).
    Add {
        feature: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        code: Option<String>,
    },
    /// List a feature's todo-lists (newest first).
    List { feature: String },
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Add a task to a feature's todo-list.
    Add {
        feature: String,
        todo: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        key: Option<String>,
    },
    /// Set a task's state within a todo-list.
    State {
        feature: String,
        todo: String,
        key: String,
        state: String,
    },
    /// List tasks (optionally for a single todo-list).
    List {
        feature: String,
        todo: Option<String>,
    },
}

#[derive(Subcommand)]
enum MilestoneCmd {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Option<Vec<String>>,
        #[arg(long)]
        code: Option<String>,
    },
    List,
    Edit {
        code: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Option<Vec<String>>,
    },
    Delete {
        code: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    Show,
    /// Whether this project works in sprints and releases, and what it estimates in (FEAT-121).
    /// Both are off unless switched on — most projects follow a different rhythm.
    Cadence {
        /// Use sprints (and burndown and velocity): on | off.
        #[arg(long, value_parser = ["on", "off"])]
        sprints: Option<String>,
        /// Use releases: on | off.
        #[arg(long, value_parser = ["on", "off"])]
        releases: Option<String>,
        /// Estimate in `days` or `points`.
        #[arg(long, value_parser = ["days", "points"])]
        unit: Option<String>,
        /// The sprint length `sprint add` uses by default, in days.
        #[arg(long)]
        sprint_length: Option<u32>,
        /// How often a release is cut: per_sprint | every_n | on_demand.
        #[arg(long)]
        release: Option<String>,
    },
    SetTransition(SetTransitionArgs),
    DisplayedStates {
        #[arg(value_delimiter = ',')]
        states: Vec<String>,
    },
    DefaultState {
        state: String,
    },
    /// Rename a status everywhere (config + transitions) and migrate the feature items in it.
    RenameStatus {
        from: String,
        to: String,
    },
    /// Set which statuses are functionally inert (no-op) dispositions (always non-displayed).
    NoOpStates {
        #[arg(value_delimiter = ',')]
        states: Vec<String>,
    },
    /// Reset/redefine the whole workflow at once: statuses, transitions, default/displayed/no-op states.
    Workflow {
        /// Start from a named process (FEAT-116): default, scheduled, togaf, pdca, design-control.
        /// Its statuses, transitions and gates replace the current ones; any flags below then
        /// override them. `--preset list` shows what each one is.
        #[arg(long, value_name = "NAME")]
        preset: Option<String>,
        /// Load a whole workflow — statuses, transitions and gates — from a YAML file, such as an
        /// organisation's own process. `--export` writes one to start from.
        #[arg(long, value_name = "FILE")]
        from_file: Option<String>,
        /// Print this project's workflow, gates included, as a file `--from-file` can load (a read).
        #[arg(long)]
        export: bool,
        /// Print the working agreement — each stage and what entering it asks — generated from the
        /// gates (a read). Under Scrum that is the Definition of Ready and of Done.
        #[arg(long)]
        agreement: bool,
        /// Write the working agreement to the board as `process/working-agreement.md`.
        #[arg(long)]
        write_agreement: bool,
        /// Same as `--preset default`.
        #[arg(long)]
        defaults: bool,
        /// Same as `--preset togaf`.
        #[arg(long)]
        togaf: bool,
        #[arg(long, value_delimiter = ',')]
        statuses: Option<Vec<String>>,
        /// Allowed transitions as `From>To` pairs, e.g. --transitions "Planned>Scheduled,Scheduled>Done".
        #[arg(long, value_delimiter = ',')]
        transitions: Option<Vec<String>>,
        #[arg(long)]
        default_state: Option<String>,
        #[arg(long, value_delimiter = ',')]
        displayed_states: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        no_op_states: Option<Vec<String>>,
        /// Explicit terminal (end) states — a feature here is "done". (FEAT-039)
        #[arg(long, value_delimiter = ',')]
        terminal_states: Option<Vec<String>>,
        /// Print the workflow as a Mermaid `stateDiagram-v2` (a read; ignores the other flags).
        #[arg(long)]
        to_mermaid: bool,
        /// Import the workflow from a Mermaid `stateDiagram-v2` file (or `-` for stdin); replaces
        /// statuses/transitions/default/terminal from the diagram.
        #[arg(long, value_name = "FILE")]
        from_mermaid: Option<String>,
    },
}

#[derive(Subcommand)]
enum ReleaseCmd {
    /// Add a planned release.
    Add {
        version: String,
        /// Target date, YYYY-MM-DD.
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List the project's releases.
    List,
    /// Plan items into a release.
    Plan {
        version: String,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Cut a release: ship its finished items, write its notes, carry the rest.
    Cut {
        version: String,
        /// Also tag the code repository with the version.
        #[arg(long)]
        tag: bool,
    },
}

#[derive(Subcommand)]
enum SprintCmd {
    /// Add a sprint.
    Add {
        /// First day, YYYY-MM-DD.
        #[arg(long)]
        start: String,
        /// Length, e.g. `2w` or `10d` (default: the project's cadence, else two weeks).
        #[arg(long)]
        length: Option<String>,
        /// What this sprint is for, in a sentence.
        #[arg(long)]
        goal: Option<String>,
        /// What it can take, in the project's estimate unit.
        #[arg(long)]
        capacity: Option<f64>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List the project's sprints.
    List,
    /// A sprint's goal, dates, days left, committed against done, and burndown (default: the
    /// active one).
    Show { code: Option<String> },
    /// Plan items into a sprint; warns when it goes over capacity.
    Plan {
        code: String,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Make a sprint the active one.
    Start { code: String },
    /// Close a sprint, carrying unfinished items to another sprint or back to the backlog.
    Close {
        code: String,
        /// A sprint code, or `backlog` (the default).
        #[arg(long)]
        carry_to: Option<String>,
    },
}

#[derive(Subcommand)]
enum DocCmd {
    Add {
        path: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        content: Option<String>,
    },
    List,
    Tree,
    Folder {
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    Show {
        path: String,
    },
    Rm {
        path: String,
    },
}

#[derive(Args)]
struct QueryArgs {
    /// Filter to this status.
    #[arg(long)]
    status: Option<String>,
    /// Filter to this milestone.
    #[arg(long)]
    milestone: Option<String>,
    /// Filter to this work kind.
    #[arg(long)]
    kind: Option<String>,
    /// Filter to this priority.
    #[arg(long)]
    priority: Option<String>,
    /// Filter to features carrying ANY of these labels (comma-separated).
    #[arg(long, value_delimiter = ',')]
    label: Option<Vec<String>>,
    /// Filter to this assignee.
    #[arg(long)]
    assignee: Option<String>,
    /// Filter to this team.
    #[arg(long)]
    team: Option<String>,
    /// Due on/before this date (string compare; ISO dates sort lexically).
    #[arg(long)]
    due_before: Option<String>,
    /// Due on/after this date (string compare).
    #[arg(long)]
    due_after: Option<String>,
    /// Only features that are ready (all dependencies terminal).
    #[arg(long)]
    ready: bool,
    /// Only features that are blocked (a dependency is not yet terminal).
    #[arg(long)]
    blocked: bool,
    /// Free-text term matched against the title (and spec body with --full-text).
    #[arg(long)]
    text: Option<String>,
    /// Also match --text against feature specification bodies (reads spec files).
    #[arg(long)]
    full_text: bool,
    /// Only items serving this charter goal id (e.g. G-2) — "what is this work for?".
    #[arg(long)]
    goal: Option<String>,
    /// Only items with this gap: why | test | approval. The troubleshooting filter.
    #[arg(long)]
    gap: Option<String>,
    /// Span every project (portfolio-wide) instead of just the current one.
    #[arg(long)]
    all_projects: bool,
}

#[derive(Subcommand)]
enum CharterCmd {
    /// Print the charter as markdown (or JSON with --json).
    Show,
    /// Replace the charter from a YAML or JSON file (or stdin when --file is omitted).
    /// Goals without an `id` are assigned one; an empty charter removes it.
    Set {
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
enum HooksCmd {
    /// Add the hooks to this project (skipped if already there or provided by the kanbanr plugin).
    Install {
        /// Register them for every project on this machine instead of just this one.
        #[arg(long)]
        global: bool,
    },
    /// Show whether the hooks are registered and their scripts exist.
    Status {
        /// Look at the machine-wide settings instead of this project's.
        #[arg(long)]
        global: bool,
    },
    /// Remove kanbanr's hooks (other hooks are left alone).
    Uninstall {
        /// Remove them from the machine-wide settings instead of this project's.
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand)]
enum ClaudeCmd {
    /// Write (or refresh) the generated block in this project's CLAUDE.md.
    Sync {
        /// Where to write it (default: CLAUDE.md beside the project root).
        #[arg(long)]
        file: Option<PathBuf>,
        /// Print the block instead of writing it.
        #[arg(long)]
        show: bool,
    },
    /// Decide whether a file write belongs on the board (run by the Claude Code hook).
    Guard,
}

#[derive(Subcommand)]
enum AdrCmd {
    /// Scaffold a decision: front-matter plus the five sections it has to answer.
    New {
        title: String,
        /// Items that will rest on it, comma-separated.
        #[arg(long, value_delimiter = ',')]
        affects: Vec<String>,
        /// Requirements that force it (`FEAT-046/R-2`), usually quality ones.
        #[arg(long, value_delimiter = ',')]
        driven_by: Vec<String>,
        /// ISO/IEC 25010 characteristics at stake.
        #[arg(long, value_delimiter = ',')]
        quality: Vec<String>,
        /// Zachman columns it answers.
        #[arg(long, value_delimiter = ',')]
        zachman: Vec<String>,
        /// conceptual | logical | physical.
        #[arg(long)]
        layer: Option<String>,
        /// proposed | accepted | rejected.
        #[arg(long, default_value = "proposed")]
        status: String,
        #[arg(long, value_delimiter = ',')]
        deciders: Vec<String>,
    },
    /// Every decision, with what it affects and whether it still stands.
    List {
        /// Only those bearing on this item.
        #[arg(long = "for")]
        for_item: Option<String>,
    },
    /// Overturn a decision: writes both sides and lists what was resting on the old one.
    Supersede {
        /// The new decision.
        id: String,
        #[arg(long)]
        replaces: String,
    },
    /// Walk the lineage: 0001 → 0003 → 0007.
    History { id: String },
}

#[derive(Subcommand)]
enum LessonCmd {
    /// Record what was learned. Recording one that already exists affirms it instead.
    Add {
        lesson: String,
        /// practice | pitfall | decision.
        #[arg(long, default_value = "pitfall")]
        kind: String,
        /// The item that taught it.
        #[arg(long = "from")]
        from_item: Option<String>,
        /// The retrospective that promoted it.
        #[arg(long)]
        from_retro: Option<String>,
        /// What actually happened — without this it is an opinion.
        #[arg(long)]
        evidence: Option<String>,
        /// Labels this applies to, comma-separated.
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Charter goals it bears on, comma-separated.
        #[arg(long, value_delimiter = ',')]
        goals: Vec<String>,
    },
    /// It held again: raise confidence and reset the clock.
    Affirm {
        id: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// It did not hold: drop confidence further than an affirmation raises it.
    Contradict {
        id: String,
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
enum GitCmd {
    /// Install the commit-msg and pre-commit hooks in this repository.
    InstallHooks {
        /// Keep an existing hook (moved to `<name>.pre-kanbanr`) and chain it first.
        #[arg(long)]
        force: bool,
    },
    /// Remove kanbanr's hooks, restoring anything they replaced.
    UninstallHooks,
    /// Validate a commit message file (run by the commit-msg hook).
    CheckMsg { file: PathBuf },
    /// Check the current branch belongs to an item (run by the pre-commit hook).
    CheckBranch,
    /// What is installed, which branch this is, and which item it belongs to.
    Status,
    /// Check a `git commit` before it is attempted (run by the Claude Code PreToolUse hook).
    /// Reads the tool payload on stdin and answers with a decision and a reason.
    Guard,
}

#[derive(Subcommand)]
enum MirrorCmd {
    /// Turn the mirror on for this project. Requires `gh` logged in; refuses a public repo unless
    /// --allow-public. Features created from now on get issues; `sync --all` backfills.
    Enable {
        /// GitHub repository, `owner/repo`.
        #[arg(long)]
        repo: String,
        /// Allow mirroring to a public repository (specs and notes become public).
        #[arg(long)]
        allow_public: bool,
    },
    /// Turn the mirror off (existing issue links are kept).
    Disable,
    /// Show the mirror settings and what a sync would do (no GitHub calls).
    Status {
        /// Include older open features that don't have an issue yet.
        #[arg(long)]
        all: bool,
    },
    /// Create/update issues for features that are missing or out of date.
    Sync {
        /// Also create issues for open features created before the mirror was enabled.
        #[arg(long)]
        all: bool,
    },
    /// Link a feature to an existing issue (its content is replaced on the next sync).
    Link { code: String, number: u64 },
    /// Show what changed on GitHub since the last sync: edits, state, new comments (read-only).
    Pull { code: String },
}

#[derive(Args)]
struct SetTransitionArgs {
    from: String,
    to: String,
    #[arg(long)]
    allow: bool,
    #[arg(long)]
    deny: bool,
}

fn main() -> ExitCode {
    quiet_on_a_closed_pipe();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `kanbanr … | head` must end the way `git … | head` does: quietly (FEAT-110).
///
/// Rust ignores SIGPIPE, so a write to a pipe whose reader has gone fails with EPIPE, and
/// `println!` turns that into a panic and a stack trace — after a command that had succeeded. In an
/// agent's session the trace reads as the command failing. Restoring the default signal instead
/// would also kill `serve` whenever a browser drops a connection, so only this one panic is
/// recognised, and it becomes the exit status a SIGPIPE would have given (128 + 13).
fn quiet_on_a_closed_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        // "Broken pipe" on Unix; ERROR_NO_DATA (232) is what Windows says for the same thing.
        if message.starts_with("failed printing to stdout")
            && (message.contains("Broken pipe") || message.contains("os error 232"))
        {
            std::process::exit(141);
        }
        default(info);
    }));
}

/// The legacy `./data` path, when that fallback is all that resolved and nothing is there — the
/// one case where opening the board would *create* it. An explicit `--data-dir` or
/// `$KANBANR_DATA_DIR` is a request for that folder, so it is created as before.
fn no_board_here(cli: &Cli) -> Option<PathBuf> {
    let resolved = project::resolve_data_dir_detailed(cli.data_dir.as_deref());
    (resolved.source == project::DataDirSource::Default && !resolved.path.exists())
        .then_some(resolved.path)
}

/// The commands Claude Code and git hooks run on every tool call or commit, in any folder.
fn runs_from_a_hook(command: &Command) -> bool {
    matches!(
        command,
        Command::Capture
            | Command::Git(GitCmd::Guard | GitCmd::CheckMsg { .. } | GitCmd::CheckBranch)
            | Command::Claude(ClaudeCmd::Guard)
    )
}

/// The CLI is local-only: operate on the data folder directly (store + dispatch + git).
fn make_backend(cli: &Cli) -> Backend {
    Backend::new(project::resolve_data_dir(cli.data_dir.as_deref()))
}

/// The monitor URL (`$KANBANR_SERVER_URL` or the localhost default).
fn monitor_url() -> String {
    std::env::var("KANBANR_SERVER_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://localhost:8080".to_string())
}

/// Run the view daemon over this data folder — the single binary's serving mode (no Docker).
/// `allow_writes` opts into the single-writer daemon (write routes exposed); default OFF keeps the
/// historical read-only monitor unchanged. (FEAT-034)
fn run_serve(
    cli: &Cli,
    bind: Option<String>,
    ui_dir: Option<String>,
    allow_writes: bool,
) -> anyhow::Result<()> {
    let dir = project::resolve_data_dir(cli.data_dir.as_deref());
    let bind = bind
        .or_else(|| std::env::var("KANBANR_BIND").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let ui_dir = ui_dir.or_else(|| {
        std::env::var("KANBANR_UI_DIR")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(kanbanr_server::run(dir, bind, ui_dir, allow_writes))
}

/// Set the commit identity on the data repo.
fn run_identity(cli: &Cli, name: &str, email: &str) -> anyhow::Result<()> {
    let dir = project::resolve_data_dir(cli.data_dir.as_deref());
    std::fs::create_dir_all(dir.join("projects"))?;
    kanbanr_core::git::ensure_repo(&dir);
    kanbanr_core::git::set_identity(&dir, name, email).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "local commit identity set: {name} <{email}>  (data dir: {})",
        dir.display()
    );
    Ok(())
}

/// One-shot local setup: data dir + git repo + identity + a first project, selected here.
fn run_init(
    cli: &Cli,
    name: Option<String>,
    description: Option<String>,
    author: Option<String>,
    email: Option<String>,
    no_hooks: bool,
    force: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let (dir, record) = choose_init_data_dir(cli, &cwd)?;
    if record
        && let Some(repo) = project::enclosing_git_worktree(&dir, project::home_dir().as_deref())
    {
        println!(
            "⚠ {} is inside the git repo at {}: the board would be nested in that repo \
                 (gitignore it, or pick a folder outside the repo)",
            dir.display(),
            repo.display()
        );
    }
    std::fs::create_dir_all(dir.join("projects"))?;
    kanbanr_core::git::ensure_repo(&dir);
    if record {
        println!("✓ board: {}", project::normalize(&dir).display());
    }

    if let (Some(n), Some(e)) = (&author, &email) {
        kanbanr_core::git::set_identity(&dir, n, e).map_err(|e| anyhow::anyhow!(e))?;
        println!("✓ commit identity: {n} <{e}>");
    } else if kanbanr_core::git::identity(&dir)
        .map(|(n, _)| n == "kanbanr")
        .unwrap_or(true)
    {
        println!(
            "• tip: set your commit identity with `kanbanr identity --name \"You\" --email you@example.com`"
        );
    }

    let proj = name
        .or_else(|| project::resolve_project(None))
        .unwrap_or_else(|| "myproject".to_string());
    let backend = Backend::new(dir.clone());
    let body = obj(vec![
        ("name", Some(json!(proj))),
        ("description", description.map(|d| json!(d))),
    ]);
    match backend.write(Method::Post, "/projects", Some(body)) {
        Ok(_) => println!("✓ created project '{proj}'"),
        Err(e) if e.to_string().contains("already exists") => {
            println!("• project '{proj}' already exists — using it")
        }
        Err(e) => return Err(e),
    }
    // The marker is the ONLY pointer from this folder to its board, so it is data, not scratch
    // state (FEAT-085). Overwriting it makes a full board read as empty — the failure that cost
    // this repository its own pointer, recoverable only because FEAT-073 had made the file tracked.
    let wanted = Marker {
        project: Some(proj.clone()),
        data_dir: record.then(|| marker_path(&dir, &cwd)),
    };
    let existing = std::fs::read_to_string(cwd.join(project::MARKER_FILE))
        .ok()
        .map(|c| Marker::parse(&c));
    if let Some(current) = &existing
        && !force
        && (current.project != wanted.project || current.data_dir != wanted.data_dir)
    {
        let shown = |m: &Marker| {
            format!(
                "project {}, board {}",
                m.project.clone().unwrap_or_else(|| "<none>".into()),
                m.data_dir
                    .clone()
                    .unwrap_or_else(|| "<not recorded>".into())
            )
        };
        anyhow::bail!(
            "this folder already names a board, and init would replace it:\n  \
             now:      {}\n  \
             would be: {}\n\
             Nothing was changed. To switch project within the SAME board, use \
             `kanbanr project use <name>`; to repoint deliberately, re-run with `--force`.",
            shown(current),
            shown(&wanted)
        );
    }
    let replaced =
        existing.filter(|c| c.project != wanted.project || c.data_dir != wanted.data_dir);
    project::write_marker(&cwd, &wanted)?;
    if let Some(old) = replaced {
        println!(
            "! replaced this folder's previous pointer (project {}, board {})",
            old.project.unwrap_or_else(|| "<none>".into()),
            old.data_dir.unwrap_or_else(|| "<not recorded>".into())
        );
    }
    println!("✓ selected '{proj}' here (.kanbanr)");
    if !no_hooks {
        report_hooks_install();
    }
    println!();
    println!("Next:");
    println!("  kanbanr milestone add --name \"v1\" --code MS-001");
    println!("  kanbanr feature add --title \"First feature\" --milestone MS-001 --spec \"# …\"");
    println!("  kanbanr board                 # see the board");
    println!("  kanbanr open                  # watch it live in the browser");
    Ok(())
}

/// Which settings file `kanbanr hooks` acts on. Project scope is the default: the hooks exist to
/// serve a board, so a checkout with no board should carry none of them.
fn hooks_dir_for(global: bool) -> anyhow::Result<PathBuf> {
    if global {
        return hooks::claude_dir()
            .ok_or_else(|| anyhow::anyhow!("no home directory: set CLAUDE_CONFIG_DIR"));
    }
    let cwd = std::env::current_dir()?;
    let root = project::project_root(&cwd, project::home_dir().as_deref());
    Ok(hooks::project_config_dir(&root))
}

/// Register the Claude Code hooks during `init`; never fails `init`. (FEAT-044)
fn report_hooks_install() {
    let Ok(dir) = hooks_dir_for(false) else {
        println!("• Claude Code hooks not installed: could not resolve this project's .claude dir");
        return;
    };
    let settings = hooks::settings_path(&dir);
    let scripts = match hooks::claude_dir() {
        Some(home) => hooks::scripts_dir(&home),
        None => {
            println!("• Claude Code hooks not installed: no home directory found");
            return;
        }
    };
    match hooks::install_in(&dir, &scripts) {
        Ok(hooks::Installed::Added(events)) => println!(
            "✓ Claude Code hooks added to {} ({}) — this project only; `--global` for all of them",
            settings.display(),
            events.join(", ")
        ),
        Ok(hooks::Installed::AlreadyPresent) => println!("✓ Claude Code hooks already installed"),
        Ok(hooks::Installed::ProvidedByPlugin) => {
            println!("✓ Claude Code hooks provided by the kanbanr plugin")
        }
        Ok(hooks::Installed::SkillMissing(scripts)) => println!(
            "• Claude Code hooks not installed: the kanbanr skill isn't at {}. Install it \
             (`make install-skill` in the kanbanr repo), then run `kanbanr hooks install`.",
            scripts.display()
        ),
        Err(e) => println!("• Claude Code hooks not installed: {e}"),
    }
}

/// `kanbanr hooks …` (FEAT-044, scoped per project by FEAT-065).
fn run_hooks(cli: &Cli, cmd: &HooksCmd) -> anyhow::Result<()> {
    let global = matches!(
        cmd,
        HooksCmd::Install { global: true }
            | HooksCmd::Status { global: true }
            | HooksCmd::Uninstall { global: true }
    );
    let dir = hooks_dir_for(global)?;
    // The scripts are installed once per machine even when the settings belong to one project.
    let scripts = hooks::scripts_dir(
        &hooks::claude_dir()
            .ok_or_else(|| anyhow::anyhow!("no home directory: set CLAUDE_CONFIG_DIR"))?,
    );
    match cmd {
        HooksCmd::Install { .. } => match hooks::install_in(&dir, &scripts)? {
            hooks::Installed::Added(events) => println!(
                "added kanbanr hooks ({}) to {}. They take effect in new Claude Code sessions.",
                events.join(", "),
                hooks::settings_path(&dir).display()
            ),
            hooks::Installed::AlreadyPresent => println!("kanbanr hooks are already installed"),
            hooks::Installed::ProvidedByPlugin => {
                println!("the kanbanr plugin is enabled and provides the hooks; nothing to add")
            }
            hooks::Installed::SkillMissing(scripts) => anyhow::bail!(
                "the kanbanr skill isn't installed at {}: run `make install-skill` in the kanbanr repo first",
                scripts.display()
            ),
        },
        HooksCmd::Uninstall { .. } => {
            let n = hooks::uninstall(&dir)?;
            println!(
                "removed {n} kanbanr hook entr{}",
                if n == 1 { "y" } else { "ies" }
            );
        }
        HooksCmd::Status { .. } => {
            let st = hooks::status_in(&dir, &scripts)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&st)?);
                return Ok(());
            }
            println!("settings: {}", st.settings);
            println!(
                "skill installed: {}",
                if st.skill_installed { "yes" } else { "no" }
            );
            if st.plugin {
                println!("kanbanr plugin: enabled (provides the hooks)");
            }
            for h in &st.hooks {
                let event = h["event"].as_str().unwrap_or("");
                match h["command"].as_str() {
                    Some(c) if h["script_exists"] == true => println!("{event}: ✓ {c}"),
                    Some(c) => {
                        println!("{event}: ✗ script missing: {c} (run `kanbanr hooks install`)")
                    }
                    None => println!("{event}: not registered"),
                }
            }
        }
    }
    Ok(())
}

/// Pick the data folder for `init`. `--data-dir` or an existing marker wins (and is recorded in the
/// marker); `$KANBANR_DATA_DIR` or an existing `./data` board is used as-is (not recorded).
/// Otherwise ask in a terminal, or take the recommended `<repo>.kanbanr` sibling when there is no
/// terminal. Returns the folder and whether to record it in the marker.
fn choose_init_data_dir(cli: &Cli, cwd: &Path) -> anyhow::Result<(PathBuf, bool)> {
    let home = project::home_dir();
    let resolved = project::resolve_data_dir_detailed(cli.data_dir.as_deref());
    match resolved.source {
        DataDirSource::Flag | DataDirSource::Marker => return Ok((resolved.path, true)),
        DataDirSource::Env => return Ok((resolved.path, false)),
        DataDirSource::Default if resolved.path.join("projects").is_dir() => {
            println!("• using the existing data folder ./data");
            return Ok((resolved.path, false));
        }
        DataDirSource::Default => {}
    }

    let suggested =
        project::suggested_data_dir(cwd, home.as_deref()).unwrap_or_else(|| cwd.join("data"));
    if cli.json || !std::io::stdin().is_terminal() {
        println!(
            "• board folder: {} (pass --data-dir to choose another)",
            suggested.display()
        );
        return Ok((suggested, true));
    }

    let mut options = vec![suggested];
    for dir in project::nearby_data_dirs(cwd, home.as_deref()) {
        if !options.contains(&dir) {
            options.push(dir);
        }
    }
    println!("Where should kanbanr keep this project's board? (a separate git repo)");
    for (i, dir) in options.iter().enumerate() {
        let note = if dir.join("projects").is_dir() {
            "existing kanbanr folder, shared with its other projects"
        } else {
            "new folder next to the project, recommended"
        };
        println!("  {}) {}  ({note})", i + 1, dir.display());
    }
    print!("Choose a number or type a path [1]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim();
    let picked = answer
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| options.get(i));
    let chosen = match picked {
        _ if answer.is_empty() => options[0].clone(),
        Some(dir) => dir.clone(),
        None => cwd.join(project::expand_tilde(answer, home.as_deref())),
    };
    Ok((chosen, true))
}

/// Stamp file-sourced `feature.add` ops with the project repo's HEAD commit when the bundle didn't
/// say, so an imported item records which version of the file it came from. (FEAT-042)
fn fill_file_source_revisions(body: &mut Value) {
    let Some(ops) = body["operations"].as_array_mut() else {
        return;
    };
    let needs = |op: &Value| {
        op["op"] == "feature.add"
            && op["source"]["system"] == "file"
            && op["source"]["revision"].is_null()
    };
    if !ops.iter().any(needs) {
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Some(rev) = project::project_revision(&cwd, project::home_dir().as_deref()) else {
        return;
    };
    for op in ops.iter_mut().filter(|op| needs(op)) {
        op["source"]["revision"] = json!(rev);
    }
}

/// One line of a `batch --dry-run` preview.
fn describe_batch_result(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    let op = s("op");
    if r["skipped"] == true {
        return match op.as_str() {
            "feature.add" | "milestone.add" => format!(
                "  = skip {op:<12} {} {} ({})",
                s("code"),
                s("title"),
                s("reason")
            ),
            _ => format!(
                "  = skip {op:<12} (under already-imported '{}')",
                s("target")
            ),
        };
    }
    let detail = match op.as_str() {
        "feature.add" => format!("{} {} [{}]", s("code"), s("title"), s("status")),
        "milestone.add" => format!("{} {}", s("code"), s("title")),
        "todo.add" => format!("{} {}", s("feature"), s("code")),
        "task.add" => s("key"),
        "task.state" | "feature.move" => format!("{} {}", s("code"), s("status")),
        "doc.write" | "doc.folder" => s("path"),
        _ => s("code"),
    };
    format!("  + {op:<17} {}", detail.trim())
}

/// `kanbanr sources`: list imported features' provenance and check file sources against the
/// project folder. With `write`, record/clear `missing_since` in one commit. (FEAT-042)
fn run_sources(cli: &Cli, client: &Backend, write: bool) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let cwd = std::env::current_dir()?;
    let root = project::project_root(&cwd, project::home_dir().as_deref());
    let today = kanbanr_core::now_rfc3339();

    let mut rows = Vec::new();
    let mut edits = Vec::new();
    for f in &project.features {
        let Some(src) = &f.source else { continue };
        // Only file sources can be checked locally; `TODO.md:14` -> `TODO.md`.
        let present = (src.system == "file").then(|| {
            let path = match src.reference.rsplit_once(':') {
                Some((file, line)) if line.chars().all(|c| c.is_ascii_digit()) => file,
                _ => src.reference.as_str(),
            };
            root.join(path).exists()
        });
        let mut updated = src.clone();
        match present {
            Some(false) if src.missing_since.is_none() => {
                updated.missing_since = Some(today.clone())
            }
            Some(true) if src.missing_since.is_some() => updated.missing_since = None,
            _ => {}
        }
        if updated != *src {
            edits.push(json!({"op": "feature.edit", "code": f.code, "source": updated}));
        }
        rows.push(json!({
            "code": f.code,
            "system": src.system,
            "ref": src.reference,
            "revision": src.revision,
            "url": src.url,
            "imported_at": src.imported_at,
            "present": present,
            "missing_since": updated.missing_since,
        }));
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else if rows.is_empty() {
        println!("(no imported features)");
    } else {
        for r in &rows {
            let state = match r["present"].as_bool() {
                Some(true) => "present".to_string(),
                Some(false) => format!(
                    "MISSING since {}",
                    r["missing_since"]
                        .as_str()
                        .unwrap_or("")
                        .get(..10)
                        .unwrap_or("now")
                ),
                None => "-".to_string(),
            };
            let rev = r["revision"]
                .as_str()
                .map(|v| format!(" @{}", v.get(..7).unwrap_or(v)))
                .unwrap_or_default();
            println!(
                "{:<12} {:<22} {} {}{rev}",
                r["code"].as_str().unwrap_or(""),
                state,
                r["system"].as_str().unwrap_or(""),
                r["ref"].as_str().unwrap_or(""),
            );
        }
    }

    if !edits.is_empty() {
        if write {
            client.write(
                Method::Post,
                &format!("/projects/{p}/batch"),
                Some(
                    json!({"operations": edits, "message": "sources: update missing-source marks"}),
                ),
            )?;
            if !cli.json {
                println!("recorded {} source change(s)", edits.len());
            }
        } else if !cli.json {
            println!(
                "{} source mark(s) out of date; run `kanbanr sources --write` to record them",
                edits.len()
            );
        }
    }
    Ok(())
}

/// A duration a person would recognise. Days are the unit that matters for a board, but work that
/// takes an afternoon is not "0.0 days" — printing it that way makes a working metric look broken
/// and teaches the reader to skip the line (FEAT-062).
fn duration(days: f64) -> String {
    if days >= 1.0 {
        format!("{days:.1}d")
    } else if days * 24.0 >= 1.0 {
        format!("{:.0}h", days * 24.0)
    } else {
        format!("{:.0}m", days * 24.0 * 60.0)
    }
}

/// Stale evidence, summarised. Every commit moves HEAD, so in an active repo this fires on nearly
/// everything and clears on the next full test run — enumerating it buries the cases that matter
/// and trains the reader to ignore the line entirely (FEAT-062).
fn stale_line(stale: &[String]) -> String {
    const SHOWN: usize = 3;
    let head = stale
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if stale.len() <= SHOWN {
        format!("evidence recorded at an earlier revision: {head} — re-run the suite to refresh it")
    } else {
        format!(
            "evidence recorded at an earlier revision: {} requirement(s), e.g. {head} — re-run the \
             suite to refresh it",
            stale.len()
        )
    }
}

/// Trace (FEAT-057). The gaps print last and unindented, because they are what the command is
/// for: a chain that only lists what exists lets a requirement with no test read as fine.
fn run_trace(
    cli: &Cli,
    client: &Backend,
    subject: Option<&str>,
    zachman: bool,
) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    // A bare `R-2` means the requirement of whatever item this branch belongs to.
    let branch_code = scm_context(cli, client)
        .ok()
        .and_then(|ctx| branch_item(&ctx));

    if zachman {
        let scope = subject.map(str::to_string);
        let resp = client.get(&format!(
            "/projects/{p}/zachman{}",
            scope
                .map(|s| format!("?scope={}", urlencode(&s)))
                .unwrap_or_default()
        ))?;
        if cli.json {
            println!("{}", pretty(&resp));
            return Ok(());
        }
        let view: kanbanr_core::trace::ZachmanView = serde_json::from_str(&resp)?;
        println!("# Zachman view — {}\n", view.scope);
        for cell in &view.cells {
            println!(
                "{:<6} {}/{} item(s){}",
                cell.column,
                cell.answered_by,
                cell.items,
                if cell.decisions.is_empty() {
                    String::new()
                } else {
                    format!("   decisions: {}", cell.decisions.join(", "))
                }
            );
        }
        print_gaps(&view.gaps);
        return Ok(());
    }

    let subject = subject
        .map(str::to_string)
        .or_else(|| branch_code.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "trace what? a goal (G-2), an item (FEAT-046) or a requirement (FEAT-046/R-2)"
            )
        })?;
    let mut query = format!("subject={}", urlencode(&subject));
    if let Some(code) = &branch_code {
        query.push_str(&format!("&item={}", urlencode(code)));
    }
    let resp = client.get(&format!("/projects/{p}/trace?{query}"))?;
    if cli.json {
        println!("{}", pretty(&resp));
        return Ok(());
    }
    let t: kanbanr_core::trace::Trace = serde_json::from_str(&resp)?;
    println!("# {}\n", t.heading);
    if let Some(goal) = &t.goal {
        println!("goal:    {goal}");
    }
    if let Some(purpose) = &t.purpose {
        println!("purpose: {purpose}");
    }
    for item in &t.items {
        println!("\n{} [{}] {}", item.code, item.status, item.title);
        for r in &item.requirements {
            println!("  {} ({}) {}", r.id, r.kind, r.text.trim());
            for (name, green) in &r.tests {
                println!("      {} {name}", if *green { "green" } else { "  —  " });
            }
        }
    }
    if !t.decisions.is_empty() {
        println!("\ndecisions: {}", t.decisions.join(", "));
    }
    if !t.documents.is_empty() {
        println!("documents: {}", t.documents.join(", "));
    }
    print_gaps(&t.gaps);
    Ok(())
}

fn print_gaps(gaps: &[String]) {
    if gaps.is_empty() {
        println!("\nno gaps");
        return;
    }
    println!("\ngaps ({}):", gaps.len());
    for gap in gaps {
        println!("  - {gap}");
    }
}

/// `kanbanr why` (FEAT-057) — the direct answer to "why does this code exist?".
///
/// Three sources, in order of how much they can be trusted: an annotation on the line names the
/// requirement outright; failing that, `git blame` finds the commit that wrote it and its trailer
/// names one; failing that, the line has no reference and the command says so rather than
/// guessing from the file's neighbours.
fn run_why(cli: &Cli, client: &Backend, target: &str) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let (path, line) = match target.rsplit_once(':') {
        Some((path, number)) if number.chars().all(|c| c.is_ascii_digit()) => {
            (path, number.parse::<usize>().ok())
        }
        _ => (target, None),
    };
    let root = scm::repo_root().ok_or_else(|| anyhow::anyhow!("not inside a git repository"))?;
    let full = root.join(path);
    if !full.exists() {
        anyhow::bail!("{path} is not in this repository");
    }

    // 1) The annotation on the line, or the nearest one above it — `(FEAT-046 R-2)`.
    let text = std::fs::read_to_string(&full).unwrap_or_default();
    let annotated = annotation_for(&text, line);
    // 2) The commit that last touched it, and what its trailer says.
    let blamed = line.and_then(|n| blame_refs(&root, path, n));

    let (source, refs) = match (&annotated, &blamed) {
        (Some(refs), _) => ("the annotation on the code", refs.clone()),
        (None, Some((sha, refs))) if !refs.is_empty() => (
            Box::leak(format!("commit {sha}").into_boxed_str()) as &str,
            refs.clone(),
        ),
        _ => {
            println!(
                "{target} carries no reference, and the commit that wrote it named none.\n\
                 Nothing on the board explains this code — which is the finding, not an error."
            );
            return Ok(());
        }
    };

    println!("{target} — from {source}: {}\n", refs.join(", "));
    for reference in &refs {
        let resp = client.get(&format!(
            "/projects/{p}/trace?subject={}",
            urlencode(reference)
        ));
        match resp {
            Ok(resp) => {
                let t: kanbanr_core::trace::Trace = serde_json::from_str(&resp)?;
                for item in &t.items {
                    for r in &item.requirements {
                        println!("  requirement  {}/{}: {}", item.code, r.id, r.text.trim());
                    }
                    if item.requirements.is_empty() {
                        println!("  item         {} — {}", item.code, item.title);
                    }
                }
                if let Some(goal) = &t.goal {
                    println!("  goal         {goal}");
                }
                if let Some(purpose) = &t.purpose {
                    println!("  purpose      {purpose}");
                }
                if !t.decisions.is_empty() {
                    println!("  decisions    {}", t.decisions.join(", "));
                }
            }
            Err(e) => println!("  {reference}: {e}"),
        }
    }
    Ok(())
}

/// A code annotation: `(FEAT-046 R-2)` or `(FEAT-046)` in a comment. Looks at the line itself,
/// then upward — annotations sit on the unit that owns the behaviour, not on every line of it.
fn annotation_for(text: &str, line: Option<usize>) -> Option<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let start = line
        .map(|n| n.min(lines.len()).saturating_sub(1))
        .unwrap_or(0);
    let search: Vec<&str> = match line {
        // Upward from the line, but not the whole file: 40 lines is about a function.
        Some(_) => lines[start.saturating_sub(40)..=start]
            .iter()
            .rev()
            .copied()
            .collect(),
        None => lines.clone(),
    };
    for candidate in search {
        if let Some(found) = parse_annotation(candidate) {
            return Some(found);
        }
    }
    None
}

fn parse_annotation(line: &str) -> Option<Vec<String>> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let inside = line[open + 1..close].trim();
    let mut parts = inside.split_whitespace();
    let code = parts.next()?;
    // A code is `PREFIX-123`; anything else in parentheses is ordinary prose.
    let (prefix, number) = code.split_once('-')?;
    if prefix.is_empty()
        || !prefix.chars().all(|c| c.is_ascii_uppercase())
        || !number.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    match parts.next() {
        Some(requirement) if requirement.starts_with("R-") => {
            Some(vec![format!("{code}/{requirement}")])
        }
        _ => Some(vec![code.to_string()]),
    }
}

/// The references in the trailer of the commit that last touched this line.
fn blame_refs(root: &Path, path: &str, line: usize) -> Option<(String, Vec<String>)> {
    // -C -M follow the line through moves and copies, which is what makes blame survive the
    // refactors that would otherwise lose the link.
    let range = format!("{line},{line}");
    let out = scm::git(
        root,
        &["blame", "-C", "-M", "-L", &range, "--porcelain", "--", path],
    )
    .ok()?;
    let sha = out.split_whitespace().next()?.to_string();
    let message = scm::git(root, &["log", "-1", "--format=%B", &sha]).ok()?;
    let refs: Vec<String> = kanbanr_core::scm::parse_refs(&message)
        .into_iter()
        .map(|r| r.as_token().trim_start_matches("kanbanr:").to_string())
        .collect();
    Some((sha.get(..8).unwrap_or(&sha).to_string(), refs))
}

/// Where the built monitor lives, for `review --ui`. `$KANBANR_UI_DIR` wins; otherwise the copy
/// installed beside the binary, or a `web/dist` in the current checkout.
fn ui_dir_for_review() -> Option<String> {
    if let Ok(dir) = std::env::var("KANBANR_UI_DIR") {
        return Some(dir);
    }
    let candidates = [
        std::env::current_dir().ok().map(|d| d.join("web/dist")),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("../share/kanbanr/web"))),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|d| d.join("index.html").is_file())
        .map(|d| d.display().to_string())
}

/// The board's reasoning, placed where the agent reads its instructions (FEAT-065).
fn run_claude(cli: &Cli, client: &Backend, cmd: &ClaudeCmd) -> anyhow::Result<()> {
    match cmd {
        ClaudeCmd::Sync { file, show } => {
            let p = require_project(cli)?;
            let resp = client.get(&format!("/projects/{p}/claude-block"))?;
            let block: Option<String> = serde_json::from_str(&resp)?;
            let Some(block) = block else {
                anyhow::bail!(
                    "this project has no charter, so there is nothing to put in front of anyone. \
                     Write one first: `kanbanr charter set --file charter.yaml`."
                );
            };
            if *show {
                print!("{block}");
                return Ok(());
            }
            let path = match file {
                Some(path) => path.clone(),
                None => {
                    let cwd = std::env::current_dir()?;
                    project::project_root(&cwd, project::home_dir().as_deref()).join("CLAUDE.md")
                }
            };
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let merged = kanbanr_core::claude::merge(&existing, &block);
            if merged == existing {
                println!("{} is already current", path.display());
                return Ok(());
            }
            std::fs::write(&path, &merged)?;
            println!(
                "{} {} — regenerate it with `kanbanr claude sync` whenever the charter changes",
                if existing.trim().is_empty() {
                    "wrote"
                } else {
                    "refreshed the kanbanr block in"
                },
                path.display()
            );
            Ok(())
        }
        ClaudeCmd::Guard => run_docs_guard(cli, client),
    }
}

/// The documentation rule, enforced rather than requested (FEAT-040, FEAT-065).
///
/// Every document a project produces belongs on its board unless it is part of what the repository
/// ships. That rule lived only in the skill's prose — which is to say, in good intentions — and an
/// agent that forgets it leaves the project's reasoning scattered through a codebase where nothing
/// can find it.
///
/// The decision is **mechanical**: a path, and whether a board is active. It never reads the file
/// and never weighs the charter's prose. A guard that has to interpret is a guard that misfires,
/// and one that misfires gets turned off (ADR-0004).
fn run_docs_guard(cli: &Cli, client: &Backend) -> anyhow::Result<()> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let path = payload["tool_input"]["file_path"]
        .as_str()
        .unwrap_or_default();
    if path.is_empty() {
        return Ok(());
    }
    // Only a project that actually keeps a board has an alternative to offer.
    let Ok(p) = require_project(cli) else {
        return Ok(());
    };
    if client.get(&format!("/projects/{p}")).is_err() {
        return Ok(());
    }
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    let root = project::project_root(&cwd, project::home_dir().as_deref());
    let Some(suggestion) = board_path_for(path, &root, &cwd) else {
        return Ok(());
    };
    println!(
        "{}",
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": format!(
                "Documentation for this project belongs on its board, not in the code repository \
                 — the board is where it can be found, searched, and linked to the work it \
                 explains.\n\nWrite it there instead:\n    kanbanr doc add notes/{suggestion}.md \
                 --file <path>\n\nIf this file is part of what the repository ships (README, \
                 CHANGELOG, docs/, CONTRIBUTING, LICENSE, a doc-site source or a fixture), say so \
                 and write it: those are deliverables, not project reasoning."
            ),
        }})
    );
    Ok(())
}

/// The board path to suggest for a file the guard refuses, or `None` when the write is not the
/// guard's business.
///
/// The guard judges only files **inside the tracked repository** (FEAT-111). A path outside it —
/// Claude Code's own plan file in `~/.claude/plans`, a scratch file, anything reached through
/// `..` — is none of the board's concern, and refusing it once blocked plan mode outright. The
/// suggestion is built from the repo-relative path, never the absolute one, which used to come out
/// as `notes//home/…`.
fn board_path_for(path: &str, root: &Path, cwd: &Path) -> Option<String> {
    let given = Path::new(path);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        cwd.join(given)
    };
    // Resolve `.` and `..` by the text alone: the file may not exist yet, so it cannot be
    // canonicalised, and an escape through `..` must not count as inside the repository.
    let mut resolved = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    let relative = resolved.strip_prefix(root).ok()?;
    let relative: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let relative = relative.join("/");
    if relative.is_empty() || !belongs_on_the_board(&relative) {
        return None;
    }
    Some(
        relative
            .trim_end_matches(".md")
            .trim_end_matches(".markdown")
            .replace(' ', "-")
            .to_lowercase(),
    )
}

/// Is this markdown file project *reasoning* (board) rather than a shipped *deliverable* (repo)?
///
/// The allow-list is what a repository publishes. Everything else that is markdown, written loose
/// in a checkout, is the kind of note that should have gone on the board.
fn belongs_on_the_board(relative: &str) -> bool {
    let lower = relative.to_lowercase();
    if !lower.ends_with(".md") && !lower.ends_with(".markdown") {
        return false;
    }
    const SHIPPED_DIRS: [&str; 8] = [
        "docs/", "doc/", "book/", "site/", "web/", "skill/", ".github/", "editor/",
    ];
    const SHIPPED_FILES: [&str; 9] = [
        "readme.md",
        "changelog.md",
        "contributing.md",
        "security.md",
        "code_of_conduct.md",
        "license.md",
        "third_party.md",
        "claude.md",
        "agents.md",
    ];
    if SHIPPED_DIRS.iter().any(|d| lower.starts_with(d)) {
        return false;
    }
    if SHIPPED_FILES.contains(&lower.as_str()) {
        return false;
    }
    // A test fixture or a vendored file is not project reasoning either.
    if ["target/", "node_modules/", "tests/", "fixtures/", "vendor/"]
        .iter()
        .any(|d| lower.contains(d))
    {
        return false;
    }
    true
}

/// Architecture decisions (FEAT-057).
fn run_adr(cli: &Cli, client: &Backend, cmd: &AdrCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        AdrCmd::New {
            title,
            affects,
            driven_by,
            quality,
            zachman,
            layer,
            status,
            deciders,
        } => {
            let body = json!({
                "title": title,
                "status": status,
                "affects": affects,
                "driven_by": driven_by,
                "quality": quality,
                "zachman": zachman,
                "layer": layer.clone().unwrap_or_default(),
                "deciders": deciders,
            });
            let resp = client.write(Method::Post, &format!("/projects/{p}/adrs"), Some(body))?;
            let adr: kanbanr_core::adr::Adr = serde_json::from_str(&resp)?;
            print_write(
                cli,
                &resp,
                format!(
                    "{} scaffolded at {} — fill in Context, Decision, Alternatives, Consequences \
                     and Compliance (`kanbanr doc show {}`)",
                    adr.id, adr.path, adr.path
                ),
            );
            Ok(())
        }
        AdrCmd::List { for_item } => {
            let resp = client.get(&format!("/projects/{p}/adrs"))?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let adrs: Vec<kanbanr_core::adr::Adr> = serde_json::from_str(&resp)?;
            let shown: Vec<&kanbanr_core::adr::Adr> = adrs
                .iter()
                .filter(|a| {
                    for_item.as_deref().is_none_or(|code| {
                        a.affects.iter().any(|c| c == code)
                            || a.driven_by
                                .iter()
                                .any(|d| d.split('/').next().is_some_and(|c| c == code))
                    })
                })
                .collect();
            if shown.is_empty() {
                println!("(no decisions recorded)");
            }
            for adr in shown {
                println!("{:<9} {:<11} {}", adr.id, adr.status, adr.title);
                let mut detail = Vec::new();
                if !adr.affects.is_empty() {
                    detail.push(format!("affects {}", adr.affects.join(", ")));
                }
                if !adr.driven_by.is_empty() {
                    detail.push(format!("driven by {}", adr.driven_by.join(", ")));
                }
                if !adr.superseded_by.is_empty() {
                    detail.push(format!("superseded by {}", adr.superseded_by));
                }
                let missing = adr.missing_sections();
                if !missing.is_empty() {
                    detail.push(format!("unwritten: {}", missing.join(", ")));
                }
                if !detail.is_empty() {
                    println!("          {}", detail.join(" · "));
                }
            }
            Ok(())
        }
        AdrCmd::Supersede { id, replaces } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/adrs/{id}/supersede"),
                Some(json!({ "replaces": replaces })),
            )?;
            let out: Value = serde_json::from_str(&resp)?;
            let affected = out["affected"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            print_write(
                cli,
                &resp,
                format!(
                    "{id} supersedes {replaces}{}",
                    if affected.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\nthese rested on {replaces} and now stand on an overturned \
                             decision — review them: {affected}"
                        )
                    }
                ),
            );
            Ok(())
        }
        AdrCmd::History { id } => {
            let resp = client.get(&format!("/projects/{p}/adrs"))?;
            let adrs: Vec<kanbanr_core::adr::Adr> = serde_json::from_str(&resp)?;
            let chain = kanbanr_core::adr::lineage(&adrs, id);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&chain)?);
                return Ok(());
            }
            for (i, step) in chain.iter().enumerate() {
                let title = adrs
                    .iter()
                    .find(|a| &a.id == step)
                    .map(|a| a.title.clone())
                    .unwrap_or_default();
                println!("{}{step}  {title}", "  ".repeat(i));
            }
            Ok(())
        }
    }
}

/// Lessons (FEAT-055). Confidence is printed as a percentage, with what it came from, because a
/// lesson you cannot trace is advice — and advice with a number on it is worse than advice.
fn lesson_line(l: &kanbanr_core::lessons::Lesson) -> String {
    let mut out = format!(
        "{:<6} {:>3}%  {}\n",
        l.id,
        (l.confidence_now() * 100.0).round() as i64,
        l.lesson.trim()
    );
    let mut origin: Vec<String> = Vec::new();
    if !l.from_item.is_empty() {
        origin.push(format!("from {}", l.from_item));
    }
    if !l.from_retro.is_empty() {
        origin.push(format!("retro {}", l.from_retro));
    }
    if !l.evidence.is_empty() {
        origin.push(l.evidence.trim().to_string());
    }
    if !l.tags.is_empty() {
        origin.push(format!("tags: {}", l.tags.join(", ")));
    }
    if !origin.is_empty() {
        out.push_str(&format!("       {}\n", origin.join(" · ")));
    }
    out
}

fn run_lesson(cli: &Cli, client: &Backend, cmd: &LessonCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        LessonCmd::Add {
            lesson,
            kind,
            from_item,
            from_retro,
            evidence,
            tags,
            goals,
        } => {
            let body = json!({
                "lesson": lesson,
                "kind": kind,
                "from_item": from_item.clone().unwrap_or_default(),
                "from_retro": from_retro.clone().unwrap_or_default(),
                "evidence": evidence.clone().unwrap_or_default(),
                "tags": tags,
                "goals": goals,
            });
            let resp = client.write(Method::Post, &format!("/projects/{p}/lessons"), Some(body))?;
            let recorded: kanbanr_core::lessons::Lesson = serde_json::from_str(&resp)?;
            print_write(cli, &resp, lesson_line(&recorded).trim_end().to_string());
            Ok(())
        }
        LessonCmd::Affirm { id, note } | LessonCmd::Contradict { id, note } => {
            let affirm = matches!(cmd, LessonCmd::Affirm { .. });
            let verdict = if affirm { "affirm" } else { "contradict" };
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/lessons/{id}/{verdict}"),
                Some(json!({ "note": note.clone().unwrap_or_default() })),
            )?;
            let judged: kanbanr_core::lessons::Lesson = serde_json::from_str(&resp)?;
            let retired = judged.status == kanbanr_core::lessons::LessonStatus::Retired;
            print_write(
                cli,
                &resp,
                format!(
                    "{}{}",
                    lesson_line(&judged).trim_end(),
                    if retired {
                        "\n       retired — kept as a record, no longer surfaced"
                    } else {
                        ""
                    }
                ),
            );
            Ok(())
        }
    }
}

/// A wave's retrospective (FEAT-054). The printed form is deliberately plain: these are the
/// numbers a narrative has to be consistent with, and a chart would invite reading a trend into
/// five data points.
#[allow(clippy::too_many_arguments)]
fn run_retro(
    cli: &Cli,
    client: &Backend,
    milestone: Option<&str>,
    since: Option<&str>,
    label: Option<&str>,
    sprint: Option<&str>,
    write: bool,
    due: bool,
) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    if due {
        let resp = client.get(&format!("/projects/{p}/retro/due"))?;
        if cli.json {
            println!("{}", pretty(&resp));
            return Ok(());
        }
        let waves: Vec<String> = serde_json::from_str(&resp)?;
        if waves.is_empty() {
            println!("no retro is due");
        } else {
            for wave in waves {
                println!("{wave} is finished and has no retro — `kanbanr retro {wave} --write`");
            }
        }
        return Ok(());
    }

    let since = since.map(|s| match s.strip_suffix('d') {
        Some(days) => days
            .parse::<i64>()
            .map(kanbanr_core::report::days_ago)
            .unwrap_or_else(|_| s.to_string()),
        None => s.to_string(),
    });
    let mut query = Vec::new();
    for (key, value) in [
        ("milestone", milestone.map(str::to_string)),
        ("since", since.clone()),
        ("label", label.map(str::to_string)),
        ("sprint", sprint.map(str::to_string)),
    ] {
        if let Some(value) = value {
            query.push(format!("{key}={}", urlencode(&value)));
        }
    }
    let path = format!(
        "/projects/{p}/retro{}{}",
        if query.is_empty() { "" } else { "?" },
        query.join("&")
    );
    let resp = client.get(&path)?;
    if cli.json {
        println!("{}", pretty(&resp));
        return Ok(());
    }
    let retro: kanbanr_core::retro::Retro = serde_json::from_str(&resp)?;
    let facts = retro_markdown(&retro);
    print!("{facts}");

    if write {
        let wave = kanbanr_core::retro::Wave {
            milestone: milestone.map(str::to_string),
            since,
            label: label.map(str::to_string),
            sprint: sprint.map(str::to_string),
        };
        let doc = kanbanr_core::retro::document_path(&wave, milestone.or(sprint));
        let body = format!(
            "{facts}\n## What we make of it\n\n\
             _Written by whoever writes it, from the numbers above. It may explain them, and it \
             may disagree with what we expected — it may not contradict them._\n\n\
             <!-- narrative goes here -->\n"
        );
        client.write(
            Method::Put,
            &format!("/projects/{p}/docs/content"),
            Some(json!({ "path": doc, "content": body })),
        )?;
        println!("\nwritten to {doc}");
    }
    Ok(())
}

/// The facts section, as markdown — the same text the CLI prints and the document stores, so the
/// two can never drift apart.
fn retro_markdown(r: &kanbanr_core::retro::Retro) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Retrospective — {}\n", r.wave);
    let _ = writeln!(out, "## What the board recorded\n");
    let _ = writeln!(
        out,
        "- items: {} ({} finished, {} still open)",
        r.items, r.completed, r.still_open
    );
    // A sprint retro: what it committed to, what it finished, and what it carried out (FEAT-119).
    if let Some(sprint) = &r.sprint {
        let unit = sprint["unit"].as_str().unwrap_or("");
        let _ = writeln!(
            out,
            "- sprint {}: {} of {} {unit} done",
            sprint["code"].as_str().unwrap_or(""),
            num(&sprint["done"]),
            num(&sprint["committed"])
        );
        let carried: Vec<String> = sprint["carried"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                format!(
                    "{} → {}",
                    c["code"].as_str().unwrap_or(""),
                    c["to"].as_str().unwrap_or("")
                )
            })
            .collect();
        if !carried.is_empty() {
            let _ = writeln!(out, "- carried over: {}", carried.join(", "));
        }
    }
    if let Some(started) = &r.started {
        let end = match (&r.finished, r.all_done) {
            (Some(f), _) => f[..10.min(f.len())].to_string(),
            // Over, but the board never recorded when: saying "still running" would be false.
            (None, true) => "finished (date not recorded)".into(),
            (None, false) => "still running".into(),
        };
        let _ = writeln!(out, "- ran: {} → {end}", &started[..10.min(started.len())]);
    }
    let growth = &r.scope_growth;
    let _ = writeln!(
        out,
        "- scope: {} to begin with, {} added ({} defect(s), {} split, {} unaccounted for)",
        growth.original,
        growth.added(),
        growth.defects.len(),
        growth.split.len(),
        growth.unclassified.len()
    );
    for (title, list) in [
        ("added as defects", &growth.defects),
        ("split out of other work", &growth.split),
        ("added without a recorded cause", &growth.unclassified),
    ] {
        if !list.is_empty() {
            let _ = writeln!(out, "  - {title}: {}", list.join(", "));
        }
    }
    let _ = writeln!(
        out,
        "- defects: {} ({} escaped)",
        r.defects.total, r.defects.escaped
    );
    if !r.defects.self_inflicted.is_empty() {
        let _ = writeln!(
            out,
            "  - caused by work in this same wave: {}",
            r.defects.self_inflicted.join(", ")
        );
    }
    match &r.cycle_time_days {
        Some(c) => {
            let _ = writeln!(
                out,
                "- cycle time: p50 {}, p90 {}, max {}",
                duration(c.p50),
                duration(c.p90),
                duration(c.max)
            );
        }
        None => {
            let _ = writeln!(
                out,
                "- cycle time: nothing finished with a recorded history"
            );
        }
    }
    if !r.rework.is_empty() {
        let _ = writeln!(
            out,
            "- rework (called done, then reopened): {}",
            r.rework.join(", ")
        );
    }
    let _ = writeln!(
        out,
        "- requirements proven by a green test: {}/{}",
        r.evidence.proven, r.evidence.requirements
    );
    if !r.evidence.finished_unproven.is_empty() {
        let _ = writeln!(
            out,
            "  - finished with unproven requirements: {}",
            r.evidence.finished_unproven.join(", ")
        );
    }
    if !r.estimates.is_empty() {
        let _ = writeln!(out, "- estimate vs actual:");
        for e in &r.estimates {
            let _ = writeln!(
                out,
                "  - {}: estimated {}, took {} ({:.1}×)",
                e.code,
                duration(e.estimate_days),
                duration(e.actual_days),
                if e.estimate_days > 0.0 {
                    e.actual_days / e.estimate_days
                } else {
                    0.0
                }
            );
        }
    }
    if !r.approximate.is_empty() {
        let _ = writeln!(
            out,
            "- {} item(s) have only changelog timestamps, which record when the board was written \
             rather than how long work took — too coarse for cycle time: {}",
            r.approximate.len(),
            r.approximate.join(", ")
        );
    }
    if !r.no_history.is_empty() {
        let _ = writeln!(
            out,
            "- no recorded moves, so no flow numbers: {}",
            r.no_history.join(", ")
        );
    }
    if let Some(starts) = &r.log_starts {
        let _ = writeln!(
            out,
            "- this wave began before the activity log, which starts {starts} — anything earlier \
             is not recorded here rather than absent"
        );
    }
    out.push('\n');

    // What the wave taught, as it stood when this was written. Still facts — each one carries the
    // evidence it was recorded with — so it sits above the narrative, not inside it.
    if !r.lessons.is_empty() {
        let _ = writeln!(out, "## What this wave taught\n");
        for l in &r.lessons {
            let retired = l.status == kanbanr_core::lessons::LessonStatus::Retired;
            let _ = writeln!(
                out,
                "- **{}** ({}%{}) {}",
                l.id,
                (l.confidence_now() * 100.0).round() as i64,
                if retired { ", since retired" } else { "" },
                l.lesson.trim()
            );
            let mut detail: Vec<String> = Vec::new();
            if !l.from_item.is_empty() {
                detail.push(format!("from {}", l.from_item));
            }
            if !l.evidence.trim().is_empty() {
                detail.push(l.evidence.trim().to_string());
            }
            if !detail.is_empty() {
                let _ = writeln!(out, "  - {}", detail.join(" · "));
            }
        }
        let _ = writeln!(
            out,
            "\n_Live state is `kanbanr lessons`; confidence decays, so these are the figures as of \
             this retrospective._\n"
        );
    }
    out
}

/// The board, the repo and the branch, resolved together (FEAT-056). Every SCM command needs the
/// same three things, and needs to fail the same helpful way when one is missing.
struct Context {
    project: Project,
    root: PathBuf,
    branch: Option<String>,
}

fn scm_context(cli: &Cli, client: &Backend) -> anyhow::Result<Context> {
    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let root = scm::repo_root().ok_or_else(|| {
        anyhow::anyhow!("not inside a git repository — run this in the code repo")
    })?;
    let branch = scm::current_branch(&root);
    Ok(Context {
        project,
        root,
        branch,
    })
}

/// The item the current branch is about. This is the whole reason the branch rule exists: with it,
/// nothing else has to ask what is being worked on.
fn branch_item(ctx: &Context) -> Option<String> {
    let pattern = ctx.project.config.branch_pattern();
    let code = kanbanr_core::scm::code_from_branch(pattern, ctx.branch.as_deref()?)?;
    ctx.project
        .features
        .iter()
        .any(|f| f.code == code)
        .then_some(code)
}

fn run_git(cli: &Cli, client: &Backend, cmd: &GitCmd) -> anyhow::Result<()> {
    match cmd {
        GitCmd::InstallHooks { force } => {
            let root =
                scm::repo_root().ok_or_else(|| anyhow::anyhow!("not inside a git repository"))?;
            let installed = scm::install_hooks(&root, *force)?;
            println!(
                "installed {} in {}: a commit now has to say which item it serves",
                installed.join(" and "),
                root.display()
            );
            Ok(())
        }
        GitCmd::UninstallHooks => {
            let root =
                scm::repo_root().ok_or_else(|| anyhow::anyhow!("not inside a git repository"))?;
            let removed = scm::uninstall_hooks(&root)?;
            println!("removed {removed} kanbanr hook(s)");
            Ok(())
        }
        GitCmd::CheckMsg { file } => run_check_msg(cli, client, file),
        GitCmd::Guard => run_guard(cli, client),
        GitCmd::CheckBranch => run_check_branch(cli, client),
        GitCmd::Status => {
            let ctx = scm_context(cli, client)?;
            let item = branch_item(&ctx);
            println!("repo:    {}", ctx.root.display());
            println!(
                "hooks:   {}",
                if scm::hooks_installed(&ctx.root) {
                    "installed"
                } else {
                    "not installed (kanbanr git install-hooks)"
                }
            );
            println!("branch:  {}", ctx.branch.as_deref().unwrap_or("(detached)"));
            println!("default: {}", scm::default_branch(&ctx.root));
            match item {
                Some(code) => println!("item:    {code}"),
                None => println!("item:    (this branch belongs to no item)"),
            }
            Ok(())
        }
    }
}

/// The Claude Code side of the commit guardrail (FEAT-056): the same rules as the git hooks, but
/// answered *before* the command runs, with the reference to use. Feedback that arrives as a
/// failed command teaches avoidance; feedback that arrives with the fix teaches the rule.
///
/// It only ever denies a `git commit`. Anything it cannot parse, and any repo without a board,
/// passes silently — a guard that blocks what it does not understand would be turned off.
fn run_guard(cli: &Cli, client: &Backend) -> anyhow::Result<()> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    // A heredoc's body is data handed to another program, not shell the guard should read
    // (FEAT-108): a script that merely documents `git commit … -n` was refused as a commit.
    let command = &without_heredoc_bodies(payload["tool_input"]["command"].as_str().unwrap_or(""));
    if !is_git_commit(command) {
        return Ok(());
    }
    let deny = |reason: String| {
        println!(
            "{}",
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }})
        );
    };
    // Tokens of the commit invocation, not substrings of the whole line (FEAT-082, again). The
    // first fix scoped `inline_message` and left this check beside it still scanning `command`, so
    // a message *explaining* the shell idiom was read as passing the flag and the commit was
    // refused. One instance of a bug fixed, its twin two lines above left standing.
    if skips_hooks(command) {
        deny(
            "This skips the commit hooks, which is how a reference gets lost. If the commit \
             genuinely serves no board item, say so on the record instead: put `[no-ref] <why>` \
             in the message."
                .to_string(),
        );
        return Ok(());
    }
    let Ok(ctx) = scm_context(cli, client) else {
        return Ok(()); // not a kanbanr project: not this guard's business
    };
    // Completing a merge is the intended way work reaches the default branch (FEAT-094).
    // So is a repository's first commit (FEAT-105): nothing can branch from a repo with none.
    if let Some(branch) = &ctx.branch
        && branch == &scm::default_branch(&ctx.root)
        && !scm::merge_in_progress(&ctx.root)
        && scm::has_commits(&ctx.root)
    {
        deny(format!(
            "This would commit straight to {branch}. Work belongs on a branch for its item: run \
             `kanbanr start <CODE>` first."
        ));
        return Ok(());
    }
    // The message is only visible when it is given inline; a commit that opens an editor is
    // checked by the commit-msg hook instead.
    let Some(message) = inline_message(command) else {
        return Ok(());
    };
    if kanbanr_core::scm::is_generated_commit(&message) {
        return Ok(());
    }
    let refs = kanbanr_core::scm::parse_refs(&message);
    if refs.is_empty() {
        if kanbanr_core::scm::escape_reason(&message).is_some() {
            return Ok(());
        }
        let suggestion = branch_item(&ctx)
            .map(|code| format!("Refs: kanbanr:{code}"))
            .unwrap_or_else(|| "Refs: kanbanr:<CODE>".to_string());
        deny(format!(
            "This commit does not say which item it serves. Add the trailer `{suggestion}` (a \
             requirement is better: `kanbanr:<CODE>/R-2`), or run `kanbanr commit -m \"…\"`, \
             which fills it in from the branch."
        ));
        return Ok(());
    }
    let problems = kanbanr_core::scm::validate_refs(&ctx.project, &refs);
    if !problems.is_empty() {
        deny(format!(
            "This commit references something that is not on the board:\n{}",
            problems.join("\n")
        ));
    }
    Ok(())
}

/// The command line with every heredoc body removed, keeping the lines that introduce them.
///
/// `<<WORD`, `<<'WORD'`, `<<"WORD"` and `<<-WORD` open a body that runs until a line holding only
/// `WORD` (leading tabs allowed for `<<-`); several on one line are read in order. `<<<` is a
/// here-string, not a heredoc, and a `<<` inside quotes is text. The body is what the shell feeds a
/// program's stdin — a Python script, a JSON bundle, a document — so words in it are never a command
/// the guard should judge (FEAT-108). An unterminated body runs to the end, as it would in sh.
fn without_heredoc_bodies(command: &str) -> String {
    let mut out = String::new();
    let mut pending: std::collections::VecDeque<(String, bool)> = Default::default();
    for line in command.split_inclusive('\n') {
        if let Some((word, strip_tabs)) = pending.front() {
            let bare = line.trim_end_matches(['\n', '\r']);
            let bare = if *strip_tabs {
                bare.trim_start_matches('\t')
            } else {
                bare
            };
            if bare == word {
                pending.pop_front();
            }
            continue;
        }
        pending.extend(heredoc_delimiters(line));
        out.push_str(line);
    }
    out
}

/// The heredocs a single line opens, in order: `(delimiter, strips leading tabs)`.
fn heredoc_delimiters(line: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q && (i == 0 || chars[i - 1] != '\\') {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '<' && chars.get(i + 1) == Some(&'<') && chars.get(i + 2) != Some(&'<') {
            let mut j = i + 2;
            let strip_tabs = chars.get(j) == Some(&'-');
            if strip_tabs {
                j += 1;
            }
            while chars.get(j).is_some_and(|c| *c == ' ' || *c == '\t') {
                j += 1;
            }
            let mut word = String::new();
            let quoted = chars.get(j).copied().filter(|c| *c == '\'' || *c == '"');
            if let Some(q) = quoted {
                j += 1;
                while let Some(&c) = chars.get(j) {
                    j += 1;
                    if c == q {
                        break;
                    }
                    word.push(c);
                }
            } else {
                while let Some(&c) = chars.get(j) {
                    if c.is_whitespace() || ";&|<>()".contains(c) {
                        break;
                    }
                    word.push(c);
                    j += 1;
                }
            }
            if !word.is_empty() {
                found.push((word, strip_tabs));
            }
            i = j;
            continue;
        }
        if c == '<' && chars.get(i + 1) == Some(&'<') {
            i += 3; // `<<<`: a here-string, whose word stays on this line
            continue;
        }
        i += 1;
    }
    found
}

/// Is this shell command a `git commit`? Deliberately narrow: `git -C x commit`, `git commit`, and
/// the same after a `&&`. Anything cleverer risks guessing wrong about someone's shell.
fn is_git_commit(command: &str) -> bool {
    commit_invocation(command).is_some()
}

/// The `&&`-separated part of a command line that invokes `git commit`, if any.
///
/// Split out from `is_git_commit` so the message can be read from **that part alone** (FEAT-082).
/// Scanning the whole command line for `-m` matched those two characters wherever they occurred —
/// in prose, in another flag, in a heredoc the guard was never given — and refused commits whose
/// message was perfectly correct.
fn commit_invocation(command: &str) -> Option<&str> {
    command.split("&&").find(|part| {
        let mut words = part.split_whitespace().skip_while(|w| *w == "sudo");
        if words.next().is_none_or(|w| !w.ends_with("git")) {
            return false;
        }
        // Skip git's own options (`-C <dir>`, `-c k=v`) to reach the subcommand.
        let mut skip_next = false;
        for word in words {
            if skip_next {
                skip_next = false;
                continue;
            }
            if word == "-C" || word == "-c" {
                skip_next = true;
                continue;
            }
            if word.starts_with('-') {
                continue;
            }
            return word == "commit";
        }
        false
    })
}

/// Does this command line ask git to skip the commit hooks?
///
/// Named rather than inline so the test exercises **this** and not a copy of it: the first version
/// of that test carried its own reimplementation, passed against the production code being wrong,
/// and proved nothing (ADR-0008, and L-24 for the second time).
fn skips_hooks(command: &str) -> bool {
    commit_invocation(command).is_some_and(|part| {
        unquoted_tokens(part)
            .iter()
            .any(|t| t == "--no-verify" || t == "-n")
    })
}

/// The tokens of a command line that sit **outside** any quoted span.
///
/// Whitespace-splitting alone is not enough (FEAT-082): a commit whose MESSAGE explains a shell
/// idiom puts that idiom's text in the token stream, so a message about `[ -n "$x" ]` looked
/// exactly like passing the flag. Flags live outside quotes; message text lives inside them. This
/// is a heuristic, not a shell parser — and it only ever decides whether to *inspect* a command,
/// which is the direction where being wrong is cheap.
fn unquoted_tokens(part: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut prev = '\0';
    for c in part.chars() {
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    // A quoted value ends the token it was attached to (`-m"x"`, `--message=…`).
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                } else if c.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                } else {
                    current.push(c);
                }
            }
        }
        prev = c;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The message of a `git commit -m "…"`, when it is written inline.
///
/// The flag is matched as a **shell token** of the commit invocation, never as a substring
/// (FEAT-082): `-m`, `--message`, and the attached forms `-m"…"` / `--message=…`. A commit that
/// supplies its message by file or editor has no inline message, so this returns `None` and the
/// guard defers to the `commit-msg` hook, which reads the real thing.
fn inline_message(command: &str) -> Option<String> {
    let part = commit_invocation(command)?;
    // Byte offset just past the flag, wherever the value begins (detached or attached).
    let mut at = None;
    let mut cursor = 0usize;
    for token in part.split_whitespace() {
        let start = part[cursor..].find(token).map(|i| cursor + i)?;
        cursor = start + token.len();
        if token == "-m" || token == "--message" {
            at = Some(cursor); // detached: the value is the next token
            break;
        }
        if let Some(rest) = token.strip_prefix("--message=") {
            at = Some(cursor - rest.len());
            break;
        }
        // `-mMESSAGE` — but not `--mixed` or any other long flag that merely starts with `-m`.
        if token.len() > 2 && token.starts_with("-m") && !token.starts_with("--") {
            at = Some(start + 2);
            break;
        }
    }
    let rest = part[at?..].trim_start_matches(['=', ' ']);
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return Some(rest.split_whitespace().next()?.to_string());
    }
    closing_quoted(&rest[1..], quote).map(|m| m.replace("\\n", "\n"))
}

/// The text of a shell-quoted value up to its closing quote, or `None` when it never closes.
///
/// Inside double quotes the shell treats `\"` and `\\` as escapes, so an escaped quote is part of the
/// text, not its end (FEAT-098). Stopping at the first quote character cut a message short at
/// `\"Not started\"` and dropped the `Refs:` trailer after it, refusing a correct commit. Single
/// quotes have no escapes in sh, so for them the next `'` is the end.
///
/// `None` is the safe answer: with no message the guard defers to the `commit-msg` hook, which reads
/// the real one. It never decides on a message it could only read part of.
fn closing_quoted(body: &str, quote: char) -> Option<String> {
    if quote == '\'' {
        return body.find('\'').map(|end| body[..end].to_string());
    }
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next @ ('"' | '\\')) => out.push(next),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => return None,
            },
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

/// The commit-msg hook. Every exit here is a decision about whether a commit happens, so each
/// refusal says what to write instead — a guardrail that only says "no" gets bypassed.
fn run_check_msg(cli: &Cli, client: &Backend, file: &Path) -> anyhow::Result<()> {
    use kanbanr_core::scm;
    let message = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("could not read the commit message ({e})"))?;
    // Comment lines are git's own; they are not part of the message.
    let message: String = message
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    if scm::is_generated_commit(&message) {
        return Ok(()); // git wrote it; there is no author to ask for a reference
    }
    // Not a kanbanr project (or no board reachable): say nothing, block nothing.
    let Ok(ctx) = scm_context(cli, client) else {
        return Ok(());
    };
    // References are read before the escape is considered: a message that carries both is a
    // referenced commit that happens to mention the escape, not an escape.
    let refs = scm::parse_refs(&message);
    if refs.is_empty() {
        if let Some(reason) = scm::escape_reason(&message) {
            eprintln!("kanbanr: committing with no reference — recorded reason: {reason}");
            return Ok(());
        }
        let suggestion = branch_item(&ctx)
            .map(|code| format!("Refs: kanbanr:{code}"))
            .unwrap_or_else(|| "Refs: kanbanr:FEAT-001".to_string());
        anyhow::bail!(
            "this commit says which code changed but not which item it serves.\n\
             Add a trailer:\n\n    {suggestion}\n\n\
             or `kanbanr commit -m \"…\"`, which fills it in from the branch.\n\
             If it genuinely serves no item, say so on the record: `[no-ref] <why>`."
        );
    }
    let mut problems = scm::validate_refs(&ctx.project, &refs);
    problems.extend(validate_doc_and_adr_refs(client, &message)?);
    if !problems.is_empty() {
        anyhow::bail!(
            "this commit references something that is not on the board:\n  {}",
            problems.join("\n  ")
        );
    }
    Ok(())
}

/// `Docs:` and `ADR:` trailers resolve against the board too (FEAT-057). A stale document path or
/// an invented decision id is a broken link, exactly like a dangling dependency — and the kind
/// that is hardest to notice later, because it still reads as a reference.
fn validate_doc_and_adr_refs(client: &Backend, message: &str) -> anyhow::Result<Vec<String>> {
    let p = require_project_quiet();
    let (docs, adrs) = (
        kanbanr_core::scm::parse_doc_refs(message),
        kanbanr_core::scm::parse_adr_refs(message),
    );
    if docs.is_empty() && adrs.is_empty() {
        return Ok(Vec::new());
    }
    let Some(p) = p else { return Ok(Vec::new()) };
    let mut problems = Vec::new();
    if !docs.is_empty() {
        let tree = client
            .get(&format!("/projects/{p}/docs"))
            .unwrap_or_default();
        for path in docs {
            if !tree.contains(&path) {
                problems.push(format!("Docs: {path} is not a document on this board"));
            }
        }
    }
    if !adrs.is_empty() {
        let listed: Vec<kanbanr_core::adr::Adr> = client
            .get(&format!("/projects/{p}/adrs"))
            .ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default();
        for id in adrs {
            match listed.iter().find(|a| a.id.eq_ignore_ascii_case(&id)) {
                None => problems.push(format!("ADR: {id} is not a decision on this board")),
                // Not a refusal: sometimes the commit IS the one replacing it. Saying so beats
                // blocking, because a rule that blocks legitimate work gets bypassed wholesale.
                Some(adr) if adr.is_superseded() => eprintln!(
                    "kanbanr: {id} was superseded by {} — make sure this is deliberate",
                    adr.superseded_by
                ),
                Some(_) => {}
            }
        }
    }
    Ok(problems)
}

/// The active project, or `None` — used where a missing project means "not our business" rather
/// than an error the user should see.
fn require_project_quiet() -> Option<String> {
    kanbanr_core::project::resolve_project(None)
}

/// The pre-commit hook: work belongs on a branch that names the item it serves.
fn run_check_branch(cli: &Cli, client: &Backend) -> anyhow::Result<()> {
    let Ok(ctx) = scm_context(cli, client) else {
        return Ok(());
    };
    let Some(branch) = ctx.branch.clone() else {
        return Ok(()); // detached head: a rebase or a bisect, not a place for a policy argument
    };
    // A commit completing a merge is how an item's branch is SUPPOSED to arrive (FEAT-094), and
    // every branch rule below is about where new work is written — none of them applies to it.
    // The first fix exempted only the default-branch check, and the very next check then refused
    // the same commit because `master` names no item; running the recovery for real caught that.
    if scm::merge_in_progress(&ctx.root) {
        return Ok(());
    }
    if branch.starts_with(kanbanr_core::scm::SPIKE_PREFIX) {
        return Ok(()); // a spike is allowed to exist; `finish` is where it is refused
    }
    // The root commit has nowhere else to go: an item branch needs a commit to branch from, so a
    // new repository's first commit lands on its default branch (FEAT-105). Its message is still
    // checked by commit-msg, so it names an item or says `[no-ref] <why>` on the record.
    if !scm::has_commits(&ctx.root) && branch == scm::default_branch(&ctx.root) {
        return Ok(());
    }
    // The commit completing a merge is how an item's branch is SUPPOSED to arrive here (FEAT-094).
    if branch == scm::default_branch(&ctx.root) {
        anyhow::bail!(
            "this is {branch}, the default branch. Work belongs on a branch for its item:\n\n    \
             kanbanr start <CODE>\n\n\
             (or `spike/<name>` to explore — spikes produce a definition change, not merged code.)"
        );
    }
    if branch_item(&ctx).is_none() {
        anyhow::bail!(
            "the branch '{branch}' does not name an item on the board, so nothing here can be \
             traced back to a reason. Use `kanbanr start <CODE>` (pattern: {}), or `spike/<name>`.",
            ctx.project.config.branch_pattern()
        );
    }
    Ok(())
}

fn run_start(
    cli: &Cli,
    client: &Backend,
    code: &str,
    to: Option<&str>,
    no_branch: bool,
    unapproved: Option<&str>,
) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let feature = project
        .features
        .iter()
        .find(|f| f.code == code)
        .ok_or_else(|| anyhow::anyhow!("feature '{code}' not found"))?;
    // Where work starts is the workflow's to say: the status whose gate makes the branch
    // (FEAT-115). Under TOGAF that is Implementation, not the first phase after Vision.
    let status = match to {
        Some(s) => s.to_string(),
        None => branch_status(&project.config)
            .map(Ok)
            .unwrap_or_else(|| first_active_status(&project))?,
    };
    require_reachable(&project.config, code, &feature.status, &status)?;

    // Branch only where there is a repository to branch in. A project need not be code; there the
    // item simply moves, and says so, rather than refusing to start at all.
    let root = if no_branch { None } else { scm::repo_root() };
    if !no_branch && root.is_none() {
        println!("not a git repository: moving {code} without a branch");
    }
    if let Some(root) = root {
        // An item branch is made from the default branch, and a repository with no commits has
        // nothing to make it from (FEAT-105): branching anyway leaves an orphan that never joins.
        if !scm::has_commits(&root) {
            anyhow::bail!(
                "this repository has no commits yet, so there is nothing to branch {code} from. \
                 Make the initial commit on {} first, e.g.\n\n    \
                 git commit -m \"[no-ref] initial commit\"\n",
                scm::default_branch(&root)
            );
        }
        let branch =
            kanbanr_core::scm::branch_for(project.config.branch_pattern(), code, &feature.title);
        if scm::current_branch(&root).as_deref() == Some(branch.as_str()) {
            println!("already on {branch}");
        } else if scm::git(&root, &["rev-parse", "--verify", "--quiet", &branch]).is_ok() {
            scm::git(&root, &["checkout", "-q", &branch])?;
            println!("switched to {branch}");
        } else {
            // Branch from the default branch, not from wherever you happen to be standing.
            let base = scm::default_branch(&root);
            scm::git(&root, &["checkout", "-q", "-b", &branch, &base])
                .or_else(|_| scm::git(&root, &["checkout", "-q", "-b", &branch]))?;
            println!("created {branch} from {base}");
        }
    }

    if feature.status == status {
        println!("{code} is already {status}");
        return Ok(());
    }
    let mut body = json!({ "to": status });
    if let Some(reason) = unapproved {
        body["unapproved"] = json!(reason);
    }
    let resp = client.write(
        Method::Post,
        &format!("/projects/{p}/features/{code}/move"),
        Some(body),
    )?;
    print_write(cli, &resp, format!("{code} -> {status}"));
    print_gate_warnings(cli, &resp);
    Ok(())
}

/// What a gate let through but reported (FEAT-113): its `warns`, a `warn` gate's gaps, or what an
/// override passed over. Said on stderr so a script reading the move's output is unaffected.
fn print_gate_warnings(cli: &Cli, resp: &str) {
    if cli.json {
        return; // the warnings are in the JSON already
    }
    let Ok(v) = serde_json::from_str::<Value>(resp) else {
        return;
    };
    for warning in v["gate_warnings"].as_array().into_iter().flatten() {
        if let Some(w) = warning.as_str() {
            eprintln!("  gate: {w}");
        }
    }
}

/// The first status that means "being worked on": displayed, not the default, not terminal.
fn first_active_status(project: &Project) -> anyhow::Result<String> {
    project
        .config
        .displayed_states
        .iter()
        .find(|s| {
            *s != &project.config.default_state
                && !kanbanr_core::graph::is_terminal_status(&project.config, s)
                && !project.config.is_no_op(s)
        })
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("this workflow has no active status to start in; pass --to <STATUS>")
        })
}

/// The status whose gate creates the item's branch, in workflow order (FEAT-115).
fn branch_status(config: &kanbanr_core::ProjectConfig) -> Option<String> {
    let gates = config.effective_gates();
    config
        .statuses
        .iter()
        .find(|s| {
            gates
                .get(*s)
                .is_some_and(|g| g.on_enter.contains(&kanbanr_core::config::Action::Branch))
        })
        .cloned()
}

/// The shortest chain of allowed transitions from one status to another, both ends included.
fn status_path(config: &kanbanr_core::ProjectConfig, from: &str, to: &str) -> Option<Vec<String>> {
    let mut previous: std::collections::BTreeMap<String, String> = Default::default();
    let mut queue = std::collections::VecDeque::from([from.to_string()]);
    let mut seen = std::collections::BTreeSet::from([from.to_string()]);
    while let Some(at) = queue.pop_front() {
        if at == to {
            let mut path = vec![at.clone()];
            let mut cur = at;
            while let Some(p) = previous.get(&cur) {
                path.push(p.clone());
                cur = p.clone();
            }
            path.reverse();
            return Some(path);
        }
        for next in config.transitions.get(&at).into_iter().flatten() {
            if seen.insert(next.clone()) {
                previous.insert(next.clone(), at.clone());
                queue.push_back(next.clone());
            }
        }
    }
    None
}

/// Refuse a jump the workflow does not allow, naming the stages in between — each has its own gate,
/// so they are walked, not skipped (FEAT-115).
fn require_reachable(
    config: &kanbanr_core::ProjectConfig,
    code: &str,
    from: &str,
    to: &str,
) -> anyhow::Result<()> {
    if from == to || config.transition_allowed(from, to) {
        return Ok(());
    }
    match status_path(config, from, to) {
        Some(path) if path.len() > 2 => anyhow::bail!(
            "{code} is at {from}, and {to} is reached through {}. Move it through those first — \
             each stage has its own gate — with `kanbanr move {code} <STATUS>`.",
            path[1..path.len() - 1].join(" → ")
        ),
        _ => anyhow::bail!("the workflow has no way from {from} to {to}"),
    }
}

fn run_finish(cli: &Cli, client: &Backend, code: Option<&str>) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let ctx_branch = scm::repo_root().and_then(|r| scm::current_branch(&r));
    let code = match code {
        Some(c) => c.to_string(),
        None => {
            let branch = ctx_branch
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no branch to infer the item from; name it"))?;
            if branch.starts_with(kanbanr_core::scm::SPIKE_PREFIX) {
                anyhow::bail!(
                    "'{branch}' is a spike. A spike's output is a change to an item's definition, \
                     not merged code — write that up, then start the item it belongs to."
                );
            }
            kanbanr_core::scm::code_from_branch(project.config.branch_pattern(), &branch)
                .ok_or_else(|| {
                    anyhow::anyhow!("the branch '{branch}' names no item; pass the code")
                })?
        }
    };
    let feature = project
        .features
        .iter()
        .find(|f| f.code == code)
        .ok_or_else(|| anyhow::anyhow!("feature '{code}' not found"))?;

    // The end this item can reach next: a terminal status the workflow allows from where it is
    // (FEAT-115). The first declared terminal was taken before, and refused from any stage that
    // does not lead straight to it — Implementation under TOGAF.
    let config = &project.config;
    let ends: Vec<&String> = config
        .statuses
        .iter()
        .filter(|s| kanbanr_core::graph::is_terminal_status(config, s) && !config.is_no_op(s))
        .collect();
    let terminal = match ends
        .iter()
        .find(|s| config.transition_allowed(&feature.status, s))
    {
        Some(s) => (*s).clone(),
        None => {
            let first = ends
                .first()
                .map(|s| (*s).clone())
                .unwrap_or_else(|| "Completed".to_string());
            require_reachable(config, &code, &feature.status, &first)?;
            first
        }
    };

    // Everything that makes "done" mean something, checked before it is claimed. A workflow that
    // declares its gates says that itself, on the end status, and the move below enforces it; one
    // that does not keeps the rules `check` reports.
    let gaps: Vec<String> = if config.gates.is_empty() {
        check_report(feature)["gaps"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|g| g.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let open: Vec<String> = feature
        .todo_lists
        .iter()
        .flat_map(|l| {
            l.tasks
                .iter()
                .filter(|t| t.state != kanbanr_core::models::TaskState::Completed)
                .map(move |t| format!("{}/{} {}", l.code, t.key, t.text))
        })
        .collect();
    if !gaps.is_empty() || !open.is_empty() {
        let mut lines = gaps;
        lines.extend(open.iter().map(|t| format!("task not finished: {t}")));
        anyhow::bail!(
            "{code} is not finished:\n  {}\n\nFix these, or move it by hand if you disagree.",
            lines.join("\n  ")
        );
    }

    let resp = client.write(
        Method::Post,
        &format!("/projects/{p}/features/{code}/move"),
        Some(json!({ "to": terminal })),
    )?;
    print_write(
        cli,
        &resp,
        format!("{code} -> {terminal}. Merge when you are ready; the branch is not the archive."),
    );
    Ok(())
}

fn run_commit(
    cli: &Cli,
    client: &Backend,
    message: &str,
    all: bool,
    refs: &[String],
) -> anyhow::Result<()> {
    let ctx = scm_context(cli, client)?;
    let item = branch_item(&ctx);
    // A `--ref` that starts with a code this board knows stands on its own (a cross-cutting
    // change may serve several items); anything else is read as part of the branch's item.
    let known = |value: &str| {
        value
            .split('/')
            .next()
            .is_some_and(|code| ctx.project.features.iter().any(|f| f.code == code))
    };
    let tokens: Vec<String> = if refs.is_empty() {
        item.iter().map(|c| format!("kanbanr:{c}")).collect()
    } else {
        refs.iter()
            .map(|r| match (&item, known(r)) {
                (_, true) => format!("kanbanr:{r}"),
                (Some(code), false) => format!("kanbanr:{code}/{r}"),
                (None, false) => format!("kanbanr:{r}"),
            })
            .collect()
    };
    if tokens.is_empty() {
        anyhow::bail!(
            "this branch names no item, so there is nothing to reference. \
             `kanbanr start <CODE>` first, or pass --ref."
        );
    }
    let parsed = kanbanr_core::scm::parse_refs(&tokens.join(" "));
    let problems = kanbanr_core::scm::validate_refs(&ctx.project, &parsed);
    if !problems.is_empty() {
        anyhow::bail!("{}", problems.join("\n"));
    }
    let message = format!("{}\n\nRefs: {}\n", message.trim_end(), tokens.join(", "));

    let mut args: Vec<&str> = vec!["commit"];
    if all {
        args.push("-a");
    }
    args.push("-m");
    args.push(&message);
    let out = std::process::Command::new("git")
        .args(&args)
        .current_dir(&ctx.root)
        .status()
        .map_err(|e| anyhow::anyhow!("could not run git: {e}"))?;
    if !out.success() {
        anyhow::bail!("git commit failed");
    }
    Ok(())
}

/// Check that every tracked test name still exists in the project (FEAT-053), modelled on
/// `run_sources`. A test that has been renamed or deleted leaves its last result behind, and that
/// stale green is the most misleading thing a board can hold: it reports a requirement as proven
/// by something nobody can run. `--write` returns those to `planned` rather than deleting them —
/// the requirement still needs a test, it just no longer has one.
fn run_tests(cli: &Cli, client: &Backend, write: bool) -> anyhow::Result<()> {
    use kanbanr_core::models::TestState;
    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let cwd = std::env::current_dir()?;
    let root = project::project_root(&cwd, project::home_dir().as_deref());
    // The board itself records every test name, and the data folder often sits inside the project
    // — searching it would find every test in its own definition and report all of them present.
    let board = project::resolve_data_dir(cli.data_dir.as_deref());

    let mut rows = Vec::new();
    let mut ops = Vec::new();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                let name = t.name.trim();
                // A manual check is performed by a person and has no name in the repo to find;
                // treating its absence as rot would push people to stop recording manual gates.
                if name.is_empty() || t.kind.trim().eq_ignore_ascii_case("manual") {
                    continue;
                }
                let found = test_exists(&root, &board, name);
                rows.push(json!({
                    "code": f.code, "requirement": r.id, "test": name,
                    "state": t.state, "found": found,
                }));
                // Only recorded evidence is worth correcting: a `planned` test is expected to be
                // absent — that is what planned means.
                if !found && t.state != TestState::Planned {
                    ops.push(json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": name, "state": "planned", "checked_rev": "",
                    }));
                }
            }
        }
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else if rows.is_empty() {
        println!("(no tests are tracked yet)");
    } else {
        for r in &rows {
            println!(
                "{:<12} {:<6} {:<8} {} {}",
                r["code"].as_str().unwrap_or(""),
                r["requirement"].as_str().unwrap_or(""),
                r["state"].as_str().unwrap_or(""),
                if r["found"].as_bool() == Some(true) {
                    "found"
                } else {
                    "NOT IN THE REPO"
                },
                r["test"].as_str().unwrap_or(""),
            );
        }
    }

    if !ops.is_empty() {
        if write {
            let n = ops.len();
            client.write(
                Method::Post,
                &format!("/projects/{p}/batch"),
                Some(json!({
                    "operations": ops,
                    "message": format!("tests: {n} result(s) whose test no longer exists are back to planned"),
                })),
            )?;
            if !cli.json {
                println!("returned {n} result(s) to planned");
            }
        } else if !cli.json {
            println!(
                "{} recorded result(s) name a test that is not in the repo; \
                 run `kanbanr tests --write` to return them to planned",
                ops.len()
            );
        }
    }
    Ok(())
}

/// Is this test name still in the project? A path-like name (`src/cart.test.ts`) is a file; any
/// other name is searched for in the tracked sources. `git grep` is used when the folder is a git
/// repo — it already knows what to skip — with a bounded walk as the fallback.
fn test_exists(root: &std::path::Path, board: &std::path::Path, name: &str) -> bool {
    if name.contains('/') && root.join(name).exists() {
        return true;
    }
    // A cargo test path is `module::path::test_name`; the source only contains the last segment.
    let needle = name.rsplit("::").next().unwrap_or(name);
    if let Ok(out) = std::process::Command::new("git")
        .args(["grep", "-l", "-F", "--", needle])
        .current_dir(root)
        .output()
    {
        // 0 = matched, 1 = searched and found nothing, anything else (not a repo) = fall back.
        match out.status.code() {
            Some(0) => return !out.stdout.is_empty(),
            Some(1) => return false,
            _ => {}
        }
    }
    walk_for(root, board, needle, 0)
}

fn walk_for(dir: &std::path::Path, board: &std::path::Path, needle: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || matches!(&*name, "target" | "node_modules" | "dist") {
            continue;
        }
        if path == board {
            continue;
        }
        if path.is_dir() {
            if walk_for(&path, board, needle, depth + 1) {
                return true;
            }
        } else if std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)) {
            return true;
        }
    }
    false
}

/// A starting skeleton for a definition, shaped to the kind of work (FEAT-047). Every kind states
/// why it exists and how it is verified; what differs is the FORM a requirement takes — a feature
/// asserts new behaviour, a defect names the requirement it violates, a chore asserts an invariant.
/// Printing only the fields that apply keeps an author from staring at a form full of blanks.
fn definition_template(kind: &str) -> String {
    let head = "\
# Why this item exists, what must be true, and how that is verified.
# Fill what you know; leave the rest BLANK so `kanbanr doctor` can flag it — never invent.
statement: \"<capability> for <whom> so that <why>\"
goals: [G-1]          # charter goal ids this serves (kanbanr charter show)
zachman:
  what:  \"\"            # data, functions, rules
  how:   \"\"            # approach
  where: \"\"            # component or service
  when:  \"\"            # trigger, frequency, timing
  who:   \"\"            # stakeholder role
  why:   \"\"            # the problem or goal served
";
    let requirements = match kind.trim().to_ascii_lowercase().as_str() {
        "defect" | "bug" => {
            "\
requirements:
  # A defect violates a requirement. If none covers the case, THAT is the finding: add the
  # missing requirement here instead of leaving `violates` blank.
  - kind: functional
    violates: FEAT-000/R-0
    text: \"WHEN <the trigger>, THE SYSTEM SHALL <the behaviour that was wrong>\"
    tests:
      - name: \"<test that reproduces it>\"
        kind: unit
        state: red      # red first, then green once fixed
"
        }
        "chore" | "refactor" => {
            "\
requirements:
  # No new behaviour: the requirement is what must NOT change.
  - kind: functional
    text: \"THE SYSTEM SHALL continue to <invariant this work must preserve>\"
    tests:
      - name: \"<existing suite that proves it>\"
        kind: integration
        state: green
"
        }
        "docs" => {
            "\
requirements:
  # The requirement is the contract the documentation must match.
  - kind: functional
    text: \"THE SYSTEM SHALL document <the contract> as it actually behaves\"
    tests:
      - name: \"<doc check or doctor rule>\"
        kind: manual
        state: planned
"
        }
        "recurring" => {
            "\
requirements:
  # Standing work: one statement, verified per occurrence by its checklist.
  - kind: functional
    text: \"THE SYSTEM SHALL <the standing obligation>\"
    tests:
      - name: \"<per-occurrence check>\"
        kind: manual
        state: planned
"
        }
        _ => {
            "\
requirements:
  - kind: functional
    text: \"WHEN <trigger>, THE SYSTEM SHALL <response>\"
    tests:
      - name: \"<test that verifies it>\"
        kind: unit
        state: planned
  # A quality requirement needs a tag AND a measure that names what checks it — a number with
  # nothing behind it is an unsupported claim, so leave it out unless it is real.
  - kind: nfr
    text: \"WHERE <condition>, THE SYSTEM SHALL <quality behaviour>\"
    iso25010: [Reliability]
    scenario:
      stimulus: \"\"
      environment: \"\"
      response: \"\"
      measure: \"<number, and the test or benchmark that checks it>\"
    tests:
      - name: \"<test that verifies it>\"
        kind: integration
        state: planned
"
        }
    };
    format!("{head}{requirements}")
}

/// `kanbanr charter …` (FEAT-046). The charter is the root of a project's reasoning: goals get
/// ids here, and work items link them.
fn run_charter(cli: &Cli, client: &Backend, cmd: &CharterCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let path = format!("/projects/{p}/charter");
    match cmd {
        CharterCmd::Show => {
            let resp = client.get(&path)?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else {
                let charter: kanbanr_core::Charter = serde_json::from_str(&resp)?;
                print!("{}", kanbanr_core::export::charter_to_markdown(&charter));
            }
        }
        CharterCmd::Set { file } => {
            let raw = match file {
                Some(path) => std::fs::read_to_string(path)?,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            // YAML is a superset of JSON, so one parser accepts either form.
            let charter: kanbanr_core::Charter = serde_yaml::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("invalid charter (YAML or JSON expected): {e}"))?;
            let resp = client.write(Method::Put, &path, Some(serde_json::to_value(charter)?))?;
            let saved: kanbanr_core::Charter = serde_json::from_str(&resp)?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else if saved.is_empty() {
                println!("charter cleared for '{p}'");
            } else {
                println!(
                    "charter saved for '{p}': {} goal(s){}",
                    saved.goals.len(),
                    saved
                        .goals
                        .iter()
                        .map(|g| format!(" {}", g.id))
                        .collect::<String>()
                );
            }
        }
    }
    Ok(())
}

/// `kanbanr mirror …` (FEAT-043).
fn run_mirror(cli: &Cli, client: &Backend, cmd: &MirrorCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let gh = mirror::GhCli::new();
    match cmd {
        MirrorCmd::Enable { repo, allow_public } => {
            mirror::enable(client, &gh, &p, repo, *allow_public)?;
            println!(
                "issue mirror enabled: new features in '{p}' will be mirrored to {repo}.\n\
                 Run `kanbanr mirror sync --all` to also create issues for existing open features."
            );
        }
        MirrorCmd::Disable => {
            mirror::disable(client, &p)?;
            println!("issue mirror disabled for '{p}' (issue links are kept)");
        }
        MirrorCmd::Status { all } => {
            let Some((cfg, actions, linked)) = mirror::status(client, &p, *all)? else {
                if cli.json {
                    println!("{}", json!({"enabled": false}));
                } else {
                    println!("issue mirror: off (kanbanr mirror enable --repo owner/repo)");
                }
                return Ok(());
            };
            let gh_ok = mirror::IssueTracker::check(&gh);
            if cli.json {
                let out = json!({
                    "enabled": true, "repo": cfg.repo, "enabled_at": cfg.enabled_at,
                    "gh": gh_ok.as_ref().map(|_| "ok").unwrap_or_else(|e| e.as_str()),
                    "linked": linked, "actions": actions,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            println!(
                "issue mirror: on → {} (since {})",
                cfg.repo,
                &cfg.enabled_at[..10.min(cfg.enabled_at.len())]
            );
            match gh_ok {
                Ok(()) => println!("gh: ok"),
                Err(e) => println!("gh: {e}"),
            }
            println!("{linked} feature(s) linked; {} pending:", actions.len());
            for a in &actions {
                match a.kind {
                    kanbanr_core::mirror::MirrorActionKind::Create => {
                        println!("  + create  {} {}", a.code, a.title)
                    }
                    kanbanr_core::mirror::MirrorActionKind::Update { number } => {
                        println!("  ~ update  {} #{number} {}", a.code, a.title)
                    }
                }
            }
        }
        MirrorCmd::Sync { all } => {
            let out = mirror::sync(client, &gh, &p, *all)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            for (code, n) in &out.created {
                println!("  + {code} → {}#{n}", out.repo);
            }
            for (code, n) in &out.updated {
                println!("  ~ {code} → {}#{n}", out.repo);
            }
            for (code, e) in &out.failed {
                println!("  ! {code}: {e}");
            }
            println!(
                "created {}, updated {}, failed {}",
                out.created.len(),
                out.updated.len(),
                out.failed.len()
            );
        }
        MirrorCmd::Link { code, number } => {
            let url = mirror::link(client, &gh, &p, code, *number)?;
            println!(
                "linked {code} → {url}\nThe next sync replaces that issue's title and body with kanbanr's."
            );
        }
        MirrorCmd::Pull { code } => {
            let r = mirror::pull(client, &gh, &p, code)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(());
            }
            let state = if r.remote.doc.open { "open" } else { "closed" };
            println!(
                "{} ↔ {}#{} ({state}) {}",
                r.code, r.repo, r.remote.number, r.remote.url
            );
            println!(
                "last synced: {}",
                r.last_synced_at.as_deref().unwrap_or("never")
            );
            println!(
                "edited on GitHub since then: {}",
                if r.edited_on_tracker { "yes" } else { "no" }
            );
            println!(
                "changed in kanbanr since then: {}{}",
                if r.changed_in_kanbanr { "yes" } else { "no" },
                if r.edited_on_tracker && r.changed_in_kanbanr {
                    " (a sync would overwrite the GitHub edits: bring them into kanbanr first)"
                } else {
                    ""
                }
            );
            if r.edited_on_tracker {
                println!("\nGitHub title: {}", r.remote.doc.title);
                println!("GitHub labels: {}", r.remote.doc.labels.join(", "));
                println!("GitHub body:\n{}", r.remote.doc.body.trim_end());
            }
            if r.comments.is_empty() {
                println!("\nno new comments");
            } else {
                println!("\n{} new comment(s):", r.comments.len());
                for c in &r.comments {
                    println!(
                        "  @{} ({}): {}",
                        c.author,
                        c.created_at.get(..10).unwrap_or(""),
                        c.body.trim()
                    );
                }
            }
        }
    }
    Ok(())
}

/// How a data folder is written into a marker in `marker_dir`: relative when nearby, else absolute.
fn marker_path(data_dir: &Path, marker_dir: &Path) -> String {
    project::relative_to(data_dir, marker_dir)
        .display()
        .to_string()
}

/// Print the data folder this directory resolves to (`--json`: provenance and suggestions too).
fn run_where(cli: &Cli) -> anyhow::Result<()> {
    let resolved = project::resolve_data_dir_detailed(cli.data_dir.as_deref());
    let data_dir = project::normalize(&resolved.path);
    if !cli.json {
        println!("{}", data_dir.display());
        return Ok(());
    }
    let cwd = std::env::current_dir()?;
    let home = project::home_dir();
    let home = home.as_deref();
    let show = |p: &Path| p.display().to_string();
    let suggested = project::suggested_data_dir(&cwd, home);
    let out = json!({
        "data_dir": show(&data_dir),
        "source": resolved.source,
        "exists": data_dir.join("projects").is_dir(),
        "inside_git_repo": project::enclosing_git_worktree(&data_dir, home).map(|p| show(&p)),
        "marker": resolved.marker.as_ref().map(|m| show(&m.path)),
        "project": project::resolve_project(cli.project.as_deref()),
        "project_root": show(&project::project_root(&cwd, home)),
        "suggested_data_dir": suggested.as_deref().map(show),
        "suggested_inside_git_repo": suggested
            .as_deref()
            .and_then(|s| project::enclosing_git_worktree(s, home))
            .map(|p| show(&p)),
        "existing_data_dirs": project::nearby_data_dirs(&cwd, home)
            .iter()
            .map(|p| show(p))
            .collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// What to do when the monitor isn't running. The monitor is built into the binary (FEAT-084), so
/// the answer is plain `serve`: naming `--ui-dir web/dist` here once sent a setup off to find a
/// development build in another checkout (FEAT-104).
fn monitor_down_hint(url: &str) -> String {
    format!(
        "monitor not reachable at {url}.\nStart it first (the monitor is built into this binary):\n  \
         kanbanr serve\nthen re-run `kanbanr open`."
    )
}

/// Open the live monitor in the default browser (best-effort; warns if it isn't reachable).
fn run_open(_cli: &Cli) -> anyhow::Result<()> {
    let url = monitor_url();
    // Any HTTP status means the daemon is up; a transport error means it isn't.
    let up = !matches!(
        ureq::get(&format!("{url}/api/projects"))
            .timeout(std::time::Duration::from_millis(800))
            .call(),
        Err(ureq::Error::Transport(_))
    );
    if !up {
        println!("{}", monitor_down_hint(&url));
        return Ok(());
    }
    open_in_browser(&url);
    println!("opened {url}");
    Ok(())
}

/// Best-effort cross-platform "open this URL in the browser".
fn open_in_browser(url: &str) {
    use std::process::Command as P;
    let _ = if cfg!(target_os = "macos") {
        P::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        P::new("cmd").args(["/C", "start", "", url]).spawn()
    } else {
        P::new("xdg-open").arg(url).spawn()
    };
}

/// Who a verdict is attributed to when `--by` is not given (FEAT-077, completed in FEAT-080).
///
/// The data folder's commit identity — the same source the flag's help promises and the monitor
/// reads from `/api/meta`, so a verdict reads identically whichever surface recorded it.
///
/// This used to fall through `/auth/whoami` to the literal string `"unknown"`, which in local mode
/// is every time: there is no server to ask. FEAT-077 fixed the monitor and left this, so the CLI
/// went on writing `by: unknown` while the flag's own help said otherwise. Six ratifications were
/// recorded that way before it was noticed.
///
/// `None` means no identity is configured, and the caller must refuse rather than invent one: an
/// approval that cannot say who gave it is not evidence of agreement.
fn verdict_author(cli: &Cli, given: Option<&String>) -> anyhow::Result<String> {
    if let Some(name) = given.filter(|n| !n.trim().is_empty()) {
        return Ok(name.trim().to_string());
    }
    let dir = project::resolve_data_dir(cli.data_dir.as_deref());
    kanbanr_core::git::identity(&dir)
        .map(|(name, _)| name)
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "this board has no commit identity, so a verdict could not say who gave it.\n\
                 Set one once:  kanbanr identity --name \"You\" --email you@example.com\n\
                 or pass it explicitly with --by."
            )
        })
}

fn require_project(cli: &Cli) -> anyhow::Result<String> {
    project::resolve_project(cli.project.as_deref()).ok_or_else(|| {
        anyhow::anyhow!("could not determine project; pass --project or set KANBANR_PROJECT")
    })
}

/// Print recent activity (from the project's changelog file).
fn run_activity(cli: &Cli, client: &Backend) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    let resp = client.get(&format!("/projects/{p}/activity"))?;
    if cli.json {
        println!("{}", pretty(&resp));
    } else if let Some(arr) = serde_json::from_str::<Value>(&resp)?.as_array() {
        if arr.is_empty() {
            println!("(no activity yet)");
        }
        for a in arr {
            println!(
                "{:<20} {:<14} {}",
                a["time"].as_str().unwrap_or(""),
                a["actor"].as_str().unwrap_or(""),
                a["message"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Show / test the project's notification events log (FEAT-036).
fn run_events(cli: &Cli, client: &Backend, cmd: &EventsCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        EventsCmd::List { limit } => {
            let limit = limit.unwrap_or(25);
            let resp = client.get(&format!("/projects/{p}/events?limit={limit}"))?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else if let Some(arr) = serde_json::from_str::<Value>(&resp)?.as_array() {
                if arr.is_empty() {
                    println!("(no events yet)");
                }
                for e in arr {
                    println!(
                        "{:<20} {:<16} {}",
                        e["time"].as_str().unwrap_or(""),
                        e["kind"].as_str().unwrap_or(""),
                        e["message"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        EventsCmd::Test => {
            let delivered = client.emit_test_event(&p);
            if cli.json {
                println!(
                    "{}",
                    json!({ "project": p, "webhook_configured": delivered })
                );
            } else if delivered {
                println!("test event emitted to '{p}' and POSTed to configured webhook(s)");
            } else {
                println!(
                    "test event emitted to '{p}' (log only — no webhook configured).\n\
                     Configure one via {} or a `webhooks:` list in {} at the data-dir root.",
                    kanbanr_core::eventing::WEBHOOK_ENV,
                    kanbanr_core::eventing::CONFIG_FILE
                );
            }
            Ok(())
        }
    }
}

fn run_remote(cli: &Cli, client: &Backend, cmd: &RemoteCmd) -> anyhow::Result<()> {
    match cmd {
        RemoteCmd::Add { name, url } => {
            client.write(
                Method::Post,
                "/remotes",
                Some(json!({ "name": name, "url": url })),
            )?;
            println!("added remote {name} -> {url}");
            Ok(())
        }
        RemoteCmd::List => {
            let resp = client.get_auth("/remotes")?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else if let Some(arr) = serde_json::from_str::<Value>(&resp)?.as_array() {
                if arr.is_empty() {
                    println!("(no remotes)");
                }
                for r in arr {
                    println!(
                        "{}  {}",
                        r["name"].as_str().unwrap_or(""),
                        r["url"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        RemoteCmd::Remove { name } => {
            client.write(Method::Delete, &format!("/remotes/{name}"), None)?;
            println!("removed remote {name}");
            Ok(())
        }
    }
}

fn read_spec(spec: Option<String>, spec_file: Option<String>) -> anyhow::Result<Option<String>> {
    if let Some(path) = spec_file {
        Ok(Some(std::fs::read_to_string(path)?))
    } else {
        Ok(spec)
    }
}

/// Build a JSON object from (key, optional-value) pairs, omitting None entries.
fn obj(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        if let Some(val) = v {
            m.insert(k.to_string(), val);
        }
    }
    Value::Object(m)
}

fn pretty(s: &str) -> String {
    serde_json::from_str::<Value>(s)
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .unwrap_or_else(|_| s.to_string())
}

fn field(s: &str, key: &str) -> String {
    serde_json::from_str::<Value>(s)
        .ok()
        .and_then(|v| v.get(key).and_then(|x| x.as_str()).map(String::from))
        .unwrap_or_default()
}

/// The project repo's current short revision, for stamping test evidence.
fn project_head_rev() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    project::project_revision(&cwd, project::home_dir().as_deref())
}

/// Record test results from a real run (FEAT-053).
///
/// Reads a Claude Code PostToolUse payload on stdin, extracts the test names that passed and
/// failed, and flips the state of every one the board is tracking. Matching by test NAME rather
/// than by "whatever item is in progress" is what makes this trustworthy: it records what ran,
/// not what someone meant to run, and it is silent when nothing it knows about ran.
fn run_capture(cli: &Cli, client: &Backend) -> anyhow::Result<()> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    // The tool output lives in different places depending on the tool; take the lot as text.
    let text = [
        payload["tool_response"]["stdout"].as_str(),
        payload["tool_response"]["stderr"].as_str(),
        payload["tool_response"]["output"].as_str(),
        payload["tool_response"].as_str(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let results = parse_test_results(&text);
    if results.is_empty() {
        return Ok(()); // not a test run, or nothing recognizable: stay quiet
    }

    let p = require_project(cli)?;
    let project = get_project(client, &p)?;
    let rev = project_head_rev();
    let mut ops = Vec::new();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                if let Some(passed) = results.get(t.name.as_str()) {
                    let state = if *passed { "green" } else { "red" };
                    if t.state == kanbanr_core::models::TestState::parse(state).unwrap_or_default()
                        && (!passed || t.checked_rev.as_str() == rev.as_deref().unwrap_or(""))
                    {
                        continue; // already recorded at this revision; nothing to say
                    }
                    let mut op = json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": t.name, "state": state,
                    });
                    if let Some(rev) = &rev {
                        op["checked_rev"] = json!(rev);
                    }
                    ops.push(op);
                }
            }
        }
    }
    if ops.is_empty() {
        return Ok(());
    }
    let n = ops.len();
    client.write(
        Method::Post,
        &format!("/projects/{p}/batch"),
        Some(json!({ "operations": ops, "message": format!("record {n} test result(s) from a run") })),
    )?;
    eprintln!("kanbanr: recorded {n} test result(s) from this run");
    Ok(())
}

/// Test name -> passed, parsed from a test runner's output. Handles `cargo test` (`test NAME ...
/// ok`) and the common `✓ NAME` / `✗ NAME` shape used by JS runners. Unknown formats yield
/// nothing, which is the right failure: recording a guess would be worse than recording nothing.
fn parse_test_results(text: &str) -> std::collections::BTreeMap<String, bool> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("test ") {
            // cargo: `test module::name ... ok` / `... FAILED`
            if let Some((name, verdict)) = rest.rsplit_once(" ... ") {
                let name = name.trim();
                match verdict.trim() {
                    "ok" => {
                        out.insert(name.to_string(), true);
                    }
                    "FAILED" => {
                        out.insert(name.to_string(), false);
                    }
                    _ => {}
                }
            }
        } else if let Some(name) = line
            .strip_prefix("\u{2713} ")
            .or_else(|| line.strip_prefix("PASS "))
        {
            out.insert(name.trim().to_string(), true);
        } else if let Some(name) = line
            .strip_prefix("\u{2717} ")
            .or_else(|| line.strip_prefix("FAIL "))
        {
            out.insert(name.trim().to_string(), false);
        }
    }
    out
}

/// Check a definition that is not on a board (FEAT-052).
///
/// A contributor has no board access — the data folder is a separate repository — so the definition
/// travels with the pull request and CI validates it here. The rules are the same ones `check`
/// applies to an item: the goal link is the only thing that cannot be verified without a charter,
/// and it is reported as unverifiable rather than as absent.
fn check_definition_file(cli: &Cli, file: &str) -> anyhow::Result<()> {
    use kanbanr_core::models::FeatureDefinition;
    let raw = if file == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(file).map_err(|e| anyhow::anyhow!("could not read {file}: {e}"))?
    };
    let definition: FeatureDefinition = serde_yaml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("invalid definition (YAML or JSON expected): {e}"))?;

    if definition.goals.is_empty() {
        // Not a gap that can be judged here: without the charter there is nothing to resolve an id
        // against. Reported so a maintainer knows to check it, not counted against the contributor.
        println!("note: no goal link — the maintainer will check this against the charter");
    }
    // The same rules the board applies (FEAT-112), minus what needs a board to judge.
    let gaps: Vec<String> = kanbanr_core::readiness::evaluate_definition(
        Some(&definition),
        None,
        None,
        kanbanr_core::readiness::FILE,
    )
    .into_iter()
    .map(|g| g.message)
    .collect();

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "ok": gaps.is_empty(), "gaps": gaps }))?
        );
    } else if gaps.is_empty() {
        println!("✓ the definition meets the bar: a reason, requirements, and green evidence");
    } else {
        println!("✗ {} thing(s) to fix:", gaps.len());
        for gap in &gaps {
            println!("    {gap}");
        }
    }
    if gaps.is_empty() {
        Ok(())
    } else {
        // A non-zero exit is what makes this useful in CI.
        std::process::exit(1);
    }
}

/// What an item has not said, and what it cannot yet show (FEAT-051). The same rules `doctor`
/// applies, focused on one item and phrased as work left to do rather than as a complaint.
fn check_report(f: &kanbanr_core::FeatureItem) -> Value {
    // The engine every surface shares (FEAT-112); `check` and `finish` ask its CHECK list.
    let gaps: Vec<String> =
        kanbanr_core::readiness::evaluate(f, None, kanbanr_core::readiness::CHECK)
            .into_iter()
            .map(|g| g.message)
            .collect();
    json!({ "code": f.code, "title": f.title, "status": f.status, "gaps": gaps })
}

/// One feature, straight from the project payload.
fn get_feature(client: &Backend, p: &str, code: &str) -> anyhow::Result<kanbanr_core::FeatureItem> {
    get_project(client, p)?
        .features
        .into_iter()
        .find(|f| f.code == code)
        .ok_or_else(|| anyhow::anyhow!("feature '{code}' not found"))
}

fn get_project(client: &Backend, p: &str) -> anyhow::Result<Project> {
    let s = client.get(&format!("/projects/{p}"))?;
    Ok(serde_json::from_str(&s)?)
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    // Identity / setup / serve commands handled before constructing the store backend.
    match &cli.command {
        Command::Identity { name, email } => return run_identity(cli, name, email),
        Command::Init {
            name,
            description,
            author,
            email,
            no_hooks,
            force,
        } => {
            return run_init(
                cli,
                name.clone(),
                description.clone(),
                author.clone(),
                email.clone(),
                *no_hooks,
                *force,
            );
        }
        Command::SelfUpdate { check, tag } => {
            return self_update::run(*check, tag.as_deref(), cli.json);
        }
        Command::Hooks(cmd) => return run_hooks(cli, cmd),
        Command::Serve {
            bind,
            ui_dir,
            allow_writes,
        } => return run_serve(cli, bind.clone(), ui_dir.clone(), *allow_writes),
        Command::Open => return run_open(cli),
        Command::Where => return run_where(cli),
        // A definition in a file is checked on its own: no board, no project, no network. That is
        // what lets CI hold a contributor to the same bar as the maintainer (FEAT-052) — so it runs
        // before any board is opened, and never creates one (FEAT-103).
        Command::Check {
            file: Some(file), ..
        } => return check_definition_file(cli, file),
        _ => {}
    }

    // Nothing configured a board here and there is no legacy ./data: say so, rather than creating
    // one as a side effect of looking (FEAT-103). A read that made `./data/projects` left a stray
    // board in a project during a read-only setup interview. Hooks stay silent instead — they run
    // in every folder, and one without a board is simply not their business.
    if let Some(dir) = no_board_here(cli) {
        if runs_from_a_hook(&cli.command) {
            return Ok(());
        }
        anyhow::bail!(
            "no kanbanr board here: this folder has no .kanbanr marker, $KANBANR_DATA_DIR is not \
             set, and there is no {}.\nSet one up with `kanbanr init`, or point at an existing \
             board with --data-dir.",
            dir.display()
        );
    }
    let client = make_backend(cli);
    match &cli.command {
        Command::Identity { .. }
        | Command::Init { .. }
        | Command::Serve { .. }
        | Command::Open
        | Command::Where
        | Command::SelfUpdate { .. }
        | Command::Hooks(_) => {
            unreachable!()
        }
        Command::Sync => {
            let had = client.sync()?;
            if had {
                println!("synced: pushed local commits to remotes");
            } else {
                println!("nothing to sync (no local commits pending)");
            }
            Ok(())
        }
        Command::Whoami => {
            let resp = client.get("/auth/whoami")?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else {
                let v: Value = serde_json::from_str(&resp)?;
                println!(
                    "{} <{}>",
                    v["full_name"].as_str().unwrap_or(""),
                    v["email"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
        Command::Activity => run_activity(cli, &client),
        Command::Events(cmd) => run_events(cli, &client, cmd),
        Command::Remote(cmd) => run_remote(cli, &client, cmd),
        Command::Project(cmd) => run_project(cli, &client, cmd),
        Command::Feature(cmd) => run_feature(cli, &client, cmd),
        Command::Move {
            code,
            status,
            unapproved,
        } => {
            let p = require_project(cli)?;
            let mut body = json!({ "to": status });
            if let Some(reason) = unapproved {
                body["unapproved"] = json!(reason);
            }
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{code}/move"),
                Some(body),
            )?;
            print_write(cli, &resp, format!("{code} -> {}", field(&resp, "status")));
            print_gate_warnings(cli, &resp);
            Ok(())
        }
        Command::Test {
            code,
            requirement,
            test,
            state,
            rev,
        } => {
            let p = require_project(cli)?;
            let mut body = json!({ "state": state });
            if let Some(rev) = rev {
                body["checked_rev"] = json!(rev);
            }
            let resp = client.write(
                Method::Put,
                &format!(
                    "/projects/{p}/features/{code}/tests/{}/{}",
                    urlencode_segment(requirement),
                    urlencode_segment(test)
                ),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("{code}/{requirement} · {test} -> {state}"),
            );
            Ok(())
        }
        Command::Trace { subject, zachman } => {
            run_trace(cli, &client, subject.as_deref(), *zachman)
        }
        Command::Why { target } => run_why(cli, &client, target),
        Command::Claude(cmd) => run_claude(cli, &client, cmd),
        Command::Adr(cmd) => run_adr(cli, &client, cmd),
        Command::Lesson(cmd) => run_lesson(cli, &client, cmd),
        Command::Lessons { for_item, all } => {
            let p = require_project(cli)?;
            let mut query = Vec::new();
            if let Some(code) = for_item {
                query.push(format!("for={}", urlencode(code)));
            }
            if *all {
                query.push("all=1".to_string());
            }
            let resp = client.get(&format!(
                "/projects/{p}/lessons{}{}",
                if query.is_empty() { "" } else { "?" },
                query.join("&")
            ))?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let lessons: Vec<kanbanr_core::lessons::Lesson> = serde_json::from_str(&resp)?;
            if lessons.is_empty() {
                println!("(nothing learned here yet)");
            }
            for l in &lessons {
                print!("{}", lesson_line(l));
            }
            Ok(())
        }
        Command::Retro {
            milestone,
            since,
            label,
            sprint,
            write,
            due,
        } => run_retro(
            cli,
            &client,
            milestone.as_deref(),
            since.as_deref(),
            label.as_deref(),
            sprint.as_deref(),
            *write,
            *due,
        ),
        Command::SplitFrom { code, parent } => {
            let p = require_project(cli)?;
            let body = json!({ "parent": parent.clone().unwrap_or_default() });
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/features/{code}/split-from"),
                Some(body),
            )?;
            let line = match parent {
                Some(parent) => format!("{code} is recorded as split out of {parent}"),
                None => format!("{code} is no longer recorded as a split"),
            };
            print_write(cli, &resp, line);
            Ok(())
        }
        Command::Defect {
            code,
            severity,
            introduced_by,
            found_in,
            root_cause,
            escaped,
            fixed_by,
            clear,
        } => {
            let p = require_project(cli)?;
            let body = if *clear {
                Value::Null
            } else {
                // Start from what is already recorded, so one flag at a time is enough.
                let mut d = get_feature(&client, &p, code)?.defect.unwrap_or_default();
                let set = |field: &mut String, value: &Option<String>| {
                    if let Some(v) = value {
                        *field = v.clone();
                    }
                };
                set(&mut d.severity, severity);
                set(&mut d.introduced_by, introduced_by);
                set(&mut d.found_in, found_in);
                set(&mut d.root_cause, root_cause);
                set(&mut d.fixed_by, fixed_by);
                d.escaped = d.escaped || *escaped;
                serde_json::to_value(&d)?
            };
            let path = format!("/projects/{p}/features/{code}/defect");
            let resp = client.write(Method::Put, &path, Some(body))?;
            let f: kanbanr_core::FeatureItem = serde_json::from_str(&resp)?;
            let unset = |s: &String| {
                if s.is_empty() {
                    "unset".to_string()
                } else {
                    s.clone()
                }
            };
            let line = match &f.defect {
                None => format!("{code}: defect record cleared"),
                Some(d) => format!(
                    "{code}: severity {} · found in {} · {}",
                    unset(&d.severity),
                    unset(&d.found_in),
                    if d.escaped {
                        "escaped (the work it came from had already been called done)"
                    } else {
                        "caught before done"
                    }
                ),
            };
            print_write(cli, &resp, line);
            Ok(())
        }
        Command::Report { since } => {
            let p = require_project(cli)?;
            let since = since.as_deref().map(|s| match s.strip_suffix('d') {
                Some(days) => days
                    .parse::<i64>()
                    .map(kanbanr_core::report::days_ago)
                    .unwrap_or_else(|_| s.to_string()),
                None => s.to_string(),
            });
            let rev = project_head_rev();
            let mut query = Vec::new();
            if let Some(s) = &since {
                query.push(format!("since={}", urlencode(s)));
            }
            if let Some(r) = &rev {
                query.push(format!("rev={}", urlencode(r)));
            }
            let path = format!(
                "/projects/{p}/report{}{}",
                if query.is_empty() { "" } else { "?" },
                query.join("&")
            );
            let resp = client.get(&path)?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let r: Value = serde_json::from_str(&resp)?;
            let n = |k: &str| r[k].as_u64().unwrap_or(0);
            println!(
                "window: {}",
                r["since"].as_str().unwrap_or("everything on the board")
            );
            println!(
                "completed: {}   in progress: {}",
                n("completed"),
                n("in_progress")
            );
            match r["cycle_time_days"].as_object() {
                Some(ct) => println!(
                    "cycle time: p50 {}   p90 {}   max {}",
                    duration(ct["p50"].as_f64().unwrap_or(0.0)),
                    duration(ct["p90"].as_f64().unwrap_or(0.0)),
                    duration(ct["max"].as_f64().unwrap_or(0.0))
                ),
                None => println!("cycle time: no completed item has a recorded history yet"),
            }
            println!("rework (done -> reopened): {}", n("rework"));
            let (d, e) = (n("defects"), n("escaped_defects"));
            println!(
                "defects: {d}   escaped: {e}{}",
                if d > 0 {
                    format!(" ({:.0}% escape rate)", (e as f64 / d as f64) * 100.0)
                } else {
                    String::new()
                }
            );
            let cov = &r["requirement_coverage"];
            let (proven, total) = (
                cov["proven"].as_u64().unwrap_or(0),
                cov["total"].as_u64().unwrap_or(0),
            );
            println!(
                "requirements proven by a green test: {proven}/{total}{}",
                if total > 0 {
                    format!(" ({:.0}%)", (proven as f64 / total as f64) * 100.0)
                } else {
                    String::new()
                }
            );
            let stale: Vec<String> = r["stale_evidence"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if !stale.is_empty() {
                println!("{}", stale_line(&stale));
            }
            Ok(())
        }
        Command::Capture => run_capture(cli, &client),
        Command::Check { code, file } => {
            if let Some(file) = file {
                return check_definition_file(cli, file); // handled before the board opens
            }
            let p = require_project(cli)?;
            let project = get_project(&client, &p)?;
            let features: Vec<&kanbanr_core::FeatureItem> = match code {
                Some(c) => project
                    .features
                    .iter()
                    .filter(|f| &f.code == c)
                    .collect::<Vec<_>>(),
                None => project
                    .features
                    .iter()
                    .filter(|f| {
                        project
                            .config
                            .displayed_states
                            .iter()
                            .any(|s| s == &f.status)
                    })
                    .filter(|f| {
                        !kanbanr_core::graph::is_terminal_status(&project.config, &f.status)
                    })
                    .collect(),
            };
            if features.is_empty() {
                anyhow::bail!("no such item, or nothing in scope");
            }
            // What each item's next stage asks — the readiness route answers it, with the project's
            // unit and active sprint in hand (FEAT-117, FEAT-119).
            let reports: Vec<Value> = features
                .iter()
                .map(|f| {
                    let mut report = check_report(f);
                    report["next"] = client
                        .get(&format!("/projects/{p}/features/{}/readiness", f.code))
                        .ok()
                        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
                        .map(|v| v["next"].clone())
                        .unwrap_or_else(|| json!([]));
                    report
                })
                .collect();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
                return Ok(());
            }
            let mut ready = 0;
            for r in &reports {
                let gaps = r["gaps"].as_array().cloned().unwrap_or_default();
                if gaps.is_empty() {
                    ready += 1;
                    println!(
                        "✓ {} {}",
                        r["code"].as_str().unwrap_or(""),
                        r["title"].as_str().unwrap_or("")
                    );
                } else {
                    println!(
                        "✗ {} {}",
                        r["code"].as_str().unwrap_or(""),
                        r["title"].as_str().unwrap_or("")
                    );
                    for gap in gaps {
                        println!("    {}", gap.as_str().unwrap_or(""));
                    }
                }
                // The next stage: what it is for, and what moving on still needs.
                for next in r["next"].as_array().into_iter().flatten() {
                    let status = next["status"].as_str().unwrap_or("");
                    let lacks: Vec<&str> = next["gaps"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|g| g["message"].as_str())
                        .collect();
                    let purpose = next["purpose"].as_str().unwrap_or("");
                    let about = if purpose.is_empty() {
                        String::new()
                    } else {
                        format!(" ({purpose})")
                    };
                    if lacks.is_empty() {
                        println!("    → {status}{about}: ready to move on");
                    } else {
                        println!("    → to move to {status}{about}, still needed:");
                        for lack in lacks {
                            println!("        {lack}");
                        }
                    }
                }
            }
            println!("\n{ready}/{} ready", reports.len());
            Ok(())
        }
        Command::Review { code, pending, ui } => {
            use kanbanr_core::models::ApprovalState;
            let p = require_project(cli)?;
            if *ui {
                // Reading a page of prose belongs in something that renders prose. The daemon runs
                // with writes enabled so the button works, and stays on localhost.
                let bind =
                    std::env::var("KANBANR_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
                let url = format!("http://{bind}/p/{}/review", urlencode(&p));
                println!("opening {url} — approve there, or Ctrl-C to stop the daemon");
                let opened = url.clone();
                std::thread::spawn(move || {
                    // The daemon needs a moment to bind before the browser asks for the page.
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    open_in_browser(&opened);
                });
                return run_serve(cli, Some(bind), ui_dir_for_review(), true);
            }
            let features: Vec<kanbanr_core::FeatureItem> = match (code, pending) {
                (Some(code), _) => vec![get_feature(&client, &p, code)?],
                (None, true) => get_project(&client, &p)?
                    .features
                    .into_iter()
                    // Anything defined but not currently agreed: never approved, or lapsed because
                    // the definition changed after a yes.
                    .filter(|f| {
                        f.definition
                            .as_ref()
                            .is_some_and(|d| !matches!(d.approval_state(), ApprovalState::Current))
                    })
                    .collect(),
                (None, false) => anyhow::bail!(
                    "review what? an item code, or `--pending` for everything awaiting agreement"
                ),
            };
            if cli.json {
                let briefs: Vec<Value> = features
                    .iter()
                    .map(|f| {
                        json!({
                            "code": f.code,
                            "approval": f.definition.as_ref().map(|d| d.approval_state()),
                            "definition": f.definition,
                        })
                    })
                    .collect();
                // One item asked for, one object back: a caller that asked about FEAT-053 should
                // not have to unwrap a list of one.
                println!(
                    "{}",
                    serde_json::to_string_pretty(&if code.is_some() {
                        briefs.into_iter().next().unwrap_or(Value::Null)
                    } else {
                        Value::Array(briefs)
                    })?
                );
                return Ok(());
            }
            if features.is_empty() {
                println!("nothing is waiting for agreement");
                return Ok(());
            }
            for feature in &features {
                print!("{}", kanbanr_core::export::definition_brief(feature));
                println!();
            }
            if *pending {
                println!(
                    "{} item(s) awaiting agreement. Approve the ones you accept:\n\n    {}\n\n\
                     Leaving one unapproved is an answer. To change a definition first: \
                     `kanbanr feature define <CODE> --file …` — the approval stays off until you \
                     agree to the new version.",
                    features.len(),
                    features
                        .iter()
                        .map(|f| format!("kanbanr approve {}", f.code))
                        .collect::<Vec<_>>()
                        .join("\n    ")
                );
            }
            Ok(())
        }
        Command::Unapprove { code, reason, by } => {
            let p = require_project(cli)?;
            let who = verdict_author(cli, by.as_ref())?;
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{code}/unapprove"),
                Some(json!({ "by": who, "reason": reason })),
            )?;
            print_write(
                cli,
                &resp,
                format!(
                    "{code}: approval withdrawn by {who} — it is waiting for review again                      (`kanbanr review {code}`)"
                ),
            );
            Ok(())
        }
        Command::Ratify { code, reason, by } => {
            let p = require_project(cli)?;
            let who = verdict_author(cli, by.as_ref())?;
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{code}/ratify"),
                Some(json!({ "by": who, "reason": reason })),
            )?;
            print_write(
                cli,
                &resp,
                format!(
                    "{code} ratified by {who} — built under a recorded bypass, agreed to after the fact"
                ),
            );
            Ok(())
        }
        Command::Approve { code, by } => {
            let p = require_project(cli)?;
            let who = verdict_author(cli, by.as_ref())?;
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{code}/approve"),
                Some(json!({ "by": who })),
            )?;
            print_write(cli, &resp, format!("{code} approved by {who}"));
            Ok(())
        }
        Command::Signoff {
            code,
            name,
            note,
            doc,
            by,
        } => {
            let p = require_project(cli)?;
            let who = verdict_author(cli, by.as_ref())?;
            let resp = client.write(
                Method::Post,
                &format!(
                    "/projects/{p}/features/{code}/signoff/{}",
                    urlencode_segment(name)
                ),
                Some(json!({ "by": who, "note": note, "doc": doc })),
            )?;
            print_write(
                cli,
                &resp,
                format!("{code}: sign-off '{name}' recorded by {who}"),
            );
            Ok(())
        }
        Command::Sprint(cmd) => run_sprint(cli, &client, cmd),
        Command::Release(cmd) => run_release(cli, &client, cmd),
        Command::Todo(cmd) => run_todo(cli, &client, cmd),
        Command::Task(cmd) => run_task(cli, &client, cmd),
        Command::Milestone(cmd) => run_milestone(cli, &client, cmd),
        Command::Config(cmd) => run_config(cli, &client, cmd),
        Command::Doc(cmd) => run_doc(cli, &client, cmd),
        Command::Batch {
            file,
            message,
            dry_run,
        } => {
            let p = require_project(cli)?;
            let raw = match file {
                Some(path) => std::fs::read_to_string(path)?,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            let value: Value = serde_json::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("invalid JSON bundle: {e}"))?;
            // Accept either a bare array of operations or { "operations": [...] }.
            let mut body = if value.is_array() {
                json!({ "operations": value })
            } else {
                value
            };
            if let (Some(obj), Some(m)) = (body.as_object_mut(), message) {
                obj.insert("message".to_string(), json!(m));
            }
            fill_file_source_revisions(&mut body);
            let path = format!("/projects/{p}/batch");
            let resp = if *dry_run {
                body["dry_run"] = json!(true);
                client.preview(Method::Post, &path, Some(body))?
            } else {
                client.write(Method::Post, &path, Some(body))?
            };
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let v: Value = serde_json::from_str(&resp)?;
            let results = v["results"].as_array().cloned().unwrap_or_default();
            let skipped = results.iter().filter(|r| r["skipped"] == true).count();
            let applied = results.len() - skipped;
            if *dry_run {
                for r in &results {
                    println!("{}", describe_batch_result(r));
                }
                println!("dry run: nothing written ({applied} to apply, {skipped} skipped)");
            } else if skipped > 0 {
                println!("applied {applied} operation(s), skipped {skipped} (already present)");
            } else {
                println!("applied {applied} operation(s)");
            }
            Ok(())
        }
        Command::Git(cmd) => run_git(cli, &client, cmd),
        Command::Start {
            code,
            to,
            no_branch,
            unapproved,
        } => run_start(
            cli,
            &client,
            code,
            to.as_deref(),
            *no_branch,
            unapproved.as_deref(),
        ),
        Command::Finish { code } => run_finish(cli, &client, code.as_deref()),
        Command::Commit { message, all, refs } => run_commit(cli, &client, message, *all, refs),
        Command::Sources { write } => run_sources(cli, &client, *write),
        Command::Tests { write } => run_tests(cli, &client, *write),
        Command::Charter(cmd) => run_charter(cli, &client, cmd),
        Command::Mirror(cmd) => run_mirror(cli, &client, cmd),
        Command::Export { code, format } => {
            let p = require_project(cli)?;
            let resp = client.get(&format!(
                "/projects/{p}/features/{code}/export?format={format}"
            ))?;
            println!("{resp}");
            Ok(())
        }
        Command::Board => {
            let p = require_project(cli)?;
            print_board(&get_project(&client, &p)?);
            Ok(())
        }
        Command::Portfolio(cmd) => run_portfolio(cli, &client, cmd),
        Command::Ready { all_projects } => run_readiness(cli, &client, "ready", *all_projects),
        Command::Blocked { all_projects } => run_readiness(cli, &client, "blocked", *all_projects),
        Command::Graph {
            format,
            all_projects,
        } => {
            let path = if *all_projects {
                format!("/graph?format={format}")
            } else {
                let p = require_project(cli)?;
                format!("/projects/{p}/graph?format={format}")
            };
            // Both DOT (raw text) and JSON come back as a body string; print it verbatim, except
            // pretty-print JSON when --json is set.
            let resp = client.get(&path)?;
            if cli.json && format != "dot" {
                println!("{}", pretty(&resp));
            } else {
                println!("{resp}");
            }
            Ok(())
        }
        Command::Impact { code } => {
            let p = require_project(cli)?;
            let resp = client.get(&format!("/projects/{p}/features/{code}/impact"))?;
            print_id_list(cli, &resp, "(nothing depends on it)");
            Ok(())
        }
        Command::Gantt { all_projects } => {
            let path = if *all_projects {
                "/gantt".to_string()
            } else {
                let p = require_project(cli)?;
                format!("/projects/{p}/gantt")
            };
            // Mermaid text comes back as a body string; print it verbatim.
            println!("{}", client.get(&path)?);
            Ok(())
        }
        Command::CriticalPath { all_projects } => {
            let path = if *all_projects {
                "/critical-path".to_string()
            } else {
                let p = require_project(cli)?;
                format!("/projects/{p}/critical-path")
            };
            let resp = client.get(&path)?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else {
                // The critical path is the ordered list of qualified ids; surface it plainly.
                let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or(json!({}));
                let path_ids = v
                    .get("critical_path")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    })
                    .unwrap_or_default();
                if path_ids.is_empty() {
                    println!("(no critical path)");
                } else {
                    println!("{path_ids}");
                    if let Some(m) = v.get("makespan").and_then(|m| m.as_f64()) {
                        println!("makespan: {m} day(s)");
                    }
                }
            }
            Ok(())
        }
        Command::Doctor { all_projects } => run_doctor(cli, &client, *all_projects),
        Command::Index { all_projects } => run_index(cli, &client, *all_projects),
        Command::Query(args) => run_query(cli, &client, args),
    }
}

/// Search features with rich filters + full-text, in one project or across all. (FEAT-032)
fn run_query(cli: &Cli, client: &Backend, args: &QueryArgs) -> anyhow::Result<()> {
    // Assemble the query string from the set filters, percent-encoding each value.
    fn push(params: &mut Vec<String>, k: &str, v: &str) {
        params.push(format!("{k}={}", urlencode(v)));
    }
    let mut params: Vec<String> = Vec::new();
    if let Some(s) = &args.status {
        push(&mut params, "status", s);
    }
    if let Some(s) = &args.milestone {
        push(&mut params, "milestone", s);
    }
    if let Some(s) = &args.kind {
        push(&mut params, "kind", s);
    }
    if let Some(s) = &args.priority {
        push(&mut params, "priority", s);
    }
    if let Some(s) = &args.goal {
        push(&mut params, "goal", s);
    }
    if let Some(s) = &args.gap {
        push(&mut params, "gap", s);
    }
    if let Some(labels) = &args.label
        && !labels.is_empty()
    {
        push(&mut params, "label", &labels.join(","));
    }
    if let Some(s) = &args.assignee {
        push(&mut params, "assignee", s);
    }
    if let Some(s) = &args.team {
        push(&mut params, "team", s);
    }
    if let Some(s) = &args.due_before {
        push(&mut params, "due_before", s);
    }
    if let Some(s) = &args.due_after {
        push(&mut params, "due_after", s);
    }
    if args.ready {
        params.push("ready=1".to_string());
    }
    if args.blocked {
        params.push("blocked=1".to_string());
    }
    if let Some(s) = &args.text {
        push(&mut params, "text", s);
    }
    if args.full_text {
        params.push("full_text=1".to_string());
    }
    let qs = params.join("&");

    let path = if args.all_projects {
        format!("/query?{qs}")
    } else {
        let p = require_project(cli)?;
        format!("/projects/{p}/query?{qs}")
    };
    let resp = client.get(&path)?;
    if cli.json {
        println!("{}", pretty(&resp));
    } else if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&resp) {
        if arr.is_empty() {
            println!("(no matching features)");
        }
        for h in arr {
            println!(
                "{:<22} {:<12} {:<10} {}",
                h["id"].as_str().unwrap_or(""),
                h["status"].as_str().unwrap_or(""),
                h["milestone"].as_str().unwrap_or(""),
                h["title"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Rebuild the per-project index cache (FEAT-033). Maintenance write: rebuilds index.yaml from the
/// source-of-truth feature files for the selected project (or every project with --all-projects).
fn run_index(cli: &Cli, client: &Backend, all_projects: bool) -> anyhow::Result<()> {
    let ids = if all_projects {
        client.list_projects()?
    } else {
        vec![require_project(cli)?]
    };
    client.rebuild_index(&ids)?;
    if cli.json {
        println!("{}", json!({ "rebuilt": ids }));
    } else if ids.is_empty() {
        println!("(no projects)");
    } else {
        for id in &ids {
            println!("rebuilt index for {id}");
        }
    }
    Ok(())
}

/// Print a `ready`/`blocked` list (portfolio-wide or for the current project).
fn run_readiness(
    cli: &Cli,
    client: &Backend,
    which: &str,
    all_projects: bool,
) -> anyhow::Result<()> {
    let path = if all_projects {
        format!("/{which}")
    } else {
        let p = require_project(cli)?;
        format!("/projects/{p}/{which}")
    };
    let resp = client.get(&path)?;
    print_id_list(cli, &resp, &format!("(no {which} features)"));
    Ok(())
}

/// Print a JSON array of qualified ids as a list (or raw JSON under --json).
fn print_id_list(cli: &Cli, resp: &str, empty: &str) {
    if cli.json {
        println!("{}", pretty(resp));
    } else if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(resp) {
        if arr.is_empty() {
            println!("{empty}");
        }
        for id in arr {
            if let Some(s) = id.as_str() {
                println!("{s}");
            }
        }
    }
}

fn run_doctor(cli: &Cli, client: &Backend, all_projects: bool) -> anyhow::Result<()> {
    let resp = if all_projects {
        client.get("/doctor")?
    } else {
        let p = require_project(cli)?;
        client.get(&format!("/projects/{p}/doctor"))?
    };
    if cli.json {
        println!("{}", pretty(&resp));
        return Ok(());
    }
    let report: Value = serde_json::from_str(&resp)?;
    let issues = report["issues"].as_array().cloned().unwrap_or_default();
    let errors: Vec<&Value> = issues.iter().filter(|i| i["severity"] == "error").collect();
    let warnings: Vec<&Value> = issues
        .iter()
        .filter(|i| i["severity"] == "warning")
        .collect();
    let print_issue = |i: &Value| {
        let proj = i["project"].as_str().unwrap_or("");
        let code = i["code"]
            .as_str()
            .map(|c| format!(" {c}"))
            .unwrap_or_default();
        let msg = i["message"].as_str().unwrap_or("");
        println!("  {proj}{code}: {msg}");
    };
    if !errors.is_empty() {
        println!("Errors ({}):", errors.len());
        for i in &errors {
            print_issue(i);
        }
    }
    if !warnings.is_empty() {
        println!("Warnings ({}):", warnings.len());
        for i in &warnings {
            print_issue(i);
        }
    }
    if issues.is_empty() {
        println!("clean: no integrity issues found");
    }
    Ok(())
}

fn run_portfolio(cli: &Cli, client: &Backend, cmd: &PortfolioCmd) -> anyhow::Result<()> {
    match cmd {
        PortfolioCmd::Show => {
            let resp = client.get("/portfolio")?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let v: Value = serde_json::from_str(&resp)?;
            println!("{}", v["name"].as_str().unwrap_or("Portfolio"));
            for prog in v["programs"].as_array().cloned().unwrap_or_default() {
                let projects = prog["projects"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                println!(
                    "  {} [{}]: {projects}",
                    prog["name"].as_str().unwrap_or(""),
                    prog["id"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
        PortfolioCmd::Rollups => {
            let resp = client.get("/portfolio/rollups")?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let v: Value = serde_json::from_str(&resp)?;
            let pct = |c: &Value| c["percent"].as_u64().unwrap_or(0);
            println!(
                "{} — {}% ({}/{} tasks)",
                v["portfolio"].as_str().unwrap_or("Portfolio"),
                pct(&v["counts"]),
                v["counts"]["tasks_done"].as_u64().unwrap_or(0),
                v["counts"]["tasks_total"].as_u64().unwrap_or(0),
            );
            for prog in v["programs"].as_array().cloned().unwrap_or_default() {
                println!(
                    "  {} — {}%",
                    prog["name"].as_str().unwrap_or(""),
                    pct(&prog["counts"])
                );
                for proj in prog["projects"].as_array().cloned().unwrap_or_default() {
                    println!(
                        "    {} — {}%",
                        proj["name"].as_str().unwrap_or(""),
                        pct(&proj["counts"])
                    );
                }
            }
            Ok(())
        }
        PortfolioCmd::Board => {
            let resp = client.get("/portfolio/board")?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let v: Value = serde_json::from_str(&resp)?;
            for lane in v["lanes"].as_array().cloned().unwrap_or_default() {
                let cards = lane["cards"].as_array().cloned().unwrap_or_default();
                println!(
                    "[{}] ({})",
                    lane["disposition"].as_str().unwrap_or(""),
                    cards.len()
                );
                for c in cards {
                    println!(
                        "  {}:{} {} ({})",
                        c["project"].as_str().unwrap_or(""),
                        c["code"].as_str().unwrap_or(""),
                        c["title"].as_str().unwrap_or(""),
                        c["status"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        PortfolioCmd::AddProgram {
            id,
            name,
            description,
            projects,
        } => {
            let project_list: Vec<&str> = projects
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .map(|p| p.trim())
                        .filter(|p| !p.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let mut body = Map::new();
            body.insert("id".into(), json!(id));
            body.insert("projects".into(), json!(project_list));
            if let Some(n) = name {
                body.insert("name".into(), json!(n));
            }
            if let Some(d) = description {
                body.insert("description".into(), json!(d));
            }
            let resp = client.write(
                Method::Post,
                "/portfolio/programs",
                Some(Value::Object(body)),
            )?;
            print_write(cli, &resp, format!("program {id} saved"));
            Ok(())
        }
    }
}

fn print_write(cli: &Cli, resp: &str, human: String) {
    if cli.json {
        println!("{}", pretty(resp));
    } else {
        println!("{human}");
    }
}

fn run_project(cli: &Cli, client: &Backend, cmd: &ProjectCmd) -> anyhow::Result<()> {
    match cmd {
        ProjectCmd::Init {
            name,
            description,
            statuses,
            displayed_states,
            default_state,
            no_op_states,
            workflow,
        } => {
            let body = obj(vec![
                ("name", Some(json!(name))),
                ("description", description.clone().map(|d| json!(d))),
                ("workflow", workflow.clone().map(|w| json!(w))),
                ("statuses", statuses.clone().map(|s| json!(s))),
                (
                    "displayed_states",
                    displayed_states.clone().map(|s| json!(s)),
                ),
                ("default_state", default_state.clone().map(|s| json!(s))),
                ("no_op_states", no_op_states.clone().map(|s| json!(s))),
            ]);
            let resp = client.write(Method::Post, "/projects", Some(body))?;
            print_write(cli, &resp, format!("created project '{name}'"));
            Ok(())
        }
        ProjectCmd::Edit {
            name,
            new_name,
            description,
        } => {
            let body = obj(vec![
                ("name", new_name.clone().map(|n| json!(n))),
                ("description", description.clone().map(|d| json!(d))),
            ]);
            let resp = client.write(Method::Patch, &format!("/projects/{name}"), Some(body))?;
            print_write(cli, &resp, format!("updated project '{name}'"));
            Ok(())
        }
        ProjectCmd::List => {
            let resp = client.get("/projects")?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else {
                let v: Value = serde_json::from_str(&resp)?;
                if let Some(arr) = v.as_array() {
                    if arr.is_empty() {
                        println!("(no projects)");
                    }
                    for p in arr {
                        println!(
                            "{:<20} {}",
                            p["id"].as_str().unwrap_or(""),
                            p["description"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            Ok(())
        }
        ProjectCmd::Delete { name } => {
            client.write(Method::Delete, &format!("/projects/{name}"), None)?;
            println!("deleted project '{name}'");
            Ok(())
        }
        ProjectCmd::Use { name } => {
            // Keep (or record, with --data-dir) where the board lives alongside the project name.
            let cwd = std::env::current_dir()?;
            let resolved = project::resolve_data_dir_detailed(cli.data_dir.as_deref());
            let data_dir = matches!(resolved.source, DataDirSource::Flag | DataDirSource::Marker)
                .then(|| marker_path(&resolved.path, &cwd));
            project::write_marker(
                &cwd,
                &Marker {
                    project: Some(name.clone()),
                    data_dir,
                },
            )?;
            println!("selected project '{name}' (wrote .kanbanr)");
            Ok(())
        }
        ProjectCmd::Export { format } => {
            let p = require_project(cli)?;
            let resp = client.get(&format!("/projects/{p}/export?format={format}"))?;
            println!("{resp}");
            Ok(())
        }
    }
}

fn run_feature(cli: &Cli, client: &Backend, cmd: &FeatureCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        FeatureCmd::Add {
            title,
            milestone,
            spec,
            spec_file,
            code,
            kind,
            priority,
            start,
            due,
            estimate,
            points,
            found_in,
            assignee,
            team,
            labels,
            depends_on,
        } => {
            let spec = read_spec(spec.clone(), spec_file.clone())?;
            let body = obj(vec![
                ("title", Some(json!(title))),
                ("milestone", Some(json!(milestone))),
                ("specification", spec.map(|s| json!(s))),
                ("code", code.clone().map(|c| json!(c))),
                ("kind", kind.clone().map(|k| json!(k))),
                ("priority", priority.clone().map(|x| json!(x))),
                ("start", start.clone().map(|s| json!(s))),
                ("due", due.clone().map(|d| json!(d))),
                ("estimate_days", estimate.map(|e| json!(e))),
                ("points", points.map(|e| json!(e))),
                ("assignee", assignee.clone().map(|a| json!(a))),
                ("team", team.clone().map(|t| json!(t))),
                ("labels", labels.clone().map(|l| json!(l))),
                ("depends_on", depends_on.clone().map(|d| json!(d))),
            ]);
            let resp =
                client.write(Method::Post, &format!("/projects/{p}/features"), Some(body))?;
            // Feedback on a release: say where it was found, the way a defect does (FEAT-120).
            if let Some(version) = found_in {
                client.write(
                    Method::Put,
                    &format!("/projects/{p}/features/{}/defect", field(&resp, "code")),
                    Some(json!({ "found_in": version })),
                )?;
            }
            print_write(
                cli,
                &resp,
                format!(
                    "created {} ({})",
                    field(&resp, "code"),
                    field(&resp, "status")
                ),
            );
            Ok(())
        }
        FeatureCmd::List {
            status,
            milestone,
            assignee,
            team,
        } => {
            let project = get_project(client, &p)?;
            let items: Vec<_> = project
                .features
                .iter()
                .filter(|f| status.as_ref().map(|s| &f.status == s).unwrap_or(true))
                .filter(|f| {
                    milestone
                        .as_ref()
                        .map(|m| &f.milestone == m)
                        .unwrap_or(true)
                })
                .filter(|f| {
                    assignee
                        .as_ref()
                        .map(|a| f.assignee.as_deref() == Some(a.as_str()))
                        .unwrap_or(true)
                })
                .filter(|f| {
                    team.as_ref()
                        .map(|t| f.team.as_deref() == Some(t.as_str()))
                        .unwrap_or(true)
                })
                .collect();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for f in items {
                    println!(
                        "{:<12} {:<10} {:<10} {}/{}  {}",
                        f.code,
                        f.status,
                        f.milestone,
                        f.done_count(),
                        f.task_count(),
                        f.title
                    );
                }
            }
            Ok(())
        }
        FeatureCmd::Define {
            code,
            file,
            clear,
            template,
            kind,
        } => {
            if *template {
                print!(
                    "{}",
                    definition_template(kind.as_deref().unwrap_or("feature"))
                );
                return Ok(());
            }
            let path = format!("/projects/{p}/features/{code}/definition");
            let body = if *clear {
                Value::Null
            } else {
                let raw = match file {
                    Some(path) => std::fs::read_to_string(path)?,
                    None => {
                        use std::io::Read;
                        let mut s = String::new();
                        std::io::stdin().read_to_string(&mut s)?;
                        s
                    }
                };
                // YAML is a superset of JSON, so one parser accepts either form.
                let def: kanbanr_core::models::FeatureDefinition = serde_yaml::from_str(&raw)
                    .map_err(|e| {
                        anyhow::anyhow!("invalid definition (YAML or JSON expected): {e}")
                    })?;
                serde_json::to_value(def)?
            };
            let resp = client.write(Method::Put, &path, Some(body))?;
            if cli.json {
                println!("{}", pretty(&resp));
            } else if *clear {
                println!("cleared the definition of {code}");
            } else {
                let f: kanbanr_core::FeatureItem = serde_json::from_str(&resp)?;
                let def = f.definition.as_ref();
                let reqs = def.map(|d| d.requirements.len()).unwrap_or(0);
                let missing = def.map(|d| d.zachman.missing()).unwrap_or_default();
                println!(
                    "defined {code}: {reqs} requirement(s){}",
                    if missing.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; gaps: {}",
                            missing
                                .iter()
                                .map(|c| format!("[MISSING: {c}]"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    }
                );
            }
            Ok(())
        }
        FeatureCmd::Show { code } => {
            let resp = client.get(&format!("/projects/{p}/features/{code}/export?format=md"))?;
            println!("{resp}");
            Ok(())
        }
        FeatureCmd::Edit {
            code,
            title,
            spec,
            spec_file,
            new_code,
            milestone,
            kind,
            priority,
            start,
            due,
            estimate,
            points,
            assignee,
            team,
            labels,
            depends_on,
        } => {
            let spec = read_spec(spec.clone(), spec_file.clone())?;
            let body = obj(vec![
                ("title", title.clone().map(|t| json!(t))),
                ("specification", spec.map(|s| json!(s))),
                ("milestone", milestone.clone().map(|m| json!(m))),
                ("new_code", new_code.clone().map(|c| json!(c))),
                ("kind", kind.clone().map(|k| json!(k))),
                ("priority", priority.clone().map(|x| json!(x))),
                ("start", start.clone().map(|s| json!(s))),
                ("due", due.clone().map(|d| json!(d))),
                ("estimate_days", estimate.map(|e| json!(e))),
                ("points", points.map(|e| json!(e))),
                ("assignee", assignee.clone().map(|a| json!(a))),
                ("team", team.clone().map(|t| json!(t))),
                ("labels", labels.clone().map(|l| json!(l))),
                ("depends_on", depends_on.clone().map(|d| json!(d))),
            ]);
            let resp = client.write(
                Method::Patch,
                &format!("/projects/{p}/features/{code}"),
                Some(body),
            )?;
            print_write(cli, &resp, format!("updated {}", field(&resp, "code")));
            Ok(())
        }
    }
}

fn run_todo(cli: &Cli, client: &Backend, cmd: &TodoCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        TodoCmd::Add {
            feature,
            description,
            code,
        } => {
            let body = obj(vec![
                ("description", description.clone().map(|d| json!(d))),
                ("code", code.clone().map(|c| json!(c))),
            ]);
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{feature}/todos"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("added todo-list {} to {}", field(&resp, "code"), feature),
            );
            Ok(())
        }
        TodoCmd::List { feature } => {
            let project = get_project(client, &p)?;
            let f = project.feature(feature)?;
            let mut lists: Vec<_> = f.todo_lists.iter().collect();
            lists.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.code.cmp(&a.code)));
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&lists)?);
            } else {
                for l in lists {
                    println!(
                        "{:<10} {}/{}  {}",
                        l.code,
                        l.done_count(),
                        l.tasks.len(),
                        l.description
                    );
                }
            }
            Ok(())
        }
    }
}

fn run_task(cli: &Cli, client: &Backend, cmd: &TaskCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        TaskCmd::Add {
            feature,
            todo,
            text,
            key,
        } => {
            let body = obj(vec![
                ("text", Some(json!(text))),
                ("key", key.clone().map(|k| json!(k))),
            ]);
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{feature}/todos/{todo}/tasks"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("added task {} to {}/{}", field(&resp, "key"), feature, todo),
            );
            Ok(())
        }
        TaskCmd::State {
            feature,
            todo,
            key,
            state,
        } => {
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/features/{feature}/todos/{todo}/tasks/{key}"),
                Some(json!({ "state": state })),
            )?;
            print_write(
                cli,
                &resp,
                format!(
                    "{feature}/{todo} {key} -> {state} (feature now {})",
                    field(&resp, "status")
                ),
            );
            if !cli.json
                && let Some(held) = serde_json::from_str::<Value>(&resp)
                    .ok()
                    .and_then(|v| v["auto_advance_held"].as_str().map(str::to_string))
            {
                eprintln!("  {held}");
            }
            Ok(())
        }
        TaskCmd::List { feature, todo } => {
            let project = get_project(client, &p)?;
            let f = project.feature(feature)?;
            let lists: Vec<_> = f
                .todo_lists
                .iter()
                .filter(|l| todo.as_ref().map(|c| &l.code == c).unwrap_or(true))
                .collect();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&lists)?);
            } else {
                for l in lists {
                    println!("{} ({})", l.code, l.description);
                    for t in &l.tasks {
                        println!("  {:<8} {:<12?} {}", t.key, t.state, t.text);
                    }
                }
            }
            Ok(())
        }
    }
}

fn run_milestone(cli: &Cli, client: &Backend, cmd: &MilestoneCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        MilestoneCmd::Add {
            name,
            description,
            depends_on,
            code,
        } => {
            let body = obj(vec![
                ("name", Some(json!(name))),
                ("description", description.clone().map(|d| json!(d))),
                ("depends_on", depends_on.clone().map(|d| json!(d))),
                ("code", code.clone().map(|c| json!(c))),
            ]);
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/milestones"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("created milestone {}", field(&resp, "code")),
            );
            Ok(())
        }
        MilestoneCmd::List => {
            let project = get_project(client, &p)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&project.milestones)?);
            } else {
                for m in &project.milestones {
                    let deps = if m.depends_on.is_empty() {
                        String::new()
                    } else {
                        format!("  depends_on: {}", m.depends_on.join(", "))
                    };
                    println!("{:<10} {}{}", m.code, m.name, deps);
                }
            }
            Ok(())
        }
        MilestoneCmd::Edit {
            code,
            name,
            description,
            depends_on,
        } => {
            let body = obj(vec![
                ("name", name.clone().map(|n| json!(n))),
                ("description", description.clone().map(|d| json!(d))),
                ("depends_on", depends_on.clone().map(|d| json!(d))),
            ]);
            let resp = client.write(
                Method::Patch,
                &format!("/projects/{p}/milestones/{code}"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("updated milestone {}", field(&resp, "code")),
            );
            Ok(())
        }
        MilestoneCmd::Delete { code } => {
            client.write(
                Method::Delete,
                &format!("/projects/{p}/milestones/{code}"),
                None,
            )?;
            println!("deleted milestone {code}");
            Ok(())
        }
    }
}

fn run_config(cli: &Cli, client: &Backend, cmd: &ConfigCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        ConfigCmd::Cadence {
            sprints,
            releases,
            unit,
            sprint_length,
            release,
        } => {
            let body = obj(vec![
                ("sprints", sprints.as_deref().map(|v| json!(v == "on"))),
                ("releases", releases.as_deref().map(|v| json!(v == "on"))),
                ("estimate_unit", unit.clone().map(|u| json!(u))),
                ("sprint_length_days", sprint_length.map(|d| json!(d))),
                ("release", release.clone().map(|r| json!(r))),
            ]);
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/config/cadence"),
                Some(body),
            )?;
            let v: Value = serde_json::from_str(&resp)?;
            let c = &v["cadence"];
            print_write(
                cli,
                &resp,
                format!(
                    "sprints {}, releases {}, estimates in {}",
                    if c["sprints"].as_bool().unwrap_or(false) {
                        "on"
                    } else {
                        "off"
                    },
                    if c["releases"].as_bool().unwrap_or(false) {
                        "on"
                    } else {
                        "off"
                    },
                    v["estimate_unit"].as_str().unwrap_or("days")
                ),
            );
            Ok(())
        }
        ConfigCmd::Show => {
            let project = get_project(client, &p)?;
            let c = &project.config;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(c)?);
            } else {
                println!("name: {}", c.name);
                println!("description: {}", c.description);
                println!("statuses: {}", c.statuses.join(", "));
                println!("default_state: {}", c.default_status());
                println!("displayed_states: {}", c.displayed_states.join(", "));
                println!("no_op_states: {}", c.no_op_states.join(", "));
                println!("terminal_states: {}", c.terminal_states.join(", "));
                println!("transitions:");
                for (from, tos) in &c.transitions {
                    println!("  {from} -> {}", tos.join(", "));
                }
            }
            Ok(())
        }
        ConfigCmd::SetTransition(args) => {
            if args.allow == args.deny {
                anyhow::bail!("pass exactly one of --allow or --deny");
            }
            let body = json!({ "from": args.from, "to": args.to, "allow": args.allow });
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/config/transition"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!(
                    "{} {} -> {}",
                    if args.allow { "allowed" } else { "denied" },
                    args.from,
                    args.to
                ),
            );
            Ok(())
        }
        ConfigCmd::DisplayedStates { states } => {
            let body = json!({ "states": states });
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/config/displayed-states"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("displayed_states: {}", states.join(", ")),
            );
            Ok(())
        }
        ConfigCmd::DefaultState { state } => {
            let body = json!({ "state": state });
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/config/default-state"),
                Some(body),
            )?;
            print_write(cli, &resp, format!("default_state: {state}"));
            Ok(())
        }
        ConfigCmd::RenameStatus { from, to } => {
            let body = json!({ "old": from, "new": to });
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/config/rename-status"),
                Some(body),
            )?;
            print_write(
                cli,
                &resp,
                format!("renamed status '{from}' -> '{to}' (features migrated)"),
            );
            Ok(())
        }
        ConfigCmd::NoOpStates { states } => {
            let body = json!({ "states": states });
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/config/no-op-states"),
                Some(body),
            )?;
            print_write(cli, &resp, format!("no_op_states: {}", states.join(", ")));
            Ok(())
        }
        ConfigCmd::Workflow {
            preset,
            from_file,
            export,
            agreement,
            write_agreement,
            defaults,
            togaf,
            statuses,
            transitions,
            default_state,
            displayed_states,
            no_op_states,
            terminal_states,
            to_mermaid,
            from_mermaid,
        } => {
            if preset.as_deref() == Some("list") {
                for (name, about) in kanbanr_core::config::presets() {
                    println!("{name:<15} {about}");
                }
                return Ok(());
            }
            // The working agreement, generated from the gates (FEAT-122).
            if *agreement || *write_agreement {
                let project = get_project(client, &p)?;
                let text = kanbanr_core::config::working_agreement(&p, &project.config);
                if *write_agreement {
                    let resp = client.write(
                        Method::Put,
                        &format!("/projects/{p}/docs/content"),
                        Some(json!({ "path": "process/working-agreement.md", "content": text })),
                    )?;
                    print_write(
                        cli,
                        &resp,
                        "wrote process/working-agreement.md from the gates".to_string(),
                    );
                } else {
                    print!("{text}");
                }
                return Ok(());
            }
            // Export: the workflow as a file another project, or `--from-file`, can take (a read).
            if *export {
                let project = get_project(client, &p)?;
                let file = kanbanr_core::config::WorkflowFile::from_config(&project.config);
                print!("{}", serde_yaml::to_string(&file)?);
                return Ok(());
            }
            // Import a whole workflow, gates included, from a file.
            if let Some(src) = from_file {
                let text = std::fs::read_to_string(src)
                    .map_err(|e| anyhow::anyhow!("could not read {src}: {e}"))?;
                let file: kanbanr_core::config::WorkflowFile = serde_yaml::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("{src} is not a workflow file: {e}"))?;
                let resp = client.write(
                    Method::Put,
                    &format!("/projects/{p}/config/workflow"),
                    Some(json!({
                        "statuses": file.statuses,
                        "transitions": file.transitions,
                        "default_state": file.default_state,
                        "displayed_states": file.displayed_states,
                        "no_op_states": file.no_op_states,
                        "terminal_states": file.terminal_states,
                        "gates": file.gates,
                        "cadence": (!file.cadence.is_off()).then_some(&file.cadence),
                        "estimate_unit": (!file.estimate_unit.is_days()).then_some(file.estimate_unit),
                    })),
                )?;
                print_write(cli, &resp, format!("workflow loaded from {src}"));
                return Ok(());
            }
            // Export: print the workflow as a Mermaid state diagram (a read).
            if *to_mermaid {
                let diagram = client.get(&format!("/projects/{p}/workflow?format=mermaid"))?;
                print!("{diagram}");
                return Ok(());
            }
            // Import: parse a Mermaid diagram and feed its fields into the workflow write path.
            if let Some(src) = from_mermaid {
                let text = if src == "-" {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                } else {
                    std::fs::read_to_string(src)?
                };
                let def = kanbanr_core::mermaid::parse_state_diagram(&text)
                    .map_err(|e| anyhow::anyhow!(e))?;
                let body = obj(vec![
                    ("statuses", Some(json!(def.statuses))),
                    ("transitions", Some(json!(def.transitions))),
                    ("default_state", def.default_state.map(|s| json!(s))),
                    ("terminal_states", Some(json!(def.terminal_states))),
                ]);
                let resp = client.write(
                    Method::Put,
                    &format!("/projects/{p}/config/workflow"),
                    Some(body),
                )?;
                print_write(cli, &resp, "workflow imported from Mermaid".to_string());
                return Ok(());
            }
            // Parse "From>To" pairs into a { from: [to, ...] } map.
            let mut tmap: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            if let Some(pairs) = transitions {
                for pair in pairs {
                    let (from, to) = pair.split_once('>').ok_or_else(|| {
                        anyhow::anyhow!("bad transition '{pair}', expected From>To")
                    })?;
                    tmap.entry(from.trim().to_string())
                        .or_default()
                        .push(to.trim().to_string());
                }
            }
            let body = obj(vec![
                ("preset", preset.clone().map(|s| json!(s))),
                ("defaults", Some(json!(defaults))),
                ("togaf", Some(json!(togaf))),
                ("statuses", statuses.clone().map(|s| json!(s))),
                ("transitions", transitions.as_ref().map(|_| json!(tmap))),
                ("default_state", default_state.clone().map(|s| json!(s))),
                (
                    "displayed_states",
                    displayed_states.clone().map(|s| json!(s)),
                ),
                ("no_op_states", no_op_states.clone().map(|s| json!(s))),
                ("terminal_states", terminal_states.clone().map(|s| json!(s))),
            ]);
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/config/workflow"),
                Some(body),
            )?;
            print_write(cli, &resp, "workflow updated".to_string());
            Ok(())
        }
    }
}

/// `2w`, `10d` or a bare number of days.
fn parse_length(s: &str) -> anyhow::Result<u32> {
    let s = s.trim();
    let (n, per) = match s.strip_suffix('w') {
        Some(n) => (n, 7),
        None => (s.strip_suffix('d').unwrap_or(s), 1),
    };
    n.parse::<u32>()
        .map(|n| n * per)
        .map_err(|_| anyhow::anyhow!("'{s}' is not a length (e.g. 2w or 10d)"))
}

/// A number as a person writes it: `13`, not `13.0`.
fn num(v: &Value) -> String {
    match v.as_f64() {
        Some(x) if x.fract() == 0.0 => format!("{}", x as i64),
        Some(x) => format!("{x}"),
        None => v.to_string(),
    }
}

fn run_release(cli: &Cli, client: &Backend, cmd: &ReleaseCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        ReleaseCmd::Add {
            version,
            target,
            name,
        } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/releases"),
                Some(obj(vec![
                    ("version", Some(json!(version))),
                    ("target", target.clone().map(|t| json!(t))),
                    ("name", name.clone().map(|n| json!(n))),
                ])),
            )?;
            print_write(cli, &resp, format!("added release {version}"));
        }
        ReleaseCmd::List => {
            let resp = client.get(&format!("/projects/{p}/releases"))?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let releases: Vec<Value> = serde_json::from_str(&resp)?;
            if releases.is_empty() {
                println!("no releases yet — `kanbanr release add v0.1.0 --target YYYY-MM-DD`");
            }
            let project = get_project(client, &p)?;
            for r in releases {
                let version = r["version"].as_str().unwrap_or("");
                let planned = project
                    .features
                    .iter()
                    .filter(|f| f.release.as_deref() == Some(version))
                    .count();
                println!(
                    "{:<12} {:<8} target {:<10}  {planned} item(s)",
                    version,
                    r["state"].as_str().unwrap_or(""),
                    r["target"].as_str().unwrap_or("—")
                );
            }
        }
        ReleaseCmd::Plan { version, items } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/releases/{}/plan", urlencode_segment(version)),
                Some(json!({ "items": items })),
            )?;
            print_write(
                cli,
                &resp,
                format!("planned {} into {version}", items.join(", ")),
            );
        }
        ReleaseCmd::Cut { version, tag } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/releases/{}/cut", urlencode_segment(version)),
                Some(json!({})),
            )?;
            let v: Value = serde_json::from_str(&resp)?;
            let release = &v["release"];
            let shipped: Vec<&str> = release["shipped"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str())
                .collect();
            print_write(
                cli,
                &resp,
                format!(
                    "{version} shipped {} item(s); notes in {}",
                    shipped.len(),
                    release["notes_doc"].as_str().unwrap_or("")
                ),
            );
            if !cli.json {
                for c in release["carried"].as_array().into_iter().flatten() {
                    println!(
                        "  carried {} → {} ({})",
                        c["code"].as_str().unwrap_or(""),
                        c["to"].as_str().unwrap_or(""),
                        c["why"].as_str().unwrap_or("")
                    );
                }
            }
            if *tag {
                let root = scm::repo_root()
                    .ok_or_else(|| anyhow::anyhow!("not a git repository — nothing to tag"))?;
                scm::git(
                    &root,
                    &["tag", "-a", version, "-m", &format!("Release {version}")],
                )?;
                println!("tagged {version}");
            }
        }
    }
    Ok(())
}

fn run_sprint(cli: &Cli, client: &Backend, cmd: &SprintCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        SprintCmd::Add {
            start,
            length,
            goal,
            capacity,
            name,
        } => {
            let length = length.as_deref().map(parse_length).transpose()?;
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/sprints"),
                Some(obj(vec![
                    ("start", Some(json!(start))),
                    ("length_days", length.map(|d| json!(d))),
                    ("goal", goal.clone().map(|g| json!(g))),
                    ("capacity", capacity.map(|c| json!(c))),
                    ("name", name.clone().map(|n| json!(n))),
                ])),
            )?;
            let v: Value = serde_json::from_str(&resp)?;
            print_write(
                cli,
                &resp,
                format!(
                    "added {} ({} → {})",
                    v["code"].as_str().unwrap_or(""),
                    v["start"].as_str().unwrap_or(""),
                    v["end"].as_str().unwrap_or("")
                ),
            );
        }
        SprintCmd::List => {
            let resp = client.get(&format!("/projects/{p}/sprints"))?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let sprints: Vec<Value> = serde_json::from_str(&resp)?;
            if sprints.is_empty() {
                println!("no sprints yet — `kanbanr sprint add --start YYYY-MM-DD`");
            }
            for s in sprints {
                println!(
                    "{:<8} {:<8} {} → {}  {}",
                    s["code"].as_str().unwrap_or(""),
                    s["state"].as_str().unwrap_or(""),
                    s["start"].as_str().unwrap_or(""),
                    s["end"].as_str().unwrap_or(""),
                    s["goal"].as_str().unwrap_or("")
                );
            }
        }
        SprintCmd::Show { code } => {
            let code = code.as_deref().unwrap_or("active");
            let resp = client.get(&format!("/projects/{p}/sprints/{code}"))?;
            if cli.json {
                println!("{}", pretty(&resp));
                return Ok(());
            }
            let r: Value = serde_json::from_str(&resp)?;
            let unit = r["unit"].as_str().unwrap_or("");
            println!(
                "{} {} [{}]  {} → {}",
                r["code"].as_str().unwrap_or(""),
                r["name"].as_str().unwrap_or(""),
                r["state"].as_str().unwrap_or(""),
                r["start"].as_str().unwrap_or(""),
                r["end"].as_str().unwrap_or("")
            );
            if let Some(goal) = r["goal"].as_str().filter(|g| !g.is_empty()) {
                println!("goal: {goal}");
            }
            println!(
                "{} of {} {unit} done · {} day(s) left{}",
                num(&r["done"]),
                num(&r["committed"]),
                r["days_left"],
                r["capacity"]
                    .as_f64()
                    .map(|c| format!(" · capacity {c}"))
                    .unwrap_or_default()
            );
            let unestimated: Vec<&str> = r["unestimated"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str())
                .collect();
            if !unestimated.is_empty() {
                println!("unestimated (counted as 0): {}", unestimated.join(", "));
            }
            let days: Vec<(String, f64)> = r["burndown"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    (
                        d["date"].as_str().unwrap_or("").to_string(),
                        d["remaining"].as_f64().unwrap_or(0.0),
                    )
                })
                .collect();
            let top = r["committed"].as_f64().unwrap_or(0.0).max(1.0);
            if !days.is_empty() {
                println!("burndown ({unit} remaining):");
            }
            for (date, remaining) in days {
                let bar = "█".repeat(((remaining / top) * 30.0).round() as usize);
                println!("  {date}  {bar} {}", num(&json!(remaining)));
            }
        }
        SprintCmd::Plan { code, items } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/sprints/{code}/plan"),
                Some(json!({ "items": items })),
            )?;
            let v: Value = serde_json::from_str(&resp)?;
            print_write(
                cli,
                &resp,
                format!(
                    "planned {} into {code} ({} committed)",
                    items.join(", "),
                    num(&v["committed"])
                ),
            );
            if !cli.json
                && let Some(over) = v["over_capacity"].as_str()
            {
                eprintln!("  warning: {over}");
            }
        }
        SprintCmd::Start { code } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/sprints/{code}/start"),
                Some(json!({})),
            )?;
            print_write(cli, &resp, format!("{code} is the active sprint"));
        }
        SprintCmd::Close { code, carry_to } => {
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/sprints/{code}/close"),
                Some(obj(vec![("carry_to", carry_to.clone().map(|c| json!(c)))])),
            )?;
            let v: Value = serde_json::from_str(&resp)?;
            let carried: Vec<String> = v["carried"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    format!(
                        "{} → {}",
                        c["code"].as_str().unwrap_or(""),
                        c["to"].as_str().unwrap_or("")
                    )
                })
                .collect();
            print_write(
                cli,
                &resp,
                if carried.is_empty() {
                    format!("closed {code}; nothing carried over")
                } else {
                    format!("closed {code}; carried {}", carried.join(", "))
                },
            );
            if !cli.json {
                println!("  look back on it: `kanbanr retro --sprint {code}`");
            }
        }
    }
    Ok(())
}

fn run_doc(cli: &Cli, client: &Backend, cmd: &DocCmd) -> anyhow::Result<()> {
    let p = require_project(cli)?;
    match cmd {
        DocCmd::Add {
            path,
            file,
            content,
        } => {
            // A `--file` whose bytes aren't UTF-8 (e.g. an image) is stored as a binary asset; text
            // (and `--content`) goes through the normal markdown path.
            if let Some(f) = file {
                let bytes = std::fs::read(f)?;
                if std::str::from_utf8(&bytes).is_err() {
                    let saved = client.write_doc_asset(&p, path, &bytes)?;
                    println!("stored asset {saved} ({} bytes)", bytes.len());
                    return Ok(());
                }
                let resp = client.write(
                    Method::Put,
                    &format!("/projects/{p}/docs/content"),
                    Some(json!({ "path": path, "content": String::from_utf8_lossy(&bytes) })),
                )?;
                print_write(cli, &resp, format!("wrote doc {}", field(&resp, "path")));
                return Ok(());
            }
            let resp = client.write(
                Method::Put,
                &format!("/projects/{p}/docs/content"),
                Some(json!({ "path": path, "content": content.clone().unwrap_or_default() })),
            )?;
            print_write(cli, &resp, format!("wrote doc {}", field(&resp, "path")));
            Ok(())
        }
        DocCmd::List => {
            let tree: DocFolder =
                serde_json::from_str(&client.get(&format!("/projects/{p}/docs"))?)?;
            let mut files = Vec::new();
            collect_docs(&tree, &mut files);
            files.sort();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else if files.is_empty() {
                println!("(no documents)");
            } else {
                for f in files {
                    println!("{f}");
                }
            }
            Ok(())
        }
        DocCmd::Tree => {
            let tree: DocFolder =
                serde_json::from_str(&client.get(&format!("/projects/{p}/docs"))?)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else {
                print_doc_tree(&tree, 0);
            }
            Ok(())
        }
        DocCmd::Folder {
            path,
            name,
            description,
        } => {
            let body = obj(vec![
                ("path", Some(json!(path))),
                ("name", name.clone().map(|n| json!(n))),
                ("description", description.clone().map(|d| json!(d))),
            ]);
            client.write(
                Method::Put,
                &format!("/projects/{p}/docs/folder"),
                Some(body),
            )?;
            println!("configured folder {path}");
            Ok(())
        }
        DocCmd::Show { path } => {
            let enc = urlencode(path);
            println!(
                "{}",
                client.get(&format!("/projects/{p}/docs/content?path={enc}"))?
            );
            Ok(())
        }
        DocCmd::Rm { path } => {
            let enc = urlencode(path);
            client.write(
                Method::Delete,
                &format!("/projects/{p}/docs/content?path={enc}"),
                None,
            )?;
            println!("removed doc {path}");
            Ok(())
        }
    }
}

fn collect_docs(folder: &DocFolder, out: &mut Vec<String>) {
    for d in &folder.docs {
        out.push(d.path.clone());
    }
    for sub in &folder.folders {
        collect_docs(sub, out);
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Encode a free-text value that is **one path segment** — so `/` is escaped too (FEAT-079).
///
/// `urlencode` above leaves `/` alone, which is right for a whole path (a document path's slashes
/// are part of the value) and wrong for a segment. A test name routinely *is* a path —
/// `scripts/check-affordance.mjs` — and passing its slashes through split the route into more
/// segments than the dispatcher matches, so the write came back "unsupported operation" and the
/// test's state silently stayed `planned`. `kanbanr check` then called a passing test unproven,
/// which reads as the evidence rule being broken rather than the transport.
fn urlencode_segment(s: &str) -> String {
    urlencode(s).replace('/', "%2F")
}

fn print_doc_tree(folder: &DocFolder, depth: usize) {
    let indent = "  ".repeat(depth);
    if depth == 0 {
        println!("docs/");
    } else {
        let desc = if folder.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", folder.description)
        };
        println!("{indent}{}/ ({}){}", folder.name, folder.path, desc);
    }
    for doc in &folder.docs {
        println!("{indent}  - {} [{}]", doc.title, doc.path);
    }
    for sub in &folder.folders {
        print_doc_tree(sub, depth + 1);
    }
}

fn print_board(project: &Project) {
    let states = if project.config.displayed_states.is_empty() {
        project.config.statuses.clone()
    } else {
        project.config.displayed_states.clone()
    };
    println!("# {} board\n", project.id);
    for state in states {
        let items: Vec<_> = project
            .features
            .iter()
            .filter(|f| f.status == state)
            .collect();
        println!("## {} ({})", state, items.len());
        for f in items {
            println!(
                "  - {} {} [{}/{}]",
                f.code,
                f.title,
                f.done_count(),
                f.task_count()
            );
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-082: the guard located the message with `find("-m")` — two characters, anywhere in the
    /// command line. A commit passing its message by file was refused because the match landed in
    /// prose, and a Python script that merely mentioned the flag was blocked as if it were a commit.
    /// Third instance of a token recognised without its boundary (see L-23).
    #[test]
    fn the_guard_reads_only_a_real_message_flag() {
        // R-1: the two characters occurring in prose are not the flag.
        let by_file = "git add -A && git commit -q -F msg.txt";
        assert_eq!(inline_message(by_file), None);
        assert!(is_git_commit(by_file), "it is still recognised as a commit");

        // The original failure: a heredoc body mentioning the flag, piped to something else.
        let prose = "python3 - <<'PY'\nprint(\"use --message or -m\")\nPY";
        assert_eq!(inline_message(prose), None);

        // A real message whose TEXT mentions the flag must still be read as the message, in full.
        let tricky = r#"git commit -m "fix: the commit-msg hook and -m parsing

Refs: kanbanr:FEAT-082/R-1""#;
        let got = inline_message(tricky).expect("a real -m must be found");
        assert!(got.starts_with("fix: the commit-msg hook"), "got: {got}");
        assert!(got.contains("Refs: kanbanr:FEAT-082/R-1"), "got: {got}");

        // R-2: an editor commit supplies nothing inline.
        assert_eq!(inline_message("git commit --amend"), None);

        // R-3: the attached spellings are the same flag and must not slip past.
        assert_eq!(
            inline_message(r#"git commit --message="chore: x""#).as_deref(),
            Some("chore: x")
        );
        assert_eq!(
            inline_message(r#"git commit -m"chore: y""#).as_deref(),
            Some("chore: y")
        );
        // ...while a different long flag that merely begins with -m is not.
        assert_eq!(inline_message("git commit --amend --no-edit"), None);

        // Only the commit part is read: an earlier command's -m is not the commit's message.
        assert_eq!(
            inline_message(r#"echo -m "not this" && git commit -m "but this""#).as_deref(),
            Some("but this")
        );
    }

    /// FEAT-082, second half. The first fix scoped `inline_message` and left the hook-skipping
    /// check beside it scanning the whole command line, so a commit message *explaining* the shell
    /// idiom `[ -n "$x" ]` was read as passing that flag, and the commit was refused. Fixing one
    /// instance of a bug while its twin sits two lines above is its own lesson.
    #[test]
    fn skipping_hooks_is_detected_as_a_flag_not_as_prose() {
        let skips = skips_hooks;
        // Actually skipping them.
        assert!(skips("git commit --no-verify -m 'x'"));
        assert!(skips("git commit -n -m 'x'"));
        // Merely talking about them.
        assert!(!skips(
            r#"git commit -m "because [ -n \"$x\" ] returns 1 when the value is empty""#
        ));
        assert!(!skips(r#"git commit -m "we never use --no-verify here""#));
        // And a flag belonging to another command in the chain is not the commit's.
        assert!(!skips(r#"echo -n hi && git commit -m "x""#));
    }

    /// FEAT-098: a message quoting a phrase was cut at the escaped quote, losing its trailer.
    /// FEAT-108: a heredoc's body is data, not shell. What was refused: a script whose body documents
    /// a commit, flags and all, read as a commit that skips its hooks.
    #[test]
    fn a_heredoc_body_is_not_a_commit() {
        let script = "cd /repo && python3 - <<'PY'\n\
            doc = \"\"\"\n\
            git init && git add . && git commit -m \"[no-ref] initial commit\"\n\
            test -n \"$x\" && echo ok\n\
            \"\"\"\n\
            PY\n";
        assert!(
            is_git_commit(script),
            "the raw line does look like a commit"
        );
        let seen = without_heredoc_bodies(script);
        assert!(!is_git_commit(&seen), "{seen}");
        assert!(!skips_hooks(&seen));

        // R-2: a real commit beside a heredoc is still inspected — after it, before it, and with
        // the tab-stripping, double-quoted form.
        let after = "kanbanr batch <<JSON && git commit --no-verify -m \"x\"\n\
            {\"x\": \"git commit -n\"}\nJSON\n";
        assert!(skips_hooks(&without_heredoc_bodies(after)));
        let before = "git commit -m \"real\" && cat <<-\"EOF\"\n\tgit commit -n\n\tEOF\n";
        let seen = without_heredoc_bodies(before);
        assert!(is_git_commit(&seen));
        assert!(!skips_hooks(&seen), "the -n was in the body: {seen}");
        assert_eq!(inline_message(&seen).as_deref(), Some("real"));

        // Not heredocs: a here-string, and `<<` inside quotes.
        let herestring = "grep x <<< \"$y\" && git commit -n -m \"z\"";
        assert!(skips_hooks(&without_heredoc_bodies(herestring)));
        let quoted = "git commit -n -m \"use <<EOF for input\"\nmore";
        assert!(skips_hooks(&without_heredoc_bodies(quoted)));
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_message() {
        let q = '"';
        let cmd = format!(
            "git commit -q -m {q}fix: shown under \\{q}Not started\\{q}, wrongly\n\nRefs: kanbanr:FEAT-097/R-1{q}"
        );
        let got = inline_message(&cmd).expect("the message must be found");
        assert!(
            got.contains("Refs: kanbanr:FEAT-097/R-1"),
            "trailer lost: {got:?}"
        );
        assert!(
            got.contains("under \"Not started\","),
            "escape not unescaped: {got:?}"
        );

        // An escaped backslash before the closing quote does not escape the quote.
        let cmd = format!("git commit -m {q}path C:\\\\{q}");
        assert_eq!(inline_message(&cmd).as_deref(), Some("path C:\\"));

        // Single quotes have no escapes in sh: the next quote ends it.
        assert_eq!(
            inline_message("git commit -m 'it\\'s'").as_deref(),
            Some("it\\")
        );

        // A quote that never closes yields nothing rather than a guess.
        assert_eq!(
            inline_message(&format!("git commit -m {q}never closes")),
            None
        );
    }

    /// FEAT-079: a test named after the file that runs it could not be flipped green. The name is
    /// one path segment, and the whole-path encoder leaves `/` alone — so the route grew a segment,
    /// the write was refused as unsupported, and `kanbanr check` then called a passing test unproven.
    #[test]
    fn a_path_segment_encodes_its_slashes() {
        // R-1: a free-text value used as ONE segment has its slashes escaped.
        assert_eq!(
            urlencode_segment("web: npm run check:ui (scripts/check-affordance.mjs)"),
            "web%3A%20npm%20run%20check%3Aui%20%28scripts%2Fcheck-affordance.mjs%29"
        );
        // The route it goes into keeps the segment count the dispatcher matches on.
        let path = format!(
            "/projects/p/features/FEAT-1/tests/{}/{}",
            urlencode_segment("R-2"),
            urlencode_segment("a/b/c")
        );
        assert_eq!(path.split('/').filter(|s| !s.is_empty()).count(), 7);
        // Everything else a test name holds still round-trips.
        assert_eq!(
            urlencode_segment("eventing::tests::x y"),
            "eventing%3A%3Atests%3A%3Ax%20y"
        );

        // R-2: the whole-path encoder is unchanged — a document path's slashes are part of the
        // value, so escaping them there would address a different document.
        assert_eq!(urlencode("design/mirror.md"), "design/mirror.md");
        assert_eq!(urlencode("a b/c.md"), "a%20b/c.md");
    }

    /// The capture hook is only as good as this parser: a name it misreads is evidence recorded
    /// against the wrong test, which is worse than no evidence at all.
    /// The guard only speaks about commits, and only when it can see the message. Everything it
    /// misreads here is either a commit it wrongly blocks or one it wrongly waves through.
    #[test]
    fn durations_read_the_way_a_person_would_say_them() {
        assert_eq!(duration(3.5), "3.5d");
        assert_eq!(duration(1.0), "1.0d");
        // An afternoon's work is not "0.0 days" — the number that made the metric look broken.
        assert_eq!(duration(0.25), "6h");
        assert_eq!(duration(0.02), "29m");
        assert_eq!(duration(0.0), "0m");
    }

    #[test]
    fn stale_evidence_is_summarised_rather_than_enumerated() {
        let few = vec!["FEAT-001/R-1".to_string(), "FEAT-001/R-2".to_string()];
        let line = stale_line(&few);
        assert!(line.contains("FEAT-001/R-1, FEAT-001/R-2"), "{line}");
        assert!(line.contains("re-run the suite"), "{line}");

        let many: Vec<String> = (1..=27).map(|i| format!("FEAT-001/R-{i}")).collect();
        let line = stale_line(&many);
        assert!(line.contains("27 requirement(s)"), "{line}");
        assert!(
            line.contains("e.g. FEAT-001/R-1, FEAT-001/R-2, FEAT-001/R-3"),
            "{line}"
        );
        assert!(
            !line.contains("R-27"),
            "the point is not to print them all: {line}"
        );
    }

    /// FEAT-104 R-1: the hint names the command that works on any machine — the built-in monitor —
    /// not a development build that exists only in kanbanr's own checkout.
    #[test]
    fn open_hint_names_the_built_in_monitor() {
        let hint = monitor_down_hint("http://localhost:8080");
        assert!(hint.contains("kanbanr serve\n"), "{hint}");
        assert!(!hint.contains("--ui-dir"), "{hint}");
        assert!(!hint.contains("web/dist"), "{hint}");
    }

    /// FEAT-065: the guard decides from a path and nothing else. Every judgement it gets wrong is
    /// either a note scattered into a codebase or a legitimate write refused — and a guard that
    /// refuses legitimate writes gets turned off.
    #[test]
    fn the_docs_guard_ignores_paths_outside_the_repo() {
        let root = Path::new("/work/shop");
        // FEAT-111 R-1: Claude Code's plan file, a scratch file, and an escape through `..` are
        // all outside the repository, so the guard has no opinion on them.
        for path in [
            "/home/someone/.claude/plans/a-plan.md",
            "/tmp/scratch/NOTES.md",
            "../elsewhere/PLAN.md",
            "/work/shop/../shop-notes/PLAN.md",
        ] {
            assert_eq!(board_path_for(path, root, root), None, "{path}");
        }
        // Inside it, the rule is unchanged: loose notes are refused, deliverables are not.
        assert!(board_path_for("/work/shop/NOTES.md", root, root).is_some());
        assert_eq!(board_path_for("/work/shop/README.md", root, root), None);
    }

    /// FEAT-111 R-2: the suggestion is a board path someone can actually run — built from the
    /// repo-relative path, never `notes//home/…`.
    #[test]
    fn the_docs_guard_suggests_a_usable_board_path() {
        let root = Path::new("/work/shop");
        assert_eq!(
            board_path_for("/work/shop/api/Design Notes.md", root, root).as_deref(),
            Some("api/design-notes")
        );
        assert_eq!(
            board_path_for("./PLAN.md", root, &root.join("sub/..")).as_deref(),
            Some("plan")
        );
    }

    #[test]
    fn the_docs_guard_allows_deliverables_and_stops_the_rest() {
        // Project reasoning written loose in a checkout: this is what belongs on the board.
        for path in [
            "NOTES.md",
            "design-thoughts.md",
            "api/PLAN.md",
            "some/deep/folder/retrospective.md",
        ] {
            assert!(belongs_on_the_board(path), "{path} should go to the board");
        }

        // What the repository ships, and what is not prose at all.
        for path in [
            "README.md",
            "CHANGELOG.md",
            "CONTRIBUTING.md",
            "SECURITY.md",
            "CLAUDE.md",
            "docs/USER_GUIDE.md",
            "skill/kanbanr/SKILL.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            "web/README.md",
            "api/crates/kanbanr-cli/tests/fixtures/board.md",
            "api/target/doc/index.md",
            "src/main.rs",
            "Cargo.toml",
        ] {
            assert!(
                !belongs_on_the_board(path),
                "{path} should be allowed through"
            );
        }

        // Case is not identity: a repository's README is a README however it is spelled.
        assert!(!belongs_on_the_board("readme.md"));
        assert!(!belongs_on_the_board("Docs/design.md"));
    }

    #[test]
    fn the_commit_guard_recognizes_a_commit_and_reads_its_message() {
        assert!(is_git_commit("git commit -m \"x\""));
        assert!(is_git_commit("git -C /repo commit -am \"x\""));
        assert!(is_git_commit("cd /repo && git commit"));
        assert!(!is_git_commit("git status"));
        assert!(!is_git_commit("git log --oneline -1 && echo commit"));
        assert!(!is_git_commit("echo 'git commit'"));

        assert_eq!(
            inline_message("git commit -m \"feat: a thing\"").as_deref(),
            Some("feat: a thing")
        );
        assert_eq!(
            inline_message("git commit -m 'feat: a thing\\n\\nRefs: kanbanr:FEAT-001'").as_deref(),
            Some("feat: a thing\n\nRefs: kanbanr:FEAT-001")
        );
        // An editor commit has no inline message; the commit-msg hook covers that one.
        assert_eq!(inline_message("git commit"), None);
    }

    #[test]
    fn test_output_is_read_from_the_formats_a_run_actually_prints() {
        let text = "\
running 3 tests
test hooks::tests::install_merges ... ok
test report::tests::cycle_time ... FAILED
test slow::case ... ignored
✓ cart retains items for 7 days
✗ cart empties on sign-out
PASS src/cart.test.ts
FAIL src/checkout.test.ts
some unrelated line ... ok";
        let results = parse_test_results(text);
        assert_eq!(results.get("hooks::tests::install_merges"), Some(&true));
        assert_eq!(results.get("report::tests::cycle_time"), Some(&false));
        assert_eq!(results.get("cart retains items for 7 days"), Some(&true));
        assert_eq!(results.get("cart empties on sign-out"), Some(&false));
        assert_eq!(results.get("src/cart.test.ts"), Some(&true));
        assert_eq!(results.get("src/checkout.test.ts"), Some(&false));
        // An ignored test proves nothing, and a line that merely ends in "ok" is not a result.
        assert_eq!(results.get("slow::case"), None);
        assert_eq!(results.len(), 6, "{results:?}");
        // Nothing recognizable: the hook stays quiet rather than guessing.
        assert!(parse_test_results("cargo build finished in 3.2s").is_empty());
    }
}
