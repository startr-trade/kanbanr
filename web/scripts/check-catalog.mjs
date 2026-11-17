#!/usr/bin/env node
//
// FTR-005: the Catalog page, checked without a browser.
//   web::catalog (R-10) — catalogView groups the rows by level (System, Components, Items), words
//     the gaps as one sentence, words an off row with its reason, links a written doc, a written
//     folder and a known generated view and leaves a repository doc and a katalogr view as text,
//     and gives each protection its in-force chip; the rendered page puts the gaps first, greys an
//     off row, names `kanbanr facts set` with no facts record and `kanbanr catalog init` with no
//     catalog; the page reads only …/catalog and …/facts, never the whole board, and takes no action.
//   web::catalog_sits_under_records_in_the_navigation (R-13) — nav.ts lists one `catalog` entry, in
//     the Records area, right after Docs, linking /p/{p}/catalog; main.tsx routes it; the page marks
//     that entry active.
//
// Usage: npm run check:catalog
import { readFileSync } from "node:fs";
import { NAV, activeEntry } from "../src/nav.ts";
import { chips, gapSentence, groupRows, hasCatalog, hasFacts, sourceHref, sourceText, stateChip } from "../src/catalogView.ts";
import { closeRender, render } from "./lib/render.mjs";

let failed = false;
function report(name, problems, ok) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ${ok}`);
  }
}
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);

/** A row as `GET …/catalog` sends it. */
const row = (id, level, source, extra = {}) => ({
  id,
  title: id,
  level,
  answers: ["what"],
  for: ["architect"],
  notation: "mermaid",
  source,
  state: "present",
  applies: { applies: true, why: "", by: "always" },
  protections: [],
  ...extra,
});

const status = {
  facts_rev: "abc",
  starter: 1,
  rows: [
    row("item-design", "item", { kind: "written", path: "" }, { state: "declared", notation: "prose" }),
    row("context", "system", { kind: "written", path: "architecture/context.md" }, {
      notation: "mermaid, prose",
      protections: [
        { name: "docs_updated", in_force: true, why: "the method's docs part is on" },
        { name: "drift", in_force: false, why: "no code repository" },
      ],
    }),
    row("component-docs", "component", { kind: "written", path: "architecture/components/" }, { state: { folder: { docs: 0 } } }),
    row("purpose", "system", { kind: "generated", generator: "kanbanr:charter" }, { state: "available" }),
    row("code-structure", "system", { kind: "generated", generator: "katalogr:structure" }, { state: "needs_katalogr" }),
    row("deployment", "system", { kind: "written", path: "architecture/deployment.md" }, {
      state: "missing",
      applies: { applies: false, why: "a library has no deployment of its own", by: "override" },
    }),
    row("observability", "system", { kind: "written", path: "repo:docs/system/observability.md" }, {
      state: "no_repository",
      applies: { applies: false, why: "the facts record does not say its production", by: "facts" },
    }),
    row("tests", "system", { kind: "enforced", test: "architecture::tests::front_ends_stay_thin" }, { state: "declared" }),
  ],
  gaps: [
    { question: "when", perspective: "operator" },
    { question: "who", perspective: "builder" },
    { question: "who", perspective: "user" },
  ],
  docs_read: 3,
};

// R-10: the pure model, then the page as rendered.
{
  const problems = [];
  const expect = (what, got, want) => {
    if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  };

  const groups = groupRows(status.rows);
  expect("groupRows orders System, Components, Items", groups.map((g) => g.title), ["System", "Components", "Items"]);
  expect(
    "groupRows keeps the catalog's order within a level",
    groups[0].rows.map((r) => r.id),
    ["context", "purpose", "code-structure", "deployment", "observability", "tests"],
  );
  expect("groupRows drops an empty level", groupRows([status.rows[1]]).map((g) => g.level), ["system"]);

  expect(
    "gapSentence words the gaps question by question",
    gapSentence(status.gaps),
    "Nobody answers: when for the operator; who for the builder and the user.",
  );
  expect("gapSentence is empty with no gap", gapSentence([]), "");

  const byId = Object.fromEntries(status.rows.map((r) => [r.id, r]));
  const labels = (id) => chips(byId[id]).map((c) => c.label);
  expect("an applying row's chips", labels("context"), [
    "applies",
    "present",
    "mermaid",
    "prose",
    "docs_updated: in force",
    "drift: not configured",
  ]);
  expect("an off row says why", labels("deployment")[0], "off: a library has no deployment of its own");
  expect("a row the facts rule out says why", labels("observability")[0], "does not apply: the facts record does not say its production");
  expect("an off row's chip is greyed, not coloured", chips(byId.deployment)[0].tone, "off");
  expect("a protection carries its why as the title", chips(byId.context).find((c) => c.label.startsWith("drift"))?.title, "no code repository");
  expect("an empty folder reads as missing", stateChip({ folder: { docs: 0 } }).tone, "warn");
  expect("a folder with docs counts them", stateChip({ folder: { docs: 2 } }).label, "2 docs");
  expect("a katalogr view says it needs katalogr", stateChip("needs_katalogr").label, "needs katalogr");

  expect("a written doc links its page", sourceHref("my shop", byId.context), "/p/my%20shop/docs/file/architecture/context.md");
  expect("a written folder links the Docs page at it", sourceHref("x", byId["component-docs"]), "/p/x/docs/folder/architecture/components");
  expect("a known generator links its page", sourceHref("x", byId.purpose), "/p/x/charter");
  expect("a katalogr view has no page", sourceHref("x", byId["code-structure"]), null);
  expect("a repository doc has no page yet", sourceHref("x", byId.observability), null);
  expect("an enforced source has no page", sourceHref("x", byId.tests), null);
  expect("an item-level written row has no page", sourceHref("x", byId["item-design"]), null);
  expect("a repository doc shows its path", sourceText(byId.observability), "written repo:docs/system/observability.md");

  expect("{} is no facts record", hasFacts({}), false);
  expect("a facts record is one", hasFacts({ schema: 1, languages: ["rust"], runtimes: [], production: [], parts: [], data: [] }), true);
  expect("no rows and no starter is no catalog", hasCatalog({ facts_rev: "", starter: 0, rows: [], gaps: [], docs_read: 0 }), false);
  expect("an invalid catalog is still a catalog", hasCatalog({ facts_rev: "", starter: 1, rows: [], gaps: [], problems: ["row x: bad"], docs_read: 0 }), true);

  // The page as rendered.
  const facts = { schema: 1, languages: ["rust"], runtimes: ["native"], production: ["developer-machine"], parts: ["cli"], data: ["files"] };
  const { document } = await render("src/pages/CatalogPage.tsx", "CatalogBody", { project: "shop", status, facts }, { url: "/p/shop/catalog" });
  const text = document.body.textContent ?? "";
  const h1 = [...document.querySelectorAll("h1")].map((h) => h.textContent?.trim());
  expect("the page has one h1, Catalog", h1, ["Catalog"]);
  const h2 = [...document.querySelectorAll("h2")].map((h) => h.textContent?.trim());
  expect("the gaps come first, then System, Components, Items", h2, ["Gaps", "System", "Components", "Items"]);
  if (!text.includes("Nobody answers: when for the operator")) problems.push("the page does not show the gap sentence");
  if (!text.includes("rust")) problems.push("the page does not show the facts record");
  if (!document.querySelector("#row-deployment.muted")) problems.push("the off row deployment is not greyed");
  if (document.querySelector("#row-context.muted")) problems.push("the applying row context is greyed");
  const links = [...document.querySelectorAll("a")].map((a) => a.getAttribute("href"));
  for (const want of ["/p/shop/docs/file/architecture/context.md", "/p/shop/docs/folder/architecture/components", "/p/shop/charter"]) {
    if (!links.includes(want)) problems.push(`the page does not link ${want} (links: ${links.join(", ")})`);
  }
  if (links.some((h) => h && h.includes("observability"))) problems.push("a repository doc is linked; it has no page yet");
  if (document.querySelectorAll("button, input, select, textarea, form").length) problems.push("the page offers an action; it is read-only");

  const bare = (await render("src/pages/CatalogPage.tsx", "CatalogBody", {
    project: "shop",
    status: { facts_rev: "", starter: 0, rows: [], gaps: [], docs_read: 0 },
    facts: {},
  })).document;
  const bareText = bare.body.textContent ?? "";
  if (!bareText.includes("kanbanr facts set")) problems.push("with no facts record the page does not name kanbanr facts set");
  if (!bareText.includes("kanbanr catalog init")) problems.push("with no catalog the page does not name kanbanr catalog init");
  if ([...bare.querySelectorAll("h2")].length) problems.push("with no catalog the page still draws sections");

  const broken = (await render("src/pages/CatalogPage.tsx", "CatalogBody", {
    project: "shop",
    status: { facts_rev: "", starter: 1, rows: [], gaps: [], problems: ["row ctx: answers \"huh\" is not a question"], docs_read: 0 },
    facts: facts,
  })).document;
  if (!(broken.body.textContent ?? "").includes("is not a question")) problems.push("an invalid catalog's problems are not shown");

  // The source: only the two reads, tagged so a change to the catalog, facts, docs or config
  // (protections and releases decide which rows apply) refreshes it.
  const page = read("pages/CatalogPage.tsx");
  const reads = [...page.matchAll(/apiUrl\(`([^`]*)`\)/g)].map((m) => m[1]);
  expect("the page reads only …/catalog and …/facts", reads, ["/projects/${enc}/catalog", "/projects/${enc}/facts"]);
  if (/\bapi\./.test(page)) problems.push("CatalogPage.tsx calls api.*; it reads through useQuery only and writes nothing");
  if (!/\["catalog", "facts", "docs", "config"\]/.test(page)) problems.push("the catalog read is not tagged catalog, facts, docs and config");
  const api = read("api.ts");
  if (!/getCatalog: \([^)]*\) =>\s*getJson<CatalogStatus>\(/.test(api)) problems.push("api.ts: getCatalog does not read a CatalogStatus");
  if (!/getFacts: \([^)]*\) =>\s*getJson<FactsOrEmpty>\(/.test(api)) problems.push("api.ts: getFacts does not read the facts record or {}");

  report(
    "web::catalog",
    problems,
    "rows grouped by level, gaps first as one sentence, an off row says why and is greyed, sources linked where they have a page, read-only.",
  );
}

// R-13: the Catalog entry sits in the Records area, after Docs.
{
  const problems = [];
  const holders = NAV.filter((a) => a.entries.some((e) => e.key === "catalog" || e.path("x") === "/p/x/catalog"));
  if (holders.length !== 1 || holders[0].key !== "records") {
    problems.push(`the Catalog entry is in ${holders.map((a) => a.key).join(", ") || "no area"}, wanted records only`);
  }
  const records = NAV.find((a) => a.key === "records")?.entries ?? [];
  const keys = records.map((e) => e.key);
  if (keys.filter((k) => k === "catalog").length !== 1) problems.push(`Records lists catalog ${keys.filter((k) => k === "catalog").length} times`);
  if (keys.indexOf("catalog") !== keys.indexOf("docs") + 1 || keys.indexOf("docs") < 0) {
    problems.push(`Records lists ${keys.join(", ")}; catalog must come right after docs`);
  }
  const e = records.find((x) => x.key === "catalog");
  if (e) {
    if (e.label !== "Catalog") problems.push(`the entry is labelled ${JSON.stringify(e.label)}, wanted "Catalog"`);
    if (e.path("a b") !== "/p/a%20b/catalog") problems.push(`the entry links ${e.path("a b")}, wanted /p/a%20b/catalog`);
    if (e.when) problems.push("the entry is shown only under a condition; the Catalog page is always there");
  }
  const at = activeEntry(NAV, "x", "/p/x/catalog");
  if (!same(at, { area: "records", entry: "catalog" })) problems.push(`/p/x/catalog marks ${JSON.stringify(at)} active`);
  const main = read("main.tsx");
  if (!/path: "p\/:project\/catalog", element: <CatalogPage \/>/.test(main)) problems.push("main.tsx does not route p/:project/catalog to CatalogPage");
  report("web::catalog_sits_under_records_in_the_navigation", problems, "Records › Catalog, after Docs, links /p/:project/catalog, routed and marked active.");
}

await closeRender();
process.exit(failed ? 1 : 0);
