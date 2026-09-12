# kanbanr enforcement hooks

Sample [Claude Code hooks](https://docs.claude.com/en/docs/claude-code/hooks)
that make the "always go through kanbanr" behavior **reliable** instead of
merely hoped-for. (Roadmap item **FEAT-017**.)

## Why hooks (vs the skill)

`skill/kanbanr/SKILL.md` tells Claude to treat kanbanr as the project's single
system of record: recover state from it at the start of every session, and
update it before and after every task. But a **skill is a soft prompt** — it is
advice the model can forget, skip, or deprioritize under load.

**Hooks are different: the Claude Code harness runs them deterministically.**
The model does not decide whether they run — the harness does, every time the
event fires. That turns the SKILL.md contract from "please remember to" into
something the environment actually enforces:

- **`SessionStart`** deterministically injects the kanbanr board into context,
  so recovery happens whether or not the model remembers to ask.
- **`Stop`** deterministically checks, after each turn, whether kanbanr was
  updated, and nudges if it looks stale.

The model can ignore a sentence in a prompt. It cannot stop the harness from
running these scripts.

## Altitude — what kanbanr tracks, and what it does NOT

Keep the boundary crisp:

- **kanbanr tracks the PLAN**: feature/work items, their specs, milestones,
  todo-lists and task states, decisions, progress, and documentation about the
  project. This lives in kanbanr's own **data folder**: a separate git repo,
  usually next to the project (`../<project>.kanbanr`, recorded in the
  `.kanbanr` marker), or `$KANBANR_DATA_DIR`, or legacy `./data`.
- **kanbanr does NOT hold the code.** The actual source you build lives in the
  **project's own git repository**, entirely separate from the kanbanr data
  repo. These hooks reflect that: they read/inspect the kanbanr *data* repo for
  plan state, and never touch or reason about the project's code repo.

So: "did we update kanbanr?" means "did the **plan** get recorded?", not "did
we commit code?". A session can produce lots of code commits in the project
repo while leaving the plan stale in kanbanr — which is exactly the failure the
`Stop` hook is trying to catch.

## What each hook does

### `session-start.sh` — recover state (SessionStart)
Best-effort recovery. If `kanbanr` is on `PATH` **and** the directory looks
tracked (a `.kanbanr` marker exists, or `$KANBANR_PROJECT` is set), it prints
`kanbanr board` and `kanbanr activity` to stdout. Claude Code surfaces that as
session context, so Claude resumes from the real plan instead of from memory.
If kanbanr isn't installed or no project is resolvable, it exits 0 and stays
quiet — it will **never** break a session.

### `stop-check.sh` — nudge to record work (Stop)
Advisory only. After Claude finishes a turn, it checks whether the kanbanr
**data git repo** has a commit within a recent window (default 30 min;
override via `KANBANR_STOP_WINDOW_MIN`). If the latest commit is older than the
window, it prints a reminder **to stderr** to record the session's work in
kanbanr (features/tasks/specs/decisions/docs). It **warns, never blocks** —
it always exits 0.

> **Honest caveat:** the Stop check is a **best-effort recency heuristic**, not
> proof. It can't see your code repo or the transcript, so it can't truly tell
> whether *substantive* work happened — only whether a kanbanr commit is
> recent. Expect occasional false reminders (ignore them if the plan is already
> current) and occasional silence. It is intentionally cheap and non-blocking
> for exactly this reason. The limitations are documented in detail in the
> comments at the top of `stop-check.sh`.

## Install

1. Make the scripts executable:
   ```bash
   chmod +x session-start.sh stop-check.sh
   ```
2. Register them in your Claude Code settings. Open `~/.claude/settings.json`
   (user-global) or a project's `.claude/settings.json`, and **merge** the
   `hooks` object from [`settings.snippet.json`](./settings.snippet.json) into
   it. If you already have `SessionStart`/`Stop` hooks, add these entries to
   those arrays rather than overwriting them.
3. Edit the `command` paths in the snippet to the **absolute path** of these
   scripts on your machine (they point at this repo's `skill/kanbanr/hooks/`
   by default). Strip the `_comment*` keys before saving if you want a minimal,
   strictly-clean settings file.
4. Start a new Claude Code session in a kanbanr-tracked directory (one with a
   `.kanbanr` marker, or with `$KANBANR_PROJECT` exported). You should see the
   board appear as recovered context at session start.

### Configuration knobs (env vars)
- `KANBANR_PROJECT` — names the active project; also serves as the "this dir is
  tracked" signal both hooks look for (alongside the `.kanbanr` marker).
- `KANBANR_DATA_DIR` — where the kanbanr data git repo lives. Normally unset:
  the `.kanbanr` marker's `data_dir` says where the board is. `stop-check.sh`
  asks `kanbanr where` for the folder and inspects that repo's commit recency.
- `KANBANR_STOP_WINDOW_MIN` — how many minutes count as "updated this session"
  for the Stop nudge (default `30`).

## Safety design

Both scripts are written to be **safe to fail silently**. They do not use
`set -e`; they guard for missing commands (`kanbanr`, `git`, `date`), a missing
or non-git data dir, and an untracked directory; and on **any** uncertainty
they `exit 0` and do nothing. A hook that breaks sessions is worse than no hook,
so these prefer to under-act rather than risk getting in your way. Neither hook
writes anything — `session-start.sh` is read-only recovery, and `stop-check.sh`
only inspects git history and prints a reminder.

## Cross-platform: Windows (PowerShell)

The same two hooks ship in two flavors so they work everywhere:

- **Linux / macOS** use the `.sh` scripts: `session-start.sh`, `stop-check.sh`.
- **Windows** use the PowerShell equivalents: `session-start.ps1`,
  `stop-check.ps1`. They mirror the `.sh` versions exactly — same positive-signal
  gating (`.kanbanr` marker or `$env:KANBANR_PROJECT`), same env knobs
  (`KANBANR_DATA_DIR`, `KANBANR_STOP_WINDOW_MIN`), and the same best-effort,
  **never-fail / never-block** semantics (always `exit 0`).

To register the PowerShell variants, use a hook entry with `"shell": "powershell"`
(or `"pwsh"`) so Claude Code runs them under PowerShell instead of a POSIX shell,
for example:

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [ { "type": "command", "shell": "powershell",
        "command": "<path>/skill/kanbanr/hooks/session-start.ps1" } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "shell": "powershell",
        "command": "<path>/skill/kanbanr/hooks/stop-check.ps1" } ] }
    ]
  }
}
```

When installed via the kanbanr **plugin** (see `.claude-plugin/plugin.json` at
the repo root), use `${CLAUDE_PLUGIN_ROOT}/skill/kanbanr/hooks/...` for the
`command` paths instead of an absolute path. The plugin manifest defaults to the
`.sh` hooks; Windows users point the `command` at the matching `.ps1` and add
`"shell": "powershell"`.
