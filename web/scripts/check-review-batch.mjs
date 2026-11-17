#!/usr/bin/env node
//
// web::check_review_page_offers_one_click_for_a_batch (FTR-037 R-5): WHEN a run's checks are
// prepared as one batch, the Review page offers "Confirm all N" and "Reject all N" for the batch,
// beside the single buttons each record keeps, and posts the batch routes with nothing typed. The
// pure grouping the page uses is checked case by case; the page and the API client are read for
// where they render and post it.
//
// Usage: npm run check:review-batch
import { readFileSync } from "node:fs";
import { batches } from "../src/prepared.ts";

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const view = (id, at, state, batch, item) => ({
  id,
  kind: "manual-test",
  subject: { item, requirement: "R-1", test: "manual: x" },
  fields: { state: "green" },
  sources: [],
  by: "Claude",
  at,
  state,
  resolution: null,
  stale_why: "",
  page: "review",
  command: "",
  ...(batch ? { batch } : {}),
});
const views = [
  view("c", "2026-11-15T10:02:00Z", "open", "togaf-2026-11-15", "DEF-040"),
  view("a", "2026-11-15T10:00:00Z", "open", "togaf-2026-11-15", "FTR-034"),
  view("b", "2026-11-15T10:01:00Z", "open", undefined, "FTR-028"),
  view("d", "2026-11-15T09:00:00Z", "confirmed", "togaf-2026-11-15", "DEF-039"),
  view("e", "2026-11-15T11:00:00Z", "open", "agile-2026-11-15", "DOC-008"),
  view("f", "2026-11-15T11:30:00Z", "open", "  ", "DOC-008"),
];
const grouped = batches(views);
expect("batches come in the order their first record was prepared", grouped.map((b) => b.name), ["togaf-2026-11-15", "agile-2026-11-15"]);
expect("a batch holds its open records, oldest first", grouped[0].views.map((v) => v.id), ["a", "c"]);
expect("an answered record is not offered again", grouped[0].views.some((v) => v.id === "d"), false);
expect("a record prepared alone, or with a blank batch, is in no batch", grouped.flatMap((b) => b.views.map((v) => v.id)).includes("b") || grouped.flatMap((b) => b.views.map((v) => v.id)).includes("f"), false);
expect("nothing makes no batch", batches(undefined), []);

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const review = read("../src/pages/ReviewPage.tsx");
const bar = (() => {
  const a = review.indexOf("function BatchBar(");
  const b = review.indexOf("\nfunction ", a + 1);
  return a < 0 ? "" : review.slice(a, b < 0 ? review.length : b);
})();
if (!bar) problems.push("ReviewPage.tsx: no BatchBar");
if (!/batches\(prepared\.data\)\.map/.test(review)) problems.push("ReviewPage.tsx: the batches are not rendered from the prepared listing");
if (!/Confirm all \$\{n\}/.test(bar)) problems.push("ReviewPage.tsx: BatchBar offers no \"Confirm all N\"");
if (!/Reject all \$\{n\}/.test(bar)) problems.push("ReviewPage.tsx: BatchBar offers no \"Reject all N\"");
if (/<(input|textarea|select)\b/.test(bar)) problems.push("ReviewPage.tsx: BatchBar asks the person to type");
if (!/api\.confirmPreparedBatch\(project, batch\.name, who\)/.test(review)) problems.push("ReviewPage.tsx: Confirm all does not post the batch confirm route as the monitor's identity");
if (!/api\.rejectPreparedBatch\(project, batch\.name, who\)/.test(review)) problems.push("ReviewPage.tsx: Reject all does not post the batch reject route");
if (!/<ManualCheck\b/.test(review)) problems.push("ReviewPage.tsx: the single buttons (a manual check's own row) are gone");

const apiTs = read("../src/api.ts");
if (!/\/prepared\/batch\/\$\{encodeURIComponent\(batch\)\}\/confirm`/.test(apiTs)) problems.push("api.ts: no batch confirm route");
if (!/\/prepared\/batch\/\$\{encodeURIComponent\(batch\)\}\/reject`/.test(apiTs)) problems.push("api.ts: no batch reject route");
const types = read("../src/types.ts");
if (!/batch\?: string;/.test(types)) problems.push("types.ts: Prepared has no batch");

if (problems.length) {
  for (const p of problems) console.error(`✗ web::check_review_page_offers_one_click_for_a_batch — ${p}`);
  process.exit(1);
}
console.log("✓ web::check_review_page_offers_one_click_for_a_batch");
console.log("web::review-batch — a batch of prepared records is confirmed or rejected with one click, beside the single buttons.");
