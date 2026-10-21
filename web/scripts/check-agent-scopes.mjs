#!/usr/bin/env node
//
// web::check-agent-scopes (FEAT-214 R-10): the monitor shows each project's agent scopes and their
// state — on the item page (a banner for each scope in force, then the item's history), on the
// Workflow page (an "Agent scopes" group of the guard rails), and on every activity line made by
// an agent — and offers no action on them: scopes are the CLI's to issue and revoke.
//
// Runs the pure helpers the pages draw with over scopes in each state, then reads the pages to check
// they use them.
//
// Usage: npm run check:agent-scopes
import { readFileSync } from "node:fs";
import { byAgentText, liveScopes, scopeBanner, scopeRows } from "../src/agentScopes.ts";
import { guardRows } from "../src/workflowSections.ts";

const problems = [];
const expect = (cond, what) => {
  if (!cond) problems.push(what);
};

const now = Date.parse("2026-10-21T12:00:00Z");
const base = {
  project: "shop",
  operations: ["task.state", "test.state", "commit"],
  issued_by: "Ada",
  note: "",
};
const scopes = [
  {
    ...base,
    id: "SC-001",
    role: "build",
    item: "FEAT-001",
    branch: "feat/FEAT-001-cart",
    issued_at: "2026-10-21T08:00:00Z",
    expires_at: "2026-10-21T16:00:00Z",
    worktree: ".claude/worktrees/agent-a",
    claimed_at: "2026-10-21T08:01:00Z",
    state: "active",
  },
  {
    ...base,
    id: "SC-002",
    role: "design",
    item: "FEAT-001",
    operations: [],
    issued_at: "2026-10-21T09:00:00Z",
    expires_at: "2026-10-21T17:00:00Z",
    state: "issued",
  },
  {
    ...base,
    id: "SC-003",
    role: "build",
    item: "FEAT-001",
    branch: "feat/FEAT-001-cart",
    issued_at: "2026-10-20T08:00:00Z",
    expires_at: "2026-10-20T16:00:00Z",
    revoked_at: "2026-10-20T15:00:00Z",
    revoked_by: "Ada",
    revoked_reason: "done",
    state: "revoked",
  },
  {
    ...base,
    id: "SC-004",
    role: "build",
    item: "FEAT-001",
    branch: "feat/FEAT-001-cart",
    issued_at: "2026-10-19T08:00:00Z",
    expires_at: "2026-10-19T16:00:00Z",
    state: "expired",
  },
  { ...base, id: "SC-005", role: "build", item: "FEAT-002", branch: "feat/FEAT-002-x", issued_at: "2026-10-21T10:00:00Z", expires_at: "2026-10-21T18:00:00Z", state: "issued" },
];

// ---- the item's history: its own scopes only, newest first, each with how it stands ----
const rows = scopeRows(scopes, "FEAT-001", now);
expect(rows.map((r) => r.id).join(",") === "SC-002,SC-001,SC-003,SC-004", `rows newest first, this item's only: ${rows.map((r) => r.id)}`);
const row = (id) => rows.find((r) => r.id === id);
expect(row("SC-001").ends === "expires in 4h", `time left: ${row("SC-001").ends}`);
expect(row("SC-001").covers === "feat/FEAT-001-cart", "a build scope covers its branch");
expect(row("SC-002").covers === "reads only", "a design scope reads only");
expect(row("SC-003").ends === "revoked by Ada 2026-10-20 15:00 UTC (done)", `revoked: ${row("SC-003").ends}`);
expect(row("SC-004").ends === "expired 2026-10-19 16:00 UTC", `expired: ${row("SC-004").ends}`);
expect(row("SC-001").issued === "by Ada 2026-10-21 08:00 UTC", `issued: ${row("SC-001").issued}`);
expect(scopeRows(scopes, undefined, now).length === 5, "a project's rows: every item's");

// ---- the banner: who, where, since when, until when, and how far ahead of the board ----
const live = liveScopes(scopes.filter((s) => s.item === "FEAT-001"));
expect(live.map((s) => s.id).join(",") === "SC-001,SC-002", `in force: ${live.map((s) => s.id)}`);
const banner = scopeBanner(scopes[0], now, 2);
for (const want of [
  "build agent SC-001 on feat/FEAT-001-cart",
  "working in .claude/worktrees/agent-a",
  "issued by Ada 2026-10-21 08:00 UTC",
  "expires in 4h",
  "2 commits ahead of its tasks",
]) {
  expect(banner.includes(want), `the banner says "${want}": ${banner}`);
}
expect(!scopeBanner(scopes[0], now, 0).includes("ahead"), "no drift, nothing said");
expect(scopeBanner(scopes[1], now).includes("design agent SC-002 (reads only) · not yet claimed"), scopeBanner(scopes[1], now));

// ---- activity: the person, by the agent for its item ----
expect(
  byAgentText({ actor: "Ada", by_agent: { scope: "SC-001", role: "build", item: "FEAT-001" } }) ===
    "Ada, by the agent for FEAT-001 (SC-001)",
  "an agent's change names the agent",
);
expect(byAgentText({ actor: "Ada" }) === "Ada", "anyone else's names the person");

// ---- the Workflow page's guard rails ----
const guards = {
  method: [],
  commits: { item_branch: true, branch_pattern: "feat/{code}-{slug}", trailer: "Refs", escape: "[no-ref] <reason>", protected: ["main"], hooks_installed: null },
  diagrams: { lint_on_write: true, strict: false },
  coverage: [],
  push_policy: { policy: "off", source: "board" },
  code_pushes: "you",
  agents: {
    roles: [
      { role: "design", operations: [] },
      { role: "build", operations: ["task.state", "test.state", "commit"] },
    ],
    scopes: [scopes[0], scopes[4]],
    push_hook_installed: null,
  },
};
const agentRows = guardRows(guards).filter((r) => r.group === "Agent scopes");
const labels = agentRows.map((r) => r.label);
for (const want of ["design", "build", "SC-001", "SC-005", "pre-push hook"]) {
  expect(labels.includes(want), `the Agent scopes group lists ${want}: ${labels}`);
}
const design = agentRows.find((r) => r.label === "design");
expect(design.value === "reads only", `design: ${design.value}`);
const build = agentRows.find((r) => r.label === "build");
expect(build.value === "task.state, test.state, commit", `build: ${build.value}`);
const sc1 = agentRows.find((r) => r.label === "SC-001");
expect(sc1.value.includes("FEAT-001") && sc1.value.includes("active"), `a scope row: ${sc1.value}`);
expect(sc1.command === "kanbanr scope revoke SC-001 --reason done", `its command: ${sc1.command}`);
expect(guardRows({ ...guards, agents: undefined }).every((r) => r.group !== "Agent scopes") === false, "with no agents field, the group still says how to issue one");

// ---- the pages draw them, and change nothing ----
const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const feature = read("../src/pages/FeaturePage.tsx");
expect(feature.includes("<h2>Agents</h2>"), "the item page has an Agents section");
for (const fn of ["scopeBanner", "scopeRows", "liveScopes", "byAgentText"]) {
  expect(feature.includes(`${fn}(`), `the item page draws with ${fn}`);
}
expect(read("../src/pages/BoardPage.tsx").includes("byAgentText(a)"), "the board's activity names the agent");
expect(read("../src/workflowSections.ts").includes('group: "Agent scopes"'), "the Workflow page groups them");
for (const [name, text] of [["FeaturePage.tsx", feature], ["agentScopes.ts", read("../src/agentScopes.ts")]]) {
  expect(!/api\.(issue|claim|revoke|renew)Scope/.test(text), `${name} offers no scope action`);
}

if (problems.length) {
  for (const p of problems) console.error(`web::check-agent-scopes — ${p}`);
  process.exit(1);
}
console.log("web::check-agent-scopes — the monitor shows every scope and its state, on the item page, the Workflow page and the activity, and changes none.");
