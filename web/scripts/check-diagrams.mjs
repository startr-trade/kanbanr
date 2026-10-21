#!/usr/bin/env node
//
// Draw every ```mermaid block in the given markdown files with the SAME Pele the book and the
// monitor draw them with (FEAT-203), and fail if any of them cannot be drawn.
//
// Why this exists: a diagram that does not parse renders as nothing. Nothing looks like a missing
// image, not like a syntax error, so it survives review and is found months later by a reader —
// which is exactly how the sequence diagram in DESIGN.md was found (L-10). The failure mode is
// silence, so the check has to be mechanical.
//
// The classic cause is punctuation that means something to Mermaid's grammar: a `;` inside a
// message is a statement separator and cuts the line in half; parentheses in a participant alias,
// and quotes in a label, do similar damage.
//
// It also checks that the book's copy of Pele (docs/pele.global.js) is the pinned package's build,
// byte for byte, so what this check drew is what a reader's browser draws.
//
// Usage: node scripts/check-diagrams.mjs <file.md> [more.md …]
import fs from "node:fs";
import { render } from "pele";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: check-diagrams.mjs <file.md> [more.md …]");
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
      const { svg } = render(block[1]);
      if (!svg.startsWith("<svg")) throw new Error("Pele returned no SVG");
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

const pinned = new URL("../node_modules/pele/dist/pele.global.js", import.meta.url);
const book = new URL("../../docs/pele.global.js", import.meta.url);
const same = fs.existsSync(book) && fs.readFileSync(pinned).equals(fs.readFileSync(book));
if (!same) {
  console.error("FAIL docs/pele.global.js is not the pinned Pele's build — run tools/docs/pele.sh");
}
console.log(`${checked} diagram(s) drawn with Pele, ${failed} broken; the book's Pele is ${same ? "" : "NOT "}the pinned one`);
process.exit(failed || !same ? 1 : 0);
