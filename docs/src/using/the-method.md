# The method: why work exists

## The method: why work exists, and what proves it done

Everything above tracks *what* is being built. This section is about *why* — the part a board
normally loses. It is opt-in: a project with no charter behaves exactly as it always did, and items
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
