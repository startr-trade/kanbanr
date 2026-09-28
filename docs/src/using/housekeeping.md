# Housekeeping and troubleshooting

## Housekeeping on the board repository

The board is a git repository and every write is a commit, so an active project accumulates objects.
Nothing breaks if you ignore this — git is designed for it — but two things are worth knowing.

```bash
cd "$(kanbanr where)"
git count-objects -vH      # loose objects and pack size
git gc                     # pack them; safe, and never touches your data
```

A board with a few hundred commits and no pack can hold a few thousand loose objects. `git gc`
collapses that. It compacts storage and changes nothing about content — and it is **not** a way to
reclaim anything: the logs keep every entry deliberately (one file per day under `activity/` and
`events/`), because raw data is never discarded. What is bounded is what a *report* shows you.

If you have a remote configured, an occasional `kanbanr sync` keeps the board pushed; `kanbanr
where --json` tells you which folder is in use and why.


## Troubleshooting

- **`monitor not reachable` from `kanbanr open`** — start the view daemon first:
  `kanbanr serve --ui-dir web/dist` (or `make docker-up`).
- **Commits authored as `kanbanr <kanbanr@local>`** — set your identity:
  `kanbanr identity --name "You" --email you@example.com`.
- **`could not determine project`** — pass `--project`, set `$KANBANR_PROJECT`, or
  `kanbanr project use <name>` (writes a `.kanbanr` marker).
- **`project '…' not found` / an empty board** — you may be pointed at the wrong data folder.
  `kanbanr where --json` shows which folder is in use and why (`--data-dir`, `$KANBANR_DATA_DIR`,
  the marker, or the `./data` fallback).
- **Push/pull conflicts** — the data folder is a normal git repo; resolve in it with `git` as
  usual, then continue.
- **`kanbanr: command not found`** — run `make install` and ensure `~/.cargo/bin` is on PATH.
- **A move was rejected** — the transition isn't allowed; check `kanbanr config show`.
- **A feature add was rejected** — every feature needs an existing `--milestone`.

See **[DESIGN.md](DESIGN.md)** for architecture and on-disk layout.
