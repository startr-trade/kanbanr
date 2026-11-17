#!/usr/bin/env node
//
// The Review page groups waiting decisions by the decision (FTR-038), the pure parts:
//   web::check_review_tabs_carry_counts_and_open_the_largest_group (R-1) — groupTabs' labels and
//     hidden groups; defaultGroup honours ?group=, falls back to the largest, and ignores a hidden
//     requested group;
//   web::check_review_selection_stays_within_one_group (R-2) — select ignores a key from another
//     group and a blocked row; selectAll takes only unblocked, checked rows; switching groups
//     empties the selection; actionLabel names the action and the count;
//   web::check_review_prepared_rows_arrive_selected (R-5) — preselected takes the prepared,
//     unblocked rows; unticking one keeps the rest;
//   web::check_review_url_names_the_group_and_open_rows (R-6) — ?group=docs-folded#DEF-066 opens
//     that tab and row; a changed filter writes r.q; rendered, the hash opens the row and r.q
//     filters the rows;
// and the page (T5), rendered through scripts/lib/render.mjs:
//   web::check_review_group_action_asks_first_naming_the_items (R-3) — askText's wording with the
//     five-names-and-more rule; the group button arms on the first press (AskFirst) and posts
//     api.decideGroup only on the second;
//   web::check_review_undecidable_row_says_why_and_cannot_be_selected (R-2) — a blocked row shows
//     its sentence beside a disabled checkbox, and select refuses it;
//   web::check_review_rows_are_single_line_and_the_action_bar_is_sticky (R-9) — the group's head is
//     sticky at the top; a row is an AccordionRow whose only second line is its records line, with
//     its properties as pills;
//   web::check_review_rows_are_headed (R-10) — each row's title is inside an h3, and the page has
//     one h1;
// and each row's evidence (T6):
//   web::check_review_reasoned_actions_stay_in_the_row (R-11) — actionLabel never offers ADR Reject
//     or a manual Passed or Failed; the delegated group offers Keep N and Undo N; the rendered ADR
//     body has Reject with a reason box, an unprepared manual body its own Passed and Failed, and a
//     delegated body its Undo beside the reason.
//
// Usage: npm run check:review-groups
import {
  GROUP_PARAM,
  TABLE_STATE_KEY,
  actionLabel,
  askText,
  batchOf,
  decisionBody,
  defaultGroup,
  emptySelection,
  groupTabs,
  preselected,
  reviewTabs,
  select,
  selectAll,
} from "../src/reviewGroups.ts";
import { parseOpen, parseState, writeState } from "../src/ui/urlState.ts";
import { readFileSync, readdirSync } from "node:fs";
import { JSDOM } from "jsdom";
import { closeRender, render } from "./lib/render.mjs";

const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
};
const keys = (sel) => [...sel.keys].sort();

const count = (key, label, n, action = label) => ({ key, label, count: n, action, ready: n, prepared: 0 });
const row = (key, extra = {}) => ({
  key,
  code: key,
  title: `Title of ${key}`,
  what: "never approved",
  records: "Records your approval",
  release: "",
  sprint: "",
  changed_at: "",
  blocked: "",
  pills: [],
  prepared: "",
  batch: "",
  rev: `rev-${key}`,
  spec_rev: "",
  signoff: "",
  requirement: "",
  test: "",
  ...extra,
});
const slice = (key, label, action, rows, choices = []) => ({ key, label, action, rows, choices });

let ok = true;

// R-1: the tabs and the group the page opens on.
{
  const problems = [];
  const groups = [
    count("approve", "Approve", 3),
    count("ratify", "Ratify", 0),
    count("docs-folded", "Sign off docs-folded", 7, "Sign off"),
    count("manual", "Manual checks", 2, "Confirm"),
  ];
  const tabs = groupTabs(groups);
  eq(problems, "one tab per group, in the summary's order", tabs.map((t) => t.key), ["approve", "ratify", "docs-folded", "manual"]);
  eq(problems, "each tab is the group's label with its count", tabs.map((t) => [t.label, t.count]), [
    ["Approve", 3],
    ["Ratify", 0],
    ["Sign off docs-folded", 7],
    ["Manual checks", 2],
  ]);
  eq(problems, "an empty group is hidden", tabs.map((t) => !!t.hidden), [false, true, false, false]);
  eq(problems, "with no group in the URL, the largest group opens", defaultGroup(groups, null), "docs-folded");
  eq(problems, "?group= names the group that opens", defaultGroup(groups, "manual"), "manual");
  eq(problems, "a requested empty (hidden) group falls back to the largest", defaultGroup(groups, "ratify"), "docs-folded");
  eq(problems, "an unknown requested group falls back to the largest", defaultGroup(groups, "nope"), "docs-folded");
  eq(problems, "the largest on a tie is the first", defaultGroup([count("a", "A", 2), count("b", "B", 2)], null), "a");
  eq(problems, "no groups: nothing opens", defaultGroup([], "approve"), "");
  ok = report("web::check_review_tabs_carry_counts_and_open_the_largest_group", problems) && ok;
}

// R-2: selection stays inside one group.
{
  const problems = [];
  const rows = [row("FTR-1"), row("FTR-2"), row("FTR-3", { blocked: "Claude settles 1 text first: ask Claude to check them." }), row("FTR-4")];
  const approve = slice("approve", "Approve", "Approve", rows);
  let sel = emptySelection("approve");
  eq(problems, "an empty selection names its group", [sel.group, keys(sel)], ["approve", []]);

  sel = select(sel, "approve", "FTR-1", true, rows);
  sel = select(sel, "approve", "FTR-2", true, rows);
  eq(problems, "ticking two rows of the group", keys(sel), ["FTR-1", "FTR-2"]);
  const before = sel;
  eq(problems, "a key from another group is ignored", keys(select(sel, "docs-folded", "FTR-4", true, rows)), ["FTR-1", "FTR-2"]);
  if (select(sel, "docs-folded", "FTR-4", true, rows) !== before) problems.push("select with another group must return the selection unchanged");
  eq(problems, "a blocked row cannot be selected", keys(select(sel, "approve", "FTR-3", true, rows)), ["FTR-1", "FTR-2"]);
  eq(problems, "a key that is not a row of the group is ignored", keys(select(sel, "approve", "FTR-9", true, rows)), ["FTR-1", "FTR-2"]);
  eq(problems, "a row whose diagram check is pending cannot be selected", keys(select(sel, "approve", "FTR-4", true, rows, new Set(["FTR-4"]))), ["FTR-1", "FTR-2"]);
  eq(problems, "unticking a row", keys(select(sel, "approve", "FTR-1", false, rows)), ["FTR-2"]);
  if (!before.keys.has("FTR-1") || before.keys.size !== 2) problems.push("select must not change the selection it was given");

  eq(problems, "select-all takes only unblocked, checked rows", keys(selectAll(emptySelection("approve"), rows, true, new Set(["FTR-2"]))), ["FTR-1", "FTR-4"]);
  eq(problems, "select-all with nothing pending", keys(selectAll(emptySelection("approve"), rows, true)), ["FTR-1", "FTR-2", "FTR-4"]);
  eq(problems, "select-all off empties the group's selection", keys(selectAll(sel, rows, false)), []);

  const other = slice("docs-folded", "Sign off docs-folded", "Sign off", [row("FTR-1"), row("DEF-9")]);
  const switched = preselected(other);
  eq(problems, "switching groups empties the selection", [switched.group, keys(switched)], ["docs-folded", []]);
  eq(problems, "a key of the old group is not carried into the new one", keys(select(switched, "approve", "FTR-2", true, rows)), []);

  eq(problems, "the approve button names the action and the count", actionLabel(approve, sel), "Approve 2");
  eq(problems, "the sign-off button names the sign-off", actionLabel(other, { group: "docs-folded", keys: new Set(["FTR-1", "DEF-9"]) }), "Sign off docs-folded 2");
  const which = slice("which-release", "Which release", "Plan into", [row("FTR-1"), row("FTR-2")], ["v0.8.0", "v0.9.0"]);
  eq(problems, "the which-release button names the release", actionLabel(which, { group: "which-release", keys: new Set(["FTR-1", "FTR-2"]) }, "v0.8.0"), "Plan 2 into v0.8.0");
  eq(problems, "a selection from another group gives no count", actionLabel(approve, { group: "docs-folded", keys: new Set(["FTR-1"]) }), "Approve 0");
  eq(problems, "the manual group confirms", actionLabel(slice("manual", "Manual checks", "Confirm", []), emptySelection("manual")), "Confirm 0");

  eq(problems, "the question names the selected items in row order", askText(approve, { group: "approve", keys: new Set(["FTR-4", "FTR-1"]) }, rows), "Approve FTR-1 and FTR-4?");
  const many = Array.from({ length: 8 }, (_, i) => row(`DEF-${i + 1}`));
  const all8 = { group: "approve", keys: new Set(many.map((r) => r.key)) };
  eq(problems, "more than six: the first five and how many more", askText(slice("approve", "Approve", "Approve", many), all8, many), "Approve DEF-1, DEF-2, DEF-3, DEF-4, DEF-5 and 3 more?");
  const six = many.slice(0, 6);
  eq(problems, "six are all named", askText(slice("approve", "Approve", "Approve", six), { group: "approve", keys: new Set(six.map((r) => r.key)) }, six), "Approve DEF-1, DEF-2, DEF-3, DEF-4, DEF-5 and DEF-6?");

  const body = decisionBody(approve, sel, rows, "Alice <a@example.invalid>", "");
  eq(problems, "the decision body names the group, the action and who", [body.group, body.action, body.by, body.target, body.reason], ["approve", "Approve", "Alice <a@example.invalid>", "", ""]);
  eq(problems, "each item carries the pins the slice showed", body.items, [
    { key: "FTR-1", code: "FTR-1", rev: "rev-FTR-1", spec_rev: "", signoff: "", requirement: "", test: "", prepared: "" },
    { key: "FTR-2", code: "FTR-2", rev: "rev-FTR-2", spec_rev: "", signoff: "", requirement: "", test: "", prepared: "" },
  ]);
  eq(problems, "a selection from another group sends no items", decisionBody(approve, { group: "docs-folded", keys: new Set(["FTR-1"]) }, rows, "a", "").items, []);
  eq(problems, "the which-release body carries the target", decisionBody(which, { group: "which-release", keys: new Set(["FTR-2"]) }, which.rows, "a", "v0.9.0").target, "v0.9.0");
  ok = report("web::check_review_selection_stays_within_one_group", problems) && ok;
}

// R-5: a batch Claude prepared arrives selected, and the person can untick any of it.
{
  const problems = [];
  const rows = [
    row("DEF-1/R-1/manual: a", { code: "DEF-1", requirement: "R-1", test: "manual: a", prepared: "P-1", batch: "agile-1" }),
    row("DEF-2/R-1/manual: b", { code: "DEF-2", requirement: "R-1", test: "manual: b", prepared: "P-2", batch: "agile-1" }),
    row("DEF-3/R-2/manual: c", { code: "DEF-3", requirement: "R-2", test: "manual: c", prepared: "P-3", blocked: "Do the check, then press Passed or Failed in the row." }),
    row("DEF-4/R-1/manual: d", { code: "DEF-4", requirement: "R-1", test: "manual: d" }),
  ];
  const manual = slice("manual", "Manual checks", "Confirm", rows);
  const pre = preselected(manual);
  eq(problems, "the prepared, unblocked rows arrive selected", [pre.group, keys(pre)], ["manual", ["DEF-1/R-1/manual: a", "DEF-2/R-1/manual: b"]]);
  eq(problems, "the selection that is one whole batch reads Confirm all N", actionLabel(manual, pre), "Confirm all 2");
  eq(problems, "and is that batch", batchOf(manual, pre), "agile-1");
  eq(problems, "the question names the batch's items", askText(manual, pre, rows), "Confirm DEF-1 R-1 and DEF-2 R-1?");
  const unticked = select(pre, "manual", "DEF-2/R-1/manual: b", false, rows);
  eq(problems, "unticking one keeps the rest", keys(unticked), ["DEF-1/R-1/manual: a"]);
  eq(problems, "a partial batch is not the batch", batchOf(manual, unticked), null);
  eq(problems, "a partial batch reads the plain count", actionLabel(manual, unticked), "Confirm 1");
  const plus = select(pre, "manual", "DEF-4/R-1/manual: d", true, rows);
  eq(problems, "a batch plus another row is not the batch", batchOf(manual, plus), null);
  eq(problems, "nothing selected is no batch", batchOf(manual, emptySelection("manual")), null);
  const body = decisionBody(manual, unticked, rows, "a", "");
  eq(problems, "a prepared row sends its record", [body.items[0].prepared, body.items[0].requirement, body.items[0].test], ["P-1", "R-1", "manual: a"]);
  const preWithPending = preselected(slice("approve", "Approve", "Approve", [row("FTR-1", { prepared: "P-9" }), row("FTR-2", { prepared: "P-8" })]), new Set(["FTR-2"]));
  eq(problems, "a prepared row whose diagram check is pending is not preselected", keys(preWithPending), ["FTR-1"]);
  ok = report("web::check_review_prepared_rows_arrive_selected", problems) && ok;
}

// R-6, the pure parts: the URL names the group and the open rows; the filter is kept under r.
{
  const problems = [];
  eq(problems, "the group parameter", GROUP_PARAM, "group");
  const url = "?group=docs-folded";
  const requested = parseState(url, { [GROUP_PARAM]: "" })[GROUP_PARAM];
  const groups = [count("approve", "Approve", 9), count("docs-folded", "Sign off docs-folded", 2, "Sign off")];
  eq(problems, "?group=docs-folded opens that tab", defaultGroup(groups, requested), "docs-folded");
  eq(problems, "#DEF-066 opens that row", [...parseOpen("#DEF-066")], ["DEF-066"]);
  eq(problems, "#DEF-066,FTR-041 opens both", [...parseOpen("#DEF-066,FTR-041")], ["DEF-066", "FTR-041"]);
  const tableDefaults = { sort: "", dir: "asc", q: "" };
  eq(problems, "a changed filter writes r.q and keeps the group", writeState(url, { q: "manual" }, tableDefaults, TABLE_STATE_KEY), "?group=docs-folded&r.q=manual");
  eq(problems, "the filter is read back from r.q", parseState("?group=docs-folded&r.q=manual&r.sort=release", tableDefaults, TABLE_STATE_KEY), { sort: "release", dir: "asc", q: "manual" });

  // Rendered: the hash opens the row it names, and r.q filters the rows drawn.
  try {
    const rows = [row("DEF-066", { what: "waiting on docs-folded" }), row("FTR-041", { what: "waiting on docs-folded" })];
    const docs = slice("docs-folded", "Sign off docs-folded", "Sign off", rows);
    const opened = await renderPanel(docs, { url: "/p/x/review?group=docs-folded#DEF-066" });
    const toggle = (id) => opened.document.getElementById(`${id}-head`);
    eq(problems, "#DEF-066 opens that row", toggle("DEF-066")?.getAttribute("aria-expanded"), "true");
    eq(problems, "the other row stays closed", toggle("FTR-041")?.getAttribute("aria-expanded"), "false");
    eq(problems, "the open row's body is its evidence", opened.document.getElementById("DEF-066-body")?.textContent, "evidence of DEF-066");
    const filtered = await renderPanel(docs, { url: "/p/x/review?group=docs-folded&r.q=FTR-041" });
    eq(problems, "r.q filters the rows drawn", [...filtered.document.querySelectorAll("section.ui-row")].map((s) => s.id), ["FTR-041"]);
    eq(problems, "the filter box shows r.q", filtered.document.querySelector("input.review-group-filter")?.getAttribute("value"), "FTR-041");
  } catch (e) {
    problems.push(`rendering the group failed: ${e?.stack ?? e}`);
  }
  ok = report("web::check_review_url_names_the_group_and_open_rows", problems) && ok;
}

/** GroupView's panel, rendered with a slice and a body that names the row. */
async function renderPanel(s, { url = "/p/x/review", writable = true } = {}) {
  return render(
    "src/pages/review/GroupView.tsx",
    "GroupPanel",
    { project: "x", slice: s, writable, who: "Alice <a@example.invalid>", tick: 0, body: (r) => `evidence of ${r.key}` },
    { url },
  );
}

try {
  // R-3: every group action asks first, naming the items; the second press posts.
  {
    const problems = [];
    const many = Array.from({ length: 7 }, (_, i) => row(`FTR-${i + 1}`));
    const approve = slice("approve", "Approve", "Approve", many);
    const sel = (keys) => ({ group: "approve", keys: new Set(keys) });
    eq(problems, "one item", askText(approve, sel(["FTR-1"]), many), "Approve FTR-1?");
    eq(problems, "three items", askText(approve, sel(["FTR-1", "FTR-2", "FTR-3"]), many), "Approve FTR-1, FTR-2 and FTR-3?");
    eq(problems, "seven: five named and how many more", askText(approve, sel(many.map((r) => r.key)), many), "Approve FTR-1, FTR-2, FTR-3, FTR-4, FTR-5 and 2 more?");

    const prepared = [row("DEF-1", { prepared: "P-1" }), row("DEF-2", { prepared: "P-2" })];
    const docs = slice("docs-folded", "Sign off docs-folded", "Sign off", prepared);
    const { document, html } = await renderPanel(docs);
    const head = document.querySelector(".review-group-head");
    if (!head) problems.push(`no .review-group-head: ${html}`);
    const buttons = [...(head?.querySelectorAll("button") ?? [])].map((b) => b.textContent.trim());
    if (!buttons.includes("Sign off docs-folded 2")) problems.push(`the group button does not read "Sign off docs-folded 2": ${JSON.stringify(buttons)}`);
    if (document.querySelector(".ask-first")) problems.push("the question shows before the first press");

    const src = read("../src/pages/review/GroupView.tsx");
    if (!/import \{ AskFirst \} from "\.\.\/\.\.\/components\/AskFirst"/.test(src)) problems.push("GroupView.tsx does not use DEF-034's AskFirst");
    const posts = src.match(/api\.decideGroup\(/g) ?? [];
    if (posts.length !== 1) problems.push(`GroupView.tsx posts api.decideGroup ${posts.length} times, expected once`);
    const sendAt = src.indexOf("const send = ");
    const sendEnd = src.indexOf("\n  };\n", sendAt);
    if (sendAt < 0 || sendEnd < 0 || !src.slice(sendAt, sendEnd).includes("api.decideGroup(")) problems.push("api.decideGroup is not posted from send");
    if (!/<AskFirst[\s\S]*?onConfirm=\{\(\) => send\(/.test(src)) problems.push("send is not AskFirst's onConfirm (the second press)");
    if (/onClick=\{[^}]*\bsend\(/.test(src)) problems.push("a button posts on its first press (onClick calls send)");
    if (!/guardInFlight\(/.test(src)) problems.push("the send does not go through guardInFlight (DEF-090)");

    const readOnly = await renderPanel(docs, { writable: false });
    if (readOnly.document.querySelector('input[type="checkbox"]')) problems.push("a read-only monitor shows checkboxes");
    if ([...readOnly.document.querySelectorAll(".review-group-head button")].some((b) => /Sign off/.test(b.textContent))) {
      problems.push("a read-only monitor offers the group button");
    }
    ok = report("web::check_review_group_action_asks_first_naming_the_items", problems) && ok;
  }

  // R-2: a row that cannot be decided says why, and cannot be selected.
  {
    const problems = [];
    const why = "Claude settles 1 text first: ask Claude to check them.";
    const rows = [row("FTR-1"), row("FTR-3", { blocked: why })];
    const docs = slice("docs-folded", "Sign off docs-folded", "Sign off", rows);
    eq(problems, "select refuses the blocked row", [...select(emptySelection("docs-folded"), "docs-folded", "FTR-3", true, rows).keys], []);
    const { document, html } = await renderPanel(docs);
    const box = (id) => document.getElementById(id)?.querySelector('.ui-row-select input[type="checkbox"]');
    if (!box("FTR-3")) problems.push(`the blocked row has no checkbox: ${html}`);
    else if (!box("FTR-3").hasAttribute("disabled")) problems.push("the blocked row's checkbox is not disabled");
    eq(problems, "the blocked row says why", document.getElementById("FTR-3")?.querySelector(".ui-row-reason")?.textContent, why);
    if (!box("FTR-1") || box("FTR-1").hasAttribute("disabled")) problems.push("a row that can be decided is not selectable");
    const approve = await renderPanel(slice("approve", "Approve", "Approve", [row("FTR-1")]));
    const checking = approve.document.getElementById("FTR-1");
    if (!checking?.querySelector('.ui-row-select input[type="checkbox"]')?.hasAttribute("disabled")) {
      problems.push("an approve row is selectable before its diagram check has finished");
    }
    eq(problems, "an approve row says it is checking diagrams", checking?.querySelector(".ui-row-reason")?.textContent, "checking diagrams…");
    ok = report("web::check_review_undecidable_row_says_why_and_cannot_be_selected", problems) && ok;
  }

  // R-9: single-line rows with pills; the group's action bar is sticky at its top.
  {
    const problems = [];
    const css = read("../src/styles.css").replace(/\/\*[\s\S]*?\*\//g, "");
    const rule = /\.review-group-head\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    if (!rule) problems.push("styles.css has no .review-group-head rule");
    if (!/position:\s*sticky/.test(rule)) problems.push(".review-group-head is not position: sticky");
    if (!/\btop:\s*0\b/.test(rule)) problems.push(".review-group-head is not at top: 0");
    if (!/background:\s*var\(--surface\)/.test(rule)) problems.push(".review-group-head has no var(--surface) background, so rows would show through");
    if (/#[0-9a-fA-F]{3,8}\b|rgba?\(/.test(rule)) problems.push(".review-group-head uses a raw colour, not a token");

    const r = row("DEF-066", { pills: ["prepared", "batch agile-1"], release: "v0.8.0", sprint: "SP-004", records: "Records the docs-folded sign-off" });
    const { document, html } = await render("src/pages/review/ReviewRow.tsx", "ReviewRow", {
      project: "x",
      row: r,
      open: false,
      onToggle: () => {},
      children: () => "body",
    });
    const head = document.querySelector("section.ui-row > .ui-row-head");
    if (!head) problems.push(`the row is not an AccordionRow: ${html}`);
    else {
      const records = head.querySelectorAll(".review-row-records");
      eq(problems, "one records line in the header", records.length, 1);
      eq(problems, "the records line's text", records[0]?.textContent, "Records the docs-folded sign-off");
      if (head.querySelector("p, div, ul, table")) problems.push("the row's header holds a block beyond its records line");
      const pills = [...head.querySelectorAll(".ui-pill")].map((e) => e.textContent);
      eq(problems, "the properties are pills", pills, ["prepared", "batch agile-1", "v0.8.0", "SP-004"]);
    }
    ok = report("web::check_review_rows_are_single_line_and_the_action_bar_is_sticky", problems) && ok;
  }

  // R-10: every row's header is a heading; the page has one h1.
  {
    const problems = [];
    const rows = [row("FTR-41"), row("FTR-43"), row("DEF-71")];
    const { document, html } = await renderPanel(slice("docs-folded", "Sign off docs-folded", "Sign off", rows));
    const sections = [...document.querySelectorAll("section.ui-row")];
    eq(problems, "three rows", sections.length, 3);
    for (const s of sections) {
      const h3 = s.querySelector(".ui-row-head h3");
      if (!h3) problems.push(`${s.id}: no h3 in the row's header: ${html}`);
      else if (!h3.textContent.includes(`Title of ${s.id}`)) problems.push(`${s.id}: the title is not inside the h3`);
    }
    if (document.querySelector("h1")) problems.push("the group view draws an h1 of its own");
    const page = read("../src/pages/ReviewPage.tsx");
    eq(problems, "the page has one h1", (page.match(/<h1[\s>]/g) ?? []).length, 1);
    ok = report("web::check_review_rows_are_headed", problems) && ok;
  }

  // R-11: actions that need a reason or the person's own observation stay in the opened row.
  {
    const problems = [];
    const two = [row("ADR-0001", { code: "ADR-0001" }), row("ADR-0002", { code: "ADR-0002" })];
    const both = (rows, group) => ({ group, keys: new Set(rows.map((r) => r.key)) });
    const reasoned = /Reject|Passed|Failed/;
    const adrs = slice("adr", "Proposed ADRs", "Accept", two);
    const manualRows = [row("DEF-1/R-1/manual: a", { code: "DEF-1", requirement: "R-1", test: "manual: a" })];
    const manual = slice("manual", "Manual checks", "Confirm", manualRows);
    for (const [s, sel] of [
      [adrs, both(two, "adr")],
      [adrs, emptySelection("adr")],
      [manual, both(manualRows, "manual")],
      [manual, emptySelection("manual")],
    ]) {
      const label = actionLabel(s, sel);
      if (reasoned.test(label)) problems.push(`the ${s.key} group's action offers a reasoned action: ${label}`);
    }
    eq(problems, "the ADR group accepts", actionLabel(adrs, both(two, "adr")), "Accept 2");
    const delegatedRows = [row("FTR-1/approve/-/at", { code: "FTR-1" }), row("FTR-2/approve/-/at", { code: "FTR-2" })];
    const delegated = slice("delegated", "Delegated decisions", "Keep", delegatedRows);
    eq(problems, "the delegated group keeps", actionLabel(delegated, both(delegatedRows, "delegated")), "Keep 2");
    const heads = async (s) => {
      const { document } = await renderPanel(s);
      return [...document.querySelectorAll(".review-group-head button")].map((b) => b.textContent.trim());
    };
    const keepUndo = await heads(delegated);
    if (!keepUndo.some((t) => /^Keep \d+$/.test(t)) || !keepUndo.some((t) => /^Undo \d+$/.test(t))) {
      problems.push(`the delegated group's header does not offer Keep N and Undo N: ${JSON.stringify(keepUndo)}`);
    }
    for (const s of [adrs, manual]) {
      const got = await heads(s);
      if (got.some((t) => reasoned.test(t))) problems.push(`the ${s.key} group's header offers a reasoned action: ${JSON.stringify(got)}`);
    }

    // The ADR's own body: Reject with a reason box, the button off until a reason is given. Its
    // text is drawn by Markdown, whose sanitizer needs a DOM window when it is first loaded.
    globalThis.window ??= new JSDOM("<!DOCTYPE html>").window;
    const adr = await render("src/pages/review/AdrEvidence.tsx", "AdrEvidence", {
      project: "x",
      adr: { id: "ADR-0001", title: "Keep it simple", body: "# Keep it simple\n\nWe keep it simple.", path: "decisions/0001-keep.md", missing: [], affects: [] },
      writable: true,
      busy: false,
      onRetry: () => {},
      onDecide: () => {},
    });
    if (!adr.document.querySelector('input[aria-label="Reason for rejecting ADR-0001"]')) problems.push(`the ADR's body has no reason box: ${adr.html}`);
    const reject = [...adr.document.querySelectorAll("button")].find((b) => b.textContent.trim() === "Reject ADR-0001");
    if (!reject) problems.push("the ADR's body has no Reject ADR-0001");
    else if (!reject.hasAttribute("disabled")) problems.push("Reject is offered before a reason is given");

    // An unprepared manual check's own body: Passed and Failed.
    const check = await render("src/pages/review/ManualEvidence.tsx", "ManualEvidence", {
      project: "x",
      m: {
        row: { code: "DEF-1", title: "Cart", status: "In Progress", requirement: "R-1", requirement_text: "THE SYSTEM SHALL keep it.", test: "manual: a", state: "planned", checked_rev: "", last: null },
      },
      writable: true,
      busy: false,
      onRetry: () => {},
      onRecord: () => {},
      onConfirm: async () => {},
      onReject: async () => {},
    });
    const checkButtons = [...check.document.querySelectorAll("button")].map((b) => b.textContent.trim());
    eq(problems, "an unprepared manual check's body has its own Passed and Failed", checkButtons, ["Passed", "Failed"]);

    // A delegated decision's own body: Undo, with the reason box beside it.
    const undo = await render("src/pages/review/DelegationEvidence.tsx", "DelegationEvidence", {
      project: "x",
      delegated: { decision: "FTR-1/approve/-/2026-11-16T10:00:00Z", code: "FTR-1", title: "One", verdict: "approve", delegation: "DLG-1", at: "2026-11-16T10:00:00Z", status: "Planned", undo_route: "" },
      writable: true,
      who: "Alice <a@example.invalid>",
    });
    if (!undo.document.querySelector('input[aria-label="Reason for FTR-1/approve/-/2026-11-16T10:00:00Z"]')) problems.push(`the delegated body has no reason box: ${undo.html}`);
    const undoButtons = [...undo.document.querySelectorAll("button")].map((b) => b.textContent.trim());
    eq(problems, "the delegated body has its own Keep and Undo", undoButtons, ["Keep", "Undo"]);
    ok = report("web::check_review_reasoned_actions_stay_in_the_row", problems) && ok;
  }
  // DEF-104 (supporting R-4): what a delegation holds is shown last, uncounted, and still decidable.
  {
    const problems = [];
    const g = (key, label, n, held) => ({ ...count(key, label, n), held });
    const tabs = groupTabs([g("approve", "Approve", 3, 0), g("docs-folded", "Sign off docs-folded", 0, 4), g("ratify", "Ratify", 0, 0), g("manual", "Manual", 2)]);
    const by = (k) => tabs.find((t) => t.key === k);
    eq(problems, "a held-only group's tab is shown", by("docs-folded")?.hidden, false);
    eq(problems, "its count stays 0", by("docs-folded")?.count, 0);
    eq(problems, "its label says what Claude holds", by("docs-folded")?.label, "Sign off docs-folded · 4 Claude's");
    eq(problems, "a group with nothing at all is hidden", by("ratify")?.hidden, true);
    eq(problems, "a group with no held count keeps its label", by("approve")?.label, "Approve");
    eq(problems, "a group without the field keeps its label", by("manual")?.label, "Manual");

    const held = (k) => row(k, { held_by: "DLG-001", held_until: "2026-12-04T10:00:00Z", pills: ["Claude holds it, under DLG-001"] });
    const rows = [row("DEF-1"), held("DEF-2")];
    const docs = slice("docs-folded", "Sign off docs-folded", "Sign off", rows);
    eq(problems, "a held row stays selectable", [...select(emptySelection("docs-folded"), "docs-folded", "DEF-2", true, rows).keys], ["DEF-2"]);
    const shown = await renderPanel(docs);
    const sentence = shown.document.querySelector(".review-group-sentence")?.textContent ?? "";
    if (!sentence.includes("1 of these Claude holds under DLG-001 until 4 Dec; you can still decide them.")) {
      problems.push(`the sentence does not name the delegation: ${sentence}`);
    }
    const plain = await renderPanel(slice("docs-folded", "Sign off docs-folded", "Sign off", [row("DEF-1")]));
    if ((plain.document.querySelector(".review-group-sentence")?.textContent ?? "").includes("Claude holds")) {
      problems.push("a group with no held row names a delegation");
    }
    const box = shown.document.getElementById("DEF-2")?.querySelector('.ui-row-select input[type="checkbox"]');
    if (!box || box.hasAttribute("disabled")) problems.push("a held row's checkbox is missing or disabled");

    const view = {
      mode: "delegated",
      modes: [],
      to_review: [],
      delegations: [
        { id: "DLG-001", state: "granted", verdicts: ["approve"], until: "2026-12-04T10:00:00Z" },
        { id: "DLG-002", state: "granted", verdicts: ["approve"], until: "2026-12-04T10:00:00Z" },
      ],
      waiting: [
        ...["A", "B", "C", "D"].map((c) => ({ code: `FTR-${c}`, what: "sign off design-review", place: "delegated", delegation: "DLG-001" })),
        ...["E", "F"].map((c) => ({ code: `FTR-${c}`, what: "approve its definition", place: "delegated", delegation: "DLG-001" })),
        { code: "FTR-G", what: "approve its definition", place: "monitor" },
      ],
    };
    const here = await render("src/pages/review/DecisionsHere.tsx", "DecisionsHere", { project: "x", view, error: null, writable: false, who: null });
    eq(problems, "the delegation's block is the card's link target", here.document.getElementById("DLG-001")?.tagName, "DIV");
    const text = (id) => here.document.getElementById(id)?.textContent ?? "";
    if (!text("DLG-001").includes("Holds now: 4 design reviews, 2 approvals")) problems.push(`DLG-001 does not say what it holds: ${text("DLG-001")}`);
    if (!text("DLG-002").includes("Holds nothing now.")) problems.push(`DLG-002 does not say it holds nothing: ${text("DLG-002")}`);
    ok = report("web::check_review_groups_show_what_claude_holds", problems) && ok;
  }
  // DEF-106 R-1: the tabs from the outline alone, then from the summary.
  {
    const problems = [];
    const outline = [
      { key: "approve", label: "Approve", action: "Approve" },
      { key: "docs-folded", label: "Sign off docs-folded", action: "Sign off" },
      { key: "manual", label: "Manual checks", action: "Confirm" },
    ];
    eq(problems, "neither read gives no tabs", reviewTabs(null, null), []);
    const early = reviewTabs(outline, null);
    eq(problems, "an outline alone gives its groups in its order", early.map((t) => t.key), ["approve", "docs-folded", "manual"]);
    eq(problems, "an outline tab carries its label", early.map((t) => t.label), ["Approve", "Sign off docs-folded", "Manual checks"]);
    eq(problems, "every outline tab is pending", early.map((t) => t.pending === true), [true, true, true]);
    eq(problems, "no outline tab is hidden", early.map((t) => t.hidden === true), [false, false, false]);
    const counted = [
      count("approve", "Approve", 3),
      count("docs-folded", "Sign off docs-folded", 0, "Sign off"),
      count("signoff-design-review", "Sign off design-review", 2, "Sign off"),
    ];
    eq(problems, "a summary decides once it is there", reviewTabs(outline, counted), groupTabs(counted));
    eq(problems, "a re-asked sign-off only the queue knows appears", reviewTabs(outline, counted).map((t) => t.key), ["approve", "docs-folded", "signoff-design-review"]);
    eq(problems, "an empty group is hidden once counted", reviewTabs(outline, counted).map((t) => t.hidden === true), [false, true, false]);
    eq(problems, "a summary without an outline still gives its tabs", reviewTabs(null, counted), groupTabs(counted));
    ok = report("web::check_review_tabs_merge_outline_and_counts", problems) && ok;
  }

  // DEF-106 R-1, R-10: the groups are drawn before any count, then the counts fill in.
  {
    const problems = [];
    const outline = [
      { key: "approve", label: "Approve", action: "Approve" },
      { key: "manual", label: "Manual checks", action: "Confirm" },
    ];
    const query = (extra) => ({ data: null, error: null, loading: true, refreshing: false, since: null, ...extra });
    const head = (summary, url = "/p/x/review") =>
      render(
        "src/pages/ReviewPage.tsx",
        "ReviewGroupsHead",
        { outline: query({ data: outline, loading: false }), summary, label: "Decisions waiting", children: (k) => `panel of ${k}` },
        { url },
      );
    const waiting = await head(query({ since: Date.now() - 4000 }));
    const tabs = [...waiting.document.querySelectorAll('[role="tab"]')];
    eq(problems, "every outline group is a tab before the summary answers", tabs.map((t) => t.textContent.replace(/\s+/g, " ").trim()), ["Approve …", "Manual checks …"]);
    eq(problems, "a pending tab says it is counting", tabs.map((t) => t.getAttribute("aria-label")), ["Approve, counting", "Manual checks, counting"]);
    const working = waiting.document.querySelector(".ui-working");
    if (!working || !working.textContent.includes("Counting what is waiting…")) problems.push(`no counting line: ${waiting.html}`);
    else if (!/· 4 s/.test(working.textContent)) problems.push(`the counting line does not say how long: ${working.textContent}`);
    if (!waiting.document.querySelector('[role="tabpanel"]')) problems.push("the open tab has no panel while the counts are pending");

    const counted = [count("approve", "Approve", 3), count("manual", "Manual checks", 0, "Confirm")];
    const filled = await head(query({ data: counted, loading: false }));
    const names = [...filled.document.querySelectorAll('[role="tab"]')].map((t) => t.textContent.replace(/\s+/g, " ").trim());
    eq(problems, "counted, the empty group goes and the count fills in", names, ["Approve 3"]);
    if (filled.document.querySelector(".ui-working")) problems.push("a line says it is working when nothing is in flight");

    const again = await head(query({ data: counted, loading: false, refreshing: true, since: Date.now() - 3000 }));
    eq(problems, "a re-read keeps the old counts", [...again.document.querySelectorAll('[role="tab"]')].map((t) => t.textContent.replace(/\s+/g, " ").trim()), ["Approve 3"]);
    if (!(again.document.querySelector(".ui-working")?.textContent ?? "").includes("Updating the counts…")) problems.push("a re-read does not say it is updating the counts");
    ok = report("web::check_review_draws_groups_before_counts", problems) && ok;
  }

  // DEF-106 R-3: nothing on the page reads outside the shared cache.
  {
    const problems = [];
    const dir = new URL("../src/pages/review/", import.meta.url);
    const files = [
      ["src/pages/ReviewPage.tsx", read("../src/pages/ReviewPage.tsx")],
      ...readdirSync(dir)
        .filter((f) => f.endsWith(".tsx"))
        .map((f) => [`src/pages/review/${f}`, read(`../src/pages/review/${f}`)]),
    ];
    for (const [name, text] of files) {
      const code = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
      if (/useLiveTick/.test(code)) problems.push(`${name} opens the page's own event stream (useLiveTick)`);
      if (/EventSource/.test(code)) problems.push(`${name} uses an EventSource`);
      if (/useAsync\((?:(?!;)[\s\S])*?\btick\b/.test(code)) problems.push(`${name} re-reads a useAsync on a tick`);
      if (/\btick\b/.test(code)) problems.push(`${name} still names a tick`);
    }
    const page = files[0][1];
    for (const want of ["urls.reviewOutline(", "urls.reviewSummary(", "urls.prepared(", "urls.decisions("]) {
      if (!page.includes(want)) problems.push(`ReviewPage.tsx does not read ${want} through the cache`);
    }
    const manual = read("../src/pages/review/ManualGroup.tsx");
    for (const want of ["urls.manualTests(", "urls.prepared("]) if (!manual.includes(want)) problems.push(`ManualGroup.tsx does not read ${want} through the cache`);
    if (!read("../src/pages/review/ReviewRow.tsx").includes("urls.itemDiagrams(")) problems.push("useDiagramCheck does not read through the cache");
    ok = report("web::check_review_page_reads_only_through_the_cache", problems) && ok;
  }
} catch (e) {
  console.error(`check-review-groups: rendering failed — ${e?.stack ?? e}`);
  ok = false;
} finally {
  await closeRender();
}

if (!ok) process.exit(1);
