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
0. **Write the charter first.** A project with no stated purpose cannot have its work judged
   against anything. Ask the user for the purpose, the goals (an outcome plus how you would know it
   happened), what is explicitly **not** in scope, and who it is for; then
   `kanbanr charter set --file charter.yaml`. Goals get ids (`G-1`) that every item links.
   Read it back at the start of a session with `kanbanr charter show`, alongside `kanbanr board`.
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

## Defining work: why it exists, what must be true, how it is verified

Every item the project creates carries a **definition** — this is not optional, and not only for
features. Work whose reasoning was never written down is work that can be built for seven hours
and then rejected.

**The contract, identical for every item:**

1. a **statement** — one sentence: what, for whom, why;
2. a **goal link** — the charter goal id this serves (`kanbanr charter show`);
3. the **six dimensions** — what / how / where / when / who / why, one line each;
4. at least one **requirement**;
5. **test evidence** for every requirement.

What varies by `kind` is only the *shape* of a requirement, because the thing being asserted
differs. Start from the skeleton: `kanbanr feature define <CODE> --template --kind <kind>`.

| kind | the requirement is | the evidence is |
|---|---|---|
| feature | a new behaviour, in EARS | a test per requirement |
| defect / bug | the requirement it **violates** (or the missing one it adds) | the test that reproduces it, `red` → `green` |
| chore / refactor | an **invariant**: "THE SYSTEM SHALL continue to …" | the named existing suite, still green |
| docs | the contract the documentation must match | a doc or doctor check |
| recurring | the standing obligation | the per-occurrence checklist |

**EARS** — one behaviour per requirement, independently testable:
`THE SYSTEM SHALL …` · `WHEN <trigger>, THE SYSTEM SHALL …` · `WHILE <state>, THE SYSTEM SHALL …`
· `WHERE <feature is included>, THE SYSTEM SHALL …` · `IF <undesired condition>, THE SYSTEM SHALL …`

A **quality requirement** (`kind: nfr`) carries an ISO/IEC 25010 tag *and* a measure that names the
test or benchmark checking it. A number with nothing behind it is an unsupported claim — leave it
out rather than invent one.

**Never invent an answer.** If a dimension is genuinely unknown, ask the user **one** clarifying
question. If it stays unknown, leave the field **blank** so `kanbanr doctor` flags it — a blank is
visible, a plausible guess is not. Never write `[MISSING: …]` into the data; the tools derive it.

Write the definition with the batch path (`definition` on `feature.add`/`feature.edit`) or
`kanbanr feature define <CODE> --file def.yaml`.

## Agreement before work: the approval gate

**Scope → define → present the brief → wait for the user's approval → only then write code.**

- `kanbanr review <CODE>` prints the one-screen decision brief. Show it, or its substance, and
  **stop**. Do not start implementing while the answer is still outstanding.
- The user approves with `kanbanr approve <CODE>`; ask them to, rather than approving on their
  behalf — an agent approving its own brief is the failure this exists to prevent.
- When several items are waiting, offer `kanbanr review --ui`: it opens the review queue in the
  browser with the approve button beside each brief. Reading a page of markdown in a terminal to
  make a decision is poor, and an expensive gate gets rubber-stamped, which is the same failure
  with extra steps. `kanbanr review --pending` is the terminal equivalent.
- A verdict records **who** gave it, defaulting to the board's commit identity. One with no named
  approver is refused, so never invent a value for `--by`: if it fails, the board has no identity
  and the user sets one with `kanbanr identity`.
- An approval recorded in error is withdrawn with `kanbanr unapprove <CODE> --reason "…"`. If you
  ever approve something on the user's behalf, say so and withdraw it.
- kanbanr enforces it: moving an item into an active status is refused unless its definition is
  approved. If the definition changes after approval, the approval **lapses** and must be renewed —
  so scope cannot drift silently past a yes.
- Genuinely urgent work can proceed with `kanbanr move <CODE> <status> --unapproved "<reason>"`.
  The reason is recorded on the item. Use it for real emergencies, not to avoid asking.
- **Subagents inherit, never invent.** A coordinator passes the approved definition to each
  subagent as its brief. A subagent that finds work outside it returns a **proposed change to the
  definition**, not merged code.
- The gate is dormant for projects with no charter, and for items created before the charter was
  adopted — so adopting the method never breaks an existing board.

## Code points back at the item that justifies it

One item, one branch, and every commit says what it serves. Do not commit on the default branch,
and do not start work without a branch — the branch is how everything else knows what you are
working on, including the commit check and the test capture.

```
kanbanr start FEAT-046            # branches feat/FEAT-046-<slug> and moves the item to active
kanbanr commit -m "feat(x): …"    # fills in `Refs: kanbanr:FEAT-046` from the branch
kanbanr commit -m "…" --ref R-2   # better: the requirement this change exists to satisfy
kanbanr finish                    # refuses while tasks are open or requirements unproven
```

- Reference the **requirement** (`kanbanr:FEAT-046/R-2`) or the **task**
  (`kanbanr:FEAT-046/TL-001/T3`) when you know which one the change serves; the item alone is the
  floor, not the goal. One commit may reference several items — a cross-cutting change needs no
  artificial split.
- **Never** pass `--no-verify`. If a commit genuinely serves no item, say so on the record:
  `[no-ref] <why>` in the message passes the check and leaves the reason in git history.
- `spike/<name>` branches are for exploring. Their output is a **change to a definition**, never
  merged code — `kanbanr finish` refuses them.
- `kanbanr git install-hooks` puts the checks in the repo (an existing hook is kept and chained
  with `--force`). The board's own data folder is exempt: kanbanr authors those commits itself.

## Tests are evidence, not intentions

Work test-first: create each test entry as `planned`, set it `red` when the failing test exists,
and `green` only when it actually passes. Flip states in the same bundle as the task update, never
as an afterthought. A requirement with no test is not ready, and an item is not done because its
checkboxes are ticked — it is done when its requirements have passing tests.

**A green is recorded by the run, not by you.** A PostToolUse hook reads the output of every test
command you run and flips the tracked tests to match what actually happened, stamped with the
project revision it was observed at. So:

- Name the test entry exactly as the runner prints it (`cart::retains_for_seven_days`,
  `src/cart.test.ts`), or the run cannot find it.
- **Never** set a test `green` by hand to make `check` pass. If the hook did not record it, the
  test did not pass — say so instead.
- A green recorded at an older revision is reported as **stale evidence**: the code has changed
  since anything proved that requirement. Re-run rather than re-assert.

`kanbanr check FEAT-001` before calling anything done: it reports what the item has not said and
what it cannot yet show. `kanbanr tests` finds evidence that has rotted — a green whose test was
renamed or deleted — and `--write` returns it to `planned`. Mark a check a person performs as
`kind: manual`; it is exempt from that sweep, so use it only when a person really did it.

## Every change points back at the reason for it

`kanbanr trace <G-2 | FEAT-046 | FEAT-046/R-2>` goes **down** — requirements, tests, documents,
decisions — and ends with the gaps, which are the point of it. `kanbanr why <file>:<line>` goes
**up**: the annotation on the line, else the trailer of the commit that wrote it, then requirement
→ goal → charter purpose. A line nobody claimed is reported as exactly that.

- **Annotate the unit that owns the behaviour**, not every line: `// … (FEAT-046 R-2)` on the
  module, function or test. The trailer is the precise link; the annotation is the one that
  survives the refactors that destroy `git blame`.
- **Architecture decisions are documents, not items.** `kanbanr adr new "…" --affects FEAT-046
  --driven-by FEAT-046/R-2 --quality Reliability` scaffolds Context, Decision, Alternatives,
  Consequences and Compliance. Fill them in — `adr list` reports which are still unwritten, and
  doctor warns about a missing Decision or Consequences.
- The links live **on the decision** (`affects`, `driven_by`); the item's view is derived, so it
  can never disagree. `kanbanr adr supersede ADR-0007 --replaces ADR-0003` writes both sides and
  names the items now standing on an overturned decision. Deciding is *work*: it is a task on the
  item that needed the decision, and the ADR is its output.
- A commit may add `Docs: design/mirror.md` and `ADR: ADR-0003`; both are checked against the
  board like any other reference.
- `kanbanr trace <MS-006> --zachman` reports which of the six columns nothing in scope addresses.

## Where the rules live

The board is the system of record for the project's **reasoning**; `CLAUDE.md` is a pointer at it;
this skill is the method; the hooks are enforcement. Each fact lives in exactly one of those — a
fact in two places drifts.

- `kanbanr claude sync` writes a marked block into the project's `CLAUDE.md` with its purpose,
  goals, non-goals and constraints, and regenerates that block whenever the charter changes.
  Everything outside the markers belongs to the author. **Never hand-copy the charter into
  instructions** — reference it.
- `kanbanr hooks install` registers the hooks for **this project** (`<project>/.claude/settings.json`),
  not the machine; `--global` is the opt-in. A checkout with no board carries none of them.
- Prose is surfaced, never enforced. Non-goals appear as context so you do not propose against
  them; nothing blocks on them, because a guard that has to interpret is a guard that misfires.
- Documentation written loose in a code repository is **denied** by a hook, with the
  `kanbanr doc add` equivalent named. What the repository ships (README, CHANGELOG, `docs/`,
  CONTRIBUTING, a doc-site source) is allowed.

## Read what this project already learned, before you start

`kanbanr lessons --for FEAT-001` — run it before starting an item, not after something goes wrong.
The SessionStart hook prints the most-believed ones for the project as a whole.

- **Record one when you learn it**, not at the end:
  `kanbanr lesson add "…" --kind pitfall --from FEAT-043 --evidence "what actually happened"
  --tags mirror`. Without evidence it is an opinion; say what happened.
- Recording the same lesson twice **affirms** it instead of duplicating it — repetition is evidence.
- **Confidence decays.** A lesson nobody reaffirms fades and eventually retires, so the list stays
  short without pruning. `kanbanr lesson affirm L-1 --note "held again on FEAT-050"` when it holds.
- **Say so when one is wrong**: `kanbanr lesson contradict L-1 --note "…"`. Contradiction costs
  more than affirmation gains, and a retired lesson is kept as a record rather than deleted. Being
  wrong later is normal; leaving a stale lesson to mislead the next session is not.
- A defect's `--root-cause` is usually a lesson waiting to be written. Write it there and then.

## Look back at a wave before starting the next one

When a milestone finishes — or every couple of weeks — run `kanbanr retro <MS-00x> --write`. It
reports what the board recorded (scope growth, defects and escapes, cycle time, rework, evidence
at completion, estimate vs actual) and writes it to a document with an empty narrative section.

- **Fill in the narrative from the numbers.** It may explain them; it may not contradict them. The
  two sections stay separate so a reader always knows which part the board vouches for.
- A wave grows for three reasons and the board can only tell them apart if items say so: a defect
  records `--introduced-by`, and work sliced off an existing item records
  `kanbanr split-from FEAT-new FEAT-parent`. Anything else lands in "added without a recorded
  cause", which is the honest bucket, not a spare one — keep it small by recording as you go.
- The Stop hook surfaces a finished wave whose retro is unwritten. Write it, or say in one line
  why it is not worth one.

## What the work actually cost (measurement)

Every status change is appended to the item's history, so flow numbers are derived rather than
claimed. `kanbanr report --since 14d` gives throughput, cycle time (p50/p90), rework
(done → reopened), the defect escape rate and how many requirements are proven by a green test.

When a defect is found, record where it came from:

```
kanbanr defect FEAT-042 --severity high --introduced-by FEAT-031 --found-in production \
  --root-cause "the cutoff was compared as text"
```

Whether it **escaped** is derived, not asked: it escaped if the work that introduced it had
already been called done. Do not claim an escape rate anywhere else — quote the report or say the
board cannot answer yet.

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
  milestones as needed) AND a **definition** (see "Defining work" above). Each FI starts in the
  project's default state.
- **Get agreement before building.** Present the decision brief (`kanbanr review <CODE>`) and wait
  for the user's approval. Moving an item into an active status is refused without it.
- **Before starting** a chunk of work on a feature: make sure it has a **todo-list** for this
  effort with its **items**, and mark the item you're about to do as **InProgress**.
- **Spec-staleness check:** when moving a feature item **from `Deferred` into any active state**
  (anything that is not Completed and not a no-op state), first **review the feature's
  specification for staleness** and update it if it no longer reflects reality — *before*
  proceeding with the work.
- **After finishing** a task: mark its item **Completed**, flip the requirement's test entries to
  `green` once they actually pass, update the **specification** and any **documentation**
  affected, and **move** the feature's status as appropriate. When every task
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
Batch op types: `feature.add`, `feature.edit`, `feature.move` (accepts `unapproved`),
`feature.approve`, `milestone.add`, `todo.add`,
`task.add`, `task.state`, `test.state`, `lesson.add`, `doc.folder`, `doc.write`. Operations apply in order; on failure the
response names the failing operation index. The whole bundle is **one git commit** — pass
`--message "…"` to title it (a default is framed if you omit it). `feature.add` also accepts
`source`, `original` and `issue` (for imports; see above), plus `definition`; `feature.edit`
accepts `source`, `issue`, `defect`, `split_from` and `definition` (which replaces the block wholesale — it is authored
whole, not merged). `kanbanr batch --dry-run` validates a bundle and reports what it would do without
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
kanbanr charter show | set --file charter.yaml     # the project's purpose, goals, non-goals
kanbanr feature define FEAT-001 --template --kind defect   # skeleton for that kind
kanbanr feature define FEAT-001 --file def.yaml | --clear  # write / clear the definition
kanbanr review FEAT-001           # the one-screen decision brief — show this BEFORE building
kanbanr check [FEAT-001]          # what this item has not said and cannot yet show
kanbanr test FEAT-001 R-1 cart::retains green [--rev <sha>]   # normally the capture hook does this
kanbanr report [--since 14d]      # throughput, cycle time, rework, escape rate, coverage
kanbanr tests [--write]           # tracked tests that no longer exist; --write un-proves them
kanbanr retro [MS-006] [--since 14d] [--label x] [--write] | --due
kanbanr claude sync [--show]                    # refresh the generated block in CLAUDE.md
kanbanr hooks install [--global] | status | uninstall   # per project unless --global
kanbanr review --pending                        # every item awaiting agreement, in one pass
kanbanr trace [G-2|FEAT-001|FEAT-001/R-2] [--zachman]   # down the chain, ending in the gaps
kanbanr why src/cart.rs:42                      # up: annotation or trailer -> requirement -> goal
kanbanr adr new "…" [--affects …] [--driven-by FEAT-001/R-2] [--quality Reliability] [--zachman How]
kanbanr adr list [--for FEAT-001] | supersede ADR-0007 --replaces ADR-0003 | history ADR-0007
kanbanr lessons [--for FEAT-001] [--all]        # read BEFORE starting work
kanbanr lesson add "…" --kind pitfall --from FEAT-043 --evidence "…" [--tags a,b] [--goals G-1]
kanbanr lesson affirm L-1 [--note "…"] | contradict L-1 [--note "…"]
kanbanr split-from FEAT-060 FEAT-046   # this item was sliced out of that one
kanbanr start FEAT-001 [--to STATUS] [--no-branch] [--unapproved "<reason>"]
kanbanr commit -m "…" [--ref R-2] [--ref TL-001/T3] [-a]   # trailer filled from the branch
kanbanr finish [FEAT-001]         # gated: tasks complete, requirements proven
kanbanr git install-hooks [--force] | uninstall-hooks | status
kanbanr defect FEAT-002 --introduced-by FEAT-001 --found-in production [--severity …] [--root-cause …]
kanbanr approve FEAT-001          # the user records agreement (do not approve on their behalf)
kanbanr move FEAT-001 Scheduled [--unapproved \"<reason>\"]   # gated; the override is recorded
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
