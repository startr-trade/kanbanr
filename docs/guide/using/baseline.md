# The engineering baseline: practice packs asked once

A project makes technical choices that every item builds on: the language, where it runs, how
TLS ends, which database, how migrations run, what the pipeline checks before a release. kanbanr
asks them as questions, once per project, at setup or at the first design that needs each one.
The answers form the project's **engineering baseline**: one record beside the charter, accepted
by you as a whole on the monitor's **Baseline** page (FTR-033).

The baseline is not an item and not part of an item. An item's design names the entries it
touches; work that sets an entry up, such as building the pipeline the baseline promises, is still
an item, and it links the entry it implements.

## The four kinds of entry

Each answer records one or more entries, each of one kind:

| Kind | What it says | Example |
|---|---|---|
| `nfr` | a requirement on the whole system, with its ISO 25010 characteristic | "A secret never appears in a log or a repository" (Security) |
| `constraint` | a decision every design follows; each gets an ADR when you accept it | "PostgreSQL is the database"; "TLS ends at nginx" |
| `non_goal` | something the project will not do | "No Kubernetes: containers run on a host with Compose" |
| `practice` | what every design and build must include | "A migration is safe to run twice" |

An area answered **decide later** records nothing and is asked again at the next design that
touches it. An area answered **not applicable** records the reason and is never asked again.

## Packs and their libraries

The questions are data, not code. They come in **practice packs**: YAML files, each listing
**areas** (a question, its options and what each option records). A project follows a set of
packs. You add, update and remove them independently, and each area name is unique across the
packs a project follows.

Packs live in the same three libraries as saved processes:

| Library | Where | Changed by |
|---|---|---|
| Built-in | in the kanbanr binary | kanbanr releases; the changelog lists added or changed areas |
| Board | `profiles/<name>.yaml` in the board, beside `processes/` | the team, through the board's git remote |
| Personal | `~/.kanbanr/profiles/` (or `KANBANR_PROFILES_DIR`) | you, on any board |

A project keeps the applied copy of each pack it follows in `projects/<id>/packs/<name>.yaml`, and
records the pack's name, library and version in `baseline.yaml`. When a library's copy moves on,
`kanbanr doctor` and the session start say so: new areas to answer, recommendations that changed.
`kanbanr profile diff` shows the change, and `kanbanr profile update` adopts it, only when you ask.
**An answer is never overwritten**: a changed recommendation leaves your answer alone, and a new
area becomes an open question, asked at first need.

**Removing a pack discards nothing.** Its answered entries stay in the baseline, marked "pack
removed", keep their ADRs, and are no longer asked about or checked against designs. Adding the
pack again brings them back unchanged.

**Every option teaches.** Each option carries its `why` (what choosing it means), `fits_when`
(when it is the right choice) and `costs` (what it gives up or needs). The agent shows the `why` with
each option, and **Explain the options** prints the other two for every option of the area, with
what the project folder shows that bears on them. A built-in pack with an option missing any of
the three does not pass `kanbanr profile check`.

A team writes its own packs in conversation ("add an area for audit logging", "our default proxy
is Caddy"): the agent drafts the file, `kanbanr profile check` validates it without applying it, and
`kanbanr profile save` writes a new version to the board or your personal library.

## The built-in packs

`core` is followed by every project; the others name `core` in their `requires`. At setup, kanbanr
proposes the packs that fit what it found in the folder, each a yes or no in the setup plan, and
any pack can be added later, at the first design that touches its areas.

| Pack | Areas (asked at setup in **bold**; the rest at first need) |
|---|---|
| `core` | **`project-kind`**, **`stack`**, **`runs-where`**, **`iac`**, **`delivery`**, **`practice-baseline`**, `edge`, `identity`, `config-secrets`, `performance-budgets`, `data-protection` |
| `observability` | `observability-needs`, `general-observability`, `audit-trail`, `dashboards` |
| `pipeline` | `pipeline-shape`, `runners`, `ci-tiers` |
| `devsecops` | `code-review`, `static-analysis`, `supply-chain`, `secrets-scanning`, `artifact-security`, `deployment-approval`, `deployment-audit`, `rollback` |
| `web` | `ui-style`, `web-packaging`, `web-origins`, `e2e-tests`, `dev-exposure` |
| `api` | `api-style`, `api-docs` |
| `data` | `database`, `migrations`, `seed-data`, `backups` |
| `containers` | `containers`, `packaging` |
| `kubernetes` | `kubernetes` |
| `infra` | `infra-state-environments` |

`kanbanr profile show <pack>` prints a pack's file, with every option and its reasoning. Some areas
take several options at once (`runs-where`, `delivery`, `database`, `packaging` and others): a
product can ship as a container image, a Compose file and Kubernetes manifests together.

## Asked once: at setup and at first need

**At setup**, in the setup interview, the agent runs `kanbanr profile ask --json`. It lists the setup
areas the folder does not already answer, with the detected option first (or else the recommended
one), in dialogs of at most four questions. **Decide later** and **Explain the options** are always
offered. A detected answer is still yours to confirm. The setup plan shows the packs to follow and
the **draft baseline** (each area, its answer, the kind of entry it records), and after setup the
draft waits on the Baseline page for you to accept.

**At the first design that needs an area**, `kanbanr check <CODE>` names the areas the design
touches that are not answered yet, and the agent asks them in the design dialog, before the design is
offered for review. The answer is proposed as a change to the baseline, not written into the item,
and you accept it on the Baseline page alongside the design-review. An answered area is never
asked again.

**Changing an answer** ("move from SQLite to PostgreSQL") is a new revision: the agent shows what
depends on the old answer (the items, ADRs and docs that name it), proposes the revision with the
new option, and files the work items the change needs. Nothing changes until you accept it. The
old answer stays in the history, and the new constraint's ADR supersedes the old one.

## The answers file

A proposal is written as an answers file and proposed with
`kanbanr baseline propose --answers <file> [--item CODE] [--note "…"]`:

```yaml
item: FTR-012            # optional: the design that asked
note: The first table    # optional
answers:
  - area: database
    options: [postgresql]
  - area: ui-style
    options:
      - {option: component-library, value: MUI}   # an option that asks for a value
  - area: migrations
    later: true                                   # decide later
  - area: dev-exposure
    not_applicable: the product has no dev instance to share
```

`option: <id>` reads as one option. A proposal is refused, by name, for an unknown area or option,
a single-choice area given other than one option, an option that asks for a value given none, or
an area another proposed revision already answers ("REV-002 already proposes 'database': accept or
withdraw it first").

## The Baseline page

The **Baseline** page is in the monitor's **Architecture** menu, at `/p/<project>/baseline`. It
shows:

- **the packs** the project follows, each with its version and library, and whether it is
  `current`, `changed` in its library or `removed`;
- **proposed revisions**, each as a panel with the answers, the entries it adds and replaces, the
  release-stage gates it adds and what depends on them. **Accept** is the one action, and you can
  withdraw a revision with a reason;
- **the areas**, grouped accepted, proposed, open (deferred ones marked "decide later") and not
  applicable with the reason. Each entry shows its kind, its text, a link to its ADR and the items
  that name it;
- **also in the charter**: the charter's own constraints, which stay where they are;
- **the history** of accepted revisions.

Accepting is yours alone: `kanbanr baseline accept` is refused inside an agent session, and an
acceptance counts only when it was committed through kanbanr. Without an accepted baseline, the
gates treat the project as having none and say so.

## A design names what it touches: `### Baseline`

A design that touches an accepted entry (its text mentions the entry's words, or a path it names)
says how, under a `### Baseline` subsection of its `## Design`, one line per entry:

```markdown
### Baseline
- B-003: migrations::tests::a_migration_runs_twice_safely
- B-007: no new endpoint, so the API contract is unchanged
```

The `baseline` check ([The checks](processes.md#the-checks)) fails while a touched entry is not
named, and while the design names another option of an answered area (it says "SQLite" where the
baseline says PostgreSQL) without a reason. A reason written there is the record; otherwise propose
a revision.

## Release-stage gates from the baseline

Some options promise support a release must not ship without. Such an option carries
`adds.release_gates`, a release stage and a gate name: choosing GitHub Actions under `delivery`
asks for a passed `ci` gate at `main`, and choosing `sast` under `static-analysis` a passed `sast`
gate there. When you accept the revision, the same commit adds the gate to that stage in the
project's `cadence.release_gates`, marked with the entry it came from.

Promoting a release into that stage is then refused until a passed result for the gate is recorded
for the release's commit: "stage main requires a passed `sast` result for the release tip (B-007)".
The Releases page and `kanbanr release stages` show each such gate under its stage, as "requires
`sast` (baseline B-007)".

So the baseline becomes the pipeline's to-do list: the gates it names are the steps the pipeline
must record before the first release. Until a result of that gate is recorded on the board at all,
`kanbanr check` says so: "the baseline requires the `sast` gate at main; nothing records it yet:
add the pipeline step (see the practice text) or revise the entry". A revision that supersedes or withdraws the entry,
or removes its pack, removes the gate again; a gate removed by hand is reported by `kanbanr doctor`.

## The commands

| Command | What it does |
|---|---|
| `kanbanr profile list` | every pack in every library, with the projects that follow each |
| `kanbanr profile show <NAME>` | a pack's file |
| `kanbanr profile check <FILE\|-\|NAME>` | check a pack without applying it |
| `kanbanr profile save <NAME> [--from-file F] [--personal]` | save a pack to the board or your personal library |
| `kanbanr profile add <NAME>` / `remove <NAME>` | follow a pack in this project, or stop following it |
| `kanbanr profile ask [--at setup\|first_need] [--item CODE] [--answers F] [--explain AREA]` | what to ask now, as JSON |
| `kanbanr profile status` / `diff [<NAME>]` / `update [<NAME>]` | whether the packs moved on, how, and adopting it |
| `kanbanr baseline show` | the baseline: packs, areas, entries, revisions |
| `kanbanr baseline propose --answers F` | propose answers as a revision |
| `kanbanr baseline accept <REV-n>` | yours: accept a revision (on the Baseline page, or in your terminal) |
| `kanbanr baseline withdraw <REV-n> --reason "…"` | withdraw a proposed revision |

The [command reference](../reference/cli.md) has every flag.
