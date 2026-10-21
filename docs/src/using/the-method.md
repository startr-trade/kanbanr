# The method: why work exists

## The method: why work exists, and what proves it done

Everything above tracks *what* is being built. This section is about *why* — the part a board
normally loses. You rarely write any of it by hand: Claude drafts it in a plan and asks what it
cannot infer — see [Working with Claude](with-claude.md). It is opt-in: a project with no charter behaves exactly as it always did, and items
created before a charter was adopted are never reported against it.

### The charter — what the project is for

```bash
kanbanr charter show
kanbanr charter set --file charter.yaml     # purpose, vision, goals, non-goals, stakeholders
```

Goals carry ids (`G-1`, `G-2`) that work items link. A goal with no work behind it is a stated
intention nobody is delivering, and the Charter tab shows that; so is an item that serves no goal.

### Defining an item — the bar, and it is the same for everything

```bash
kanbanr feature define FEAT-001 --template --kind defect   # a skeleton shaped to the kind
kanbanr feature define FEAT-001 --file def.yaml            # write it
kanbanr check FEAT-001                                     # what it has not said, and cannot show
```

A definition states the item in one sentence, links a goal, answers the six interrogatives — what, how, where, when, who, why —
(what / how / where / when / who / why) in a line each, and carries **requirements** in
[EARS](https://alistairmavin.com/ears/) form with the tests that will prove them. Quality
requirements additionally carry an ISO/IEC 25010 characteristic and a measured scenario whose
measure names the test that checks it.

What varies by kind is only the *shape* of a requirement: a feature asserts new behaviour, a defect
names the requirement it violates, a chore asserts an invariant ("shall continue to …"). The bar
does not move. **Leave what you do not know blank** — `kanbanr doctor` reports a blank; it cannot
report an invented answer.

### Agreement before work

```bash
kanbanr review FEAT-001      # the one-screen decision brief — read this BEFORE building
kanbanr review --pending     # every item waiting, in one pass
kanbanr review --ui          # read and approve in the browser instead (see below)
kanbanr approve FEAT-001     # records agreement, pinned to the definition's content
kanbanr start FEAT-001       # refuses without a current approval
```

Approval is pinned to a hash of the definition, so editing the definition afterwards **lapses** the
approval rather than silently keeping it. The escape is explicit and recorded:
`kanbanr start FEAT-001 --override "why you are going ahead anyway"` (`--unapproved` still works), which stays on the item and
is reported by `doctor` until it is reviewed.

**Reviewing in the browser.** Reading a page of markdown in a terminal is a poor way to decide
anything, so `kanbanr review --ui` starts the monitor with writes enabled and opens the review queue:
one collapsible card per item, with the approve button inside the brief it belongs to. The ordinary
`kanbanr serve` monitor stays read-only and says so rather than offering a button that would fail.

The queue holds only items where agreement can still change something — not work that is finished, and
not a status parked off the board. Approving merged work records a signature that changes nothing, and
a gate that asks for those gets rubber-stamped, which is the failure it exists to prevent.

The one exception is work **finished under a recorded bypass** and never agreed to. That is still a
question for a person, so it heads the queue with a **Ratify** button instead of Approve.
Ratifying agrees to the work after the fact and is recorded as its own verdict, never passed off
as prior approval. It's the same list `doctor` reports, and `kanbanr ratify <CODE>` does the same
from the terminal.

**A verdict names who gave it.** `--by` defaults to the board's commit identity, and the monitor uses
the same one, so a verdict reads identically whichever surface recorded it. A verdict with no named
approver is **refused** rather than attributed to nobody — set an identity once with
`kanbanr identity --name "You" --email you@example.com`. An approval that cannot say who agreed is
not evidence of agreement.

**Taking one back.**

```bash
kanbanr unapprove FEAT-001 --reason "the requirements changed after the walkthrough"
```

The agreement goes and the item returns to the queue, start gate and all; the record of having given
it stays, because an approval given and later withdrawn says more than none ever having existed. A
reason is required — an agreement needs no explanation, taking one back does. The monitor offers the
same action on the item's page, collecting the reason in the page. Both verdicts emit an event
(`ApprovalRecorded`, `ApprovalWithdrawn`) and appear in the activity log.

### Evidence, not intentions

```bash
kanbanr test FEAT-001 R-1 cart::retains green    # normally you never run this by hand
kanbanr tests [--write]                          # tracked tests that no longer exist in the repo
```

A PostToolUse hook reads the output of every test run you make and flips the tracked tests to match,
stamped with the project revision it saw. Name a test exactly as your runner prints it
(`cart::retains_for_seven_days`, `src/cart.test.ts`) or the run cannot find it. A green recorded at
an older revision is reported as **stale evidence**, not as proof; re-running the suite refreshes
it. Mark a check a person performs as `kind: manual` — it is exempt from the rot sweep, so use it
only when a person really did it.

## The method by stage role

kanbanr carries one way of working for every process: a build-ready design before code,
architecture docs kept as built, live links, a git model that follows your cadence, and releases
that move through stages. It is the same under the default kanban, Scrum, TOGAF, PDCA or a process
of your own, because it is tied to the **role** a stage holds, not to what the stage is called
([Stage roles](processes.md#stage-roles)). Claude follows it from the skill; you don't need to
copy it into a personal or project instruction file, and a copy would drift from the tool.

**Roles first.** `kanbanr process show <name>` prints the working agreement with each stage's role,
and `kanbanr process checks` lists the roles and the checks a gate can ask for. `kanbanr config
method` says which parts of the method your project has on ([The method, on by
default](processes.md#the-method-on-by-default)). A part you turn off is no longer asked for, but
it is still good practice, and Claude never turns one off or on for you.

**At the design role.** The item's spec gets a `## Design` written at implementation level
([The design stage](processes.md#the-design-stage-a-build-ready-design-in-the-spec)), ending with
an implementation task list that a cheaper model can build one task at a time. The Design ends with
**Affects docs**, links to the board docs the change touches, mirrored into the definition's
`affects_docs`. Every reference is a live link ([Tying code to the reason for
it](traceability.md)). Decisions are drafted as proposed ADRs and put to you at once, and a question
the design leaves open comes to you, never to a subagent. Leaving the role, the build stage asks for
the Design and your **design-review** sign-off.

**At the build role.** `kanbanr start <CODE>` makes the item branch. With the git model on, it
branches from the line of integration `kanbanr git model` names, not always from main. Commits go
through `kanbanr commit`, which names the item. Subagents take one task at a time from the item's
todo-list. Nobody commits on main or on a line of integration (the hooks refuse), and kanbanr never
pushes.

**Sub-agents: what each may do.** When Claude hands work to a sub-agent, each role has fixed
boundaries, kanbanr enforces them, and Claude puts them in the agent's brief word for word:

| | A **build** agent (one item, in its own worktree) | A **design** agent |
|---|---|---|
| Read the board and the code | everything | everything |
| Its item's record | `kanbanr task state` as each task starts and lands; `kanbanr test` evidence for tests it actually ran green | nothing |
| Everything else on the board | refused: moves, approvals, sign-offs, definitions, specs, docs, other items, config; a `batch` only of these ops | refused |
| Code | commits only on its item's branch, each naming the item | no edits, no commits |
| Remote | never: no `git push`, no `kanbanr sync` | never |
| Tests | on temporary boards only, never a real board | — |

Before starting each sub-agent, Claude issues it a scope with one command,
`kanbanr scope issue <design|build> <CODE>`, starts the agent in its own worktree (design agents
too), and pastes the brief the command prints at the top of the agent's brief. The agent's first
command claims the scope; from then on every board write, commit and push from its worktree that
the scope does not allow is refused, naming the boundary it crossed. See
[Agent scopes](with-claude.md#sub-agents-and-agent-scopes).

Claude, as the coordinator, keeps the board current as the agents work, never waiting for their
reports, and takes back every question and every piece of work outside the brief. Verdicts, moves
and pushes stay with Claude and you; when the agent's work has been reviewed, Claude revokes its
scope.

**At the integrate role.** Before the move, the Design is folded into each doc in `affects_docs`,
describing it as built (the `docs_updated` check), and you sign off **docs-folded**. With the git
model on, the move merges the item branch into its line (`--no-ff`, naming the item). If the merge
would conflict, the move is refused and the item stays where it was: the line is merged into the
item branch, the conflict resolved there, and the move tried again; nothing is rebased.
`kanbanr git doctor` names any disagreement between the board and the branches.

**At the release role.** `kanbanr release promote <V> [stage]` moves a release through its stages
([Release stages](processes.md#release-stages)): the first makes `release/<V>`, and the last,
production, is the cut, where kanbanr tags the version and fast-forwards main to it. A patch is
`kanbanr release add <V> --patch-of <P>` ([Patch releases](processes.md#patch-releases)), and a fix
made on a release branch reaches the line through `kanbanr git forward <V>`. With sprints and no
releases, `kanbanr sprint close <SP>` is the cut.

**Carry-over.** `kanbanr sprint close <SP> --carry-to <next>` carries unfinished items and keeps
their branches. kanbanr merges the next sprint's branch into each carried item's branch; a conflict
is reported for that item and its branch is left as it was.

**Who merges, who pushes.** `kanbanr git model --sprints on|off --releases on|off` shows the model
for a cadence before you choose it (setup runs it for you), and `kanbanr git model` the one your
project follows:

| Cadence | Items branch from and merge into | Production moves at | kanbanr merges (with the git model on) |
|---|---|---|---|
| neither (trunk) | main | the integrate merge | each item branch into main |
| sprints only | `sprint/<SP>` | the sprint close | item branches into `sprint/<SP>`; `sprint/<SP>` into main at the close; the sprint into carried items |
| releases only | `integration` | the release cut | item branches into `integration`; `release/<V>` into main at its last stage |
| sprints and releases | `sprint/<SP>` | the release cut | item branches into `sprint/<SP>`; `release/<V>` into main at its last stage; the sprint into carried items |

With the git model off, kanbanr merges nothing and the model is only a description
([The git model](processes.md#the-git-model)). **You push your code repository, in every row**:
neither kanbanr nor Claude ever does. The board's own remote is a different repository, which
kanbanr syncs after each write as its push policy says (`kanbanr remote push-policy`).

**Each milestone ends** with an architecture compliance review: every component has a doc that
describes it as built, and the views are current ([Living architecture docs](living-docs.md)).

## On Zachman and TOGAF

kanbanr borrows from both without adopting either, and it is worth being precise about which half.

**Zachman: the columns, not the rows.** The six dimensions an item answers — what, how, where,
when, who, why — are Zachman's six interrogatives, and `kanbanr trace <CODE> --zachman` reports
which of them nothing in scope addresses. Zachman's *rows* — the perspective layers from Executive
down to Technician — are **not** modelled. There is a `layer` on an ADR (`conceptual` / `logical` /
`physical`) which gestures at the same idea, but it is three values on a decision record, not the
framework's six perspectives. So: a completeness checklist taken from Zachman, not an
implementation of the Zachman Framework, and nothing here obliges you to think in one.

**TOGAF: a preset, and nothing else.** `kanbanr config workflow --preset togaf` gives a board
whose columns are the phases, Vision → Business Arch → System Design → Implementation → Migration
→ Operations, with forward and backward transitions, because rework is normal. **The phase is the
status**; there is no second field to keep in step with it. Its gates grow the definition phase by
phase: Business Arch asks who, what and why; System Design asks how and where; Implementation asks
for a named test per requirement and makes the branch.

It is opt-in because a phase model is a real commitment. TOGAF is one preset among several (PDCA,
a design-control flow, or your organisation's own), and a project that never asks for one never
sees it. See [Processes](processes.md).

Neither is recommended. The bar kanbanr actually holds you to is the one above: state why, link a
goal, carry requirements, show evidence. The two frameworks supply a vocabulary for the *why* and
an optional shape for the *when*, and a project that uses neither passes every check.
