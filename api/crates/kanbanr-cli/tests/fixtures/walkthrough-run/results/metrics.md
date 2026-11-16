# Walkthrough metrics: shop

Derived from `raw/` alone; a second derive gives this file byte for byte.

## The run

| Measure | Value |
|---|---|
| kanbanr | kanbanr 0.7.0 (0000000000000000000000000000000000000000, built 2026-11-15) |
| Claude Code | 2.1.325 (Claude Code) |
| model | sonnet |
| user turns | 3 |
| cost (USD, cumulative from the last result line) | 0.55 |
| session stops (error results) | 0 |
| wall time (setup.log started to ended) | 0h 25m 00s |
| pushes | 2 |
| findings | 1 |
| truth mismatches | 1 |
| truth unknown | 0 |

## Decisions

7 decision(s) on the board copy.

| Item or release | Decisions |
|---|---|
| 9f078bd | 1 |
| FEAT-001 | 3 |
| v0.1.0 | 3 |

| Kind | Count |
|---|---|
| approve | 1 |
| cut | 1 |
| gate-result | 1 |
| manual-test | 1 |
| promote | 2 |
| signoff | 1 |

| Taken where | Count |
|---|---|
| monitor | 7 |

Prepared records confirmed: 2 (0 with a field edited). A walkthrough types nothing: the target for MT-1 is zero edited.

## Questions

3 question(s) asked: 3 in text, 0 through the dialog.

| Answered by | Count |
|---|---|
| recommended | 1 |
| runbook | 2 |

## Truth

| Verdict | Check | Board | Truth |
|---|---|---|---|
| OK | tag v0.1.0 | v0.1.0 shipped at stage tag, tag commit 9f078bd | refs/tags/v0.1.0 is on the code remote |
| MISMATCH | ci v0.1.0 | v0.1.0 shipped at stage tag, tag commit 9f078bd | no run is recorded for 9f078bd |
| OK | git-doctor | 0 warning(s) | every integrated item is merged where the process says |
| OK | doctor | no problem | no problem |
