#!/usr/bin/env node
//
// web::check_drift_age_label (FTR-045 R-8): the monitor labels the latest background drift result
// with the code revision and age it was computed at, says recomputing while a newer run is going,
// and says drift is unavailable when the monitor has no code repository. `driftLabel` is checked
// for each state and `ago` at its boundaries.
//
// Usage: npm run check:drift-age
import { ago, driftLabel, driftTitle, UNAVAILABLE } from "../src/driftAge.ts";

const T = "web::check_drift_age_label";
const problems = [];
const expect = (name, got, want) => {
  if (got !== want) problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

const now = new Date("2026-11-17T12:00:00Z");
const before = (secs) => new Date(now.getTime() - secs * 1000).toISOString();

// ago's boundaries: 44 s, 45 s, 59 min, 60 min and 24 h.
expect("ago 0 s", ago(before(0), now), "just now");
expect("ago 44 s", ago(before(44), now), "just now");
expect("ago 45 s", ago(before(45), now), "1 min ago");
expect("ago 4 min", ago(before(4 * 60), now), "4 min ago");
expect("ago 59 min", ago(before(59 * 60), now), "59 min ago");
expect("ago 59 min 59 s", ago(before(59 * 60 + 59), now), "59 min ago");
expect("ago 60 min", ago(before(60 * 60), now), "1 h ago");
expect("ago 23 h", ago(before(23 * 3600), now), "23 h ago");
expect("ago 24 h", ago(before(24 * 3600), now), "1 d ago");
expect("ago 3 d", ago(before(3 * 86400), now), "3 d ago");
expect("ago in the future", ago(before(-30), now), "just now");

const asOf = {
  board_rev: "b0a4d1e9c2f3a4b5c6d7e8f90123456789abcdef",
  code_rev: "3f2a1c9e8d7c6b5a49382716051423324150607f",
  finished_at: before(4 * 60),
  took_ms: 48700,
};

// Each state.
expect("fresh", driftLabel({ state: "fresh", as_of: asOf, count: 3 }, now), "as of 3f2a1c9, 4 min ago");
expect("stale, queued", driftLabel({ state: "stale", as_of: asOf, count: 3 }, now), "as of 3f2a1c9, 4 min ago");
expect(
  "stale, recomputing",
  driftLabel({ state: "stale", as_of: asOf, recomputing: true, count: 3 }, now),
  "as of 3f2a1c9, 4 min ago · recomputing…",
);
expect("computing", driftLabel({ state: "computing", count: 0 }, now), "computing…");
expect("computing while running", driftLabel({ state: "computing", recomputing: true, count: 0 }, now), "computing…");
expect(
  "unavailable",
  driftLabel({ state: "unavailable", count: 0, reason: "needs the code repository" }, now),
  "Drift unavailable: this monitor has no code repository",
);
expect("unavailable constant", UNAVAILABLE, "Drift unavailable: this monitor has no code repository");
expect(
  "failed",
  driftLabel({ state: "failed", count: 0, reason: "git broke", failed_at: before(2 * 60) }, now),
  "failed 2 min ago: git broke",
);
// The board revision is on hover.
expect("title", driftTitle({ state: "fresh", as_of: asOf, count: 3 }), "board b0a4d1e");
expect("title before a result", driftTitle({ state: "computing", count: 0 }), "");

if (problems.length) {
  for (const p of problems) console.error(`${T} — ${p}`);
  process.exit(1);
}
console.log(`${T} — every state is labelled with its revision and age, recomputing and unavailable included.`);
