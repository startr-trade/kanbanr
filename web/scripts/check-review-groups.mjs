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
//   web::check_review_url_names_the_group_and_open_rows (R-6), the pure parts — ?group=docs-folded
//     #DEF-066 opens that tab and row; a changed filter writes r.q.
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
  select,
  selectAll,
} from "../src/reviewGroups.ts";
import { parseOpen, parseState, writeState } from "../src/ui/urlState.ts";

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
  ok = report("web::check_review_url_names_the_group_and_open_rows", problems) && ok;
}

if (!ok) process.exit(1);
