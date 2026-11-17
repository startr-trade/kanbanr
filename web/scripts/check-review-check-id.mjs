#!/usr/bin/env node
//
// web::check_review_page_shows_a_results_check_id (DEF-089 R-3, R-5): each result a walkthrough
// prepares carries the runbook's check id (MT-n), and the Review page shows it as a plain field row
// "Check" beside Result, Note and Checked at; the batch summary names the batch's checks, as
// "(checks MT-1, MT-4)". The pure helpers are checked case by case; the page is read for where it
// renders them.
//
// Usage: npm run check:review-check-id
import { readFileSync } from "node:fs";
import { batchChecks, fieldLabel } from "../src/prepared.ts";

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const view = (id, item, check) => ({
  id,
  kind: "manual-test",
  subject: { item, requirement: "R-1", test: "manual: x" },
  fields: { state: "green", checked_rev: "0123456", ...(check === undefined ? {} : { check }) },
  sources: [],
  by: "Claude",
  at: "2026-11-17T10:00:00Z",
  state: "open",
  resolution: null,
  stale_why: "",
  page: "review",
  command: "",
  batch: "b2",
});

expect("the check id's label is the plain field row", fieldLabel("manual-test", "check"), "Check");
expect("the other rows keep their labels", ["state", "note", "checked_rev"].map((f) => fieldLabel("manual-test", f)), ["Result", "Note", "Checked at"]);
expect(
  "a batch's checks in their prepared order",
  batchChecks({ name: "b2", views: [view("a", "FTR-034", "MT-1"), view("b", "DEF-040", "MT-4")] }),
  ["MT-1", "MT-4"],
);
expect(
  "a check named twice is listed once; a record without one adds nothing",
  batchChecks({ name: "b2", views: [view("a", "FTR-034", "MT-1"), view("b", "DEF-040"), view("c", "DEF-040", "MT-1")] }),
  ["MT-1"],
);
expect("a batch whose records carry no check id names none", batchChecks({ name: "b2", views: [view("a", "FTR-034"), view("b", "DEF-040", "")] }), []);
const checks = batchChecks({ name: "b2", views: [view("a", "FTR-034", "MT-1"), view("b", "DEF-040", "MT-4")] });
expect("the summary text", checks.length ? ` (checks ${checks.join(", ")})` : "", " (checks MT-1, MT-4)");

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const review = read("../src/pages/ReviewPage.tsx");
const bar = (() => {
  const a = review.indexOf("function BatchBar(");
  const b = review.indexOf("\nfunction ", a + 1);
  return a < 0 ? "" : review.slice(a, b < 0 ? review.length : b);
})();
if (!bar) problems.push("ReviewPage.tsx: no BatchBar");
if (!/batchChecks\(/.test(bar)) problems.push("ReviewPage.tsx: BatchBar does not name the batch's checks (batchChecks)");
if (!/\(checks \$\{checks\.join\(", "\)\}\)/.test(bar)) problems.push("ReviewPage.tsx: BatchBar's summary does not read \"(checks MT-1, MT-4)\"");
if (/className="chip[^"]*"[^>]*>\s*\{?check/.test(bar)) problems.push("ReviewPage.tsx: the check id is a chip, not the decided plain text");

if (problems.length) {
  for (const p of problems) console.error(`✗ web::check_review_page_shows_a_results_check_id — ${p}`);
  process.exit(1);
}
console.log("✓ web::check_review_page_shows_a_results_check_id");
console.log("web::review-check-id — each prepared result shows its runbook check id, and the batch summary names them.");
