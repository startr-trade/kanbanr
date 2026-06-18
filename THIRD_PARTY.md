# Third-party notices

kanbanr is distributed under **MIT OR Apache-2.0** (see [LICENSE-MIT](LICENSE-MIT) /
[LICENSE-APACHE](LICENSE-APACHE)). It builds on third-party components under their own licenses.
This file is a human-readable summary; generate the authoritative, complete list with the tooling
noted at the bottom before each release.

## Statically linked C libraries (notable)

The Rust build **vendors and statically links** these from source — so the shipped binary contains
them, which makes their licenses relevant to redistribution:

- **libgit2** — license: **GPL-2.0 WITH the GCC/Git linking exception**. The linking exception
  explicitly permits linking libgit2 into a binary under a different license (here MIT/Apache-2.0)
  and distributing that binary, without the combined work becoming GPL. We do not modify libgit2;
  we link the vendored sources via the `git2`/`libgit2-sys` crates.
- **OpenSSL** — license: **Apache-2.0** (OpenSSL 3.x). Vendored via `openssl-sys`
  (`vendored-openssl`). Permissive; compatible with redistribution under MIT/Apache-2.0.

If you build with system libgit2/OpenSSL instead of the vendored copies, these notices may not
apply to your build.

## Rust crates (api/)

The CLI/engine/daemon depend on the Rust crate ecosystem. The overwhelming majority are
**MIT OR Apache-2.0** (or MIT). Notable direct dependencies include: `axum`, `tokio`, `serde`,
`serde_yaml`, `serde_json`, `anyhow`, `thiserror`, `time`, `clap`, `git2`, `notify`, `ureq`, and
`tower`/`tower-http`. See each crate's repository for its exact license text; the generated report
(below) is authoritative.

## Web dependencies (web/)

The read-only monitor is a React + Vite app. Runtime dependencies — `react`, `react-dom`,
`react-router-dom`, and `marked` — are **MIT**-licensed. Build tooling (`vite`, `typescript`,
`@vitejs/plugin-react`, type stubs) is MIT/Apache-2.0.

## Generating the authoritative list

Before a release, regenerate a complete, machine-verified inventory:

```bash
# Rust: license inventory + policy check
cargo install cargo-about cargo-deny
cd api && cargo about generate about.hbs > ../THIRD_PARTY_RUST.html   # full attributions
cd api && cargo deny check licenses                                   # fail on disallowed licenses

# Web: dependency license summary
cd web && npx license-checker --summary
```

Keep `cargo deny check licenses` in CI so a newly-pulled, incompatibly-licensed dependency fails the
build rather than shipping silently.
