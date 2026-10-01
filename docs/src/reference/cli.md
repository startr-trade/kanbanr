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
| `kanbanr config show / set-transition / displayed-states / default-state / no-op-states / workflow` | the workflow; `config workflow --preset <name>`, `--from-file`, `--export`, `--write-agreement` ([Processes](../using/processes.md)) |
| `kanbanr config rename-status <OLD> <NEW>` | rename a status and move every item in it |
| `kanbanr config cadence [--sprints on\|off] [--releases on\|off] [--unit points\|days]` | switch sprints, releases and the estimate unit on or off for a project |
| `kanbanr hooks install / status / uninstall` | the Claude Code hooks (session start, stop nudge, test capture, commit guard) |
| `kanbanr serve [--ui-dir …]` | the read-only monitor over this board |
| `kanbanr open` | open the running monitor in a browser |
| `kanbanr self-update [--check]` | replace this binary with the release's, checksum-verified |

**The work**

| Command | What it does |
|---|---|
| `kanbanr board` / `kanbanr feature list / show / add / edit` | the kanban and its items |
| `kanbanr move <CODE> <STATUS> [--override "…"]` | a status change, validated against the workflow and the stage's gate; `--override` passes a gate and records why |
| `kanbanr start <CODE>` / `kanbanr finish <CODE>` | begin (branch, active stage) and end an item, each through its gate |
| `kanbanr milestone add / list / edit / delete` | milestones (a dependency DAG; cycles rejected) |
| `kanbanr todo add / list`, `kanbanr task add / state / list` | persistent todo-lists on an item |
| `kanbanr export <CODE> --format md\|json` | one item, rendered for a human or a machine |
| `kanbanr query "text" [--goal G-1] [--gap …] [--all-projects]` | rich filters plus full text, across projects |
| `kanbanr activity` / `kanbanr events` | the changelog, and the notification event log |
| `kanbanr doc folder / add / tree / list / show / rm` | the documentation tree |

**Process: definitions, agreement and sign-off**

| Command | What it does |
|---|---|
| `kanbanr charter show / set` | why the project exists: purpose, goals, non-goals, constraints |
| `kanbanr feature define <CODE> [--file …] [--template]` | an item's definition: statement, goals, Zachman, requirements and their tests |
| `kanbanr signoff <CODE> <name>` | record a named sign-off a stage's gate asks for, pinned to the current definition |
| `kanbanr ratify <CODE>` | agree after the fact to work started under a recorded override |
| `kanbanr test <CODE> <R-n> <test> planned\|red\|green` / `kanbanr tests` | move a test along its lifecycle; list tracked tests and check each still exists |
| `kanbanr defect <CODE> …` | what a defect cost and where it came from |

**Sprints and releases** (projects with cadence switched on)

| Command | What it does |
|---|---|
| `kanbanr sprint add / list / show / plan / start / close` | timeboxes: add one, list them, see one with its burndown, plan items in, start, close (carrying unfinished work) |
| `kanbanr release add / list / plan / cut` | releases: add one, list them, plan items in, cut it (ship what is finished, write notes, carry the rest) |
| `kanbanr retro --sprint <SP-…>` | how a sprint went |

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

**Reasoning and evidence**

| Command | What it does |
|---|---|
| `kanbanr why <file>:<line>` | the item, requirement and goal behind a line of code |
| `kanbanr trace <G-…\|CODE\|CODE/R-n>` | what hangs off a goal, an item or a requirement, and what is missing |
| `kanbanr adr new / list / supersede / history` | architecture decisions, linked to the items they affect |
| `kanbanr lessons [--for CODE]` / `kanbanr lesson add / affirm / contradict` | what the project has learned, and judging it |
| `kanbanr retro <MS-…>` / `kanbanr report` | how a milestone went; flow and quality derived from the record |

**Code and Claude Code**

| Command | What it does |
|---|---|
| `kanbanr git install-hooks / uninstall-hooks / status` | the commit hooks that require each commit to name its item |
| `kanbanr commit -m "…"` | commit with the board reference filled in from the branch |
| `kanbanr claude sync` | write the project's charter and stages into `CLAUDE.md` |
| `kanbanr git guard` / `kanbanr claude guard` / `kanbanr git check-msg` / `kanbanr git check-branch` | the checks the hooks run (you never call them) |

**Sharing**

| Command | What it does |
|---|---|
| `kanbanr remote add / list / remove`, `kanbanr sync` | git remotes for the board, and an immediate push |
| `kanbanr mirror enable / disable / status / sync / link / pull` | one-way mirror of items to GitHub issues |
| `kanbanr batch [--dry-run] [--file …]` | many changes in one call and one commit |
