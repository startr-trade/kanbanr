# PDCA from the command line

This guide takes one item through Plan, Do, Check and Act with you running each command. To work by
talking to Claude instead, see [PDCA with Claude](pdca-with-claude.md).

The commands and output come from a real run on a scratch project. `<tmp>` stands for a temporary
folder.

PDCA uses no sprints or releases by default. There is no cut: an item is done when it reaches Act.

## Setting it up

In your project folder, which is a git repository:

```text
kanbanr init shop --data-dir <tmp>/shop-board --author Ada --email ada@example.com
kanbanr config workflow --preset pdca
kanbanr config cadence --git-model on
kanbanr git install-hooks
```

```text
workflow updated
sprints off, releases off, estimates in days, git model off
installed commit-msg and pre-commit and pre-push in <tmp>/shop: a commit now has to say which item it serves
```

`kanbanr git model` shows what the git model will do. With no sprints, work merges straight into
`main`. Nothing is pushed for you:

```text
topology:   trunk
main:       main
line:       main
branch at:  main
production: at the integrate merge
items:      feat/{code}-{slug}, fix/{code}-{slug}, spike/*
protected:  main, sprint/*, release/*
kanbanr merges:
  - each item branch into main when the item enters the integrate stage (--no-ff, naming the item)
pushing:    never automatic — you review and push
automation: on — start, sprints and the hooks follow this model
```

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

## The stages

| Stage | What must be true to enter | Commands |
|---|---|---|
| **Plan** | Nothing | `kanbanr feature add` |
| **Do** | A definition with statement, goals, requirements and a test named for each; approval; a `## Design` section in the spec; the `design-review` sign-off | `kanbanr feature define`, `kanbanr feature edit`, `kanbanr signoff`, `kanbanr approve`, `kanbanr start` |
| **Check** | Every requirement proven by a green test; the `docs-folded` sign-off | `kanbanr test`, `kanbanr signoff`, `kanbanr move` |
| **Act** | The `review` sign-off | `kanbanr signoff`, `kanbanr move` |

## Plan

```text
kanbanr milestone add --name "Carts"
kanbanr feature add --title "Restore the cart" --milestone MS-001 --spec "# Restore the cart"
```

```text
created MS-001
created FEAT-001 (Plan)
```

## Do

`kanbanr check` says what the next stage still needs:

```text
kanbanr check FEAT-001
```

```text
✗ FEAT-001 Restore the cart
    no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`)
    → to move to Do (Make the change as planned.), still needed:
        no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`)
        the spec has no 'Design' section — add it as a heading, with its content
        no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`

0/1 ready
```

`kanbanr start` makes the item's branch and moves the item if its gate is met. If it isn't, it
says what's missing and makes no branch:

```text
kanbanr start FEAT-001 --to "Do"
```

```text
error: unsupported operation: cannot move FEAT-001 to 'Do': no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`); the spec has no 'Design' section — add it as a heading, with its content; no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`. To proceed anyway, record why with --override "<reason>"
```

Write the definition, add the design to the spec and record the design review:

```text
kanbanr feature define FEAT-001 --file definition.yaml
kanbanr feature edit FEAT-001 --spec-file spec.md
kanbanr signoff FEAT-001 design-review
```

```text
defined FEAT-001: 1 requirement(s)
updated FEAT-001
FEAT-001: sign-off 'design-review' recorded by Ada
```

Starting again is still refused until you approve:

```text
created feat/FEAT-001-restore-the-cart from main
error: unsupported operation: cannot move FEAT-001 to 'Do': not approved. To proceed anyway, record why with --override "<reason>"
```

```text
kanbanr approve FEAT-001
kanbanr start FEAT-001 --to "Do"
```

```text
FEAT-001 approved by Ada
created feat/FEAT-001-restore-the-cart from main
FEAT-001 -> Do
```

**Adding keeps your approval; changing lapses it.** Text added to the definition is reviewed at the
next decision. If you change or remove approved text, approve it again.

Do the work, then commit with `kanbanr commit`, which adds `Refs: kanbanr:FEAT-001` to the commit:

```text
kanbanr commit -m "feat: restore the cart"
```

```text
[feat/FEAT-001-restore-the-cart 9a5061a] feat: restore the cart
 1 file changed, 1 insertion(+)
 create mode 100644 cart.txt
```

## Check

The move is refused until the requirements are proven and the docs are folded in:

```text
kanbanr move FEAT-001 "Check"
```

```text
error: unsupported operation: cannot move FEAT-001 to 'Check': R-1 has tests but none is green — not yet proven; no sign-off 'docs-folded' — `kanbanr signoff FEAT-001 docs-folded`. To proceed anyway, record why with --override "<reason>"
```

Record the test result against the commit it ran on, give the sign-off and move:

```text
kanbanr test FEAT-001 R-1 "cart_restores_after_return" green --rev 9a5061a96d32f217bcdad0c4013554298b7607c5
kanbanr signoff FEAT-001 docs-folded
kanbanr move FEAT-001 "Check"
```

```text
FEAT-001/R-1 · cart_restores_after_return -> green
FEAT-001: sign-off 'docs-folded' recorded by Ada
FEAT-001 -> Check
merged feat/FEAT-001-restore-the-cart into main (52dd737ee47d)
```

**Check is where the merge happens.** The item's branch is merged into `main`. Your checkout stays on
the item's branch.

## Act

```text
kanbanr move FEAT-001 "Act"
```

```text
error: unsupported operation: cannot move FEAT-001 to 'Act': no sign-off 'review' — `kanbanr signoff FEAT-001 review`. To proceed anyway, record why with --override "<reason>"
```

```text
kanbanr signoff FEAT-001 review
kanbanr move FEAT-001 "Act"
```

```text
FEAT-001: sign-off 'review' recorded by Ada
FEAT-001 -> Act
```

Act is the end of the cycle:

```text
kanbanr board
```

```text
# shop board

## Plan (0)

## Do (0)

## Check (0)

## Act (1)
  - FEAT-001 Restore the cart [0/0]
```

The history shows the merge on `main`:

```text
*   52dd737 Merge feat/FEAT-001-restore-the-cart into main
|\  
| * 9a5061a feat: restore the cart
|/  
* 1f0a61c select the board
* 403525e first commit
```

Push `main` yourself when you are ready: `git push origin main`. kanbanr never pushes.

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| Moves succeed that shouldn't | No charter, or the item predates it | Set the charter before creating items |
| `not approved` | The definition was never approved, or changed since | `kanbanr approve FEAT-001` |
| `the spec has no 'Design' section` | Do needs a build-ready design | `kanbanr feature edit`, then `kanbanr signoff FEAT-001 design-review` |
| `none is green — not yet proven` | A requirement has no passing test | Run the test, then `kanbanr test` |
| `no sign-off 'review'` | Act needs a person's review | `kanbanr signoff FEAT-001 review` |
| `doctor` says the branch is merged | The item is finished | `git branch -d feat/FEAT-001-restore-the-cart` |

A move can also go past its gate with `--override "<reason>"`, and the reason stays on the item. Save
it for real exceptions.
