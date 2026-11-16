#!/usr/bin/env node
//
// web::check_review_page_says_a_grant_cannot_be_checked_and_why (DEF-080 R-2): WHERE a
// delegation's grant cannot be checked, because the board's git history cannot be read, the
// Review page's delegations panel says "can't be checked" with the reason, and does not also say
// "not granted through kanbanr" for it.
//
// Static, like check-read-only.mjs: the page and the type are read as text.
//
// Usage: npm run check:grant-unverifiable
import { readFileSync } from "node:fs";

const NAME = "web::check_review_page_says_a_grant_cannot_be_checked_and_why";
const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");
const page = read("../src/pages/ReviewPage.tsx");
const types = read("../src/types.ts");
const problems = [];

if (!/\bgrant_unverifiable\?:\s*string\b/.test(types)) {
  problems.push("types.ts does not declare grant_unverifiable?: string on the delegation view");
}
// The branch that renders it: from `{g.grant_unverifiable ? (` to its `) : null}`.
const branch = page.match(/\{g\.grant_unverifiable \? \(([\s\S]*?)\) : null\}/);
if (!branch) {
  problems.push("ReviewPage.tsx does not render g.grant_unverifiable");
} else {
  const body = branch[1];
  if (!body.includes("can't be checked")) problems.push("ReviewPage.tsx: the line does not say \"can't be checked\"");
  if (!/\{g\.grant_unverifiable\}/.test(body)) problems.push("ReviewPage.tsx: the line does not show the reason");
  if (!body.includes("nothing is taken under it until it can")) {
    problems.push("ReviewPage.tsx: the line does not say nothing is taken under it until it can");
  }
  if (body.includes("not granted through kanbanr")) {
    problems.push("ReviewPage.tsx: the line also says \"not granted through kanbanr\"");
  }
}
// The "not granted" branch is still gated on its own field alone, which the API leaves false for
// an unverifiable grant.
const notGranted = page.match(/\{([^{}]*)\? \(\s*<>\s*\{" "\}\s*· <strong>not granted through kanbanr<\/strong>/);
if (!notGranted || notGranted[1].trim() !== "g.not_granted_through_kanbanr") {
  problems.push("ReviewPage.tsx: \"not granted through kanbanr\" is not shown on not_granted_through_kanbanr alone");
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — a grant that cannot be checked says so and why, and is not called not granted.`);
