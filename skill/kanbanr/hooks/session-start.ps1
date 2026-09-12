#!/usr/bin/env pwsh
#
# kanbanr SessionStart hook (PowerShell / Windows)
# ---------------------------------------------------------------------------
# Windows equivalent of session-start.sh. Same purpose and SAFETY CONTRACT:
# at the start of a Claude Code session, recover the project's state FROM
# kanbanr so Claude resumes from the live plan instead of restarting from
# memory (operationalizes the "Recover & resume" section of SKILL.md).
#
# What it does: if `kanbanr` is on PATH *and* a project is resolvable (a
# `.kanbanr` marker in the cwd, or $env:KANBANR_PROJECT set), it prints the
# current board and recent activity on stdout. SessionStart hook stdout is
# surfaced to Claude as context.
#
# SAFETY CONTRACT: BEST-EFFORT and must NEVER break a session.
#   - If kanbanr is not installed, or no project is resolvable, or any command
#     fails, we exit 0 (optionally printing a short, harmless note).
#   - We never write anything; this is a read-only recovery step.
# ---------------------------------------------------------------------------

# Swallow errors ourselves and always exit 0 — a failing recovery step must
# never abort the user's session. Do NOT use `$ErrorActionPreference='Stop'`.
$ErrorActionPreference = 'Continue'

# 1) kanbanr must be installed. If not, silently do nothing.
if (-not (Get-Command kanbanr -ErrorAction SilentlyContinue)) {
    exit 0
}

# 2) A project must be resolvable. Only auto-recover on a *positive* signal
#    that this directory is tracked, so we don't dump an unrelated project's
#    board into the session. Positive signals: $env:KANBANR_PROJECT set, or a
#    `.kanbanr` marker file present in the cwd. kanbanr resolves the project
#    itself from the env var / marker, so we pass no extra project args.
if (-not $env:KANBANR_PROJECT -and -not (Test-Path -LiteralPath '.kanbanr' -PathType Leaf)) {
    # No marker and no env var -> cannot be confident this dir is a kanbanr
    # project. Do nothing rather than guess.
    exit 0
}

# 3) Print the board and recent activity. Guard EACH call so a failure here
#    (e.g. data dir missing, project mis-resolved) still exits 0.
Write-Output "## kanbanr — recovered project state (SessionStart)"
Write-Output ""

Write-Output "### Board"
try {
    $board = & kanbanr board 2>$null
    if ($LASTEXITCODE -eq 0 -and $board) {
        $board | Write-Output
    } else {
        Write-Output "(kanbanr board unavailable — continuing without recovered board)"
    }
} catch {
    Write-Output "(kanbanr board unavailable — continuing without recovered board)"
}
Write-Output ""

Write-Output "### Recent activity"
try {
    $activity = & kanbanr activity 2>$null
    if ($LASTEXITCODE -eq 0 -and $activity) {
        $activity | Write-Output
    } else {
        Write-Output "(kanbanr activity unavailable)"
    }
} catch {
    Write-Output "(kanbanr activity unavailable)"
}
Write-Output ""

Write-Output "_kanbanr is the system of record for this project's PLAN (features/tasks/"
Write-Output "specs/decisions/progress). Resume from the board above; record new work in"
Write-Output "kanbanr as you go. Every document (requested or self-initiated) goes in"
Write-Output "kanbanr docs (``kanbanr doc add …``), NOT the working folder — unless the user"
Write-Output "asks for it in the project folder._"

# Always succeed: recovery is advisory, never fatal.
exit 0
