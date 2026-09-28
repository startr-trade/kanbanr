# Everyday use

## Everyday use — just talk to Claude

| You say to Claude | What the skill runs |
|---|---|
| "Set up kanbanr for this project, states Backlog→Doing→Done, new features start in Backlog" | `kanbanr project init … --statuses Backlog,Doing,Done --default-state Backlog` |
| "Add a Foundations milestone" | `kanbanr milestone add --name Foundations --code MS-001` |
| "Track a feature for the login flow under MS-001 with a spec" | `kanbanr feature add --title "Login flow" --milestone MS-001 --spec "…"` |
| "Start a todo-list for this session on FEAT-001" | `kanbanr todo add FEAT-001 --description "session 1"` (→ TL-001) |
| "Add tasks to TL-001" | `kanbanr task add FEAT-001 TL-001 --text "…"` |
| "Start task T2 in TL-001" / "T2 is done" | `kanbanr task state FEAT-001 TL-001 T2 InProgress` / `Completed` |
| "Move the login feature to Scheduled" | `kanbanr move FEAT-001 Scheduled` |
| "What's on the board?" | `kanbanr board` |
| "Save these API notes under design/api" | `kanbanr doc add design/api …` |

> Every feature requires a milestone — create the milestone first.


## How Claude uses kanbanr (the contract)

Say **"start using kanbanr for this project"** once. After that, for the rest of the project you
don't have to say anything about kanbanr — Claude treats it as the **single system of record**:

- **Everything about the project's activity lives in kanbanr** — scope, specs, progress, task
  status, decisions, docs. The *only* thing kept outside it is your conversation transcript.
- **Docs live in kanbanr by default.** Any document, whether you asked for it or Claude wrote it on
  its own (design notes, decisions, research, runbooks, plans, guides), is saved as a kanbanr doc
  (`kanbanr doc add …`), not as a file in your codebase. Claude writes a doc into the project
  folder only when you ask, e.g. when you want an mdBook/MkDocs site or README as a deliverable.
- **No ephemeral lists.** Claude does not track project work in a throwaway session list; it
  creates persistent **todo-lists on the feature items** instead — so nothing is lost.
- **Resumable across sessions.** At the start of a session Claude recovers state from kanbanr
  (`kanbanr board`) and continues exactly where things left off.
- **Updated before and after every task** — it reflects what it's about to do (todo-list item →
  In progress) and what it finished (→ Completed, spec/docs updated, status moved).
- When moving a feature **out of `Deferred`**, Claude first reviews its spec for **staleness**.
- **Feature items are never deleted** — to retire one it's moved to a no-op state.
- For several changes at once, Claude sends **one bundled `kanbanr batch` call** (new/edited
  feature items, status moves, new todo-lists + items, task-state updates, doc changes).
