<!-- Thanks for contributing to kanbanr! Keep PRs focused; open an issue first for large changes. -->

## What & why

<!-- What does this change, and what problem does it solve? Link any issue: Closes #123 -->

## How verified

<!-- Commands you ran and what you observed. -->

- [ ] `cd api && cargo fmt --all --check`
- [ ] `cd api && cargo clippy --workspace -- -D warnings`
- [ ] `cd api && cargo test --workspace`
- [ ] `cd web && npm ci && npm run build` (if the web changed)
- [ ] Tried it locally (`make serve`) where applicable

## Design check

- [ ] Writes still go only through the CLI / `kanbanr-core` (the web/`serve` layer stays read-only).
- [ ] No accounts/auth added inside kanbanr (sharing remains a git-remote concern).
- [ ] Docs / skill / `CHANGELOG.md` updated if a contract or user-facing behavior changed.

## Notes for the reviewer

<!-- Anything you want a second look at. -->
