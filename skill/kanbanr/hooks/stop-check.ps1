#!/usr/bin/env pwsh
#
# kanbanr Stop hook (PowerShell / Windows, ADVISORY)
# ---------------------------------------------------------------------------
# Windows equivalent of stop-check.sh. Same heuristic and semantics: when
# Claude finishes responding, nudge it to record work in kanbanr if it looks
# like substantive work happened but the kanbanr plan was not updated this
# session (operationalizes the "Keep the tool updated — before AND after every
# task" section of SKILL.md).
#
# ADVISORY, NOT a hard block: prints a reminder to STDERR, exits 0, never
# blocks the Stop event.
#
# Heuristic: kanbanr's data folder is a git repo and every write is a commit.
# So "did we update kanbanr recently?" ~= "does the data repo's HEAD have a
# commit within the last N minutes?". If the latest commit is OLDER than the
# window, assume the board was NOT touched this session and remind. This is a
# best-effort recency heuristic, not proof (see the .sh version's header for
# the full list of limitations) — so we WARN, never block.
# ---------------------------------------------------------------------------

$ErrorActionPreference = 'Continue'

# How recent a kanbanr data commit must be (minutes) to count as "updated this
# session". Override with $env:KANBANR_STOP_WINDOW_MIN. Default: 30 minutes.
$WindowMin = 30
if ($env:KANBANR_STOP_WINDOW_MIN) {
    $parsed = 0
    if ([int]::TryParse($env:KANBANR_STOP_WINDOW_MIN, [ref]$parsed)) {
        $WindowMin = $parsed
    }
}

# --- Guard 1: git must be available. -------------------------------------
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { exit 0 }

# --- Guard 2: only nudge when this directory looks kanbanr-tracked. -------
# Same positive-signal rule as session-start.ps1: a `.kanbanr` marker or
# $env:KANBANR_PROJECT. Otherwise stay silent (may not be a kanbanr project).
if (-not $env:KANBANR_PROJECT -and -not (Test-Path -LiteralPath '.kanbanr' -PathType Leaf)) {
    exit 0
}

# --- Guard 3: locate the kanbanr DATA dir (a git repo). ------------------
# Ask the CLI when it's installed: `kanbanr where` applies the full resolution
# ($env:KANBANR_DATA_DIR, the nearest `.kanbanr` marker's data_dir, else ./data),
# so a board kept next to the project (e.g. ../app.kanbanr) is found. Without
# the CLI, fall back to $env:KANBANR_DATA_DIR, else ./data.
$DataDir = $null
if (Get-Command kanbanr -ErrorAction SilentlyContinue) {
    try {
        $where = & kanbanr where 2>$null
        if ($LASTEXITCODE -eq 0 -and $where) { $DataDir = "$where".Trim() }
    } catch { $DataDir = $null }
}
if (-not $DataDir) {
    $DataDir = if ($env:KANBANR_DATA_DIR) { $env:KANBANR_DATA_DIR } else { './data' }
}

# Must be an existing directory that is (or is inside) a git work tree.
if (-not (Test-Path -LiteralPath $DataDir -PathType Container)) { exit 0 }
& git -C $DataDir rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { exit 0 }

# --- Heuristic: age of the latest commit in the data repo. ---------------
# %ct = committer date, unix seconds. No commits yet -> bail quietly.
$lastEpochRaw = (& git -C $DataDir log -1 --format=%ct 2>$null)
if (-not $lastEpochRaw) { exit 0 }
$lastEpochRaw = "$lastEpochRaw".Trim()
$LastCommitEpoch = 0L
if (-not [long]::TryParse($lastEpochRaw, [ref]$LastCommitEpoch)) { exit 0 }

# Current time as unix seconds.
$NowEpoch = [long][Math]::Floor(([DateTimeOffset]::UtcNow).ToUnixTimeSeconds())

$AgeSec = $NowEpoch - $LastCommitEpoch
$WindowSec = $WindowMin * 60

# Clock skew / future commit timestamp -> treat as fresh, don't nag.
if ($AgeSec -lt 0) { exit 0 }

# kanbanr was updated within the window: assume the plan is current.
if ($AgeSec -le $WindowSec) { exit 0 }

# --- Stale: emit an advisory reminder to STDERR (never block). -----------
$AgeMin = [int][Math]::Floor($AgeSec / 60)
$msg = @"
── kanbanr reminder (advisory) ──────────────────────────────────────
The kanbanr data repo's last commit was ~$AgeMin min ago (> $WindowMin min).
If you did substantive work this session, record it in kanbanr — the
single system of record for this project's PLAN:
  • scope new work as feature items (with --spec) and milestones
  • add/update todo-lists + tasks and their states (NotStarted/InProgress/Completed)
  • update specs, decisions, and docs you changed; move feature statuses
Bundle related changes into one 'kanbanr batch' call. (Note: this is a
best-effort recency heuristic — ignore it if the plan is already current.)
─────────────────────────────────────────────────────────────────────
"@
[Console]::Error.WriteLine($msg)

# Advisory only: succeed so the Stop event is never blocked.
exit 0
