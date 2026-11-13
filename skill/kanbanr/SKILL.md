---
name: kanbanr
description: >-
  The project management system of record for Claude-driven development. Use it whenever doing
  ANY work on a project that is tracked with kanbanr — and adopt it for the whole project the
  moment the user says "start using kanbanr for this project" (or asks to set up / init kanbanr —
  which runs a plan-mode setup interview first), then keep using it for the rest of the project's
  life with no further instruction. Use to: scope work as feature
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

## Activation: a setup interview in plan mode, then one approved setup

When the user asks to **set up kanbanr**, **init kanbanr**, or **"start using kanbanr for this
project"** (or anything equivalent):

0. **Check the program is installed** (`kanbanr --version`). The skill is instructions; the
   `kanbanr` program does the work. If it is missing, stop and tell the user it installs with
   `curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh`
   (Windows: `irm https://github.com/startr-trade/kanbanr/releases/latest/download/install.ps1 | iex`)
   — nothing below can run without it.
1. **Check whether it's already set up.** Run `kanbanr where --json`. If `source` is `marker` and
   `exists` is true, the project is already tracked: skip the interview, recover the board (see
   "Recover & resume") and carry on. Everything below is for a folder that is not yet tracked.
2. **Enter plan mode before anything writes** (plan mode is for setup and for designing the process, only: never for a routine move or a new item, and **never while a sub-agent is running**, because entering it pauses the agent; if one is running, use a question, or wait until it reports) (the `EnterPlanMode` tool). Setting up is the first
   decision of the project, and the rule for every other decision applies to it too: nothing gets
   built before its reasoning is agreed. Set the user's original request aside until setup is
   done — it is picked up again at step 5.
3. **Run the interview.** Read what you can first (`kanbanr where --json`, `git config user.name`,
   `git config user.email`, `git remote -v`, the README and manifests, any tracker files), then ask
   only what that cannot answer, a few questions at a time with AskUserQuestion. **Every entry in
   `ask_the_user` from `kanbanr where --json` is asked, always**: the board's place, the process,
   its cadence (sprints, releases), whether the project has CI (only with releases on), the
   commit identity (once, when git has none), and the charter (once). Each
   usually has an obvious answer; recommend it as the first option, and let the user give it. Never
   write one of these into the plan as decided. **Ask them in order, each after the answer it
   depends on**: the process question only once the board is chosen (its options are that board's
   saved processes first), and the cadence question only once the process is chosen (it recommends
   the process's own setting). The name and description are not asked: read `suggested_name` and
   `suggested_description` from `kanbanr where --json` and ask only if they are null or unclear.
   Collect three things: (a) board, process and cadence; (b) the identity, once; (c) the charter,
   once.

   **a. Board**
   - *Where the board lives.* It is its own git repo and should sit **outside** the project folder,
     so it never nests inside the project's repo. Options from `kanbanr where --json`:
     `suggested_data_dir` (a new `<repo>.kanbanr` beside the repo; recommended, list it first),
     each of `existing_data_dirs` (share a board with other projects: needed for portfolio views,
     cross-project dependencies and the cross-project Gantt), or a path the user types. Never pick
     silently; if the folder is inside a git repo (`suggested_inside_git_repo`), say so.
   - *Project name and one-line description*: not asked. Use `suggested_name` (the folder's) and
     `suggested_description` (the README's first line, or a manifest's `description`); ask only
     when the description is null or the README is empty.

   **b. Identity, once.** One question for the board and the repository both: name and email,
   defaulted from `git config`, else the board's author. It is asked when git has none here;
   when git already has an identity, use it and do not ask. Every board change is committed as
   this identity, and it is also set for this repository (`git config user.name "<name>"` and
   `git config user.email "<email>"`, run in the project folder; never `--global`). Offer **Yes
   (Recommended)**, or **another name/email**; **No** leaves git alone, and then the initial commit
   passes the identity for that one commit (`git -c user.name="<name>" -c user.email="<email>"
   commit …`) and the plan tells the user to set their own before committing again.

   **c. Charter**: why the project exists, which every item will link to. **One confirmation for
   the whole charter**, not one question per goal or field.
   - Draft it from what the repository already says (the README, manifests, existing docs), and
     show the whole draft, marked **as a draft**, in the plan. Ask one question: confirm, or say
     what to change. Exiting plan mode on the plan is the approval of the draft; nothing is adopted
     before it. The file
     `kanbanr charter set` takes has exactly these fields. A goal's text is `statement`, not
     `outcome`:

     ```yaml
     purpose: One paragraph on why the project exists.
     vision: One line (optional).
     goals:
       - id: G-0                      # optional; blank ids are assigned on save
         statement: The system stays operable and maintainable
         measure: How you would know it happened
     non_goals: [Things the project will deliberately NOT do]
     stakeholders:
       - {name: Solo developer, role: Maintainer, interest: What they need from it}
     constraints: [Rules every piece of work must respect]
     ```
   - Check that each non-goal reads as something the project will **not** do. Users often list
     aims such as "flexible to add languages" under non-goals. Ask before moving one to the goals.
   - Leave anything the repository cannot answer **blank** and ask. Don't invent it. A plausible
     goal nobody agreed to is worse than a missing one, because it looks settled.
   - Suggest a standing `G-0: the system stays operable and maintainable` for maintenance work,
     so chores link to something honest.

   **Process, cadence and the rest**
   - *Saved processes first.* `kanbanr process list --json --data-dir <board>` (with the board
     chosen in **a**; it needs no board otherwise) lists the processes the team saved on the board
     (`"library": "board"`) and the user's own (`"personal"`). In the process question, **those are
     the first options** — board ones, then personal, each with its description and who uses it —
     and the built-in processes come after them. A team's agreed process is usually the answer;
     recommend it when the board has one.
   - *Workflow*: a preset (`kanbanr config workflow --preset list` describes them):
     - **default**: Planned → In Progress → Completed, plus Deferred and Ongoing.
     - **scheduled**: Planned → Scheduled → Completed.
     - **togaf**: architecture phases, where the definition grows phase by phase.
     - **pdca**: Plan → Do → Check → Act.
     - **design-control**: modelled on ISO 9001 §8.3, with sign-offs.
     - **scrum**: Backlog → Ready → In Progress → Review → Testing → Done → Released, worked in
       sprints. Ready is the Definition of Ready, and Done is the Definition of Done.
     - **agile**: Plan → Design → Develop → Test → Review → Released.

     Or **"our own process"**: design it with the user now — see "Designing a process with the
     user" — and the plan saves it and applies it. An organisation's existing process file is
     loaded the same way: check it with `kanbanr process check`, then save and apply it. The phased
     presets ask for part of the definition at each stage, not all of it up front. For "our own
     process", which stage holds each role (design, build, integrate, release) is part of the
     design.
   - *Cadence*, asked for **every** process, as one question with two parts: "Sprints? Releases?"
     Recommend the chosen process's own setting: on for scrum and agile, off for the rest, which
     most projects need. Ask nothing more about it: no sprint length, start or capacity, no release
     rhythm or version. The defaults apply (14-day sprints, releases on demand; a sprint can be
     edited until it starts). **Setup creates no sprint and no release**: the first is made when the
     user asks for it (see "Starting the first sprint or release"). When releases are on, ask once
     whether the project has CI (Yes or No, default No).

     After the answer, run `kanbanr git model --sprints on|off --releases on|off` with it. It needs
     no board. Show its output verbatim in the follow-up and in the plan: where items branch from
     and merge into, what kanbanr merges, and that pushing is always the user's. With sprints or
     releases on, setup turns the git model on: the preview says `automation: on`, and kanbanr
     merges as it shows. Say so in the plan. Turning it off is the user's choice
     (`kanbanr config cadence --git-model off`).
   - *Definitions*: every item carries the Zachman six-dimension definition, EARS requirements
     and tests. That is the method, not an option. Say so, so the user is not surprised by it.
     Say also, without asking, that design before build and the docs fold-in are on by default
     (`kanbanr config method --off` turns them off later).
   - *Git hooks* in the code repo (`kanbanr git install-hooks`): every commit names its board
     item, and commits on the default branch are refused. Offer them **defaulting to yes**.
   - *No git repository yet?* If the project folder isn't one, say so and plan `git init` plus
     one initial commit (the `.kanbanr` marker, `CLAUDE.md`, a `.gitignore`) **before** the git
     hooks. An item branch needs a commit to branch from, and `kanbanr start` refuses until one
     exists. The first commit lands on the default branch and says `[no-ref] initial commit` in its
     message. The `.gitignore` lists `.claude/settings.json` and `.claude/settings.local.json`.
     `kanbanr hooks install` generates the settings file on each machine with that machine's
     absolute paths, so it's never committed.
   - *Optional*: a git remote to back the board up to, the GitHub issue mirror (see below; a
     public repo needs an explicit OK), and importing an existing tracker (see "Importing existing
     task trackers": list what you found and ask which to bring in).
4. **Write the setup plan and exit plan mode.** The plan states every answer and the exact
   commands, in order, so approving it approves precisely what will run. Under **Process** it
   lists the stage that holds each role, as `kanbanr process show <name>` prints them; under
   **Branching** it quotes the `kanbanr git model` output from the cadence question; and it says
   the method's gates are on (`kanbanr config method --off` turns them off later):

   ```bash
   kanbanr init <name> --data-dir <board> --author "<name>" --email <email> --description "<…>"
   kanbanr process save <name> --description "<…>" [--personal] --from-file <scratch>/process.yaml   # a process designed now
   kanbanr config workflow --preset <name>         # a saved, built-in or just-saved process; omit for default
   kanbanr charter set --file <scratch>/charter.yaml
   kanbanr hooks status                            # init registered them; install if it did not
   kanbanr claude sync                             # CLAUDE.md block pointing at the charter
   printf '.claude/settings.json\n.claude/settings.local.json\n' >> .gitignore
   git config user.name "<name>" && git config user.email "<email>"   # if git had none and the user said Yes
   git init && git add .kanbanr CLAUDE.md .gitignore && git commit -m "[no-ref] initial commit"
                                                   # with -c user.name=… -c user.email=… if they said No
                                                   # these two only if the folder was not a git repo
   kanbanr git install-hooks                       # if chosen
   kanbanr config cadence --sprints on|off --releases on|off   # every process: the answer given
   kanbanr config cadence --has-ci on|off          # only when releases are on: the CI answer
   kanbanr config workflow --write-agreement       # the working agreement, generated from the gates
   kanbanr remote add <name> <url>                 # if chosen
   kanbanr mirror enable --repo <owner/repo>       # if chosen
   # import: one `kanbanr batch` of the chosen tracker items, if chosen
   kanbanr doc add setup/<YYYY-MM-DD>-setup.md --file <scratch>/setup-plan.md
   kanbanr doctor && kanbanr board
   ```

   **Exiting plan mode is the approval.** If the user sends changes back instead, revise the plan
   and exit again. Do not start setting up on a partial yes.
5. **Run the approved setup, all of it, before anything else.** Run the commands in the plan's
   order and stop at the first failure — one command per call, or joined with `&&`; never as a
   script that carries on past a failed step: report it, fix it, and rerun from there, rather than
   leaving a half-configured project. Relay what `init` printed. If the Claude Code sandbox blocks
   writes to the board folder, tell the user to allow it (e.g. `sandbox.filesystem.allowWrite`).
   Save the approved plan as the board doc named in it, so the reasons for the setup outlive the
   session. Finish by checking `kanbanr hooks status`, `kanbanr git model`, `kanbanr git status`
   and `kanbanr git doctor` (these two if the hooks were chosen) and `kanbanr doctor`, then show
   the board. `kanbanr git model` must say `automation: on` when sprints or releases are on; if it
   does not, that is a setup fault to fix now. On a new board, doctor's only expected
   warnings are the goals that no item links to yet. Anything else is a setup fault to fix now.
   Doctor warns only about what an item's current stage asks; what later stages will ask appears
   after the warnings as counts ("Later stages will ask"), which are not warnings and need no
   clearing. `kanbanr check` lists them.
   Tell the user how to watch the board: `kanbanr serve` starts the monitor, which is built into
   the binary (it needs no `--ui-dir`, and never another checkout's build), and `kanbanr open`
   opens it. **Only then** go back to what the user originally asked for.
6. From that point on, treat kanbanr as the **system of record for the entire project**, with no
   further prompting.

The charter is read back at the start of every session with `kanbanr charter show`, alongside
`kanbanr board`. If a `.kanbanr` marker (or `$KANBANR_PROJECT`) is already present, the project is
already tracked, so behave as if activated. Every change is committed to the data repo **authored
by the configured identity** (`kanbanr identity`), and sharing happens through git remotes.

## Starting the first sprint or release

Setup makes neither, so the first one is made when the user asks for it, with the answers then
known:

- **A sprint.** When the user first asks to start or plan a sprint, ask its goal, its first day
  (default next Monday) and its capacity (points under Scrum), then `kanbanr sprint add`. If a
  planned sprint with no items already exists, offer `kanbanr sprint edit <SP>` or reuse it. Never
  add a second sprint to change a date. A mistaken empty sprint is `kanbanr sprint remove <SP>`.
- **A release.** When the user first plans an item into a release and none is suitable, ask the
  version and the target date, then `kanbanr release add`. If an empty planned release exists,
  reuse it; a mistaken empty one is `kanbanr release remove <V>`.
- A started sprint is a record and is not edited. A sprint or release that holds items, a stage,
  a branch or a patch is not removed.

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
3. the **six dimensions** — what / how / where / when / who / why, one line each
   (Zachman's interrogatives, used as a completeness check; its perspective rows are not
   modelled, so nothing here asks you to think in the framework);
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

## Agreement before work: decided on the pages

**Scope → define → the user decides on the Review page → only then write code.** Every verdict is
the user's and is taken on the monitor: approve, ratify, sign off, withdraw, accept or reject a
decision (Review page), promote or cut a release and confirm each stage after the tag (Releases
page). **You never record a verdict, and never ask for one in a plan or a question.** kanbanr's
guard refuses the commands if you try. Your part is one message saying what is waiting and where:
`kanbanr open --check` gives the addresses. If no monitor accepts writes, offer to start one with
`kanbanr review --ui` (run it in the background) or tell the user the command to run.

**Prepare the record before you say a decision waits** (FTR-034). When you have its evidence (the
CI run's link and result, the commit, the doc that holds the minutes, what the person said), put
it on the record with `kanbanr prepare signoff|promotion|gate|ratify …`, naming every source you
read with `--source` (`LABEL=URL`, a link, `doc:<path>`, `item:<CODE>`, `commit:<sha>`, or a bare
label such as "what the person said"). The page then shows it filled in beside the decision's one
button; the person confirms it, edits a field first, or rejects it, and the verdict is theirs.
Preparing is not deciding: nothing a gate reads changes until they click, and there is no confirm
command. **Never type into the monitor, and never clear a check by editing the text it flags.**

### The workflow decides what you ask — never a preset name

Do not reason from "this is TOGAF" or "this is Scrum". Ask the board what the next stage needs:

```bash
kanbanr check <CODE> --json   # rev, and for each next stage: status, purpose, gaps, warnings, signoffs_needed
```

Turn each failing check into one action — the same for every workflow, so an edited preset or an
organisation's own process file changes what you ask with nothing changed here:

| Check | What you do |
|---|---|
| `definition`, `statement`, `goals`, `goals_known`, `zachman` (its named columns), `requirements`, `ears`, `quality`, `tests_named`, `small`, `spec_sections` (its named sections; `Design` is the build-ready design) | **draft it into the definition**; ask only what you cannot work out from the charter and the code |
| `approved` | the user's approval **on the Review page**: say it is waiting, with the link |
| `bypass` | a ratification **on the Review page**: say it is waiting |
| `signoff` (a name in `signoffs_needed`, e.g. `design-review`, `docs-folded`) | a sign-off **on the Review page**: say it is waiting |
| `estimated`, `in_sprint`, `in_release` | a **question** with the choices the board offers (points or days; the active sprint; planned releases) |
| `tests_green` | **never a question** — evidence decides; report what is not green yet, and make it green |
| `docs_updated` | **never a question** — fold the design into each doc in `affects_docs`, as built, and draft that fold-in into the definition, saying which docs change; if the item names none, ask whether it really changes no architecture doc |
| `diagrams_valid` | **never a question** — fix each Mermaid diagram the lint names, in the spec or a doc in `affects_docs`, then run `kanbanr doctor` to confirm none is left |
| `docs_cover` | **never a question** — fix the coverage: add a link to the item in each view doc the rule names (`[zachman] FEAT-012 is not referenced from architecture/zachman.md`), then move |

Warnings go into the plan with a choice: fix it, or go ahead with the warning on record. When more
than one next stage is possible, ask which. Head the plan with the stage's own `purpose`.

**A kind can have its own path.** An item of a kind with a path (`kanbanr process show` lists them
under "Lighter paths"; the built-in processes give `chore` one) skips the stages the path leaves
out. Do not walk a chore through stages its path omits, and do not pad answers to satisfy a stage it
never meets: `kanbanr check` lists only the next stage on its path. Its first move asks the path's
own conditions too (a statement, goals and requirements), so draft those once. Make one with
`kanbanr feature add --kind chore`. The approval and the sign-offs the stage it enters asks for
stay the user's, as ever.

**Stages have roles** (`kanbanr process checks` lists them): design, build, integrate, release.
Whatever a process calls its stages, the design is due before the build role's stage, and code is
written there, on the item's branch. Say which stage holds which role when you plan work. What
each role asks of you is in "The method by stage role".

**The purpose is a question too.** A stage's `purpose` can name something no check measures — "the
quality it must reach", "when it happens". If the definition does not yet address it, ask (with
options, e.g. a latency target, or "no target"), rather than noting its absence afterwards.

### Diagrams

A design usually carries Mermaid diagrams, and a broken one draws as an error the reviewer finds at
approval time. kanbanr lints every `mermaid` block when you write a spec or a doc and prints each
problem as `diagram: <file>, diagram #n, line N: … (rule)` on stderr; fix it before you go on, not
after. Write them so they draw:
- **Quote flowchart labels:** `A["f(x) {y}"]`, never `A[f(x)]`. Parentheses, braces, brackets,
  `|` and quotes inside an unquoted label break it, and GitHub's Mermaid also rejects a `;` there.
- **Keep sequence messages to plain words:** no `;` (it ends the statement and cuts the message in
  half), no braces, no quotes, no JSON. Describe the payload in words.
- **Never use `end` as an id** — not as a node, a subgraph or a participant. Call it `done` or
  `finish`.
- Close every `subgraph`, `alt`, `loop`, `opt` and `par` with `end`.
- **Run `kanbanr doctor` after writing diagrams**: it lists every diagram problem on the board, with
  file, diagram number, line and rule. The Review page parses each diagram with Pele before it
  offers Approve, so a broken one blocks the approval you are waiting for.

A project can refuse broken diagrams outright (`kanbanr config diagrams --strict on`) or require
`diagrams_valid` on a gate. Both are the user's choice; never switch them yourself.

### A new item: scope it in the conversation

When the user asks for work, do not enter plan mode. Read the charter and the
code; ask only what you cannot find, a few questions at a time (AskUserQuestion, with your
recommendation first). The brief is the definition: statement, goals, the six dimensions the stage asks
for, EARS requirements with named tests, and the stage it starts in — as far as the target stage's
gate requires, and no further unless the user asks for the whole definition up front.

**Ask the user's choices; don't make them.** Anything with a consequence the user would recognise —
a business rule (how two baskets merge), a retention or expiry, a limit, who may see what, what is
in or out of scope — is an AskUserQuestion *before* you write the definition, with your recommendation
first.
Choices you still had to make go in the brief under **Choices I made**, so the approval is
informed. Never leave them as "for you to decide" after the approval is given.

When one request spans several stages ("take it to business architecture, then design it"), say at
the top of the brief which stages it covers.

Then write the definition to the board (`kanbanr feature define`), show its brief in a few lines,
and say it is waiting for their approval on the Review page, with the link. Choices you had to
make go in the brief under **Choices I made**, so the approval is informed. Moving an item on is
the same: grow the definition, then say what the next decision is and where it waits.

Moving an item on to a stage that asks for more is the same loop: plan the growth, the user
accepts, record the next decision the stage asks for. Adding to an approved definition keeps the
approval; what you add is shown and reviewed at the next decision the stage asks for (usually
design-review). Changing or removing approved text lapses it, and the item returns to the Review
page.

### Waiting decisions: on the Review and Releases pages

When items are waiting (`kanbanr review --pending --json`, `kanbanr adr list --json` for
`proposed` decisions, and the Releases page for a release ready to move), say so in one message:
how many, which, and the page's address. Never list them as questions. If the user asks you to
explain one, explain it, then send them back to the page. If they want a change, make it; the item
returns to the queue as its approval or sign-off requires. A verdict records where it was taken (monitor,
conversation or terminal), and one that repeats the latest on unchanged text records nothing: if a
command answers "already …; nothing recorded", tell the user who decided first and stop.
**If a verdict was ever recorded that
the user did not give, say so at once**, and ask them to withdraw it on the item's page.

The queue offers a sign-off only for the stage the item reaches **next**, and only once that
stage's other requirements are met. A sign-off for a later stage, or for work not done yet, is not
asked for, so don't ask for it either.

**A decision you draft is announced at once:** fill its sections, show it in a few lines, and say
it waits on the Review page.

### Designing a process with the user

Plan mode is for this, and for setup, only: never for a routine move or a new item, and **never
while a sub-agent is running**, because entering it pauses the agent. If one is running, use a
question, or wait until it reports.

When the user wants a process of their own — at setup, or later ("let's change our process", "we
need a design review before build") — design it **in plan mode**, from the board's vocabulary, so
what is agreed is exactly what will be enforced:

1. **Start from what exists.** `kanbanr process list --json`: a saved process may already fit;
   offer saved ones before built-in ones. If
   not, ask how work moves today, in their words, and start from the nearest built-in process
   (`kanbanr process show <name>`), so the user corrects a draft instead of facing a blank page.
2. **Name the stages with them**: one AskUserQuestion proposing the stage list, with your
   recommendation first and "Other" for their own.
3. **One question per stage: "What must be true before work enters *Stage*?"** Build the options
   from `kanbanr process checks --json`, each phrased in the user's terms (`tests_named` → "every
   requirement names the test that proves it"); multi-select. Recommend the checks the stage's
   purpose needs and no more: a stage asks for what it needs, and the definition grows stage by
   stage.
4. **What the board can't see becomes a named sign-off.** "The architecture board has reviewed
   it" → `signoffs: [architecture-review]`. Say it back plainly: a person records it, never you.
5. **The process-wide choices, together in one AskUserQuestion batch:** block or only warn at each
   gate; which stage holds each role (design, build, integrate and release; build is where the
   code branch is made, `roles: [build]`, usually the first building stage); which stage ends the
   work; which moves back for rework are allowed.
6. **Name it, and ask where it is kept:** on the board (the team gets it through the board's
   remote; recommended when the board has one) or personal (`~/.kanbanr/processes`, theirs on any
   board).
7. **Draft, check, show — before the plan.** Plan mode allows no file writes, so check the draft
   from stdin: `kanbanr process check - <<'YAML'` … `YAML`. Fix every problem it reports, and
   check again. Then put in the plan, **verbatim, the working agreement and the Mermaid diagram it
   printed** (what each stage asks, in words), not your own table: the plan must show what will be
   enforced, as the board will enforce it. Add a one-line description of what the process is for.
   The plan's commands, after acceptance, write the draft to a file and save it:
   `kanbanr process save <name> --description "<one line>" [--personal] --from-file <file>`, then
   `kanbanr config workflow --preset <name>`. **Accepting the plan accepts the process.**

**Changing a saved process** is the same loop, starting from `kanbanr process show <name>`. Saving
it gives a new version; nothing else changes until each project takes it. `process save` then
lists the projects on the board still on an earlier version. Ask, **for every project it lists** — `used_by` in `kanbanr process list --json`, the project you
are in **and every other one** — **one question each**: show `kanbanr process diff --project <id>`
in a few lines and ask Update / Not now. Update → `kanbanr process update --project <id>`.

Whether another project takes the new version is the user's choice, not yours: kanbanr has no
project owners, so never skip a project because it "belongs to someone else", and never leave the
update as a command for the user to run. If they say another person decides for that project,
"Not now" records nothing and the notice stays for that person. If an update is refused because a
stage it removes still holds items, say which, and offer to move them or rename the status first.

**When a session opens with a process notice** ("v3 is saved now"), mention it once and ask
Update / Not now. Never run `kanbanr process update` unasked, and never edit a project's gates to
match a process by hand.

### Moving work

The user moves work by asking: "park FEAT-074", "put FEAT-150 back to Planned". Run
`kanbanr move <CODE> <status>`. If a gate refuses, say what the stage still needs — it is the same
table — and offer to work through it. `--override "<reason>"` is for genuine emergencies, with the
user's reason, and the item then needs ratifying. A write that prints `coverage:` lines has left a
view behind: add the links each one names before you move the item on.

### The rules underneath

- A verdict records **who** gave it, defaulting to the board's commit identity. One with no named
  approver is refused, so never invent `--by`: if it fails, the board has no identity and the user
  sets one with `kanbanr identity`.
- kanbanr enforces the gates: a move into a stage whose gate needs `approved` is refused unless the
  definition's approval is current. Change or remove approved text and it **lapses**; added text is
  reviewed at the next decision — scope cannot drift silently past a yes.
- **Subagents inherit, never invent.** A coordinator passes the approved definition to each
  subagent as its brief. A subagent that finds work outside it returns a **proposed change to the
  definition**, not merged code.
- A project with no charter isn't refused: a move its gate would stop goes through with a warning
  naming what was missing and that `kanbanr charter set` turns the gates on. Tell the user what it
  says. Items created before the charter was adopted stay exempt, so adopting the method never
  breaks an existing board.

## The method by stage role

kanbanr carries one method for every process: a build-ready design before code, architecture docs
kept as built, live links, a git model that follows the cadence, and releases that move through
stages. It is stated here once, by the **role** a stage holds, so it holds the same under the
default kanban, Scrum, TOGAF, PDCA or a process of the user's own.

**Roles first.** `kanbanr process show <name>` prints the working agreement with each stage's role,
and `kanbanr process checks --json` lists the roles and the checks a gate can ask for. Find the
status that holds each of design, build, integrate and release, and reason from that, never from a
preset's name. `kanbanr config method` says which parts of the method this project has on
(`design`, `design-review`, `docs`, `docs-folded`, `links`). A part that is off is not asked for by
the gates, but it is still good practice: keep doing it, and never turn a part off or on yourself.

### At the design role

The item's spec gains its build-ready `## Design` (the next subsection). Also:
- **Affects docs**: the Design ends with links to the board docs the change touches, and the same
  paths go into the definition's `affects_docs`, which the integrate role checks. Under the same
  heading, write each repository doc the change touches (guides, reference pages, `SECURITY.md`,
  `CHANGELOG.md`) as a path in backticks, one per bullet, and add a task for each document to
  build, naming its path. `docs-folded` is offered to the user only once those are built.
- **Every reference is a live link**: items, requirements, milestones, goals, ADRs and docs, in the
  forms under "Every change points back at the reason for it". `kanbanr doctor` reports a link that
  points at nothing.
- **Decisions are drafted as proposed ADRs** (`kanbanr adr new "…" --affects <CODE>`) and asked
  about at once: Accept, Reject or Later.
- **An undecided question goes to the user** before the design is signed off, never to a subagent.
  A design subagent works under a design scope, in its own worktree: it reads, and writes nothing
  (see "Sub-agents: what each may do").

- **Ask whether the product has a user interface** the first time the technology or stack is chosen
  (for TOGAF, in the technology phase; elsewhere, when the first item is designed). Ask it as a
  question with the options a screen a person uses, a service, library or command-line tool only,
  and not decided yet; recommend nothing the user has not said. Record the answer in the design. When
  there are screens, ask for them as items of their own; when there are none, write "no user
  interface" in the design so that is a recorded decision. Write the first item as what a person
  does ("a shopper opens their basket and finds it kept"), not as a capability.

Leaving the design role, the build stage's gate asks for `spec_sections: [Design]` and the
`design-review` sign-off. Say **design-review** is waiting on the Review page once the section exists; the user gives
it, never you.

**A design-review covers the whole spec as it was when given.** Any edit to the spec afterwards (a
correction, a note, a link) brings the sign-off back to the Review page, with only the change shown.
Make spec edits after design-review together, not one by one. Then say the sign-off is waiting again
on the Review page; don't ask the user to carry on in a question. A queue entry with
`"approval": "spec_changed"` carries `spec_rev` and the diff; pass `--spec-rev <spec_rev>` with the
sign-off so it is refused if the spec moved on after the user was shown it.

### The design stage: the spec carries the build-ready design

When an item reaches a stage whose purpose is *how and where* (System Design, Design, or the
equivalent in the project's process), its **spec** gains a `## Design` section written at
implementation level. A reviewer or another session should be able to build from it without
guessing:
- components, modules and files;
- public interfaces with signatures, and what each must do;
- the data model and on-disk or wire formats;
- algorithms and edge cases, and control flow (a diagram where it helps);
- concurrency, transactions and failure handling;
- configuration and its defaults (with how each default was chosen);
- dependencies;
- security and privacy;
- performance budgets that name their benchmark;
- a test plan mapping each requirement to its tests;
- compatibility and migration;
- links to ADRs, and open questions stated as questions;
- **Affects docs**: links to the board docs this change touches. Mirror the same paths into the
  definition's `affects_docs`, which a `docs_updated` gate checks. Include the project's view
  docs — the Zachman matrix, the component catalog, the ADR index (`kanbanr config coverage`
  names them) — whenever the change adds an item, a component or a decision they list.

The definition's `how` and `where` lines are one-line summaries of this section, never a
substitute for it. A section that does not apply says "n/a" and why. A spec that stays
non-technical at this stage leaves implementation decisions unreviewed, which is the failure the
gate exists to prevent.

**Write it for hand-off.** The design is the point where a capable model does the thinking, so
that cheaper models can do the building. End the section with an **implementation task list** a
lower-cost subagent can carry out one task at a time, without making a design decision. Each task
names:
- the files it creates or changes;
- the exact signatures;
- the behaviour, including errors;
- the tests that prove it, by name, and the requirement each serves;
- the tasks it depends on.

Leave nothing open inside a task. A question still undecided goes to the user before the design is
signed off, never to the subagent. Mirror the tasks into the item's todo-list, so each one can be
handed out and tracked.

A project makes the section a condition of moving on with
`spec_sections: [Design]` in the build stage's `requires`. If it also wants a person to review the
design before build, it adds `signoffs: [design-review]` to the same gate.

### At the build role

- `kanbanr start <CODE>` makes the item branch. With the git model on, it branches from the line of
  integration `kanbanr git model` names (main, `sprint/<SP>` or the integration branch), not always
  from main; a fix to a release under test branches from `release/<V>`.
- Commit with `kanbanr commit`: it fills in `Refs:` from the branch.
- Subagents take one task at a time from the item's todo-list, with the approved definition and
  that task as their brief, under a build scope issued for the item (next section).
- Never commit on main or on a line of integration (the hooks refuse), and never push.

### Sub-agents: what each may do

When you hand work to a sub-agent, its limits are not left to its judgement or to a one-off line
in the brief. Each role has fixed boundaries, kanbanr enforces them,
and the brief states them **verbatim**:

| | A **build** agent (one item, in its own worktree) | A **design** agent (in its own worktree) |
|---|---|---|
| Read the board and the code | everything | everything |
| Its item's record | `kanbanr task state <CODE> …` as each task starts and lands; `kanbanr test <CODE> R-n "<name>" green --rev <sha>` for a test it actually ran green | nothing |
| Everything else on the board | refused: moves, approvals, sign-offs, definitions, specs, docs, other items, config; a `batch` only of these ops | refused |
| Code | commits only on its item's branch, each naming the item | no edits, no commits |
| Remote | never: no `git push`, no `kanbanr sync` | never |
| Tests | on a temporary board: a folder under the system temporary directory that holds no scope, named by a literal path or `$TMPDIR/…`; never a real board | — |

To walk a process through, an agent runs `kanbanr --data-dir $TMPDIR/walk init …` and then the same `--data-dir` (or `KANBANR_DATA_DIR`) on each command.

**Issue a scope before each sub-agent.** One command records what the agent may do on the board
and prints its brief:

    kanbanr scope issue <design|build> <CODE>      # --for 8h by default, 72h at most; --note "T1–T4"

Then:
- start the agent with `isolation: worktree`, design agents included: a scope binds to the agent's
  own worktree, never to your checkout;
- with `--worktree <path>` the scope is bound at once and the brief has no claim step; without it
  the agent's first command is the claim;
- paste the printed brief **verbatim** at the top of the agent's brief. Its first line has the
  agent claim the scope (`kanbanr scope claim SC-…`); from then on kanbanr refuses, and names, every
  board write, commit and push that agent makes from that worktree that the scope does not allow;
- **stay in the project folder.** A scope binds the agent it was issued to, never you, but your own
  shell is kept out of an agent's worktree: never `cd` into an agent's worktree. Look inside with
  `git -C <path>`, `make -C <path>`, `npm --prefix <path>`, `cargo --manifest-path <path>/Cargo.toml`
  or a subshell `(cd <path> && …)`. If your shell is already there, `cd` back to the project folder;
  the Bash guard says so too;
- add the item, its todo-list and the tasks the agent takes, and nothing that contradicts the
  table;
- when the agent's work has been reviewed, revoke the scope: `kanbanr scope revoke SC-… --reason
  done`. `kanbanr scope list` shows what is in force; `kanbanr scope renew SC-… --for 4h` extends
  one. Revoking a scope stops the agent on its next tool call and never restricts you;
- never issue scopes from inside an agent: an agent's worktree refuses it;
- put your own worktree, for a merge or a finished item, under `.claude/worktrees/` too, with a
  name that does not start `agent-`: only `agent-*` worktrees are refused without a scope.

A build scope needs its item at the build stage and kanbanr's hooks installed
(`kanbanr git install-hooks`, which adds the pre-push hook, and `kanbanr hooks install`, whose
Claude Code hooks tell an agent from you). It finds or makes the item branch;
the claim switches the agent's worktree to it.

As the coordinator you:
- put the role's table in every brief, word for word, with the item, its branch and its todo-list
  — the printed brief is that table;
- have build agents record their own progress, so the board never lags the code; and when you look
  at an agent's branch, bring its item's tasks up to date at once, never waiting for its report.
  `kanbanr scope drift` (and the Stop hook) names an agent branch ahead of its item's tasks;
- take back every question and every piece of work outside the brief: the agent reports it, you
  decide or ask the user, it never decides a design question itself. A refusal an agent reports is
  the same: decide it or ask the user, never widen the scope to get past it;
- **report changes, not events.** Reply to a status line, monitor event or agent message only when
  it changes what the user knows: a task landed, a failure, a stall, a decision waiting, the work
  done. Gather what arrives together into one message. Send nothing for a report that repeats one
  already given. Never write 'already reported'.
- **put rework on the board before it starts.** Add each piece as a task on the item's todo-list
  first, so the item never reads as finished while someone works on it. A build scope is refused
  for an item with no open task.
- give no agent a verdict to record, a stage to move or anything to push; those stay yours and the
  user's. When a scoped agent finishes an item's last task, the item stays where it is: you move it.

A sub-agent is not a way round the method: it works inside the same gates, on the same branch rules,
and what it records is checked like anything else. Every write and commit it makes is recorded as
the user, by the agent for its item (`Agent-Scope:` on the commit).

### At the integrate role

Before the move:
- fold the Design into each doc in `affects_docs`, describing it as built (`docs_updated`);
- build every document the item names: the board docs, the repository docs its Design lists under
  `Affects docs` and each task that writes docs. Until then the Review page does not offer
  **docs-folded** and `kanbanr check` names what is unbuilt;
- say **docs-folded** is waiting for the user's sign-off on the Review page, once it is offered.
  The Review page may list **texts to check**: docs and items that name a flag, route, setting,
  stage, status or file the item changed and have not changed since (`kanbanr drift` lists them for
  the board; a merge prints them on stderr). A flag says what to read, never that the text is wrong.
  For each, the user says it is still right, or asks you to file a correction. Fix a filed
  correction as any defect, and never confirm a text on the user's behalf.

With the git model on, entering the integrate role (`kanbanr move <CODE> <status>`) merges the item
branch into its line, `--no-ff`, with `Refs: kanbanr:<CODE>`. If the merge would conflict, the move
is refused and the item stays where it was: merge the line *into* the item branch, resolve, commit,
and move again. Never rebase. `kanbanr git doctor` names any disagreement between the board and the
branches; fix it before going on.

### At the release role

- A release moves through its stages with `kanbanr release promote <V> [stage]`. By default the
  first stage makes `release/<V>` and the last, production, is the cut: kanbanr brings main up to
  the release (a fast-forward, refused with nothing changed if main has commits the release lacks)
  and tags the version.
- Stages can follow the tag (`tag_passed`): the stage holding `tag` is the cut and ships nothing;
  each later stage is the user's confirmation (`kanbanr release promote <V> --note …`, or the
  Releases page), and the last one ships. **Report where the release is and what the tag's CI
  result is; never confirm a stage, start a patch from a failed tag, or record the result for the
  user.** Those are their verdicts.
- Turning releases on (`kanbanr config cadence --releases on`) with no stages named gives
  `candidate`, `main` and `tag` with their gates; `--has-ci on` makes `tag` also require a passed
  `ci` result. A board that already had releases on keeps `candidate, test, staging, production`,
  where `test` and `staging` do nothing: configuring a stage that has no condition and no action
  prints a warning and `kanbanr release stages` marks it `does nothing`. Tell the user; don't
  remove the stage for them. `kanbanr config cadence --release-stages candidate,main,tag` moves an old board to
  the three.
- Stages can act (`kanbanr release stages`, `kanbanr config release-stage`): each may require
  `items_finished`, `prepared`, `main_at_release`, `pushed`, `items_merged` (every planned item at the integrate
  stage or later), or name earlier stages in `--after` (the release must have been promoted into each), and do `branch`, `prepare`,
  `move_main`, `tag` on entry. A refusal lists every missing condition with its fix; nothing changed.
- **A release prepares itself.** The stage with `prepare` commits the version files, the dated
  changelog heading and a notes draft onto `release/<V>` (`kanbanr config release-prepare`). Never
  make a feature item to prepare a release.
- **The notes are read and edited from the release.** They live on `release/<V>`, not in the
  checkout: `kanbanr release notes <V>` shows them and where they are; `--file F` (or `-`) or
  `--edit` commits new text there, nothing checked out, until the cut. When the user asks where
  the notes are or wants them changed, use this — never a fix item, never a checkout of
  `release/<V>`. The Releases page shows them too, with Edit on a write-enabled monitor.
- **A merge sends a promoted release back.** When a move merges an item into a release that is past
  candidate, kanbanr sends the release back to candidate and says so. Tell the person, in one
  message, what the Releases page now offers: update the notes, run the gate again, promote to
  main, push and wait for CI, then cut. Confirm no stage yourself. A release that is tagged and not
  shipped refuses the merge and needs a patch release.
- **The date follows the release.** Entering the stage that moves main dates the notes and changelog
  today. If main moved earlier, the cut's plan warns: run `kanbanr release redate <V>` before the
  cut, and say to push main again if it says so.
- Plan items into a release before promoting it: a release with nothing planned is refused. So a
  release starts with `release add`, then `release plan <V> <CODE>…`, then promoting it to its
  first stage (candidate), which makes `release/<V>`; `release add` has no `--branch`. When a
  sprint is running, plan the item into it too (`kanbanr sprint plan <SP> <CODE>`) unless the person
  said otherwise; `release plan` names the ones it missed. A promotion held for a merged item names
  the sign-off it lacks: send the person to the Review page; never record it.
- Check with `kanbanr release promote <V> --plan` and tell the user what it still needs; the
  promotion itself, the cut included, is theirs, on the Releases page. **The user pushes main and
  the tag**, by hand; a cut that requires
  `pushed` waits until main is pushed (and CI has run on it).
- **A CI result is the user's to confirm.** A stage that requires `gate_passed` needs a recorded
  passed result for the release tip. kanbanr never asks a server. You may read the CI's result
  with your own tools; then `kanbanr prepare gate <gate> --passed|--failed --commit <sha> --url
  <the run>` puts it beside **Record result** on the Releases page, and the person confirms it
  there with one click (or runs `kanbanr gate record` in their own terminal). When it is missing,
  say what the plan names and where it waits. **Never run `kanbanr gate record` yourself**, and
  never prepare a verdict you did not see.
- A patch is `kanbanr release add <V> --patch-of <P>`: `release/<V>` is made from `<P>`'s tag. A fix
  integrated into a release branch reaches the line through `kanbanr git forward <V>`, which the
  move says is needed; it is never done for you.
- In the sprints-only model, `kanbanr sprint close <SP>` is the cut: main fast-forwards to
  `sprint/<SP>`, tagged with the sprint's code.
- **Designing CI or a release workflow.** When the user asks for a pipeline, design it as
  `docs/src/guides/delivery-pipeline.md` says, and as an item with its own definition: start with
  one job on one platform, tests first; ask the user which checks their plan and settings allow
  (you cannot see their account) and leave out any they cannot run; make the release description the
  notes kanbanr drafted (`kanbanr release notes <V>`), never a list generated from commits; add a
  manual trigger that runs everything up to publishing and publishes nothing, and ask the user to
  run it before the cut; run each check once per commit; and test the built product the way a
  person will use it. Name no remote server or CI product that the project has not already chosen.

### Carry-over

`kanbanr sprint close <SP> --carry-to <next SP>` carries the unfinished items and keeps their
branches. kanbanr merges the next sprint's branch into each carried item's branch (a catch-up, never
a rebase); a conflict is reported for that item and its branch is left as it was, for you to resolve
on the item branch.

### Who merges, who pushes

`kanbanr git model --sprints on|off --releases on|off` shows the model for a cadence, and
`kanbanr git model` the one this project follows:

| Cadence | Items branch from and merge into | Production moves at | kanbanr merges (with the git model on) |
|---|---|---|---|
| neither (trunk) | main | the integrate merge | each item branch into main |
| sprints only | `sprint/<SP>` | the sprint close | item branches into `sprint/<SP>`; `sprint/<SP>` into main at the close; the sprint into carried items |
| releases only | `integration` | the release stage with `move_main` (by default the last) | item branches into `integration`; `release/<V>` into main at its `move_main` stage; tagged at the cut, after main is pushed if the stage requires `pushed` |
| sprints and releases | `sprint/<SP>` | the release stage with `move_main` (by default the last) | item branches into `sprint/<SP>`; `release/<V>` into main at its `move_main` stage; tagged at the cut, after main is pushed if the stage requires `pushed`; the sprint into carried items |

kanbanr commits only its release-preparation commit on `release/<V>`.

With the git model off, kanbanr merges nothing and the model is a description. The **code**
repository is never pushed, by kanbanr or by you: the user reviews and pushes, in every row. The
board's own remote is a different repository, which kanbanr syncs after each write as its push
policy says (`kanbanr remote push-policy`).

### Each milestone ends with a compliance review

Review the architecture as a whole: every component has a board doc that describes it as built,
its front matter (`items`, `adrs`, `covers`) is current, and the views are updated. `kanbanr trace
<MS> --zachman` reports a column nothing in scope answers. See "Architecture docs stay as built"
below.

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

`start` moves the item to the status whose gate makes the branch (`on_enter: [branch]`). With
the default workflow that is the first active status; under a phased workflow such as TOGAF it
is Implementation. If the item is further back, `start` names the stages in between: move it
through them with `kanbanr move`, since each has its own gate. With the git model on, `start`
branches from the line of integration `kanbanr git model` names, not always from main (see "The
method by stage role"). In a folder that isn't a git repository, `start` moves the item without a
branch. `finish` ends at a terminal status the
workflow allows from where the item is, and when every task is done the item advances there by
itself, unless that status's gate isn't met.

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

`kanbanr trace <G-2 | FEAT-046 | FEAT-046/R-2 | doc.md>` goes **down** — requirements, tests, documents,
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
- **A decision is the user's, never yours.** Draft it as `proposed` (the default — never pass
  `--status accepted`), then say it is waiting. The user accepts or rejects it on the Review page or with
  `kanbanr adr accept ADR-0007` / `adr reject ADR-0007 --reason "…"` in their own terminal,
  exactly as they approve a definition. Accept is refused while a section is unanswered; the verdict, who gave it and when
  are recorded on the decision and in the activity log.
- The links live **on the decision** (`affects`, `driven_by`); the item's view is derived, so it
  can never disagree. `kanbanr adr supersede ADR-0007 --replaces ADR-0003` writes both sides and
  names the items now standing on an overturned decision. Deciding is *work*: it is a task on the
  item that needed the decision, and the ADR is its output.
- A commit may add `Docs: design/mirror.md` and `ADR: ADR-0003`; both are checked against the
  board like any other reference.
- `kanbanr trace <MS-006> --zachman` reports which of the six columns nothing in scope addresses.
- **Diagrams draw, or the item does not move.** A `diagrams_valid` gate refuses a move while the
  spec or a doc in `affects_docs` has a Mermaid diagram the lint rejects; `kanbanr doctor` lists the
  same problems first, so fix them there before calling the item done.
- **The method is on by default.** Every built-in process asks for the Design and its
  `design-review` at the build stage, and `docs_updated` with `docs-folded` at the integrate stage
  (`kanbanr config method` shows where). Never turn a part off yourself: it is the user's choice,
  through `kanbanr config method --off <part>`.
- **Architecture docs stay as built.** Keep one board doc per component, describing it as it is,
  with front matter `items`, `adrs` and `covers`. Before an item is released, fold its Design into
  each doc in its `affects_docs`; a `docs_updated` gate refuses the move until each changed since
  the item entered its stage. End each milestone with an architecture compliance review.
- **Every reference in a spec, doc or ADR is a live link** — the board checks them:
  `[FEAT-012](/p/<project>/feature/FEAT-012)` (a requirement: `…/feature/FEAT-012#R-2`),
  `/p/<project>/milestone/MS-004`, `/p/<project>/charter#G-1`, and
  `/p/<project>/docs/file/<path>` for an ADR or a doc. A code in prose is linked too, but a link
  says you mean it. A file in the code repository is not a board doc: name it as a code path
  (`docs/src/using/processes.md`), never as a `/docs/file/` link. A design's **Affects docs** list
  is made of these links. `kanbanr doctor` reports any that point at nothing — fix them before
  calling the item done. `kanbanr trace <doc.md>` shows what a doc names and what links to it.

## Where the rules live

The board is the system of record for the project's **reasoning**; `CLAUDE.md` is a pointer at it;
this skill is the method; the hooks are enforcement. Each fact lives in exactly one of those — a
fact in two places drifts.

This skill carries the method ("The method by stage role"). A personal or project instruction file
that restates it is unnecessary, and it drifts from the tool. If one disagrees with the board's
gates, tell the user what differs and ask which holds.

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
- **Index every decision.** When an ADR is accepted, add a link to it in the architecture index
  (or whichever view the project's `adr` coverage rule names) in the same session.

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
  On a project with releases, an item planned into a release that has not been cut stays where it
  is, however its last task or sign-off arrives: the cut moves it to its end status. A move of your
  own to an end status needs a `--override "<reason>"`.
- **Before merging a sprint or cutting a release**, run `kanbanr doctor` and fix every coverage
  rule it lists, so the architecture views describe everything that ships.

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
**Pass `--kind` when creating, not after:** where the project maps kinds to prefixes
(`kanbanr config codes`), the kind chooses the code (`DEF-001` for a defect, `MNT-001` for a
chore, a counter per prefix), and a later kind change never renames it. Don't propose codes in the
setup interview: a new project keeps `FEAT-` unless the user asks for kind prefixes.

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
writing anything. A field an operation does not accept is refused, naming the operation and the
field, and nothing is written. Sprint membership is not a batch field: use `kanbanr sprint plan`
and `kanbanr sprint unplan`.

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
kanbanr serve                       # the monitor is built into the binary; nothing else to point at
kanbanr open                        # open it in the browser
kanbanr open --check [--json]       # is the monitor up, with writes; its Review and Releases addresses
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
kanbanr trace [G-2|FEAT-001|FEAT-001/R-2|doc.md] [--zachman]   # down the chain, ending in the gaps
kanbanr why src/cart.rs:42                      # up: annotation or trailer -> requirement -> goal
kanbanr adr new "…" [--affects …] [--driven-by FEAT-001/R-2] [--quality Reliability] [--zachman How]
kanbanr adr list [--for FEAT-001] | supersede ADR-0007 --replaces ADR-0003 | history ADR-0007
kanbanr adr accept ADR-0007 | adr reject ADR-0007 --reason "…"   # the user's, on the monitor; refused when Claude runs it
kanbanr lessons [--for FEAT-001] [--all]        # read BEFORE starting work
kanbanr lesson add "…" --kind pitfall --from FEAT-043 --evidence "…" [--tags a,b] [--goals G-1]
kanbanr lesson affirm L-1 [--note "…"] | contradict L-1 [--note "…"]
kanbanr split-from FEAT-060 FEAT-046   # this item was sliced out of that one
kanbanr start FEAT-001 [--to STATUS] [--no-branch] [--override "<reason>"]
kanbanr commit -m "…" [--ref R-2] [--ref TL-001/T3] [-a]   # trailer filled from the branch
kanbanr finish [FEAT-001]         # gated: tasks complete, requirements proven
kanbanr git install-hooks [--force] | uninstall-hooks | status
kanbanr git model [--json]                       # main, the line, item branches, protected lines, what kanbanr merges
kanbanr git model --sprints on|off --releases on|off   # preview a cadence's model; needs no board
kanbanr git forward <VERSION>                    # merge a release's fixes forward into the line
kanbanr git doctor [--json]                      # board vs branches; read-only, exits 1 on an error
kanbanr defect FEAT-002 --introduced-by FEAT-001 --found-in production [--severity …] [--root-cause …]
kanbanr approve FEAT-001 --rev <rev>   # the user's, on the monitor; refused when Claude runs it (so are ratify, unapprove)
kanbanr signoff FEAT-001 design-review [--note …] [--doc path]   # a named sign-off a stage asks for — the user's, on the monitor; refused when Claude runs it
kanbanr signoff FEAT-001 design-review --withdraw --reason "…"     # take one back (the user's too); the earlier entry stays as history
kanbanr move FEAT-001 Scheduled [--override \"<reason>\"]     # gated; the override is recorded
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
kanbanr config workflow --preset <name> | --from-file process.yaml | (--statuses … --transitions "A>B" --default-state … --displayed-states … --no-op-states …)
kanbanr config workflow --preset list             # what each preset is
kanbanr config cadence --sprints on --releases on --unit points   # both off unless switched on; setup asks for every process
kanbanr config method [--off|--on] [design|design-review|docs|docs-folded|links]   # the method's gates; the user's choice, never yours
kanbanr config coverage [--from-file rules.yaml | --clear]   # architecture coverage rules: what each view must reference
kanbanr feature edit FEAT-001 --points 5          # a story-point estimate (--estimate is days)
kanbanr sprint add --start 2026-10-05 --length 2w --goal "…" [--capacity 20]
kanbanr sprint plan SP-001 FEAT-001 FEAT-002      # warns over capacity
kanbanr sprint unplan FEAT-001                    # back to no sprint (not for a closed sprint)
kanbanr sprint start SP-001 | show [SP-001] | list | close SP-001 --carry-to SP-002|backlog
kanbanr sprint edit SP-001 [--name N] [--goal G] [--start D] [--length 2w | --end D] [--capacity N|0]   # only while planned
kanbanr sprint remove SP-001                      # only an empty, unstarted sprint
kanbanr release remove v0.1.0                     # only an empty, unstarted release
kanbanr retro --sprint SP-001                     # the sprint's items, what it carried, its burndown
kanbanr release add v0.1.0 --target 2026-10-31 | plan v0.1.0 FEAT-001 … | list
kanbanr release cut v0.1.0 [--tag]                # the user's, on the monitor; refused when Claude runs it. Ship the finished items, write notes, carry the rest
kanbanr release promote v0.1.0 [<stage>] [--reason "…"]   # the user's, on the monitor; refused when Claude runs it (so is each stage confirmed after the tag). Next or named stage, doing what its gates say (default candidate → main → tag: the first makes release/<V>, main fast-forwards main, the stage holding `tag` is the cut and tags); refused with every missing condition and its fix; back a stage needs --reason. Prints each step and next:. Run from the project folder; never pushes
kanbanr release promote v0.1.0 [<stage>] --plan [--fetch]  # what it would do and still needs; changes nothing
kanbanr gate list [--name ci] [--commit REV]   # recorded gate results, newest first (reading is yours; `gate record` is the user's)
kanbanr prepare gate ci --passed --url <the run> [--commit REV] [--note …] [--source S…]   # a CI result you read, for the person to confirm on the Releases page
kanbanr prepare signoff <CODE> <NAME> [--note …] [--doc PATH] --source S…   # a sign-off's note and doc, for the Review page
kanbanr prepare promotion <V> [--stage S] --checked "…" --source S…   # what was checked, for a stage confirmed after the tag
kanbanr prepare ratify <CODE> --reason "…" --source S…   # why the work went ahead, for the person to ratify
kanbanr prepare list [--all] [--item CODE] [--release V]   # what waits for the person, and where; stale ones say why
kanbanr release redate v0.1.0                     # date the notes and changelog today, before the cut
kanbanr release notes v0.1.0 [--file F | --edit]  # the notes on release/<V> and where; --file/--edit commits new text there until the cut
kanbanr release stages                            # each stage: requires, on_enter, declared or default
kanbanr config release-stage main --requires items_finished --on-enter move_main [--after ci] [--clear]
kanbanr config release-prepare --version-file api/Cargo.toml --changelog CHANGELOG.md --notes 'docs/src/releases/{version}.md'
kanbanr config release-prepare --version-file 'api/Cargo.lock=name = "a"\nversion = "{version}"' --version-file 'api/Cargo.lock=name = "b"\nversion = "{version}"'   # one file, several entries, each applied
kanbanr release add v0.1.1 --patch-of v0.1.0      # a patch: release/v0.1.1 from v0.1.0's tag, at the first stage
kanbanr config cadence --release-stages candidate,qa,live [--integration-branch develop] [--main-branch main] [--has-ci on]
kanbanr feature add --title "…" --milestone MS-001 --found-in v0.1.0   # feedback on a shipped release
kanbanr config workflow --export > process.yaml   # this project's workflow, gates included
kanbanr process check process.yaml|-              # check a process without applying it (- reads stdin): every problem, then its agreement and diagram
kanbanr process checks [--json]                   # what a gate can ask for, and when each check passes
kanbanr process list [--json]                     # the board's, your personal and the built-in processes, and who uses each
kanbanr process show <name>                       # a process's file, working agreement and diagram
kanbanr process save <name> [--personal]          # to the board (default) or ~/.kanbanr/processes; --from-file F, else this project's
kanbanr process status                            # which saved process this project uses, and whether it moved on
kanbanr process diff [<name>]                     # how this project's workflow differs from its saved process (or <name>)
kanbanr process update                            # take the saved version — only when the user says so

A phased process (togaf, pdca, design-control) or the organisation's own is **a workflow with
gates**. The phase IS the status; there is no second field. Offer one only if the user asks for
phases: it is a real commitment, the default suits most work, and it cannot be changed casually
once items are in flight.

Under a phased process, grow an item's definition stage by stage.
- **A named manual test is an open check.** `kanbanr check` lists `manual test not run: R-n: <name>`
  without blocking. Tell the user, and don't call the item done until someone has run it.
- **A finished item on the Review page** is there because its approval lapsed or text was added since: ask
  the user to read what changed and approve again, or say why the change doesn't need it.
- **Guides for each built-in process** are in the book, under Process guides. Point a user there
  when they ask how a whole project runs under their process.
- **Before each move,** run `kanbanr check <CODE>`. It names what the next stage needs: fill in
  that, and not everything at once.
- **Growth keeps the approval; changes lapse it.** Adding to the definition leaves the approval
  standing, to be reviewed at the next decision (before build) or at once (from build on, when the
  Review page shows it as `added`). Changing or removing approved text lapses it, so ask the user
  to approve again before the move.
- **A sign-off (`kanbanr signoff`) is the user's**, like an approval: say it is waiting on the Review page; never record it
  yourself.
- **Overrides:** `--override "<reason>"` gets past a gate on the record. Use it only when the
  user says so.
```

Feature items (a milestone is REQUIRED; code auto-generates):
```
kanbanr feature add --title "Login flow" --milestone MS-002 --spec "# Login\n…"   # or --spec-file path.md
kanbanr feature add --title "Save crashes" --milestone MS-002 --kind defect       # DEF-001 where codes map defect=DEF
kanbanr config codes [--set defect=DEF,chore=MNT] [--unset KIND] [--default P] [--clear]   # kind -> code prefix
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
