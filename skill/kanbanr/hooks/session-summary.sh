#!/bin/bash
# Session summaries on the kanbanr board (FEAT-099). Registered for PostCompact, SessionEnd and
# SessionStart. It returns at once: saving a compaction summary is one board write, and anything
# needing a model call runs detached so it can neither time out the hook nor delay the session.
# Shipped with the skill and registered by `kanbanr hooks install` (FEAT-101). Best-effort: without
# jq, python3 or a kanbanr board to write to there is no summary, never a failed hook.
[ -n "$KANBANR_SESSION_SUMMARY" ] && exit 0   # the summariser's own claude -p: never recurse
command -v jq >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1 || exit 0

input="$(cat)"
field() { jq -r ".$1 // empty" <<<"$input"; }
here="$(cd "$(dirname "$0")" && pwd)"
export CLAUDE_PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(field cwd)}"
# Only a folder tracked with kanbanr has a board to save to: look for its marker, walking up.
d="$CLAUDE_PROJECT_DIR"
while [ -n "$d" ] && [ ! -f "$d/.kanbanr" ]; do [ "$d" = / ] && exit 0; d="$(dirname "$d")"; done
[ -n "$d" ] || exit 0
transcript="$(field transcript_path)"
# Detached from the hook's session so it outlives it; macOS has no setsid, and nohup alone suffices there.
detach=""; command -v setsid >/dev/null 2>&1 && detach=setsid
summarise() { $detach nohup python3 "$here/summarise_session.py" "$@" >/dev/null 2>&1 </dev/null & }

case "$(field hook_event_name)" in
  PostCompact)
    field compact_summary | python3 "$here/summarise_session.py" compact \
      "$(field session_id)" "$(field trigger)" "$transcript" >/dev/null 2>&1 ;;
  SessionEnd)
    [ -n "$transcript" ] && summarise transcript "$transcript" "end of session ($(field reason))" ;;
  SessionStart)
    [ -n "$transcript" ] && summarise sweep "$(dirname "$transcript")" "$(field session_id)" ;;
esac
exit 0
