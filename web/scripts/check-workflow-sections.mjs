#!/usr/bin/env node
//
// web::workflow_shows_cadence_git_model_and_guards (FEAT-213 R-3): the Workflow page shows the
// cadence, the git model and the guard rails, each line with the command that changes it, and
// changes nothing itself.
//
// Runs the page's pure section builders over what the daemon answers for a project with sprints,
// releases, the git model on, strict diagrams and one coverage rule, and asserts what they say; then
// reads the page to check it draws them and offers no action.
//
// Usage: npm run check:workflow
import { readFileSync } from "node:fs";
import { cadenceSection, doctorWarning, gitModelRows, guardRows } from "../src/workflowSections.ts";

const problems = [];
const expect = (cond, what) => {
  if (!cond) problems.push(what);
};

// ---- Cadence ----
const config = {
  name: "shop",
  description: "",
  statuses: [],
  default_state: "",
  transitions: {},
  displayed_states: [],
  no_op_states: [],
  cadence: {
    sprints: true,
    releases: true,
    sprint_length_days: 14,
    release: "per_sprint",
    release_stages: ["candidate", "test", "production"],
    git_model: true,
  },
};
const sprints = [
  { code: "SP-001", start: "2026-10-05", end: "2026-10-18", state: "closed" },
  { code: "SP-002", name: "Polish", start: "2026-10-19", end: "2026-11-01", state: "active" },
];
const releases = [
  { version: "v0.1.0", state: "shipped" },
  { version: "v0.2.0", state: "planned", stage: "test" },
  { version: "v0.3.0", state: "planned" },
];
const cadence = cadenceSection("shop", config, sprints, releases);
expect(cadence !== null, "cadence: a project with sprints and releases has a Cadence section");
expect(cadence?.sprints?.rows[0]?.value === "14 days", `cadence: sprint length reads ${cadence?.sprints?.rows[0]?.value}`);
expect(cadence?.sprints?.active?.code === "SP-002", "cadence: the running sprint is shown");
expect(cadence?.sprints?.active?.dates === "2026-10-19 to 2026-11-01", "cadence: the running sprint's dates are shown");
expect(cadence?.sprints?.active?.to === "/p/shop", "cadence: the running sprint links to the board, which opens on it");
expect(
  JSON.stringify(cadence?.releases?.stages) ===
    JSON.stringify([
      { name: "candidate", cut: false },
      { name: "test", cut: false },
      { name: "production", cut: true },
    ]),
  `cadence: the stages in order, the last marked the cut: ${JSON.stringify(cadence?.releases?.stages)}`,
);
expect(
  JSON.stringify(cadence?.releases?.open.map((r) => [r.version, r.stage])) ===
    JSON.stringify([
      ["v0.2.0", "test"],
      ["v0.3.0", "not started"],
    ]),
  `cadence: each unshipped release with its stage: ${JSON.stringify(cadence?.releases?.open)}`,
);
expect(cadence?.releases?.open.every((r) => r.to === "/p/shop/releases"), "cadence: releases link to the Releases page");
expect(cadence?.releases?.rows[0]?.value === "a release each sprint", "cadence: how often releases are cut");
expect(
  cadenceSection("shop", { ...config, cadence: { sprints: false, releases: false } }, [], []) === null,
  "cadence: a project with neither sprints nor releases has no Cadence section",
);
const sprintsOnly = cadenceSection("shop", { ...config, cadence: { sprints: true } }, [], []);
expect(sprintsOnly?.releases === null && sprintsOnly?.sprints?.active === null, "cadence: sprints only, none running");
for (const row of [...(cadence?.sprints?.rows ?? []), ...(cadence?.releases?.rows ?? [])]) {
  expect(row.command?.startsWith("kanbanr "), `cadence: "${row.label}" names its command`);
}

// ---- Git model ----
const model = {
  topology: "sprints_and_releases",
  line: "sprint/SP-002",
  branch_from: "sprint/SP-002",
  production_at: "release_cut",
  kanbanr_merges: ["each item branch into sprint/SP-002 when the item enters the integrate stage"],
  automated: true,
  pushes: "you",
  model: {
    main: "main",
    line: "sprint/SP-002",
    item_patterns: ["feat/{code}-{slug}", "fix/{code}-{slug}", "spike/*"],
    protected: ["main", "sprint/*", "release/*"],
  },
  facts: { main: "main" },
};
const gitRows = gitModelRows(model);
const row = (rows, label) => rows.find((r) => r.label === label);
expect(row(gitRows, "Main")?.value === "main", "git model: main");
expect(row(gitRows, "Line of integration")?.value === "sprint/SP-002", "git model: the line of integration");
expect(row(gitRows, "Item branches from")?.value === "sprint/SP-002", "git model: where items branch from");
expect(row(gitRows, "Production moves")?.value === "when a release is cut", "git model: where production moves");
expect(row(gitRows, "Item branch names")?.value.includes("feat/{code}-{slug}"), "git model: item branch names");
expect(row(gitRows, "Protected (merges only)")?.value === "main, sprint/*, release/*", "git model: protected branches");
expect(row(gitRows, "kanbanr merges")?.value.includes("integrate stage"), "git model: what kanbanr merges and when");
expect(row(gitRows, "Pushing")?.value.includes("never pushes"), "git model: pushing is yours");
const automation = row(gitRows, "Automation");
expect(automation?.tone === "on" && automation?.command === "kanbanr config cadence --git-model off", "git model: automation on, and the command that switches it off");
const manual = gitModelRows({ ...model, automated: false });
expect(row(manual, "Automation")?.command === "kanbanr config cadence --git-model on", "git model: automation off, and the command that switches it on");
expect(row(manual, "kanbanr merges")?.value.startsWith("nothing"), "git model: with automation off kanbanr merges nothing");

expect(doctorWarning([]) === null, "git doctor: nothing found, no warning");
const issues = ["a", "b", "c", "d"].map((m, i) => ({ severity: "warning", project: "shop", code: i === 0 ? "FEAT-001" : null, message: m }));
const warning = doctorWarning(issues);
expect(warning?.count === 4 && warning.first.length === 3, `git doctor: the count and the first few: ${JSON.stringify(warning)}`);
expect(warning?.first[0] === "FEAT-001: a", "git doctor: a problem names its item");

// ---- Guard rails ----
const guards = {
  method: ["design", "design-review", "docs", "docs-folded", "links"].map((part) => ({
    part,
    on: part !== "docs-folded",
    status: part === "links" ? undefined : "In Progress",
    means: "",
  })),
  commits: {
    item_branch: true,
    branch_pattern: "feat/{code}-{slug}",
    trailer: "Refs",
    escape: "[no-ref] <reason>",
    protected: ["main", "sprint/*", "release/*"],
    hooks_installed: null,
  },
  diagrams: { lint_on_write: true, strict: true },
  coverage: [{ id: "listed", subjects: 3, uncovered: 1 }],
  push_policy: { policy: "off", source: "board" },
  code_pushes: "you",
};
const rails = guardRows(guards);
const group = (g) => rails.filter((r) => r.group === g);
expect(group("Method").length === 5, "guard rails: every part of the method");
expect(group("Method").find((r) => r.label === "docs-folded")?.command === "kanbanr config method docs-folded --on", "guard rails: an off part says how to turn it on");
expect(group("Method").find((r) => r.label === "design")?.command === "kanbanr config method design --off", "guard rails: an on part says how to turn it off");
expect(group("Commits").some((r) => r.label === "only on an item branch" && r.value.includes("feat/{code}-{slug}")), "guard rails: commits only on an item branch");
expect(group("Commits").some((r) => r.label.startsWith("Refs") && r.value.includes("kanbanr:<CODE>")), "guard rails: the Refs trailer");
expect(group("Commits").some((r) => r.label === "never on a protected branch" && r.value === "main, sprint/*, release/*"), "guard rails: never on a protected branch");
const hooks = group("Commits").find((r) => r.label === "hooks in this repository");
expect(hooks?.tone === "unknown" && hooks.command === "kanbanr git status", "guard rails: hooks not known here say which command tells");
const installed = guardRows({ ...guards, commits: { ...guards.commits, hooks_installed: false } });
expect(installed.find((r) => r.label === "hooks in this repository")?.command === "kanbanr git install-hooks", "guard rails: missing hooks say how to install them");
expect(group("Diagrams").find((r) => r.label === "strict")?.command === "kanbanr config diagrams --strict off", "guard rails: strict diagrams, and how to relax them");
expect(group("Coverage").length === 1 && group("Coverage")[0].value === "1 of 3 uncovered" && group("Coverage")[0].tone === "warn", "guard rails: each coverage rule with what it leaves uncovered");
const none = guardRows({ ...guards, coverage: [] });
expect(none.find((r) => r.group === "Coverage")?.value === "none", "guard rails: no coverage rules says so");
expect(group("Pushing").find((r) => r.label === "the board")?.value === "off (this board's setting)", "guard rails: the board's push policy and where it came from");
expect(group("Pushing").find((r) => r.label === "the code")?.value.includes("never by kanbanr"), "guard rails: kanbanr never pushes code");
for (const r of rails) {
  expect(/^(kanbanr|git) /.test(r.command), `guard rails: "${r.group} / ${r.label}" names its command (${r.command})`);
}

// ---- The page draws them, and offers nothing to click ----
const page = readFileSync(new URL("../src/pages/WorkflowPage.tsx", import.meta.url), "utf8");
for (const builder of ["cadenceSection", "gitModelRows", "doctorWarning", "guardRows"]) {
  expect(page.includes(`${builder}(`), `the page draws ${builder}`);
}
for (const heading of ["Cadence", "Git model", "Guard rails"]) {
  expect(page.includes(`<h2>${heading}</h2>`), `the page has a ${heading} section`);
}
expect(page.includes("<code>{row.command}</code>"), "the page shows each command as code");
expect(!/<button|onClick|writePost|postWrite|api\.(approve|signoff|promote|plan|add|decide|ratify|unapprove)/.test(page), "the Workflow page offers no action: it is read-only");

if (problems.length) {
  for (const p of problems) console.error(`web::workflow_shows_cadence_git_model_and_guards — ${p}`);
  process.exit(1);
}
console.log(
  "web::workflow_shows_cadence_git_model_and_guards — the Workflow page shows the cadence, the git model and every guard rail, each with its command, read-only.",
);
