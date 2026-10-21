# Tying code to the reason for it

## Tying code to the reason for it

```bash
kanbanr start FEAT-001                       # branches feat/FEAT-001-<slug>, moves the item
kanbanr commit -m "feat(x): …" --ref R-2     # fills in Refs: kanbanr:FEAT-001/R-2
kanbanr finish                               # refuses while tasks are open or requirements unproven
kanbanr git install-hooks                    # commit-msg + pre-commit checks in your repo
kanbanr trace G-2 | FEAT-001 | FEAT-001/R-2  # down the chain, ending in the gaps
kanbanr trace architecture/data.md          # a doc: what it names, and what links to it
kanbanr trace MS-006 --zachman               # which of the six columns nothing addresses
kanbanr why src/cart.rs:42                   # up: annotation or trailer → requirement → goal
kanbanr adr new "…" --affects FEAT-001 --driven-by FEAT-001/R-2 --quality Reliability
kanbanr adr list [--for FEAT-001] | adr supersede ADR-0007 --replaces ADR-0003 | adr history ADR-0007
kanbanr adr accept ADR-0007 | adr reject ADR-0007 --reason "…"   # the user's verdict
```

One item, one branch, and every commit says what it serves. The hooks refuse a commit on the
default branch, a branch that names no item, and a message with no reference — and they let through
merges, reverts, `spike/*` branches, and a recorded escape (`[no-ref] <why>` in the message, which
leaves the reason in git history forever). If kanbanr is not on PATH the hooks step aside rather
than making the repository uncommittable for someone who never installed it.

### Lines of integration

With the git model on ([Processes](processes.md#the-git-model)), the hooks also guard the
branches work integrates into: main, the integration branch, `sprint/*` and `release/*`. A plain
commit there is refused — work belongs on an item branch — and a merge into one must carry a
`Refs:` trailer naming what it brings, because a line is where "which items reached it" is read
from. A defect's `fix/<CODE>-…` branch names its item as `feat/` does. The merges kanbanr itself makes
(integrating an item, catching up a carried one, forwarding a release) carry those trailers, so a
line's history says which items reached it.

### Drift between board and branches

`kanbanr git doctor` compares the board with the code repository's branches, in the git model's
terms, and changes nothing:

| Finding | |
|---|---|
| an integrated item whose branch is not merged into its line | error |
| commits on a line serving only items that are not integrated | error |
| a commit naming an item the board does not have | error |
| a branch naming no item, or an item, sprint or release the board does not have | warning |
| a release branch with fixes its line lacks (`kanbanr git forward <VERSION>`) | warning |
| a finished item's merged branch (delete it), or a parked item's branch | warning |

It exits 1 when it finds an error. With the git model on, `kanbanr doctor` includes the same
findings when run from the project folder. In the trunk topology, main's last 200 commits are read.

### Links between specs, docs and decisions

A spec or a doc that names `FEAT-012`, `FEAT-012/R-2`, `MS-004`, `ADR-0005`, `G-1` or a doc path
(`architecture/data.md`) is linked to it. The monitor turns each into a link to its page, and
`kanbanr trace` follows them both ways: an item lists the docs that name it and what its own spec
links to; a doc can be traced on its own. `kanbanr why` adds **Documented in** to its answer.

Write references as links where you mean them to be followed:

| Reference | Link |
|---|---|
| an item | `[FEAT-012](/p/<project>/feature/FEAT-012)`, a requirement `…/feature/FEAT-012#R-2` |
| a milestone | `/p/<project>/milestone/MS-004` |
| a goal | `/p/<project>/charter#G-1` |
| an ADR or a doc | `/p/<project>/docs/file/<path>` |

A code is a reference only when its prefix is one this board uses, so `UTF-8` and `ISO-9001` are
left alone; code spans, code blocks and HTML comments are never read. `kanbanr doctor` reports a
reference to something the board does not have — an **error** for a link, which is meant to be
followed, and a **warning** for a code named in prose. The monitor strikes the same ones through.

Architecture decisions stay **documents** with front-matter that joins them to the graph: they have
no estimate, branch or tests, so counting them as work items would distort the flow metrics.
Deciding is still work — it is a task on the item that needed the decision, and the ADR is its
output.

A decision starts **proposed** and is accepted or rejected by its owner — never by Claude — on the
Review page (beside the definitions waiting for approval) or with `kanbanr adr accept` /
`adr reject --reason`. Accepting is refused while a section is unanswered; a rejection keeps its
reason. The verdict, the decider and the date are written on the decision, and the verdict appears
in the project's activity like every other change.
