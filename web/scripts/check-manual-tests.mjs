#!/usr/bin/env node
//
// web::manual_tests (DEF-040 R-1): WHILE an item has a manual test that is not green, the Review
// page lists it with its requirement and the check to make, and records Passed or Failed for the
// person. The pure functions the page uses are checked case by case; the page is read for where it
// renders the section, what it shows, and that nothing has to be typed.
//
// web::check_manual_check_offers_no_result_before_its_item_is_built (DEF-076 R-3): a check whose
// item has not reached its build stage says why, offers no Passed, Failed or prepared record, and
// is listed apart: since FTR-038 its row in the Review page's manual group carries DEF-076's
// sentence beside a disabled checkbox, so it cannot be selected for a group decision either.
//
// Usage: npm run check:manual-tests
import { readFileSync } from "node:fs";
import { closeRender, render } from "./lib/render.mjs";
import { lastResultLine, manualCommand, manualRows, notDueLine } from "../src/manualTests.ts";

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
// Since FTR-038 the checks are the Review page's manual group: each row's body is ManualCheck,
// fed by the row's own evidence (GET /review/groups/manual/rows/{row}).
has("does not render a manual row's evidence through ManualCheck", /if \(e\.manual\) \{[\s\S]*?<ManualCheck\b/);
has("does not show the requirement's text", /row\.requirement_text/);
has("does not show the check (the test's name)", /\{row\.test\}/);
has("has no Passed button", />\s*Passed\s*</);
has("has no Failed button", />\s*Failed\s*</);
has("has no Add a note link", />\s*Add a note\s*</);
has("does not record through recordManual", /api\.recordManual\(/);
// The page is empty only when no group, the manual group included, holds anything.
has("does not call the empty page empty while checks wait", /groups\.every\(\(g\) => g\.count === 0\)/);
// The note's input is only rendered once the person asks for it.
const manualPart = page.slice(page.indexOf("function ManualCheck("), page.indexOf("/** One proposed decision"));
const inputs = manualPart.split("<input").length - 1;
const gated = /\{noting \? \(\s*<p className="small">\s*<label>[\s\S]*?<input/.test(manualPart);
if (inputs !== 1 || !gated) problems.push("ReviewPage.tsx: the manual check renders an input before Add a note is chosen");

// DEF-076 R-3: no result is offered before the item's build stage.
const notDue = [];
const expectNotDue = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    notDue.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};
const early = waiting("FEAT-003", "R-1", "manual: it prints", { status: "System Design", opens_at: "Build" });
expectNotDue(
  "a check before its build stage says why",
  notDueLine(early),
  "Not yet: FEAT-003 is at System Design. Its check opens at Build, once the code it checks exists.",
);
expectNotDue("a due check has no line", notDueLine(a), "");
expectNotDue("an empty opens_at is due", notDueLine({ ...a, opens_at: "" }), "");
// ManualCheck: the not-due branch comes before every way to record a result, so none renders.
const branch = manualPart.indexOf(") : notDue ? (");
const firstAfter = (re) => {
  const m = manualPart.slice(Math.max(branch, 0)).search(re);
  return m < 0 ? -1 : m + branch;
};
if (!/const notDue = notDueLine\(row\);/.test(manualPart)) notDue.push("ManualCheck does not read notDueLine(row)");
if (branch < 0) notDue.push("ManualCheck has no branch for a check that is not due");
else {
  const before = manualPart.slice(0, branch);
  for (const [name, re] of [
    ["PreparedPanel", /<PreparedPanel/],
    ["Passed", />\s*Passed\s*</],
    ["Failed", />\s*Failed\s*</],
    ["the terminal command", /manualCommand\(/],
  ]) {
    if (re.test(before)) notDue.push(`ManualCheck renders ${name} before the not-due branch`);
    if (firstAfter(re) < 0) notDue.push(`ManualCheck: ${name} is not behind the not-due branch`);
  }
  if (!/^\) : notDue \? \(\s*<p className="small muted">\{notDue\}<\/p>\s*\) :/.test(manualPart.slice(branch))) {
    notDue.push("ManualCheck's not-due branch renders more than its sentence");
  }
}
// In the manual group (FTR-038) a check that is not due is a row core blocks with DEF-076's
// sentence: shown beside its checkbox, which is disabled, so no group decision takes it.
try {
  const sentence = "FEAT-003 is at System Design: its manual check opens at Build, once the code it checks exists. Record it then.";
  const blank = { title: "Cart", what: "R-1: manual: it prints", records: "Records the prepared result as you", release: "", sprint: "", changed_at: "", pills: [], batch: "", rev: "", spec_rev: "", signoff: "" };
  const rows = [
    { ...blank, key: "FEAT-003/R-1/manual: it prints", code: "FEAT-003", blocked: sentence, prepared: "P-1", requirement: "R-1", test: "manual: it prints" },
    { ...blank, key: "FEAT-004/R-1/manual: it saves", code: "FEAT-004", blocked: "", prepared: "P-2", requirement: "R-1", test: "manual: it saves" },
  ];
  const slice = { key: "manual", label: "Manual checks", action: "Confirm", rows, choices: [] };
  const { document } = await render("src/pages/review/GroupView.tsx", "GroupPanel", {
    project: "x",
    slice,
    writable: true,
    who: "Alice <a@example.invalid>",
    tick: 0,
    body: () => "",
  });
  const early = document.getElementById(rows[0].key);
  if (!early?.querySelector('input[type="checkbox"]')?.hasAttribute("disabled")) notDue.push("a check that is not due can be selected in the manual group");
  if (early?.querySelector(".ui-row-reason")?.textContent !== sentence) notDue.push("a check that is not due does not say why beside its row");
  if (early?.querySelector('input[type="checkbox"]')?.hasAttribute("checked")) notDue.push("a check that is not due arrives selected");
  if (!document.getElementById(rows[1].key)?.querySelector('input[type="checkbox"]')?.hasAttribute("checked")) {
    notDue.push("a due check with a prepared result does not arrive selected");
  }
} catch (e) {
  notDue.push(`rendering the manual group failed: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

for (const p of problems) console.error(`web::manual_tests — ${p}`);
for (const p of notDue) console.error(`web::check_manual_check_offers_no_result_before_its_item_is_built — ${p}`);
if (problems.length === 0) {
  console.log("web::manual_tests — manual checks are paired with open records, listed with their requirement and check, and need nothing typed.");
}
if (notDue.length === 0) {
  console.log(
    "web::check_manual_check_offers_no_result_before_its_item_is_built — a check before its item's build stage says why, offers no result, and cannot be selected in its group.",
  );
}
if (problems.length || notDue.length) process.exit(1);
