# Changelog

All notable changes to kanbanr are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **What 1.0 keeps compatible is written down (FEAT-148).** A new chapter,
  [Stability](https://kanbanr.startr.trade/project/stability.html), lists what stays stable from
  1.0 until 2.0 — the board format and `config.yaml`, the `.kanbanr` marker, CLI commands, flags,
  exit status and `--json`, the monitor's `/api/…`, the commit trailer and hooks — what is not
  covered, and how anything stable is deprecated before it is removed. Until 1.0 the feature set
  is frozen.

### Changed

- **The release proves its installers on every system it publishes for (FEAT-149).** After a
  release is published, `install.sh` now runs on macOS and `install.ps1` on Windows — run as the
  docs tell a user to, `irm … | iex` included — beside the two Linux containers, and each install
  must name the release and its commit and serve the monitor. The checks live in
  `scripts/verify-install.sh` and its Windows twin, and `make ci` runs the first against the latest
  published release.

## [0.1.1] - 2026-10-02

The container image is the released program.

### Fixed

- **The container image's kanbanr could not say which build it is (FEAT-144).** `kanbanr --version`
  in the GHCR image printed `(unknown, built unknown)`: the image is built without `.git`, and nothing
  handed it the commit. The release now passes the commit and its date in as build arguments, and
  refuses to push an image whose binary does not name them — the check the archives already had.
  `make ci` builds the image the same way and runs the same check.
- **The container image's kanbanr carried no skill (FEAT-145).** The image's build stage copied only
  `api/`, so the program embedded no Claude Code skill and `kanbanr skill install` in the image
  refused — the same version as the archives, but not the same program. The build now copies
  `skill/`, and the check before the push (`scripts/check-image.sh`) also requires the image's
  kanbanr to install the repository's skill, file for file.

## [0.1.0] - 2026-10-01

The first public release.

### Added

#### One install: the program carries its skill (FEAT-141)

The `kanbanr` program embeds the Claude Code skill it matches — SKILL.md, the hook scripts and
their prompt — and `kanbanr skill install | status | uninstall` puts it in `~/.claude/skills/kanbanr`.
The release installers run it when Claude Code is on PATH (`--no-skill` to skip), so installing
kanbanr is one step, and the skill is always the release's, never a branch's. `self-update`
updates a skill kanbanr installed; a folder it did not write — a link to a clone, a plugin's copy —
is left alone. In a tracked project without the program, the session-start hook says how to
install it, and the setup interview checks for it first.

#### The kanbanr mark, and screenshots of the monitor as it is (FEAT-133)

kanbanr has a mark — a `k` whose stem is a kanban column and whose arms are the thread the work
runs along — in the startr.trade palette, with light and dark lockups, a favicon, an app icon and a
social preview in `assets/brand/`. The monitor's header, its favicon, the docs site, the VS Code
extension and the README use it. Every screenshot is retaken, and `make screenshots` now also builds
two demo boards — a portfolio, and a Scrum shop (`tools/demo/cadence.sh`) — to show the sprint
header with its burndown, the Releases page and the Gantt with sprints, which kanbanr's own board
does not use.

#### kanbanr's own board is public, and the code points at it (FEAT-136)

The board kanbanr is developed on is published at `startr-trade/kanbanr-board`, its history
sanitised first. The README gains "Developed with kanbanr" — the charter, an item's definition and
tests, the decisions, retros and lessons, commit trailers and `kanbanr why` — and the contributing
chapter shows how to read the board. A committed `.kanbanr` points at `../kanbanr-board`, so a clone
beside the board needs no setup, and `kanbanr claude sync` names the board's published address in
the CLAUDE.md block once the board has a remote.

#### Security scanning in CI, and one CI run per change (FEAT-130)

- **CodeQL** (`codeql.yml`) analyses the Rust, the TypeScript (monitor and VS Code extension) and
  the workflows themselves, on `main`, on pull requests and weekly.
- **Trivy** (`trivy.yml`) scans dependencies, committed secrets and the Dockerfile; `release.yml`
  scans the image before pushing it and stops on a fixable HIGH or CRITICAL finding.
- **Supply chain**: ci.yml's `supply-chain` job runs `cargo deny` (licence allowlist in
  `api/deny.toml`, sources, advisories), `cargo audit` and `npm audit --omit=dev`, and fails on a
  finding. A suppression needs a reason and an expiry (`.trivyignore.yaml`), checked in CI.
- Code-scanning uploads are skipped while the repository is private, rather than failing.
- Every action is pinned to a commit SHA of a Node 24 release (Node 20 actions are deprecated),
  runners name their image (`ubuntu-24.04`, `macos-15`, `windows-2025`) rather than `-latest`, and
  CI runs once per change — on a push to `main` or a pull request — cancelling superseded runs.
- Locally: `make audit`, `make scan-deps`, `make scan-image`, `make codeql`; `make ci` includes the
  audit and the Trivy scans.

#### `make ci`: every CI check, locally, before a push (FEAT-131)

`make ci` copies the tracked tree to a clean folder and runs every CI check that can run off
GitHub: actionlint over the workflows, a check that every action they use exists, and each
workflow step's own script read from the workflow file (web, Rust, installers, docs, the release
build and packaging, the Docker image). Every script step of every workflow must be classified as
run locally or GitHub-only, so a new CI check cannot go unverified unnoticed, and the run ends by
listing what only GitHub can verify. Its first run found the release workflow naming a retired
macOS runner.

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

kanbanr is **not** published to crates.io: a published crate cannot carry built assets without
committing generated files, so the archive and the installer are the supported way to get a
complete binary (ADR-0011). `ears-classifier` is the one published crate.

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

- **The release reported a correct binary as wrong, and skipped crates.io (FEAT-143).** The installer
  check ran in a container, where steps run under `sh` (dash), and cut the commit to 12 characters
  with a bash-only expansion — so v0.1.0 published correctly and then said its binary did not name
  its commit. The check is POSIX now. `make ci` shellchecks every container-job step as `sh`, and
  fails when a workflow reads a `vars.` setting the repository holds only as a secret, which is how
  the crates.io publish was skipped without a word.
- **A board could not reach GitHub, and its batched push never fired (FEAT-142).** The git library
  was built without its network transports, so `kanbanr sync` to an SSH or HTTPS remote failed with
  "unsupported URL protocol" — and then reported "nothing to sync", because a failed push cleared
  its marker. The batched push counted commits in memory, so the CLI, a new process every command,
  never reached the threshold. Remotes over SSH (agent or `~/.ssh` key) and HTTPS (git's credential
  helper) now work; the count comes from git; a failed push is reported, stays pending and makes
  `sync` fail; the policy is a board setting (`kanbanr remote push-policy`) with `KANBANR_PUSH` as an
  override; and `kanbanr doctor` warns when a board is too far behind its remote.
- **The Releases page counted Done items as unfinished (FEAT-138).** It kept its own copy of the
  "finished" rule and counted only the end status, so a planned scrum release read 0% while items in
  it were Done. The daemon now reports each release's items and finished count by the sprint's rule
  (FEAT-137), and the page shows what it reports.
- **The Claude Code plugin failed validation (FEAT-139).** Its skill path used
  `${CLAUDE_PLUGIN_ROOT}`, which Claude Code expands only in hook commands, so `claude plugin
  validate` failed and the plugin could not load its skill. The path is `./skill/` now, the
  manifests carry no comment keys (the notes are in `.claude-plugin/README.md`), the description no
  longer points at crates.io, the licence reads `MIT OR Apache-2.0`, and `make ci` validates the
  plugin wherever `claude` is installed. The README and the installation chapter now say how to
  install the skill — as a plugin, or from a clone.
- **The monitor rendered board markdown unsanitised (FEAT-140).** CodeQL's first run on the public
  repository flagged a Mermaid diagram inserted as raw HTML; behind it, every spec and board
  document went into the page as `marked` produced it. A shared board's documents are written by
  whoever can push to it, and with `--allow-writes` the page can approve, ratify and sign off, so a
  pushed document could act as the viewer. Markdown and diagrams now pass through DOMPurify, and
  `npm run check:markdown` (in CI) renders a hostile document through the same code.
- **A Scrum sprint burned down nothing until release day (FEAT-137).** The burndown counted an item
  finished only at the workflow's end status, which in the `scrum` preset is Released. A stage's
  gate can now say `done: true`; the `scrum` preset marks Done, its Definition of Done, so items
  burn down as they are done, and a closing sprint no longer carries over work that is Done but not
  yet released.
- **Session summaries were committed with the board (FEAT-135).** A summary is a condensed
  conversation, and it was written as a board document, so it was committed and pushed with
  everything else. Summaries now go to `<board>/.sessions/<project>/`, beside the board, and every
  board ignores `.sessions/`, existing ones from their next write.
- **Dependency advisories in what kanbanr ships (FEAT-132).** The first supply-chain scans found
  a TLS 1.3 handshake flaw in `rustls` (now 0.23.45), an unsound `anyhow` API (now 1.0.104), three
  unsound `git2` APIs (now `git2` 0.21, which also brings a newer libgit2), and advisories in the
  monitor's `dompurify`, `mermaid` and `react-router` (the monitor now uses React Router 7.18).
  The tests' `testcontainers` moved to 0.28, dropping a vulnerable `tokio-tar` and an unmaintained
  crate, and the screenshot tool's lockfile took a fixed `quinn-proto`. The Docker image now runs as
  an unprivileged user and declares a `HEALTHCHECK` against `/healthz`.
- **CI failed on `main` on all three platforms (FEAT-129).** The `rust` job tested a binary with
  no monitor inside, because it never built `web/dist`; it now builds the web app first. The docs
  guard compared a file path it had not resolved against a repository root it had, so on macOS —
  where the temp folder is a symlink — a loose note inside the repository was let through; both are
  now resolved on disk before comparing. And every binary that links libgit2 now also links
  Windows' `advapi32`, which the vendored libgit2 needs and the current MSVC no longer adds itself. With that
  linked, every command then overflowed the 1 MiB stack Windows gives a program's main thread
  (`--version` included, in a debug build); the CLI now runs on a thread with a 16 MiB stack on
  every platform.
  With that, three Windows-only defects surfaced: a `.kanbanr` marker written on Windows stored
  `..\app.kanbanr`, which Linux and macOS cannot read — markers now always use forward slashes —
  and messages named paths in Windows' verbatim form (`\\?\C:\…`), which is now dropped.
  CI's test step now runs with `--no-fail-fast`, so one platform's failures all show in one run
  rather than one test binary at a time.
  The release workflow also named the retired `macos-13` runner for the Intel macOS build, which
  would have left that job waiting for a runner that no longer exists; it now uses `macos-15-intel`.
- **Board commits went out as `kanbanr <kanbanr@local>` (FEAT-128).** A new data repository was
  given that placeholder identity in its own git config and committed with it before
  `init --author/--email` recorded anyone, so every board's first commit was nobody's; a board set
  up without those flags kept committing as nobody, because the placeholder also hid the user's own
  git identity. Now the identity is recorded before the first commit, the user's git identity is
  used when the board has none, and with no identity at all `init` creates nothing and writes are
  refused with the command that fixes it. `doctor` reports a board still carrying the placeholder.
- **The commit guard judged the wrong repository (FEAT-127).** `cd ../other && git commit …` (or
  `git -C ../other commit …`) was judged by the branch and board of the session's folder, so a
  correct commit elsewhere could be refused as "straight to master". The guard now follows the
  command to the repository it commits in and uses that repository's board; a folder without one,
  or a directory only the shell could work out (`cd $X`), is left to that repository's git hooks.
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
- **Cadence in the monitor (FEAT-123).** Where a project uses sprints:
  - the board has a sprint selector, defaulting to the active sprint;
  - it shows a header with the goal, dates, days left, done against committed, and capacity;
  - it shows a burndown: one accent line against a dashed ideal, a tooltip on every day, and a
    table view.

  Where it uses releases, a **Releases** tab lists each release's scope, how much is finished, its
  target and its notes. The project Gantt draws sprints as bars and releases as milestones. A
  project that uses neither sees none of it.
- **A gate that didn't name `definition` let an undefined item through (FEAT-125).** The scrum
  Ready gate passed items with no definition at all. A missing definition now fails every check
  that reads it.
- **Scrum and agile, ready to use (FEAT-122).**
  - Two new presets:
    - `scrum`: Backlog → Ready → In Progress → Review → Testing → Done → Released. Ready is the
      Definition of Ready, and work starts only inside the active sprint.
    - `agile`: Plan → Design → Develop → Test → Review → Released.
  - Both switch sprints and releases on, with their defaults. Other presets leave the cadence
    alone, and a workflow file can carry its own.
  - `kanbanr config workflow --write-agreement` writes the working agreement to the board,
    generated from the gates.
  - The setup interview asks for sprint length, first start, capacity, release cadence and first
    version when the chosen process uses sprints.
- **Releases (FEAT-120).** For projects that switch them on: `kanbanr release add | plan | list |
  cut`. A release is planned up front, in `projects/<id>/releases.yaml`, and items carry `release`.
  - `release cut` ships the planned items that are finished. An item whose work is done is moved
    to its end status through that status's gate.
  - It writes release notes from the shipped items' statements and requirements to
    `releases/<version>.md` on the board.
  - Anything that didn't make it is carried to the next planned release, or back to unplanned,
    with the reason. `--tag` also tags the code repository.
  - `feature add --found-in <version>` records feedback against a shipped release.
  - A new `in_release` check lets a gate require an item to be planned into a release.
- **Sprints (FEAT-119).** For projects that switch them on: `kanbanr sprint add | plan | start |
  show | list | close`.
  - A sprint (SP-001) has a goal, dates and a capacity, and lives in `projects/<id>/sprints.yaml`.
    Items carry `sprint`.
  - Planning past capacity warns but still plans.
  - Only one sprint is active at a time.
  - Closing a sprint carries unfinished items to the next sprint or the backlog, and records what
    was carried.
  - `sprint show` gives the burndown, derived day by day from the moves items recorded. Nothing
    is stored for it.
  - `kanbanr report` includes velocity per closed sprint only where sprints are on.
  - `retro --sprint` covers one sprint.
  - A new `in_sprint` check lets a gate require an item to be in the active sprint.
- **Story points and cadence switches (FEAT-121).**
  - Items can carry `points` beside `estimate_days` (`--points` on `feature add` and `feature
    edit`).
  - A project chooses its unit with `kanbanr config cadence --unit points`. A new `estimated`
    check judges estimates in that unit, and a Definition of Ready can require it.
  - Sprints and releases are **off unless switched on** (`kanbanr config cadence --sprints on
    --releases on`), because most projects follow a different rhythm. A project that never
    switches them on carries no cadence at all on disk.
- **Processes are documented, and the decision recorded (FEAT-118).** A new book chapter,
  *Processes: presets, gates and sign-offs*, covers the presets, the gate fields, every check, how
  sign-offs work, how a definition grows stage by stage, a worked PDCA example, and how to write
  your own process file. ADR-0010, *Process is configuration: kanbanr owns the checks, the project
  owns the process*, is on the board. The skill teaches stage-by-stage definitions, and that
  sign-offs belong to the user.
- **Every surface says what the next stage needs (FEAT-117).**
  - `kanbanr check` lists, for each stage an item can move on to, what that stage is for and what
    is still missing.
  - `doctor` is stage-aware on workflows that declare gates. It reports what the next stage asks,
    not what a later stage will ask.
  - The Review page offers **Sign off** buttons for sign-offs a next stage is waiting on.
  - Board cards show the next stage and how much it still lacks.
  - The Workflow page draws the daemon's diagram, gates included, instead of a hand-kept copy of
    the exporter, and lists each stage's requirements.
  - `kanbanr claude sync` writes each stage's purpose into CLAUDE.md, so the agent grows a
    definition one stage at a time.
- **Workflow presets are data, and a project can load its own process (FEAT-116).**
  - Presets are YAML files shipped in the binary:
    - `default`: this board's shape, Planned → In Progress → Completed plus Deferred and Ongoing.
      This is now what a **new** project gets.
    - `scheduled`: the old default, Planned → Scheduled → Completed.
    - `togaf`: the definition grows phase by phase, with the branch at Implementation, a release
      sign-off, and a direct Implementation → Operations edge.
    - `pdca`.
    - `design-control`: modelled on ISO 9001 §8.3, not claimed compliant.
  - `kanbanr config workflow --preset <name>` applies one, and `--preset list` describes them.
    `--from-file` loads an organisation's own process, and `--export` writes a project's workflow,
    gates included.
  - An unknown preset name is refused with the list of known ones. `project init --workflow` used
    to fall back silently.
  - A preset's statuses and gates are replaced in one write.
  - The Mermaid export shows each declared gate as a note.
  - Existing boards keep their own workflow.
- **start, finish and auto-advance follow the workflow (FEAT-115).**
  - `start` goes to the status whose gate makes the branch: Implementation under TOGAF, not the
    phase after Vision. A jump the workflow doesn't allow is refused, naming the stages in between.
    In a folder that isn't a git repository, the item moves without a branch.
  - `finish` ends at a terminal status reachable from where the item is. It used to take the
    first terminal regardless.
  - When every task is done, the item advances to that terminal status only if its gate is met.
    Otherwise it stays and `task state` says why. It no longer depends on a status literally named
    "Completed".
- **Sign-offs (FEAT-114).** `kanbanr signoff <CODE> <name>` records a named agreement a stage can
  require, such as a design review held or a release approved: who, when, in which status, with an
  optional note and doc. A gate lists them as `signoffs: [design-review]`.
  - A sign-off is tied to the definition it covered, so changing the definition lapses it and the
    gate asks again. Earlier sign-offs are kept.
  - Approvals now also record the status they were given in.
  - `feature show` lists each sign-off and says whether it has lapsed.
- **Declarable gates (FEAT-113).** A workflow can say, per status, what an item must show before
  it enters that status. The config's `gates` map takes `purpose`, `requires`, `warns`,
  `enforce: block|warn`, `kinds` and `on_enter`, and a Zachman condition can name only the columns
  a stage needs. Unknown statuses, checks or columns are refused when the workflow is saved.
  - **No existing board changes.** With no gates declared, today's rule is synthesised exactly:
    entering a status that means "working on it" needs a definition and a current approval.
  - **Overrides are recorded.** `--override "<reason>"` (formerly `--unapproved`, which still
    works) passes a blocking gate, and the reason is kept in the move's history.
  - **Schema 3, only when needed.** A board that declares gates is stamped `schema_version: 3`, so
    an older kanbanr refuses it instead of ignoring its guardrails. Boards without gates stay at 2.
  - **Terminal statuses.** A status named "Completed" counts as terminal only where the workflow
    declares no terminal states.
- **"What is this item missing?" has one answer (FEAT-112).** `check`, `finish`, `doctor`,
  `check --file`, `query --gap` and the monitor's board cards now share one readiness engine,
  instead of five copies that had drifted apart. Each surface still asks its own set of checks,
  but a rule means the same thing and reads the same everywhere. Two drifts are corrected: the
  query no longer counts a ratified item as unapproved, and an exempt item has no gaps anywhere.
  New read routes serve the monitor: `…/features/{code}/readiness` and `…/readiness` (per live
  item).
- **Session summaries can keep private topics off the board (FEAT-124).** A private exclusion list,
  kept outside every repository, names terms that must never appear. Transcript messages that
  mention one are dropped before summarising, and summary lines that mention one are removed
  before anything is written.
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
- **The README describes kanbanr as it now is (FEAT-134).** It leads with why kanbanr exists and
  what it gives you — the method, processes and gates, sprints and releases, traceability, the
  monitor, local-first — each linked to its chapter, then the screenshots, a quick start, how kanbanr
  is developed with kanbanr, and how it works. The screenshot gallery moved to the monitor chapter,
  the command reference now lists every command, the architecture chapter covers the readiness
  engine, gates and cadence, and the quick start no longer says hooks are installed machine-wide.
- Relicensed the workspace to **MIT OR Apache-2.0** (was MIT) — the Rust-ecosystem norm.
- **kanbanr is not published to crates.io (FEAT-024, ADR-0011).** It ships only as the GitHub
  release archives, the `install.sh` / `install.ps1` installers and the GHCR image; `kanbanr-cli`,
  `kanbanr-core` and `kanbanr-server` now declare `publish = false`. `ears-classifier` is the one
  published crate. The open-sourcing guide, the release workflow's notes and the VS Code
  extension's install hint no longer point at `cargo install kanbanr`, and `.github/CODEOWNERS`
  asks the `kanbanr-maintainers` team to review every pull request.
- **Releases and the docs site come from `main` only (FEAT-024).** The release workflow refuses a
  version tag whose commit is not on `main`, before anything is built or published, and the docs
  site deploys only from `main` (a manual run elsewhere builds the book without deploying it).
- **No Dependabot version updates (FEAT-024).** `.github/dependabot.yml` is removed: on the first
  push it opened eighteen pull requests, one per major bump across cargo, the web app, the VS Code
  extension and the workflows. Dependencies are updated deliberately, and major upgrades are
  planned as board items.

### Added: the foundation
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

[Unreleased]: https://github.com/startr-trade/kanbanr/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/startr-trade/kanbanr/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/startr-trade/kanbanr/releases/tag/v0.1.0
