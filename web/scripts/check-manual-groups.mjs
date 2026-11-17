#!/usr/bin/env node
//
// FTR-057: the Review page groups manual checks by item and puts the release being shipped first.
// The pure grouping the page uses (manualGroups, src/manualTests.ts) is checked case by case:
//   web::check_manual_checks_group_under_their_item (R-1) — one item with three checks gives one
//     group with three rows and count 3; two items give two groups.
//   web::check_manual_checks_put_the_release_being_shipped_first (R-2) — a holding release's item
//     comes before others, under its own section, in the order the server names the releases;
//     `other` holds the rest; a check not due yet is kept apart.
//   web::check_manual_checks_mark_a_check_that_blocks_promote (R-3) — a row that holds a promotion
//     is counted on its item, and that item sorts first within its section.
// and the Manual group as drawn (ManualGroup, src/pages/review/ManualGroup.tsx, T4), rendered
// through scripts/lib/render.mjs:
//   R-1 — each item's heading is an h3 (FTR-046's AccordionRow) naming its count, its checks
//     beneath it;
//   R-2 — a holding release's section comes first; "Other items" is collapsed after it; a holding
//     release with nothing due draws no section;
//   R-3 — the check that holds Promote, and its item's heading, carry the pill naming the button;
//   web::check_manual_item_group_confirms_its_prepared_results_together (R-4) — two prepared rows
//     give the heading action "Confirm 2 prepared for <code>"; pressing it selects exactly those
//     rows (itemPrepared), so actionLabel reads "Confirm 2" and askText names both checks; one
//     prepared row offers no heading action.
//
// Usage: npm run check:manual-groups
import { itemPrepared, manualGroups } from "../src/manualTests.ts";
import { actionLabel, askText } from "../src/reviewGroups.ts";
import { closeRender, render } from "./lib/render.mjs";

const waiting = (code, requirement, test, extra = {}) => ({
  row: {
    code,
    title: `Item ${code}`,
    status: "Completed",
    requirement,
    requirement_text: "THE SYSTEM SHALL do it.",
    test,
    state: "planned",
    checked_rev: "",
    last: null,
    ...extra,
  },
});
const prepared = { id: "p", kind: "manual-test" };

const results = [];
const check = (name, problems) => results.push({ name, problems });
const expect = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

// R-1: one item with three checks is one group; two items are two groups.
{
  const problems = [];
  const one = manualGroups(
    [
      waiting("SPK-002", "R-1", "manual: one", { release: "v0.9.0" }),
      { ...waiting("SPK-002", "R-2", "manual: two", { release: "v0.9.0" }), prepared },
      waiting("SPK-002", "R-3", "manual: three", { release: "v0.9.0" }),
    ],
    [],
  );
  expect(problems, "groups for one item", one.other.length, 1);
  expect(problems, "its code", one.other[0]?.code, "SPK-002");
  expect(problems, "its rows, in the server's order", one.other[0]?.rows.map((r) => r.row.requirement), ["R-1", "R-2", "R-3"]);
  expect(problems, "its count", one.other[0]?.rows.length, 3);
  expect(problems, "its prepared count", one.other[0]?.prepared, 1);
  expect(problems, "its heading's fields", [one.other[0]?.title, one.other[0]?.status, one.other[0]?.release], ["Item SPK-002", "Completed", "v0.9.0"]);
  const two = manualGroups(
    [waiting("FEAT-150", "R-1", "manual: a"), waiting("FTR-033", "R-1", "manual: b"), waiting("FEAT-150", "R-2", "manual: c")],
    [],
  );
  expect(problems, "groups for two items", two.other.map((g) => [g.code, g.rows.length]), [["FEAT-150", 2], ["FTR-033", 1]]);
  check("web::check_manual_checks_group_under_their_item", problems);
}

// R-2: the release being shipped first.
{
  const problems = [];
  const s = manualGroups(
    [
      waiting("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }),
      waiting("FEAT-002", "R-1", "manual: none"),
      waiting("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" }),
      waiting("FEAT-004", "R-1", "manual: at main", { release: "v0.7.1" }),
      waiting("FEAT-005", "R-1", "manual: later", { release: "v0.8.0", opens_at: "Build" }),
    ],
    ["v0.8.0", "v0.7.1"],
  );
  expect(problems, "holding sections, in the server's order", s.holding.map((h) => h.version), ["v0.8.0", "v0.7.1"]);
  expect(problems, "the candidate's items", s.holding[0]?.items.map((g) => g.code), ["FEAT-003"]);
  expect(problems, "the other release's items", s.holding[1]?.items.map((g) => g.code), ["FEAT-004"]);
  expect(problems, "other items", s.other.map((g) => g.code), ["FEAT-001", "FEAT-002"]);
  expect(problems, "not due yet", s.notDue.map((r) => r.row.code), ["FEAT-005"]);
  const none = manualGroups([waiting("FEAT-001", "R-1", "manual: x", { release: "v0.9.0" })], []);
  expect(problems, "with nothing holding", [none.holding.length, none.other.length], [0, 1]);
  check("web::check_manual_checks_put_the_release_being_shipped_first", problems);
}

// R-3: a check that holds Promote is counted and its item sorts first within its section.
{
  const problems = [];
  const holds = "Promote v0.8.0 to main";
  const s = manualGroups(
    [
      waiting("FEAT-010", "R-1", "manual: free", { release: "v0.8.0" }),
      waiting("FEAT-011", "R-1", "manual: free too", { release: "v0.8.0" }),
      waiting("FEAT-011", "R-2", "manual: blocks", { release: "v0.8.0", holds }),
      waiting("FEAT-012", "R-1", "manual: elsewhere", { release: "v0.9.0" }),
    ],
    ["v0.8.0"],
  );
  expect(problems, "the holding item sorts first", s.holding[0]?.items.map((g) => g.code), ["FEAT-011", "FEAT-010"]);
  expect(problems, "its holds count", s.holding[0]?.items.map((g) => g.holds), [1, 0]);
  expect(problems, "its rows keep their order", s.holding[0]?.items[0]?.rows.map((r) => r.row.holds ?? ""), ["", holds]);
  expect(problems, "an item of another release holds nothing", s.other.map((g) => [g.code, g.holds]), [["FEAT-012", 0]]);
  check("web::check_manual_checks_mark_a_check_that_blocks_promote", problems);
}


// The Manual group as drawn (T4). A slice row as core builds it (reviewgroups.rs, manual_row), and
// the waiting check it stands for, paired by its key <CODE>/<R-n>/<test>.
const sliceRow = (code, requirement, test, extra = {}) => ({
  key: `${code}/${requirement}/${test}`,
  code,
  title: `Item ${code}`,
  what: `${requirement}: ${test}`,
  records: "Records the prepared result as you",
  release: "",
  sprint: "",
  changed_at: "",
  blocked: "",
  pills: [],
  prepared: "",
  batch: "",
  rev: "",
  spec_rev: "",
  signoff: "",
  requirement,
  test,
  ...extra,
});
const manualSlice = (rows, holding = []) => ({ key: "manual", label: "Manual checks", action: "Confirm", rows, choices: [], holding });
const drawn = (slice, manual) =>
  render("src/pages/review/ManualGroup.tsx", "ManualGroup", {
    project: "x",
    slice,
    rows: slice.rows,
    manual,
    writable: true,
    sel: { group: "manual", keys: new Set() },
    onSelect: () => {},
    onSelectItem: () => {},
    onConfirmItem: () => {},
    reasonFor: (r) => r.blocked,
    openRows: new Set(),
    onOpen: () => {},
    inFlight: false,
    body: (r) => `evidence of ${r.key}`,
  });
const heads = (document) => [...document.querySelectorAll('section[id^="manual-item-"] > .ui-row-head h3')].map((h) => h.textContent.trim());
const renderChecks = [];
try {
  // R-1, rendered: one h3 heading per item, its three checks beneath it.
  {
    const problems = [];
    const rows = [sliceRow("SPK-002", "R-1", "manual: one"), sliceRow("SPK-002", "R-2", "manual: two"), sliceRow("SPK-002", "R-3", "manual: three")];
    const manual = [
      waiting("SPK-002", "R-1", "manual: one", { status: "Build", release: "v0.9.0" }),
      waiting("SPK-002", "R-2", "manual: two", { status: "Build", release: "v0.9.0" }),
      waiting("SPK-002", "R-3", "manual: three", { status: "Build", release: "v0.9.0" }),
    ];
    const { document, html } = await drawn(manualSlice(rows), manual);
    const items = [...document.querySelectorAll('section[id^="manual-item-"]')];
    expect(problems, "one item group", items.length, 1);
    expect(problems, "its heading is an h3 naming the item and its count", heads(document), ["SPK-002 Item SPK-002 · Build · v0.9.0 · 3 checks"]);
    const inside = [...(items[0]?.querySelectorAll('section.ui-row:not([id^="manual-item-"])') ?? [])].map((s) => s.id);
    expect(problems, "its three checks beneath it", inside, rows.map((r) => r.key));
    if (problems.length && !items.length) problems.push(html.slice(0, 400));
    renderChecks.push(["web::check_manual_checks_group_under_their_item", problems]);
  }
  // R-2, rendered: the holding release's section first, "Other items" collapsed after it, and a
  // holding release with nothing due draws no section.
  {
    const problems = [];
    const rows = [sliceRow("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }), sliceRow("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" })];
    const manual = [
      waiting("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }),
      waiting("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" }),
    ];
    const { document } = await drawn(manualSlice(rows, ["v0.8.0", "v0.7.1"]), manual);
    const titles = [...document.querySelectorAll(".manual-groups h2")].map((h) => h.textContent.trim());
    expect(problems, "the sections, holding first", titles, ["Holding v0.8.0 · 1 item, 1 check", "Other items (1)"]);
    expect(problems, "the holding release's item is drawn first", heads(document), ["FEAT-003 Item FEAT-003 · Completed · v0.8.0 · 1 check"]);
    const other = document.getElementById("manual-other-head");
    expect(problems, "Other items is collapsed", other?.getAttribute("aria-expanded"), "false");
    if (document.getElementById("FEAT-001/R-1/manual: other")) problems.push("a check under the collapsed Other items is drawn");
    if (/Holding v0\.7\.1/.test(document.body.textContent)) problems.push("a holding release with nothing due draws a section");
    renderChecks.push(["web::check_manual_checks_put_the_release_being_shipped_first", problems]);
  }
  // R-3, rendered: the check that holds Promote and its item's heading carry the pill.
  {
    const problems = [];
    const holds = "Promote v0.8.0 to main";
    const rows = [sliceRow("FEAT-010", "R-1", "manual: free", { release: "v0.8.0" }), sliceRow("FEAT-011", "R-2", "manual: blocks", { release: "v0.8.0" })];
    const manual = [
      waiting("FEAT-010", "R-1", "manual: free", { release: "v0.8.0" }),
      waiting("FEAT-011", "R-2", "manual: blocks", { release: "v0.8.0", holds }),
    ];
    const { document } = await drawn(manualSlice(rows, ["v0.8.0"]), manual);
    expect(problems, "the holding item sorts first", heads(document).map((h) => h.split(" ")[0]), ["FEAT-011", "FEAT-010"]);
    const pills = (el) => [...(el?.querySelectorAll(":scope > .ui-row-head .ui-pill-warn") ?? [])].map((x) => x.textContent.trim());
    expect(problems, "the item's heading names the button it holds", pills(document.getElementById("manual-item-FEAT-011")), [`warning holds ${holds}`]);
    expect(problems, "so does the check", pills(document.getElementById("FEAT-011/R-2/manual: blocks")), [`warning holds ${holds}`]);
    expect(problems, "an item that holds nothing has no pill", pills(document.getElementById("manual-item-FEAT-010")), []);
    renderChecks.push(["web::check_manual_checks_mark_a_check_that_blocks_promote", problems]);
  }
  // R-4: one item's prepared results confirmed together.
  {
    const problems = [];
    const rows = [
      sliceRow("SPK-002", "R-1", "manual: one", { prepared: "P-1" }),
      sliceRow("SPK-002", "R-2", "manual: two", { prepared: "P-2" }),
      sliceRow("SPK-002", "R-3", "manual: three", { blocked: "Do the check, then press Passed or Failed in the row." }),
      sliceRow("FEAT-009", "R-1", "manual: alone", { prepared: "P-3" }),
    ];
    const slice = manualSlice(rows);
    const sel = itemPrepared(slice, "SPK-002");
    expect(problems, "pressing it selects exactly that item's prepared rows", [sel.group, [...sel.keys]], ["manual", [rows[0].key, rows[1].key]]);
    expect(problems, "the group's action reads", actionLabel(slice, sel), "Confirm 2");
    expect(problems, "the question names both checks", askText(slice, sel, rows), "Confirm SPK-002 R-1 and SPK-002 R-2?");
    expect(problems, "one prepared row is one row", [...itemPrepared(slice, "FEAT-009").keys], [rows[3].key]);
    const manual = rows.map((r) => ({ ...waiting(r.code, r.requirement, r.test), prepared: r.prepared ? { id: r.prepared, kind: "manual-test" } : undefined }));
    const { document } = await drawn(slice, manual);
    const actions = (id) => [...(document.getElementById(id)?.querySelectorAll(":scope > .ui-row-head .ui-row-actions button") ?? [])].map((b) => b.textContent.trim());
    expect(problems, "two prepared rows give the heading action", actions("manual-item-SPK-002"), ["Confirm 2 prepared for SPK-002"]);
    expect(problems, "one prepared row offers none", actions("manual-item-FEAT-009"), []);
    if (document.querySelector(".ask-first")) problems.push("the question shows before the first press");
    renderChecks.push(["web::check_manual_item_group_confirms_its_prepared_results_together", problems]);
  }
} catch (e) {
  renderChecks.push(["web::check_manual_groups_render", [`rendering the Manual group failed: ${e?.stack ?? e}`]]);
} finally {
  await closeRender();
}
// A check's pure and rendered parts report as one line.
for (const [name, problems] of renderChecks) {
  const known = results.find((r) => r.name === name);
  if (known) known.problems.push(...problems.map((p) => `rendered: ${p}`));
  else check(name, problems);
}

let failed = false;
for (const { name, problems } of results) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}
if (failed) process.exit(1);
