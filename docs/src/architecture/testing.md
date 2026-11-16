# Testing

## Testing

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
- **Build provenance** (`api/crates/kanbanr-cli/tests/build_stamp.rs`): `#[ignore]`d; clones the
  repository into a scratch folder, adds a linked worktree, checks the CLI twice around an empty
  commit and reads the stamp the build script emitted. Run by `make ci` and the Linux CI leg as the
  step 'Build stamp follows HEAD in a worktree'.
- **Layer 2 — packaging smoke** (`api/crates/kanbanr-cli/tests/integration.rs`): uses
  the `docker` CLI (driven directly, no test-container crate) to confirm the real Docker **image** boots, scaffolds a project, and serves
  the read-only view (no auth). It does not re-test business logic (that's layer 1). `#[ignore]`d
  (needs Docker + image); run with `make itest`.
