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

### Coverage rules: keeping the views whole

`docs_updated` checks the docs an item names. The views that span every item — a Zachman matrix,
an ADR index, the component catalog — are rarely named, so they fall behind unnoticed: every
item's own docs current, and the matrix missing two dozen of them. A **coverage rule** says what a
view must reference, and kanbanr checks it whenever the board changes.

Rules live in the project's config, because they name this project's docs. Write them in a file
and set them:

```yaml
coverage:
  - id: zachman                       # lower-case letters, digits, hyphens; unique
    description: the Zachman matrix lists every designed item
    every: item                       # item (the default) | adr | doc
    from: "@design"                   # this stage or later: a status, or "@<role>"
    # in: [Implementation, Review]    # or exactly these stages
    # kinds: [feature]                # only these kinds (no kind = feature)
    referenced_from: architecture/zachman.md
  - id: adr-index
    every: adr                        # accepted ADRs; superseded and rejected ones are not
    referenced_from: architecture/    # a folder: any doc under it will do
  - id: catalog
    every: doc
    under: architecture/components/   # the docs that must be listed
    referenced_from: architecture/catalog.md
  - id: views-listed
    in: [Implementation]
    lists: architecture/zachman.md    # instead of referenced_from: the item names it in affects_docs
    except: [FEAT-001]                # grandfathered out
```

```bash
kanbanr config coverage --from-file coverage.yaml   # set (refused, naming the rule, if invalid)
kanbanr config coverage                             # each rule: uncovered/subjects, and its source
kanbanr config coverage --clear                     # remove them
```

- **What counts as a reference** is what the board's link finder finds: a live link, a relative
  link (`components/store.md` from `architecture/catalog.md`), or a board-shaped token in prose
  (`FEAT-012`, `ADR-0005`). A requirement reference (`FEAT-012/R-3`) covers its item. A name in a
  code span or block does not count. A doc's own file is never its own source.
- **"S or later"** is every status at or after S in the workflow's order, no-op statuses left
  out. Naming S as `@design` makes one rule work in every process. `in:` lists exact statuses, and
  may name a no-op. With neither, every status that is not a no-op counts. An exempt item is never
  a subject.
- **When they are checked.** After adding, editing, defining or moving an item, accepting an ADR,
  writing or deleting a doc, closing a sprint, and cutting or promoting a release. The command
  prints what the rules now lack, on stderr (`coverage: [zachman] FEAT-012 is not referenced from
  architecture/zachman.md — add a link to it there`); `--json` carries them as
  `coverage_warnings`. Each such write also records a `coverage_gap` event, which an opted-in
  webhook receives. A sprint close or a release lists every uncovered rule. `kanbanr doctor` lists
  them all, at any time.
- **Nothing configured, nothing changes.** With no rules, no doc is read for coverage and no
  output changes.

**Asking for it at a gate.** `docs_cover` refuses a move while the item fails a rule, judged as if
it were already in the stage it is entering — an item moving into Design is held to the rule that
starts at Design. Narrow it to some rules with `{docs_cover: [zachman, catalog]}`; a rule id the
project does not configure is an error.

```yaml
gates:
  Completed:
    requires: [tests_green, {docs_cover: [zachman, catalog]}]
```

**Default docs per stage.** A gate can name the docs every item entering its stage must list. The
move adds them to the item's `affects_docs`, without reopening its approval, so the views are part
of every fold-in and `docs_updated` checks them:

```yaml
gates:
  "@design": { affects_docs: [architecture/zachman.md] }
```

An item with no definition is moved, with a warning that the docs were not recorded. Default docs
travel with the process; coverage rules stay with the project.

**A batch** judges its gates with the docs as they were when it started, and writes its doc
operations after its moves. Write the view doc in an earlier command, then run the batch that
moves the item.

### Before a release, and at each milestone

Before an item is released, fold its Design into the component docs and views it named; the gate
makes sure that happened. At the end of each milestone, review the architecture as a whole: every
component has a doc that describes it as built, every doc's `items` and `adrs` are current, and the
Zachman coverage has no column nothing answers (`kanbanr trace MS-… --zachman`). Run
`kanbanr doctor` before merging a sprint or cutting a release: it lists every coverage rule a
view still fails.

### Drift as work lands

A doc can name `--since` or a route that an item then changed, and stay as it was. kanbanr finds
these when the work lands, rather than when someone notices. It reads the names an item put in the
code, from the diff of the commits whose `Refs:` trailer names it: long flags, commands, routes,
`KANBANR_` variables, settings, stage, check and status names, and files. Documentation files add no
names. It then looks at the texts linked to the item: the docs in its `affects_docs`, the docs whose
front matter lists it under `items` or `refs`, the ADRs whose `affects` names it, the docs and items
that link it, and the items its spec links. A text is **flagged** when it holds one of those names as
a whole token and last changed before the commit that changed the name. A config key, check, stage
or status name counts only inside a code span, and a command only on a line that says `kanbanr`.

A flag says which text names which changed name. It never says the text is wrong, so read it and
decide:

- On the Review page, `docs-folded` lists each flagged text with the names found and their lines.
  For each, say it is still right, or ask Claude to file a correction and enter its code. The
  sign-off is accepted only once every text is settled, and it records how.
- When an item is merged, the flagged texts are listed on stderr.
- `kanbanr drift` lists them for the board, grouped by text, for the items with commits since
  `--since` (default: the latest shipped release's tag date, else 60 days back). `--json` adds when
  it was generated, so two runs can be compared.

Without the code repository the check is skipped, the Review page says so, and the sign-off is
offered as before. A skipped check is never read as "no drift".

### When docs-folded is offered

The `docs-folded` sign-off says the documents are folded into the record, so it is offered only
once there is something to confirm. An item's documents are built when all three hold:

1. every board doc it names in `affects_docs` has changed since it entered its stage;
2. every repository doc its Design lists under `Affects docs`, written as a path in backticks, has
   changed on its branch against the line it merges into (guides, reference pages, `SECURITY.md`,
   the changelog);
3. every task that writes docs is complete. A task writes docs when its text names a listed
   document, a path under `docs/`, or `CHANGELOG.md`, `SECURITY.md` or `README.md`.

An item that names no documents at all is offered the sign-off once it has tasks and all are
complete, and the Review page says the item changes no documents, so the sign-off confirms that.

Until then the Review page does not list the sign-off, the sign-off request is refused naming what
is unbuilt, and `kanbanr check` prints `Documents not built yet:` with each document and why. If
the code repository cannot be read, because none is attached or the item has no branch in it, the
sign-off is held and the reason is said. A sign-off given before this check existed stays recorded
and counted.
