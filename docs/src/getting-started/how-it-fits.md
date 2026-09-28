# How the pieces fit

## How the pieces fit

```mermaid
flowchart LR
  You["You + Claude (VS Code)"] -->|skill| CLI["kanbanr (CLI: local writer)"]
  CLI -->|"writes + commits"| Data[("data/ — a git repo")]
  CLI -->|"pull/push"| Remote[("git remote (sharing)")]
  Serve["kanbanr serve (view daemon)"] -->|reads| Data
  Serve -->|live SSE| Monitor["Web monitor (view-only)"]
```

You talk to Claude → Claude runs `kanbanr` commands → the CLI writes the files and commits to git
→ `kanbanr serve` (if running) pushes the change to the monitor live.
