<!-- Thanks for contributing to kanbanr! Keep PRs focused; open an issue first for large changes. -->

## What & why

<!-- What does this change, and what problem does it solve? Link any issue: Closes #123 -->

## Definition

<!--
Paste the item's definition here. If you have a kanbanr board, `kanbanr feature show <CODE>`
prints it; if you don't, fill this in by hand — it is the same bar either way, and it is what CI
validates with `kanbanr check --file`.

Leave anything you genuinely don't know BLANK. A blank is reported; an invented answer is not.
-->

```yaml
statement: "<capability> for <whom> so that <why>"
goals: []                 # charter goal ids this serves, if the project has a charter
zachman:
  what: ""
  how: ""
  where: ""
  when: ""
  who: ""
  why: ""
requirements:
  - kind: functional      # functional | nfr
    text: "WHEN <trigger>, THE SYSTEM SHALL <response>."
    tests:
      - name: "<test name exactly as your runner prints it>"
        kind: unit
        state: green
```

## How verified

<!-- Commands you ran and what you observed. -->

- [ ] `cd api && cargo fmt --all --check`
- [ ] `cd api && cargo clippy --workspace -- -D warnings`
- [ ] `cd api && cargo test --workspace`
- [ ] `cd web && npm ci && npm run build` (if the web changed)
- [ ] Tried it locally (`make serve`) where applicable

## Design check

- [ ] Every requirement above has a test, and every test is **green** — evidence, not intent.
- [ ] Anything I did not know is blank rather than guessed.

- [ ] Writes still go only through the CLI / `kanbanr-core` (the web/`serve` layer stays read-only).
- [ ] No accounts/auth added inside kanbanr (sharing remains a git-remote concern).
- [ ] Docs / skill / `CHANGELOG.md` updated if a contract or user-facing behavior changed.

## Notes for the reviewer

<!-- Anything you want a second look at. -->
