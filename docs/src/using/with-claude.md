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

## In other workflows

The same conversation, with only as many decisions as each workflow asks for:

| Workflow | Your decisions per item |
|---|---|
| default (Kanban) | one approval to start; Completed when the tests are green |
| Scrum | its size as a question at Ready (the Definition of Ready); approval and its sprint to start; Done needs green tests |
| PDCA | one approval at Do; a review sign-off at Act |
| design control | approval, plus sign-offs at Review and Validation |
| your own process file | whatever its gates ask, as the table above |
