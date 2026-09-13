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
  documentation (every doc, requested or self-initiated, goes into kanbanr docs, NOT the working
  folder, unless the user asks for it in the project folder, e.g. an mdBook/MkDocs site or
  README deliverable); and recover/resume the full project state at the start of a
  session. Drives the
  `kanbanr` CLI, which writes the local data folder directly (git-backed); a read-only web monitor
  views it live.
---

# kanbanr — the project's system of record

kanbanr tracks development work as **feature items** (epics with a spec + persistent todo-lists),
grouped into **milestones** (a dependency DAG). The data is durable, git-backed YAML/markdown in a
local folder; a read-only web monitor renders it live.

## Activation (one phrase, whole project)

When the user says **"start using kanbanr for this project"** (or anything equivalent):
1. **Check whether it's already set up.** Run `kanbanr where --json`. If `source` is `marker` and
   `exists` is true, the project is already tracked: skip to step 4.
2. **Ask the user where to keep the board.** The board is its own git repo and should live
   **outside** the project folder, so it never nests inside the project's git repo. Ask one
   question (e.g. with AskUserQuestion). Options, taken from `kanbanr where --json`:
   - `suggested_data_dir`: a new folder next to the project's git repo, named `<repo>.kanbanr`
     (recommended, list it first);
   - each of `existing_data_dirs`: an existing kanbanr folder, shared with its other projects
     (one data folder is needed for portfolio views, cross-project dependencies and the
     cross-project Gantt);
   - or a path the user types.
   Never pick for the user silently. If the chosen folder is inside a git repo (warned by
   `init`, or `suggested_inside_git_repo` is set), say so and confirm before continuing.
3. **Set up in one command:**
   `kanbanr init <name> --data-dir <chosen folder> --author "Name" --email you@x`.
   It creates the data folder + git repo, sets the commit identity, scaffolds the project, and
   writes a `.kanbanr` marker in the project folder recording the project and the board's
   location (e.g. `data_dir: ../app.kanbanr`). Every `kanbanr` command run anywhere inside the
   project then finds the board: no env vars needed.
   `init` also registers kanbanr's Claude Code hooks (session-start board recovery, stop-time
   reminder) in the global Claude Code settings, once per machine; relay what it printed. If it
   says the skill isn't installed where the hooks expect it, pass that on to the user.
   If the Claude Code sandbox is on, writes outside the project folder can be blocked: tell the
   user to allow the board folder (e.g. `sandbox.filesystem.allowWrite`).
4. **Offer to import existing tasks** (see "Importing existing task trackers" below).
5. From that point on, treat kanbanr as the **system of record for the entire project** — no
   further prompting required.

If a `.kanbanr` marker (or `$KANBANR_PROJECT`) is already present, the project is already tracked —
behave as if activated. Every change you make is committed to the data repo **authored by the
configured identity** (`kanbanr identity`); sharing/centralization is via git remotes.

## Importing existing task trackers

At activation (and whenever the user asks), check whether the project already tracks work
somewhere else, so it isn't lost and doesn't stay a second, conflicting plan.

1. **Look for trackers:**
   - Markdown: `TODO.md`, `TASKS.md`, `ROADMAP.md`, `PLAN.md`, `BACKLOG.md`, `- [ ]` checklists
     in docs.
   - Plans from AI dev tools: Spec Kit `specs/*/tasks.md`, Kiro `.kiro/specs/*/tasks.md`, plan
     files under `.claude/`.
   - GitHub issues and milestones, when the repo has a GitHub remote and `gh` is logged in
     (`gh issue list --state open --limit 500 --json number,title,body,labels,milestone,url`);
     GitLab likewise with `glab`.
   - Exports the user points you to (Jira or Linear CSV).
   Skip `TODO:`/`FIXME` code comments unless the user asks for them.
2. **Ask before importing.** List what you found (source and item counts) and ask which to
   import. Default to **open and in-progress items only**; bring finished items in (as Completed)
   only if the user wants that history. Never import silently.
3. **Map into one bundle:**
   - Epics / milestones → `milestone.add`; issues / stories / top-level items → `feature.add`
     (description → `spec`); sub-checklists → `todo.add` + `task.add`; labels, priority, due,
     "blocked by" → fields and `depends_on`.
   - Map statuses onto the project's workflow. Flag any you can't map clearly to the user
     instead of guessing.
   - On every imported `feature.add`, set `source` and `original`:
     `"source": {"system": "file", "ref": "TODO.md:14"}` or
     `{"system": "github", "ref": "owner/repo#123", "url": "…"}`, and `"original"` = the item's
     original text, verbatim. kanbanr preserves `original` in the spec under "Imported from",
     stamps the import time, derives a re-import key, and records the project commit for file
     sources. The source is **history, not a live link**: the item stays meaningful even after the
     file is deleted or the issue is gone.
   - For GitHub issues, also set `"issue": {"system": "github", "repo": "owner/repo", "number":
     123, "url": "…"}` so the GitHub mirror updates those issues instead of creating duplicates.
   - Before retiring a whole file, copy it into the bundle with
     `{"op": "doc.write", "path": "imports/<YYYY-MM-DD>-<file name>", "content": "…"}`.
4. **Preview, confirm, apply:** run `kanbanr batch --dry-run --file bundle.json` (writes nothing)
   and show the user what will be created and skipped. After they confirm, run
   `kanbanr batch --file bundle.json --message "import: …"`: one commit. Re-running is safe:
   items already imported (same source key), milestones with the same name, and the ops under
   skipped items are skipped.
5. **Retire the old tracker, only with a yes.** Ask what to do with it: leave it; replace the file
   with a short pointer to kanbanr; delete it; for GitHub issues, comment with the kanbanr code
   (`gh issue comment`) and/or close them (`gh issue close`), or keep them in step with the
   GitHub mirror instead. Never edit, delete, comment on or close anything without explicit
   approval.

Later, `kanbanr sources` (run in the project folder) shows whether file sources still exist;
`kanbanr sources --write` records the ones that are gone, and the monitor labels them.

## GitHub issue mirror (optional)

kanbanr can keep GitHub issues in step with the board, **one way**: kanbanr is the source of truth
and the issues follow it. Offer it when the project lives on GitHub and people follow its issues;
enable it only with the user's yes.

- **Enable:** `kanbanr mirror enable --repo owner/repo` (needs `gh` installed and logged in).
  For a **public** repo it refuses unless you add `--allow-public`: first tell the user that specs,
  todo-lists and notes will be publicly visible, and get an explicit OK.
- **Then it's automatic.** After every kanbanr write, changed features are pushed:
  - a new feature gets a new issue;
  - title, spec, labels, todo-lists and status changes update the issue;
  - Completed closes it as completed; a no-op state closes it as not planned.

  If a push fails (no network, `gh` logged out), the CLI prints a warning and the write still
  succeeds; run `kanbanr mirror sync` later. `KANBANR_MIRROR=off` skips auto-sync for a session.
- **Existing features** created before enabling get issues only with `kanbanr mirror sync --all`.
  Ask first: it can create many issues. `kanbanr mirror status` shows the plan without calling
  GitHub.
- **No duplicates for imported issues:** features imported from GitHub carry an `issue` link, so
  the mirror updates those same issues. `kanbanr mirror link FEAT-012 45` links an existing issue
  by hand; tell the user its title and body will be replaced by kanbanr's on the next sync.
- **Changes on GitHub don't flow back automatically.** When the user asks, or before a sync that
  would overwrite edits, run `kanbanr mirror pull FEAT-012`: it shows whether the issue was edited
  on GitHub since the last sync, its current title/body/labels, and new comments. Bring what
  matters into kanbanr (spec, tasks, status) deliberately; the next sync then pushes the result.
- `kanbanr mirror disable` turns it off; issue links are kept.

## The prime directive: one system of record

**kanbanr is the single source of truth for everything about the project's activity.** The ONLY
project information allowed to live outside kanbanr is the raw conversation transcript itself.

- **Never** track project work in an ephemeral / in-session todo list. Do not use the session
  scratch todo for project tasks. Instead, create a **persistent todo-list on the relevant
  feature item** (`kanbanr todo add …` then `kanbanr task add …`).
- All scope, specifications, progress, task status, decisions, and documentation go INTO kanbanr.
- Because everything is persisted, the project's state is fully **recoverable and resumable**
  across sessions — nothing is lost when a session ends.

## Where documentation goes (default: kanbanr, not the working folder)

Every document goes into kanbanr, **whether the user asked for it or you are creating it on your
own initiative**, unless the user asks for it to be stored in the project folder. Decide where a
doc goes **before** creating any file:

- **Default → kanbanr docs.** Design notes, architecture, decisions/ADRs, research & investigation
  write-ups, plans, runbooks, how-tos, guides, meeting notes, reports: write them with
  `kanbanr doc add <folder>/<name>.md` (create folders with `kanbanr doc folder …`; images and
  diagrams go there too). Detail that belongs to one piece of work goes in that **feature's spec**.
  Do **not** create `docs/*.md`, `NOTES.md`, `PLAN.md`, `DESIGN.md`, `ARCHITECTURE.md`, `adr/` and
  the like in the working folder.
- **Exception → the project folder, only when the user asks for it there.** Typical cases: the
  user wants documentation as a **deliverable** of the codebase: a hand-rolled mdBook / MkDocs /
  Docusaurus / Sphinx site, a README, a CHANGELOG, contributor or API docs. Once the user has set
  up such an in-repo deliverable, keeping it in sync with later code changes falls under that
  request, but don't put internal project notes into it.
  (Code comments and doc comments are part of the code, not documents, so this rule doesn't cover them.)
- **Don't ask each time.** Default to kanbanr and say where you saved it (e.g. "saved to kanbanr
  docs: `design/auth-flow.md`"). The user can ask to have it in the project folder instead.
- **One home per doc.** Never keep the same content in both places; link to the kanbanr doc path
  from specs or tasks instead of copying.

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
`--message "…"` to title it (a default is framed if you omit it). `feature.add` also accepts
`source`, `original` and `issue` (for imports; see above), and `feature.edit` accepts `source`
and `issue`. `kanbanr batch --dry-run` validates a bundle and reports what it would do without
writing anything.

---

## How it runs (local-only, no server needed)

kanbanr is a **local-only** tool: the CLI writes the data folder directly (no server, no login, no
accounts). Each write is a git commit authored by the configured identity; sharing/centralization
is via **git remotes**.

- `kanbanr identity --name "You" --email you@example.com` — set the commit identity (once).
- `kanbanr whoami` — show the identity. `kanbanr activity` — recent changes (from the changelog).
- The data dir resolves from `--data-dir` / `$KANBANR_DATA_DIR` / the nearest `.kanbanr` marker's
  `data_dir` / legacy `./data`. The active project from `--project` / `$KANBANR_PROJECT` / the
  nearest `.kanbanr` marker / the directory name. Markers are found by walking up from the current
  directory. `kanbanr where` prints the board folder in use. Add `--json` for machine-readable
  output.

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
kanbanr project use <name>        # mark this directory as tracked by <name> (keeps the marker's data_dir)
kanbanr where [--json]            # which board folder this directory uses (+ suggestions with --json)
kanbanr batch --dry-run --file b.json   # preview a bundle (e.g. an import) without writing
kanbanr sources [--write]         # imported items' sources; --write records ones that are gone
kanbanr mirror enable --repo owner/repo [--allow-public] | disable | status [--all]
kanbanr mirror sync [--all] | link FEAT-012 45 | pull FEAT-012   # GitHub issue mirror (via gh)
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
