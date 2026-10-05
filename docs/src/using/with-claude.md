# Working with Claude

You can run kanbanr entirely from Claude Code. You don't need a browser or a terminal. You ask for
work, accept plans, and answer questions, and Claude records everything on the board. This chapter
shows what that conversation looks like and why it asks what it asks.

Three things are worth knowing up front:

- **Your workflow decides the questions.** Claude doesn't follow a script for TOGAF or Scrum. At
  each step it asks the board what the next stage still needs and turns each gap into a question
  or a draft. Change your workflow and the conversation changes with it.
- **Only your explicit choice counts.** An approval, ratification or sign-off is recorded only when
  you accept a plan or pick that answer in a question. Claude never infers it from the
  conversation.
- **A verdict covers what you were shown.** Claude records your choice pinned to the definition you
  saw (`--rev`). If the definition changed in between, the command refuses and Claude asks again.

The [Review page](the-monitor.md) and the plain commands still work, for anyone who prefers them.

## Starting work: a plan you accept

Ask for what you need, in your own words:

> We need saved carts: shoppers should be able to come back to their basket.

Claude enters **plan mode**. It reads the charter and the code, then asks only what it can't work
out, a few questions at a time:

```
Which goal does this serve?   ● G-1 A customer can pay for a basket on their own   ○ a new goal
Who is it for?                ● returning shoppers   ○ guests too   ○ other…
```

The plan it writes is the item's brief: a one-line statement, the goals it serves, the
[six dimensions](the-method.md), EARS requirements each naming its test, and the stage it starts
in. It goes only as far as the first stage asks, unless you ask for the whole definition up front.

**Choices are yours.** Anything you'd recognise as a decision (a business rule, a retention period,
a limit, what's in or out of scope) is asked before the plan, with Claude's recommendation first.
Anything Claude still had to choose is listed in the plan under "Choices I made", so accepting is
informed. If one request spans several stages, the plan says which ones it covers.

**Accepting the plan creates the item and records your approval of exactly that text.** Sending it
back with changes revises the plan, and nothing is recorded until you accept.

## Deciding: one question per item

When decisions are waiting, the session start mentions them. Say "let's review" and Claude takes
them one at a time. It shows each brief in a few lines and asks one question whose answers are the
verdicts that item can take:

```
FEAT-012 Saved carts: 2 requirements, both with named tests; approval lapsed when R-2 was added.
   ● Approve   ○ Change it   ○ Skip
```

| You choose | What is recorded |
|---|---|
| Approve | `kanbanr approve FEAT-012 --rev <rev>`, in your name |
| Ratify (work done under a recorded override) | `kanbanr ratify FEAT-012 --rev <rev>` |
| Sign off *release* (a stage asks for it) | `kanbanr signoff FEAT-012 release --rev <rev>` |
| Accept / Reject (a proposed design decision; asked as soon as Claude drafts one, or later in a review) | `kanbanr adr accept ADR-0007` / `adr reject … --reason "…"` |
| Change it | nothing yet: Claude asks what to change, revises the definition, and the item comes back to the queue |
| Skip | nothing: it stays waiting |

## Plan or question?

Claude uses each one for a single job:

- **Plan mode** is for *writing* a definition: a new item, or growing one for its next stage.
  There's a brief to draft, and you read the whole text before you accept it.
- **A question** is for *deciding* on a definition that already exists. There's nothing to write,
  so each item gets a few lines and one question. A ten-item review is ten clicks, not ten plans.

"Change it" in a review is usually a small edit, so it's shown and asked again rather than planned.
A change that amounts to a new stage's worth of definition ("take FEAT-002 to System Design") goes
through plan mode.

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
| `small` | proposes splitting the item |
| `approved` | your acceptance of the plan, or an Approve question when nothing needs drafting |
| `bypass` | a Ratify question |
| `signoff` | a "Sign off *name*" question |
| `estimated` | a question: how big, in your project's unit (points or days) |
| `in_sprint` | a question: plan it into the active sprint? |
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
browser?"), and you accept the plan. That's approval 1.

**System Design.** Now only `[MISSING: How] [MISSING: Where]`. Claude proposes the design and checks
it against the requirement you agreed: "server-side storage keyed by account satisfies R-1 across
devices; browser storage would not". It asks for a quality target. The definition grew, so the
earlier approval lapsed (`Approval lapsed — the definition changed after it was approved`), and
accepting this plan approves the larger definition. That's approval 2. The gate's warning (`R-2 is
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

| Workflow | Your decisions per item |
|---|---|
| default (Kanban) | one approval to start; Completed when the tests are green |
| Scrum | its size as a question at Ready (the Definition of Ready); approval and its sprint to start; Done needs green tests |
| PDCA | one approval at Do; a review sign-off at Act |
| design control | approval, plus sign-offs at Review and Validation |
| your own process | whatever its gates ask, as the table above |
