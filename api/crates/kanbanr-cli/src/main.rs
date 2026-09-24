//! kanbanr CLI — driven by the Claude skill. It talks to a kanbanr **server** (the default), or in
//! **local mode** operates on the data folder directly via `kanbanr-core` with no server running
//! (auto-detected when no server is configured but a data dir is present; forced with `--local`).

mod backend;
mod hooks;
mod mirror;

use backend::{Backend, Method};
use clap::{Args, Parser, Subcommand};
use kanbanr_core::docs::DocFolder;
use kanbanr_core::project::{DataDirSource, Marker};
use kanbanr_core::{project, Project};
use serde_json::{json, Map, Value};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "kanbanr",
    version,
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
    Move { code: String, status: String },
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
        /// Start from the built-in default workflow (Deferred/Planned/Scheduled/Completed +
        /// the 3 default no-op states); any flags below then override it.
        #[arg(long)]
        defaults: bool,
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
    /// Add the hooks (skipped if already there or provided by the kanbanr plugin).
    Install,
    /// Show whether the hooks are registered and their scripts exist.
    Status,
    /// Remove kanbanr's hooks (other hooks are left alone).
    Uninstall,
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
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
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
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let (dir, record) = choose_init_data_dir(cli, &cwd)?;
    if record {
        if let Some(repo) = project::enclosing_git_worktree(&dir, project::home_dir().as_deref()) {
            println!(
                "⚠ {} is inside the git repo at {}: the board would be nested in that repo \
                 (gitignore it, or pick a folder outside the repo)",
                dir.display(),
                repo.display()
            );
        }
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
    project::write_marker(
        &cwd,
        &Marker {
            project: Some(proj.clone()),
            data_dir: record.then(|| marker_path(&dir, &cwd)),
        },
    )?;
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

/// Register the Claude Code hooks during `init`; never fails `init`. (FEAT-044)
fn report_hooks_install() {
    let Some(dir) = hooks::claude_dir() else {
        println!("• Claude Code hooks not installed: no home directory found");
        return;
    };
    let settings = hooks::settings_path(&dir);
    match hooks::install(&dir) {
        Ok(hooks::Installed::Added(events)) => println!(
            "✓ Claude Code hooks added to {} ({}); they act only in kanbanr-tracked folders",
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

/// `kanbanr hooks …` (FEAT-044).
fn run_hooks(cli: &Cli, cmd: &HooksCmd) -> anyhow::Result<()> {
    let dir = hooks::claude_dir()
        .ok_or_else(|| anyhow::anyhow!("no home directory: set CLAUDE_CONFIG_DIR"))?;
    match cmd {
        HooksCmd::Install => match hooks::install(&dir)? {
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
        HooksCmd::Uninstall => {
            let n = hooks::uninstall(&dir)?;
            println!("removed {n} kanbanr hook entr{}", if n == 1 { "y" } else { "ies" });
        }
        HooksCmd::Status => {
            let st = hooks::status(&dir)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&st)?);
                return Ok(());
            }
            println!("settings: {}", st.settings);
            println!("skill installed: {}", if st.skill_installed { "yes" } else { "no" });
            if st.plugin {
                println!("kanbanr plugin: enabled (provides the hooks)");
            }
            for h in &st.hooks {
                let event = h["event"].as_str().unwrap_or("");
                match h["command"].as_str() {
                    Some(c) if h["script_exists"] == true => println!("{event}: ✓ {c}"),
                    Some(c) => println!("{event}: ✗ script missing: {c} (run `kanbanr hooks install`)"),
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
            println!("linked {code} → {url}\nThe next sync replaces that issue's title and body with kanbanr's.");
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
        println!(
            "monitor not reachable at {url}.\nStart it first:\n  \
             kanbanr serve --ui-dir web/dist        # this same binary, no Docker\nthen re-run `kanbanr open`."
        );
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
        } => {
            return run_init(
                cli,
                name.clone(),
                description.clone(),
                author.clone(),
                email.clone(),
                *no_hooks,
            );
        }
        Command::Hooks(cmd) => return run_hooks(cli, cmd),
        Command::Serve {
            bind,
            ui_dir,
            allow_writes,
        } => return run_serve(cli, bind.clone(), ui_dir.clone(), *allow_writes),
        Command::Open => return run_open(cli),
        Command::Where => return run_where(cli),
        _ => {}
    }

    let client = make_backend(cli);
    match &cli.command {
        Command::Identity { .. }
        | Command::Init { .. }
        | Command::Serve { .. }
        | Command::Open
        | Command::Where
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
        Command::Move { code, status } => {
            let p = require_project(cli)?;
            let resp = client.write(
                Method::Post,
                &format!("/projects/{p}/features/{code}/move"),
                Some(json!({ "to": status })),
            )?;
            print_write(cli, &resp, format!("{code} -> {}", field(&resp, "status")));
            Ok(())
        }
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
        Command::Sources { write } => run_sources(cli, &client, *write),
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
    if let Some(labels) = &args.label {
        if !labels.is_empty() {
            push(&mut params, "label", &labels.join(","));
        }
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
        } => {
            let body = obj(vec![
                ("name", Some(json!(name))),
                ("description", description.clone().map(|d| json!(d))),
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
                ("assignee", assignee.clone().map(|a| json!(a))),
                ("team", team.clone().map(|t| json!(t))),
                ("labels", labels.clone().map(|l| json!(l))),
                ("depends_on", depends_on.clone().map(|d| json!(d))),
            ]);
            let resp =
                client.write(Method::Post, &format!("/projects/{p}/features"), Some(body))?;
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
            defaults,
            statuses,
            transitions,
            default_state,
            displayed_states,
            no_op_states,
            terminal_states,
            to_mermaid,
            from_mermaid,
        } => {
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
                ("defaults", Some(json!(defaults))),
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
