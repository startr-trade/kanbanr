# Working with Claude

You can run kanbanr from Claude Code. You ask for work and answer questions, Claude records the
work on the board, and you take every decision on the monitor's pages. This chapter
shows what that conversation looks like and why it asks what it asks.

Three things are worth knowing up front:

- **Your workflow decides the questions.** Claude doesn't follow a script for TOGAF or Scrum. At
  each step it asks the board what the next stage still needs and turns each gap into a question
  or a draft. Change your workflow and the conversation changes with it.
- **Decisions are taken on the pages.** An approval, ratification, sign-off or its withdrawal, and
  accepting or rejecting a decision, are taken on the **Review** page. Promoting or cutting a
  release is taken on the **Releases** page. Claude tells you what is waiting there, with the
  page's address, and never records one for you: kanbanr refuses if it tries, and never asks for
  one in a plan or a question. If no monitor accepts writes, Claude offers to start one with
  `kanbanr review --ui`.
- **Decisions arrive filled in.** What Claude already knows, such as a CI run's link and result or
  what a sign-off covered, is prepared beside the button with where it came from. You check it and
  click, or change it first ([Prepared by Claude](the-monitor.md#prepared-by-claude)).
- **A verdict records where it was taken.** The monitor, a command in your terminal and an answer
  recorded in a Claude Code session are told apart, and the item shows which beside each approval
  and sign-off. Deciding the same thing twice records nothing the second time.
- **A verdict covers what you were shown.** The page pins your choice to the definition you saw
  (`--rev`). If the definition changed in between, it refuses and you read the change first.

See [the monitor](the-monitor.md) for the pages.

This chapter explains how the conversation works under any process. To follow one project from setup
to a shipped release, see the [Process guides](../guides/index.md).

## Setting up: the cadence and the git model

When Claude sets kanbanr up for a project, it asks where the board lives, which process to use,
and then, for every process, **whether to work in sprints and releases**. It recommends the
process's own setting (on for Scrum and agile, off for the rest) and shows you the git model your
answer gives before anything is written:

```text
$ kanbanr git model --sprints on --releases off
topology:   sprints
main:       main
line:       none — start a sprint to have one
branch at:  sprint/<SP>
production: at the sprint close
…
kanbanr merges (once the model is on):
  - each item branch into sprint/<SP> when the item enters the integrate stage (--no-ff, naming the item)
  - sprint/<SP> into main when the sprint closes (a fast-forward, tagged with the sprint's code)
  - sprint/<SP> into each item branch carried into that sprint (a catch-up merge; a conflict is reported for that item)
pushing:    never automatic — you review and push
automation: on
```

Choosing sprints or releases turns the git model on.

The plan quotes that output under *Branching*, lists which stage holds each role (design, build,
integrate, release) under *Process*, and says that the method's gates are on. Approving the plan
applies it. What each role then asks for is in [The method by stage role](the-method.md#the-method-by-stage-role).

Not sure which process to pick? [Choosing a guide](../guides/index.md) lists every process and its
guides, and [Processes](processes.md) compares them and their gates.

## Starting work: a definition you approve

Ask for what you need, in your own words:

> A shopper opens their basket on our shop's web page, leaves, and comes back the next day to find what they chose still there.

Claude reads the charter and the code, then asks only what it can't work
out, a few questions at a time:

```
Which goal does this serve?   ● G-1 A customer can pay for a basket on their own   ○ a new goal
Who is it for?                ● returning shoppers   ○ guests too   ○ other…
```

The definition it writes is the item's brief: a one-line statement, the goals it serves, the
[six dimensions](the-method.md), EARS requirements each naming its test, and the stage it starts
in. It goes only as far as the first stage asks, unless you ask for the whole definition up front.

**Choices are yours.** Anything you'd recognise as a decision (a business rule, a retention period,
a limit, what's in or out of scope) is asked before the definition is written, with Claude's recommendation first.
Anything Claude still had to choose is listed in the brief under "Choices I made", so your
approval is informed. If one request spans several stages, the brief says which ones it covers.

Claude writes the definition to the board, shows you its brief in a few lines, and says it is
waiting for your approval on the Review page, with the link. Nothing is recorded as approved until
you approve it there.

## Deciding: on the Review and Releases pages

When decisions are waiting, the session start mentions them. Claude says in one message how many
there are, which, and the address of the page where they wait. It doesn't list them as questions.
If you ask it to explain one, it does, and sends you back to the page.

| You decide | Where |
|---|---|
| Approve a definition, or ratify work done under a recorded override | **Review** page |
| Sign off a stage (for example *release*) | **Review** page |
| Accept or reject a proposed design decision | **Review** page |
| Withdraw an approval or a sign-off | the item's page |
| Promote or cut a release, and confirm each stage after the tag | **Releases** page |
| Change it | tell Claude what to change: it revises the definition, and the item comes back to the queue |

Checks that need a push are marked to be checked after it; Claude records them once CI has run;
anything still unchecked at the cut waits for you to accept it, and the notes say so. An override
Claude makes past a gate waits on the Review page for you to ratify, and the item doesn't ship
until you do.

Claude may run `kanbanr release promote <V> --plan`, `kanbanr sprint close` and
`kanbanr release plan`, which record no verdict. If a verdict was ever recorded that you did not
give, Claude says so at once and asks you to withdraw it on the item's page.

## Where decisions are taken, and delegating them

Each project chooses where its decisions are taken. In **monitor** mode, the default, every
decision is yours, on the Review and Releases pages, as above. In **delegated** mode you still
decide there, and Claude may also take the decisions you have delegated to it, for a time.

To delegate, tell Claude what it may decide, for example: "you can sign off the design review of
small defects for the next two weeks". Claude proposes a delegation: which verdicts (approve,
ratify, or a named sign-off), for which items, kinds of item, milestone or stages, and until when
(two weeks unless you say otherwise, three months at most). A proposal does nothing until you
grant it: it waits in the **Decisions here** panel at the top of the Review page, where you grant
it or discard it, and where you switch the project to delegated mode. Only you can grant one,
change the mode, or review what was decided; kanbanr refuses Claude all three.

When you set a project up, Claude proposes the delegations its process usually wants, and the
setup plan shows them under **Decisions**: for most processes, the design review of defects and
chores, and the docs fold-in of every item; for TOGAF, only the docs fold-in, so Business
Architecture and System Design stay yours; for design control, none. Each covers the stage an item
is in when that sign-off is asked: a sign-off named for a stage, such as the fold-in "at
Migration", is the one for the move into it, so it is taken in the stage before. Only forward
moves are delegated: when an item has moved back to an earlier stage, Claude asks you before
deciding or changing anything for it, even if a delegation would cover it. You grant them on the
Review page after setup, or don't.

When Claude takes a decision under a delegation, it tells you which one and under which delegation,
and the decision is recorded as delegated, by Claude, naming the delegation. It comes back on the
Review page under **Delegated decisions to review**: **Keep** it, or **Undo** it, which withdraws
it with a reason that names the delegation. Until you do, a release holding the item shows it as a
warning beside its promote button and in `kanbanr release promote --plan`; it never blocks the
release. A delegation can't cover a release, a withdrawal, a
design decision (ADR) or the decisions settings themselves, and only the Claude you are talking to
acts under one, never a sub-agent it briefs. Revoke a delegation on the same panel at any time;
switching back to monitor mode suspends them all.

Outside a delegation nothing changes: a verdict run inside a Claude session is refused by kanbanr
itself, not only by its hook. This keeps Claude to what you delegated through kanbanr's own
commands and hooks, and makes any departure visible; it is not a security boundary against a
program running as you.

## Plan mode

Claude uses plan mode for the setup interview and for designing or changing your process. It never
uses it for a routine move or a new item, and never while a sub-agent is running, because entering
it pauses the agent.

While agents build, Claude tells you only what changed: a task landed, a failure, a stall, a
decision waiting. It sends nothing for a report that repeats one it gave. Rework goes on the item's
todo-list before anyone starts it, and kanbanr refuses a build scope for an item with no open task.

## Moving work: just ask

> Park FEAT-074, we're not doing it this quarter.
>
> Put FEAT-150 back to Planned.

Claude runs `kanbanr move`. If a stage's gate refuses, Claude says what the stage still needs and
offers to work through it, using the table below. To run a command yourself without leaving
Claude Code, start the line with `!`:

```
! kanbanr move FEAT-074 Deferred
```

## Sub-agents and agent scopes

When Claude splits work across sub-agents, a design agent to draft, build agents in their own git
worktrees to write the code, each agent's limits are not left to its brief. Claude issues each one
a **scope** first, and kanbanr enforces it (FEAT-214):

```
kanbanr scope issue build FEAT-214            # --for 8h by default, 72h at most; --note "T1–T4"
```

The command records the scope on the board (who issued it, the item, what it allows, the item's
branch, when it expires) and prints the boundary table Claude pastes at the top of the agent's
brief. The agent starts in its own worktree (`isolation: worktree`), and its first command is the
one the brief names, `kanbanr scope claim SC-007`. The claim binds the scope to that worktree, on
the board and in a pointer file in the worktree's own git directory, and switches a build agent to
its item's branch.

From then on, in that worktree:

| | A **build** scope | A **design** scope |
|---|---|---|
| Board writes | its own item's task states and test evidence; a batch only of those | none |
| Commits | on its item's branch, each naming its item | none |
| Pushing | never: `kanbanr sync`, `git push` and the board's push policy are all refused | never |
| `identity`, `hooks`, `migrate`, `serve --allow-writes`, `start`, `finish`, … | refused | refused |

A refusal names the scope and the boundary it crossed:

```
refused by agent scope SC-007 (build, FEAT-214): `move feature FEAT-214` is not one of its
operations. Under this scope you may: `kanbanr task state FEAT-214 …`, `kanbanr test FEAT-214 …`,
commits on feat/FEAT-214-agent-scopes naming FEAT-214. Report what you needed to the coordinator; do
not work around it.
```

A worktree under `.claude/worktrees/` whose name starts with `agent-`, the name Claude Code gives an
agent's, is refused every write and commit while no scope is bound to it. A worktree you or the
coordinator make there for your own work, such as one to resolve a merge, takes any other name and
commits under the ordinary hooks; once a scope is bound to a worktree, it holds whatever the
worktree is called.
When the agent finishes its item's last task, the item stays where it is: Claude moves it. Every
write and commit an agent makes is recorded as you, by the agent for its item: on the activity
entry and the event, and as an `Agent-Scope:` trailer on the board commit and the code commit.

Claude keeps an eye on the agents' branches: `kanbanr scope drift` names a build agent's commits
that are newer than its item's last task change, and so do the Stop hook, session start and
`kanbanr git doctor`. When the agent's work has been reviewed, Claude revokes its scope
(`kanbanr scope revoke SC-007 --reason done`); `kanbanr scope list` shows the ones in force, and
`kanbanr scope renew SC-007 --for 4h` extends one. Your own checkout is never scoped.

A scope binds the agent it was issued to, never whoever stands in its folder. The three hooks
(the Bash guard, the Write guard and test capture) tell a sub-agent's calls, which carry an
`agent_id`, from Claude's own, and a command run in your own terminal is not an agent's at all. So a
scope never governs you or Claude's session, and a revoke stops the agent on its next tool call and
frees every other shell at once. Issuing a scope therefore needs those hooks: `kanbanr hooks
install`. Claude stays in the project folder and looks inside an agent's worktree with `git -C
<path>`, `make -C <path>` or `npm --prefix <path>`; a `cd` into one is refused, with the way out
named.

Scopes guard against mistakes and casual crossings, not a hostile process: the agent runs as you,
with your file permissions. A script that leaves the worktree, a commit with its hooks switched off
inside a script, or a hand edit to the board's `scopes.yaml` is not stopped, though every scoped
write carries its agent and a trailer, so an unattributed change stands out in the history.
A board that holds scopes is governed wherever it is, so setting `TMPDIR` does not make it look
like a test board; only a board in the temporary directory that holds no scopes is exempt.

## Why it asks what it asks

Claude asks the board, `kanbanr check <CODE> --json`, what each possible next stage still lacks.
The answer comes from your [workflow's gates](processes.md): the stage's purpose, the failing
checks, the warnings, and the sign-offs needed. Each kind of check becomes one kind of action, the
same in every workflow:

| Check | What Claude does |
|---|---|
| `definition` | drafts the definition into the plan |
| `statement` | drafts the one-line statement into the plan |
| `goals` | proposes the goal it serves, from your charter; asks if it isn't clear |
| `goals_known` | asks about a goal the charter doesn't have, or offers to add it |
| `zachman` | drafts the dimensions the stage names (what, how, where, when, who, why); asks what it can't infer |
| `requirements` | drafts EARS requirements into the plan |
| `ears` | rewrites a requirement into EARS form |
| `quality` | drafts the quality scenario, and asks for the target (for example "300 ms at p95") |
| `tests_named` | names a test for each requirement |
| `spec_sections` | drafts each named section of the spec into the definition; for `Design`, the build-ready design and its implementation task list |
| `docs_updated` | folds the item's design into each doc in `affects_docs` before the move, and says which docs it changed |
| `diagrams_valid` | fixes each Mermaid diagram the lint names, in the spec or a doc in `affects_docs`, then runs `kanbanr doctor` to confirm none is left |
| `docs_cover` | adds a link to the item in each view doc a coverage rule names (the Zachman matrix, the catalog), then moves; it never asks you |
| `small` | proposes splitting the item |
| `approved` | your approval on the Review page; Claude says it is waiting and links to it |
| `bypass` | your ratification on the Review page |
| `signoff` | a "Sign off *name*" question |
| `estimated` | a question: how big, in your project's unit (points or days) |
| `in_sprint` | a question: plan it into the running sprint, or start its sprint today? |
| `in_release` | a question: which planned release? |
| `tests_green` | **never a question**: evidence decides; Claude reports what isn't green yet and makes it green |

A **warning** goes into the plan as a choice: fix it, or go ahead with the warning on record. When
more than one next stage is possible, Claude asks which. Each plan is headed with the stage's own
`purpose` from your configuration.

So if you edit your process (say, to ask *when* a feature happens at your business-architecture
stage), Claude starts asking "When does this happen?" there, with nothing else to change.

## A worked example: one feature through TOGAF

The [TOGAF preset](processes.md) runs Vision → Business Arch → System Design → Implementation →
Migration → Operations, and the definition grows phase by phase. The gate output below is what
kanbanr prints.

**Vision.** A statement and the goal. You accept a short plan; Vision has no gate, so nothing needs
approval yet.

**Business Architecture.** The board reports:

```
→ to move to Business Arch (Who it is for, what it must do, and why — the behaviour, as requirements.), still needed:
    no requirements · not approved · [MISSING: What] [MISSING: Who] [MISSING: Why]
```

Claude drafts who, what and why and the requirement ("WHEN a shopper returns within 7 days, THE
SYSTEM SHALL restore their cart."), asks what only you can decide ("any device, or only the same
browser?"), and you approve the definition on the Review page. That's approval 1.

**System Design.** Now only `[MISSING: How] [MISSING: Where]`. Claude proposes the design and checks
it against the requirement you agreed: "server-side storage keyed by account satisfies R-1 across
devices; browser storage would not". It asks for a quality target. The definition grew, so the
earlier approval stands, since text was only added, and the additions are reviewed at the next
decision, the design review. Nothing to approve here. The gate's warning (`R-2 is
a quality requirement with no scenario`) is in the plan for you to fix or accept. The architecture
decision is drafted as *proposed* for you to accept.

**Implementation.**

```
→ to move to Implementation (Build it, with a test named for every requirement.), still needed:
    R-1 has no test — it cannot be shown to be met
    R-2 has no test — it cannot be shown to be met
```

Claude names the tests and asks once: Approve / Change it / Skip. That's approval 3. The branch is
created here, so code starts only after the design was agreed.

**Migration.** No question: the move waits until the tests are green.

**Operations.** One question, "Sign off release?", with the tests re-checked.

Your part, in all: a short plan, two design plans, and three one-click questions. Each approval
covers more than the one before, and the item's history keeps the trail. If you already know the
design, ask for the whole definition up front: one plan and one approval then cover every stage
where nothing changes.

For a whole project on TOGAF with sprints and releases, from setup to a shipped release, see
[TOGAF with Claude](../guides/togaf-with-claude.md).

## What it looked like in practice

These are from the walkthroughs that tested this flow before it shipped, on a TOGAF project and on
a project with its own process file:

- **One request, two stages.** "Take saved carts to business architecture, then design it" produced
  one plan covering both stages. Accepting it approved the definition at each stage, and the item
  went Vision → Business Arch → System Design, through each gate in turn.
- **Choices asked first.** Before the plan for named baskets, Claude asked about renaming, items no
  longer for sale, and deleting. The plan then listed what it had still chosen itself ("a named
  basket is a snapshot") under *Choices I made*.
- **A drafted decision, asked at once.** The design decision behind saved carts came up as Accept /
  Reject / Later, not as a command to type.
- **A review with Change it.** Two items were waiting. Approve recorded the first. "Change it — no
  cap on named baskets" removed the cap. That put the definition back to text already approved,
  so the earlier approval counted again and nothing new was asked: approvals follow the content,
  not the edit history.
- **The process decides the questions.** The project whose own process file asks *when* at its
  shaping stage was asked "when does this happen?" there. The TOGAF project, whose gates never ask
  it, was not, and left How and Where blank until its System Design stage asked for them.

## Your own process, designed in the conversation

At setup, or whenever you say "let's change our process", Claude designs the process with you
instead of asking for a file. It offers the processes your team already saved first. For a new one
it starts from the closest built-in process. Then it asks one question per stage, *what must be
true before work enters it?*, with only the checks kanbanr can enforce as choices; anything else
becomes a named sign-off. The plan shows the result as a working agreement and a diagram, and
accepting it saves the process (on the board for the team, or in your personal library) and applies
it.

When a saved process changes, each project using it is told: at the start of the next session,
Claude mentions it once and asks whether to update. It never updates on its own. See
[Processes](processes.md#designing-a-process-with-claude).

## In other workflows

The same conversation, with only as many decisions as each workflow asks for:

| Workflow | Your decisions per item | Guide |
|---|---|---|
| default (Kanban) | approval and `design-review` to start; `docs-folded` to complete. The move to Completed doesn't ask for green tests, so ask Claude to record them first | [Default](../guides/default-with-claude.md) |
| scheduled | approval and `design-review` to schedule; `docs-folded` to complete. Completed doesn't ask for green tests either | [Scheduled](../guides/scheduled-with-claude.md) |
| PDCA | approval and `design-review` at Do; `docs-folded` at Check; a `review` sign-off at Act | [PDCA](../guides/pdca-with-claude.md) |
| design control | approval and `design-review` at Design; `verification-review` before Verification; `docs-folded` at Validation; `validation` to reach Released | [Design control](../guides/design-control-with-claude.md) |
| Scrum | its size as a question at Ready (the Definition of Ready); approval, `design-review` and its sprint to start; green tests and `docs-folded` for Done; the release is yours to promote and cut | [Scrum](../guides/scrum-with-claude.md) |
| agile | approval at Design; `design-review` at Develop; green tests and `docs-folded` at Review; the release is yours to promote and cut | [Agile](../guides/agile-with-claude.md) |
| your own process | whatever its gates ask, as the table above | |

For the whole journey under one process, including its sprints and releases, see the
[Process guides](../guides/index.md).
