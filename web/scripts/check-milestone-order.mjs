#!/usr/bin/env node
//
// web::milestone_order (FTR-031 R-3): the Milestones page lists milestones newest first, the
// highest code first; a milestone's page and its Plan row keep their dependency order (the Plan
// row's order is core's graph::order_by_deps since FTR-043 removed the Schedule page).
//
// Usage: npm run check:milestone-order
import { readFileSync } from "node:fs";
import { newestFirst } from "../src/milestoneOrder.ts";

const order = (codes) => newestFirst(codes.map((code) => ({ code }))).map((m) => m.code).join(",");
const problems = [];
const expect = (name, got, want) => {
  if (got !== want) problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

expect("MS-001, MS-002, MS-010", order(["MS-001", "MS-002", "MS-010"]), "MS-010,MS-002,MS-001");
expect("numeric, not textual", order(["MS-9", "MS-10"]), "MS-10,MS-9");
expect("number then text", order(["REL-2", "MS-3"]), "MS-3,REL-2");
expect("equal numbers by text, descending", order(["MS-3", "REL-3"]), "REL-3,MS-3");
expect("unnumbered last", order(["ALPHA", "MS-1", "BETA"]), "MS-1,BETA,ALPHA");
expect("empty", order([]), "");
const input = [{ code: "MS-1" }, { code: "MS-2" }];
newestFirst(input);
expect("input untouched", input[0].code, "MS-1");

const src = (f) => readFileSync(new URL(`../src/pages/${f}`, import.meta.url), "utf8");
if (!src("MilestonesPage.tsx").includes("newestFirst(data.milestones)")) {
  problems.push("MilestonesPage.tsx does not render newestFirst(data.milestones)");
}
for (const f of ["plan/RowItems.tsx", "MilestonePage.tsx"]) {
  if (src(f).includes("newestFirst(")) problems.push(`${f} must keep its own order, not call newestFirst(`);
}

if (problems.length) {
  for (const p of problems) console.error(`web::milestone_order — ${p}`);
  process.exit(1);
}
console.log("web::milestone_order — the Milestones page lists newest first; a milestone and its Plan row keep dependency order.");
