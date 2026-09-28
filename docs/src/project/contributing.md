# Contributing to kanbanr

Thanks for your interest! kanbanr is a **personal open-source project** — maintained best-effort,
friendly to contributors, no SLA. Small fixes and focused features are very welcome; please open an
issue before large changes so we can agree on the approach.

## What kanbanr is (so contributions fit the design)

One binary. `kanbanr` is the **only writer** — the CLI edits a git-backed `data/` folder directly
(driven by a Claude skill). `kanbanr serve` runs a **read-only** view daemon (localhost, no auth, no
accounts) over the same folder. Sharing is delegated to git remotes. Keep this split intact:

- **All writes go through `kanbanr-core`** (the engine: store · `dispatch` router · git · activity).
  The CLI and the daemon both call core; don't add a second write path.
- **The web UI and the `serve` daemon never mutate data.** They read and stream. New "actions" in
  the UI should be read-only (e.g. export).
- Data is **human-readable YAML/markdown**; no database.

See [Architecture overview](../architecture/overview.md) for the full architecture.

## Prerequisites

- **Rust** stable (`rustup`), with `rustfmt` and `clippy`.
- **Node** 22+ and npm (for the web SPA).
- A **C toolchain** for the vendored libgit2 + OpenSSL build: `cmake`, `make`, `perl`, and a C
  compiler (`gcc`/clang). On Debian/Ubuntu: `sudo apt-get install -y cmake make perl gcc`. On macOS:
  `brew install cmake` (perl ships with macOS). On Windows: cmake + Strawberry Perl on `PATH`.
  > The **first** build compiles vendored C from source and is slow (a few minutes); it's cached
  > afterward. This is expected, not a hang.

## Build, test, lint

From the repo root (the `Makefile` wraps the common flows):

```bash
make build          # cargo build --release (api/) + web build
make test           # cargo test --workspace (unit + Docker-less integration)
make itest          # packaging smoke: build the image + testcontainers test (needs Docker)
```

Run the same checks CI runs before opening a PR:

```bash
cd api && cargo fmt --all --check          # formatting
cd api && cargo clippy --workspace -- -D warnings   # lints (warnings are errors)
cd api && cargo test --workspace
cd web && npm ci && npm run build          # web type-check + build
```

Run the app locally to try your change:

```bash
make serve          # builds the SPA + serves the read-only monitor on http://localhost:8080
```

## Coding conventions

- **Rust:** `cargo fmt` + `clippy` clean (no warnings). Match the surrounding style; comments explain
  *why*, not *what*, at the density of the file you're editing.
- **TypeScript/React:** the UI is read-only and plain (`fetch` + `EventSource`); no state libraries.
  Keep components small and typed.
- Prefer a single `kanbanr batch` / `dispatch` route over bespoke one-off code paths.
- Update tests for behavior changes; `kanbanr-core` is where the engine tests live.
- Update the relevant docs (the [Using kanbanr](../using/everyday.md) and [Architecture](../architecture/overview.md) chapters,
  the [skill](skill/kanbanr/SKILL.md)) when you change a contract.

## The bar: say why, and show it works

kanbanr asks the same thing of a contribution that it asks of its own maintainer, because this
project's board is the demonstration that the method works — a change that skipped it would break
the demonstration. The bar is one sentence long:

> **State why the change exists, and prove each requirement with a test that actually passed.**

In practice, the PR template has a `Definition` block. Fill it in:

- **A statement** — what the change gives whom, and why. One sentence.
- **The six dimensions** — what / how / where / when / who / why, a line each.
- **Requirements** in [EARS](https://alistairmavin.com/ears/) form (`WHEN … THE SYSTEM SHALL …`),
  each with the test that proves it, named **exactly as your test runner prints it**.
- **Leave what you don't know blank.** A blank is reported and can be filled; an invented answer
  reads like rigour and is worse than nothing. If something is genuinely ambiguous, ask in the issue
  rather than guessing.

You do **not** need the board to do this. `kanbanr check --file definition.yaml` validates a
definition on its own, which is what CI runs on your PR — no board access required.

### A worked example

A real change from this project's own history, end to end. The defect: ticking an item's last task
auto-completed it without recording the status change, so cycle time was blind to the normal way
items finish.

```yaml
statement: "A recorded transition for every status change, however it happened, so that flow numbers
  describe the items that finished normally rather than only the ones moved by hand"
goals: [G-2]
zachman:
  what: "The history entry an auto-completion did not write"
  how: "The auto-complete branch appends the same Transition a manual move does"
  where: "kanbanr-core/src/store.rs, set_task_state_on"
  when: "Whenever the last open task of an item is completed"
  who: "Anyone reading cycle time, which was blind to the normal path"
  why: "Auto-completion is how most items reach a terminal status, so the measurement was missing
    precisely where it mattered"
requirements:
  - kind: functional
    text: "WHEN completing a task auto-completes its item, THE SYSTEM SHALL append the same
      transition a manual move records."
    tests:
      - name: tests::auto_completion_records_the_move_like_any_other
        kind: unit
        state: green
  - kind: functional
    text: "WHEN a status is renamed, THE SYSTEM SHALL leave the items' histories unchanged."
    tests:
      - name: tests::auto_completion_records_the_move_like_any_other
        kind: unit
        state: green
```

The commit that followed carried `Refs: kanbanr:FEAT-061/R-1` in its trailer, and the test named
above is the one that ran. That is the whole shape: a reason, a requirement, a test, a reference.

> **A note on this project's own board.** Items created before the method was adopted have no
> definition, and `kanbanr doctor` deliberately does not report them. The board reads as *adopted*,
> not abandoned — everything from that point on meets the bar above.

## Commits & pull requests

1. Fork, branch off `main` (`feature/short-name` or `fix/short-name`).
2. Keep commits focused; write clear messages (imperative mood: "Add …", "Fix …").
3. Reference what the change serves in the commit trailer: `Refs: kanbanr:FEAT-046/R-2` if you know
   the item, or describe it in the PR if you don't have the board.
4. Optionally sign off your commits (`git commit -s`) to certify the
   [Developer Certificate of Origin](https://developercertificate.org/).
5. Make sure `fmt` / `clippy` / `test` / web build are green, and that every requirement in your
   definition has a **green** test.
6. Open a PR; fill in the template, definition included; link any issue.

## Licensing of contributions

kanbanr is dual-licensed **MIT OR Apache-2.0** (see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE)). Unless you state otherwise, any contribution you intentionally
submit for inclusion is licensed under those same terms, per section 5 of the Apache-2.0 license,
with no additional terms or conditions.

## Reporting bugs / requesting features

Use the GitHub issue templates. For anything security-sensitive, **do not open a public issue** —
follow [SECURITY.md](SECURITY.md).
