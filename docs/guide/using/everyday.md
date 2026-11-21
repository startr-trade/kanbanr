# Everyday use

## Everyday use — just talk to your agent

Scoping work, deciding on it and moving it all happen in the conversation: see
[Working with your agent](with-claude.md) for the plans and questions, and why the agent asks what it asks.

| You say to the agent | What the skill runs |
|---|---|
| "Set up kanbanr for this project, states Backlog→Doing→Done, new features start in Backlog" | `kanbanr project init … --statuses Backlog,Doing,Done --default-state Backlog` |
| "Add a Foundations milestone" | `kanbanr milestone add --name Foundations --code MS-001` |
| "Track a feature for the login flow under MS-001 with a spec" | `kanbanr feature add --title "Login flow" --milestone MS-001 --spec "…"` |
| "Log a defect: saving crashes" | `kanbanr feature add --title "Saving crashes" --milestone MS-001 --kind defect` (with `config codes` set, `--kind` chooses the code's prefix: `DEF-001`; a later kind change does not rename; renaming is the explicit step below) |
| "Make defects read DEF and chores MNT" | `kanbanr config codes --set defect=DEF,bug=DEF,chore=MNT` |
| "Rename FEAT-150 to DEF-095" | `kanbanr feature edit FEAT-150 --code DEF-095`: the old code stays on the item as an alias, so old links, commit trailers and records still reach it, and no new item is ever given it |
| "Bring the old items to their kinds' prefixes" | `kanbanr feature recode` prints the plan (what each item becomes) and writes nothing; the agent shows it to you, runs `kanbanr prepare recode`, and you confirm it in the Review page's Recode group: one board commit, never applied by the agent |
| "Start a todo-list for this session on FEAT-001" | `kanbanr todo add FEAT-001 --description "session 1"` (→ TL-001) |
| "Add tasks to TL-001" | `kanbanr task add FEAT-001 TL-001 --text "…"` |
| "Start task T2 in TL-001" / "T2 is done" | `kanbanr task state FEAT-001 TL-001 T2 InProgress` / `Completed` |
| "Move the login feature to Scheduled" | `kanbanr move FEAT-001 Scheduled` |
| "What's on the board?" | `kanbanr board` |
| "Save these API notes under design/api" | `kanbanr doc add design/api …` |

> Every feature requires a milestone — create the milestone first.


## How the agent uses Kanbanr (the contract)

Say **"start using kanbanr for this project"** once. In a folder that isn't tracked yet, the agent
first runs a short setup interview in plan mode: the board, the charter and the workflow. Once you
approve it, the setup runs. See [the quickstart](../getting-started/quickstart.md). After that, for
the rest of the project you don't have to say anything about Kanbanr — the agent treats it as the **single system of record**:

- **Everything about the project's activity lives in kanbanr** — scope, specs, progress, task
  status, decisions, docs. The *only* thing kept outside it is your conversation transcript.
- **Docs live in Kanbanr by default.** Any document, whether you asked for it or the agent wrote it on
  its own (design notes, decisions, research, runbooks, plans, guides), is saved as a kanbanr doc
  (`kanbanr doc add …`), not as a file in your codebase. The agent writes a doc into the project
  folder only when you ask, e.g. when you want an mdBook/MkDocs site or README as a deliverable.
- **No ephemeral lists.** The agent does not track project work in a throwaway session list; it
  creates persistent **todo-lists on the feature items** instead — so nothing is lost.
- **Resumable across sessions.** At the start of a session the agent recovers state from Kanbanr
  (`kanbanr board`) and continues exactly where things left off.
- **Updated before and after every task** — it reflects what it's about to do (todo-list item →
  In progress) and what it finished (→ Completed, spec/docs updated, status moved).
- When moving a feature **out of `Deferred`**, the agent first reviews its spec for **staleness**.
- **Feature items are never deleted** — to retire one it's moved to a no-op state.
- **Completed is not released.** On a project that works in releases, `kanbanr feature show` and the
  item's page carry a `Release` line: released in a version, planned for one and not released yet,
  or in no release. Read it before saying an item has shipped.
- For several changes at once, the agent sends **one bundled `kanbanr batch` call** (new/edited
  feature items, status moves, new todo-lists + items, task-state updates, doc changes).
