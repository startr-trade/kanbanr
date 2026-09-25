# kanbanr

A kanban-based task manager for **Claude-driven development**. You manage the board through a
Claude **skill** (which drives the `kanbanr` CLI), and a **read-only web monitor** shows what's
happening live as you work in VS Code.

## Screenshots

The web monitor is a **read-only** live view of your local data folder — the board updates over SSE
as the CLI (driven by Claude) changes things. No refresh, no login.

<p align="center"><img src="docs/images/board.png" alt="kanbanr board" width="860"></p>

A page per view — feature items (spec + persistent todo-lists), milestones, the derived schedule,
status pages, the off-board "Ongoing" stream, and per-project docs:

<table>
  <tr>
    <td><img src="docs/images/feature.png" alt="Feature item" width="260"></td>
    <td><img src="docs/images/milestones.png" alt="Milestones" width="260"></td>
    <td><img src="docs/images/schedule.png" alt="Schedule" width="260"></td>
  </tr>
  <tr>
    <td align="center"><sub>Feature item — spec + todo-lists</sub></td>
    <td align="center"><sub>Milestones</sub></td>
    <td align="center"><sub>Schedule (derived)</sub></td>
  </tr>
  <tr>
    <td><img src="docs/images/status.png" alt="Status page" width="260"></td>
    <td><img src="docs/images/ongoing.png" alt="Ongoing stream" width="260"></td>
    <td><img src="docs/images/home.png" alt="Project tiles" width="260"></td>
  </tr>
  <tr>
    <td align="center"><sub>Status page (grouped by feature)</sub></td>
    <td align="center"><sub>"Ongoing" — off-board work</sub></td>
    <td align="center"><sub>Home — project tiles</sub></td>
  </tr>
  <tr>
    <td><img src="docs/images/docs.png" alt="Docs tree" width="260"></td>
    <td><img src="docs/images/doc-file.png" alt="Doc with embedded image" width="260"></td>
    <td><img src="docs/images/doc-mermaid.png" alt="Doc with a Mermaid diagram" width="260"></td>
  </tr>
  <tr>
    <td align="center"><sub>Per-project docs (folder tree)</sub></td>
    <td align="center"><sub>Embedded images</sub></td>
    <td align="center"><sub>Live <b>Mermaid</b> diagrams</sub></td>
  </tr>
</table>

Docs support **embedded images** (any PNG/SVG, resolved per-folder and served from the data folder)
and **native Mermaid** diagrams. Any other diagram tool — PlantUML, Graphviz/DOT, D2, Excalidraw,
draw.io — works too: export to PNG/SVG and embed it like any image.

Light theme is the default; one toggle flips the whole UI to dark:

<p align="center"><img src="docs/images/board-dark.png" alt="kanbanr board — dark theme" width="860"></p>

<sub>Screenshots are generated reproducibly via a Selenium Grid in Docker — see
<a href="tools/screenshots/">tools/screenshots/</a> (<code>make screenshots</code>).</sub>

## Model

- **Feature items** (epics) — each has a code (e.g. `FEAT-001`), a markdown **specification**, a
  status, a **required milestone**, and one or more **persistent todo-lists** (add one per work
  session) whose tasks are tri-state (Not started / In progress / Completed).
- **Milestones** — group feature items and depend on other milestones (a dependency DAG). A
  milestone page shows its dependencies and its feature items.
- **Schedule** — a **derived view** (not a stored object): for a given feature status, the
  milestones that contain feature items in that status, dependency-ordered.
- **Configurable workflow** — statuses, allowed transitions, the dashboard's displayed states,
  and the default state for new features are all per-project.
- **Documentation** — a nested folder tree of markdown files per project, with **embedded images**
  (relative names resolve per-folder) and **Mermaid** diagrams; any other diagram tool (PlantUML,
  Graphviz/DOT, D2, Excalidraw, draw.io) works via an exported PNG/SVG.

## Architecture

```mermaid
flowchart LR
  C["Claude (skill)<br/>in VS Code"] -->|runs commands| CLI["kanbanr CLI<br/>(the only writer)"]
  CLI -->|commit| D[("data/ folder<br/>git repo · YAML + markdown")]
  D -->|pull / push| R["git remote<br/>(sharing)"]
  D -->|read| S["kanbanr serve<br/>read-only · localhost"]
  S -->|"read API + SSE"| UI["React monitor<br/>(future: VS Code ext)"]
```

- **One binary, no Docker required.** `kanbanr` writes the data folder directly (the CLI *is* the
  writer — no server, no login, no accounts), and `kanbanr serve` runs a read-only **view daemon**
  over the same folder. Docker is just one optional packaging.
- The data folder is a **git repository**: every write is a commit authored by the configured
  identity (`kanbanr identity`); optional remotes are pulled + pushed after each commit. libgit2 is
  linked in — no external `git` binary. **Sharing/centralization is the git remote's job** (e.g.
  GitHub), so kanbanr has no accounts of its own.
- The **monitor is a localhost, read-only view** of your local folder — no auth. The viewer is
  pluggable (today a React SPA; a VS Code extension is on the roadmap).
- The shared `kanbanr-core` (incl. a `dispatch` router) is the single engine used by both the
  writer and the view daemon.

## How Claude uses it

Say **"start using kanbanr for this project"** once, and Claude adopts kanbanr as the **single
system of record** for the rest of the project — no further instruction needed. Everything about
the project's activity lives in kanbanr (the only thing kept outside it is your conversation
transcript); it uses **persistent todo-lists on feature items** instead of ephemeral session
lists, **recovers/resumes** state at the start of each session, keeps the board updated **before
and after every task**, reviews a feature's spec for **staleness** when moving it out of
`Deferred`, **never deletes** feature items (moves them — e.g. to a no-op state), and bundles
multiple changes into a single **`kanbanr batch`** call. The contract lives in
[skill/kanbanr/SKILL.md](skill/kanbanr/SKILL.md).

The skill and its enforcement hooks are also packaged as a **Claude Code plugin** — install both in
one step with `claude plugin marketplace add startr-trade/kanbanr` then `claude plugin install kanbanr`
(the `kanbanr` binary ships separately; see [docs/OPEN_SOURCING.md](docs/OPEN_SOURCING.md)).

## Repository layout (monorepo)

```
kanbanr/
├── api/      Rust workspace — kanbanr-core (engine: store · dispatch · git · activity) ·
│             kanbanr-cli (the one `kanbanr` binary: local writer + `serve`) ·
│             kanbanr-server (view-daemon library used by `kanbanr serve`)
├── web/      React + Vite read-only monitor (built SPA, served by `kanbanr serve`)
├── docker/   Optional single-image Dockerfile (runs `kanbanr serve`)
├── docs/     USER_GUIDE.md, DESIGN.md, ROADMAP.md, OPEN_SOURCING.md
├── skill/    The Claude skill (skill/kanbanr/SKILL.md)
└── data/     The data git repo: projects/<name>/… (YAML/markdown). No accounts/secrets.
```

## Quick start (60 seconds)

```bash
make install                                   # build/install the one `kanbanr` binary + skill
kanbanr init my-app --author "You" --email you@example.com   # data dir + git + identity + project
kanbanr milestone add --name Foundations --code MS-001
kanbanr feature add --title "Login flow" --milestone MS-001 --spec "# Login\nEmail + password."
kanbanr board                                  # you now have a populated board
```

That's it — the CLI writes the data folder directly (each change is a git commit). Then develop
with Claude in VS Code: say **"start using kanbanr for this project"** and it takes over the board.

### Add the live monitor (optional)

```bash
kanbanr serve --ui-dir web/dist     # the same binary, no Docker (or: make docker-up)
kanbanr open                        # opens http://localhost:8080 — no login, just the board
```

The monitor is a localhost read-only view; expose it beyond your machine only behind a reverse
proxy you control. See **[docs/USER_GUIDE.md](docs/USER_GUIDE.md)**.

## Tests

```bash
make test     # unit tests + Docker-less integration: the real `kanbanr` CLI drives local writes,
              # and `kanbanr serve` serves them read-only over a port — no Docker.
make itest    # packaging smoke (testcontainers): the image boots and serves the view (no auth).
```

See **[docs/USER_GUIDE.md](docs/USER_GUIDE.md)**, **[docs/DESIGN.md](docs/DESIGN.md)**,
and **[docs/ROADMAP.md](docs/ROADMAP.md)**. The project's own assessment, decisions and proposals
live on its board (`kanbanr adr list`, `kanbanr doc tree`) rather than in this repository — they are
reasoning about the work, not part of the shipped artifact.
