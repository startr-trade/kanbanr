# Configuring the workflow

## Configuring the workflow (via the CLI/skill)

```bash
kanbanr config show
kanbanr config set-transition Planned Scheduled --allow      # or --deny
kanbanr config displayed-states Planned,Scheduled,Completed  # which states the dashboard shows
kanbanr config default-state Planned                         # status assigned to new features
kanbanr config no-op-states "No Action,Not Applicable,Out-of-Scope"   # inert dispositions

# Reset / redefine the WHOLE workflow at once (instead of many set-transition calls):
kanbanr config workflow --statuses Backlog,Doing,Done,Dropped --transitions "Backlog>Doing,Doing>Done" \
                        --default-state Backlog --displayed-states Backlog,Doing,Done --no-op-states Dropped
kanbanr config workflow --defaults                           # restore the built-in default workflow
```

For a brand-new project, `kanbanr project init … --statuses … --default-state … --displayed-states …
--no-op-states …` sets everything at creation. `config workflow` resets/redefines it on an existing project.

A feature can only move along an allowed transition; kanbanr rejects anything else. When all of
a feature's tasks are Completed it auto-advances to a "Completed" status if that move is allowed.

**No-op states** are statuses flagged as functionally inert dispositions (new projects ship with
`No Action`, `Not Applicable`, `Out-of-Scope`). They are always **non-displayed** on the board, and
a feature parked in one does **not** auto-complete. Move a feature into one with
`kanbanr move FEAT-001 "Out-of-Scope"`. Non-displayed states (incl. no-op) still have status-page
links on the home tiles and the dashboard.


## Releases and the workflow

Merging an item into a release that is past its first stage and not yet cut sends the release back
to that stage, whatever the item's own workflow. The release then follows its steps again; see
[A change merged into a promoted release](processes.md#a-change-merged-into-a-promoted-release).

A release does not read as shipped before the workflow its tag started has passed. When the tag
stage is the last one and a commit gate judges the tag, the cut tags the release and leaves it
"tagged, not released yet"; its items stay where they are. Recording a passed result for the tag's
commit (a gate result recorded after the tag was made) ships it, as the person who recorded it, and
writes the release notes. A failed result keeps it tagged and the Releases page and
`kanbanr release promote --plan` name the patch release as the way on; a re-run that passes ships
it. A project with no commit gate has nothing to wait for and ships at the tag as before, and a
stage after the tag still confirms the release as it did.

## Permanence & deletion

- **Feature items can't be deleted** — they're the project's work. To retire one, `move` it to a
  no-op state, don't delete it.
- A **milestone** is removable only when no feature references it.
- A **project** is deletable (`kanbanr project delete <name>`) only when it has no features and no
  milestones — so any project with real work is locked from deletion.
