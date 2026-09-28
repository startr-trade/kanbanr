# Tying code to the reason for it

## Tying code to the reason for it

```bash
kanbanr start FEAT-001                       # branches feat/FEAT-001-<slug>, moves the item
kanbanr commit -m "feat(x): …" --ref R-2     # fills in Refs: kanbanr:FEAT-001/R-2
kanbanr finish                               # refuses while tasks are open or requirements unproven
kanbanr git install-hooks                    # commit-msg + pre-commit checks in your repo
kanbanr trace G-2 | FEAT-001 | FEAT-001/R-2  # down the chain, ending in the gaps
kanbanr trace MS-006 --zachman               # which of the six columns nothing addresses
kanbanr why src/cart.rs:42                   # up: annotation or trailer → requirement → goal
kanbanr adr new "…" --affects FEAT-001 --driven-by FEAT-001/R-2 --quality Reliability
kanbanr adr list [--for FEAT-001] | adr supersede ADR-0007 --replaces ADR-0003 | adr history ADR-0007
```

One item, one branch, and every commit says what it serves. The hooks refuse a commit on the
default branch, a branch that names no item, and a message with no reference — and they let through
merges, reverts, `spike/*` branches, and a recorded escape (`[no-ref] <why>` in the message, which
leaves the reason in git history forever). If kanbanr is not on PATH the hooks step aside rather
than making the repository uncommittable for someone who never installed it.

Architecture decisions stay **documents** with front-matter that joins them to the graph: they have
no estimate, branch or tests, so counting them as work items would distort the flow metrics.
Deciding is still work — it is a task on the item that needed the decision, and the ADR is its
output.
