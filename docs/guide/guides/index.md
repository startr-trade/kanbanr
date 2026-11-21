# Process guides

Each guide follows one project from setup to a shipped release under one process. There are two ways
to run any of them:

- **With your agent:** you talk to your agent, decide on the monitor, and push from your terminal. You don't
  need to learn any commands. This is the usual way.
- **From the command line:** you run every command yourself. Use this to work without the agent, or to
  see exactly what the agent does for you.

| Process | Stages | Guides |
|---|---|---|
| Agile, with sprints and releases (recommended to start) | Plan → Design → Develop → Test → Review → Released | [With your agent](agile-with-claude.md) · [Command line](agile-cli.md) |
| TOGAF, with sprints and releases | Vision → Business Arch → System Design → Implementation → Migration → Operations | [With your agent](togaf-with-claude.md) · [Command line](togaf-cli.md) |
| Default (Kanban), no sprints or releases | Planned → In Progress → Completed | [With your agent](default-with-claude.md) · [Command line](default-cli.md) |
| Scheduled, no sprints or releases | Planned → Scheduled → Completed | [With your agent](scheduled-with-claude.md) · [Command line](scheduled-cli.md) |
| PDCA, no sprints or releases | Plan → Do → Check → Act | [With your agent](pdca-with-claude.md) · [Command line](pdca-cli.md) |
| Design control, no sprints or releases | Planning → Inputs → Design → Review → Verification → Validation → Released | [With your agent](design-control-with-claude.md) · [Command line](design-control-cli.md) |
| Scrum, with sprints and releases | Backlog → Ready → In Progress → Review → Testing → Done → Released | [With your agent](scrum-with-claude.md) · [Command line](scrum-cli.md) |

**Which process first?** Start with agile unless your team already has a process of its own. TOGAF, Scrum and the others stay here for projects that need their stages. The walkthrough that led to this advice is [the report](/p/kanbanr/docs/file/notes/developer-path-togaf-vs-agile.md).

To compare the processes and their gates, see
[Processes: presets, gates and sign-offs](../using/processes.md).

Before your first release, read [Designing CI and the release workflow](delivery-pipeline.md).
