#!/usr/bin/env node
//
// DEF-082 R-2: the monitor draws no text under 13 px.
//   web::check_min_font_size — no px font size below 13 and no rem or em below 0.8125 in any CSS
//     file or inline style; every `font-size` in styles.css and ui.css is `var(--fs-N)` or
//     `inherit`, and a `font:` shorthand's size is a token. tokens.css defines the scale, so it is
//     exempt, but its smallest step must be 0.8125rem.
//
// Usage: npm run check:a11y-type
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const srcDir = new URL("../src", import.meta.url).pathname;

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

/** Blank out comments but keep line breaks, so a match's line number is the file's. */
const stripComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, " "));
const lineOf = (text, index) => text.slice(0, index).split("\n").length;

/** A size below the bar: px under 13, rem or em under 0.8125. */
function tooSmall(value) {
  const m = /^([0-9]*\.?[0-9]+)(px|rem|em)$/.exec(value.trim());
  if (!m) return false;
  const n = Number(m[1]);
  return m[2] === "px" ? n < 13 : n < 0.8125;
}

const TOKEN = /^var\(--fs-[1-5]\)$/;
const STRICT = new Set(["styles.css", "ui/ui.css"]);

const problems = [];
const files = walk(srcDir);

for (const path of files.filter((f) => f.endsWith(".css"))) {
  const name = relative(srcDir, path);
  const css = stripComments(readFileSync(path, "utf8"));
  if (name === "ui/tokens.css") {
    const steps = [...css.matchAll(/--fs-[0-9]+\s*:\s*([^;]+);/g)].map((m) => m[1].trim());
    const rem = steps.map((v) => (/^([0-9.]+)rem$/.exec(v) ? Number(v.slice(0, -3)) : NaN));
    const smallest = Math.min(...rem);
    if (!steps.length) problems.push(`${name}: defines no --fs-N steps`);
    else if (rem.some(Number.isNaN)) problems.push(`${name}: every --fs-N step must be in rem (${steps.join(", ")})`);
    else if (smallest !== 0.8125) problems.push(`${name}: the smallest step is ${smallest}rem, expected 0.8125rem`);
    continue;
  }
  for (const m of css.matchAll(/(?<![-\w])font-size\s*:\s*([^;}]+)/g)) {
    const value = m[1].trim();
    const at = `${name}:${lineOf(css, m.index)}`;
    if (tooSmall(value)) problems.push(`${at}: font-size ${value} is under 13 px`);
    else if (STRICT.has(name) && !TOKEN.test(value) && value !== "inherit") {
      problems.push(`${at}: font-size ${value} — use var(--fs-1) … var(--fs-5) or inherit`);
    }
  }
  for (const m of css.matchAll(/(?<![-\w])font\s*:\s*([^;}]+)/g)) {
    const value = m[1].trim();
    const at = `${name}:${lineOf(css, m.index)}`;
    if (value === "inherit") continue;
    // The size is the token before an optional `/line-height` and the family.
    const size = /(var\(--[a-z0-9-]+\)|[0-9]*\.?[0-9]+(?:px|rem|em|%))(?:\s*\/|\s)/.exec(value)?.[1];
    if (!size || !TOKEN.test(size)) problems.push(`${at}: font shorthand "${value}" — its size must be var(--fs-N)`);
  }
}

for (const path of files.filter((f) => f.endsWith(".tsx"))) {
  const name = relative(srcDir, path);
  const text = readFileSync(path, "utf8");
  for (const m of text.matchAll(/fontSize\s*:\s*(?:(["'`])([^"'`]+)\1|([0-9.]+))/g)) {
    const value = m[2] ?? `${m[3]}px`;
    if (tooSmall(value)) problems.push(`${name}:${lineOf(text, m.index)}: inline fontSize ${value} is under 13 px`);
  }
  for (const m of text.matchAll(/font-size\s*:\s*([0-9]*\.?[0-9]+(?:px|rem|em))/g)) {
    if (tooSmall(m[1])) problems.push(`${name}:${lineOf(text, m.index)}: inline font-size ${m[1]} is under 13 px`);
  }
}

if (problems.length) {
  for (const p of problems) console.error(`web::check_min_font_size — ${p}`);
  process.exit(1);
}
console.log("web::check_min_font_size — ok");
