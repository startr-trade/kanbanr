#!/usr/bin/env node
//
// web::check_item_page_shows_a_tests_last_duration (FTR-058 R-8): the item page shows a timed test's
// last duration as muted text after its name, `· 4 ms`, and nothing for an untimed one. The format
// is the CLI's `duration_text`: under 1 s `N ms`, under 60 s `S.ss s`, else `M min S s`. The page
// reads the durations through the shared cache with the tag `timings`. Static for the page, as the
// other check-*.mjs, and run for the formatter and the lookup.
//
// Usage: npm run check:test-durations
import { readFileSync } from "node:fs";
import { durationText, lastDuration } from "../src/types.ts";

const NAME = "web::check_item_page_shows_a_tests_last_duration";
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const problems = [];

// The format, at its edges.
const formats = [
  [0, "0 ms"], [4, "4 ms"], [999, "999 ms"], [1000, "1.00 s"], [2340, "2.34 s"], [59990, "59.99 s"],
  [60000, "1 min 0 s"], [125000, "2 min 5 s"],
];
for (const [ms, want] of formats) {
  const got = durationText(ms);
  if (got !== want) problems.push(`durationText(${ms}) is ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
}

// A fixture: a timed test shows its duration, an untimed one none, and the same test under another
// requirement is not mistaken for it.
const timings = [
  { code: "FTR-058", requirement: "R-1", test: "timed_one", ms: 4, at: "2026-11-17T10:00:00Z" },
  { code: "FTR-058", requirement: "R-2", test: "other", ms: 1500, at: "2026-11-17T10:00:00Z" },
];
const shown = (req, test) => {
  const ms = lastDuration(timings, req, test);
  return ms === undefined ? "" : ` · ${durationText(ms)}`;
};
if (shown("R-1", "timed_one") !== " · 4 ms") problems.push(`a timed test shows ${JSON.stringify(shown("R-1", "timed_one"))}, want " · 4 ms"`);
if (shown("R-1", "untimed") !== "") problems.push("an untimed test shows a duration");
if (shown("R-2", "timed_one") !== "") problems.push("a duration was taken from another requirement's test");
if (lastDuration(undefined, "R-1", "timed_one") !== undefined) problems.push("no timings read must show no duration");

// The page: reads `timings` through useQuery and shows the duration only when there is one, muted.
const page = read("pages/FeaturePage.tsx");
if (!/useQuery<[^>]*TestTiming[^>]*>\([^)]*urls\.timings\(project, [\w.]+\)[^)]*TAGS\.timings/s.test(page)) {
  problems.push("FeaturePage.tsx: no useQuery of urls.timings(project, <code>) with TAGS.timings");
}
if (!/ms === undefined \? null : \(?\s*<span className="muted">\s*· \{durationText\(ms\)\}/s.test(page)) {
  problems.push('FeaturePage.tsx: a timed test does not show `<span className="muted">· {durationText(ms)}</span>`, or an untimed one is not skipped');
}
const urlsSrc = read("data/urls.ts");
if (!/timings: \(p: string, code\?: string\)/.test(urlsSrc) || !/timings: \["timings"\]/.test(urlsSrc)) {
  problems.push("data/urls.ts: urls.timings or TAGS.timings is missing");
}
if (!/getTimings:/.test(read("api.ts"))) problems.push("api.ts: getTimings is missing");

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — a timed test shows its last duration as muted \`· 4 ms\`, an untimed one none.`);
