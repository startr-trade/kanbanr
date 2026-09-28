# Evidence and measurement

## Measuring what happened

```bash
kanbanr report --since 14d        # throughput, cycle time, rework, escape rate, coverage
kanbanr retro MS-006 --write      # a wave's account, written to a document
kanbanr retro --due               # finished waves whose retro is unwritten
kanbanr defect FEAT-042 --introduced-by FEAT-031 --found-in production --severity high
kanbanr lessons [--for FEAT-001]  # what this project learned, most believed first
kanbanr lesson add "…" --kind pitfall --from FEAT-043 --evidence "what actually happened"
kanbanr lesson affirm L-1 | kanbanr lesson contradict L-1 --note "…"
```

Every number is derived from what the board recorded — status history, defect records, test states
— and anything that cannot be derived is **absent rather than estimated**. Whether a defect
*escaped* is not asked, it is derived: it escaped if the work that introduced it had already been
called done. Lessons lose confidence with age unless something reaffirms them, and one that falls
below the threshold retires: kept as a record, no longer surfaced.


## Tests

- `make test` — Rust unit tests + the Docker-less integration tests: the real CLI writes a local
  data dir, and `kanbanr serve` serves it read-only over a port.
- `make itest` — builds the Docker image and runs the **testcontainers** smoke test (the image
  boots and serves the read-only view, no auth).
