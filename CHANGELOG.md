# Changelog

All notable changes to kanbanr are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Setup is a plan-mode interview, then one approved setup (FEAT-100)

Asking Claude to set up kanbanr in an untracked folder now starts in **plan mode**: board folder
and commit identity, a charter drafted from the repository and marked as a draft, and the process
(default kanban, TOGAF phases or custom, git hooks, remote, mirror, import). Exiting plan mode is
the approval; the whole setup then runs, is saved as a board doc and checked with `doctor` before
any other work starts.

#### Session summaries ship with kanbanr (FEAT-101)

The session-summary hooks (a compaction's summary, the end of a session, and a sweep at the next
start) moved from one checkout's ignored `.claude/` into `skill/kanbanr/hooks/`, and
`kanbanr hooks install` registers them for PostCompact, SessionEnd and SessionStart, as does the
plugin. They act only in folders with a `.kanbanr` marker, need `bash`, `jq` and `python3`, and
are not offered on Windows. A project's own `.claude/commands/create-summary.md` sets the
summary's shape when it has one. `.gitignore` now ignores only the machine-local parts of
`.claude/`.

#### One binary, installed in one line (FEAT-084)

The web monitor is now **compiled into the binary**, gzipped and decompressed once at startup, so
`kanbanr serve` shows it with no `--ui-dir` and nothing to build. An explicit `--ui-dir` still wins
for SPA development, and a build without `web/dist` embeds nothing, succeeds, and **says so** at
startup rather than serving a blank page. The binary grows 12 MB → 13 MB: the assets' compressed
size, asserted from both sides in CI.

`scripts/install.sh` and `install.ps1` install it in one line, verifying the download against the
release's own `SHA256SUMS`. They ship **as release assets**, so the documented one-liner comes from
the release host rather than a CDN of the default branch that could serve a mismatched script.
`GH_TOKEN` lifts the API rate limit, and is sent only to `api.github.com`. `release.yml` then proves
the whole thing: `verify-install` runs the published one-liner in clean Debian and Ubuntu containers
and asserts the monitor is actually served. See `docs/INSTALL.md` and `ADR-0009`.

kanbanr is **not** published to crates.io with the UI: a published crate cannot carry built assets
without committing generated files, so the archive and the installer are the supported way to get a
complete binary.

#### Reasoning, evidence and traceability (MS-006)

A board records *what* is being built; this milestone adds **why it exists, what must be true, and
what proves it** — and refuses to record anything it cannot derive. All of it is opt-in: a project
with no charter behaves exactly as before, and items created before a charter was adopted are never
reported against it.

- **Project charter** (FEAT-046): `charter.yaml` holds the purpose, vision, goals with ids,
  non-goals, stakeholders and constraints. Work items link goals; `kanbanr charter show|set` and a
  Charter tab in the monitor. A goal with no work behind it, and an item serving no goal, are both
  reported.
- **Feature definitions** (FEAT-047): why an item exists, the six Zachman dimensions in a line
  each, requirements in EARS form with the tests that prove them, and a design-doc pointer —
  inline on the item, so an older board still loads byte-identically. `kanbanr feature define
  [--template --kind defect]`.
- **Approval gates** (FEAT-048): `kanbanr review` renders a one-screen decision brief, `approve`
  records agreement pinned to a hash of the definition's *content*, and starting an item without a
  current approval is refused. Editing the definition afterwards **lapses** the approval rather
  than silently keeping it; the override (`--unapproved "<reason>"`) is recorded on the item.
- **EARS and ISO/IEC 25010 checks** (FEAT-049): requirements are classified into the five EARS
  patterns (never rejected), quality tags canonicalised against the nine 2023 characteristics, and
  `doctor` reports unsupported claims — a measured scenario whose measure names no test, a quality
  tag with no scenario, a blank dimension.
- **Surfacing and search** (FEAT-050): definitions render in `feature show`, `query --goal/--gap`
  searches them, and the monitor shows the definition grid, requirements with their evidence, and
  gap chips.
- **TDD test states** (FEAT-051): each requirement carries its tests through planned → red → green
  with the revision they were observed at; `kanbanr check` reports what an item has not said and
  cannot yet show. Optional TOGAF phase preset for the workflow.
- **Measurement** (FEAT-053): every status change appends to the item's history; a defect record
  whose *escaped* flag is derived (it escaped if the work that introduced it was already called
  done); a PostToolUse hook that reads real test output and records which tracked tests passed,
  stamped with the revision; `kanbanr report --since` for throughput, cycle time, rework, escape
  rate and requirement coverage; `kanbanr tests --write` returns a green whose test no longer
  exists to planned.
- **Wave retrospectives** (FEAT-054): `kanbanr retro <milestone|--since|--label> [--write]` reports
  scope growth split by what items record, self-inflicted defects, cycle time, rework, evidence at
  completion and estimate vs actual, and writes a document whose computed facts and narrative are
  separate sections. Finishing a milestone's last item emits an event; the Stop hook surfaces a
  retro that is due. `kanbanr split-from` records work sliced out of another item.
- **Lessons with confidence decay** (FEAT-055): recorded in flight with their evidence, deduplicated
  (saying one again affirms it), decaying unless reaffirmed, contradiction weighted heavier than
  affirmation, and retired rather than deleted below the threshold. Surfaced at session start, per
  item (`kanbanr lessons --for`), and on the Charter tab.
- **SCM traceability** (FEAT-056): one item, one branch, one reference per commit. `kanbanr start`
  branches and moves the item, `kanbanr commit` fills in `Refs: kanbanr:FEAT-046/R-2`, `finish`
  refuses while tasks are open or requirements unproven. `kanbanr git install-hooks` adds commit-msg
  and pre-commit checks that keep any hook already there, and a Claude Code guard answers the same
  rules *before* a commit is attempted. Escapes are explicit: `[no-ref] <why>` stays in git history.
- **Code tied to the why** (FEAT-057): `kanbanr trace` down from a goal, item or requirement —
  ending in the gaps — and `kanbanr why <file>:<line>` up through the annotation or the commit
  trailer to requirement, goal and purpose. Architecture decisions become documents with
  front-matter that joins them to the graph (`adr new|list|supersede|history`), with `Docs:` and
  `ADR:` commit trailers validated like any other reference, and `trace --zachman` reporting the
  columns nothing addresses.

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

- **The shipped session-start and stop-check hooks were empty (FEAT-102).** A commit on 28 Sep
  replaced both with `exit 0` stubs, so sessions stopped recovering the board and turns stopped
  being nudged to record work, while `hooks status` still reported them healthy. Both are
  restored, and a test now fails if a shipped hook script is a stub.
- **Looking in a folder with no board created one (FEAT-103).** Any read (`whoami`, `board`,
  `doctor` and so on) run with no marker and no `$KANBANR_DATA_DIR` left a stray `./data/`
  behind. Reads now say there is no board and point at `kanbanr init`. Hook commands stay silent
  there, and an existing legacy `./data` board still works.
- **`kanbanr open` suggested `serve --ui-dir web/dist` (FEAT-104).** The monitor has been built
  into the binary since FEAT-084. The hint, the skill and the docs now say `kanbanr serve`.
- **A fresh repository could not make its first commit (FEAT-105).** With no commits, the git
  hooks called the default branch `main` while HEAD was an unborn `master`, then refused the root
  commit. An unborn HEAD is now the default branch, the root commit may land on it, `start`
  refuses until a first commit exists, and `init.defaultBranch` only counts when that branch
  exists.
- **Board columns scrolled inside a 1200px page (FEAT-107).** The board now uses the window's
  width, so a six-status TOGAF board fits. Reading pages keep their 1200px measure.
- **The docs guard refused files outside the repository (FEAT-111).** It blocked Claude Code's
  own plan file in `~/.claude/plans` and suggested an unusable `notes//home/…` board path. It now
  judges only files inside the tracked repository, and suggests a path relative to it.
- **Piping output into `head` printed a panic (FEAT-110).** `kanbanr git status | head -1` ended
  with a stack trace after a command that had succeeded. A closed pipe now ends the command
  quietly with status 141, as it does for `git`.
- **Work finished under a recorded bypass couldn't be ratified from the monitor (FEAT-109).**
  `doctor` was the only place it appeared, and `kanbanr ratify` the only way to agree to it. It now
  heads the Review page with a **Ratify** button, chosen with the same rule `doctor` uses.
- **The commit guard read heredoc bodies as commands (FEAT-108).** A script that only *mentioned*
  `git commit` was refused. Heredoc bodies are now skipped. A real commit on the same line is
  still checked.
- **Setup interview gaps from its first real run (FEAT-106).** The skill now gives the charter's
  exact fields (`statement`, not `outcome`), checks that non-goals are non-goals, plans
  `git init` plus an initial commit for a folder that isn't a repository, and ends by saying how
  to open the monitor.

- **The released Linux binary would not have run on Debian stable** (FEAT-087): a `-gnu` target
  links the build runner's glibc, and the release matrix built on the newest one — so the binaries
  required glibc 2.39 and would have died on bookworm (2.36) with `libc.so.6: version GLIBC_2.39
  not found`. `docker/Dockerfile` already carried a comment describing this exact failure from when
  it bit the image; nothing carried that note to the workflow that makes what people download. The
  Linux legs now build on Ubuntu 22.04 (glibc 2.35), `verify-install` brackets the claimed range,
  and `docs/INSTALL.md` states the floor and quotes the error.
- **An approval named a place, not a person** (FEAT-077): the monitor recorded verdicts as
  `by: "reviewed in the monitor"`, naming the surface the click happened on. FEAT-069 already
  required the record to carry *who*. `/api/meta` now reports the board's commit identity and the
  monitor attributes a verdict to the same name `kanbanr approve` would; a verdict with no named
  approver is refused rather than attributed to `"unknown"`. Approving and withdrawing also emit
  events now — a withdrawal means work already in flight lost its mandate, and the only trace used
  to be inside the item's own file.
- **The review queue asked for agreement on finished work** (FEAT-078): `doctor` and the queue
  answered the same question differently — 2 items against 6, the extras Completed or deliberately
  Deferred. Approving merged work records a signature that changes nothing, and a gate that asks
  for those gets rubber-stamped. Both now read one `graph::is_live_work` gate, and in-flight work
  is asked about first.
- **A test named after a file could not be recorded** (FEAT-079): a test name is one path segment,
  and the CLI's encoder leaves `/` alone — right for a whole path, wrong for a segment. The write
  was refused and the state silently stayed `planned`, so `kanbanr check` called a passing test
  unproven, which reads as the evidence rule being broken rather than the transport.
- **A control that did not look like one** (FEAT-076, FEAT-081): the review queue ran its briefs
  together as one column and its approve action used the label style. Briefs are now collapsible
  cards, and `.btn` has its own raised surface — it had been declared with the *same* background as
  `.chip`, so changing the class satisfied the requirement while the button still read as a tag.
  `npm run check:ui` asserts both the class rule and that the two surfaces differ. See ADR-0008.
- **A published snippet carried a local path** (FEAT-024): `skill/kanbanr/hooks/settings.snippet.json`
  registered its hooks by an absolute path into the author's home directory — one that had not
  existed since the repository moved, so anyone following it registered two hooks that silently did
  nothing. Now a placeholder, leading with `kanbanr hooks install`, which resolves the paths itself.
- **Auto-completion left no transition behind** (FEAT-061): ticking an item's last task completed it
  by assigning the status directly, so cycle time was blind to the normal way items finish and
  reported only on hand-moved ones.
- **The retrospective over-claimed** (FEAT-060, FEAT-063): it counted items planned together as
  scope growth, required evidence to match the current revision in a historical account, demanded
  retrospectives for waves the board never watched, and reported changelog timestamps — which time
  board writes, not work — as cycle time.
- **Report warnings that fired on everything** (FEAT-062): sub-day cycle times printed as `0.0`
  days, and every green recorded before the last commit was listed as stale evidence.
- **A guardrail that matched its own explanation** (FEAT-056): a commit message explaining the
  `[no-ref]` escape was read as taking it, waving through a commit that had a valid reference.
- **A mermaid diagram in DESIGN.md that rendered as nothing** — a `;` inside a sequence-diagram
  message is a statement separator. `npm run check:docs` now parses every diagram in CI with the
  same library the monitor renders them with.
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
