#!/usr/bin/env node
//
// web::card_says_what_is_next (FEAT-205): a board card says in plain words what an item's next
// stage needs, and never "ready" while tasks or sign-offs are outstanding. The same function the
// card renders with is checked here, case by case.
//
// Usage: npm run check:card-text
import { delegatedHints, nextStageHint } from "../src/cardText.ts";
import { closeRender, render } from "./lib/render.mjs";

// [name, next stage, expected text]
const cases = [
  ["conditions unmet", { status: "In Progress", missing: 3, signoffs: [], tasks_open: 0, ends: false }, "In Progress needs 3"],
  ["tasks open: say nothing", { status: "Completed", missing: 1, signoffs: ["docs-folded"], for_you: 1, tasks_open: 1, ends: true }, null],
  ["only a sign-off due", { status: "Completed", missing: 1, signoffs: ["docs-folded"], for_you: 1, tasks_open: 0, ends: true }, "waiting on you: docs-folded"],
  ["only approval due", { status: "Migration", missing: 1, signoffs: [], for_you: 1, tasks_open: 0, ends: false }, "waiting on you: approval"],
  ["conditions and a sign-off", { status: "In Progress", missing: 3, signoffs: ["design-review"], for_you: 1, tasks_open: 0, ends: false }, "In Progress needs 3, 1 yours"],
  ["ready for a middle stage", { status: "In Progress", missing: 0, signoffs: [], for_you: 0, tasks_open: 0, ends: false }, "ready for In Progress"],
  ["ready at the end", { status: "Completed", missing: 0, signoffs: [], for_you: 0, tasks_open: 0, ends: true }, "ready to complete"],
  ["not the immediate stage", { status: "Operations", missing: 2, signoffs: ["release"], for_you: 1, tasks_open: 0, ends: true, immediate: false }, null],
  ["older daemon: fields absent", { status: "Migration", missing: 2, signoffs: ["docs-folded"], tasks_open: 0, ends: false }, "Migration needs 2"],
  ["the walkthrough: six against four", { status: "Migration", missing: 6, signoffs: ["docs-folded"], for_you: 1, tasks_open: 0, ends: false, immediate: true }, "Migration needs 6, 1 yours"],
];
const problems = [];
for (const [name, next, want] of cases) {
  const got = nextStageHint(next)?.text ?? null;
  if (got !== want) problems.push(`${name}: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`);
}
// Nothing a card says may look like a status pill: no arrow, no tick.
for (const [, next] of cases) {
  const text = nextStageHint(next)?.text ?? "";
  if (/[→✓]/.test(text)) problems.push(`"${text}" still reads like a status pill`);
}
// A card never says it waits while Claude's work remains.
for (const [name, next] of cases) {
  const text = nextStageHint(next)?.text ?? "";
  const held = (next.delegated ?? []).length;
  if (next.missing - held > (next.for_you ?? 0) && /waiting/.test(text)) problems.push(`${name}: "${text}" says waiting while work remains`);
}
// Named outputs for the board's tests.
const named = {
  "web::card_counts_match_check": "the walkthrough: six against four",
  "web::card_names_only_the_immediate_stage": "not the immediate stage",
  "web::card_says_who_it_waits_on": "only a sign-off due",
};
for (const [test, caseName] of Object.entries(named)) {
  if (!cases.some(([n]) => n === caseName)) problems.push(`${test}: case "${caseName}" is missing`);
}

// ---- web::check_card_shows_delegated_gaps_as_claudes (DEF-104, R-3) -----------------------------
const NOW = new Date("2026-11-17T12:00:00Z");
const AHEAD = "2026-12-04T23:59:59Z";
const PAST = "2026-11-17T11:59:59Z";
const gap = (check, name, until = AHEAD, delegation = "DLG-001") => ({ check, name, delegation, until });
const dr = gap("signoff", "design-review");
const ap = gap("approved", "");
// [name, next stage, expected hint text, expected delegated-hint texts]
const held = [
  ["held beside Claude's work and a sign-off of yours", { status: "In Progress", missing: 3, signoffs: ["design-review", "docs-folded"], for_you: 1, tasks_open: 0, delegated: [dr] }, "In Progress needs 3, 1 yours", ["design review: Claude, under DLG-001"]],
  ["only a held gap left", { status: "In Progress", missing: 1, signoffs: ["design-review"], for_you: 0, tasks_open: 0, delegated: [dr] }, null, ["design review: Claude, under DLG-001"]],
  ["held and yours", { status: "Completed", missing: 2, signoffs: ["design-review", "docs-folded"], for_you: 1, tasks_open: 0, delegated: [dr] }, "waiting on you: docs-folded", ["design review: Claude, under DLG-001"]],
  ["two held gaps", { status: "In Progress", missing: 2, signoffs: ["design-review"], for_you: 0, tasks_open: 0, delegated: [dr, ap] }, null, ["design review, approval: Claude, under DLG-001"]],
  ["held, tasks open", { status: "In Progress", missing: 1, signoffs: ["design-review"], for_you: 0, tasks_open: 2, delegated: [dr] }, null, ["design review: Claude, under DLG-001"]],
  ["the delegation ended since the read", { status: "In Progress", missing: 1, signoffs: ["design-review"], for_you: 0, tasks_open: 0, delegated: [gap("signoff", "design-review", PAST)] }, "waiting on you: design-review", []],
  ["two delegations", { status: "In Progress", missing: 2, signoffs: ["design-review"], for_you: 0, tasks_open: 0, delegated: [dr, gap("approved", "", AHEAD, "DLG-002")] }, null, ["design review: Claude, under DLG-001", "approval: Claude, under DLG-002"]],
  ["not the immediate stage", { status: "Operations", missing: 1, signoffs: ["release"], for_you: 0, tasks_open: 0, immediate: false, delegated: [gap("signoff", "release")] }, null, []],
  ["older daemon: no delegated", { status: "Completed", missing: 1, signoffs: ["design-review"], for_you: 1, tasks_open: 0 }, "waiting on you: design-review", []],
];
for (const [name, next, wantHint, wantDelegated] of held) {
  const got = nextStageHint(next, NOW)?.text ?? null;
  if (got !== wantHint) problems.push(`${name}: hint expected ${JSON.stringify(wantHint)}, got ${JSON.stringify(got)}`);
  const hints = delegatedHints(next, NOW).map((h) => h.text);
  if (JSON.stringify(hints) !== JSON.stringify(wantDelegated)) problems.push(`${name}: delegated hints expected ${JSON.stringify(wantDelegated)}, got ${JSON.stringify(hints)}`);
  if (/waiting/.test(hints.join(" "))) problems.push(`${name}: a delegated hint says waiting`);
  for (const h of delegatedHints(next, NOW)) {
    if (h.kind !== "delegated" || !/until 4 Dec/.test(h.title) || !/Review page/.test(h.title)) problems.push(`${name}: delegated hint title "${h.title}"`);
  }
}
// The card draws a held gap as a link to the delegation, with the title still the way in.
const feature = { code: "FTR-9", title: "A held thing", done: 0, total: 0 };
const card = async (next) => (await render("src/components/bits.tsx", "FeatureCard", { project: "p", feature, next: [next] })).document;
const heldNext = { status: "In Progress", missing: 1, signoffs: ["design-review"], for_you: 0, tasks_open: 0, delegated: [{ ...dr, until: "2099-01-01T00:00:00Z" }] };
const doc = await card(heldNext);
const link = doc.querySelector("a.hint-delegated");
if (!link) problems.push("the card has no a.hint-delegated");
else {
  if (link.getAttribute("href") !== "/p/p/review#DLG-001") problems.push(`the delegated link goes to ${link.getAttribute("href")}`);
  if (!link.textContent.includes("design review: Claude, under DLG-001")) problems.push(`the delegated link reads "${link.textContent}"`);
}
if (/waiting on you/.test(doc.body.textContent)) problems.push("a held card says 'waiting on you'");
const title = doc.querySelector("h3.card-title a.card-link");
if (!title || !title.getAttribute("href").endsWith("/feature/FTR-9") || title.textContent !== "A held thing") problems.push("h3.card-title does not hold the title link");
if (!doc.querySelector("article.card")) problems.push("the card is not an article.card");
const back = await card({ ...heldNext, for_you: 1, delegated: [{ ...dr, until: "2000-01-01T00:00:00Z" }] });
if (!/waiting on you: design-review/.test(back.body.textContent)) problems.push("an ended delegation's gap does not read as waiting on you");
if (back.querySelector("a.hint-delegated")) problems.push("an ended delegation still has a delegated link");
await closeRender();

if (problems.length) {
  for (const p of problems) console.error(`web::card_says_what_is_next|check_card_shows_delegated_gaps_as_claudes — ${p}`);
  process.exit(1);
}
for (const t of Object.keys(named)) console.log(`${t} — ok`);
console.log("web::check_card_shows_delegated_gaps_as_claudes — a gap a delegation holds reads as Claude's, with a link, and never as waiting on you.");
console.log("web::card_says_what_is_next — cards say what is next in plain words, and never ready while work or a sign-off is due.");
