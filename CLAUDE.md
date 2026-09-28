<!-- kanbanr:begin — generated, edits here are overwritten -->

## This project is tracked with kanbanr

The board is the system of record for scope, reasoning, progress and documentation. It is a separate git repository beside this one. **Recover from it at the start of every session** and record work there as you go — this block is a pointer, not a copy.

**Why this project exists.** An AI agent doing real project work needs a durable, external system of record — not an in-session scratchpad. Without one, the plan, the reasoning and the evidence live only in a transcript: they vanish between sessions, cannot be reviewed, and work gets built that nobody agreed to. kanbanr keeps all of it in plain, git-backed files a human can read, and makes Claude keep it current as part of doing the work rather than as an afterthought.

**What it commits to** (work items link these by id):

- `G-1` A session recovers the full plan and continues, with no human recap
- `G-2` Every work item records why it exists and how it will be verified
- `G-3` Nothing substantial gets built before its reasoning is agreed
- `G-4` Any line of code can be traced back to the requirement and goal it serves
- `G-5` The tool stays low-friction enough that it is never worth bypassing
- `G-0` The system stays operable and maintainable

**Deliberately out of scope** — do not propose these as gaps:

- An enterprise/team PM platform — the bar is one developer plus their Claude, on one machine
- Horizontal scaling, or multiple server instances over one data dir
- Multi-server merge-conflict resolution (single push-to-backup is the model)
- SSO, role hierarchies, enterprise RBAC, audit-compliance tooling
- A query engine or pagination for tens of thousands of items
- Two-way sync with any external tracker — kanbanr is the source of truth, mirrors follow

**Constraints:**

- Local-first: plain YAML and markdown in a git repo, no database, no accounts, no server required
- The CLI is the single writer; the web monitor is read-only
- Sharing happens through a git remote, which is also the access boundary
- One binary, installable without a toolchain beyond cargo
- Raw data is never discarded — it is captured as files and kept; every summary, count and report is derived from it

**Before starting an item:** `kanbanr lessons --for <CODE>` — what this project already learned. **Before calling one done:** `kanbanr check <CODE>`.

| Question | Command |
|---|---|
| What is planned, in progress and done? | `kanbanr board` |
| What can I pick up now? | `kanbanr ready` |
| Why does this item exist, and how is it verified? | `kanbanr feature show <CODE>` |
| Why is the architecture like this? | `kanbanr adr list` |
| Why does this line of code exist? | `kanbanr why <file>:<line>` |
| What did the last wave cost? | `kanbanr retro <MS-00x>` |

Regenerate this block with `kanbanr claude sync`.

<!-- kanbanr:end -->
