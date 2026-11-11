#!/usr/bin/env node
//
// web::sprint_filter (FTR-032 R-1, R-2): the board's Sprint filter can show only the items in no
// sprint; All items still shows every item; a project without sprints shows every item and no
// Sprint filter. The same function the page filters with is checked here, case by case.
//
// Usage: npm run check:sprint-filter
import { readFileSync } from "node:fs";
import { ALL, UNSCHEDULED, inSprintFilter } from "../src/sprintFilter.ts";

const items = [
  { code: "A" },
  { code: "B", sprint: null },
  { code: "C", sprint: "" },
  { code: "D", sprint: "SP-001" },
];
const kept = (usesSprints, choice) =>
  items.filter((f) => inSprintFilter(f, usesSprints, choice)).map((f) => f.code).join(",");

const problems = [];
const expect = (name, got, want) => {
  if (got !== want) problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

// R-1
expect("Unscheduled keeps only items in no sprint", kept(true, UNSCHEDULED), "A,B,C");
expect("All items keeps every item", kept(true, ALL), "A,B,C,D");
expect("a sprint keeps only its own items", kept(true, "SP-001"), "D");
expect("a sprint with no items keeps none", kept(true, "SP-002"), "");
// R-2
for (const choice of [ALL, UNSCHEDULED, "SP-001"]) {
  expect(`no sprints: '${choice}' keeps every item`, kept(false, choice), "A,B,C,D");
}
const page = readFileSync(new URL("../src/pages/BoardPage.tsx", import.meta.url), "utf8");
const block = page.slice(page.indexOf("{usesSprints && ("));
if (page.indexOf("{usesSprints && (") < 0 || block.indexOf("<select") < 0 || block.indexOf("<select") > block.indexOf(")}")) {
  problems.push("BoardPage.tsx: the Sprint <select> is no longer inside the usesSprints block");
}
if (!/<option value=\{UNSCHEDULED\}>Unscheduled<\/option>/.test(page)) {
  problems.push("BoardPage.tsx: no Unscheduled option in the Sprint filter");
}

if (problems.length) {
  for (const p of problems) console.error(`web::sprint_filter — ${p}`);
  process.exit(1);
}
console.log("web::sprint_filter — Unscheduled keeps items in no sprint; All keeps all; no sprints, no filter.");
