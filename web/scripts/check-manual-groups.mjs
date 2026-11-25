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
//   R-2 — a holding release's section comes first; the sections after it (DEF-113) are collapsed;
//     a holding release with nothing due draws no section;
//   R-3 — the check that holds Promote, and its item's heading, carry the pill naming the button;
//   web::check_manual_item_group_confirms_its_prepared_results_together (R-4) — two prepared rows
//     give the heading action "Confirm 2 prepared for <code>"; pressing it selects exactly those
//     rows (itemPrepared), so actionLabel reads "Confirm 2" and askText names both checks; one
//     prepared row offers no heading action.
// and DEF-113, where the group says what it is for and lets the person retire what shipped unjudged:
//   web::check_manual_checks_say_what_they_are_for (R-1) — the group starts with MANUAL_PURPOSE,
//     with or without holding sections, read-only too;
//   web::check_manual_checks_split_the_rest_by_release (R-2) — manualGroups splits `other` into
//     shipped, later and unplanned; the page draws them as collapsed sections, no "Other items";
//   web::check_manual_shipped_checks_retire_together (R-7) — Retire N asks, wants a reason, and
//     calls onRetire with the section's selection and the trimmed reason.
//   web::check_manual_a_shipped_row_selects_for_retire (R-7) — a not-prepared row the group refuses
//     (blocked) still ticks into Retire; Passed/Failed stay in the row; the prepared row keeps Confirm.
//
// Usage: npm run check:manual-groups
import { MANUAL_PURPOSE, itemPrepared, lastResultLine, manualGroups, retirable } from "../src/manualTests.ts";
import { actionLabel, askText, decisionBody } from "../src/reviewGroups.ts";
import { readFileSync } from "node:fs";
import { closeRender, render } from "./lib/render.mjs";
import { JSDOM } from "jsdom";

// The pressed checks need a DOM before React loads: react-dom decides at load whether the browser
// has an `input` event, and onChange on a text box is silent if it loaded without one.
const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

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
const ok = (problems, what, cond, detail = "") => {
  if (!cond) problems.push(`${what}${detail ? `: ${detail}` : ""}`);
};
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
const drawn = (slice, manual, extra = {}) =>
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
    onRetire: () => {},
    reasonFor: (r) => r.blocked,
    openRows: new Set(),
    onOpen: () => {},
    inFlight: false,
    body: (r) => `evidence of ${r.key}`,
    ...extra,
  });
// DEF-113 R-2: `other` is split by where each item's release stands, and `other` stays as it was.
{
  const problems = [];
  const s = manualGroups(
    [
      waiting("FEAT-001", "R-1", "manual: shipped", { release: "v0.6.0" }),
      waiting("FEAT-002", "R-1", "manual: later", { release: "v1.0.0" }),
      waiting("FEAT-003", "R-1", "manual: none"),
      waiting("FEAT-004", "R-1", "manual: candidate", { release: "v0.8.0" }),
      waiting("FEAT-005", "R-1", "manual: also shipped", { release: "v0.5.0" }),
    ],
    ["v0.8.0"],
    ["v0.6.0", "v0.5.0"],
  );
  expect(problems, "shipped", s.shipped.map((g) => g.code), ["FEAT-001", "FEAT-005"]);
  expect(problems, "later", s.later.map((g) => g.code), ["FEAT-002"]);
  expect(problems, "unplanned", s.unplanned.map((g) => g.code), ["FEAT-003"]);
  expect(problems, "other is their union, as before", s.other.map((g) => g.code), ["FEAT-001", "FEAT-002", "FEAT-003", "FEAT-005"]);
  expect(problems, "holding is untouched", s.holding[0]?.items.map((g) => g.code), ["FEAT-004"]);
  const old = manualGroups([waiting("FEAT-001", "R-1", "manual: x", { release: "v0.9.0" })], []);
  expect(problems, "without a shipped list the release is later", [old.shipped.length, old.later.length], [0, 1]);
  expect(problems, "a retired result reads as one", lastResultLine({ code: "A", requirement: "R-1", test: "t", state: "retired", by: "Ada", at: "2026-11-12T10:00:00Z", note: "shipped, not judged" }), "retired by Ada on 12 Nov 2026: shipped, not judged");
  expect(problems, "an unprepared due row is retirable", retirable(waiting("A", "R-1", "t")), true);
  expect(problems, "a prepared row is not retirable", retirable({ ...waiting("A", "R-1", "t"), prepared }), false);
  expect(problems, "a row not yet due is not retirable", retirable(waiting("A", "R-1", "t", { opens_at: "Build" })), false);
  const slice = manualSlice([sliceRow("A", "R-1", "t")]);
  const body = decisionBody(slice, { group: "manual", keys: new Set(["A/R-1/t"]) }, slice.rows, "Ada", "", "retire", "why");
  expect(problems, "the decision carries the action and reason", [body.action, body.reason], ["retire", "why"]);
  check("web::check_manual_checks_split_the_rest_by_release", problems);
}

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
  // R-2, rendered: the holding release's section first, the sections after it collapsed and named
  // by where their items stand, and a holding release with nothing due draws no section. FTR-057's
  // "Other items" is replaced by DEF-113's three sections, as the person decided (17 Nov 2026).
  {
    const problems = [];
    const rows = [sliceRow("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }), sliceRow("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" })];
    const manual = [
      waiting("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }),
      waiting("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" }),
    ];
    const { document } = await drawn(manualSlice(rows, ["v0.8.0", "v0.7.1"]), manual);
    const titles = [...document.querySelectorAll(".manual-groups h2")].map((h) => h.textContent.trim());
    expect(problems, "the sections, holding first", titles, ["Holding v0.8.0 · 1 item, 1 check", "Later releases · 1 item, 1 check"]);
    expect(problems, "the holding release's item is drawn first", heads(document), ["FEAT-003 Item FEAT-003 · Completed · v0.8.0 · 1 check"]);
    const later = document.getElementById("manual-later-head");
    expect(problems, "Later releases is collapsed", later?.getAttribute("aria-expanded"), "false");
    if (document.getElementById("FEAT-001/R-1/manual: other")) problems.push("a check under the collapsed Later releases is drawn");
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
  // DEF-113 R-1, rendered: the line on top, in every state of the group.
  {
    const problems = [];
    const rows = [sliceRow("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" })];
    const manual = [waiting("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" })];
    const first = (document) => document.querySelector(".manual-groups")?.firstElementChild;
    for (const [what, slice, props] of [
      ["with a holding release", manualSlice(rows, ["v0.8.0"]), {}],
      ["with none", manualSlice(rows, []), {}],
      ["read-only", manualSlice(rows, ["v0.8.0"]), { writable: false }],
    ]) {
      const { document } = await drawn(slice, manual, props);
      const el = first(document);
      expect(problems, `the first thing ${what}`, [el?.tagName, el?.className.includes("manual-purpose"), el?.textContent], ["P", true, MANUAL_PURPOSE]);
    }
    expect(problems, "its words", MANUAL_PURPOSE, "A manual check is one only you can judge. An item ships when its check is recorded passed.");
    renderChecks.push(["web::check_manual_checks_say_what_they_are_for", problems]);
  }
  // DEF-113 R-2, rendered: shipped, later and not planned, each collapsed and counted.
  {
    const problems = [];
    const rows = [
      sliceRow("FEAT-001", "R-1", "manual: a", { release: "v0.6.0" }),
      sliceRow("FEAT-001", "R-2", "manual: b", { release: "v0.6.0" }),
      sliceRow("FEAT-002", "R-1", "manual: c", { release: "v1.0.0" }),
      sliceRow("FEAT-004", "R-1", "manual: d"),
      sliceRow("FEAT-003", "R-1", "manual: e", { release: "v0.8.0" }),
      sliceRow("FEAT-005", "R-1", "manual: f", { release: "v0.8.0", blocked: "Not yet" }),
    ];
    const manual = rows.map((r) => waiting(r.code, r.requirement, r.test, { release: r.release || undefined, ...(r.code === "FEAT-005" ? { opens_at: "Build" } : {}) }));
    const slice = { ...manualSlice(rows, ["v0.8.0"]), shipped: ["v0.6.0", "v0.5.0"] };
    const { document } = await drawn(slice, manual);
    const titles = [...document.querySelectorAll(".manual-groups h2")].map((h) => h.textContent.trim());
    expect(problems, "the sections, in order", titles, [
      "Holding v0.8.0 · 1 item, 1 check",
      "Shipped without a judgement · 1 item, 2 checks",
      "Later releases · 1 item, 1 check",
      "Not planned · 1 item, 1 check",
      "Not due yet (1)",
    ]);
    expect(problems, "the holding section is open", document.getElementById("manual-holding-v0.8.0") != null && document.getElementById("manual-item-FEAT-003-head")?.getAttribute("aria-expanded"), "true");
    for (const id of ["manual-shipped", "manual-later", "manual-unplanned", "manual-not-due"]) {
      expect(problems, `${id} is collapsed`, document.getElementById(`${id}-head`)?.getAttribute("aria-expanded"), "false");
    }
    if (/Other items/.test(document.body.textContent)) problems.push("Other items is still drawn");
    // An empty section is not drawn.
    const some = await drawn(manualSlice([rows[2]], []), [manual[2]]);
    expect(problems, "with only a later release, only that section", [...some.document.querySelectorAll(".manual-groups h2")].map((h) => h.textContent.trim()), ["Later releases · 1 item, 1 check"]);
    expect(problems, "and it is open when nothing holds", some.document.getElementById("manual-later-head")?.getAttribute("aria-expanded"), "true");
    // A project with no release at all lists its items with no section heading.
    const bare = await drawn(manualSlice([rows[3]], []), [manual[3]]);
    expect(problems, "no release anywhere: no section heading", [...bare.document.querySelectorAll(".manual-groups h2")].map((h) => h.textContent.trim()), []);
    expect(problems, "but the item is listed", heads(bare.document), ["FEAT-004 Item FEAT-004 · Completed · 1 check"]);
    renderChecks.push(["web::check_manual_checks_split_the_rest_by_release", problems]);
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
  // DEF-113 R-7, rendered and pressed: Retire N over the shipped section's own selection.
  {
    const problems = [];
    document.body.innerHTML = '<div id="root"></div>';
    const { load } = await import("./lib/render.mjs");
    const mod = await load("src/pages/review/ManualGroup.tsx");
    const React = (await import("react")).default;
    const { createRoot } = await import("react-dom/client");
    const { MemoryRouter } = await import("react-router-dom");
    const rows = [
      sliceRow("FEAT-001", "R-1", "manual: a", { release: "v0.6.0" }),
      sliceRow("FEAT-001", "R-2", "manual: b", { release: "v0.6.0" }),
      sliceRow("FEAT-002", "R-1", "manual: c", { release: "v0.5.0", prepared: "P-1" }),
    ];
    const manual = rows.map((r) => ({ ...waiting(r.code, r.requirement, r.test, { release: r.release }), prepared: r.prepared ? { id: r.prepared, kind: "manual-test" } : undefined }));
    const slice = { ...manualSlice(rows, []), shipped: ["v0.6.0", "v0.5.0"] };
    const retired = [];
    const picked = [];
    const props = (writable) => ({
      project: "x", slice, rows, manual, writable,
      sel: { group: "manual", keys: new Set() },
      onSelect: (k, v) => picked.push([k, v]),
      onSelectItem: () => {}, onConfirmItem: () => {},
      onRetire: (sel, reason) => retired.push([[...sel.keys], reason]),
      reasonFor: (r) => r.blocked, openRows: new Set(), onOpen: () => {}, inFlight: false, body: () => "",
    });
    const root = createRoot(document.getElementById("root"));
    const show = (w) => React.act(() => root.render(React.createElement(MemoryRouter, null, React.createElement(mod.ManualGroup, props(w)))));
    const text = () => document.getElementById("root").textContent;
    const buttons = () => [...document.querySelectorAll("button")].map((b) => b.textContent.trim());
    const click = (el) => React.act(async () => { el.dispatchEvent(new window.MouseEvent("click", { bubbles: true })); });
    const press = (label) => {
      const b = [...document.querySelectorAll("button")].find((x) => x.textContent.trim() === label);
      if (!b) throw new Error(`no button "${label}" among ${JSON.stringify(buttons())}`);
      return click(b);
    };
    await show(true);
    await click(document.getElementById("manual-shipped-head"));
    const boxes = (id) => [...document.querySelectorAll(`[id="${id}"] input[type=checkbox]`)];
    expect(problems, "three checkboxes in the shipped section", document.querySelectorAll('[id="manual-shipped-body"] input[type=checkbox]').length, 3);
    // The prepared row keeps the group's checkbox; the other two are the section's own.
    await click(boxes("FEAT-002/R-1/manual: c")[0]);
    expect(problems, "the prepared row ticks the group's selection", picked, [["FEAT-002/R-1/manual: c", true]]);
    expect(problems, "and does not arm Retire", buttons().filter((b) => /^Retire/.test(b)), ["Retire 0"]);
    await click(boxes("FEAT-001/R-1/manual: a")[0]);
    await click(boxes("FEAT-001/R-2/manual: b")[0]);
    expect(problems, "two ticked rows read Retire 2", buttons().filter((b) => /^Retire/.test(b)), ["Retire 2"]);
    expect(problems, "and did not touch the group's selection", picked.length, 1);
    await press("Retire 2");
    expect(problems, "the first press calls nothing", retired.length, 0);
    ok(problems, "it asks, naming both", text().includes("Retire FEAT-001 R-1 and FEAT-001 R-2? They stay not proven; the reason is kept with each."), text().slice(0, 500));
    const reason = document.querySelector('input[aria-label="Reason"]');
    expect(problems, "with a reason box", [reason != null, reason?.getAttribute("maxlength")], [true, "4000"]);
    const second = () => [...document.querySelectorAll(".ask-first button, .retire-ask button")].find((b) => b.textContent.trim() === "Retire 2");
    expect(problems, "the second button waits for a reason", second()?.disabled, true);
    await React.act(async () => {
      const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      set.call(reason, "   ");
      reason.dispatchEvent(new window.Event("input", { bubbles: true }));
    });
    expect(problems, "a blank reason keeps it disabled", second()?.disabled, true);
    await React.act(async () => {
      const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      set.call(document.querySelector('input[aria-label="Reason"]'), "  shipped in v0.6.0; not judged  ");
      document.querySelector('input[aria-label="Reason"]').dispatchEvent(new window.Event("input", { bubbles: true }));
    });
    expect(problems, "a reason enables it", second()?.disabled, false);
    await click(second());
    expect(problems, "the second press calls onRetire with those two keys and the trimmed reason", retired, [[["FEAT-001/R-1/manual: a", "FEAT-001/R-2/manual: b"], "shipped in v0.6.0; not judged"]]);
    expect(problems, "and the section's selection is cleared", buttons().filter((b) => /^Retire/.test(b)), ["Retire 0"]);
    await React.act(() => root.unmount());
    // A read-only monitor offers no Retire.
    const ro = createRoot(document.getElementById("root"));
    await React.act(() => ro.render(React.createElement(MemoryRouter, null, React.createElement(mod.ManualGroup, props(false)))));
    await click(document.getElementById("manual-shipped-head"));
    if (buttons().some((b) => /Retire|Select all/.test(b)) || document.querySelector("input[type=checkbox]")) problems.push(`a read-only render offers a way to retire: ${JSON.stringify(buttons())}`);
    await React.act(() => ro.unmount());
    // The GroupView sends it through the same guard as every group action.
    const view = readFileSync(new URL("../src/pages/review/GroupView.tsx", import.meta.url), "utf8");
    if (!/send\("retire", chosen, reason\)/.test(view)) problems.push('GroupView does not send("retire", chosen, reason)');
    if (!/guard\.current\?\.run/.test(view)) problems.push("GroupView lost its guardInFlight");
    renderChecks.push(["web::check_manual_shipped_checks_retire_together", problems]);
  }
  // DEF-113 R-7 (25 Nov 2026): a shipped row the group's Confirm refuses is still ticked into Retire.
  {
    const problems = [];
    document.body.innerHTML = '<div id="root"></div>';
    const { load } = await import("./lib/render.mjs");
    const mod = await load("src/pages/review/ManualGroup.tsx");
    const React = (await import("react")).default;
    const { createRoot } = await import("react-dom/client");
    const { MemoryRouter } = await import("react-router-dom");
    const why = "No result is prepared for this check";
    const rows = [
      sliceRow("FEAT-001", "R-1", "manual: a", { release: "v0.6.0", blocked: why }),
      sliceRow("FEAT-002", "R-1", "manual: c", { release: "v0.5.0", prepared: "P-1" }),
    ];
    const manual = rows.map((r) => ({ ...waiting(r.code, r.requirement, r.test, { release: r.release }), prepared: r.prepared ? { id: r.prepared, kind: "manual-test" } : undefined }));
    const slice = { ...manualSlice(rows, []), shipped: ["v0.6.0", "v0.5.0"] };
    const props = {
      project: "x", slice, rows, manual, writable: true,
      sel: { group: "manual", keys: new Set() },
      onSelect: () => {}, onSelectItem: () => {}, onConfirmItem: () => {}, onRetire: () => {},
      reasonFor: (r) => r.blocked, openRows: new Set(), onOpen: () => {}, inFlight: false, body: () => "",
    };
    const root = createRoot(document.getElementById("root"));
    await React.act(() => root.render(React.createElement(MemoryRouter, null, React.createElement(mod.ManualGroup, props))));
    await React.act(async () => { document.getElementById("manual-shipped-head").dispatchEvent(new window.MouseEvent("click", { bubbles: true })); });
    const box = document.querySelector('[id="FEAT-001/R-1/manual: a"] input[type=checkbox]');
    expect(problems, "a refused, not-prepared shipped row's checkbox is enabled", box?.disabled, false);
    if (box) await React.act(async () => { box.dispatchEvent(new window.MouseEvent("click", { bubbles: true })); });
    const retire = [...document.querySelectorAll("button")].map((b) => b.textContent.trim()).filter((b) => /^Retire/.test(b));
    expect(problems, "ticking it arms Retire 1", retire, ["Retire 1"]);
    const prepared = document.querySelector('[id="FEAT-002/R-1/manual: c"] input[type=checkbox]');
    expect(problems, "the prepared row keeps the group's checkbox, enabled", prepared?.disabled, false);
    await React.act(() => root.unmount());
    renderChecks.push(["web::check_manual_a_shipped_row_selects_for_retire", problems]);
  }
  // FTR-038 T12 (2) and (3), the stylesheets as the browser reads them (jsdom draws no layout).
  {
    const sheet = (rel) => {
      const css = readFileSync(new URL(rel, import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
      return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({ sel: m[1].trim(), body: m[2] }));
    };
    const rules = [...sheet("../src/ui/ui.css"), ...sheet("../src/styles.css")];
    const decl = (sel, re) => rules.some((r) => r.sel.split(",").map((x) => x.trim()).includes(sel) && re.test(r.body));
    // (2) every row keeps its arrow and ellipsizes inside the page.
    {
      const problems = [];
      const want = (sel, re, why) => {
        if (!decl(sel, re)) problems.push(`${sel} lacks ${why}`);
      };
      want(".ui-row", /min-width:\s*0/, "min-width: 0");
      want(".ui-row-head", /min-width:\s*0/, "min-width: 0");
      want(".ui-row-meta", /min-width:\s*0/, "min-width: 0 (the pills give way, the title keeps its arrow)");
      want(".ui-row-meta", /flex:\s*0\s+1\s+auto/, "flex: 0 1 auto (the pills shrink)");
      want(".ui-row-title", /min-width:\s*(?:[3-9]|\d\d)(?:\.\d+)?rem/, "a minimum width that keeps the arrow and a few letters");
      want(".review-row-title", /min-width:\s*0/, "min-width: 0");
      want(".review-row-title", /text-overflow:\s*ellipsis/, "an ellipsis");
      want(".review-row-records", /min-width:\s*0/, "min-width: 0");
      for (const c of [".review-group", ".review-group-rows", ".manual-groups", ".manual-section", ".manual-section-items", ".manual-item-wrap", ".manual-item-rows"])
        want(c, /min-width:\s*0/, "min-width: 0 (a flex item that cannot shrink widens the page)");
      want(".ui-row-body", /overflow-wrap:\s*anywhere|overflow-x:\s*auto/, "overflow-wrap or overflow-x, so long evidence stays inside the row");
      renderChecks.push(["web::check_manual_check_rows_fit_the_page_with_their_arrow", problems]);
    }
    // (3) the item heading's arrow sits on the heading's first line, like a Review row's.
    {
      const problems = [];
      const has = (re, pred) => rules.some((r) => re.test(r.sel) && pred(r.body));
      if (!has(/\.ui-row-title > button:has\(> \.manual-item-title\)$/, (b) => /display:\s*grid/.test(b) && /grid-template-columns:\s*auto\s+minmax\(0,\s*1fr\)/.test(b)))
        problems.push("an item heading's button is not the two-column grid (auto, minmax(0, 1fr))");
      if (!has(/\.ui-row-title > button:has\(> \.manual-item-title\)::before$/, (b) => /grid-column:\s*1/.test(b) && /grid-row:\s*1/.test(b)))
        problems.push("the heading's arrow is not placed in column 1, row 1");
      if (!has(/\.ui-row-title > button:has\(> \.manual-item-title\) > \*$/, (b) => /grid-column:\s*2/.test(b)))
        problems.push("the heading's text is not placed in column 2");
      if (!decl(".manual-item-title", /text-overflow:\s*ellipsis/)) problems.push(".manual-item-title lost its ellipsis");
      renderChecks.push(["web::check_manual_item_arrow_sits_beside_its_heading", problems]);
    }
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
