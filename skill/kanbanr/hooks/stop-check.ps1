#!/usr/bin/env pwsh
#
# kanbanr Stop hook (PowerShell / Windows)
# ---------------------------------------------------------------------------
# Windows equivalent of stop-check.sh, with the same rules: when a reminder is
# due, print {"decision":"block","reason":"..."} so Claude Code shows the reason
# to Claude, which records unrecorded work (or says there's nothing to record)
# and stops. A reminder is due only when ALL hold:
#   1. the kanbanr data repo's last commit is older than the window
#      (default 30 min; $env:KANBANR_STOP_WINDOW_MIN);
#   2. the project shows work since then: uncommitted changes (the `.kanbanr`
#      marker aside) or a newer commit (outside a git repo, staleness alone);
#   3. no reminder in this session within the window (a per-session timestamp
#      in the temp dir);
#   4. Claude isn't already continuing because of a Stop hook
#      (`stop_hook_active`), so it can never loop.
# Best-effort: any missing tool or unexpected state exits 0 silently. See the
# header of stop-check.sh for the limitations.
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

# --- Hook input (JSON on stdin). -------------------------------------------
$HookInput = $null
if ([Console]::IsInputRedirected) {
    try { $HookInput = [Console]::In.ReadToEnd() | ConvertFrom-Json } catch { $HookInput = $null }
}
# Rule 4: already continuing because of a Stop hook -> never block again.
if ($HookInput -and $HookInput.stop_hook_active -eq $true) { exit 0 }

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

# --- Rule 2: the project shows work since the last board update. --------
$ChangedNote = ''
& git rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) {
    $dirty = (& git status --porcelain -- . ':(exclude).kanbanr' 2>$null | Select-Object -First 1)
    $headRaw = (& git log -1 --format=%ct 2>$null)
    $HeadEpoch = 0L
    if ($headRaw) { [void][long]::TryParse("$headRaw".Trim(), [ref]$HeadEpoch) }
    if (-not $dirty -and $HeadEpoch -le $LastCommitEpoch) { exit 0 }
    $ChangedNote = ', but the project has changed since then'
}

# --- Rule 3: at most one reminder per window per session. ----------------
$session = 'nosession'
if ($HookInput -and $HookInput.session_id) {
    $session = ("$($HookInput.session_id)" -replace '[^A-Za-z0-9_-]', '')
}
$Stamp = Join-Path ([System.IO.Path]::GetTempPath()) "kanbanr-stop-$session"
if (Test-Path -LiteralPath $Stamp) {
    $last = 0L
    if ([long]::TryParse((Get-Content -LiteralPath $Stamp -Raw -ErrorAction SilentlyContinue), [ref]$last)) {
        if (($NowEpoch - $last) -lt $WindowSec) { exit 0 }
    }
}
try { Set-Content -LiteralPath $Stamp -Value "$NowEpoch" -NoNewline -ErrorAction Stop } catch { }

# --- Remind Claude (block this stop once). -------------------------------
$AgeMin = [int][Math]::Floor($AgeSec / 60)
$reason = "kanbanr reminder: this project's board was last updated ~$AgeMin min ago$ChangedNote. If this session did project work that is not recorded in kanbanr yet, record it now in one kanbanr batch call: feature items and specs, todo-list task states, status moves, decisions and docs. If everything is already recorded, or nothing substantive happened, reply in one short line and stop without making changes."
[ordered]@{ decision = 'block'; reason = $reason } | ConvertTo-Json -Compress
exit 0
