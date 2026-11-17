#!/usr/bin/env node
//
// DEF-082 R-3: the monitor always shows the keyboard focus ring.
//   web::check_focus_visible — in every CSS file under src/, no rule sets `outline: none` or
//     `outline: 0` unless its selector ends in `:focus:not(:focus-visible)` (mouse focus only), and
//     one `:focus-visible` rule's selector list covers a, button, input, select, textarea, summary,
//     [tabindex] and .tab with a non-zero outline width.
//
// Usage: npm run check:a11y-focus
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

/** Every innermost `selector { body }` rule, with the index of its selector. @-rule wrappers are
 *  skipped by taking only the text after the last `{` or `}` before each body. */
function rules(css) {
  const out = [];
  for (const m of css.matchAll(/([^{}]*)\{([^{}]*)\}/g)) {
    const raw = m[1];
    const lead = raw.length - raw.trimStart().length;
    out.push({ selector: raw.trim(), body: m[2], index: m.index + lead });
  }
  return out;
}

const splitSelectors = (selector) => selector.split(",").map((s) => s.trim()).filter(Boolean);

/** `outline: none` / `outline: 0` / `outline-style: none` / `outline-width: 0`. */
const REMOVES_OUTLINE = /(?<![-\w])outline(?:-style|-width)?\s*:\s*(?:none|0(?:px)?)\s*(?:!important\s*)?(?:;|$)/;

/** A non-zero outline width from `outline:` or `outline-width:`. */
function hasRing(body) {
  const short = /(?<![-\w])outline\s*:\s*([^;]+)/.exec(body)?.[1];
  const width = /(?<![-\w])outline-width\s*:\s*([^;]+)/.exec(body)?.[1];
  for (const value of [short, width]) {
    if (!value) continue;
    const m = /([0-9]*\.?[0-9]+)(px|rem|em)|\b(thin|medium|thick)\b/.exec(value);
    if (m && (m[3] || Number(m[1]) > 0)) return true;
  }
  return false;
}

const REQUIRED = ["a", "button", "input", "select", "textarea", "summary", "[tabindex]", ".tab"];

const problems = [];
let ring = null;

for (const path of walk(srcDir).filter((f) => f.endsWith(".css"))) {
  const name = relative(srcDir, path);
  const css = stripComments(readFileSync(path, "utf8"));
  for (const rule of rules(css)) {
    const selectors = splitSelectors(rule.selector);
    if (REMOVES_OUTLINE.test(rule.body)) {
      for (const s of selectors) {
        if (!s.endsWith(":focus:not(:focus-visible)")) {
          problems.push(`${name}:${lineOf(css, rule.index)}: "${s}" removes the outline; only a selector ending in :focus:not(:focus-visible) may`);
        }
      }
    }
    if (!ring && hasRing(rule.body)) {
      const covered = new Set(selectors.filter((s) => s.endsWith(":focus-visible")).map((s) => s.slice(0, -":focus-visible".length)));
      if (REQUIRED.every((r) => covered.has(r))) ring = `${name}:${lineOf(css, rule.index)}`;
    }
  }
}

if (!ring) {
  problems.push(`no :focus-visible rule with a non-zero outline covers all of ${REQUIRED.join(", ")}`);
}

if (problems.length) {
  for (const p of problems) console.error(`web::check_focus_visible — ${p}`);
  process.exit(1);
}
console.log(`web::check_focus_visible — ok (the shared ring is at ${ring})`);
