#!/usr/bin/env node
//
// web::check_releases_page_lists_prepared_records_no_form_shows (DEF-095 R-2, R-5): an open
// prepared record for a release that no form on its entry shows (a gate result for a gate no stage
// asks for, a promotion for a stage that is not the plan's next) is listed in that release's entry
// under "Prepared, not required", below its button and above its notes, with PreparedPanel's
// Confirm and Reject. The pure filter is checked case by case; the page and the types are read for
// where they render it and what they declare.
//
// Usage: npm run check:releases-prepared
import { readFileSync } from "node:fs";
import { unshownFor } from "../src/prepared.ts";

const NAME = "web::check_releases_page_lists_prepared_records_no_form_shows";
const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};
const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

const view = (id, kind, state, subject, extra = {}) => ({
  id,
  kind,
  subject,
  fields: kind === "gate-result" ? { passed: true } : {},
  sources: [],
  by: "Claude",
  at: `2026-11-16T10:0${id.length}:00Z`,
  state,
  resolution: null,
  stale_why: "",
  page: "releases",
  command: "",
  ...extra,
});
const sha = "1b4d4b2aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const views = [
  // A gate result no stage asks for, on v0.7.0's tip: listed.
  view("local", "gate-result", "open", { gate: "local", commit: sha }, { release: "v0.7.0" }),
  // The same gate on a commit a form shows: not listed twice.
  view("ci", "gate-result", "open", { gate: "ci", commit: sha }, { release: "v0.7.0", required_by: ["production"] }),
  // A stale one (its release was cut after it): not listed.
  view("old", "gate-result", "stale", { gate: "local", commit: sha }, { release: "v0.7.0", stale_why: "v0.7.0 was cut" }),
  // Another release's: not listed here.
  view("other", "gate-result", "open", { gate: "local", commit: sha }, { release: "v0.8.0" }),
  // A promotion for a stage that is not the plan's next: listed, by its own subject's release.
  view("promo", "promotion", "open", { release: "v0.7.0", stage: "staging" }),
  // An answered promotion: not listed.
  view("done", "promotion", "confirmed", { release: "v0.7.0", stage: "main" }),
  // No release at all: not listed.
  view("none", "gate-result", "open", { gate: "local", commit: "f".repeat(40) }),
];

expect(
  "unshownFor lists the open records for the release that no form shows",
  unshownFor(views, "v0.7.0", new Set(["ci"])).map((v) => v.id),
  ["local", "promo"],
);
expect(
  "unshownFor lists a promotion at a stage other than the plan's next",
  unshownFor(views, "v0.7.0", new Set(["ci", "local"])).map((v) => v.id),
  ["promo"],
);
expect("unshownFor keeps to the release", unshownFor(views, "v0.8.0", new Set()).map((v) => v.id), ["other"]);
expect("unshownFor on nothing", unshownFor(undefined, "v0.7.0", new Set()), []);

// The page: the section is inside each release's entry, below its button and above its notes.
const page = read("../src/pages/ReleasesPage.tsx");
// Each release's entry is its accordion body, ReleaseBody, beside PreparedNotRequired in
// releases/ReleaseBody.tsx (FTR-041 T4, T5).
const body = read("../src/pages/releases/ReleaseBody.tsx");
if (!body.includes(">Prepared, not required<")) {
  problems.push('releases/ReleaseBody.tsx: no section titled exactly "Prepared, not required"');
}
const tile = body.slice(body.indexOf("export function ReleaseBody("), body.indexOf("export function PreparedNotRequired("));
const button = tile.indexOf("<ReleaseButton");
const panel = tile.indexOf("<PreparedNotRequired");
const notes = tile.indexOf("<ReleaseNotesSection");
if (button < 0 || panel < 0 || notes < 0 || !(button < panel && panel < notes)) {
  problems.push("releases/ReleaseBody.tsx: PreparedNotRequired is not rendered in the release's body between its button and its notes");
}
if (page.includes(">Prepared, not required<") || page.includes("<PreparedNotRequired")) {
  problems.push("ReleasesPage.tsx: the section is rendered at the page top, not inside a release's entry");
}
const section = body.slice(body.indexOf("function PreparedNotRequired("));
const sectionEnd = section.indexOf("\nfunction ", 1);
const block = sectionEnd > 0 ? section.slice(0, sectionEnd) : section;
if (!body.includes("function PreparedNotRequired(")) {
  problems.push("releases/ReleaseBody.tsx: no PreparedNotRequired component");
} else {
  for (const want of ["<PreparedPanel", "api.confirmPrepared(", "api.rejectPrepared(", "no stage asks for", "Prepared, not required"]) {
    if (!block.includes(want)) problems.push(`releases/ReleaseBody.tsx: PreparedNotRequired does not use ${want}`);
  }
  if (!/Record \$\{[^}]+\} \$\{[^}]+\? "passed" : "failed"\} on \$\{/.test(block)) {
    problems.push("releases/ReleaseBody.tsx: a gate result's label is not `Record {gate} {passed|failed} on {sha7}`");
  }
}
if (!/unshownFor\(/.test(tile)) problems.push("releases/ReleaseBody.tsx: the release's body does not filter with unshownFor");
if (!/onShown/.test(tile)) problems.push("releases/ReleaseBody.tsx: the forms do not report the records they show (onShown)");

// The types: the two derived fields the server sends.
const types = read("../src/types.ts");
const pv = types.slice(types.indexOf("export interface PreparedView"));
const pvBlock = pv.slice(0, pv.indexOf("\n}"));
for (const f of ["release?: string;", "required_by?: string[];"]) {
  if (!pvBlock.includes(f)) problems.push(`types.ts: PreparedView does not declare ${f}`);
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`✓ ${NAME} — an open prepared record no form shows is listed in its release's entry under Prepared, not required.`);
