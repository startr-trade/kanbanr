# Decision: MCP server interface (FEAT-005)

> A decision record for the **MCP question** raised in [ROADMAP.md](../ROADMAP.md) ("Decide the
> MCP question") and [ASSESSMENT.md](../ASSESSMENT.md) §4. Format: Context / Options / Decision /
> Consequences.

**Status:** Accepted · **Date:** 2026-06-12

## Context

kanbanr is driven by Claude through the **kanbanr skill**, which shells out to the local `kanbanr`
CLI. The CLI is the single writer: it links `kanbanr-core` and applies every mutation through the
`dispatch` router (`(method, path, body) → store op`), commits the git-backed data folder, and
pushes optional remotes. `kanbanr serve` runs a read-only, localhost view daemon over the same
folder, also via `dispatch`. There is no server in the write path, no accounts, no login.

The **Model Context Protocol (MCP)** is the dominant integration pattern for AI task tools (Task
Master AI, GitHub Projects, Linear, Jira all ship MCP servers). An MCP server would let *any*
MCP-capable agent — not just Claude Code with the skill — call kanbanr natively as tools, instead
of an agent shelling out to a CLI. The question, per the roadmap, is explicit: **skill+CLI only,
MCP only, or both** — decide before building.

A key constraint: kanbanr is **local-only by design**. The writer is "you on this machine"; there
is no auth because there is nothing to authenticate against. Any MCP surface inherits that process
model — it must run locally, against the local data folder, as the same single writer.

## Options

**(a) skill+CLI only (status quo).** Keep the skill as the sole integration. Simple, already works,
nothing new to maintain. The behavioral contract (the real product) lives in `SKILL.md`; the CLI is
plumbing. Limitation: reach is effectively Claude Code (or anything that can run the skill + a
local binary); non-Claude MCP agents can't call kanbanr natively.

**(b) MCP server, replacing the skill+CLI.** Expose `kanbanr-core`/`dispatch` only over MCP. Broad
agent reach, but throws away the working, well-tested skill+CLI path and the CLI's value as a
human-usable tool. The behavioral contract would have to be re-encoded for MCP clients with no
guarantee they honor it. High cost, real regression — rejected outright.

**(c) both.** Keep skill+CLI as-is; add a **thin MCP server** that wraps the same
`kanbanr-core`/`dispatch` engine. Because every mutation already funnels through `dispatch`, the MCP
shim is genuinely thin — it maps MCP tool calls onto the same `(method, path, body)` shapes the CLI
and daemon already use, so all three agree by construction. Broadens reach to any MCP agent without
touching the existing path. Cost: one more surface to maintain and document, and a process-model
question (the writer is local-only, so the MCP server must be a local stdio server running as the
same single writer — not a network service).

## Decision

**Adopt (c) — both — but stage it: ship the thin MCP server as an additive, optional surface,
keeping skill+CLI as the primary, default path.**

Rationale:

- The skill+CLI path **already works, is simple, and is the differentiator** — the behavioral
  contract in `SKILL.md` is the moat, not the transport. We don't disturb it (rules out (b)).
- Because `dispatch` is already the single source of truth for data operations, an MCP shim over it
  is **thin and low-risk** — it reuses the engine the CLI and daemon share, so correctness comes for
  free. This is the cheap part of (c).
- It **broadens reach to non-Claude-Code agents** (the dominant pattern in this space) at marginal
  cost, which directly answers the strategic question in ASSESSMENT.md §4.
- Staging it (additive, off the critical path, default stays skill+CLI) means the new surface is
  opt-in: if it doesn't earn its keep we can drop it without affecting day-to-day use.

We explicitly do **not** make MCP the only or default interface, and we do **not** turn the MCP
server into a remote/network service — it stays local, matching kanbanr's "you are this machine's
user" model.

## Implementation sketch (recommended path)

- Add a small `kanbanr-mcp` surface — ideally a new `kanbanr mcp` subcommand on the existing binary
  (one runtime, no extra install), speaking **MCP over stdio** so the host agent spawns it locally.
- It links `kanbanr-core` and **calls `dispatch` directly** — the same `(method, path, body)` ops
  the CLI uses. No HTTP, no new validation: transitions, milestone-required, DAG cycles, and
  referential integrity are already enforced in the engine.
- Expose a focused tool set that mirrors `dispatch`'s write ops plus reads: `feature.add/edit/move`,
  `milestone.add`, `todo.add`, `task.add/state`, `doc.folder/write`, `board`/project read, and the
  existing **`batch`** op so an agent can bundle changes into one commit. Each tool call commits the
  data repo under the configured identity, exactly like the CLI.
- It is the **same single writer**: run locally against the local data dir; do not expose it on a
  network. (The read-only view daemon stays the only HTTP surface.)
- Carry the behavioral contract across: ship a short MCP-side description (or reference `SKILL.md`)
  so MCP clients get the same "single system of record / persistent todo-lists / resume each
  session" guidance the skill encodes.

## Consequences

- **Positive:** any MCP-capable agent can drive kanbanr natively; the implementation is thin
  because it reuses `dispatch`; the proven skill+CLI path is untouched and remains the default;
  one binary, one engine, three callers (CLI, view daemon, MCP) that agree by construction.
- **Negative / cost:** one more interface to maintain, test, and document; a second place the
  behavioral contract must be conveyed; a stdio process-model the host agent must launch and manage.
- **Out of scope (unchanged):** MCP does **not** become a remote/network service, does **not** add
  auth or a write-over-HTTP path, and does **not** replace the skill+CLI. kanbanr stays local-only.

## Revisit when…

- A concrete **non-Claude-Code MCP agent** is a real target for kanbanr (the clearest trigger to
  actually build (c) rather than just having decided it).
- The skill+CLI path starts feeling like friction for agents that natively prefer MCP tools.
- kanbanr ever pivots toward **teams / a hosted deployment** — at which point the local-only
  constraint, and therefore the MCP process model and auth question, must be reopened (see
  [ROADMAP.md](../ROADMAP.md) "Explicitly out of scope").
