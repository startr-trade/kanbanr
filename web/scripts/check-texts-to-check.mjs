#!/usr/bin/env node
//
// web::texts_to_check (DEF-039 R-1): the Review page shows no sign-off form while texts are left to
// check. It says which are left and that Claude settles them, offers docs-folded only when none is
// left (a skipped check still offers it, as FTR-013 R-9 keeps), and shows what Claude checked.
//
// Usage: npm run check:texts-to-check
import { readFileSync } from "node:fs";
import { docsFoldedOffer } from "../src/textsToCheck.ts";

const flag = (id) => ({ target: { kind: "doc", id, title: id }, via: [], hits: [], commits: [], target_changed_at: "" });
const settled = { flag: flag("b.md"), check: { target: "doc:b.md", outcome: "confirmed", why: "still a date", by: "Claude" } };

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

expect("texts left: no offer", docsFoldedOffer({ to_check: [flag("a.md")], settled: [settled] }).offer, false);
expect("texts left are listed", docsFoldedOffer({ to_check: [flag("a.md")] }).left.length, 1);
expect("none left: offered", docsFoldedOffer({ to_check: [], settled: [settled] }).offer, true);
expect("none left: what Claude settled is kept", docsFoldedOffer({ to_check: [], settled: [settled] }).settled.length, 1);
expect("check skipped: offered", docsFoldedOffer({}).offer, true);

const page = readFileSync(new URL("../src/pages/ReviewPage.tsx", import.meta.url), "utf8");
if (/type="checkbox"/.test(page)) problems.push('ReviewPage.tsx: still has a type="checkbox"');
if (/placeholder="DEF-/.test(page)) problems.push('ReviewPage.tsx: still has a placeholder="DEF-…" input');
if (!/docsFolded\.offer/.test(page)) problems.push("ReviewPage.tsx: docs-folded is not offered by docsFoldedOffer");
if (!/Claude to settle them/.test(page) && !/settle them/.test(page)) {
  problems.push('ReviewPage.tsx: does not say Claude settles the texts left ("settle them")');
}
if (!/Checked by Claude/.test(page)) problems.push("ReviewPage.tsx: does not show what Claude checked");

if (problems.length) {
  for (const p of problems) console.error(`web::texts_to_check — ${p}`);
  process.exit(1);
}
console.log("web::texts_to_check — no form while texts are left; Claude settles them; sign-off once none is left.");
