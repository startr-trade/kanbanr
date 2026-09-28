# kanbanr — Design

Architecture, data model, on-disk layout, security, and runtime flows. Diagrams use
[Mermaid](https://mermaid.js.org/) (rendered by GitHub and VS Code).

## 1. Goals & principles

- **YAML/markdown files only** — no database; human-readable and version-controllable.
- **The CLI is the single writer** — it edits the local data folder **directly** (store +
  `dispatch` + git). There is no server in the write path, no login, no accounts.
- **The view is a read-only, localhost monitor** — `kanbanr serve` serves the read API + live SSE +
  the SPA over the same folder. No auth (expose beyond localhost only behind a reverse proxy). The
  viewer is pluggable (today a React SPA; a VS Code extension is on the roadmap).
- **Git is the sharing/centralization boundary** — the data folder is a git repo; every write is a
  commit by the configured identity (`kanbanr identity`); optional remotes are pulled + pushed
  after each commit. **Merge conflicts are left for normal git resolution** — kanbanr never
  auto-resolves.
- **One runtime, no Docker required** — a single `kanbanr` binary is both the writer and (via
  `kanbanr serve`) the view daemon. Docker is just one optional packaging.
- **On-disk layout mirrors the board** — feature items live in a folder named after their status.
- **Multi-project** — one data dir with a folder per project.

## 2. Components

```mermaid
flowchart LR
  subgraph dev["Developer in VS Code"]
    Claude["Claude + kanbanr skill"]
  end
  CLI["kanbanr (CLI)<br/>local writer + 'serve'"]
  Core["kanbanr-core<br/>store · dispatch · git · activity · export"]
  FS[("data/ — a git repo<br/>YAML + markdown")]
  Daemon["kanbanr serve<br/>view daemon (read API + SSE + SPA)"]
  Browser["React monitor (view-only)<br/>· future: VS Code ext / TUI"]

  Claude -->|"runs commands"| CLI
  CLI --> Core
  Core <-->|"read/write (the writer)"| FS
  CLI -->|"commit + pull/push"| Remote[("git remote<br/>(sharing)")]
  Daemon --> Core
  FS -. "file change events" .-> Daemon
  Daemon -->|"JSON + SSE /api/*"| Browser
  Daemon -->|"serves built SPA"| Browser
```

`kanbanr-core` is the engine (models, store, validation, git, the activity changelog, and a
`dispatch` router that maps `(method, path, body)` onto store calls — the single source of truth
for data operations). The **CLI** links it to write the folder directly; the **view daemon**
(`kanbanr-server`, a library run by `kanbanr serve`) links it to read. Both go through `dispatch`,
so they agree by construction. It's all one binary.

## 3. Runtime: one writer, a read-only view, git

```mermaid
sequenceDiagram
  participant C as Claude (skill)
  participant K as kanbanr (CLI, writer)
  participant R as board repo (sibling)
  participant D as kanbanr serve (view)
  participant B as Browser (monitor)

  B->>D: GET /api/projects/:p   (no auth)
  B->>D: open SSE /api/projects/:p/events
  C->>K: edit (add feature / move / task …)
  K->>R: dispatch → write YAML/md, append today's activity file, git commit
  K->>R: pull + push remotes — best-effort, conflicts left for normal git
  R-->>D: file change event (notify)
  D-->>B: SSE changed event → refetch live
```

- **No accounts, no JWT.** The writer is you on your machine; the view is a localhost read-only
  window onto your files. **Access control for sharing is the git remote's job** (e.g. GitHub
  decides who can pull/push the data repo).
- **Identity** for commits comes from the data repo's git config (`kanbanr identity --name --email`),
  the same in every write.
- **Git** uses `git2`/libgit2, statically linked — no external `git` binary. Sync is best-effort and
  never blocks a write; a conflict aborts the merge cleanly and is resolved with normal git in the
  folder.
- **Exposure.** `kanbanr serve` binds `127.0.0.1` by default and has no TLS/auth by design. To
  reach it from another machine, leave the bind alone and put it behind a reverse proxy that
  terminates TLS and adds auth (Caddy/nginx examples in
  [USER_GUIDE.md](USER_GUIDE.md#exposing-the-monitor-beyond-localhost)).

## 4. Data model

```mermaid
erDiagram
  PROJECT ||--o{ MILESTONE : has
  PROJECT ||--o{ FEATURE : has
  PROJECT ||--o{ DOC_FOLDER : has
  MILESTONE ||--o{ MILESTONE : "depends_on (DAG)"
  MILESTONE ||--o{ FEATURE : "groups (required)"
  FEATURE ||--o{ TODO_LIST : "todo-lists (epic)"
  TODO_LIST ||--o{ TASK : tasks
  DOC_FOLDER ||--o{ DOC_FOLDER : "nested"
  DOC_FOLDER ||--o{ DOC_FILE : contains

  PROJECT {
    string name
    string description
    list statuses
    string default_state
    map transitions
    list displayed_states
    list no_op_states
  }
  FEATURE {
    string code
    string title
    string status
    string milestone
    string specification
  }
  TODO_LIST {
    string code
    string description
    string created_at
  }
  TASK {
    string key
    string text
    enum state
  }
  MILESTONE {
    string code
    string name
    list depends_on
  }
  DOC_FOLDER {
    string path
    string name
    string description
  }
  DOC_FILE {
    string path
    string title
  }
```

- **Status** is a configurable label; **transitions** is a map `from → [allowed to]`;
  `displayed_states` is the ordered subset the dashboard shows; `default_state` is the status a
  new feature starts in. **`no_op_states`** flags statuses that are inert dispositions (e.g.
  "No Action", "Not Applicable", "Out-of-Scope"): always non-displayed (kept disjoint from
  `displayed_states`), and a feature in a no-op state does **not** auto-advance to Completed.
- **Permanence / referential integrity:** feature items are **never deletable** (the work is
  permanent). A milestone can be deleted only when unreferenced; a status removed only when no
  feature is in it; a project deleted only when it has no features and no milestones — so any
  project with work is locked from deletion.
- A feature's **milestone is required**. A feature acts as an **epic**: it holds many persistent
  **todo-lists** (one added per work session), each with its own tasks (keys unique per list).
  `TASK.state` is `NotStarted | InProgress | Completed`; when every task across ALL the feature's
  todo-lists is Completed, the feature auto-advances to a "Completed" status when allowed.
  Todo-lists are displayed newest-first; the status page shows only the not-fully-completed ones.
- **There is no Schedule entity.** A "schedule" is derived: for a status, the milestones holding
  features in that status (dependency-ordered).

## 5. On-disk layout

The board is a git repository **beside** the code repository, not inside it — `<repo>.kanbanr` as
a sibling (FEAT-041). A board inside a checkout is one `git add -A` away from being committed; a
sibling cannot be.

```
<repo>.kanbanr/                         # a git repository (every write is a commit); no accounts/secrets
└── projects/
    └── <project>/
        ├── config.yaml                 # statuses, default_state, transitions, displayed/no-op/terminal states, branch_pattern
        ├── activity/                   # the activity changelog, one file per day (FEAT-066)
        │   └── 2026-09-28.yaml          # {time, actor, message, item}, appended in order
        ├── events/                     # the notification event log, one file per day (FEAT-036, FEAT-066)
        │   └── 2026-09-28.yaml
        ├── charter.yaml                # purpose, goals, non-goals, stakeholders, adopted_at (FEAT-046)
        ├── lessons.yaml                # what was learned, with decaying confidence (FEAT-055)
        ├── mirror.yaml                 # GitHub issue mirror config, when enabled (FEAT-043)
        ├── index.yaml                  # derived cache of item metadata, rebuildable (FEAT-033)
        ├── features/                   # every item, filed under its status (FEAT-071)
        │   └── <Status>/                # e.g. Planned/  In Progress/  Completed/
        │       ├── FEAT-001.yaml         # item METADATA only — including definition, defect, history
        │       └── features-spec/
        │           └── FEAT-001.md       # the specification markdown
        ├── milestones/MS-001.yaml       # code, name, description, depends_on[]
        └── docs/                        # documentation tree (markdown)
            ├── decisions/               # ADRs: front-matter + prose (FEAT-057)
            ├── retros/                  # wave retrospectives (FEAT-054)
            └── design/
                ├── _folder.yaml         # folder name + short description
                └── overview.md
```

**Side files, not new entities.** `charter.yaml`, `lessons.yaml` and `mirror.yaml` follow one
pattern: absent means "none", empty removes the file, and none of them is part of `Project` — so
`GET /projects/{p}` is unchanged by their presence and an older board loads without them.

**Everything else on an item is inline.** `definition`, `defect`, `split_from` and `history` live
in the item's own yaml, because they then ride move/rename/index/flush for free and every consumer
(doctor, export, query, report, trace) wants them anyway. Each new field is optional and
`skip_serializing_if` its empty value, so a board written by an older version **re-serializes byte
for byte** (ADR-0005). Adding one means touching exactly three places — the `FeatureItem` struct,
`meta()` and `from_meta()` — which the compiler enforces.

Changing a feature's status **moves both** its `<Status>/<code>.yaml` and
`<Status>/features-spec/<code>.md` into the new status folder, and appends a transition to the
item's `history[]` — including when ticking the last task auto-completes it, which is how most
items actually finish. All writes are done by the CLI (the single writer). There is **no**
`security.yaml` — kanbanr has no accounts (ADR-0001).

## 6. Activity changelog

Each write appends an entry to `projects/<id>/activity/<YYYY-MM-DD>.yaml` — `{time, actor, message,
item}`, where actor is the commit identity and message a short description of the change. Events
(FEAT-036) are stored the same way under `events/`.

**Nothing is ever trimmed** (FEAT-066). An earlier version kept one capped file, which meant the log
quietly discarded its own oldest entries — and a retrospective reading a wave that had aged out
reported "no recorded moves", which reads as *nothing happened* rather than *the evidence was
deleted*. One file per day instead: a read walks the days newest-first and stops once it has enough,
so bounding the read costs nothing while the raw record stays complete. Summaries are derived from
it; it is never derived from them.

The view daemon serves it at `GET /api/projects/:p/activity`, and the monitor renders a "Recent
activity" panel. It is plain data in the folder (no git plumbing needed to read it) and works the
same with or without a remote. (The full audit trail still lives in git history.)

## 7. Web navigation map

```mermaid
flowchart TD
  Home["/  Home — project tiles (dashboard + status + docs links)"]
  Board["/p/:project  Board (columns = displayed states)"]
  Status["/p/:project/state/:state  Status page (grouped by feature → tasks)"]
  Feature["/p/:project/feature/:code  Feature page (Specification + Tasks + export)"]
  Ms["/p/:project/milestones  Milestones list"]
  MsOne["/p/:project/milestone/:code  Milestone (depends-on + feature tiles)"]
  Sched["/p/:project/schedule  Schedule (derived: milestones per status)"]
  Docs["/p/:project/docs  Documentation tree"]

  Home --> Board --> Status --> Feature
  Board --> Feature
  Board --> Ms --> MsOne --> Feature
  Board --> Sched --> MsOne
  Home --> Docs
```

## 8. HTTP API (the view daemon — read-only, no auth)

The daemon serves **only reads** (writes happen in the CLI, not over HTTP). There is no auth; it
binds localhost by default.

| Method & path | Purpose |
|---|---|
| `GET /healthz` | liveness/readiness probe |
| `GET /api/projects` | project summaries (home tiles) |
| `GET /api/projects/:p` | full project (config, features, milestones) |
| `GET /api/projects/:p/features/:code/export?format=md\|json` | Claude-ready export |
| `GET /api/projects/:p/docs` · `…/docs/content?path=` | docs tree / a doc's markdown |
| `GET /api/projects/:p/activity` | recent activity (the changelog) |
| `GET /api/projects/:p/events` · `GET /api/events` | SSE change streams |

### Writes happen in the CLI (`dispatch`)

All mutations go through `kanbanr-core::dispatch` from the CLI (and the same `(method, path,
body)` shapes the daemon would *read*): create/edit projects, features (never deletable), tasks,
todo-lists, milestones, config, docs — plus `POST /projects/:p/batch` for a bundle in one commit.
Each write appends to the activity log and commits the data repo. The **batch** body is
`{ "operations": [ { "op": "...", ... } ] }` with op types `feature.add`, `feature.edit`,
`feature.move`, `milestone.add`, `todo.add`, `task.add`, `task.state`, `doc.folder`, `doc.write`;
created items may carry a `ref` alias later ops reference; the first failure names its index.

## 9. Workflow contract (how Claude uses it)

kanbanr is designed to be the **single system of record** for a project's activity — the only
project information that stays outside it is the raw conversation transcript. The skill
(`skill/kanbanr/SKILL.md`) encodes the behavioral contract: adopt kanbanr for the whole project
on one trigger phrase ("start using kanbanr for this project"); never track work in an
ephemeral/in-session list (use persistent todo-lists on feature items); recover/resume state from
kanbanr at the start of each session; keep it updated before and after every task; review a
feature's spec for staleness when moving it out of `Deferred`; never delete feature items (move
them, e.g. to a no-op state); model every kind of work as a work item (feature/chore/recurring,
ongoing items in a non-displayed status); and prefer one `batch` call to bundle changes. The
durable, file-backed data model makes the project resumable across sessions by design.

## 11. Reasoning, evidence and traceability (MS-006)

The board above records *what* is being built. This layer records **why it exists, what must be
true, and what proves it** — and, deliberately, records nothing it cannot derive.

### The graph

```
charter.purpose
  └── G-2  goal
       └── FEAT-046  item              definition.goals: [G-2]
            ├── R-2  requirement        (EARS text, ISO tag, measured scenario)
            │    └── test               (planned → red → green, stamped with checked_rev)
            ├── FEAT-058  defect        violates: FEAT-046/R-2   introduced_by: FEAT-046
            ├── design/mirror.md        refs: [FEAT-046, R-2, G-2]
            ├── ADR-0003                affects: [FEAT-046]  driven_by: [FEAT-046/R-2]
            └── code / commits          // … (FEAT-046 R-2)  ·  Refs: kanbanr:FEAT-046/R-2
```

Three rules keep it from rotting:

1. **The manifest is canonical** — board yaml, document front-matter, and the commit trailer. An id
   that does not resolve is an *error*, like a dangling dependency.
2. **Inline annotations are a derived convenience** — `(FEAT-046 R-2)` on the module or function
   that owns the behaviour. They survive the refactors that destroy `git blame`; they are never the
   only record.
3. **Views are derived, never stored** (ADR-0002). `kanbanr trace --json` generates the
   traceability manifest for CI or an audit; nothing writes it back. A stored manifest would be a
   third copy of links that already exist, and the copy that goes stale first.

### Modules

| Module | What it owns |
|---|---|
| `charter` | the project's purpose and goals — a side file, absent by default |
| `ears` | the five EARS patterns and the nine ISO/IEC 25010 characteristics; classifies, never rejects |
| `doctor` | every structural check, all warnings except broken references; scoped so it stays readable |
| `report` | flow and quality derived from history, defects and test states |
| `retro` | a wave's account, with changelog-derived spans reported apart from measured ones |
| `lessons` | what was learned, with confidence that decays unless reaffirmed |
| `scm` | the trailer grammar, branch naming and reference validation |
| `adr` | decisions as documents with front-matter; supersede is the one two-sided link |
| `trace` | the downward chain and its gaps, and the derived Zachman view |
| `query`, `graph`, `gantt`, `portfolio`, `mirror`, `eventing` | search, dependencies, scheduling, rollups, the GitHub mirror, notifications |
| `validate`, `hash`, `error`, `docs` | id generation, stable hashing, error taxonomy, the doc tree |

### Two deliberate asymmetries

**Approval is pinned to content, not to time.** `definition.approval.rev` is a stable hash of the
definition with the approval itself *and the test states* excluded — recording evidence must not
lapse an approval, but changing scope must. Editing the definition after a yes therefore reports
"approval lapsed", which is a different thing from "never approved".

**The gates refuse two things and warn about everything else.** A reference that names something
the board does not have, and a commit that names nothing, are refused — both fixable in the message
the author is already writing. Everything else warns, because a check that blocks legitimate work
gets bypassed wholesale (ADR-0004), and a check that fires on everything gets ignored.

## 10. Testing

Testing is **two-layered**:

- **Unit tests** (`kanbanr-core`): the `dispatch` router, transition validation, code generation,
  milestone DAG cycle detection, all-tasks-done auto-complete (across todo-lists), status-folder
  file moves, milestone-required.
- **Layer 1 — functional integration** (`api/crates/kanbanr-cli/tests/`): the real coverage, no
  Docker. `local_mode.rs` drives the real **`kanbanr` CLI** writing an **ephemeral data dir under
  the build output (`CARGO_TARGET_TMPDIR`)** — create/move/todo/task, auto-complete, init,
  identity, and that the data dir is a git repo committed under the configured identity.
  `view_daemon.rs` then runs **`kanbanr serve`** over that folder and asserts the read API + the
  activity endpoint serve it **with no auth**.
- **Layer 2 — packaging smoke** (`api/crates/kanbanr-cli/tests/integration.rs`): uses
  **testcontainers** to confirm the real Docker **image** boots, scaffolds a project, and serves
  the read-only view (no auth). It does not re-test business logic (that's layer 1). `#[ignore]`d
  (needs Docker + image); run with `make itest`.
