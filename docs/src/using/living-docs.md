# Living architecture docs

## Living architecture docs

A design written into an item's spec explains one change. Six months later, nobody reads a
hundred specs to learn how the system works. They read the architecture docs, and those are only
worth reading if every change that touched them updated them. kanbanr makes that a checked step.

### The architecture repository

Keep it in the board's docs (`kanbanr doc add …`), beside the decisions:

```text
architecture/
  overview.md          # the views: context, containers, how data flows
  components/
    store.md           # one doc per component, describing it as built
    monitor.md
  zachman.md           # which of the six columns the docs cover
decisions/             # ADRs (kanbanr adr new …)
```

A component doc's front matter says what it covers, so a stale one can be found:

```yaml
---
items: [FEAT-012, FEAT-046]   # the work that shaped it
adrs: [ADR-0005]
covers: [What, How, Where]
---
```

### Naming the docs a change affects

At the design stage, the spec's Design ends with **Affects docs**: links to each doc the change
touches ([Tying code to the reason for it](traceability.md) has the link forms). The same list
goes into the definition, where it can be checked:

```yaml
affects_docs: [architecture/components/store.md, architecture/overview.md]
```

Paths are board doc paths; `.md` is added when missing, and a path outside the doc tree is
refused. Naming the docs does not reopen the item's approval: it is part of the design, written
after the definition was agreed. `kanbanr feature show` and the item's page in the monitor list
them as links, and `kanbanr doctor` reports one the board does not have.

### Checking they were updated

A gate can ask for it:

```yaml
gates:
  Completed:
    requires: [tests_green, docs_updated]
```

`docs_updated` passes when every doc in `affects_docs` has changed since the item entered its
current stage. "Changed" is read from the board's git history: the doc now, against the doc in the
last board commit before the item entered the stage. A change written but not yet committed
counts. An item that names no docs passes, since not every change touches the architecture;
`kanbanr doctor` points it out when the next stage asks for `docs_updated`, so a missing list is
seen rather than waved through.

### Before a release, and at each milestone

Before an item is released, fold its Design into the component docs and views it named; the gate
makes sure that happened. At the end of each milestone, review the architecture as a whole: every
component has a doc that describes it as built, every doc's `items` and `adrs` are current, and the
Zachman coverage has no column nothing answers (`kanbanr trace MS-… --zachman`).
