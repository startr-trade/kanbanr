---
name: kanbanr
description: >-
  The project management system of record for Claude-driven development. Use it whenever doing
  ANY work on a project that is tracked with kanbanr — and adopt it for the whole project the
  moment the user says "start using kanbanr for this project", then keep using it for the rest of
  the project's life with no further instruction. Use to: scope work as feature
  items with specs; group them into milestones; plan via the workflow (statuses, allowed
  transitions, displayed/no-op states); create persistent todo-lists with items on a feature and
  mark them Not started / In progress / Completed; move feature items between states; record
  documentation; and recover/resume the full project state at the start of a session. Drives the
  `kanbanr` CLI, which writes the local data folder directly (git-backed); a read-only web monitor
  views it live.
---

# kanbanr — the project's system of record

kanbanr tracks development work as **feature items** (epics with a spec + persistent todo-lists),
grouped into **milestones** (a dependency DAG). The data is durable, git-backed YAML/markdown in a
local folder; a read-only web monitor renders it live.

## Activation (one phrase, whole project)

When the user says **"start using kanbanr for this project"** (or anything equivalent):
0. **One-shot setup.** The fastest path is `kanbanr init <name> --author "Name" --email you@x` —
   it creates the local data dir + git repo, sets the commit identity, scaffolds the project, and
   selects it here. (Equivalently: `kanbanr identity …` once, then `kanbanr project init <name>`.)
1. Ensure a project exists (`kanbanr project list`; `kanbanr project init <name>` if needed).
2. Select it for this directory: `kanbanr project use <name>` (writes a `.kanbanr` marker).
3. From that point on, treat kanbanr as the **system of record for the entire project** — no
   further prompting required.

If a `.kanbanr` marker (or `$KANBANR_PROJECT`) is already present, the project is already tracked —
behave as if activated. Every change you make is committed to the data repo **authored by the
configured identity** (`kanbanr identity`); sharing/centralization is via git remotes.

## The prime directive: one system of record

**kanbanr is the single source of truth for everything about the project's activity.** The ONLY
project information allowed to live outside kanbanr is the raw conversation transcript itself.

- **Never** track project work in an ephemeral / in-session todo list. Do not use the session
  scratch todo for project tasks. Instead, create a **persistent todo-list on the relevant
  feature item** (`kanbanr todo add …` then `kanbanr task add …`).
- All scope, specifications, progress, task status, decisions, and documentation go INTO kanbanr.
- Because everything is persisted, the project's state is fully **recoverable and resumable**
  across sessions — nothing is lost when a session ends.

## Recover & resume (start of every session)

Before doing project work in a new or resumed session, **recover the state from kanbanr first**:
run `kanbanr board` and read the relevant feature items (`kanbanr feature show …`,
`kanbanr todo list …`). Continue exactly where the project left off; do not restart from memory.

## Keep the tool updated — before AND after every task

Treat kanbanr updates as part of doing the work, not an afterthought:

- **Scope first.** Turn requested work into **feature items** with clear `--spec` markdown (and
  milestones as needed). Each FI starts in the project's default state.
- **Before starting** a chunk of work on a feature: make sure it has a **todo-list** for this
  effort with its **items**, and mark the item you're about to do as **InProgress**.
- **Spec-staleness check:** when moving a feature item **from `Deferred` into any active state**
  (anything that is not Completed and not a no-op state), first **review the feature's
  specification for staleness** and update it if it no longer reflects reality — *before*
  proceeding with the work.
- **After finishing** a task: mark its item **Completed**, update the **specification** and any
  **documentation** affected, and **move** the feature's status as appropriate. When every task
  across all of a feature's todo-lists is Completed it auto-advances to "Completed".

## Feature items are permanent

Feature items are the project's work and are **never deleted** — they only **move between
states**. To take a feature out of play, move it to a **no-op state** (e.g. `Out-of-Scope`,
`No Action`, `Not Applicable`), never delete it. (A project that has any feature items therefore
cannot be deleted, which protects work from being erased.)

## Modeling ALL kinds of work (not just features)

A "feature item" is really a generic **work item** — use ONE item type for everything, and convey
*kind* in the title until labels exist (e.g. `Chore: …`, `Refactor: …`, `Bug: …`, `Docs: …`).

- **One-off work** (a feature, refactor, chore, or bug): a work item that moves through states to
  **Completed** once.
- **Implementation-incidental cleanup** (you tidy while building): a **task in that item's
  todo-list**, not a separate item. Standalone/deliberate cleanup (tech-debt): its **own** item.
- **Recurring / routine work** (dep bumps, audits, periodic reviews): a **permanent item that is
  never Completed**, accruing a **new todo-list per occurrence**.
- **Triggered "keep-in-sync" work** (an invariant a change can break — e.g. *SKILL.md and the docs
  must match the system*): when a change affects a documented contract, the sync is part of that
  change's **definition of done** — add a todo-list task and complete it before the item is done,
  or log it on a standing "docs/contract sync" item. Never leave the docs drifting from the code.

**Populate the work-item fields as you scope.** When creating/editing a feature item, set these
(via `feature add/edit` flags or `batch`) so the board is genuinely useful — don't leave them blank:
- `--kind` — `feature` | `chore` | `bug` | `refactor` | `docs` | `recurring` (what sort of work).
- `--priority` — `low` | `medium` | `high` when you know it.
- `--labels a,b` — lightweight grouping/tags (area, component).
- `--due` — only if there's a real deadline.
- `--depends-on FEAT-004,FEAT-007` — other features this one is **blocked by** (validated; no cycles).
Set `--kind` at minimum on every item; it's what lets the board separate features from chores/bugs.

**Focus vs non-focus.** The dashboard is for **focus** (delivery) work. Recurring/ongoing and
maintenance items are **non-focus background** — keep them OFF the board by putting them in a
**non-displayed status** (e.g. an `Ongoing` status not in `displayed_states`); they still have a
status page and an "other states" link. To set this up: add an `Ongoing` status via
`kanbanr config workflow` with `--displayed-states` that omits it, and move ongoing items there.

## Prefer bulk: bundle changes into ONE call

When you have several changes to make, send them in a single `kanbanr batch` call rather than
many commands. A bundle can include, in one request: new feature items, edits to existing ones
(spec changes, moving milestones, status moves), documentation changes, new todo-lists with
items, and task-state updates. Newly-created items get a `ref` alias that later operations in the
same bundle reference.

```bash
# Pass the bundle on stdin (or --file bundle.json). Codes are server-assigned; use "ref".
kanbanr batch <<'JSON'
{"operations":[
  {"op":"milestone.add","ref":"m1","name":"Auth"},
  {"op":"feature.add","ref":"login","title":"Login flow","milestone":"m1","spec":"# Login\n…"},
  {"op":"feature.move","code":"login","to":"Scheduled"},
  {"op":"todo.add","ref":"s1","feature":"login","description":"session 1"},
  {"op":"task.add","feature":"login","todo":"s1","text":"build form"},
  {"op":"task.state","feature":"login","todo":"s1","key":"T1","state":"InProgress"},
  {"op":"doc.write","path":"design/overview","content":"# Overview\n…"}
]}
JSON
```
Batch op types: `feature.add`, `feature.edit`, `feature.move`, `milestone.add`, `todo.add`,
`task.add`, `task.state`, `doc.folder`, `doc.write`. Operations apply in order; on failure the
response names the failing operation index. The whole bundle is **one git commit** — pass
`--message "…"` to title it (a default is framed if you omit it).

---

## How it runs (local-only, no server needed)

kanbanr is a **local-only** tool: the CLI writes the data folder directly (no server, no login, no
accounts). Each write is a git commit authored by the configured identity; sharing/centralization
is via **git remotes**.

- `kanbanr identity --name "You" --email you@example.com` — set the commit identity (once).
- `kanbanr whoami` — show the identity. `kanbanr activity` — recent changes (from the changelog).
- The data dir resolves from `--data-dir` / `$KANBANR_DATA_DIR` / `./data`. The active project from
  `--project` / `$KANBANR_PROJECT` / a `.kanbanr` marker / the directory name. Add `--json` for
  machine-readable output.

**Git remotes** (sharing — the access boundary is the remote, e.g. GitHub):
```
kanbanr remote add origin git@host:org/data.git ; kanbanr remote list ; kanbanr remote remove origin
```
Commits are pulled + pushed to remotes after each write. **Merge conflicts are left for normal git
resolution** in the data folder (kanbanr never auto-resolves).

**The live monitor** is a separate, read-only **view** of the local folder — no auth, localhost:
```
kanbanr serve --ui-dir web/dist     # run the view daemon (the same one binary; no Docker)
kanbanr open                        # open it in the browser
```

## Command reference

Setup / configuration:
```
kanbanr project init <name> [--description D] [--statuses A,B,C] [--default-state A] [--displayed-states A,B] [--no-op-states X,Y]
kanbanr project edit <name> [--name N] [--description D]
kanbanr project list
kanbanr project use <name>        # mark this directory as tracked by <name>
kanbanr project delete <name>     # ONLY if it has no feature items and no milestones
kanbanr config show
kanbanr config set-transition <from> <to> --allow      # or --deny
kanbanr config displayed-states Planned,Scheduled,Completed
kanbanr config default-state Planned
kanbanr config no-op-states "No Action,Not Applicable,Out-of-Scope"   # inert dispositions
kanbanr config workflow --defaults | (--statuses … --transitions "A>B" --default-state … --displayed-states … --no-op-states …)
```

Feature items (a milestone is REQUIRED; code auto-generates):
```
kanbanr feature add --title "Login flow" --milestone MS-002 --spec "# Login\n…"   # or --spec-file path.md
kanbanr feature list [--status Scheduled] [--milestone MS-002]
kanbanr feature show FEAT-001
kanbanr feature edit FEAT-001 [--title …] [--spec … | --spec-file …] [--milestone MS-003] [--code NEW]
kanbanr move FEAT-001 Scheduled
kanbanr export FEAT-001 --format md
```

Todo-lists & tasks (persistent; add a NEW todo-list per work session; keys unique within a list):
```
kanbanr todo add FEAT-001 --description "session 1"     # -> TL-001
kanbanr todo list FEAT-001
kanbanr task add FEAT-001 TL-001 --text "Build the login form"
kanbanr task state FEAT-001 TL-001 T1 InProgress        # NotStarted | InProgress | Completed
kanbanr task list FEAT-001 [TL-001]
```

Milestones (dependency DAG; cycles rejected) and documentation:
```
kanbanr milestone add --name "Auth" [--code MS-002] [--depends-on MS-001]
kanbanr milestone list ; kanbanr milestone edit MS-002 […] ; kanbanr milestone delete MS-002  # only if unreferenced
kanbanr doc folder design --name "Design" --description "…"
kanbanr doc add design/overview.md --file notes.md      # or --content "# Title\n…"
kanbanr doc tree | list | show <path> | rm <path>
```

The "schedule" is a **derived view** (milestones grouped by feature status) — there is no
schedule command. Use `kanbanr board` to see the kanban; surface CLI error messages to the user
(bad transition, missing milestone, dependency cycle, auth) rather than retrying blindly.

See `docs/USER_GUIDE.md` and `docs/DESIGN.md` for the full guide and diagrams.
