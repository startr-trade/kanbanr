# On-disk layout

## On-disk layout

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
        ├── gates/                      # recorded gate results, a new file per run, never rewritten (FTR-002)
        │   └── <name>/20261112T100000Z-<sha7>.yaml
        ├── prepared/                   # evidence prepared for a decision, each file written once (FTR-034)
        │   ├── <id>.yaml
        │   └── <id>.resolved.yaml
        ├── manual/                     # manual test results, one file per result, written once (DEF-040)
        │   └── <time>-<CODE>-<R-n>.yaml
        ├── pins/<CODE>/<rev>.md        # a spec as a design sign-off saw it, written once (FTR-023)
        ├── charter.yaml                # purpose, goals, non-goals, stakeholders, adopted_at (FEAT-046)
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
