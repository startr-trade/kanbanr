# Design control with Claude

This guide takes one item through the `design-control` process, from planning to released. The
process is modelled on the controls of ISO 9001 clause 8.3 (design and development). It is modelled
on them, not a claim of compliance: that is for a certification body to decide.

You don't need to learn any commands:

- **You talk to Claude** for everything except decisions and pushes. Use your own words. The ones
  here are only examples.
- **You decide on the monitor:** approvals and sign-offs on the **Review** page.
- **You push from your terminal.** Nothing reaches GitHub until you do.

Prefer to run the commands yourself? See [Design control from the command line](design-control-cli.md).
For the other processes, see the [guides index](index.md) and
[Processes: presets, gates and sign-offs](../using/processes.md).

## How it differs from the others

- **It uses no sprints and no releases by default.** Setting it up turns the git model on and leaves
  sprints and releases off. Work goes from an item's branch straight to `main`, so there is no
  release to promote and no cut. You can turn sprints and releases on later if you want them.
- **Released is a stage, not an event.** An item is Released when you sign off its validation. That
  is the end of the process.
- **It asks for four sign-offs.** Each one is your recorded decision, at the stage that needs it.
- **Merging happens at Validation**, not at Released. See the table below.

## The stages

```mermaid
flowchart LR
  A["Planning"] --> B["Inputs"]
  B --> C["Design"]
  C --> D["Review"]
  D --> E["Verification"]
  E --> F["Validation"]
  F --> G["Released"]
```

You can send an item back to Design from Review, Verification, Validation or Released. An item that
turns out not to need work can go to No Action, Not Applicable or Out-of-Scope.

## Setting it up

> Start using kanbanr for this project. Use the design-control process.

Claude asks where the board should live and what the project is for, then writes a plan. **Accepting
the plan applies it.**

> Agree a charter with me first.

**Do the charter first.** Gates aren't enforced until a project has one, and items created before it
are exempt. Claude asks for the project's purpose and goals. Every item then links to a goal.

> Open the monitor, with writes on.

Keep it open. Every decision below happens there.

## One item, stage by stage

Ask for the work in your own words, and Claude asks only what it can't work out.

> We need to restore a shopper's cart when they come back. Take it through design control.

| Stage | What Claude does | What you do on the Review page |
|---|---|---|
| **Planning** | Creates the item and writes what it is | Nothing |
| **Inputs** | Records the statement, the goal it serves and the requirements, each in the EARS form | Nothing |
| **Design** | Makes the item's branch, writes the design and its task list, and names a test for each requirement | **Approve**, then **Sign off design-review** |
| **Review** | Reviews the design against the inputs | Nothing |
| **Verification** | Checks the outputs meet the inputs | **Sign off verification-review** |
| **Validation** | Builds and runs the tests, updates the docs, then merges the branch into `main` | **Sign off docs-folded** |
| **Released** | Nothing | **Sign off validation** |

**Approve first, then sign off.** An item with both waiting shows **Approve**, then **Sign off** once
you've approved it, so check back after approving.

**Each sign-off is your decision, recorded.** Claude never gives one for you. If it isn't there, the
item stays where it is and Claude tells you which one is missing.

**Just ask to move it on:**

> Move the cart restore on to Design.

If something's missing, Claude tells you what and either fixes it or asks you. Nothing moves past a
gate unless you say why, and that reason stays on the item.

**Review has no sign-off of its own.** The review is recorded as the `verification-review` sign-off
you give before Verification starts.

## When an item counts as done

Only **Released** ends the process. Until you sign off validation, the item is not finished, even
when its code is already merged. Merging into `main` happens when the item enters Validation, so the
code can be on `main` while the item is still waiting for your last sign-off.

If you turn sprints on, ask Claude how you want an item to count as done for a sprint, because
Released is the only end stage.

## Handing work to sub-agents

> Have a sub-agent build these tasks.

Claude gives each agent a scope, which kanbanr enforces. An agent can record its own tasks and test
results and commit on its own branch. It can't move items, approve anything, give a sign-off or push.
Its progress shows on the item's page, and its scope on the **Workflow** page. When it finishes,
Claude reviews the branch and brings you any decisions.

## Pushing

Moving an item through the process doesn't push anything. Merging into `main` happens on your
machine only.

> The cart restore is released. What do I push?

Claude tells you. Then you push `main` from your terminal.

## Who does what

| | You | Claude |
|---|---|---|
| **Inputs** | Answer what only you know | Draft the statement and requirements |
| **Design** | **Approve**, then **Sign off design-review** | Write the design and its task list |
| **Verification** | **Sign off verification-review** | Check outputs against inputs |
| **Code and tests** | Review what you want to | Build, or give the work to sub-agents, and record results |
| **Merging** | **Sign off docs-folded** | Update the docs and move the item |
| **Release** | **Sign off validation** | Move the item |
| **GitHub** | **Every push** | Nothing |

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| Moves go through that shouldn't | No charter, or the item is older than it | Ask Claude to agree a charter before creating items |
| An item is back on the Review page | Its definition changed after you approved it | Read the change, then **Approve** again |
| A move is refused for a missing sign-off | You haven't given the one that stage asks for | Sign off on the Review page |
| A move to Validation is refused for tests | A requirement has no green test yet | Ask Claude to build and run it |
| The board warns a finished item's branch is merged | The branch is no longer needed | Ask Claude to delete it |

## What Claude runs

You never need these, but here they are if you want to check what Claude did.
[Design control from the command line](design-control-cli.md) covers them step by step.

| You asked for | Claude runs |
|---|---|
| Set up the process | `kanbanr config workflow --preset design-control` |
| The git model | `kanbanr config cadence --git-model on` |
| The charter | `kanbanr charter set` |
| A new item | `kanbanr feature add`, `kanbanr feature define` |
| Checking what's missing | `kanbanr check` |
| Starting the build | `kanbanr start` |
| Moving an item | `kanbanr move` |
| Recording test results | `kanbanr test` |
| Committing | `kanbanr commit` |
