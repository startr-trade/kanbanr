# Process guides

Each guide follows one project from setup to a shipped release under one process. There are two ways
to run any of them:

- **With Claude:** you talk to Claude, decide on the monitor, and push from your terminal. You don't
  need to learn any commands. This is the usual way.
- **From the command line:** you run every command yourself. Use this to work without Claude, or to
  see exactly what Claude does for you.

| Process | Stages | Guides |
|---|---|---|
| TOGAF, with sprints and releases | Vision → Business Arch → System Design → Implementation → Migration → Operations | [With Claude](togaf-with-claude.md) · [Command line](togaf-cli.md) |
| Default (Kanban), no sprints or releases | Planned → In Progress → Completed | [With Claude](default-with-claude.md) · [Command line](default-cli.md) |
| Scheduled, no sprints or releases | Planned → Scheduled → Completed | [With Claude](scheduled-with-claude.md) · [Command line](scheduled-cli.md) |
| PDCA, no sprints or releases | Plan → Do → Check → Act | [With Claude](pdca-with-claude.md) · [Command line](pdca-cli.md) |
| Design control, no sprints or releases | Planning → Inputs → Design → Review → Verification → Validation → Released | [With Claude](design-control-with-claude.md) · [Command line](design-control-cli.md) |
| Scrum, with sprints and releases | Backlog → Ready → In Progress → Review → Testing → Done → Released | [With Claude](scrum-with-claude.md) · [Command line](scrum-cli.md) |
| Agile, with sprints and releases | Plan → Design → Develop → Test → Review → Released | [With Claude](agile-with-claude.md) · [Command line](agile-cli.md) |

To compare the processes and their gates, see
[Processes: presets, gates and sign-offs](../using/processes.md).
