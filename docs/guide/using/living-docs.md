# Living architecture docs

## Living architecture docs

A design written into an item's spec explains one change. Six months later, nobody reads a
hundred specs to learn how the system works. They read the architecture docs, and those are only
worth reading if every change that touched them updated them. kanbanr makes that a checked step.

### What a project documents: the content catalog

Notations are means. Before choosing a diagram, a project needs a list of what it must document,
and that list depends on what the project is: a library has no deployment of its own, a service
needs its configuration and observability written down. kanbanr keeps the list as the project's
**content catalog**, and what the project is as **the project's facts**.

**The facts** (`projects/<id>/facts.yaml`) answer five questions, each from a fixed vocabulary
(`kanbanr facts vocabulary`) or as `other:<text>`:

| Key | What it says | Words |
|---|---|---|
| `languages` | what it is written in | rust, go, python, typescript, javascript, java, kotlin, csharp, cpp, c, ruby, php, swift, shell, sql |
| `runtimes` | what it runs on | native, jvm, node, browser, python, dotnet, wasm, container |
| `production` | where the built thing runs | developer-machine, server, container, kubernetes, serverless, browser, mobile, embedded, library |
| `parts` | what it is made of | cli, api, web-ui, desktop-ui, mobile-app, library, worker, daemon |
| `data` | what it keeps | none, files, sql, nosql, object-store, queue, cache |

Claude asks them once at setup, recommending what the folder's manifests suggest
(`suggested_facts` in `kanbanr where --json`), and records them with `kanbanr facts set`.

**The catalog** (`projects/<id>/catalog.yaml`) is a list of rows. Each row says:

- what the content **answers**: one or more of the six questions what, how, where, when, who and
  why (the Zachman columns);
- whom it is **for**: one or more of five perspectives, each standing in for a Zachman row:

  | Perspective | Zachman row | Reads the system as |
  |---|---|---|
  | `sponsor` | Executive (scope) | why it exists, what it is for, its boundary |
  | `user` | Business management | what it does for whom, in which processes |
  | `architect` | Architect (system logic) | its structure: context, parts, data, interfaces, decisions |
  | `builder` | Engineer and Technician | its code: modules, dependencies, tests |
  | `operator` | Enterprise (operations) | where and how it runs: deployment, configuration, releases |

- its **level** (`system`, `component` or `item`) and its **notation** (prose, table, mermaid,
  bpmn, openapi and so on);
- its **source**, one of three kinds: **written** (a board doc, a board folder ending `/`, or a
  repository doc `repo:<path>`), **generated** (a view kanbanr derives, such as `kanbanr:charter`
  or `kanbanr:zachman`, or one katalogr serves, `katalogr:<op>`), or **enforced** (a test or rule
  that keeps it true);
- its **protection** against drift: `docs_updated`, `coverage:<rule>`, `drift`, `spec_sections`,
  `design-review`, `view-id`, `test:<name>`, or `none` when drift is accepted;
- when it **applies**: a `when` clause over the facts (`when: {production: [server, container]}`),
  absent when it always applies.

`kanbanr catalog init` writes the **starter catalog** kanbanr carries; every question has at least
one row that always applies:

| Row | Answers | For | Source | Applies when |
|---|---|---|---|---|
| `purpose` | why | sponsor, user | generated `kanbanr:charter` | always |
| `goals` | why, when | sponsor | generated `kanbanr:trace` | always |
| `roles` | who | sponsor, user | generated `kanbanr:charter` | always |
| `context` | what, where | sponsor, architect | written `architecture/context.md` | always |
| `behaviour` | how | user | written `architecture/behaviour.md` | always |
| `requirements` | what, how | user | generated `kanbanr:requirements` | always |
| `components` | what, how | architect | written `architecture/components.md` | always |
| `component-docs` | what, how, where | builder | written folder `architecture/components/` | always |
| `data-model` | what | architect, builder | written `architecture/data.md` | the data is not only `none` |
| `interfaces` | how, where | architect, builder | written `architecture/interfaces.md` | the parts have an api or a library |
| `code-structure` | what, how | builder | generated `katalogr:structure` | always |
| `decisions` | why | architect, builder | generated `kanbanr:adr-index` | always |
| `deployment` | where, when | operator | written `architecture/deployment.md` | it runs on a server, container, kubernetes, serverless, mobile or embedded |
| `configuration` | how, where | operator | written `architecture/configuration.md` | it is not only a library |
| `observability` | when, how | operator | written `architecture/observability.md` | it runs on a server, container, kubernetes or serverless |
| `security` | who, where | architect, operator | written `architecture/security.md` | always |
| `timeline` | when | sponsor | generated `kanbanr:gantt` | always |
| `releases` | when | operator | generated `kanbanr:releases` | releases are on |
| `tests` | how | builder | generated `kanbanr:evidence` | always |
| `zachman-coverage` | all six | architect | generated `kanbanr:zachman` | always |
| `item-design` | how, where | builder | written: every item's spec, `## Design` | always |

The project can add rows, change notations and sources, and **switch a row off with a reason**:
`kanbanr catalog off deployment --why "a library has no deployment of its own"`. A row is never
silently dropped: it stays in the list, greyed, with its reason. `kanbanr catalog on <row>` lets
its `when` clause decide again, and `kanbanr catalog add <row>` copies a row the starter gained
since.

`kanbanr catalog` derives each row's state on every read and stores none of it: whether it
applies, whether its source is present, a stub or missing, which protections are in force, and
the **gaps**, the question-and-perspective pairs no applying row answers. The Catalog page in the
monitor shows the same, gaps first. `kanbanr doctor` warns when the project has no facts, and for
each applying row whose source is missing. A project with neither file sees nothing of this
beyond doctor's one hint.

### The architecture repository

Keep it in the board's docs (`kanbanr doc add …`), beside the decisions. The folders below are the
starter catalog's written rows; a project's own catalog decides which it keeps:

```text
architecture/
  context.md           # the system in its surroundings
  behaviour.md         # what it does, in which processes
  components.md        # its parts and how they connect
  components/
    store.md           # one doc per component, describing it as built
    monitor.md
  data.md              # the data model, when it keeps data
  interfaces.md        # its APIs, when it has one
  deployment.md        # where it runs, when it is deployed
  configuration.md
  observability.md
  security.md
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
`kanbanr check` warns "declares no docs" on the next stage that asks for `docs_updated`, the move
into it prints the same warning, and `kanbanr doctor` points it out, so a missing list is seen
rather than waved through. The warning never blocks the move.

When the person confirms an item changes no doc, its definition records why, and the warning
stops:

```yaml
affects_no_docs: "test-only: the change adds tests and touches no design"
```

It sits beside `affects_docs` and, like it, does not reopen the item's approval. `kanbanr feature
show` and the item's page print it as "Changes no docs". If an item names docs and also records
`affects_no_docs`, the named docs are judged and `kanbanr doctor` asks you to remove one.

`docs_updated` also reads the board docs a Design links under its `Affects docs` heading, as
`/p/<project>/docs/file/<path>`. Each must have changed since the item entered its stage, and one
that is not on the board is an error. A doc the Design only reads is marked on its list line with
the word `unchanged` (or `not changed`) and needs no change. An item with no `Affects docs` links
is judged by `affects_docs` alone.

After the move, doctor re-judges the gate over the same window the move used, from when the item
entered the stage it moved from, so a doc folded just before the move is not reported.

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

### A view is exact at a position

`kanbanr view docs` draws the docs at a **position**: a commit and a hash of the uncommitted
changes. The answer says which position it reflects and whether it is exact, and an exact answer
has an **ID** that repeats whenever the position, the board commit and the question repeat. So an
ID you saw last week names the same picture today, and a different ID means something moved. A
view that could not be read consistently (a file changed under it) has no ID and a warning.

Two things to know. A submodule's own state does not move the position (a committed change to the
submodule's pointer does). And a checkout whose line endings git converts on checkout
(`core.autocrlf`, `.gitattributes` `eol=crlf`) reads as clean, as `git status` and katalogr read it,
so a view there is exact at the commit too.

### Drift as work lands

A doc can name `--since` or a route that an item then changed, and stay as it was. kanbanr finds
these when the work lands, rather than when someone notices. It reads the names an item put in the
code, from the diff of the commits whose `Refs:` trailer names it: long flags, commands, routes,
`KANBANR_` variables, settings, stage, check and status names, and files. Documentation files add no
names. It then looks at the texts linked to the item: the docs in its `affects_docs`, the docs whose
front matter lists it under `items` or `refs`, the ADRs whose `affects` names it, the docs and items
that link it, and the items its spec links. A text is **flagged** when it holds one of those names as
a whole token when the commit that changed the name was made. A config key, check, stage
or status name counts only inside a code span, and a command only on a line that says `kanbanr`.

A flag says which text names which changed name. It never says the text is wrong, so read it and
decide:

- Claude settles each flagged text before `docs-folded` is offered: it reads the text against the
  change, then records that it is still right, with why, or that a correction is filed
  (`kanbanr prepare text`). The record names who settled it and the revision it checked. A new
  commit that names the text again, or a later edit to the text, puts it back among those left. The
  Review page lists the texts left, with the names found and their lines, and says Claude settles
  them; the sign-off appears once none is left, and the page shows what Claude checked under
  "Checked by Claude". The sign-off keeps those records. `kanbanr check` lists the texts left under
  "Texts to settle".
- An edit never settles a flag. A flagged text that is edited stays flagged until Claude's check
  covers the edit, and the Review page shows the edit, with its diff, as "Edited while flagged", so
  the person signing docs-folded sees a text that changed while it was flagged. The board does not
  record who made an edit, so every edit to the text is shown. Claude is also refused an edit to
  another item's approved specification: to change one, the person unapproves that item first (the
  Review page), and the edit goes back through review.
- When an item is merged, the flagged texts are listed on stderr.
- `kanbanr drift` lists them for the board, grouped by text, for the items with commits since
  `--since` (default: the latest shipped release's tag date, else 60 days back). A date means from
  the start of that day, UTC, so the result does not depend on the hour you run it. `--json` adds
  when it was generated, so two runs can be compared.

Without the code repository the check is skipped, the Review page says so, and the sign-off is
offered as before. A skipped check is never read as "no drift".

On the monitor, drift is computed in the background, not when a page asks. A daemon started in the
code repository runs it a couple of seconds after the board changes, or within 30 seconds of a new
commit, one run at a time, and keeps the latest result in memory only. Pages show that result with
the code revision and age it was computed at ("as of 3f2a1c9, 4 min ago"), add "recomputing…" while a
newer run goes, and say "computing…" until the first run after a restart finishes, which takes about
as long as `kanbanr drift` does. A monitor with no code repository says "Drift unavailable: this
monitor has no code repository". Before a docs-folded sign-off, `kanbanr drift` is still the
current read.

### When docs-folded is offered

The `docs-folded` sign-off says the documents are folded into the record, so it is offered only
once there is something to confirm. An item's documents are built when all three hold:

1. every board doc it names in `affects_docs`, or links under `Affects docs` as a page of this
   board's docs (`/p/<project>/docs/file/<path>`), has changed since it entered its stage;
2. every repository doc its Design lists under `Affects docs`, written as a path in backticks, has
   changed on its branch against the line it merges into (guides, reference pages, `SECURITY.md`,
   the changelog);
3. every task is complete. A task that names a listed document, a path under `docs/`, or
   `CHANGELOG.md`, `SECURITY.md` or `README.md` is said to write docs.

An item that names no documents at all is offered the sign-off once it has tasks and all are
complete, and the Review page says the item changes no documents, so the sign-off confirms that.

Until then the Review page does not list the sign-off, the sign-off request is refused naming what
is unbuilt, and `kanbanr check` prints `Documents not built yet:` with each document and why. If
the code repository cannot be read, because none is attached or the item has no branch in it, the
sign-off is held and the reason is said. A sign-off given before this check existed stays recorded
and counted.

A `docs-folded` sign-off given with the code repository records the tip of each of the item's
branches. The sign-off lapses when a later commit on the branch changes shipped code or a doc the
item names. A commit that changes only tests, or docs the item does not name, leaves it standing.
Once lapsed, the Review page asks for it again, saying which branch changed which file (or that the
branch was rewritten), and a move into the stage that asks for it is refused until it is signed
again. Fold the docs first, and commit no shipped code to the branch after asking for the sign-off.
A sign-off recorded without branch tips, by an older kanbanr or without the repository, is not
judged by its branches.
