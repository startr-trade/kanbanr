#!/usr/bin/env node
//
// The review page keeps Approve disabled until every diagram parses (FEAT-200/R-14), and a diagram
// that does not draw shows its error with its line (R-15). Runs the page's pure check
// (src/diagramCheck.ts) with the SAME Pele the monitor parses with:
//   - locateError: Pele's 1-based block line becomes a file line; "no type" lands on the first line;
//   - checkBlocks: every broken block is listed once, with file, doc, block and file line; a core
//     lint finding on a block Pele accepted is listed too (fail closed); a clean item lists nothing;
//   - approveBlocker: disabled while loading, when the check failed, and while any problem is left.
//
// Usage: npm run check:diagram-review
import { parse } from "pele";
import { approveBlocker, checkBlocks, locateError } from "../src/diagramCheck.ts";

const problems = [];
const expect = (ok, what) => {
  if (!ok) problems.push(what);
};
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// ---- locateError, on real Pele errors ----
const thrown = (source) => {
  try {
    parse(source);
  } catch (e) {
    return e;
  }
  throw new Error(`Pele parsed what this check expects it to reject:\n${source}`);
};
const semicolon = locateError(thrown("sequenceDiagram\n  A->>B: one; two\n"), 14);
expect(semicolon.line === 15, `a Pele error on block line 2 of a block at file line 14 is file line 15, got ${semicolon.line}`);
expect(semicolon.message.length > 0 && !semicolon.message.includes("\n"), `the message is one line: ${JSON.stringify(semicolon.message)}`);
const untyped = locateError(thrown("flowchrt LR\n  A --> B\n"), 7);
expect(same(untyped, { line: 7, message: "no diagram type on the first line" }), `no type: ${JSON.stringify(untyped)}`);
expect(locateError("plain text", 3).line === 3, "a thrown string lands on the first line");
expect(locateError({ line: 0, message: "x\ny" }, 5).message === "x", "only the first line of a message is kept");

// ---- checkBlocks ----
const item = {
  code: "FEAT-012",
  blocks: [
    { file: "FEAT-012 spec", doc: null, block: 1, line: 4, source: "flowchart LR\n  A --> B\n" },
    { file: "FEAT-012 spec", doc: null, block: 2, line: 10, source: "sequenceDiagram\n  A->>B: one; two\n" },
    { file: "docs/arch/flow.md", doc: "arch/flow.md", block: 1, line: 3, source: "flowchart LR\n  A[f(x)] --> B\n" },
    { file: "docs/arch/flow.md", doc: "arch/flow.md", block: 2, line: 9, source: "pie\n  \"a\" : 1\n" },
  ],
  findings: [
    // On a block Pele also rejects: listed once, as Pele's error.
    { file: "FEAT-012 spec", block: 2, line: 11, rule: "semicolon_in_text", message: "`;` ends the statement early" },
    // On a block Pele accepted (lint and Pele disagree): listed anyway — fail closed.
    { file: "docs/arch/flow.md", block: 2, line: 10, rule: "unknown_statement", message: "not a statement" },
  ],
};
const found = checkBlocks(item, parse);
expect(found.length === 3, `three problems, one per broken block: ${JSON.stringify(found)}`);
const spec = found.find((p) => p.file === "FEAT-012 spec");
expect(spec && spec.block === 2 && spec.line === 11 && spec.doc === null, `the spec's block 2 at file line 11: ${JSON.stringify(spec)}`);
const doc = found.find((p) => p.file === "docs/arch/flow.md" && p.block === 1);
expect(doc && doc.doc === "arch/flow.md" && doc.line === 4, `the doc's block 1 at file line 4, with its key: ${JSON.stringify(doc)}`);
const lintOnly = found.find((p) => p.file === "docs/arch/flow.md" && p.block === 2);
expect(lintOnly && lintOnly.line === 10 && lintOnly.message.endsWith("(unknown_statement)") && lintOnly.doc === "arch/flow.md",
  `a lint finding Pele did not raise is still listed: ${JSON.stringify(lintOnly)}`);
expect(checkBlocks({ code: "FEAT-1", blocks: [item.blocks[0]], findings: [] }, parse).length === 0, "a clean item lists nothing");
expect(checkBlocks({ code: "FEAT-1", blocks: [], findings: [] }, parse).length === 0, "an item with no diagrams lists nothing");

// ---- approveBlocker ----
expect(approveBlocker({ loading: true, failed: null, problems: [] }) !== null, "disabled while loading");
expect(approveBlocker({ loading: false, failed: "500 Internal Server Error", problems: [] })?.includes("500"), "disabled, saying why, when the check failed");
expect(approveBlocker({ loading: false, failed: null, problems: found })?.startsWith("3 diagrams do not parse"), "disabled while problems are left");
expect(approveBlocker({ loading: false, failed: null, problems: found.slice(0, 1) })?.startsWith("1 diagram does not parse"), "one problem, in the singular");
expect(approveBlocker({ loading: false, failed: null, problems: [] }) === null, "enabled when every diagram parses");

for (const p of problems) console.error(`FAIL ${p}`);
console.log(`check:diagram-review: ${problems.length ? `${problems.length} problem(s)` : "the review page's diagram check holds"}`);
process.exit(problems.length ? 1 : 0);
