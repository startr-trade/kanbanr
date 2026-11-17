#!/usr/bin/env node
//
// web::check_feature_page_redirects_an_earlier_code (FTR-056 R-3): an item page opened under a code
// the item has since left redirects to its current code, keeping the hash. `currentCode` is run on
// its three cases (exact, alias, unknown); the page is read statically, as the other check-*.mjs:
// it must render `Navigate replace` to the current code with `location.hash`.
//
// Usage: npm run check:aliases
import { readFileSync } from "node:fs";
import { currentCode } from "../src/aliases.ts";

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

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — an earlier code resolves to the item's current code and the page redirects there, hash kept.`);
