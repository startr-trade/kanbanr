#!/usr/bin/env node
//
// web::check_feature_page_redirects_an_earlier_code (FTR-056 R-3): an item page opened under a code
// the item has since left redirects to its current code, keeping the hash. `currentCode` is run on
// its three cases (exact, alias, unknown); the page is read statically, as the other check-*.mjs:
// it must render `Navigate replace` to the current code with `location.hash`.
//
// web::check_review_page_shows_a_recode_plan_to_confirm (FTR-056 R-7, R-11): a prepared `recode`
// record is the Review page's `recode` group (never a page-level panel): its evidence is a two-column
// from → to table with the count of renames in its heading, the group's action is Confirm, and the
// question asks before it renames.
//
// Usage: npm run check:aliases
import { readFileSync } from "node:fs";
import { currentCode } from "../src/aliases.ts";
import { askText } from "../src/reviewGroups.ts";
import { closeRender, render } from "./lib/render.mjs";

const NAME = "web::check_feature_page_redirects_an_earlier_code";
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const problems = [];

const items = [
  { code: "DEF-001", aliases: ["FEAT-001", "BUG-001"] },
  { code: "FTR-002" },
  // A broken board that gives one code to two items: the current code wins, as the server's find.
  { code: "FEAT-001", aliases: [] },
];
const cases = [
  ["DEF-001", "DEF-001"],
  ["FTR-002", "FTR-002"],
  ["BUG-001", "DEF-001"],
  ["FEAT-001", "FEAT-001"],
  ["FEAT-009", null],
];
for (const [code, want] of cases) {
  const got = currentCode(items, code);
  if (got !== want) problems.push(`currentCode(${code}) is ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
}
if (currentCode([{ code: "A-1", aliases: ["B-1"] }], "B-1") !== "A-1") problems.push("an alias does not resolve");
if (currentCode([], "A-1") !== null) problems.push("an empty index must answer null");

const page = read("pages/FeaturePage.tsx");
const nav = page.match(/<Navigate\s+replace\s+to=\{`([^`]*)`\}\s*\/>/);
if (!nav) {
  problems.push("FeaturePage.tsx: no `<Navigate replace to={…} />` for an earlier code");
} else {
  if (!/\$\{encodeURIComponent\(current\)\}/.test(nav[1])) problems.push("FeaturePage.tsx: the redirect does not go to the current code");
  if (!/\$\{location\.hash\}/.test(nav[1])) problems.push("FeaturePage.tsx: the redirect drops the hash");
}
if (!/currentCode\(index\.data\?\.items/.test(page)) problems.push("FeaturePage.tsx: the current code is not read from the index");

const types = read("types.ts");
for (const name of ["Feature", "IndexItem"]) {
  const t = types.match(new RegExp(`export interface ${name} \\{[\\s\\S]*?\\n\\}`));
  if (!t || !/\baliases\?: string\[\];/.test(t[0])) problems.push(`types.ts: ${name} has no \`aliases?: string[]\``);
}

// ---- the recode group (R-7, R-11) ----
const PLAN = [
  "FEAT-150 \u2192 DEF-095  bug  The board drops a field",
  "FEAT-151 \u2192 MNT-012  chore  Tidy the loader",
  "FEAT-152 \u2192 FTR-077  feature  A new card",
  "3 items; plan abc1234",
  "",
].join("\n");
const NAME2 = "web::check_review_page_shows_a_recode_plan_to_confirm";
const problems2 = [];
try {
  const ev = await render("src/pages/review/RecodeEvidence.tsx", "RecodeEvidence", { plan: PLAN });
  const doc = ev.document;
  const heading = doc.querySelector("h4")?.textContent ?? "";
  if (!heading.includes("3 items")) problems2.push(`the heading does not carry the count: ${JSON.stringify(heading)}`);
  const heads = [...doc.querySelectorAll("thead th")].map((th) => th.textContent.trim());
  if (JSON.stringify(heads) !== JSON.stringify(["From", "To"])) problems2.push(`the table's columns are ${JSON.stringify(heads)}, want From and To`);
  const cells = [...doc.querySelectorAll("tbody tr")].map((tr) => [...tr.querySelectorAll("td")].map((td) => td.textContent.trim()));
  if (JSON.stringify(cells[0]) !== JSON.stringify(["FEAT-150", "DEF-095"]) || cells.length !== 3) problems2.push(`the rows are ${JSON.stringify(cells)}`);
  if (!doc.querySelector(".ui-table-wrap[style*='overflow']")) problems2.push("the table does not scroll");

  const row = {
    key: "P-1", code: "recode", title: "Recode 3 items", what: "3 items to their kinds' prefixes, prepared 2026-11-17",
    records: "Renames 3 items", release: "", sprint: "", changed_at: "", blocked: "", pills: ["prepared"], prepared: "P-1",
    batch: "", rev: "abc1234", spec_rev: "", signoff: "", requirement: "", test: "",
  };
  const slice = { key: "recode", label: "Recode", action: "Confirm", rows: [row], choices: [] };
  const panel = await render(
    "src/pages/review/GroupView.tsx",
    "GroupPanel",
    { project: "x", slice, writable: true, who: "Ada <a@example.invalid>", tick: 0, body: (r) => `evidence of ${r.key}` },
    { url: "/p/x/review?group=recode" },
  );
  const head = panel.document.querySelector(".review-group-head");
  if (!/renames every item in the plan/.test(head?.textContent ?? "")) problems2.push("the recode group has no sentence of its own");
  const buttons = [...(head?.querySelectorAll("button") ?? [])].map((b) => b.textContent.trim());
  if (!buttons.some((b) => /^Confirm( all)? 1$/.test(b))) problems2.push(`the group's action is not Confirm: ${JSON.stringify(buttons)}`);
  const ask = askText(slice, { group: "recode", keys: new Set(["P-1"]) }, slice.rows);
  if (ask !== "Confirm recode 3 items to their kinds' prefixes?") problems2.push(`the question reads ${JSON.stringify(ask)}`);

  const page = read("pages/ReviewPage.tsx");
  if (!/e\.prepared\?\.kind === "recode"[\s\S]{0,80}<RecodeEvidence/.test(page)) problems2.push("ReviewPage.tsx does not draw a recode record's plan");
  if ((page.match(/<RecodeEvidence/g) ?? []).length !== 1) problems2.push("RecodeEvidence is mounted other than once, as a row's evidence");
} catch (e) {
  problems2.push(`threw: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}
problems.push(...problems2.map((p) => `${NAME2} — ${p}`));

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — an earlier code resolves to the item's current code and the page redirects there, hash kept.`);
console.log(`${NAME2} — the recode group draws its plan as a from → to table with the count, and Confirm asks first.`);
