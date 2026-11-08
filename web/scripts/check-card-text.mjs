#!/usr/bin/env node
//
// web::card_says_what_is_next (FEAT-205): a board card says in plain words what an item's next
// stage needs, and never "ready" while tasks or sign-offs are outstanding. The same function the
// card renders with is checked here, case by case.
//
// Usage: npm run check:card-text
import { nextStageHint } from "../src/cardText.ts";

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
  if (next.missing > (next.for_you ?? 0) && /waiting/.test(text)) problems.push(`${name}: "${text}" says waiting while work remains`);
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
if (problems.length) {
  for (const p of problems) console.error(`web::card_says_what_is_next — ${p}`);
  process.exit(1);
}
for (const t of Object.keys(named)) console.log(`${t} — ok`);
console.log("web::card_says_what_is_next — cards say what is next in plain words, and never ready while work or a sign-off is due.");
