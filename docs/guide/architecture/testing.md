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
- **Deployment** (`deploy/tests/`): the hosted site ([Hosting a read-only
  monitor](../using/hosting.md)). Static checks: `check_tfvars.py` and its tests keep secrets and
  state out of git; `test_sync_sh.py` and `test_serve_sh.py` run the sync and serve scripts against
  bare remotes and a stub `kanbanr` in the temp dir; `validate.sh` validates the module, the example
  root and the e2e root under both OpenTofu and Terraform (from pinned images, through `tf.sh`) and
  runs the module's plan tests against a mock provider; `compose-config.sh` validates the compose
  file in both modes. Heavy checks: `e2e-kind.sh` applies a two-board site to a throwaway kind
  cluster, with the boards' remotes served inside it, and checks that both boards are served from a
  read-only mount by a non-root user, that the host check holds, and that a pushed change is listed
  and refreshes the page within one pull interval; `compose-smoke.sh` runs compose mode (b) over a
  local board and waits for it to report healthy. Each script prints `ok: <name>` per case, the
  names the item's tests are recorded under. The heavy checks need Docker, kind and kubectl, never
  gate a pull request, and run in `make ci` before a release.
