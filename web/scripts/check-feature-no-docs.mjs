#!/usr/bin/env node
//
// web::check_feature_page_shows_why_an_item_changes_no_docs (DEF-075 R-6): the item page shows the
// definition's `affects_no_docs` as a plain line, `Changes no docs: <why>`, directly after the
// Affects docs list, in body text and never as a chip, and renders nothing when the field is empty.
// Static, as the other check-*.mjs: it reads FeaturePage.tsx and types.ts.
//
// Usage: npm run check:feature-no-docs
import { readFileSync } from "node:fs";

const NAME = "web::check_feature_page_shows_why_an_item_changes_no_docs";
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const page = read("pages/FeaturePage.tsx");
const types = read("types.ts");

const problems = [];

// The definition type carries the field, optional, as the API sends it only when set.
const defType = types.match(/export interface FeatureDefinition \{[\s\S]*?\n\}/);
if (!defType || !/\baffects_no_docs\?: string;/.test(defType[0])) {
  problems.push("types.ts: FeatureDefinition has no `affects_no_docs?: string`");
}

// The Affects docs block: from its condition to the end of its `) : null}`.
const docsStart = page.indexOf("{(def.affects_docs ?? []).length ? (");
const docsEnd = docsStart < 0 ? -1 : page.indexOf(") : null}", docsStart);
if (docsStart < 0 || docsEnd < 0) {
  problems.push("FeaturePage.tsx: the Affects docs list was not found");
} else {
  // The next JSX expression after the Affects docs block, comments skipped, is the no-docs line.
  let rest = page.slice(docsEnd + ") : null}".length);
  rest = rest.replace(/^(\s|\{\/\*[\s\S]*?\*\/\})*/, "");
  const block = rest.match(/^\{def\.affects_no_docs\?\.trim\(\) \? \(([\s\S]*?)\) : null\}/);
  if (!block) {
    problems.push(
      "FeaturePage.tsx: directly after the Affects docs list there is no " +
        "`{def.affects_no_docs?.trim() ? (…) : null}` (nothing renders when the field is empty)",
    );
  } else {
    const jsx = block[1];
    if (!/Changes no docs:/.test(jsx)) problems.push("FeaturePage.tsx: the line does not say `Changes no docs:`");
    if (!/\{def\.affects_no_docs(\.trim\(\))?\}/.test(jsx)) {
      problems.push("FeaturePage.tsx: the line does not show the definition's affects_no_docs");
    }
    if (/className="[^"]*\bchip\b/.test(jsx) || /className=\{[^}]*chip/.test(jsx)) {
      problems.push("FeaturePage.tsx: the no-docs line has a `chip` class; it is a plain body-text line");
    }
  }
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — 'Changes no docs: <why>' is a plain line under Affects docs, and absent when empty.`);
