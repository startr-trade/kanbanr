# kanbanr — Roadmap

> Prioritized improvements, framed for a **personal / single-developer tool** (see
> [ASSESSMENT.md](ASSESSMENT.md) §0). Priorities reflect *friction and day-to-day usefulness*, not
> enterprise hardening. Anything about scaling, multi-writer conflicts, rate-limiting, pagination,
> or enterprise auth is intentionally absent — it's out of scope.

Legend: **P0** = do first (removes friction / unblocks adoption), **P1** = clear value soon,
**P2** = nice to have. Each item notes the rough surface it touches.

---

## P0 — remove the biggest friction

### Local / serverless mode  *(core + cli)*
Today the CLI is a thin HTTP client and the **server is mandatory** — even to add one task. For a
personal tool that's the dominant friction.

- Let the CLI talk to `kanbanr-core`'s store **directly** against the local data dir (commit via
  the vendored git), with **no server running**. The server becomes *optional*, started only when
  you want the live web monitor.
- Resolution: `--local` flag / `KANBANR_LOCAL=1`, or auto-detect "no `KANBANR_SERVER_URL` and a
  data dir is present → local". When a server *is* configured, keep using it (so the monitor stays
  consistent).
- Auth in local mode collapses to "you are this machine's user"; commits can use a configured
  identity (`kanbanr config identity --name --email`) instead of a login.
- Keep the server path as-is for the monitor / sharing.

**Why first:** turns kanbanr from "run a service" into "a CLI that happens to have an optional live
dashboard," which is the right shape for a personal tool.

### Frictionless onboarding  *(cli + docs)*
- `kanbanr init` — one command: pick/confirm identity, create the data dir + git repo, scaffold a
  first project, print next steps.
- `kanbanr open` — launch the monitor (start the server if needed) and open the browser.
- A 60-second "Quickstart" at the top of the README that ends with a populated board.

---

## P1 — make it pleasant to live in

### "Recent activity" view  *(server + web)*
The per-user git history is already there but invisible. Add a read-only timeline (from
`git log`) — "FEAT-003 moved to Scheduled · 2h ago · you" — on the project page. High value,
nearly free, and it makes the watch-along story much stronger.

### Web token refresh  *(web + server)*
A JWT expiry currently drops the monitor to the login screen. Silently re-mint from the stored
credentials (or a refresh token) so a left-open dashboard just keeps working.

### Decide the MCP question  *(new surface — research spike first)*
The dominant integration pattern for AI task tools is an **MCP server**. Evaluate shipping an MCP
interface alongside the skill so Claude (and other MCP agents) call kanbanr natively instead of
shelling out to the CLI. Could wrap the same `kanbanr-core`. Decide explicitly: *skill+CLI only*,
*MCP only*, or *both*. Don't build before deciding.

### Model fields a solo dev still hits  *(core + cli + web)*
- **Priority** (e.g. low/med/high) and **due date** on feature items, surfaced on cards.
- **Labels/tags** for lightweight grouping and filtering.
- **Cross-feature dependencies** ("FEAT-007 blocked by FEAT-004"), with the same cycle-rejection
  the milestone DAG already has.

### Cross-platform  *(core + cli + ci)*
Verify macOS and Windows: `HOME`/`~/.kanbanr` resolution, path separators, file moves on status
change, the vendored git build. Add macOS/Windows runners to CI ([OPEN_SOURCING.md](OPEN_SOURCING.md)).

---

## P2 — nice to have

- **Filter/search in the UI** (by status/label/milestone/text) — useful well before any "scale"
  concern, purely for ergonomics.
- **Markdown niceties:** task checkboxes that link back, spec preview in cards, code-block copy.
- **Export/snapshot:** `kanbanr export-project` to a single markdown bundle (portable report).
- **TLS note in docs** for exposing the monitor beyond localhost (guidance only).
- **Health/readiness endpoint + structured logs** for when you self-host the monitor.
- **Theme/polish:** light theme, mobile-friendly monitor layout, accessibility pass.
- **Editor integration:** a tiny VS Code view or status-bar item reflecting the current board
  (optional; the web monitor already covers most of this).

---

## P3 — enterprise-scale coordination *(team pivot — exploratory)*

Tracked as milestone **MS-005** on kanbanr's own board. This is the "pivot toward teams" the
out-of-scope section below anticipates: **multiple projects with cross-project dependencies**,
derived *ready/blocked* + critical path, portfolio/program rollups, ownership, and **safe
concurrent use** (write lock → single-writer daemon). It deliberately reaches into the "won't do"
list — pursue only if kanbanr takes on multi-project coordination.

Full code-anchored design: [proposals/enterprise-scale.md](proposals/enterprise-scale.md).

---

## Explicitly out of scope (won't do)

Recorded so they don't get re-proposed as "gaps":

- Horizontal scaling / multiple server instances over one data dir.
- Multi-server remote **merge-conflict** resolution (single-server push-to-backup is the model).
- Token-endpoint rate-limiting, brute-force lockout, WAF concerns.
- Pagination / query engine for tens of thousands of items.
- SSO, org/role hierarchies, enterprise RBAC, audit-compliance tooling.

If kanbanr ever pivots toward teams, revisit this section first — most of it would move back in.
That pivot is now sketched under **P3 / MS-005** above ([proposals/enterprise-scale.md](proposals/enterprise-scale.md)).
