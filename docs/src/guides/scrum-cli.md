# Scrum, sprints and releases from the command line

This guide takes a project from setup to a shipped release using Scrum, with you running each
command. To work by talking to Claude instead, see [Scrum with Claude](scrum-with-claude.md). Other
processes are listed in the [guides index](index.md).

The commands and output come from a real run on a scratch project. `<tmp>` stands for a temporary
folder.

## Three clocks

| | What it answers | Moves when |
|---|---|---|
| **Stage** | How far has this item got? | Its gates are met, one item at a time |
| **Sprint** | What are we working on these two weeks? | You start and close it |
| **Release** | What has reached users? | You promote it, stage by stage |

Scrum switches sprints, releases and points on. An item is *done* for its sprint at **Done**, and
*released* only when a release that holds it is cut.

```mermaid
flowchart LR
  A["Ready"] -->|"kanbanr start"| B["item branch"]
  B -->|"move to Done"| C["sprint/SP-001"]
  C -->|"promote to candidate"| D["release/v1.0.0"]
  D -->|"promote to main"| E["main"]
  E -->|"cut"| F["tag v1.0.0"]
```

## Setting it up

In your project folder:

```text
kanbanr init shop --data-dir <tmp>/shop-board --author Ada --email ada@example.com
kanbanr config workflow --preset scrum
kanbanr config cadence --git-model on
kanbanr git install-hooks
```

```text
sprints on, releases on, estimates in points
installed commit-msg and pre-commit and pre-push in <tmp>/shop: a commit now has to say which item it serves
```

`kanbanr git model` shows what the git model will do. Nothing is pushed for you:

```text
topology:   sprints and releases
main:       main
line:       none — start a sprint to have one
branch at:  sprint/<SP>
production: at the release cut
items:      feat/{code}-{slug}, fix/{code}-{slug}, spike/*
protected:  main, sprint/*, release/*
kanbanr merges:
  - each item branch into sprint/<SP> when the item enters the integrate stage (--no-ff, naming the item)
  - release/<V> into main at the release's last stage (tagged <V>, a fast-forward)
  - sprint/<SP> into each item branch carried into that sprint (a catch-up merge; a conflict is reported for that item)
pushing:    never automatic — you review and push
automation: on — start, sprints and the hooks follow this model
```

In Scrum the integrate stage is **Done**, so that is where an item's branch merges.

### The charter comes first

**The gates are not enforced until the project has a charter**, and an item created before the
charter was adopted is exempt from them. Without one, moves that should be refused still succeed,
but each says what the gate would have needed and that `kanbanr charter set` turns the gates on.
So agree the charter before creating any items:

```text
kanbanr charter set --file charter.yaml
```

```text
charter saved for 'shop': 1 goal(s) G-1
```

### Release stages

By default a release has four stages and the cut does everything at once. We recommend three, so
that main has to be pushed before anything is tagged. kanbanr can't see whether CI passed, so check
that yourself before the cut:

```text
kanbanr config cadence --release-stages candidate,main,production
kanbanr config release-stage candidate --on-enter branch,prepare
kanbanr config release-stage main --requires items_finished --on-enter move_main
kanbanr config release-stage production --requires main_at_release,pushed --on-enter tag
kanbanr config release-prepare --version-file package.json --changelog CHANGELOG.md --notes "docs/releases/{version}.md"
```

```text
STAGE          REQUIRES                           ON ENTER
candidate      —                                  branch, prepare
main           items_finished                     move_main
production     main_at_release, pushed            tag                            the cut
```

The `release-prepare` line names your own files. The version file must already be on the sprint's
branch, or promoting to candidate is refused.

## Planning a sprint

```text
kanbanr milestone add --name "Carts"
kanbanr sprint add --start 2026-11-03 --length 2w --goal "Carts survive a return"
kanbanr sprint start SP-001
kanbanr feature add --title "Restore the cart" --milestone MS-001 --spec "# Restore the cart"
kanbanr sprint plan SP-001 FEAT-001
kanbanr release add v1.0.0
kanbanr release plan v1.0.0 FEAT-001
```

```text
created milestone MS-001
added SP-001 (2026-11-03 → 2026-11-16)
SP-001 is the active sprint; created sprint/SP-001 from main
created FEAT-001 (Backlog)
planned FEAT-001 into SP-001 (0 committed)
added release v1.0.0
planned FEAT-001 into v1.0.0
```

Starting the sprint makes the `sprint/SP-001` branch from main without switching your checkout.

## One item, stage by stage

Each move is refused until its gate is met, and the refusal says what's missing. `kanbanr check`
shows the same list before you try.

| Stage | What must be true to enter | Commands |
|---|---|---|
| **Backlog** | Nothing: a title is enough | `kanbanr feature add` |
| **Ready** | Definition of Ready: statement, goal, requirements in EARS form, a size in points | `kanbanr feature define`, `kanbanr feature edit --points` |
| **In Progress** | Approved, in the current sprint, the `design-review` sign-off | `kanbanr approve`, `kanbanr signoff`, `kanbanr start` |
| **Review** | A test named for every requirement | `kanbanr test` |
| **Testing** | Nothing more | `kanbanr move` |
| **Done** | Definition of Done: every requirement proven by a green test, the `docs-folded` sign-off | `kanbanr test`, `kanbanr signoff`, `kanbanr move` |
| **Released** | In a release that has been cut | the cut does the move |

An item with no definition or points can't reach Ready:

```text
error: unsupported operation: cannot move FEAT-001 to 'Ready': no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`); not estimated — this project estimates in story points. To proceed anyway, record why with --override "<reason>"
```

Define it, give it points, and move it:

```text
kanbanr feature define FEAT-001 --file definition.yaml
kanbanr feature edit FEAT-001 --spec-file spec.md
kanbanr feature edit FEAT-001 --points 3
kanbanr move FEAT-001 "Ready"
```

```text
defined FEAT-001: 1 requirement(s)
updated FEAT-001
updated FEAT-001
FEAT-001 -> Ready
```

**Code starts at In Progress.** `kanbanr start` makes the item's branch from the sprint's branch and
moves the item if its gate is met. If it isn't, it says what's missing and makes no branch:

```text
error: unsupported operation: cannot move FEAT-001 to 'In Progress': not approved; no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`. To proceed anyway, record why with --override "<reason>"
```

Approve, sign off, and run it again:

```text
kanbanr approve FEAT-001
kanbanr signoff FEAT-001 design-review
kanbanr start FEAT-001 --to "In Progress"
```

```text
FEAT-001 approved by Ada
FEAT-001: sign-off 'design-review' recorded by Ada
created feat/FEAT-001-restore-the-cart from sprint/SP-001
FEAT-001 -> In Progress
```

**An edit lapses your approval.** When the definition changes, the next move says so, and you approve
the larger definition.

Commit with `kanbanr commit`, which adds `Refs: kanbanr:FEAT-001` to the commit:

```text
kanbanr commit -m "feat: restore the cart"
```

```text
[feat/FEAT-001-restore-the-cart 74aa8cc] feat: restore the cart
 1 file changed, 1 insertion(+)
 create mode 100644 cart.txt
```

Review and Testing need nothing beyond the named test:

```text
kanbanr move FEAT-001 "Review"
kanbanr move FEAT-001 "Testing"
```

```text
FEAT-001 -> Review
FEAT-001 -> Testing
```

**Done is where the merge happens.** It is refused until the tests are green and the docs are signed
off:

```text
error: unsupported operation: cannot move FEAT-001 to 'Done': R-1 has tests but none is green — not yet proven; no sign-off 'docs-folded' — `kanbanr signoff FEAT-001 docs-folded`. To proceed anyway, record why with --override "<reason>"
```

Record the result and the sign-off, then move:

```text
kanbanr test FEAT-001 R-1 "cart_restores_after_return" green --rev 74aa8cc83c09aa4eef7b0d856da1efbfa605609c
kanbanr signoff FEAT-001 docs-folded
kanbanr move FEAT-001 "Done"
```

```text
FEAT-001/R-1 · cart_restores_after_return -> green
FEAT-001: sign-off 'docs-folded' recorded by Ada
FEAT-001 -> Done
merged feat/FEAT-001-restore-the-cart into sprint/SP-001 (fe92a82b0741)
```

The item now counts as done for the sprint. `kanbanr check FEAT-001` says it is ready for Released,
which only the cut does. Your checkout stays on the item's branch, so switch back when you want the
line.

## Handing work to sub-agents

A scope limits what an agent working in its own worktree may do, and kanbanr enforces it. The agent
may record its own item's tasks and tests and commit on its item's branch. It can't move items,
approve or push. See [Sub-agents and agent scopes](../using/with-claude.md#sub-agents-and-agent-scopes).

Stay in the project folder. Never `cd` into an agent's worktree: look inside with `git -C <path>`,
`make -C <path>`, `npm --prefix <path>` or a subshell `(cd <path> && …)`. If your shell is already
there, `cd` back to the project folder. Revoking a scope (`kanbanr scope revoke SC-…`) stops the
agent on its next tool call and never restricts you.

## Releasing

`kanbanr release promote v1.0.0 --plan` shows what a promotion would do and changes nothing:

```text
v1.0.0: Promote to candidate (— → candidate)
  does:
    → branch (candidate): makes release/v1.0.0 from sprint/SP-001
    → prepare (candidate): commits the version 1.0.0, the changelog heading and draft notes to release/v1.0.0
  ready: `kanbanr release promote v1.0.0 candidate`
  after: then Promote to main
  (a plan: nothing was changed)
```

| Step | Command | What happens |
|---|---|---|
| 1. Candidate | `kanbanr release promote v1.0.0 candidate` | Makes `release/v1.0.0` from the sprint's branch, and commits the version, the changelog heading and draft notes |
| 2. Write the notes | `kanbanr release notes v1.0.0 --edit` | The draft only lists requirements. Rewrite it for readers |
| 3. Run your full checks | Your project's own | Over the release |
| 4. Main | `kanbanr release promote v1.0.0 main` | Fast-forwards main to the release, without tagging |
| 5. Push main, wait for CI | `git push origin main` | |
| 6. Cut | `kanbanr release promote v1.0.0 production` | Tags main, ships the items and moves them to Released |
| 7. Push the tag | `git push origin v1.0.0` | |
| 8. Merge forward | `kanbanr git forward v1.0.0` | Brings the release's commits into the sprint's branch |

Scrum asks for no release sign-off, so there is nothing to give between Done and the promotion.

```text
v1.0.0 -> candidate; created release/v1.0.0
  makes release/v1.0.0 from sprint/SP-001
  commits the version 1.0.0, the changelog heading and draft notes to release/v1.0.0
  prepared 59e2014e8aae on release/v1.0.0
  next: `kanbanr release promote v1.0.0` (Promote to main)
```

The notes live on the release branch, so read them from the release itself:

```text
kanbanr release notes v1.0.0
```

```text
release/v1.0.0 @ 59e2014: docs/releases/v1.0.0.md

# v1.0.0

_Released 2026-11-04._

## What shipped

### FEAT-001 — Restore the cart

A returning shopper finds their cart for shoppers so that they don't lose it

- WHEN a shopper returns, THE SYSTEM SHALL restore their cart
```

Promote to main, then push it:

```text
v1.0.0 -> main; main fast-forwarded to it, not tagged
  fast-forwards main to release/v1.0.0 (59e2014)
  merge these forward into sprint/SP-001 (`kanbanr git forward v1.0.0` with the git model on, or by hand):
    59e2014e8aae Release v1.0.0: version, changelog and notes
  nothing was pushed: push main when you are ready
  next: push main (`git push origin main`), then wait for CI, then Cut v1.0.0
```

```text
To <tmp>/origin.git
   c699fc1..59e2014  main -> main
```

The cut, then the tag push:

```text
v1.0.0 -> production; tagged v1.0.0, where main already is
  tags 59e2014 "Release v1.0.0" as v1.0.0
  shipped 1 item(s); notes in releases/v1.0.0.md
  merge these forward into sprint/SP-001 (`kanbanr git forward v1.0.0` with the git model on, or by hand):
    59e2014e8aae Release v1.0.0: version, changelog and notes
  nothing was pushed: push the tag when you are ready
  push the tag: `git push origin v1.0.0`
```

```text
To <tmp>/origin.git
 * [new tag]         v1.0.0 -> v1.0.0
```

The item is now Released, and the release is shipped:

```text
- **Status:** Released
v1.0.0       shipped  production   target —           1 item(s)
```

Merge forward so the sprint's branch has the release commit:

```text
merged release/v1.0.0 forward into sprint/SP-001 (a76f622b0e74)
```

**Freeze the candidate.** Once a release is a candidate, send anything new to the next release, and
change only its notes.

**The date follows the release.** The notes and the changelog heading are dated when the candidate is
made, and promoting to main dates them today. If main moved earlier and the cut comes later, the
cut's `--plan` warns, and `kanbanr release redate v1.0.0` dates them today. Notes can't be edited
after the cut.

## Closing the sprint

```text
kanbanr sprint close SP-001
kanbanr retro --sprint SP-001
```

```text
closed SP-001; nothing carried over
  look back on it: `kanbanr retro --sprint SP-001`
```

Items at Done count as done for the sprint, so they are not carried over, even if their release is
not cut yet. Closing changes the board only. With no sprint running, there's no line for new items to
branch from, so start the next sprint before starting any item.

The retro opens like this:

```text
# Retrospective — sprint SP-001

## What the board recorded

- items: 1 (1 finished, 0 still open)
- sprint SP-001: 3 of 3 points done
- ran: 2026-11-04 → 2026-11-04
- scope: 1 to begin with, 0 added (0 defect(s), 0 split, 0 unaccounted for)
- defects recorded with their cause: 0 (0 escaped)
- cycle time: p50 0m, p90 0m, max 0m
- requirements proven by a green test: 1/1
```

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| Moves succeed that shouldn't | No charter, or the item predates it | Set the charter before creating items |
| `no definition` or `not estimated` at Ready | The item lacks a definition or points | `kanbanr feature define`, then `kanbanr feature edit --points` |
| `not approved` at In Progress | The definition isn't approved | `kanbanr approve FEAT-001` |
| The move says the approval lapsed | The definition changed after you approved it | Read the change and approve again |
| `none is green` at Done | A requirement has no passing test | `kanbanr test` with the result |
| `no sign-off 'docs-folded'` | The docs sign-off is missing | `kanbanr signoff FEAT-001 docs-folded` |
| A defect in a shipped release | It needs a fix on its own | See [Patch releases](../using/processes.md#patch-releases) |

A move can also go past its gate with `--override "<reason>"`, and the reason stays on the item. Save
it for real exceptions.
