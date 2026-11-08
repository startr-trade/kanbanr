# Design control from the command line

This guide takes one item through the `design-control` process, with you running each command. To
work by talking to Claude instead, see [Design control with Claude](design-control-with-claude.md).

The commands and output come from a real run on a scratch project. `<tmp>` stands for a temporary
folder. Sprints and releases are off in this process by default, so there is no release to cut: the
item is finished when it reaches **Released**.

## Setting it up

In your project folder:

```text
kanbanr init shop
kanbanr config workflow --preset design-control
kanbanr config cadence --git-model on
kanbanr git install-hooks
```

```text
workflow updated
sprints off, releases off, estimates in days
installed commit-msg and pre-commit and pre-push in <tmp>/shop: a commit now has to say which item it serves
```

`kanbanr git model` shows what the git model will do. Nothing is pushed for you:

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

Planning → Inputs → Design → Review → Verification → Validation → Released. Each move is refused
until its gate is met, and the refusal says what's missing. `kanbanr check FEAT-001` lists it first.

| Stage | What must be true to enter | Commands |
|---|---|---|
| **Planning** | Nothing | `kanbanr feature add` |
| **Inputs** | A definition: statement, goal, requirements in EARS form | `kanbanr feature define` |
| **Design** | Approval, and the `design-review` sign-off | `kanbanr approve`, `kanbanr signoff`, `kanbanr start` |
| **Review** | A test named for every requirement | `kanbanr move` |
| **Verification** | The `verification-review` sign-off | `kanbanr signoff` |
| **Validation** | Green tests, and the `docs-folded` sign-off | `kanbanr test`, `kanbanr signoff` |
| **Released** | Green tests, and the `validation` sign-off | `kanbanr signoff` |

## One item, stage by stage

```text
kanbanr milestone add --name "Carts"
kanbanr feature add --title "Restore the cart" --milestone MS-001 --spec "# Restore the cart"
```

```text
created milestone MS-001
created FEAT-001 (Planning)
```

### Planning to Inputs

An item with no definition can't leave Planning:

```text
kanbanr move FEAT-001 "Inputs"
```

```text
error: unsupported operation: cannot move FEAT-001 to 'Inputs': no definition — why it exists, what must be true, how it is verified (`kanbanr feature define`). To proceed anyway, record why with --override "<reason>"
```

Define it from a file, and give the spec a `## Design` section, then move:

```text
kanbanr feature define FEAT-001 --file definition.yaml
kanbanr feature edit FEAT-001 --spec-file spec.md
kanbanr move FEAT-001 "Inputs"
```

```text
defined FEAT-001: 1 requirement(s)
updated FEAT-001
FEAT-001 -> Inputs
```

### Inputs to Design

`kanbanr check` says what Design needs:

```text
✗ FEAT-001 Restore the cart
    not approved
    R-1 has tests but none is green — not yet proven
    → to move to Design (Produce the design outputs against those inputs.), still needed:
        not approved
        no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`
```

**Code starts at Design.** `kanbanr start` makes the item's branch and moves the item if its gate is
met. If it isn't, it says what's missing and makes no branch:

```text
error: unsupported operation: cannot move FEAT-001 to 'Design': not approved; no sign-off 'design-review' — `kanbanr signoff FEAT-001 design-review`. To proceed anyway, record why with --override "<reason>"
```

Approve, sign off, and run it again:

```text
kanbanr approve FEAT-001
kanbanr signoff FEAT-001 design-review
kanbanr start FEAT-001 --to "Design"
```

```text
FEAT-001 approved by Ada
FEAT-001: sign-off 'design-review' recorded by Ada
created feat/FEAT-001-restore-the-cart from main
FEAT-001 -> Design
```

Do the work on the branch. Commit with `kanbanr commit`, which adds `Refs: kanbanr:FEAT-001` to the
commit:

```text
kanbanr commit -m "feat: restore the cart"
```

```text
[feat/FEAT-001-restore-the-cart 88f968e] feat: restore the cart
 1 file changed, 1 insertion(+)
 create mode 100644 cart.txt
```

### Design to Review

```text
✗ FEAT-001 Restore the cart
    R-1 has tests but none is green — not yet proven
    → Review (Review the design against its inputs.): ready to move on
```

```text
kanbanr move FEAT-001 "Review"
```

```text
FEAT-001 -> Review
```

### Review to Verification

The review is recorded as a sign-off before Verification starts. Without it, the move is refused:

```text
error: unsupported operation: cannot move FEAT-001 to 'Verification': no sign-off 'verification-review' — `kanbanr signoff FEAT-001 verification-review`. To proceed anyway, record why with --override "<reason>"
```

```text
kanbanr signoff FEAT-001 verification-review
kanbanr move FEAT-001 "Verification"
```

```text
FEAT-001: sign-off 'verification-review' recorded by Ada
FEAT-001 -> Verification
```

### Verification to Validation

**Validation is where the merge happens.** It needs a green test and the
`docs-folded` sign-off. Without them:

```text
error: unsupported operation: cannot move FEAT-001 to 'Validation': R-1 has tests but none is green — not yet proven; no sign-off 'docs-folded' — `kanbanr signoff FEAT-001 docs-folded`. To proceed anyway, record why with --override "<reason>"
```

Record the test result against the commit it ran on, give the sign-off, and move:

```text
kanbanr test FEAT-001 R-1 "cart_restores_after_return" green --rev 88f968e792aede0f44a1b9a82c5bd0dd821f308e
kanbanr signoff FEAT-001 docs-folded
kanbanr move FEAT-001 "Validation"
```

```text
FEAT-001/R-1 · cart_restores_after_return -> green
FEAT-001: sign-off 'docs-folded' recorded by Ada
FEAT-001 -> Validation
merged feat/FEAT-001-restore-the-cart into main (9837a0e70aa0)
```

Your checkout stays on the item's branch, so switch back when you want `main`.

### Validation to Released

The item is ready, but Released needs the `validation` sign-off:

```text
✓ FEAT-001 Restore the cart
    → to move to Released (Released, with validation recorded.), still needed:
        no sign-off 'validation' — `kanbanr signoff FEAT-001 validation`
```

```text
error: unsupported operation: cannot move FEAT-001 to 'Released': no sign-off 'validation' — `kanbanr signoff FEAT-001 validation`. To proceed anyway, record why with --override "<reason>"
```

```text
kanbanr signoff FEAT-001 validation
kanbanr move FEAT-001 "Released"
```

```text
FEAT-001: sign-off 'validation' recorded by Ada
FEAT-001 -> Released
```

`kanbanr board` now shows the item under Released. That is the end: there is no release to promote
and no tag to cut.

```text

### Chores

A process can give a kind its own path. The built-in `design-control` process gives `chore` the path
Planning → Design → Verification → Released, skipping Inputs, Review and Validation. `kanbanr process show design-control` lists each kind's path under
"Lighter paths".

```bash
kanbanr feature add --title "Pin the CI image" --milestone MS-001 --kind chore
```

A chore's first move off Planning asks the path's own conditions (`statement`, `goals`,
`requirements`, and `ears` where the path lists it) as well as the entered stage's gate, so it still
records why it exists and how it is verified. A move to a stage off the path is refused, and the
message names the path. The entered stage's gate still applies, `approved` and sign-offs included; the sign-offs at Review and Validation belong to stages a chore does not enter. The `design` and `design-review` parts do not apply. An item whose kind is changed to `chore` while it's at a
stage off the path keeps the ordinary transitions, so it is never stranded.

## Released (1)
  - FEAT-001 Restore the cart [0/0]
```

The history shows the merge made at Validation:

```text
*   9837a0e Merge feat/FEAT-001-restore-the-cart into main
|\  
| * 88f968e feat: restore the cart
|/  
* 0df855e select the board
* 403525e first commit
```

`kanbanr doctor` then reminds you that the merged branch can go:

```text
shop FEAT-001: FEAT-001 is finished and feat/FEAT-001-restore-the-cart is merged: delete it with `git branch -d feat/FEAT-001-restore-the-cart`
```

Nothing was pushed. Push `main` yourself when you've reviewed it.

## Handing work to sub-agents

A scope limits what an agent working in its own worktree may do, and kanbanr enforces it. An agent
can record its own tasks and tests and commit on its item's branch. It can't move items, approve,
give sign-offs or push. See
[Sub-agents and agent scopes](../using/with-claude.md#sub-agents-and-agent-scopes).

Stay in the project folder. Never `cd` into an agent's worktree: look inside with `git -C <path>`,
`make -C <path>`, `npm --prefix <path>` or a subshell `(cd <path> && …)`. If your shell is already
there, `cd` back to the project folder. Revoking a scope (`kanbanr scope revoke SC-…`) stops the
agent on its next tool call and never restricts you.

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| Moves succeed that shouldn't | No charter, or the item predates it | Set the charter before creating items |
| `no sign-off '...'` | The stage asks for a sign-off you haven't given | `kanbanr signoff FEAT-001 <name>` |
| `R-1 has tests but none is green` | A requirement isn't proven yet | Run the test and record it with `kanbanr test` |
| `no definition` | The item has no definition yet | `kanbanr feature define` |

A move can also go past its gate with `--override "<reason>"`, and the reason stays on the item. Save
it for real exceptions.
