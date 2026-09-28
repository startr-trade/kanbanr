#!/usr/bin/env bash
#
# kanbanr SessionStart hook
# ---------------------------------------------------------------------------
# Purpose: at the start of a Claude Code session, recover the project's state
# FROM kanbanr so Claude continues where the project left off instead of
# restarting from memory. This operationalizes the "Recover & resume" section
# of skill/kanbanr/SKILL.md.
#
# What it does: if `kanbanr` is on PATH *and* a project is resolvable (a
# `.kanbanr` marker in the cwd, or $KANBANR_PROJECT set), it prints the current
# board and recent activity on stdout. SessionStart hook stdout is surfaced to
# Claude as context, so the model sees the live plan without having to remember
# to ask for it.
#
# SAFETY CONTRACT: this hook is BEST-EFFORT and must NEVER break a session.
#   - If kanbanr is not installed, or no project is resolvable, or any command
#     fails, we exit 0 (optionally printing a short, harmless note).
#   - We never write anything; this is a read-only recovery step.
# ---------------------------------------------------------------------------

# Be defensive but do not use `set -e`: we want to swallow errors ourselves and
# always exit 0. A failing recovery step must not abort the user's session.
set -u

# 1) kanbanr must be installed. If not, silently do nothing.
if ! command -v kanbanr >/dev/null 2>&1; then
  exit 0
fi

# 2) A project must be resolvable. kanbanr resolves the active project from
#    --project / $KANBANR_PROJECT / a `.kanbanr` marker / the directory name.
#    We only auto-recover when we have a *positive* signal that this directory
#    is tracked, so we don't dump an unrelated project's board into the session.
#    Positive signals: $KANBANR_PROJECT is set, or a `.kanbanr` marker exists.
PROJECT_ARGS=()
if [ -n "${KANBANR_PROJECT:-}" ]; then
  # Explicit env override wins; let kanbanr use it directly.
  :
elif [ -f ".kanbanr" ]; then
  # `.kanbanr` marker present in the cwd: this directory is tracked. kanbanr
  # picks the project up from the marker on its own, so no extra args needed.
  # (We could parse the marker, but letting the CLI resolve it is more robust
  #  to format changes — the marker currently holds just the project name.)
  :
else
  # No marker and no env var -> we cannot be confident a kanbanr project
  # belongs to this directory. Do nothing rather than guess.
  exit 0
fi

# 3) Print the board and recent activity. Guard EACH call: a failure here
#    (e.g. data dir missing, project mis-resolved) should still exit 0.
echo "## kanbanr — recovered project state (SessionStart)"
echo

echo "### Board"
if ! kanbanr "${PROJECT_ARGS[@]}" board 2>/dev/null; then
  echo "(kanbanr board unavailable — continuing without recovered board)"
fi
echo

echo "### Recent activity"
if ! kanbanr "${PROJECT_ARGS[@]}" activity 2>/dev/null; then
  echo "(kanbanr activity unavailable)"
fi
echo

# Lessons (FEAT-055): the few most-believed ones, so the next piece of work
# starts from what this project already learned rather than rediscovering it.
# Confidence decays, so this list stays short on its own.
LESSONS="$(kanbanr "${PROJECT_ARGS[@]}" lessons 2>/dev/null | head -n 10 || true)"
if [ -n "$LESSONS" ] && [ "$LESSONS" != "(nothing learned here yet)" ]; then
  echo "### Lessons this project has learned"
  printf '%s\n' "$LESSONS"
  echo
  echo "_Before starting an item, check \`kanbanr lessons --for <CODE>\`. If one of"
  echo "these turns out to be wrong, say so: \`kanbanr lesson contradict <L-n> --note '…'\`._"
  echo
fi

echo "_kanbanr is the system of record for this project's PLAN (features/tasks/"
echo "specs/decisions/progress). Resume from the board above; record new work in"
echo "kanbanr as you go. Every document (requested or self-initiated) goes in"
echo "kanbanr docs (\`kanbanr doc add …\`), NOT the working folder — unless the user"
echo "asks for it in the project folder._"

# Always succeed: recovery is advisory, never fatal.
exit 0
