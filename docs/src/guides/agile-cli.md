# Agile, sprints and releases from the command line

This guide takes a project from setup to a shipped release using the `agile` process, with you
running each command. To work by talking to Claude instead, see
[Agile with Claude](agile-with-claude.md).

The commands and output come from a real run on a scratch project. `<tmp>` stands for a temporary
folder.

## Setting it up

```text
git init -q -b main
kanbanr init shop --data-dir <tmp>/shop-board --author Ada --email ada@example.com
kanbanr config workflow --preset agile
kanbanr config cadence --git-model on
kanbanr git install-hooks
```

```text
workflow updated
sprints on, releases on, estimates in days
installed commit-msg and pre-commit and pre-push in <tmp>/shop: a commit now has to say which item it serves
```

The agile preset already turns sprints and releases on. `kanbanr git model` shows what the git model
will do. Nothing is pushed for you:

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

Then agree the charter before creating items. Without one, moves that should be refused still
succeed, but each says what the gate would have needed and that `kanbanr charter set` turns the
gates on:

```text
kanbanr charter set --file charter.yaml
```

### Release stages

We recommend three stages, so that main has to be pushed before anything is tagged. kanbanr can't
see whether CI passed. Look at your CI yourself, then record what it said with
`kanbanr gate record ci --passed --url <link to the run>`. To have the cut require that, run
`kanbanr config commit-gates ci` and add `gate_passed` to the production stage's `--requires`
([Gate results](../using/processes.md#gate-results)):

```text
kanbanr config cadence --release-stages candidate,main,production
kanbanr config release-stage candidate --requires items_merged --on-enter branch,prepare
kanbanr config release-stage main --requires items_finished --on-enter move_main
kanbanr config release-stage production --requires main_at_release,pushed --on-enter tag
kanbanr config release-prepare --version-file package.json --changelog CHANGELOG.md --notes "docs/releases/{version}.md"
```

```text
STAGE          REQUIRES                           ON ENTER
candidate      items_merged                       branch, prepare
main           items_finished                     move_main
production     main_at_release, pushed            tag                            the cut
```

To stop a release being promoted while work is still going on, have the candidate stage require
`items_merged`: every item planned into the release must be at the integrate stage or later (for
TOGAF, Migration), and the plan names each item that is not, with the status it is at. And to stop a
named promotion jumping over a stage, list it in `--after`: kanbanr then refuses to enter the stage
until the release has been promoted into the one named. For example, with a `ci` stage you record
yourself:

```text
kanbanr config cadence --release-stages candidate,main,ci,tag
kanbanr config release-stage tag --after ci
```

kanbanr records that the stage was entered, not that CI passed. To require a recorded CI result as
well, use `gate_passed`.

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
created FEAT-001 (Plan)
planned FEAT-001 into SP-001 (0 committed)
added release v1.0.0
planned FEAT-001 into v1.0.0
```

## One item, stage by stage

Each move is refused until its gate is met, and the refusal says what's missing. `kanbanr check` shows
it before you try.

| Stage | What must be true to enter | Commands |
|---|---|---|
| **Plan** | Nothing | `kanbanr feature add` |
| **Design** | Statement, goals, requirements, approval | `kanbanr feature define`, `kanbanr approve` |
| **Develop** | How and where, a test named for every requirement, the `design-review` sign-off | `kanbanr feature edit`, `kanbanr signoff`, `kanbanr start` |
| **Test** | Nothing | `kanbanr move` |
| **Review** | Requirements proven by green tests, the `docs-folded` sign-off | `kanbanr test`, `kanbanr signoff`, `kanbanr move` |
| **Released** | In a release; the cut does the move | `kanbanr release promote` |

Moving to Design with no definition:

```text
$ kanbanr move FEAT-001 "Design"
error: unsupported operation: cannot move FEAT-001 to 'Design': no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`). To proceed anyway, record why with --override "<reason>"
```

Define it, then write the spec. The move is still refused until you approve:

```text
$ kanbanr feature define FEAT-001 --file definition.yaml
defined FEAT-001: 1 requirement(s)
$ kanbanr feature edit FEAT-001 --spec-file spec.md
updated FEAT-001
$ kanbanr move FEAT-001 "Design"
error: unsupported operation: cannot move FEAT-001 to 'Design': not approved. To proceed anyway, record why with --override "<reason>"
$ kanbanr approve FEAT-001
FEAT-001 approved by Ada
$ kanbanr move FEAT-001 "Design"
FEAT-001 -> Design
```

**Code starts at Develop.** `kanbanr start` makes the item's branch from the sprint's branch. Without
the `design-review` sign-off it says what's missing and makes no branch:

```text
$ kanbanr start FEAT-001 --to "Develop"
error: unsupported operation: cannot move FEAT-001 to 'Develop': no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`. To proceed anyway, record why with --override "<reason>"
$ kanbanr signoff FEAT-001 design-review
FEAT-001: sign-off 'design-review' recorded by Ada
$ kanbanr start FEAT-001 --to "Develop"
created feat/FEAT-001-restore-the-cart from sprint/SP-001
FEAT-001 -> Develop
```

Commit with `kanbanr commit`, then move to Test:

```text
$ kanbanr commit -m "feat: restore the cart"
[feat/FEAT-001-restore-the-cart 74aa8cc] feat: restore the cart
 1 file changed, 1 insertion(+)
 create mode 100644 cart.txt
$ kanbanr move FEAT-001 "Test"
FEAT-001 -> Test
```

**Review is where the merge happens.** It needs green tests and the `docs-folded` sign-off:

```text
$ kanbanr move FEAT-001 "Review"
error: unsupported operation: cannot move FEAT-001 to 'Review': R-1 has tests but none is green — not yet proven; no sign-off 'docs-folded' — `kanbanr signoff FEAT-001 docs-folded`. To proceed anyway, record why with --override "<reason>"
$ kanbanr test FEAT-001 R-1 "cart_restores_after_return" green --rev 74aa8cc83c09aa4eef7b0d856da1efbfa605609c
FEAT-001/R-1 · cart_restores_after_return -> green
$ kanbanr signoff FEAT-001 docs-folded
FEAT-001: sign-off 'docs-folded' recorded by Ada
$ kanbanr move FEAT-001 "Review"
FEAT-001 -> Review
merged feat/FEAT-001-restore-the-cart into sprint/SP-001 (fe92a82b0741)
```

Now `kanbanr check` says the item is ready for Released:

```text
✓ FEAT-001 Restore the cart
    → Released (Shipped in a release.): ready to move on
```

Your checkout stays on the item's branch, so switch back when you want the sprint's line. A move can
also go past its gate with `--override "<reason>"`, and the reason stays on the item. Save it for real
exceptions.

## Releasing

`kanbanr release promote v1.0.0 --plan` shows what a promotion would do and changes nothing.

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
| 1. Candidate | `kanbanr release promote v1.0.0 candidate` | Makes `release/v1.0.0` from the sprint's branch, and commits the version, the changelog heading and draft notes onto it |
| 2. Read the notes | `kanbanr release notes v1.0.0` | The draft only lists requirements. Rewrite it with `--edit` |
| 3. Run your full checks | Your project's own | Over the release |
| 4. Main | `kanbanr release promote v1.0.0 main` | Fast-forwards main to the release, without tagging |
| 5. Push main, wait for CI | `git push origin main` | |
| 6. Cut | `kanbanr release promote v1.0.0 production` | Tags main, ships the items and moves them to Released |
| 7. Push the tag | `git push origin v1.0.0` | |
| 8. Merge forward | `kanbanr git forward v1.0.0` | Brings the release's commits into the sprint's branch |

The candidate:

```text
v1.0.0 -> candidate; created release/v1.0.0
  makes release/v1.0.0 from sprint/SP-001
  commits the version 1.0.0, the changelog heading and draft notes to release/v1.0.0
  prepared 59e2014e8aae on release/v1.0.0
  next: `kanbanr release promote v1.0.0` (Promote to main)
```

The draft notes:

```text
release/v1.0.0 @ 59e2014: docs/releases/v1.0.0.md

# v1.0.0

_Released 2026-11-04._

## What shipped

### FEAT-001 — Restore the cart

A returning shopper finds their cart for shoppers so that they don't lose it

- WHEN a shopper returns, THE SYSTEM SHALL restore their cart
```

Main:

```text
v1.0.0 -> main; main fast-forwarded to it, not tagged
  fast-forwards main to release/v1.0.0 (59e2014)
  merge these forward into sprint/SP-001 (`kanbanr git forward v1.0.0` with the git model on, or by hand):
    59e2014e8aae Release v1.0.0: version, changelog and notes
  nothing was pushed: push main when you are ready
  next: push main (`git push origin main`), then wait for CI, then Cut v1.0.0
```

```text
$ git push origin main
To <tmp>/origin.git
   c699fc1..59e2014  main -> main
```

The cut, then the tag push and the merge forward:

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
$ git push origin v1.0.0
To <tmp>/origin.git
 * [new tag]         v1.0.0 -> v1.0.0
$ kanbanr git forward v1.0.0
merged release/v1.0.0 forward into sprint/SP-001 (a76f622b0e74)
```

The item and the release now read:

```text
$ kanbanr release list
v1.0.0       shipped  production   target —           1 item(s)
```

**Freeze the candidate.** Once a release is a candidate, send anything new to the next release and
change only its notes. **Promoting moves main, not your checkout.** **The date follows the release:**
promoting to main dates the notes and changelog today. If main moved earlier and the cut comes
later, the cut's `--plan` warns, and `kanbanr release redate v1.0.0` dates them today.

## Feedback on a release

Feedback is a new item that names the version it was found in:

```text
kanbanr feature add --title "…" --milestone MS-001 --found-in v1.0.0
```

(I did not run this in the walkthrough.) For a fix to a shipped release, see
[Patch releases](../using/processes.md#patch-releases).

## Closing the sprint

```text
$ kanbanr sprint close SP-001
closed SP-001; nothing carried over
  look back on it: `kanbanr retro --sprint SP-001`
```

Closing changes the board only. Start the next sprint before starting any item, because new items
branch from the running sprint. After the cut, `kanbanr doctor` suggests deleting the merged item
branch with `git branch -d feat/FEAT-001-restore-the-cart`.

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| `no definition` | The item has no definition | `kanbanr feature define` |
| `not approved` | The definition isn't approved, or changed since | `kanbanr approve` |
| `no sign-off 'design-review'` on `start` | The branch was made, but the item stayed in Design | Sign off, then run `kanbanr start` again |
| `R-1 has tests but none is green` | A requirement isn't proven | `kanbanr test` with the result |
| `no sign-off 'docs-folded'` | The docs step isn't signed off | `kanbanr signoff FEAT-001 docs-folded` |
| A defect in a shipped release | It needs a fix on its own | See [Patch releases](../using/processes.md#patch-releases) |

See also the [guides index](index.md).
