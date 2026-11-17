#!/usr/bin/env node
//
// FTR-048: the Items page, checked without a browser.
//   web::check_items_page_reads_its_filters_from_the_url (R-9) — the pure itemsQuery(search) turns
//     the page's query string into the /items API query (fixed key order, other parameters and a
//     bogus evidence value dropped, "" for none) and reading its own output gives it back; the page
//     reads urls.items with exactly that query; rendered at ?kind=defect&evidence=red its selects
//     show those values and its rows link each item; with no rows it says "No items match these
//     filters."; nav.ts lists Items under Work at /p/:project/items and main.tsx routes it.
//
// Usage: npm run check:items-page
import { readFileSync } from "node:fs";
import { EVIDENCE, itemsCsvName, itemsFilter, itemsQuery } from "../src/itemsQuery.ts";
import { NAV, activeEntry } from "../src/nav.ts";
import { urls } from "../src/data/urls.ts";
import { closeRender, render } from "./lib/render.mjs";

const NAME = "web::check_items_page_reads_its_filters_from_the_url";
const problems = [];
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);
const expect = (what, got, want) => {
  if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();

// The pure query, and its round trip.
expect("no filter is no query", itemsQuery(""), "");
expect("a page with only table state is no query", itemsQuery("?sort=code&dir=desc&q=x"), "");
expect("two filters keep the API's order", itemsQuery("?evidence=red&kind=defect"), "?kind=defect&evidence=red");
expect("table state is dropped", itemsQuery("?kind=defect&sort=code&evidence=red"), "?kind=defect&evidence=red");
expect("an empty value is unset", itemsQuery("?kind=&status=Done"), "?status=Done");
expect("a bogus evidence value is dropped", itemsQuery("?evidence=bogus&goal=G-1"), "?goal=G-1");
expect("a value is encoded", itemsQuery("?status=In%20Progress"), "?status=In%20Progress");
expect("untyped is kind=none", itemsQuery("?kind=none"), "?kind=none");
for (const e of EVIDENCE) expect(`evidence ${e} is kept`, itemsQuery(`?evidence=${e}`), `?evidence=${e}`);
const all = "?release=v0.8.0&sprint=SP-004&goal=G-2&milestone=MS-004&status=Done&kind=feature&evidence=proven";
const q = itemsQuery(all);
expect("all seven, in order", q, "?kind=feature&status=Done&milestone=MS-004&goal=G-2&sprint=SP-004&release=v0.8.0&evidence=proven");
for (const s of ["", "?kind=defect&evidence=red", all, "?status=In%20Progress&x=1"]) {
  expect(`itemsQuery round-trips ${s}`, itemsQuery(itemsQuery(s)), itemsQuery(s));
  expect(`itemsFilter round-trips ${s}`, itemsFilter(itemsQuery(s)), itemsFilter(s));
  const api = urls.items("x", itemsFilter(s));
  if (!api.endsWith(`/projects/x/items${itemsQuery(s)}`)) problems.push(`urls.items for ${s} is ${api}, wanted …/projects/x/items${itemsQuery(s)}`);
}
expect("the CSV is named for its filters", itemsCsvName("?kind=defect&evidence=red"), "items-defect-red");
expect("the CSV with no filter is items", itemsCsvName(""), "items");

// The page reads /items through the shared cache with the URL's filters.
const page = read("pages/ItemsPage.tsx");
if (!/urls\.items\(\s*project\s*,\s*itemsFilter\(/.test(page)) problems.push("ItemsPage.tsx does not read urls.items(project, itemsFilter(…))");
if (!/TAGS\.items\b/.test(page)) problems.push("ItemsPage.tsx does not depend on TAGS.items");
if (!/useUrlState\(\s*ITEM_FILTERS\b/.test(page)) problems.push("ItemsPage.tsx does not keep its filters with useUrlState(ITEM_FILTERS)");

// Rendered at a filtered URL.
const index = {
  project: "shop",
  config: { name: "Shop", description: "", statuses: ["Planned", "In Progress", "Done"], cadence: { sprints: true, releases: true, git_model: false }, codes: {} },
  items: [
    { code: "DEF-1", title: "Cart drops a line", kind: "defect", status: "In Progress", milestone: "MS-001", done: 0, total: 0, created_at: "2026-11-01T00:00:00Z" },
    { code: "FTR-2", title: "Checkout", kind: "feature", status: "Done", milestone: "MS-001", done: 0, total: 0, created_at: "2026-11-01T00:00:00Z" },
    { code: "X-3", title: "Untyped", status: "Planned", milestone: "MS-002", done: 0, total: 0, created_at: "2026-11-01T00:00:00Z" },
  ],
  milestones: [{ code: "MS-001", name: "One" }, { code: "MS-002", name: "Two" }],
  sprints: [{ code: "SP-001", name: "S1", start: "2026-11-01", end: "2026-11-14", state: "active" }],
  releases: [{ version: "v0.1.0", state: "planned" }],
  docs: [],
  goals: [{ id: "G-1" }],
};
const rows = [
  { code: "DEF-1", title: "Cart drops a line", kind: "defect", status: "In Progress", milestone: "MS-001", sprint: "SP-001", req_total: 3, req_proven: 1, tests_red: 2, tests_planned: 0, stale: true },
];
const url = "/p/shop/items?kind=defect&evidence=red";
const ok = { data: rows, loading: false, error: null };
const { document } = await render("src/pages/ItemsPage.tsx", "ItemsBody", { project: "shop", index, items: ok }, { url });
const h1 = [...document.querySelectorAll("h1")].map(text);
expect("the page has one h1, Items", h1, ["Items"]);
const selected = (name) => {
  const sel = document.querySelector(`select[name="${name}"]`);
  if (!sel) return null;
  const opt = [...sel.querySelectorAll("option")].find((o) => o.hasAttribute("selected"));
  return opt ? opt.getAttribute("value") : "";
};
expect("the kind select shows the URL's kind", selected("kind"), "defect");
expect("the evidence select shows the URL's evidence", selected("evidence"), "red");
expect("the status select shows no filter", selected("status"), "");
for (const name of ["kind", "status", "milestone", "goal", "sprint", "release", "evidence"]) {
  const sel = document.querySelector(`select[name="${name}"]`);
  if (!sel) problems.push(`the filter bar has no ${name} select`);
  else if (!sel.id || !document.querySelector(`label[for="${sel.id}"]`)) problems.push(`the ${name} select has no label`);
}
const kinds = [...(document.querySelector('select[name="kind"]')?.querySelectorAll("option") ?? [])].map((o) => o.getAttribute("value"));
expect("kinds come from the index, untyped as none", kinds, ["", "defect", "feature", "none"]);
const evidence = [...(document.querySelector('select[name="evidence"]')?.querySelectorAll("option") ?? [])].map((o) => o.getAttribute("value"));
expect("evidence offers every value", evidence, ["", ...EVIDENCE]);
const statuses = [...(document.querySelector('select[name="status"]')?.querySelectorAll("option") ?? [])].map((o) => o.getAttribute("value"));
expect("statuses come from the configuration", statuses, ["", "Planned", "In Progress", "Done"]);
const links = [...document.querySelectorAll("a")].map((a) => a.getAttribute("href"));
if (!links.includes("/p/shop/feature/DEF-1")) problems.push(`the row does not link its item (links: ${links.join(", ")})`);
const cells = [...document.querySelectorAll("tbody tr td")].map(text);
expect("the row's cells", cells, ["DEF-1", "Cart drops a line", "defect", "In Progress", "MS-001", "SP-001", "", "1 of 3", "2", "stale"]);

// A URL value the index does not list is still shown, so the select never hides a filter in force.
{
  const { document: d } = await render("src/pages/ItemsPage.tsx", "ItemsBody", { project: "shop", index, items: ok }, { url: "/p/shop/items?milestone=MS-009" });
  const opt = [...(d.querySelector('select[name="milestone"]')?.querySelectorAll("option") ?? [])].find((o) => o.hasAttribute("selected"));
  expect("an unlisted milestone in the URL is shown selected", opt?.getAttribute("value") ?? null, "MS-009");
}

// No rows: the empty state; loading: never the empty state.
{
  const { document: d } = await render("src/pages/ItemsPage.tsx", "ItemsBody", { project: "shop", index, items: { data: [], loading: false, error: null } }, { url });
  if (!text(d.body).includes("No items match these filters.")) problems.push("with no rows the page does not say \"No items match these filters.\"");
  const { document: l } = await render("src/pages/ItemsPage.tsx", "ItemsBody", { project: "shop", index, items: { data: null, loading: true, error: null } }, { url });
  if (text(l.body).includes("No items match")) problems.push("while loading the page says no items match");
  if (!l.querySelector('[role="status"]')) problems.push("while loading the page shows no loading state");
  if (!l.querySelector('select[name="kind"]')) problems.push("while loading the filter bar is not drawn");
}

// Navigation and the route.
{
  const work = NAV.find((a) => a.key === "work");
  const e = work?.entries.find((x) => x.key === "items");
  if (!e) problems.push("the Work area has no Items entry");
  else {
    expect("the Items entry is labelled Items", e.label, "Items");
    expect("the Items entry's path", e.path("x"), "/p/x/items");
  }
  expect("an Items URL marks the Items entry", activeEntry(NAV, "x", "/p/x/items"), { area: "work", entry: "items" });
  const main = read("main.tsx");
  if (!/path:\s*"p\/:project\/items",\s*element:\s*<ItemsPage\s*\/>/.test(main)) problems.push("main.tsx does not route p/:project/items to ItemsPage");
}

await closeRender();
if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — the URL's filters become the /items query and the selects, rows link their items, and Items sits under Work.`);
