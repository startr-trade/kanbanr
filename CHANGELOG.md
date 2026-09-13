# Changelog

All notable changes to kanbanr are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Claude Code hooks set up automatically** (FEAT-044): `kanbanr init` registers the skill's
  SessionStart and Stop hooks in the global Claude Code settings (`$CLAUDE_CONFIG_DIR` or
  `~/.claude`), once per machine (`--no-hooks` to skip). The merge preserves existing keys and
  hooks in order, writes atomically, leaves invalid JSON untouched, repairs registrations whose
  script is gone, and defers to the kanbanr plugin when it is enabled. New
  `kanbanr hooks install | status | uninstall`.
- **GitHub issue mirror** (FEAT-043): `kanbanr mirror enable --repo owner/repo` keeps a project's
  features in step with GitHub issues through `gh`, one way (kanbanr is the source of truth).
  - After every write, changed features are pushed: new feature → issue; title/spec/labels/tasks
    → update; Completed → closed as completed; no-op state → closed as not planned. Change
    detection uses a stable hash of the rendered issue, so unchanged features make no calls, and
    failures never fail the write (`kanbanr mirror sync` catches up; `KANBANR_MIRROR=off` pauses).
  - Refuses public repos without `--allow-public`; existing features are backfilled only with
    `mirror sync --all`.
  - `mirror status` (plan, no calls), `mirror link` (existing issue), `mirror pull` (read-only:
    edits on GitHub since the last sync and new comments), `mirror disable`.
  - Imported GitHub issues keep their `issue` link, so they are updated rather than duplicated.
- **Import existing task trackers** (FEAT-042): at activation the skill offers to import work
  already tracked in `TODO.md`/`ROADMAP.md`, AI-tool plan files (Spec Kit, Kiro), GitHub issues or
  exports, after asking which sources to bring in (open items by default).
  - `feature.add` accepts `source` (provenance: system, ref, revision, url), `original` (preserved
    in the spec under "Imported from") and `issue`; kanbanr stamps `imported_at`, derives a stable
    re-import `key`, and the CLI fills the project commit for file sources. Sources are history,
    not live pointers, so deleted files or rewritten history leave nothing dangling.
  - Re-running an import skips known sources, same-named milestones, and the ops under skipped
    items.
  - `kanbanr batch --dry-run` previews a bundle without writing (no activity entry, no commit).
  - `kanbanr sources [--write]` checks whether file sources still exist and records
    `missing_since`; the monitor labels vanished sources and links mirrored issues.
- **Board next to the project** (FEAT-041): `kanbanr init` asks where to keep the board and
  recommends a sibling of the project's git repo named `<repo>.kanbanr`, or an existing kanbanr
  folder nearby so projects can share one. The choice is recorded in the `.kanbanr` marker
  (`project:` + `data_dir:`, relative to the marker), which is now found by walking up from the
  current directory. New `kanbanr where [--json]` shows the board folder in use. Data dir order:
  `--data-dir` → `$KANBANR_DATA_DIR` → marker `data_dir` → `./data`. Legacy one-line markers and
  `./data` boards keep working. The skill asks the user for the location on activation, and the
  Stop hook finds the board via `kanbanr where`.
- **Project docs default to kanbanr** (FEAT-040): the skill writes every document (requested or
  self-initiated) as a kanbanr doc, and into the project folder only when the user asks.
- **Enterprise-scale coordination (opt-in, milestone MS-005)** — kanbanr scales from a single
  project to a portfolio without losing the git-backed, file-per-entity model:
  - **Cross-project dependencies**: a `depends_on` entry may be qualified `"<project>:<code>"`; a
    portfolio-wide graph resolver validates existence + global acyclicity (FEAT-026).
  - **Derived dependency state**: `ready`/`blocked`/`graph`/`impact` (transitive downstream closure)
    over the dependency DAG, per-project or portfolio-wide, with a "blocked" chip on the board
    (FEAT-027).
  - **Critical path & scheduling**: `start`/`estimate_days` fields, longest-path schedule, and a
    **Mermaid Gantt** view (per-project + cross-project) (FEAT-035).
  - **Portfolio/program hierarchy**: optional `workspace.yaml`, cross-project board, and task-based
    rollups (milestone→project→program→portfolio) (FEAT-030).
  - **Ownership**: `assignee`/`team` fields + by-owner/by-team filters and badges (FEAT-031).
  - **Query**: rich filters (status/milestone/kind/priority/label/owner/due/dep-state) + full-text,
    one project or cross-project (FEAT-032).
  - **Workflow as a state chart**: explicit `terminal_states` + Mermaid `stateDiagram-v2` import /
    export (YAML stays the source of truth) and a web Workflow page (FEAT-039).
  - **`doctor`**: portfolio integrity scan (dangling deps, unknown milestones, schema drift) +
    `schema_version` on project config (FEAT-037).
  - **Scale & safety**: lazy spec loading + a per-project `index.yaml` cache (FEAT-033); a
    cross-process advisory **write lock** (FEAT-029); batch ops load the project once (FEAT-028);
    **debounced git push** + an opt-in single-writer daemon (`serve --allow-writes`) (FEAT-034).
  - **Eventing**: per-project event log + opt-in webhooks, emitting `FeatureMoved`/`Completed` and
    **`DependentReady`** (cross-team handoff) notifications on state change (FEAT-036).
- Dashboard: reverse-chronological ordering + per-page pagination (default 10) (FEAT-038).
- Documentation viewer renders **Mermaid** diagrams (fenced ```mermaid blocks) and **embedded
  images** with per-folder relative resolution (any PNG/SVG asset; PlantUML/Graphviz/D2/Excalidraw
  export to an image and embed).
- Light/dark theme toggle (persisted; honors OS preference) plus mobile-responsive layout and
  accessibility improvements (focus-visible, reduced-motion, ARIA labels).
- VS Code extension scaffold under `editor/vscode/` (thin viewer + command layer over the local
  data folder).
- Packaging as a Claude Code plugin (`.claude-plugin/`) bundling the skill and enforcement hooks,
  with cross-platform (PowerShell) hook variants.
- Open-source scaffolding: dual `LICENSE-MIT`/`LICENSE-APACHE`, `CONTRIBUTING`, `SECURITY`,
  `CODE_OF_CONDUCT`, `THIRD_PARTY` notices, GitHub issue/PR templates, Dependabot, and a release
  workflow.

### Fixed
- The Stop hook's record-your-work reminder never reached Claude: it went to stderr with exit 0,
  which Claude Code doesn't pass to the model (FEAT-045). It now answers with a block-once
  `{"decision":"block","reason":…}` only when the board is stale, the project changed since the
  last board update, no reminder was given in this session within the window, and Claude isn't
  already continuing because of a Stop hook.

### Changed
- Relicensed the workspace to **MIT OR Apache-2.0** (was MIT) — the Rust-ecosystem norm.

## [0.1.0] - Unreleased

First public-candidate release.

### Added
- One binary `kanbanr`: a local, git-backed CLI **writer** plus `kanbanr serve`, a read-only
  view daemon (localhost, no accounts) over the same folder.
- `kanbanr-core` engine: YAML/markdown store, `dispatch` router, vendored-libgit2 commits +
  optional remote pull/push, per-project activity changelog, markdown/JSON export, validation
  (workflow transitions, milestone + cross-feature dependency DAG cycle detection, task
  auto-complete).
- Feature items with code/specification/status/milestone, persistent todo-lists, and fields for
  kind, priority, due date, labels, and cross-feature dependencies.
- React + Vite read-only monitor: project tiles, kanban board, status/feature/milestone/schedule
  pages, documentation viewer (with image assets), filter/search, filterable activity streams, and
  live SSE updates.
- Claude **skill** + enforcement hooks (SessionStart/Stop) making kanbanr the project's system of
  record.
- Optional Docker image and a two-layer test story (Docker-less integration + a testcontainers
  packaging smoke).

[Unreleased]: https://github.com/startr-trade/kanbanr/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/startr-trade/kanbanr/releases/tag/v0.1.0
