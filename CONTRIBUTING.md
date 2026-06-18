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

See [docs/DESIGN.md](docs/DESIGN.md) for the full architecture.

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
- Update the relevant docs ([docs/USER_GUIDE.md](docs/USER_GUIDE.md), [docs/DESIGN.md](docs/DESIGN.md),
  the [skill](skill/kanbanr/SKILL.md)) when you change a contract.

## Commits & pull requests

1. Fork, branch off `main` (`feature/short-name` or `fix/short-name`).
2. Keep commits focused; write clear messages (imperative mood: "Add …", "Fix …").
3. Optionally sign off your commits (`git commit -s`) to certify the
   [Developer Certificate of Origin](https://developercertificate.org/).
4. Make sure `fmt` / `clippy` / `test` / web build are green.
5. Open a PR; fill in the template; link any issue. Describe what changed and how you verified it.

## Licensing of contributions

kanbanr is dual-licensed **MIT OR Apache-2.0** (see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE)). Unless you state otherwise, any contribution you intentionally
submit for inclusion is licensed under those same terms, per section 5 of the Apache-2.0 license,
with no additional terms or conditions.

## Reporting bugs / requesting features

Use the GitHub issue templates. For anything security-sensitive, **do not open a public issue** —
follow [SECURITY.md](SECURITY.md).
