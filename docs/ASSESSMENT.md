# kanbanr — Honest Assessment, Comparison & Gaps

> A candid evaluation of where kanbanr stands as a task system for Claude-driven development:
> what it does well, how it compares to existing tools, and what it's missing. Paired with
> [ROADMAP.md](ROADMAP.md) (prioritized improvements) and [OPEN_SOURCING.md](OPEN_SOURCING.md)
> (release steps). This is a working document — revise it as the product evolves.
>
> *Comparisons reflect general knowledge as of early 2026; verify the current state of each tool
> before quoting it publicly.*

## 0. Scope: this is a personal tool

kanbanr is deliberately a **personal / single-developer tool** — one person (plus their Claude)
tracking their own projects, optionally backed up to their own git remote. It is **not** trying to
be an enterprise/team PM platform. That framing is load-bearing for everything below: concerns
that only matter at team/enterprise scale (horizontal scaling, multi-writer conflict resolution,
rate-limiting, pagination for thousands of items, SSO/RBAC) are **explicitly out of scope** and
should *not* be treated as gaps. The bar kanbanr must clear is different: **low friction, fast,
works on one machine, easy to install, gets out of your way.** The multi-user/permission machinery
that already exists is a bonus for "share my read-only monitor with a collaborator," not a
requirement to satisfy.

## 1. What kanbanr is betting on

kanbanr's thesis is that an AI agent doing real project work needs a **durable, external system of
record** — not an in-session scratchpad. Four design choices follow from that:

1. **Single source of truth + a behavioral contract.** The skill (`SKILL.md`) tells Claude to put
   *everything* (scope, specs, progress, decisions, docs) into kanbanr, resume from it each
   session, and never use an ephemeral todo list. This is the real product; the CLI/server are
   plumbing.
2. **Persistent todo-lists on permanent feature items.** Work is modeled as epics that are never
   deleted (only moved), each holding session-scoped todo-lists. State survives across sessions by
   construction.
3. **Git-backed YAML, authored per user.** The data folder is a git repo; every write is a commit
   by the acting user (name + email), optionally pushed to remotes. You get a full, attributable
   audit trail for free, in a diffable format with no database.
4. **A live, read-only web monitor.** A server watches the files and streams changes over SSE, so a
   human can watch the agent work in parallel without being able to corrupt the state.

## 2. Honest strengths

- **The contract is the moat.** Most "AI task" tools are storage; kanbanr ships *behavior* — when
  and how Claude should record work. That's the hard, valuable part and it's well specified.
- **Auditability.** Per-user git commits with meaningful messages, immutable email fingerprints,
  and credentials kept out of the repo make "who changed what, and why" answerable.
- **Portable data.** YAML + markdown, status-as-folders, spec-as-file. Readable, greppable,
  reviewable in a normal PR. No lock-in.
- **Clear write boundary.** The server is the *only* writer and validates every mutation
  (transitions, milestone-required, DAG cycles, referential integrity). The web is strictly view.
- **Tested.** Two-layer integration tests (Docker-less functional + container packaging smoke)
  plus core unit tests; security regressions (secret-never-committed) are asserted.
- **Sane security posture.** Per-user view/operate permissions, admin bypass, header-only JWTs,
  auto-seeded admin, vendored git (no external binary).

## 3. Honest weaknesses & trade-offs

Judged as a **personal tool** (§0), the weaknesses that actually matter:

- **It requires a running server — the defining friction.** For a solo user, having to start and
  keep a service alive just to record your own tasks is heavier than the zero-infra norm (a CLI
  that writes local files). The server earns its keep *only when you want the live monitor*; for
  the everyday "Claude updates my board" loop it's pure overhead. This is the #1 thing to fix. →
  [ROADMAP.md](ROADMAP.md) "Local mode".
- **Web session ergonomics.** A JWT expiry drops the monitor to the login screen with no silent
  refresh — mildly annoying for a tool you leave open all day.
- **Thin where a solo dev would still feel it:** no priorities/due-dates/labels on items, and no
  cross-*feature* dependencies (only milestones form a DAG). Assignees genuinely don't matter for a
  personal tool, so that's fine.
- **Activity timeline is hidden.** Per-user git history exists but the UI never shows it — a
  personal "what did I/Claude do recently" view would be high-value and is almost free.
- **Cross-platform unknowns.** `~/.kanbanr` via `HOME` and path handling are Linux-tested; a solo
  user on macOS/Windows is plausible, so this matters.
- **Single-binary robustness:** no health/readiness endpoint or structured logs — minor, but helps
  when *you* are also the one operating it.

**Explicitly NOT weaknesses for this tool** (out of scope per §0, don't spend effort here):
horizontal scaling, the in-process single-writer lock, multi-server remote-conflict policy,
token-endpoint rate-limiting, list pagination / large-scale query, and SSO/enterprise RBAC. The
existing per-user permissions are already *more* than a strictly personal tool needs.

TLS is a half-exception: irrelevant on localhost (the normal case), but worth a one-paragraph note
for the user who exposes their monitor beyond their machine. Don't build it; document it.

## 4. How it compares

| Tool | Storage | Infra | Live human view | Multi-user / authz | Git-native | AI integration |
|---|---|---|---|---|---|---|
| **kanbanr** | YAML + md in git | **server** + CLI | **Yes (SSE web)** | **Yes (per-user)** | **Yes (per-user commits)** | Claude skill (CLI) |
| Backlog.md | markdown in git | CLI only | Board (CLI/local) | No | Yes | Agent-friendly CLI |
| Task Master AI | JSON/markdown | CLI + MCP | No | No | Via your repo | MCP (Cursor/Claude) |
| GitHub Projects | GitHub | SaaS | Yes (GitHub UI) | Yes (GitHub) | Issues/PRs | MCP server |
| Linear / Jira | SaaS DB | SaaS | Yes | Yes | No | MCP servers |
| Claude `TodoWrite` | in-session | none | No | No | No | Built-in (ephemeral) |

**Reading the table.** kanbanr's unique cell is the combination: *git-authored YAML + a live SSE
monitor + a Claude behavioral contract*. The closest single tool in spirit is **Backlog.md**
(markdown + git, agent-friendly) — but it has no server, no live web monitor, and no multi-user
model. **Task Master AI** is the popularity benchmark for "AI-driven task management" and is
zero-infra via MCP, but it's storage+parsing, not a behavioral system of record with a monitor.
If the user already lives in **GitHub Projects/Linear/Jira**, an MCP server to those may be a
lower-friction choice than adopting a new store — kanbanr wins when you specifically want
*local, diffable, git-authored, agent-owned* state with a watch-along view.

**One strategic question worth answering before promoting it widely:** should kanbanr integrate as
an **MCP server** (so any MCP-capable agent can use it, and Claude can call it without shelling out
to a CLI) in addition to / instead of the skill+CLI? That's the dominant integration pattern in
this space and would broaden reach. The skill+CLI approach is valid and arguably simpler to reason
about, but it's the road less travelled. → [ROADMAP.md](ROADMAP.md).

## 5. "What else can I do better?" — the short list

For a personal tool, ordered by leverage (full detail in [ROADMAP.md](ROADMAP.md)):

1. **Kill the server requirement for everyday use:** an optional **local/serverless mode** where
   the CLI writes the files (and commits) directly, and the server is only started when you *want*
   the live web monitor. This is by far the highest-leverage change.
2. **Meet agents where they are:** consider an **MCP server** interface alongside the skill so
   Claude (and other MCP agents) can call kanbanr natively instead of shelling out.
3. **Make it nice to live in:** silent web token refresh, a **"recent activity" view** built from
   the git history, and small quality-of-life CLI touches (`kanbanr init` one-shot, `kanbanr open`
   to launch the monitor).
4. **Fill the model gaps a solo dev still hits:** priority, due dates, labels, and cross-feature
   dependencies.
5. **Cross-platform:** verify/handle macOS + Windows paths and `HOME`/`~/.kanbanr`.
6. **One paragraph on TLS** for anyone exposing the monitor off localhost (guidance, not code).
7. **Repo hygiene for OSS:** license headers, CI, CHANGELOG, contributor + security docs, release
   automation ([OPEN_SOURCING.md](OPEN_SOURCING.md)).

Deliberately **not** on this list: scaling, conflict policy, rate-limiting, pagination, enterprise
auth — see §3.

## 6. Verdict

As a **personal, agent-owned, git-authored system of record with an optional watch-along monitor**,
kanbanr is well-designed, well-tested, and genuinely differentiated — not a me-too of the existing
AI task tools. Judged on the right bar (friction, not enterprise hardening), its main shortcoming
is simply that the **server is mandatory** when it should be optional. Add a local mode, make a
deliberate call on MCP, and polish the day-to-day ergonomics, and it's a distinctive, genuinely
useful tool to put in front of other solo Claude developers.
