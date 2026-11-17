# On-disk layout

## On-disk layout

The board is a git repository **beside** the code repository, not inside it — `<repo>.kanbanr` as
a sibling (FEAT-041). A board inside a checkout is one `git add -A` away from being committed; a
sibling cannot be.

```
<repo>.kanbanr/                         # a git repository (every write is a commit); no accounts/secrets
├── profiles/<name>.yaml                # the board's practice packs (FTR-033), beside processes/
└── projects/
    └── <project>/
        ├── config.yaml                 # statuses, default_state, transitions, displayed/no-op/terminal states, branch_pattern
        ├── activity/                   # the activity changelog, one file per day (FEAT-066)
        │   └── 2026-09-28.yaml          # {time, actor, message, item}, appended in order
        ├── events/                     # the notification event log, one file per day (FEAT-036, FEAT-066)
        │   └── 2026-09-28.yaml
        ├── tasklog/                    # every task state change, one file per day, never trimmed (FTR-061)
        │   └── 2026-11-17.yaml          # {at, item, todo, task, from, to, tokens?, cost_usd?, model?, build?}
        ├── gates/                      # recorded gate results, a new file per run, never rewritten (FTR-002)
        │   └── <name>/20261112T100000Z-<sha7>.yaml
        ├── prepared/                   # evidence prepared for a decision, each file written once (FTR-034)
        │   ├── <id>.yaml
        │   └── <id>.resolved.yaml
        ├── manual/                     # manual test results, one file per result, written once (DEF-040)
        │   └── <time>-<CODE>-<R-n>.yaml
        ├── pins/<CODE>/<rev>.md        # a spec as a design sign-off saw it, written once (FTR-023)
        ├── charter.yaml                # purpose, goals, non-goals, stakeholders, adopted_at (FEAT-046)
        ├── baseline.yaml               # the engineering baseline: packs followed, areas, entries, revisions (FTR-033)
        ├── packs/<name>.yaml           # the applied copy of each practice pack the project follows (FTR-033)
        ├── facts.yaml                  # the project's facts: languages, runtimes, production, parts, data (FTR-005)
        ├── catalog.yaml                # the content catalog: what the project must document, row by row (FTR-005)
        ├── lessons.yaml                # what was learned, with decaying confidence (FEAT-055)
        ├── mirror.yaml                 # GitHub issue mirror config, when enabled (FEAT-043)
        ├── scopes.yaml                 # agent scopes, oldest first, never trimmed (FEAT-214)
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

**How a file is written** (DEF-025). kanbanr never edits a board file in place. It writes the new
content to a temporary file in the same folder (`.<name>.<pid>.<n>.tmp`), syncs it, and renames it
over the old file, so a file is always whole: the old content or the new, even if kanbanr is killed
or the disk fills mid-write. An interrupted write can leave its `.tmp` file behind. The board's
`.gitignore` ignores `*.tmp`, nothing reads it as board data, and `kanbanr doctor` warns about one
older than an hour; it is safe to delete. A file's permissions are kept, and a symbolic link is
written through.

**Item codes** (FEAT-217). An item's file is named by its code. Where the project maps kinds to
prefixes (`codes:` in `config.yaml`, set with `kanbanr config codes`), a new item's code says its
kind — `DEF-001`, `MNT-004`, `FTR-012` — with a counter per prefix; otherwise every code is
`FEAT-<n>`. Both kinds of code live side by side, and a code is never renamed when its item's kind
changes.

```yaml
codes:
  default: FEAT        # optional; absent = FEAT
  kinds:               # kind (lowercase) -> prefix
    bug: DEF
    defect: DEF
    feature: FTR
```

**Agent scopes** (FEAT-214). `scopes.yaml` holds every scope issued for the project: `id`, `role`
(`design` or `build`), `item`, `operations`, `branch` and `base` (build), `issued_by`, `issued_at`,
`expires_at`, `note`, and once claimed `worktree` (relative to the main worktree, so the board
carries no home paths) and `claimed_at`; a revoked one adds `revoked_at`, `revoked_by` and
`revoked_reason`. Its state (issued, active, expired, revoked) is derived, never stored. The agent's
worktree holds only a pointer, `kanbanr-scope.yaml` with `{scope, project}`, in the worktree's own
git directory (`.git/worktrees/<name>/`): it cannot be committed, survives `git clean`, and goes
with the worktree. A task records when its state last changed (`changed_at`), which agent drift is
measured from.

**What a task records** (FTR-060). Beside `key`, `text`, `state` and `changed_at`, a task may
record, each absent when empty:
- `depends_on`: the tasks it waits on, as written after trimming: `T3` (its own list),
  `TL-002/T3` (another list of its item), `CODE/TL-001/T3` (another item), or
  `project:CODE/TL-001/T3` (another project); duplicates dropped, order kept;
- `writes` and `reads`: the files it writes and only reads, repository-relative and
  `/`-separated, with no `..`, no leading `/` or `./`, and a folder ending in `/`. A task with no
  `writes` has an unknown footprint: it runs alone within its item;
- `estimate_h`: its estimate in agent-hours, more than 0.

The project's `config.yaml` may carry `tasks: {append_only: [...]}`: the files every item appends
to, which never make two tasks conflict (absent, it reads as `["CHANGELOG.md"]`). A build scope in
`scopes.yaml` that covers one task records it as `task` (`TL-001/T3`), and its `branch` is the item
branch with `--tl-001-t3` added. Which scope works a task is read from `scopes.yaml`, never stored
on the task. A kanbanr older than 0.8.0 keeps these keys and writes them back unchanged; it reads
a task scope as covering the whole item, so install 0.8.0 before issuing one.

**Builds** (FTR-062). How agents' builds share the machine is the project's `build:` key in
`config.yaml`, absent until something is set (`kanbanr config build`); every field is optional
and falls back to its default: `slots`, `jobs_per_slot` (5), `reserve_cpus` (2), `reserve_mem_gib`
(6), `max_load` (vCPUs less the reserve), `sccache` (`auto` or `off`), `sccache_port` (derived from
the main checkout's path), `cache_gib` (10), `linker` (`default` or `mold`), `web_cache` (on),
`queue_batch` (4) and `line_check` (a command). A kanbanr older than 0.8.0 keeps the key as it is.
Everything else about builds stays off the board:

```
<machine>/                              # $KANBANR_BUILD_DIR, else $XDG_CACHE_HOME/kanbanr/build,
│                                       # ~/.cache/kanbanr/build (Linux), ~/Library/Caches/kanbanr/build
│                                       # (macOS), %LOCALAPPDATA%\kanbanr\build (Windows)
├── slots/slot-<n>.lock, slot-<n>.json  # a held lock is a busy slot; the .json names its holder
├── queue/<prio>-<arrival-ns>-<pid>.ticket   # a waiting build, locked by its owner (+ .json)
├── web/<key>/dist/                     # a built monitor, kept under its sources' key
└── sccache/<project>-<hash8>/          # the project's compiler cache
<git-common-dir>/kanbanr/               # inside the git directory, never committed
├── builds/<YYYY-MM-DD>.jsonl           # the build log: one row per run (wait, run, exit), never trimmed
├── queue.yaml, queue.lock              # the merge queue and the lock its runner holds
└── queue-logs/<MQ-n>.log               # each batch's check output, named by its first entry
```

**The task log and sprint capacity** (FTR-061). A task's own record keeps only its current `state`
and `changed_at`; `tasklog/` keeps every change, appended in the same write as the task, so the
change and its log entry land in one board commit. An entry is `at` (the `changed_at` written),
`item`, `todo`, `task`, `from` and `to`, and, when the agent reported them on `kanbanr task state`,
`tokens`, `cost_usd`, `model` and `build` (`{runs, red, run_ms, wait_ms}`, the task's rows of the
machine-local build log since it last went in progress). Setting a task to the state it already
has writes no entry. A task's time in progress, its rework (Completed back to an open state), its
build time and its cost are derived from these entries on every read, never stored; a malformed
day file is skipped. A kanbanr older than 0.8.0 ignores the folder (its `doctor` calls it stray,
a warning only).

A sprint in `sprints.yaml` can state its capacity for agent-built work as `lanes` (agents working
at once) and `build_slots` (builds at once), beside or instead of the day or point `capacity`;
both are absent when not set. A sprint also keeps keys it does not know. A kanbanr older than
0.8.0 drops `lanes` and `build_slots` when it rewrites `sprints.yaml` (`sprint edit`, `start`,
`close`), so set them with 0.8.0 installed.

**Where a verdict was taken** (DEF-028). Every approval event and standing approval, every
sign-off entry (given or withdrawn) and every release stage change carries an optional `origin`:
`monitor` for a click on the monitor, `conversation` for a command that ran inside a Claude Code
session (`CLAUDECODE` set), `terminal` for any other, and `delegated`, reserved. The front end
that received the verdict sets it; a value in the request is ignored. A release also records
`shipped_by` and `shipped_origin` when it is cut, and a decision records `decided_origin` in its
front matter beside `deciders` and `decided`. A record with no `origin` was written before this
existed and reads as unknown; it is not back-filled. The value is kept as a string, so an origin an
older kanbanr does not know survives its write. Release stage changes also keep keys they do not
know now; a kanbanr older than 0.6.0 that rewrites `releases.yaml` drops `origin` from them. An
origin is a label for review, not proof of who was at the keyboard.

**Side files, not new entities.** `charter.yaml`, `lessons.yaml` and `mirror.yaml` follow one
pattern: absent means "none", empty removes the file, and none of them is part of `Project` — so
`GET /projects/{p}` is unchanged by their presence and an older board loads without them.

**The facts and the catalog** (FTR-005) are side files too, absent by default. Both carry
`schema: 1` and keep keys they do not know. `facts.yaml` holds five lists, each trimmed,
lower-cased, sorted and deduplicated on save, and `set_at`, stamped on save:

```yaml
schema: 1
languages: [rust, typescript]          # vocabulary words, or other:<text>
runtimes: [native, browser]
production: [developer-machine]        # where the built thing runs
parts: [cli, daemon, web-ui, library]
data: [files]
set_at: 2026-11-20T09:12:00Z
```

`catalog.yaml` holds the rows, and the starter version they were taken from:

```yaml
schema: 1
starter: 1
rows:
  - id: context                 # ^[a-z0-9][a-z0-9-]*$, unique
    title: System context
    answers: [what, where]      # what | how | where | when | who | why
    for: [sponsor, architect]   # sponsor | user | architect | builder | operator
    level: system               # system | component | item
    notation: mermaid
    source:
      kind: written             # written (path) | generated (generator) | enforced (test)
      path: architecture/context.md   # a board doc, a folder ending "/", or repo:<path>
    protection: [docs_updated, coverage:catalog]
    when: {production: [server, container]}   # optional; absent = always applies
    # applies: false            # an override, with why
    # why: "a library has no deployment of its own"
```

What each row's state is (whether it applies, its source present, a stub or missing, its
protections in force) and the gaps are derived on every read and never written to either file.

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
