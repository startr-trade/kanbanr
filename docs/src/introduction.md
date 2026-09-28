# kanbanr

A kanban board that is the **system of record** for Claude-driven development. Claude manages it
through a [skill](using/everyday.md) that drives the `kanbanr` CLI, and a read-only web monitor
shows what is happening live while you work.

> The **CLI writes the local data folder directly** — the single writer, no server, no login.
> **`kanbanr serve`** runs a read-only view daemon over the same folder, with the monitor built
> into the binary. Sharing happens through a git remote. It is **one binary**, and Docker is
> optional.

## Why this exists

An AI agent doing real project work needs a durable, external system of record — not an in-session
scratchpad. Without one, the plan, the reasoning and the evidence live only in a transcript: they
vanish between sessions, cannot be reviewed, and work gets built that nobody agreed to.

kanbanr keeps all of it in plain, git-backed files a human can read, and makes Claude keep it
current as part of doing the work rather than as an afterthought.

What it commits to — work items link these by id:

| | |
|---|---|
| **G-1** | A session recovers the full plan and continues, with no human recap |
| **G-2** | Every work item records why it exists and how it will be verified |
| **G-3** | Nothing substantial gets built before its reasoning is agreed |
| **G-4** | Any line of code can be traced back to the requirement and goal it serves |
| **G-5** | The tool stays low-friction enough that it is never worth bypassing |
| **G-0** | The system stays operable and maintainable |
| **G-6** | What is installed is what was built, and can be shown to be |

## Where to start

- **[Installation](getting-started/installation.md)** — one command; the monitor is in the binary.
- **[Set up in 60 seconds](getting-started/quickstart.md)** — a board, from nothing.
- **[How the pieces fit](getting-started/how-it-fits.md)** — the CLI, the daemon and the data folder.
- **[The method](using/the-method.md)** — why an item exists, and what proves it done.

## Constraints it works under

- Local-first: plain YAML and markdown in a git repo. No database, no accounts, no server required.
- The CLI is the single writer; the web monitor is read-only.
- Sharing happens through a git remote, which is also the access boundary.
- One binary, installable without a toolchain.
- Raw data is never discarded — it is captured as files and kept. Every summary, count and report
  is derived from it.
