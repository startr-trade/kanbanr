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


## Keeping the architecture views whole

`kanbanr doctor` lists each coverage rule a view still fails — `coverage rule 'zachman' (every
item at Design or later is referenced from architecture/zachman.md): 24 of 60 uncovered —
FEAT-012, …` — with the first ten subjects. A rule whose source doc is gone, an invalid rule in
a hand-edited config, a gate naming a rule the project does not configure, and a gate's default
doc that is not on the board are errors. Run it before merging a sprint or cutting a release;
[Coverage rules](living-docs.md#coverage-rules-keeping-the-views-whole) says how to set them.

## Troubleshooting

- **`monitor not reachable` from `kanbanr open`** — start the view daemon first:
  `kanbanr serve`. The monitor is built into the binary, so there is no `--ui-dir` to find.
- **`no commit identity for this board`** — kanbanr commits only as a real person. Set yours:
  `kanbanr identity --name "You" --email you@example.com` (or git's own `user.name` and
  `user.email`). Boards made by older versions carry the placeholder `kanbanr <kanbanr@local>`
  in their git config; `kanbanr doctor` points it out, and the same command replaces it. Commits
  already made under the placeholder keep it unless you rewrite the board's history.
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

See **[the architecture overview](../architecture/overview.md)** and **[the on-disk layout](../reference/data-layout.md)**.
