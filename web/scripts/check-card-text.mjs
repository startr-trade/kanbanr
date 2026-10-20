#!/usr/bin/env node
//
// web::card_says_what_is_next (FEAT-205): a board card says in plain words what an item's next
// stage needs, and never "ready" while tasks or sign-offs are outstanding. The same function the
// card renders with is checked here, case by case.
//
// Usage: npm run check:card-text
import { nextStageHint } from "../src/cardText.ts";

const cases = [
  ["conditions unmet", { status: "In Progress", missing: 3, signoffs: [], tasks_open: 0, ends: false }, "In Progress needs 3"],
  ["tasks open: say nothing", { status: "Completed", missing: 1, signoffs: ["docs-folded"], tasks_open: 1, ends: true }, null],
  ["only a sign-off due", { status: "Completed", missing: 1, signoffs: ["docs-folded"], tasks_open: 0, ends: true }, "waiting for sign-off docs-folded"],
  ["conditions and a sign-off", { status: "In Progress", missing: 3, signoffs: ["design-review"], tasks_open: 0, ends: false }, "In Progress needs 2"],
  ["ready for a middle stage", { status: "In Progress", missing: 0, signoffs: [], tasks_open: 0, ends: false }, "ready for In Progress"],
  ["ready at the end", { status: "Completed", missing: 0, signoffs: [], tasks_open: 0, ends: true }, "ready to complete"],
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
if (problems.length) {
  for (const p of problems) console.error(`web::card_says_what_is_next — ${p}`);
  process.exit(1);
}
console.log("web::card_says_what_is_next — cards say what is next in plain words, and never ready while work or a sign-off is due.");
