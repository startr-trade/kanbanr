# The monitor's navigation map

## Web navigation map

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
  Base["/p/:project/baseline  Baseline (packs, revisions to accept, areas, history)"]

  Home --> Board --> Status --> Feature
  Board --> Feature
  Board --> Ms --> MsOne --> Feature
  Board --> Sched --> MsOne
  Home --> Docs
  Board --> Base
```

## The meta route

`GET /api/meta` answers what the daemon can do (`writes`, `identity`, `schema_version`) and which
kanbanr it is: `version` and `commit` of the running build, `board_writer` (the record in the
board's `writer.yaml`, or null) and `writer_newer`, true when that writer is newer than the
running build (a higher version, or the same version with a commit that descends from the running
one in the code repository). The page header shows the build and, when `writer_newer`, a warning
(FTR-036). Every board write stamps `writer.yaml` in its own commit; the monitor never does.
