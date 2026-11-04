#!/usr/bin/env node
//
// web::item_release_chip (FTR-001 R-1, R-2): an item says which release shipped it, or that it is
// planned and not released yet, or that it is in none; a project without releases shows nothing.
// The same function the page renders with is checked here, case by case.
//
// Usage: npm run check:release-line
import { releaseLine } from "../src/releaseLine.ts";

const releases = [
  { version: "v1.0.0", state: "shipped", shipped: ["FEAT-001"] },
  { version: "v1.0.1", state: "shipped", shipped: ["FEAT-001"] },
  { version: "v1.1.0", state: "planned" },
];

const problems = [];
const expect = (name, got, want) => {
  if (got !== want) problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

expect("shipped: the earliest release", releaseLine({ code: "FEAT-001", release: "v1.0.0" }, releases, true), "Released in v1.0.0");
expect("shipped, no planned release", releaseLine({ code: "FEAT-001" }, releases, true), "Released in v1.0.0");
expect("planned", releaseLine({ code: "FEAT-002", release: "v1.1.0" }, releases, true), "Planned for v1.1.0, not released yet");
expect("in none", releaseLine({ code: "FEAT-003" }, releases, true), "Not in a release");
expect("names no known release", releaseLine({ code: "FEAT-003", release: "v9" }, releases, true), "Not in a release");
expect("no releases on: no chip", releaseLine({ code: "FEAT-001", release: "v1.0.0" }, releases, false), null);

if (problems.length) {
  for (const p of problems) console.error(`web::item_release_chip — ${p}`);
  process.exit(1);
}
console.log("web::item_release_chip — shipped, planned and in-none read differently; none without releases.");
