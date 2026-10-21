#!/usr/bin/env node
//
// The Rust diagram lint, the mermaid-lint crate, follows Pele (FEAT-200/R-10): it flags only
// diagrams that Pele's `parse` rejects, and never one Pele accepts. The fixture both sides read
// holds each case with the rule the lint must report, or null. This checks the Pele side: `parse`
// must throw exactly for the cases that name a rule. The Rust side is
// `mermaid_lint::tests::lint_cases_fixture`.
//
// Usage: npm run check:diagram-lint
import fs from "node:fs";
import { parse } from "pele";

const fixture = new URL("../../api/crates/mermaid-lint/src/testdata/diagram_lint_cases.json", import.meta.url);
const { cases } = JSON.parse(fs.readFileSync(fixture, "utf8"));
// Zero cases is a broken path or an emptied fixture, not a pass.
if (!Array.isArray(cases) || cases.length === 0) {
  console.error("check:diagram-lint — the fixture holds no cases; that is a broken fixture, not a pass");
  process.exit(1);
}

const problems = [];
for (const c of cases) {
  let error = null;
  try {
    parse(c.source);
  } catch (e) {
    error = e;
  }
  if (c.rule === null && error) {
    problems.push(`"${c.name}": the lint passes it, but Pele rejects it (${String(error.message).split("\n")[0]})`);
  } else if (c.rule !== null && !error) {
    problems.push(`"${c.name}": the lint reports ${c.rule}, but Pele parses it`);
  }
}

for (const p of problems) console.error(`FAIL ${p}`);
console.log(`${cases.length} fixture case(s) checked against Pele, ${problems.length} disagreement(s)`);
process.exit(problems.length ? 1 : 0);
