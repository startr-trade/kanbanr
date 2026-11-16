#!/usr/bin/env node
//
// DEF-082 R-1: the monitor's colour tokens meet WCAG 2.1 AA in both themes.
//   web::check_contrast_pairs — `failures(tokens.css)` is empty: every text pair 4.5:1, every
//     boundary and the focus ring 3:1. Unit cases pin the formula: white on #5b8cff is 3.16 (the
//     dark Passed button before the fix), black on white is 21, and aliases resolve.
//   web::check_contrast_fixes_are_the_chosen_values (R-8) — the greys and the filled accent the
//     person chose on 16 Nov 2026, option A for every row.
//
// Usage: npm run check:a11y-contrast
import { readFileSync } from "node:fs";
import { failures, parseTokens, ratio } from "../src/ui/contrast.ts";

const tokensCss = readFileSync(new URL("../src/ui/tokens.css", import.meta.url), "utf8");

let failed = false;
function report(name, problems) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}

// R-1
{
  const problems = [];
  const near = (got, want, what) => {
    if (Math.abs(got - want) > 0.005) problems.push(`${what} is ${got.toFixed(3)}, expected ${want}`);
  };
  near(ratio("#ffffff", "#5b8cff"), 3.16, 'ratio("#ffffff", "#5b8cff")');
  near(ratio("#000", "#fff"), 21, 'ratio("#000", "#fff")');
  near(ratio("#fff", "#000"), 21, 'ratio("#fff", "#000")');
  const alias = parseTokens(":root { --a: #123456; --b: var(--a); --c: var(--b); }");
  if (alias.dark["--c"] !== "#123456") problems.push(`a two-step alias resolved to ${alias.dark["--c"]}, expected #123456`);
  const mixed = parseTokens(":root { --f: #ffffff; --s: #000000; --m: color-mix(in srgb, var(--f) 50%, var(--s)); }");
  if (mixed.dark["--m"] !== "#808080") problems.push(`a 50% color-mix resolved to ${mixed.dark["--m"]}, expected #808080`);

  for (const f of failures(tokensCss)) {
    const measured = f.ratio === 0 ? "has a token tokens.css does not define as a colour" : `is ${f.ratio.toFixed(2)}:1`;
    problems.push(`${f.theme}: ${f.pair.fg} on ${f.pair.bg} ${measured}, below ${f.pair.min}:1 (${f.pair.where})`);
  }
  report("web::check_contrast_pairs", problems);
}

// R-8
{
  const problems = [];
  const themes = parseTokens(tokensCss);
  const want = {
    light: { "--text-subtle": "#616b78", "--warn": "#875f00", "--ok": "#177233" },
    dark: { "--text-subtle": "#959daa", "--accent": "#5b8cff", "--on-accent": "#0f1115" },
  };
  for (const [theme, values] of Object.entries(want)) {
    for (const [k, v] of Object.entries(values)) {
      if (themes[theme][k] !== v) problems.push(`${theme}: ${k} is ${themes[theme][k]}, expected ${v}`);
    }
  }
  report("web::check_contrast_fixes_are_the_chosen_values", problems);
}

if (failed) process.exit(1);
