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

**Agent scopes** (FEAT-214). `scopes.yaml` holds every scope issued for the project: `id`, `role`
(`design` or `build`), `item`, `operations`, `branch` and `base` (build), `issued_by`, `issued_at`,
`expires_at`, `note`, and once claimed `worktree` (relative to the main worktree, so the board
carries no home paths) and `claimed_at`; a revoked one adds `revoked_at`, `revoked_by` and
`revoked_reason`. Its state (issued, active, expired, revoked) is derived, never stored. The agent's
worktree holds only a pointer, `kanbanr-scope.yaml` with `{scope, project}`, in the worktree's own
git directory (`.git/worktrees/<name>/`): it cannot be committed, survives `git clean`, and goes
with the worktree. A task records when its state last changed (`changed_at`), which agent drift is
measured from.

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
