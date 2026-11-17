#!/usr/bin/env node
//
// The UI kit's table (FTR-046, R-3):
//   web::check_table_sorts_stably_with_nulls_last — equal keys keep input order, nulls are last in
//     both directions, and FTR-9 sorts before FTR-10;
//   web::check_table_filter_matches_every_token — every token, case-insensitively;
//   web::check_table_csv_quotes_and_neutralises_formulas — RFC 4180 quoting, CRLF, and a formula-
//     leading cell prefixed with ';
//   web::check_table_markup_has_caption_and_sort_state — the rendered table.
//
// Usage: npm run check:ui-table
import { filterRows, sortRows, toCsv } from "../src/ui/table.ts";
import { closeRender, render } from "./lib/render.mjs";

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
};

let ok = true;

const rows = [
  { id: "FTR-10", title: "Plan the sprint", size: 3, owner: "ana" },
  { id: "FTR-9", title: "Review the plan", size: null, owner: "bo" },
  { id: "DEF-2", title: "Fix the FTR plan link", size: 3, owner: null },
  { id: "FTR-100", title: "Ship it", size: 1, owner: "ana" },
  { id: "DEF-1", title: "Plain text", size: null, owner: "cy" },
];
const col = (key, label = key) => ({ key, label, value: (r) => r[key], sortable: true });
const ids = (rs) => rs.map((r) => r.id);

// R-3: stable sort, nulls last.
{
  const problems = [];
  eq(problems, "ids ascending: numeric runs (FTR-9 < FTR-10 < FTR-100)", ids(sortRows(rows, col("id"), "asc")), ["DEF-1", "DEF-2", "FTR-9", "FTR-10", "FTR-100"]);
  eq(problems, "ids descending", ids(sortRows(rows, col("id"), "desc")), ["FTR-100", "FTR-10", "FTR-9", "DEF-2", "DEF-1"]);
  eq(problems, "size ascending: equal sizes keep input order, nulls last", ids(sortRows(rows, col("size"), "asc")), ["FTR-100", "FTR-10", "DEF-2", "FTR-9", "DEF-1"]);
  eq(problems, "size descending: equal sizes keep input order, nulls still last", ids(sortRows(rows, col("size"), "desc")), ["FTR-10", "DEF-2", "FTR-100", "FTR-9", "DEF-1"]);
  eq(problems, "owner descending: a null owner is last", ids(sortRows(rows, col("owner"), "desc")), ["DEF-1", "FTR-9", "FTR-10", "FTR-100", "DEF-2"]);
  eq(problems, "numbers compare numerically (2 < 10)", sortRows([{ n: 10 }, { n: 2 }, { n: 1 }], col("n"), "asc").map((r) => r.n), [1, 2, 10]);
  const input = rows.slice();
  sortRows(input, col("id"), "asc");
  eq(problems, "the input is not reordered", ids(input), ids(rows));
  ok = report("web::check_table_sorts_stably_with_nulls_last", problems) && ok;
}

// R-3: every token must be found.
{
  const problems = [];
  const cols = [col("id"), col("title"), col("owner")];
  eq(problems, '"ftr plan" keeps rows holding both, case-insensitively', ids(filterRows(rows, cols, "ftr plan")), ["FTR-10", "FTR-9", "DEF-2"]);
  eq(problems, '"PLAN ana" across columns', ids(filterRows(rows, cols, "PLAN ana")), ["FTR-10"]);
  eq(problems, "empty text keeps every row", ids(filterRows(rows, cols, "   ")), ids(rows));
  eq(problems, "no match keeps none", ids(filterRows(rows, cols, "ftr nothing")), []);
  eq(problems, "a null value is not the text 'null'", ids(filterRows(rows, cols, "null")), []);
  ok = report("web::check_table_filter_matches_every_token", problems) && ok;
}

// R-3: CSV quoting and formula neutralising.
{
  const problems = [];
  const cols = [col("a", "Name, full"), col("b", "Note")];
  const csv = toCsv(
    [
      { a: 'say "hi"', b: "one,two" },
      { a: "line\nbreak", b: "=SUM(A1:A2)" },
      { a: "+1", b: "-2" },
      { a: "@cmd", b: "\tx" },
      { a: null, b: 42 },
    ],
    cols,
  );
  const want =
    '"Name, full",Note\r\n' +
    '"say ""hi""","one,two"\r\n' +
    '"line\nbreak",\'=SUM(A1:A2)\r\n' +
    "'+1,'-2\r\n" +
    "'@cmd,'\tx\r\n" +
    ",42\r\n";
  eq(problems, "the CSV", csv, want);
  eq(problems, "a carriage-return-leading cell is neutralised and quoted", toCsv([{ a: "\rx" }], [col("a")]), 'a\r\n"\'\rx"\r\n');
  if (!csv.split("\r\n").slice(0, -1).every((l) => !l.endsWith("\r"))) problems.push("a line does not end in CRLF");
  ok = report("web::check_table_csv_quotes_and_neutralises_formulas", problems) && ok;
}

// R-3: the rendered table.
try {
  const problems = [];
  const columns = [
    { ...col("id", "ID") },
    { ...col("title", "Title"), narrow: "hide" },
    { key: "size", label: "Size", value: (r) => r.size, align: "end", sortable: true },
    { key: "owner", label: "Owner", value: (r) => r.owner },
  ];
  const props = { rows, columns, rowKey: (r) => r.id, caption: "Items", empty: "No items", filterPlaceholder: "Filter items" };

  const sorted = await render("src/ui/DataTable.tsx", "DataTable", props, { url: "/?sort=id&dir=desc" });
  const doc = sorted.document;
  const caption = doc.querySelector("table > caption");
  if (!caption || caption.textContent !== "Items") problems.push(`no caption "Items": ${sorted.html}`);
  const ths = [...doc.querySelectorAll("thead th")];
  eq(problems, "aria-sort per header", ths.map((t) => t.getAttribute("aria-sort")), ["descending", "none", "none", null]);
  eq(problems, "sortable headers are buttons inside th", ths.map((t) => !!t.querySelector(":scope > button")), [true, true, true, false]);
  eq(problems, "rows follow ?sort=id&dir=desc", [...doc.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("td").textContent), ["FTR-100", "FTR-10", "FTR-9", "DEF-2", "DEF-1"]);
  const input = doc.querySelector('input[type="search"]');
  if (!input) problems.push("no filter input of type search");
  else {
    const label = doc.querySelector(`label[for="${input.id}"]`);
    if (!input.id || !label || !label.textContent.trim()) problems.push("the filter input has no visible label");
  }
  const count = doc.querySelector(".ui-table-count");
  if (!count || count.textContent !== "5 of 5") problems.push(`the row count is "${count?.textContent}", expected "5 of 5"`);
  if (![...doc.querySelectorAll("button")].some((b) => b.textContent === "Export CSV")) problems.push("no Export CSV button");

  // The filter and a stateKey prefix come from the URL.
  const filtered = await render("src/ui/DataTable.tsx", "DataTable", { ...props, stateKey: "t" }, { url: "/?tq=ftr+plan&q=nothing&tsort=size" });
  eq(problems, "?tq filters, ?tsort sorts", [...filtered.document.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("td").textContent), ["FTR-10", "DEF-2", "FTR-9"]);
  eq(problems, "the count after filtering", filtered.document.querySelector(".ui-table-count")?.textContent, "3 of 5");
  eq(problems, "the sorted header with a prefix", filtered.document.querySelector('th[aria-sort="ascending"]')?.textContent, "Size");

  // Nothing after filtering: the table shows `empty`, the filter stays.
  const none = await render("src/ui/DataTable.tsx", "DataTable", props, { url: "/?q=zzz" });
  if (!none.document.querySelector('input[type="search"]')) problems.push("with no rows after filtering the filter is gone");
  if (none.document.querySelector("tbody td")?.textContent !== "No items") problems.push(`with no rows after filtering, empty is not shown: ${none.html}`);
  // No rows at all: `empty` without the filter.
  const bare = await render("src/ui/DataTable.tsx", "DataTable", { ...props, rows: [] });
  if (bare.document.querySelector('input[type="search"]')) problems.push("with no rows at all the filter is shown");
  if (!bare.html.includes("No items")) problems.push("with no rows at all, empty is not shown");

  // A checkbox column: select-all over the filtered, enabled rows; a disabled row says why.
  const selectable = {
    selected: new Set(["FTR-10"]),
    onChange() {},
    disabled: (r) => (r.id === "FTR-9" ? "Already in a release" : null),
  };
  const sel = await render("src/ui/DataTable.tsx", "DataTable", { ...props, selectable }, { url: "/?q=ftr+plan" });
  const all = sel.document.querySelector("thead th input[type=checkbox]");
  if (!all || !all.getAttribute("aria-label")) problems.push("no labelled select-all checkbox");
  else if (all.hasAttribute("checked")) problems.push("select-all is checked while DEF-2 is unselected");
  const boxes = [...sel.document.querySelectorAll("tbody input[type=checkbox]")];
  eq(problems, "one checkbox per shown row", boxes.map((b) => b.getAttribute("aria-label")), ["Select FTR-10", "Select FTR-9", "Select DEF-2"]);
  eq(problems, "checked follows selected", boxes.map((b) => b.hasAttribute("checked")), [true, false, false]);
  const blocked = boxes[1];
  if (!blocked?.hasAttribute("disabled")) problems.push("a disabled row's checkbox has no disabled");
  else {
    const why = sel.document.getElementById(blocked.getAttribute("aria-describedby") ?? "");
    if (!why || why.textContent !== "Already in a release") problems.push("a disabled row's reason is not shown as text");
  }
  const allSel = await render(
    "src/ui/DataTable.tsx",
    "DataTable",
    { ...props, selectable: { ...selectable, selected: new Set(["FTR-10", "DEF-2"]) } },
    { url: "/?q=ftr+plan" },
  );
  if (!allSel.document.querySelector("thead th input[type=checkbox]")?.hasAttribute("checked")) {
    problems.push("select-all is not checked when every filtered, enabled row is selected");
  }

  // Over 500 rows: the first 200 and "Show all N".
  const many = Array.from({ length: 501 }, (_, i) => ({ id: `R-${i}`, title: "t", size: i, owner: "x" }));
  const big = await render("src/ui/DataTable.tsx", "DataTable", { ...props, rows: many });
  eq(problems, "501 rows draw 200", big.document.querySelectorAll("tbody tr").length, 200);
  if (![...big.document.querySelectorAll("button")].some((b) => b.textContent === "Show all 501")) problems.push('no "Show all 501" button');
  ok = report("web::check_table_markup_has_caption_and_sort_state", problems) && ok;
} catch (e) {
  ok = report("web::check_table_markup_has_caption_and_sort_state", [`render failed: ${e?.stack ?? e}`]) && ok;
} finally {
  await closeRender();
}

if (!ok) process.exit(1);
