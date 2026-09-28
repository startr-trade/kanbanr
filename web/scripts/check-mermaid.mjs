#!/usr/bin/env node
//
// Parse every ```mermaid block in the given markdown files with the SAME library the monitor
// renders them with, and fail if any of them does not parse.
//
// Why this exists: a diagram that does not parse renders as nothing. Nothing looks like a missing
// image, not like a syntax error, so it survives review and is found months later by a reader —
// which is exactly how the sequence diagram in DESIGN.md was found. The failure mode is silence,
// so the check has to be mechanical.
//
// The classic cause is punctuation that means something to mermaid: a `;` inside a message is a
// statement separator and cuts the line in half; parentheses in a participant alias, and quotes in
// a label, do similar damage.
//
// Usage: node tools/docs/check-mermaid.mjs <file.md> [more.md …]
import fs from "node:fs";

// mermaid's sanitizer expects a DOM. Without one, every flowchart fails on `DOMPurify.addHook`,
// which would drown the diagrams that are really broken.
import { JSDOM } from "jsdom";
const dom = new JSDOM("<!doctype html><html><body></body></html>");
globalThis.window = dom.window;
globalThis.document = dom.window.document;
const mermaid = (await import("mermaid")).default;
mermaid.initialize({ startOnLoad: false });

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: check-mermaid.mjs <file.md> [more.md …]");
  process.exit(2);
}

let checked = 0;
let failed = 0;
for (const file of files) {
  const text = fs.readFileSync(file, "utf8");
  const blocks = [...text.matchAll(/```mermaid\n([\s\S]*?)```/g)];
  for (const [i, block] of blocks.entries()) {
    checked++;
    const kind = block[1].split("\n")[0].trim();
    try {
      await mermaid.parse(block[1]);
    } catch (e) {
      failed++;
      const detail = String(e.message).split("\n").slice(0, 4).join("\n      ");
      console.error(`FAIL ${file} diagram #${i + 1} (${kind})\n      ${detail}`);
    }
  }
}
// A glob the shell did not expand leaves this checking nothing and reporting success, which is the
// same "silence read as absence" failure the rest of this project keeps finding. The docs have
// diagrams; zero means the paths are wrong, not that the docs are clean.
if (checked === 0) {
  console.error(
    `check:docs — no diagrams found in ${process.argv.length - 2} path argument(s).\n` +
      "      That means the paths did not match, not that the docs are clean. Check the glob.",
  );
  process.exit(1);
}
console.log(`${checked} diagram(s) checked, ${failed} broken`);
process.exit(failed ? 1 : 0);
