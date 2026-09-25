# kanbanr — Roadmap

**The board is the roadmap.** This file used to duplicate it — milestones, the features under each,
and a "won't do" list — and a duplicate of live state is a document that is wrong the moment
something moves. Everything it held now lives where it is maintained:

| What you want | Where it is |
|---|---|
| What is planned, in progress and done | `kanbanr board` — or the monitor, `kanbanr serve --ui-dir web/dist` |
| The milestones and their order | `kanbanr milestone list` (a dependency DAG) · `kanbanr critical-path` |
| What can be started right now | `kanbanr ready` · what is waiting: `kanbanr blocked` |
| Why the project exists, and what it will **not** do | `kanbanr charter show` — the non-goals section |
| Why an item exists and how it will be verified | `kanbanr feature show <CODE>` · `kanbanr trace <CODE>` |
| Why the architecture is the way it is | `kanbanr adr list` |
| How the last wave actually went | `kanbanr retro <MS-00x>` |

A released version's scope is in [CHANGELOG.md](../CHANGELOG.md), which is the roadmap's durable
half: it says what shipped rather than what was hoped for.

> If you are reading this in a published repository without the board beside it, the CHANGELOG is
> the honest summary. The board is a separate git repository (`<repo>.kanbanr`) because it is the
> project's working memory, not part of the shipped artifact.
