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

### `stop-check.sh` — make sure work gets recorded (Stop)
After Claude finishes a turn, it checks whether work may have gone unrecorded. Claude Code only
shows a Stop hook's output to Claude when the hook asks to block the stop, so when a reminder is
due the script prints `{"decision":"block","reason":"…"}`: Claude reads the reason, records any
unrecorded work in kanbanr (or says in one line that there's nothing to record), and stops.

A reminder is due only when **all** of these hold:

1. The kanbanr **data repo**'s last commit is older than the window (default 30 min; override
   with `KANBANR_STOP_WINDOW_MIN`). Every kanbanr write is a commit, so this means the board
   wasn't updated recently.
2. The **project** shows work since that commit: uncommitted changes (the `.kanbanr` marker
   aside) or a newer commit. Outside a git repo this can't be checked, so staleness alone counts.
3. Claude hasn't been reminded in this session within the window (a per-session timestamp in the
   temp dir).
4. Claude isn't already continuing because of a Stop hook (`stop_hook_active`), so it never
   blocks twice in a row and can't loop.

> **Honest caveat:** this is a heuristic, not proof. It can't read the transcript, so it can't
> tell whether the project changes it sees were already recorded in an older board commit, and
> long-lived uncommitted work can trigger a reminder once per window. That's why the reminder
> tells Claude to stop without changes when everything is already recorded. The rules are
> documented at the top of `stop-check.sh`.

### `session-summary.sh`: session history on the board (PostCompact, SessionEnd, SessionStart)
Keeps a record of every session as a board doc named `sessions/<YYYY-MM-DD-HHMMSS>-<sid8>.md`, so
a later session, or a person, can see what earlier ones did without the transcript.

- **PostCompact** saves the summary Claude Code has just written for the compaction (suffix
  `-compact`). It's one board write, done before the hook returns.
- **SessionEnd** summarises the transcript with `claude -p` (Sonnet) in the background.
- **SessionStart** sweeps the project's recent transcripts for sessions that ended without a
  summary, at most three per start and none older than 14 days, again in the background.

The summary's shape comes from the project's own `.claude/commands/create-summary.md` when it has
one, and otherwise from `session-summary.prompt.md` beside the script. Its bookkeeping
lives next to the transcripts in `~/.claude/projects/<project>/.session-summaries/`, so nothing is
summarised twice. It acts only where a `.kanbanr` marker is found, walking up from the project
folder.

**Keeping things off the board.** Put anything that must never appear in a summary in a private
list, `~/.claude/kanbanr-summary-exclude.txt` (or set `$KANBANR_SUMMARY_EXCLUDE`). It holds one term
per line, matched case-insensitively and as whole words, and `#` starts a comment. A transcript
message that mentions a listed term is dropped before the model sees anything. Any line of a
summary or compaction summary that mentions one is removed before it is written, so it never
reaches the board or its git history. Keep that file outside every repository: it is the one
place the terms are written down. It needs `bash`, `jq` and `python3`: without them, or without a board, it exits 0 and
writes nothing. It isn't registered on Windows. The summariser's own `claude -p` runs with
`KANBANR_SESSION_SUMMARY` set, so it never triggers a summary of itself.

## Install

**Automatic (recommended).** With the skill installed (`make install-skill`, which links it to
`~/.claude/skills/kanbanr`), `kanbanr init` registers both hooks in your global Claude Code
settings (`$CLAUDE_CONFIG_DIR/settings.json`, default `~/.claude/settings.json`). It happens once
per machine: later `init`s see the hooks are already there. Pass `kanbanr init --no-hooks` to
skip it. You can also manage them directly:

```bash
kanbanr hooks install     # add them (merges; keeps your other settings and hooks)
kanbanr hooks status      # registered? do the scripts exist?
kanbanr hooks uninstall   # remove only kanbanr's entries
```

The merge keeps your other keys and hooks in order, writes atomically, never touches a settings
file that isn't valid JSON, replaces registrations whose script path no longer exists, and adds
nothing when the kanbanr Claude Code plugin is enabled (the plugin brings its own hooks). On
Windows the PowerShell scripts are registered.

Because the scripts only act in kanbanr-tracked folders (a `.kanbanr` marker, or
`$KANBANR_PROJECT`), one global registration covers every project, and nothing is written into
project folders.

**Manual.** To register them yourself, merge the `hooks` object from
[`settings.snippet.json`](./settings.snippet.json) into `~/.claude/settings.json` (or a project's
`.claude/settings.json`), pointing the `command` paths at these scripts. If you already have
`SessionStart`/`Stop` hooks, add the entries to those arrays rather than overwriting them.

Then start a new Claude Code session in a kanbanr-tracked directory: the board appears as recovered
context at session start.

### Configuration knobs (env vars)
- `CLAUDE_CONFIG_DIR` — where `kanbanr hooks install` / `kanbanr init` register the hooks
  (default `~/.claude`).
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
writes to the project or the board: `session-start.sh` is read-only recovery, and
`stop-check.sh` only inspects git history, prints a reminder, and keeps a per-session timestamp in
the temp dir.

## Cross-platform: Windows (PowerShell)

The same two hooks ship in two flavors so they work everywhere:

- **Linux / macOS** use the `.sh` scripts: `session-start.sh`, `stop-check.sh`.
- **Windows** use the PowerShell equivalents: `session-start.ps1`,
  `stop-check.ps1`. They mirror the `.sh` versions exactly — same positive-signal
  gating (`.kanbanr` marker or `$env:KANBANR_PROJECT`), same env knobs
  (`KANBANR_DATA_DIR`, `KANBANR_STOP_WINDOW_MIN`), and the same best-effort,
  **never-fail** semantics (always `exit 0`; the Stop reminder is a block-once JSON response).

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
