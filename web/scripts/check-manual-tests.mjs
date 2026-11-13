#!/usr/bin/env node
//
// web::manual_tests (DEF-040 R-1): WHILE an item has a manual test that is not green, the Review
// page lists it with its requirement and the check to make, and records Passed or Failed for the
// person. The pure functions the page uses are checked case by case; the page is read for where it
// renders the section, what it shows, and that nothing has to be typed.
//
// Usage: npm run check:manual-tests
import { readFileSync } from "node:fs";
import { lastResultLine, manualCommand, manualRows } from "../src/manualTests.ts";

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const waiting = (code, requirement, test, extra = {}) => ({
  code,
  title: "Cart",
  status: "In Progress",
  requirement,
  requirement_text: "THE SYSTEM SHALL keep the cart.",
  test,
  state: "planned",
  checked_rev: "",
  last: null,
  ...extra,
});
const view = (id, at, state, subject) => ({
  id,
  kind: "manual-test",
  subject,
  fields: { state: "green" },
  sources: [],
  by: "Claude",
  at,
  state,
  resolution: null,
  stale_why: "",
  page: "review",
  command: "",
});
const a = waiting("FEAT-002", "R-17", "manual: the cart keeps its items");
const b = waiting("FEAT-003", "R-3", "manual: it prints");
const subject = (w) => ({ item: w.code, requirement: w.requirement, test: w.test });

// manualRows: every waiting test is a row, in order, paired with its newest open record only.
const rows = manualRows(
  [a, b],
  [
    view("old", "2026-11-12T09:00:00Z", "open", subject(a)),
    view("new", "2026-11-12T10:00:00Z", "open", subject(a)),
    view("stale", "2026-11-12T11:00:00Z", "stale", subject(b)),
    view("done", "2026-11-12T12:00:00Z", "confirmed", subject(b)),
  ],
);
expect("a row per waiting test, in order", rows.map((r) => r.row.code), ["FEAT-002", "FEAT-003"]);
expect("the newest open record is paired", rows[0].prepared?.id, "new");
expect("a stale or answered record is ignored", rows[1].prepared, undefined);
expect("no waiting tests, no rows", manualRows(undefined, undefined), []);
expect("a record for another test is not paired", manualRows([b], [view("x", "2026-11-12T10:00:00Z", "open", subject(a))])[0].prepared, undefined);

expect("the terminal command for a read-only monitor", manualCommand(a, true), "kanbanr test FEAT-002 R-17 'manual: the cart keeps its items' green");
expect("a failed result is red", manualCommand(a, false).endsWith(" red"), true);
expect(
  "the last result says who and when",
  lastResultLine({ code: "FEAT-002", requirement: "R-17", test: "t", state: "red", by: "Ada L", at: "2026-11-12T11:02:00Z" }),
  "failed by Ada L on 12 Nov 2026",
);
expect("no last result, no line", lastResultLine(null), "");

// The page: the section, the requirement's text, the check, and no input until "Add a note".
const page = readFileSync(new URL("../src/pages/ReviewPage.tsx", import.meta.url), "utf8");
const has = (name, re) => {
  if (!re.test(page)) problems.push(`ReviewPage.tsx: ${name}`);
};
has("does not read getManualTests", /api\.getManualTests\(project\)/);
has("has no Manual checks section", /<h2>Manual checks<\/h2>/);
has("does not show the requirement's text", /row\.requirement_text/);
has("does not show the check (the test's name)", /\{row\.test\}/);
has("has no Passed button", />\s*Passed\s*</);
has("has no Failed button", />\s*Failed\s*</);
has("has no Add a note link", />\s*Add a note\s*</);
has("does not record through recordManual", /api\.recordManual\(/);
has("does not call the empty page empty while checks wait", /proposed\.length > 0 \|\| manualWaiting\.length > 0/);
// The note's input is only rendered once the person asks for it.
const manualPart = page.slice(page.indexOf("function ManualCheck("), page.indexOf("/** One proposed decision"));
const inputs = manualPart.split("<input").length - 1;
const gated = /\{noting \? \(\s*<p className="small">\s*<label>[\s\S]*?<input/.test(manualPart);
if (inputs !== 1 || !gated) problems.push("ReviewPage.tsx: the manual check renders an input before Add a note is chosen");

if (problems.length) {
  for (const p of problems) console.error(`web::manual_tests — ${p}`);
  process.exit(1);
}
console.log("web::manual_tests — manual checks are paired with open records, listed with their requirement and check, and need nothing typed.");
