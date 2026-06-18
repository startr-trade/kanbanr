#!/usr/bin/env bash
#
# kanbanr Stop hook (ADVISORY)
# ---------------------------------------------------------------------------
# Purpose: when Claude finishes responding, nudge it to record the work in
# kanbanr if it looks like substantive work happened but the kanbanr plan was
# not updated this session. This operationalizes the "Keep the tool updated —
# before AND after every task" section of skill/kanbanr/SKILL.md.
#
# IMPORTANT — this hook is ADVISORY, NOT a hard block:
#   - It only prints a reminder to STDERR. It exits 0 and never blocks the Stop
#     event. We deliberately do not emit a blocking decision, because the
#     heuristic below is necessarily imperfect (see "Heuristic & limitations").
#
# Heuristic
# ---------
# kanbanr's data folder is a git repo, and every kanbanr write is a git commit.
# So "did we update kanbanr recently?" ≈ "does the data repo's HEAD have a
# commit within the last N minutes?". If the most recent commit is OLDER than
# the window, we assume the board was NOT touched this session and remind.
#
# Limitations (be honest — this is best-effort, not proof):
#   - Time-window based: a long session with one early kanbanr update then lots
#     of later work could still look "stale" (false positive reminder), and a
#     short session that did nothing substantive but happened to commit could
#     look "fresh" (false negative). The window is a heuristic, not a contract.
#   - It cannot tell whether *substantive* work happened — it has no view of the
#     project's own code repo or the transcript. It only checks recency of the
#     kanbanr commit. So it may remind after trivial chitchat, or stay silent
#     after work that produced no commit but also no code (rare).
#   - It checks the kanbanr DATA repo only. The project's actual code lives in
#     the project's OWN git repo, which this hook intentionally does NOT inspect
#     (kanbanr tracks the PLAN; the code lives elsewhere). Correlating the two
#     reliably is out of scope for a safe, best-effort nudge.
#   - If there are unpushed/uncommitted *pending* kanbanr changes, those won't
#     show as a HEAD commit; we treat only committed history as "updated".
# Because of all this, we WARN, never block. A false reminder costs one line of
# stderr; a false block would wreck the workflow.
# ---------------------------------------------------------------------------

set -u

# How recent a kanbanr data commit must be (in minutes) to count as "updated
# this session". Override with $KANBANR_STOP_WINDOW_MIN. Default: 30 minutes.
WINDOW_MIN="${KANBANR_STOP_WINDOW_MIN:-30}"

# --- Guard 1: git must be available. -------------------------------------
command -v git >/dev/null 2>&1 || exit 0

# --- Guard 2: only nudge when this directory looks kanbanr-tracked. -------
# Same positive-signal rule as session-start.sh: a `.kanbanr` marker or
# $KANBANR_PROJECT. Otherwise stay silent (this may not be a kanbanr project).
if [ -z "${KANBANR_PROJECT:-}" ] && [ ! -f ".kanbanr" ]; then
  exit 0
fi

# --- Guard 3: locate the kanbanr DATA dir (a git repo). ------------------
# Resolution mirrors the CLI: --data-dir is N/A here, so $KANBANR_DATA_DIR,
# else ./data (the documented default).
DATA_DIR="${KANBANR_DATA_DIR:-./data}"

# Must be an existing directory that is (or is inside) a git work tree.
[ -d "$DATA_DIR" ] || exit 0
if ! git -C "$DATA_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  exit 0
fi

# --- Heuristic: age of the latest commit in the data repo. ---------------
# %ct = committer date, unix seconds. If there are no commits yet, bail quietly.
LAST_COMMIT_EPOCH="$(git -C "$DATA_DIR" log -1 --format=%ct 2>/dev/null || true)"
case "$LAST_COMMIT_EPOCH" in
  ''|*[!0-9]*) exit 0 ;;   # empty or non-numeric -> can't reason; stay silent.
esac

NOW_EPOCH="$(date +%s 2>/dev/null || true)"
case "$NOW_EPOCH" in
  ''|*[!0-9]*) exit 0 ;;   # date unavailable -> can't reason; stay silent.
esac

AGE_SEC=$(( NOW_EPOCH - LAST_COMMIT_EPOCH ))
WINDOW_SEC=$(( WINDOW_MIN * 60 ))

# Clock skew / future commit timestamp -> treat as fresh, don't nag.
if [ "$AGE_SEC" -lt 0 ]; then
  exit 0
fi

if [ "$AGE_SEC" -le "$WINDOW_SEC" ]; then
  # kanbanr was updated within the window: assume the plan is current.
  exit 0
fi

# --- Stale: emit an advisory reminder to STDERR (never block). -----------
AGE_MIN=$(( AGE_SEC / 60 ))
{
  echo "── kanbanr reminder (advisory) ──────────────────────────────────────"
  echo "The kanbanr data repo's last commit was ~${AGE_MIN} min ago (> ${WINDOW_MIN} min)."
  echo "If you did substantive work this session, record it in kanbanr — the"
  echo "single system of record for this project's PLAN:"
  echo "  • scope new work as feature items (with --spec) and milestones"
  echo "  • add/update todo-lists + tasks and their states (NotStarted/InProgress/Completed)"
  echo "  • update specs, decisions, and docs you changed; move feature statuses"
  echo "Bundle related changes into one 'kanbanr batch' call. (Note: this is a"
  echo "best-effort recency heuristic — ignore it if the plan is already current.)"
  echo "─────────────────────────────────────────────────────────────────────"
} >&2

# Advisory only: succeed so the Stop event is never blocked.
exit 0
