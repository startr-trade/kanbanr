# Processes: presets, gates and sign-offs

A project's workflow is its **process**: the statuses work moves through, which moves are
allowed, and what each status asks of an item before the item may enter it. kanbanr knows how to
check things. The project decides which checks apply where. None of it is hard-coded: TOGAF, PDCA,
a design-control flow or your organisation's own process is a file.

In Claude Code you never read this table to know what to do next: Claude asks the board what the
next stage needs and turns each gap into a draft or a question — see
[Working with Claude](with-claude.md).

## Choosing a process

```bash
kanbanr config workflow --preset list          # what each preset is
kanbanr config workflow --preset togaf         # apply one (statuses, transitions and gates)
kanbanr config workflow --export > ours.yaml   # this project's workflow, as a file
kanbanr config workflow --from-file ours.yaml  # load your own
kanbanr project init shop --workflow pdca      # choose at creation
```

| Preset | Stages | Gates |
|---|---|---|
| `default` | Planned → In Progress → Completed, plus Deferred and Ongoing | the built-in rule, plus the method |
| `scheduled` | Planned → Scheduled → Completed | the built-in rule, plus the method |
| `togaf` | Vision → Business Arch → System Design → Implementation → Migration → Operations | the definition grows phase by phase |
| `pdca` | Plan → Do → Check → Act | agreement to start, evidence to check, a review sign-off to act |
| `design-control` | Planning → Inputs → Design → Review → Verification → Validation → Released | sign-offs at review and validation |
| `scrum` | Backlog → Ready → In Progress → Review → Testing → Done → Released | Definition of Ready and of Done; work starts in the sprint; sprints, releases and points on |
| `agile` | Plan → Design → Develop → Test → Review → Released | agreement before build, evidence before review; sprints and releases on |

Every built-in process also carries [the method](#the-method-on-by-default): a design before
build, and the docs it changes kept current.

`design-control` is **modelled on** the design-and-development controls of ISO 9001 clause 8.3.
That is not a claim of compliance: only a certification body can make that.

A preset is copied into the project when you choose it. From then on the project's own
`config.yaml` is what counts, and editing it never changes the preset. New projects get
`default`. Existing boards keep whatever workflow they have.

**The built-in rule.** A workflow that declares no gates of its own still has one (a gate
addressed to a role, such as the method's, adds to it rather than replacing it): moving an item into a
status that means work has started needs an approved definition. That is a status the board
displays, that is not where items start, and that is neither an end state nor a no-op. `start`
makes the branch at the first such status. And finishing an item's last task completes it only when
each of its requirements has a green test — what `kanbanr finish` asks — otherwise the item stays,
and the task write says which requirements are unproven.

## Gates

A gate is the entry criteria of one status:

```yaml
gates:
  System Design:
    purpose: How and where it will be built, and the quality it must reach.
    requires: [{zachman: [how, where]}, approved]
    warns: [quality]
  Implementation:
    purpose: Build it, with a test named for every requirement.
    requires: [tests_named, approved]
    on_enter: [branch]
  Operations:
    requires: [tests_green]
    signoffs: [release]
```

| Field | Meaning |
|---|---|
| `purpose` | What the stage is for. `kanbanr check`, the monitor and CLAUDE.md show it, so the definition is written one stage at a time. |
| `requires` | Checks that must pass to enter. |
| `warns` | Checks that are reported, never enforced. |
| `signoffs` | Named sign-offs that must be recorded against the current definition. |
| `enforce` | `block` (the default) refuses the move; `warn` allows it and reports what is missing. |
| `kinds` | Apply only to items of these kinds; an item with no kind is a `feature`. |
| `on_enter` | `branch`: `kanbanr start` makes the item's branch here, where the project is a git repository. |
| `affects_docs` | Docs every item entering this stage must list in `affects_docs`; the move adds them, without reopening the approval ([Coverage rules](living-docs.md#coverage-rules-keeping-the-views-whole)). |
| `done` | `true`: reaching this stage finishes an item for the sprint burndown, the sprint's done total and what a closing sprint carries over. Without it only the end statuses count. The `scrum` preset marks **Done**, its Definition of Done, so a sprint burns down as items are done rather than all at once when a release ships them. |

### The checks

The vocabulary is closed on purpose: a gate can only ask for what kanbanr can judge from the
board.

| Check | Passes when |
|---|---|
| `definition` | the item has a definition at all |
| `statement` | the one-sentence statement is written |
| `goals` | it links at least one charter goal |
| `zachman` | all six dimensions are answered; `{zachman: [what, why]}` asks for only those |
| `requirements` | it has at least one requirement |
| `ears` | every requirement is in EARS form |
| `tests_named` | every requirement names a test |
| `tests_green` | every requirement has a green test |
| `quality` | quality requirements carry an ISO 25010 tag and a measured scenario that names its test |
| `approved` | the definition is agreed: approved, or ratified after the fact |
| `bypass` | a recorded override has been answered |
| `goals_known` | every linked goal exists in the charter |
| `small` | not estimated above three days |
| `estimated` | estimated in the project's unit: story points or days (`kanbanr config cadence --unit points`) |
| `in_sprint` | planned into the active sprint (projects with sprints switched on) |
| `in_release` | planned into a release (projects with releases switched on) |
| `spec_sections` | the spec has each named section, with content; plain `spec_sections` asks for `Design` |
| `docs_updated` | every board doc the item names in `affects_docs` has changed since it entered its current stage |
| `diagrams_valid` | every Mermaid diagram in the spec and in each doc it names in `affects_docs` passes the diagram lint |
| `docs_cover` | the item is covered by every project coverage rule it falls under; `{docs_cover: [id]}` asks for only those rules |

An unknown check, an unknown status or an unknown Zachman column is refused when the workflow is
saved. A gate that could never match is a guardrail that silently isn't there.

`diagrams_valid` is judged when the item moves, never from a cached result, so fixing the diagram
unblocks the very next move. To keep a broken diagram from moving anywhere, ask for it on every
gate:

```yaml
gates:
  Scheduled:   {requires: [approved, diagrams_valid]}
  In Progress: {requires: [approved, diagrams_valid]}
  Completed:   {requires: [tests_green, diagrams_valid]}
```

A refused move names each diagram: `FEAT-012 spec, diagram #1, line 14: … (semicolon_in_text)`.
`warns: [diagrams_valid]` reports the same without refusing.

### When a gate isn't met

- **A blocking gate** refuses the move and lists everything missing.
- **`docs_cover`** names each rule the item fails and the view doc to add it to:
  `[zachman] FEAT-012 is not referenced from architecture/zachman.md — add a link to it there`.
  Add the link in that doc, then move again. A move that passes still prints any rule it now
  fails as a `coverage:` warning ([Coverage rules](living-docs.md#coverage-rules-keeping-the-views-whole)).
- **`--override "<reason>"`** gets past a blocking gate, and the reason is kept in the item's
  history. It also records the item as started without agreement, so it shows on the Review page
  until someone ratifies it. `--unapproved` is the older name.
- **Finishing the last task** moves the item to its end status only if that status's gate is
  met — and, on a workflow with no gates, only if every requirement has a green test. Otherwise
  it stays, and `kanbanr task state` says why. A batch is judged after its last operation, so one
  that ticks the last task and then adds another never completes the item.

## Stage roles

Every process, whatever its stages are called, has four kinds of stage. A gate says which ones its
status holds:

| Role | What the stage is for |
|---|---|
| `design` | the item's design is written and agreed: how and where, before build |
| `build` | code is written on the item's branch |
| `integrate` | the item's work joins its line of integration and the docs fold in |
| `release` | the work ships, and the item is finished |

```yaml
gates:
  Implementation:
    requires: [tests_named, approved]
    roles: [build]          # the branch is made here
  "@build":                 # a gate addressed to a role, whichever status holds it
    signoffs: [design-review]
```

- **One holder per role.** Each role belongs to one status; a status can hold several (the
  default process's Completed integrates and releases). Saving a workflow that gives one role to
  two statuses is refused, and so is a release role on a status that isn't an end.
- **Role-addressed gates.** A gate keyed `"@build"` joins the gate of whichever status holds the
  build role. A rule written once then works in every process: kanbanr's method and its git
  automation find their stage this way.
- **Every built-in process declares its roles.** `kanbanr process checks` lists them, and the
  working agreement names each stage's role.
- **Boards without roles** get build where the branch is made and release at the first end status,
  marked "(inferred)". Design and integrate are never guessed. Nothing behaves differently.
- **Older kanbanr versions:** a workflow whose gates only name roles, or that addresses a gate to a
  role, is stamped schema 4, so an older kanbanr refuses the board rather than misread it.

## The method, on by default

Every built-in process asks for the same two disciplines, at whichever stages hold the roles:

| Part | Where | What it asks |
|---|---|---|
| `design` | the build stage | `spec_sections: [Design]`: the spec has its build-ready Design ([below](#the-design-stage-a-build-ready-design-in-the-spec)) |
| `design-review` | the build stage | the `design-review` sign-off: a person reviewed that design |
| `docs` | the integrate stage | `docs_updated`: each doc in `affects_docs` changed ([Living architecture docs](living-docs.md)) |
| `docs-folded` | the integrate stage | the `docs-folded` sign-off: the design was folded into those docs |
| `links` | everywhere | `kanbanr doctor` reports references to things the board does not have |

It is written as two role-addressed gates, `"@build"` and `"@integrate"`, so you see it in your
exported workflow and in the working agreement, and it adds to each stage's own rules.

```bash
kanbanr config method                       # each part, on or off, and where it is in force
kanbanr config method --off                 # all of it off
kanbanr config method --off docs docs-folded   # just those parts
kanbanr config method --on design           # one part back
```

Turning a part off takes its gates out and records the choice: applying a built-in process, or
`kanbanr process update`, keeps it off, and drift does not report it as an edit. Saved processes
of your own are never added to; they are only shaped by the choice.

**Projects from before the method** keep their workflow as it is. `kanbanr doctor` asks once —
"this project predates the method" — until you choose either way. A project that recorded a
built-in process sees it reported as changed (`kanbanr process diff`), and `kanbanr process update`
brings the method in.

**Upgrading:** every machine that shares a board should run a kanbanr that knows these gates
(`docs_updated` and role-addressed gates); an older one refuses the board rather than misread it.

## Sign-offs

Some conditions are invisible in board data: a design review was held, a release was approved.
A **sign-off** records one:

```bash
kanbanr signoff FEAT-042 design-review --note "held 30 Sep" --doc reviews/checkout.md
```

- **What it records:** who gave it, when, in which status, and the definition it covered.
- **When it lapses:** changing the definition lapses it, just as it lapses an approval. A review
  of a different design is not a review of this one. Earlier sign-offs are kept.
- **How it is asked for:** a gate asks for it by name (`signoffs: [design-review]`). The Review
  page offers a **Sign off** button for each one the next stage is waiting on.
- **Where it can be given:** the review queue offers a sign-off only for the stage the item
  reaches next, and only once that stage's other requirements are met. A workflow that lets an
  item skip ahead doesn't get the later stage's sign-off offered early.
- **Who gives it:** a sign-off is a person's agreement. Claude asks for it and never gives it.
- **Taking it back:** a sign-off given in error is withdrawn with a reason:

  ```bash
  kanbanr signoff FEAT-042 design-review --withdraw --reason "given before the review was held"
  ```

  The earlier sign-off stays on the record and stops counting, so the stage is gated again until
  someone signs off afresh. The item's page in the monitor has a **Withdraw** button for each
  sign-off when the monitor can write.

## Growing a definition stage by stage

Under a phased process, an item doesn't need its whole definition at once. `kanbanr check` says
what the next stage needs:

```text
✗ FEAT-001 Checkout
    [MISSING: Who]
    …
    → to move to Business Arch (Who it is for, what it must do, and why), still needed:
        no goal link — nothing says what this is for
        [MISSING: Who]
        no requirements — nothing states what must be true for this to be done
```

`doctor` follows the same rule on a workflow with gates: it reports what the next stage asks, not
what a later stage will. Each time the definition grows, its approval lapses and is given again.
Each approval records the status it was given in, so the history reads "approved at Business
Arch, again at System Design".

## The design stage: a build-ready design in the spec

At a design stage, the item's **spec** carries the technical design at implementation level, in a
`## Design` section. It covers:
- components, modules and files;
- public interfaces with signatures and responsibilities;
- data model and formats, algorithms and control flow;
- concurrency and failure handling;
- configuration and defaults, and dependencies;
- security and performance budgets;
- a test plan that maps each requirement to its tests;
- compatibility, ADR links and open questions.

The definition's `how` and `where` lines summarise it. Every built-in process asks for both
through [the method](#the-method-on-by-default). In a process of your own, write it as:

```yaml
gates:
  Implementation:
    requires:
      - tests_named
      - approved
      - spec_sections: [Design]   # the spec has a non-empty "Design" heading, at any level
    signoffs: [design-review]
```

Write the section so a cheaper model can build from it. It ends with an implementation task
list: each task names its files, signatures, behaviour, tests (with the requirement each serves)
and the tasks it depends on, and leaves no decision open. The tasks go into the item's todo-list,
so a low-cost subagent can take them one at a time.

`spec_sections` passes when every named heading is in the spec with content under it. Content in
a nested subsection counts, and a heading inside a code fence does not.

## A worked example: PDCA

```bash
kanbanr config workflow --preset pdca
kanbanr feature add --title "Cut checkout errors" --milestone MS-001    # lands in Plan
kanbanr check FEAT-001                  # Do asks: statement, goals, requirements, tests named, approval
kanbanr feature define FEAT-001 --file cut-errors.yaml
kanbanr approve FEAT-001                # the user agrees to the plan
kanbanr start FEAT-001                  # Do: branch made here
# … make the change; tests go green …
kanbanr move FEAT-001 Check             # Check asks: tests green
kanbanr signoff FEAT-001 review         # the user records the review
kanbanr finish FEAT-001                 # Act: the end status, gated by the review sign-off
```

## Sprints and releases

Sprints, releases and burn-rate reports are **off unless a project switches them on**. The `scrum`
and `agile` presets do this, and any other project can too:

```bash
kanbanr config cadence --sprints on --releases on --unit points --sprint-length 14
kanbanr sprint add --start 2026-10-05 --goal "Shoppers can pay" --capacity 20
kanbanr sprint plan SP-001 FEAT-001 FEAT-002    # warns when it goes over capacity
kanbanr sprint start SP-001
kanbanr sprint list                             # every sprint and its state
kanbanr sprint show                             # goal, days left, committed vs done, burndown
kanbanr sprint close SP-001 --carry-to SP-002   # unfinished work moves on, and is recorded
kanbanr retro --sprint SP-001

kanbanr release add v0.1.0 --target 2026-10-18
kanbanr release plan v0.1.0 FEAT-001 FEAT-002
kanbanr release list                            # every release, its state and its stage
kanbanr release promote v0.1.0 --plan           # what the next stage would do and needs; changes nothing
kanbanr release promote v0.1.0                  # candidate: makes the branch release/v0.1.0
kanbanr release promote v0.1.0 staging          # a named stage; a middle one changes only the board
kanbanr release promote v0.1.0 test --reason "the payment check failed on staging"   # back a stage
kanbanr release promote v0.1.0 production       # the cut: ship, notes, fast-forward main, tag
kanbanr release stages                          # what each stage requires and does
kanbanr feature add --title "…" --milestone MS-001 --found-in v0.1.0   # feedback on a release
```

The burndown and velocity are derived from the moves items record; nothing is stored for them. An
item counts as finished from the day it first reaches a stage marked `done` — Done, in `scrum` — or
an end status.

On the board, a project that uses sprints opens on the running one — its goal, dates, days left,
committed against done, and the burndown against the ideal line:

![The board on a running sprint: goal, dates, points done and the burndown](../images/sprint.png)

The Releases page shows each release's scope, how much of it is finished, its target, and its notes
once cut; the Gantt adds the sprints as a lane and the releases as milestones:

![The Releases page: a shipped release and a planned one](../images/releases.png)

![The Gantt with a sprint lane and release milestones](../images/gantt-sprints.png)

### Release stages

A release moves through the environments it is deployed to before production. By default they are
`candidate → test → staging → production`; a project names its own with
`kanbanr config cadence --release-stages candidate,qa,uat,live`. Each stage must appear once, and
the **last one is always the cut**. Every move is recorded on the release with who made it and when.

- **The first stage** makes the branch `release/<VERSION>` from the integration branch, without
  checking it out.
- **A middle stage** is a board change only.
- **Back a stage** — staging to test after a failed check — needs `--reason`, which is recorded.
  Nothing moves back from production.
- **The last stage** is the cut. It ships the finished items, writes the notes, tags the code
  repository with the version and **fast-forwards the main branch** to the release, so main's tip
  is exactly the tagged commit. Before anything changes it checks that the working tree has no
  uncommitted changes, that the release branch exists, that the tag does not, and that main has no
  commits the release lacks; any failure refuses with its reason and **changes nothing**. If main
  has moved on, merge main into the release branch and promote again. If the board step fails after
  the tag and the fast-forward, both are undone. `kanbanr release cut <VERSION> --tag` is the same
  as promoting to the last stage; a plain `release cut` changes only the board.

A release that has no branch yet gets `release/<VERSION>` made first whichever stage it is promoted
to, so a process with a single stage (both first and the cut) works, and so does a promotion that
skips the first stage.

A release with **nothing planned into it is never promoted**, to any stage: it would ship nothing.
It is refused with "v0.3.0 has no items: plan some in with `kanbanr release plan v0.3.0 <CODE>…`",
and the Releases page offers it no button. A patch is the exception: it carries its parent's fixes,
so it is promoted with nothing of its own planned.

#### Stages that act

That is the default. A stage can instead say what must hold before a release enters it
(`requires`) and what entering it does (`on_enter`), as an item's status has a gate. The rules sit
beside the stage names, under `cadence.release_gates`, keyed by stage name:

```yaml
cadence:
  release_stages: [candidate, main, production]   # names only; the last is the cut
  release_gates:
    candidate:  { on_enter: [branch, prepare] }
    main:       { purpose: "main holds the release; push it and let CI run",
                  requires: [items_finished], on_enter: [move_main] }
    production: { requires: [main_at_release, pushed], on_enter: [tag] }
```

| action | does |
|---|---|
| `branch` | makes `release/<V>` from the line of integration, nothing checked out |
| `prepare` | one commit on `release/<V>`: the version files, the dated changelog heading and its copies, the notes draft and its listings (see below) |
| `move_main` | fast-forwards main to the release tip; refused if main has commits the release lacks (a patch of an older release leaves main where it is) |
| `tag` | tags the release tip `<V>` ("Release <V>"); the last stage only |

| condition | holds when | fix shown |
|---|---|---|
| `items_finished` | the cut would carry nothing over | finish the item, or plan it into another release |
| `prepared` | the release tip carries the version, the changelog heading and the notes | promote to the stage that prepares |
| `main_at_release` | main is at the release tip | promote to the stage with `move_main` |
| `pushed` | the release tip is on main's remote-tracking branch (`origin/main`) | push main (`git push origin main`), then wait for CI |

A stage with no gate of its own keeps the default:

| stage | default `on_enter` |
|---|---|
| the first | `branch` (unless another stage holds it) |
| a middle one | nothing |
| the last | `move_main, tag` (without `move_main` if another stage holds it) |
| the only one | `branch, move_main, tag` |

A promotion takes the actions of every stage it passes or enters, in the order `branch`, `prepare`,
`move_main`, `tag`, and needs every condition those stages require. Everything is worked out first;
if anything is missing it is refused with **each missing condition and its fix**, and nothing
changes. A condition an action of the same promotion makes true counts as met (`prepare` makes
`prepared`, `move_main` makes `main_at_release`); nothing makes `pushed` true, so skipping straight
to the cut still waits for the push. If a step or the board fails after earlier steps, every git
step is undone, newest first. Promoting to the stage a release is at takes its actions again when
any of them would change something. Moving back stays a board change: main never moves back, since
it may already be pushed.

`pushed` reads the remote-tracking branch. A promotion fetches main first; offline, it judges from
the last fetch and says so. kanbanr never pushes: pushing main and the tag is yours.

```bash
kanbanr config release-stage candidate --on-enter branch,prepare
kanbanr config release-stage main --requires items_finished --on-enter move_main
kanbanr config release-stage production --requires main_at_release,pushed --on-enter tag
kanbanr config release-stage production --clear     # back to the default
kanbanr release promote v0.3.0 --plan --fetch        # the plan, after fetching main
```

`release promote --plan` prints the button, each step, each condition with ✓ or ✗ and its fix, and
for the cut what it tags, ships and carries over. After a promotion, kanbanr says what is next —
"next: push main (`git push origin main`), then wait for CI, then Cut v0.3.0".

#### Preparing a release

A release prepares itself: entering the stage with `prepare` commits, in one commit on
`release/<V>`, what a release needs. No separate item is made for it.

```bash
kanbanr config release-prepare --version-file api/Cargo.toml \
  --version-file 'api/Cargo.lock=name = "kanbanr-core"\nversion = "{version}"' \
  --changelog CHANGELOG.md --changelog-copy docs/src/project/changelog.md \
  --notes 'docs/src/releases/{version}.md' \
  --notes-listing 'docs/src/SUMMARY.md::- [Release notes](releases/index.md)::  - [{number}](releases/{version}.md)'
kanbanr config release-prepare                     # shows it
```

- **Version files.** Each file's version is set to `{number}` (`v0.3.0` → `0.3.0`). A `*.toml` file
  matches `^version = "{version}"` and a `*.json` file `"version": "{version}"`, the first match
  only. Any other file needs a match: literal text with one `{version}`, where whitespace matches
  any whitespace (newlines too) and a leading `^` anchors to a line; an explicit match sets every
  occurrence. A missing file, or a match that finds nothing, refuses the promotion.
- **Changelog.** `## [{number}] - {date}` goes below the first `## [Unreleased]`, so what was
  unreleased belongs to the release; an `[Unreleased]: …compare/<prev>...HEAD` link moves on and
  `[{number}]` gets its own. Each copy gets the same text.
- **Notes.** A draft from the items the cut would ship, written only if the file is not there yet,
  and listed below the line named in each listing.

Nothing is checked out: kanbanr writes the commit with git's plumbing, so no hook runs, and the
commit carries `[no-ref] release preparation, made by kanbanr` and a `Kanbanr-Release:` trailer.
People's commits on `release/*` stay refused; kanbanr's preparation commit is the one exception.
With nothing configured, `prepare` bumps nothing, makes no commit, and the plan says so.

### Patch releases

A fix to a release already in production gets a release of its own, a **patch**:

```bash
kanbanr release add v1.0.1 --patch-of v1.0.0
```

The patch records its parent, which must have been cut and tagged — any cut release, an older
supported one included. `release/v1.0.1` is made from the parent's tag, not from where main has
since gone, and the patch starts at the first stage. It goes through the same stages. Its notes say
"Patch of v1.0.0", list what the patch adds, and what it keeps from the parent.

At production the patch is tagged. Main fast-forwards to it when it can; when main has moved on
past the parent, main is left where it is. The patch's commits that the integration branch does
not have yet are listed — in the output and in the notes, under "To merge forward" — so the next
release keeps the fix. kanbanr does not merge them for you: `kanbanr git forward <VERSION>` does it
when the git model is on, or merge them by hand.

The integration branch and the main branch are both the repository's default branch unless the
project names them: `kanbanr config cadence --integration-branch develop --main-branch main`.

The first and last stages act on the code repository, so run them from the project folder. The
monitor (`kanbanr serve --allow-writes`) uses the repository it was started in, when that folder's
marker names the project; otherwise it refuses them with "use the CLI from the project folder".
Nothing is pushed: publishing the tag and main is whoever pushes.

Items reach the project's end status through the production stage of a release they are planned
into: the cut moves them there, and a gate asking for `in_release` stops them getting there any
other way.

### The git model

The cadence also says how the code repository is laid out. `kanbanr git model` shows it:

| Cadence | Line of integration | Production moves at |
|---|---|---|
| neither (trunk) | main | the merge of each finished item |
| sprints only | `sprint/<SP>` of the running sprint | the sprint's close |
| releases only | the integration branch (`integration` unless named) | the release cut |
| sprints and releases | `sprint/<SP>` of the running sprint | the release cut |

Items are built on `feat/<CODE>-<slug>` (a defect on `fix/<CODE>-<slug>` when the pattern is the
default) or `spike/*`. A release is branched as `release/<VERSION>` when it reaches its first
stage. main, the integration branch, `sprint/*` and `release/*` are **lines**: they take merges
only, and a merge into one must say which items it brings (`Refs: kanbanr:<CODE>`, or `[no-ref]
<why>` on the record).

The model is always shown; it drives the work only when you switch it on:

```bash
kanbanr config cadence --git-model on
```

Then `start` branches an item from its line (making the integration branch from main the first
time), and a fix for a release under test from that release's branch. `sprint start` makes
`sprint/<SP>` from the newest sprint main does not have yet, else from main. A release's branch is
made from the line its cadence says. The commit hooks refuse plain commits on a line, and a
generated merge message that names no item. kanbanr never pushes: the code repository refuses a
push from it outright.

With the model on, the board makes the merges:

- **Integrating an item merges it.** Moving an item into the integrate stage (or past it) merges
  its branch into its line with a merge commit naming the item (`Refs: kanbanr:<CODE>`). If the
  merge would conflict, the move is refused before anything changes, naming the files: merge the
  line into the item branch, resolve, commit, and move again. The item branch is kept. A batch
  cannot integrate an item; move it on its own.
- **A sprint's close is production** when there are sprints but no releases: main fast-forwards to
  `sprint/<SP>` and is tagged with the sprint's code. If main has commits the sprint lacks, the
  close is refused and nothing changes.
- **Carried items catch up.** When the sprint they were carried into starts (or the close names
  one that already has a branch), its branch is merged into each carried item's branch — never a
  rebase. One that would conflict is reported with its files and left as it was.
- **A fix to a release under test** integrates into `release/<VERSION>`, and the move says the line
  lacks it. `kanbanr git forward <VERSION>` merges the release forward into the line, naming every
  item its commits name. It is never done for you.

`kanbanr config workflow --write-agreement` writes the team's working agreement to the board,
generated from the gates. Under Scrum that is the Definition of Ready and of Done. Because it is
generated, it can't drift from what is enforced.

## Writing your own process

Start from the closest preset and edit it:

```bash
kanbanr config workflow --preset design-control
kanbanr config workflow --export > our-process.yaml
# edit our-process.yaml: rename stages, move sign-offs, add `kinds:` to gates
kanbanr process check our-process.yaml               # every problem, then the agreement; changes nothing
kanbanr config workflow --from-file our-process.yaml
```

`kanbanr process check` runs the same validator the board runs when a workflow is saved, so a file
that checks clean is one the board accepts. It reports every problem at once, not only the first.
It needs no board, so a process can be checked before the project exists, and `-` reads the
file from stdin (`kanbanr process check - < draft.yaml`). Where there is one, it
also names any status the file would remove that still holds this project's items. When the file
is clean, it prints the working agreement and a Mermaid diagram. `kanbanr process checks` lists
every check a gate can ask for, from the same list as the table above.

The file holds statuses, `default_state`, `displayed_states`, `no_op_states`, `terminal_states`,
`transitions` and `gates`. It has no name, so one process file can serve many projects. A board
that declares gates is stamped `schema_version: 3`, so an older kanbanr refuses to open it instead
of ignoring its gates.

## Designing a process with Claude

You don't have to write the file. Tell Claude how your team works ("design review before we build,
QA signs off releases"), at setup or any time later. In plan mode, it:

1. offers the processes already saved on the board and in your personal library, then the
   built-in ones;
2. for "our own", starts from the nearest built-in process and proposes stages for you to correct;
3. asks one question per stage, *what must be true before work enters it?* The choices are the
   checks above, in plain words (`kanbanr process checks`), so you can only agree to what kanbanr
   can enforce;
4. turns anything kanbanr can't see ("the architecture board has reviewed it") into a named
   sign-off that a person records;
5. asks the process-wide choices together: block or warn, which stage holds each
   [role](#stage-roles) (design, build, integrate and release; build is where the branch is made),
   which stage ends the work, which moves back are allowed;
6. asks its name, and whether to keep it on the board for the team or in your personal library;
7. checks the draft with `kanbanr process check` and puts its working agreement and diagram in the
   plan, with a one-line description of what the process is for.

Accepting the plan saves the process and applies it. Changing it later is the same conversation,
and then Claude asks, one project at a time, whether each project using it should take the new
version.

## Saving and sharing processes

A process you will use again can be saved by name, and then chosen like a preset:

```bash
kanbanr process save our-process --description "Design review before build; QA signs off"
kanbanr process save our-process --from-file our-process.yaml   # or from a file
kanbanr process save my-process --personal                      # in your own library instead
kanbanr process list                                            # every process, and who uses it
kanbanr process show our-process                                # its file, agreement and diagram
kanbanr config workflow --preset our-process                    # apply it to this project
kanbanr project init storefront --workflow our-process          # or start a project with it
```

| Library | Where | Who has it |
|---|---|---|
| The board | `processes/` in the board, beside `projects/` | everyone with the board, through its git remote |
| Personal | `~/.kanbanr/processes/` (`KANBANR_PROCESSES_DIR` to move it) | you, on any board |
| Built-in | in kanbanr | everyone |

`process save` keeps a process on the board unless you pass `--personal`. Without `--from-file`,
it saves this project's workflow. A process must pass `process check` to be saved. A built-in
process's name can't be reused. Saving again leaves the version alone when nothing changed, and
adds one when something did.

A name is looked up on the board first, then in your personal library, then among the built-in
processes. A team's agreed process always wins over one person's copy of the same name;
`process list` marks a personal copy the board's hides.

A project records the process it was given: its name, library, version, and a hash of its content.
A personal process is sent to the board whole when you apply it, because the board can't read
anyone's home folder. `process list` shows which projects use each process. When a project uses
someone else's personal process, it says the file is not in your library.

### When a saved process changes

Changing a saved process never changes the projects using it. They are told, and each one updates
when its user says so:

```bash
kanbanr process status            # which process this project uses, and whether it has moved on
kanbanr process diff              # what is different: statuses, moves, gates
kanbanr process update            # take the saved version now
```

- `kanbanr doctor` warns when a newer version is saved, or when the project's workflow was edited
  after it was applied. The same notice opens each Claude session, and Claude asks once whether
  to update.
- The monitor's Workflow page shows the process, its version and library, and any notice.
- Items already in progress meet the new gates on their next move. Approvals already given stay.
- An update that would remove a status still holding items is refused. Move the items, or rename
  the status with `kanbanr config rename-status`, first.
- A built-in process can change with a kanbanr release, and is reported the same way.
- A personal process is compared on the machine that has it: the board doesn't hold its file.

A saved file has a short header, which an older kanbanr ignores:

```yaml
process:
  name: our-process
  description: Design review before build; QA signs off
  version: 2
statuses: [...]
```
