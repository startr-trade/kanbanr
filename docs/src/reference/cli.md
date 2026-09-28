# Command reference

## Complete command reference

Everything the CLI does, grouped by what you are trying to find out. `--json` works on every read.

**Setting up and looking around**

| Command | What it does |
|---|---|
| `kanbanr init <name>` | data folder + git repo + identity + project, in one step |
| `kanbanr project init/edit/list/use/delete` | create, rename, select (writes the `.kanbanr` marker) |
| `kanbanr where [--json]` | which board folder this directory uses, and why |
| `kanbanr whoami` / `kanbanr identity` | the commit identity this data folder writes as |
| `kanbanr config show / set-transition / displayed-states / default-state / no-op-states / workflow` | the workflow |
| `kanbanr hooks install / status / uninstall` | the Claude Code hooks (session start, stop nudge, test capture, commit guard) |
| `kanbanr serve [--ui-dir …]` | the read-only monitor over this board |

**The work**

| Command | What it does |
|---|---|
| `kanbanr board` / `kanbanr feature list / show / add / edit` | the kanban and its items |
| `kanbanr move <CODE> <STATUS> [--unapproved "…"]` | a status change, validated against the workflow |
| `kanbanr milestone add / list / edit / delete` | milestones (a dependency DAG; cycles rejected) |
| `kanbanr todo add / list`, `kanbanr task add / state / list` | persistent todo-lists on an item |
| `kanbanr export <CODE> --format md\|json` | one item, rendered for a human or a machine |
| `kanbanr query "text" [--goal G-1] [--gap …] [--all-projects]` | rich filters plus full text, across projects |
| `kanbanr activity` / `kanbanr events` | the changelog, and the notification event log |
| `kanbanr doc folder / add / tree / list / show / rm` | the documentation tree |

**Dependencies and scheduling**

| Command | What it does |
|---|---|
| `kanbanr ready` / `kanbanr blocked` | what can be started now, and what is waiting on something |
| `kanbanr impact <CODE>` | everything downstream of an item — what breaks if it slips |
| `kanbanr graph [--format dot\|json]` | the dependency graph |
| `kanbanr critical-path` / `kanbanr gantt` | the longest chain, and a Mermaid schedule |
| `kanbanr portfolio …` | cross-project rollups for a program of several boards |

**Keeping it honest**

| Command | What it does |
|---|---|
| `kanbanr doctor` | every broken reference and every gap, across the board |
| `kanbanr check [CODE]` | what one item has not said, and what it cannot yet show |
| `kanbanr review [CODE] [--pending] [--ui]` | the decision brief — one item, all of them, or in the browser |
| `kanbanr approve <CODE> [--by …]` | records agreement, attributed to the board's commit identity |
| `kanbanr unapprove <CODE> --reason "…"` | takes an agreement back; the record of having given it stays |
| `kanbanr capture` | reads a test run's output (run by the hook; you never call it) |
| `kanbanr split-from <CODE> <PARENT>` | records that an item was sliced out of another |
| `kanbanr sources [--write]` | imported items whose source file has gone |
| `kanbanr index` | rebuild the per-project cache from the source-of-truth files |

**Sharing**

| Command | What it does |
|---|---|
| `kanbanr remote add / list / remove`, `kanbanr sync` | git remotes for the board, and an immediate push |
| `kanbanr mirror enable / disable / status / sync / link / pull` | one-way mirror of items to GitHub issues |
| `kanbanr batch [--dry-run] [--file …]` | many changes in one call and one commit |
