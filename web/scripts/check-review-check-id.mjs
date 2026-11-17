#!/usr/bin/env node
//
// web::check_review_page_shows_a_results_check_id (DEF-089 R-3, R-5): each result a walkthrough
// prepares carries the runbook's check id (MT-n), and the Review page shows it as a plain field row
// "Check" beside Result, Note and Checked at; the batch's line in its Review group (FTR-038) names its checks, as
// "(checks MT-1, MT-4)". The pure helpers are checked case by case; the page is read for where it
// renders them.
//
// Usage: npm run check:review-check-id
import { readFileSync } from "node:fs";
import { batchChecks, fieldLabel } from "../src/prepared.ts";
import { JSDOM } from "jsdom";
import { closeRender, load, render } from "./lib/render.mjs";

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

// Since FTR-038 a batch is decided in its group: GroupPanel's sticky head shows one line per batch
// in the slice, "<N> results from <batch> (checks MT-1, MT-4)", and the action for exactly that
// batch reads "Confirm all N (checks MT-1, MT-4)". BatchBar, the old home, is gone.
const review = readFileSync(new URL("../src/pages/ReviewPage.tsx", import.meta.url), "utf8");
if (/function BatchBar\(/.test(review)) problems.push("ReviewPage.tsx: BatchBar is still drawn beside the groups");
const row = (key, item, batch) => ({
  key, code: item, title: item, what: "R-1: manual: x", records: "", release: "", sprint: "", changed_at: "",
  blocked: "", pills: [], prepared: `P-${key}`, batch, rev: "", spec_rev: "", signoff: "", requirement: "R-1", test: "manual: x",
});
try {
  // Nothing arrives selected (FTR-038 R-5): the batch line offers its own "Select these 2" button,
  // and the action reads "Confirm all 2 (checks ...)" once it is pressed. A group of another key than
  // Manual is used: the Manual group draws its rows through a live read (EventSource).
  const props = {
    project: "x",
    slice: { key: "manual-rows", label: "Manual checks", action: "Confirm", choices: [], rows: [row("a", "FTR-034", "b2"), row("b", "DEF-040", "b2")] },
    batches: [{ name: "b2", views: [view("a", "FTR-034", "MT-1"), view("b", "DEF-040", "MT-4")] }],
    writable: true,
    who: "Alice <a@example.invalid>",
    tick: 0,
    body: () => "",
  };
  const { document } = await render("src/pages/review/GroupView.tsx", "GroupPanel", props);
  const head = document.querySelector(".review-group-head");
  const lines = [...(head?.querySelectorAll(".review-group-batch") ?? [])];
  const plain = (l) => {
    const c = l.cloneNode(true);
    c.querySelectorAll("button").forEach((b) => b.remove());
    return c.textContent.trim();
  };
  expect("one line per batch in the group", lines.map(plain), ["2 results from b2 (checks MT-1, MT-4)"]);
  if (lines.some((l) => l.querySelector(".chip"))) problems.push("GroupPanel: the check id is a chip, not the decided plain text");
  // Pressed in a live DOM, as check-review-batch.mjs does.
  const dom = new JSDOM("<!DOCTYPE html><body><div id=root></div></body>", { url: "http://localhost/p/x/review" });
  const saved = {};
  for (const k of ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"]) saved[k] = Object.getOwnPropertyDescriptor(globalThis, k);
  Object.defineProperty(globalThis, "window", { value: dom.window, configurable: true });
  Object.defineProperty(globalThis, "document", { value: dom.window.document, configurable: true });
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  try {
    const mod = await load("src/pages/review/GroupView.tsx");
    const React = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { MemoryRouter } = await import("react-router-dom");
    const container = dom.window.document.getElementById("root");
    const root = createRoot(container);
    await React.act(async () => root.render(React.createElement(MemoryRouter, { initialEntries: ["/p/x/review"] }, React.createElement(mod.GroupPanel, props))));
    const button = [...container.querySelectorAll(".review-group-batch button")].find((b) => /Select these/.test(b.textContent));
    if (!button) problems.push("GroupPanel: the batch line has no Select these button");
    else await React.act(async () => button.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));
    const buttons = [...container.querySelectorAll(".review-group-head button")].map((x) => x.textContent.trim());
    if (!buttons.includes("Confirm all 2 (checks MT-1, MT-4)")) problems.push(`GroupPanel: the batch's action does not read "Confirm all 2 (checks MT-1, MT-4)": ${JSON.stringify(buttons)}`);
    await React.act(async () => root.unmount());
  } finally {
    for (const [k, d] of Object.entries(saved)) d ? Object.defineProperty(globalThis, k, d) : delete globalThis[k];
  }
} catch (e) {
  problems.push(`rendering the group failed: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

if (problems.length) {
  for (const p of problems) console.error(`✗ web::check_review_page_shows_a_results_check_id — ${p}`);
  process.exit(1);
}
console.log("✓ web::check_review_page_shows_a_results_check_id");
console.log("web::review-check-id — each prepared result shows its runbook check id, and the batch summary names them.");
