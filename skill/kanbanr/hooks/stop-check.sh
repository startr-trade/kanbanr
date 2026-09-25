#!/usr/bin/env bash
#
# kanbanr Stop hook
# ---------------------------------------------------------------------------
# Purpose: when Claude finishes responding, make sure work done in a
# kanbanr-tracked project gets recorded in kanbanr. This operationalizes the
# "Keep the tool updated — before AND after every task" section of
# skill/kanbanr/SKILL.md.
#
# How it reaches Claude: Claude Code only shows a Stop hook's output to Claude
# when the hook asks to block the stop. So when a reminder is due, this script
# prints {"decision":"block","reason":"..."} on stdout: Claude reads the reason,
# records any unrecorded work (or says there's nothing to record), then stops.
#
# When a reminder is due — ALL of these must hold:
#   1. The kanbanr data repo's last commit is older than the window
#      (default 30 min; $KANBANR_STOP_WINDOW_MIN). Every kanbanr write is a
#      commit, so this means "the board wasn't updated recently".
#   2. The project shows work since that commit: uncommitted changes (the
#      `.kanbanr` marker aside) or a newer commit in the project's git repo.
#      Outside a git repo this can't be checked, so staleness alone counts.
#   3. Claude hasn't already been reminded in this session within the window
#      (a timestamp per session id in the temp dir).
#   4. Claude isn't already continuing because of a Stop hook
#      (`stop_hook_active` in the hook input): it never blocks twice in a row,
#      so it can't loop.
#
# Limitations (it's a heuristic, not proof): it can't read the transcript, so
# it can't tell whether the changes it sees were already recorded in an older
# board commit, and long-lived uncommitted work can trigger a reminder once per
# window. The reminder therefore tells Claude to stop without changes when
# everything is already recorded.
#
# SAFETY CONTRACT: best-effort, never breaks a session. Any missing tool,
# untracked directory, or unexpected state exits 0 silently. It never writes to
# the project or the board; its only write is the per-session timestamp in the
# temp dir.
# ---------------------------------------------------------------------------

set -u

WINDOW_MIN="${KANBANR_STOP_WINDOW_MIN:-30}"
case "$WINDOW_MIN" in ''|*[!0-9]*) WINDOW_MIN=30 ;; esac
WINDOW_SEC=$(( WINDOW_MIN * 60 ))

# --- Hook input (JSON on stdin). -------------------------------------------
INPUT=""
if [ ! -t 0 ]; then
  INPUT="$(cat 2>/dev/null || true)"
fi

# Rule 4: already continuing because of a Stop hook -> never block again.
if printf '%s' "$INPUT" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
  exit 0
fi

command -v git >/dev/null 2>&1 || exit 0

# --- Only in kanbanr-tracked folders. ----------------------------------------
if [ -z "${KANBANR_PROJECT:-}" ] && [ ! -f ".kanbanr" ]; then
  exit 0
fi

# --- A wave that ended without a retro (FEAT-054). ---------------------------
# This is time-sensitive in a way the staleness check is not: the moment a wave
# ends is the only moment its lessons are still fresh. Checked before the
# staleness rules, and subject to the same once-per-session-window guard below.
RETRO_DUE=""
if command -v kanbanr >/dev/null 2>&1; then
  RETRO_DUE="$(kanbanr retro --due 2>/dev/null | grep -F 'has no retro' | head -n 3 || true)"
fi

# --- Locate the board (the kanbanr data repo). -------------------------------
# `kanbanr where` applies the full resolution ($KANBANR_DATA_DIR, the nearest
# `.kanbanr` marker's data_dir, else ./data). Without the CLI, fall back to
# $KANBANR_DATA_DIR, else ./data.
DATA_DIR=""
if command -v kanbanr >/dev/null 2>&1; then
  DATA_DIR="$(kanbanr where 2>/dev/null || true)"
fi
[ -n "$DATA_DIR" ] || DATA_DIR="${KANBANR_DATA_DIR:-./data}"
[ -d "$DATA_DIR" ] || exit 0
git -C "$DATA_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

BOARD_EPOCH="$(git -C "$DATA_DIR" log -1 --format=%ct 2>/dev/null || true)"
case "$BOARD_EPOCH" in ''|*[!0-9]*) exit 0 ;; esac
NOW="$(date +%s 2>/dev/null || true)"
case "$NOW" in ''|*[!0-9]*) exit 0 ;; esac

# A finished wave with no retro is worth one reminder regardless of staleness.
if [ -n "$RETRO_DUE" ]; then
  SESSION="$(printf '%s' "$INPUT" | sed -n 's/.*"session_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1 | tr -cd 'A-Za-z0-9_-')"
  STAMP="${TMPDIR:-/tmp}/kanbanr-retro-${SESSION:-nosession}"
  if [ ! -f "$STAMP" ]; then
    printf '%s' "$NOW" > "$STAMP" 2>/dev/null || true
    WAVES="$(printf '%s' "$RETRO_DUE" | tr '\n' ' ' | sed 's/"/\\"/g')"
    printf '{"decision":"block","reason":"kanbanr: a wave has finished and its retrospective is not written: %s Write it now, while it is fresh: run `kanbanr retro <MILESTONE> --write`, read the facts it recorded, and fill in the narrative section from them (it may explain the numbers, it may not contradict them). If it is genuinely not worth one, say so in one line and stop."}\n' "$WAVES"
    exit 0
  fi
fi

# Rule 1: the board was updated within the window -> nothing to do.
AGE_SEC=$(( NOW - BOARD_EPOCH ))
[ "$AGE_SEC" -lt 0 ] && exit 0
[ "$AGE_SEC" -le "$WINDOW_SEC" ] && exit 0

# Rule 2: the project shows work since the last board update.
CHANGED_NOTE=""
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  DIRTY="$(git status --porcelain -- . ':(exclude).kanbanr' 2>/dev/null | head -n 1)"
  HEAD_EPOCH="$(git log -1 --format=%ct 2>/dev/null || true)"
  case "$HEAD_EPOCH" in ''|*[!0-9]*) HEAD_EPOCH=0 ;; esac
  if [ -z "$DIRTY" ] && [ "$HEAD_EPOCH" -le "$BOARD_EPOCH" ]; then
    exit 0
  fi
  CHANGED_NOTE=", but the project has changed since then"
fi

# Rule 3: at most one reminder per window per session.
SESSION="$(printf '%s' "$INPUT" | sed -n 's/.*"session_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1 | tr -cd 'A-Za-z0-9_-')"
STAMP="${TMPDIR:-/tmp}/kanbanr-stop-${SESSION:-nosession}"
if [ -f "$STAMP" ]; then
  LAST="$(cat "$STAMP" 2>/dev/null || true)"
  case "$LAST" in
    ''|*[!0-9]*) ;;
    *) [ $(( NOW - LAST )) -lt "$WINDOW_SEC" ] && exit 0 ;;
  esac
fi
printf '%s' "$NOW" > "$STAMP" 2>/dev/null || true

# --- Remind Claude (block this stop once). -----------------------------------
AGE_MIN=$(( AGE_SEC / 60 ))
REASON="kanbanr reminder: this project's board was last updated ~${AGE_MIN} min ago${CHANGED_NOTE}. If this session did project work that is not recorded in kanbanr yet, record it now in one kanbanr batch call: feature items and specs, todo-list task states, status moves, decisions and docs. If everything is already recorded, or nothing substantive happened, reply in one short line and stop without making changes."
printf '{"decision":"block","reason":"%s"}\n' "$REASON"
exit 0
