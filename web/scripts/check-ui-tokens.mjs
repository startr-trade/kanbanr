#!/usr/bin/env node
//
// The UI kit's design tokens (FTR-046):
//   web::check_ui_uses_tokens_only (R-7) — the kit's CSS uses tokens, never raw colours or px type;
//   web::check_type_scale_is_the_chosen_five_steps (R-9) — 13 / 14 / 16 / 18 / 22 px;
//   web::check_severity_palette_is_the_chosen_values (R-10) — the computed palette, both themes.
//
// The palette check has its own small parser of tokens.css until DEF-082's `parseTokens` exists
// (DEF-082 T1 replaces it with the import).
//
// Usage: npm run check:ui-tokens
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const uiDir = new URL("../src/ui", import.meta.url).pathname;
const read = (name) => readFileSync(join(uiDir, name), "utf8");
const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const tokensCss = read("tokens.css");

/** Custom properties per theme, in cascade order, with `var()` resolved. */
function parseTokens(css) {
  const blocks = [...stripComments(css).matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({
    selectors: m[1].split(",").map((s) => s.trim()),
    props: [...m[2].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)].map((p) => [p[1], p[2].trim()]),
  }));
  const theme = (name) => {
    const applies = [":root", "[data-theme]", `[data-theme="${name}"]`];
    const raw = new Map();
    for (const b of blocks) if (b.selectors.some((s) => applies.includes(s))) for (const [k, v] of b.props) raw.set(k, v);
    const resolve = (v, depth = 0) =>
      depth > 20 ? v : v.replace(/var\((--[a-z0-9-]+)\)/g, (all, k) => (raw.has(k) ? resolve(raw.get(k), depth + 1) : all));
    return { raw, value: (k) => (raw.has(k) ? resolve(raw.get(k)) : undefined) };
  };
  return { dark: theme("dark"), light: theme("light") };
}

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

let ok = true;

// R-7: tokens only in the kit's own CSS, and every token it uses is defined.
{
  const problems = [];
  const defined = new Set([...stripComments(tokensCss).matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
  const files = readdirSync(uiDir).filter((f) => f.endsWith(".css") && f !== "tokens.css");
  for (const file of files) {
    const css = stripComments(read(file));
    for (const m of css.matchAll(/#[0-9a-fA-F]{3,8}\b/g)) problems.push(`${file}: a hex colour ${m[0]} — use a token`);
    for (const m of css.matchAll(/\brgba?\(/g)) problems.push(`${file}: ${m[0]} — use a token`);
    for (const m of css.matchAll(/font-size\s*:\s*[0-9.]+px/g)) problems.push(`${file}: "${m[0]}" — use --fs-1 … --fs-5`);
    for (const m of css.matchAll(/var\((--[a-z0-9-]+)/g)) {
      if (!defined.has(m[1])) problems.push(`${file}: var(${m[1]}) is used but tokens.css does not define it`);
    }
  }
  if (!files.includes("ui.css")) problems.push("ui.css is missing");
  ok = report("web::check_ui_uses_tokens_only", problems) && ok;
}

const themes = parseTokens(tokensCss);

// R-9: exactly five type steps, with the chosen values, the same in both themes.
{
  const problems = [];
  const want = ["0.8125rem", "0.875rem", "1rem", "1.125rem", "1.375rem"];
  for (const [name, t] of Object.entries(themes)) {
    const steps = [...t.raw.keys()].filter((k) => /^--fs-/.test(k)).sort();
    const expected = want.map((_, i) => `--fs-${i + 1}`);
    if (steps.join(" ") !== expected.join(" ")) problems.push(`${name}: type steps are ${steps.join(" ") || "none"}, expected ${expected.join(" ")}`);
    want.forEach((v, i) => {
      const got = t.value(`--fs-${i + 1}`);
      if (got !== v) problems.push(`${name}: --fs-${i + 1} is ${got}, expected ${v}`);
    });
  }
  ok = report("web::check_type_scale_is_the_chosen_five_steps", problems) && ok;
}

// R-10: the severity palette the person chose, and each pill background the 12% mix.
{
  const problems = [];
  const want = {
    light: { "--error-fg": "#b3262f", "--warn-fg": "#875f00", "--ok-fg": "#177233" },
    dark: { "--error-fg": "#ff7b72", "--warn-fg": "#d29922", "--ok-fg": "#3fb950" },
  };
  for (const [name, t] of Object.entries(themes)) {
    for (const [k, v] of Object.entries(want[name])) {
      const got = t.value(k)?.toLowerCase();
      if (got !== v) problems.push(`${name}: ${k} is ${got}, expected ${v}`);
      const plain = k.replace(/-fg$/, "");
      if (t.value(plain)?.toLowerCase() !== v) problems.push(`${name}: ${plain} is ${t.value(plain)}, expected ${v}`);
    }
    const accent = t.value("--accent");
    if (!accent || t.value("--info-fg") !== accent) problems.push(`${name}: --info-fg is ${t.value("--info-fg")}, expected --accent (${accent})`);
    for (const tone of ["ok", "warn", "error", "info"]) {
      const bg = t.raw.get(`--${tone}-bg`)?.replace(/\s+/g, " ");
      const mix = `color-mix(in srgb, var(--${tone}-fg) 12%, var(--surface))`;
      if (bg !== mix) problems.push(`${name}: --${tone}-bg is ${bg}, expected ${mix}`);
    }
  }
  ok = report("web::check_severity_palette_is_the_chosen_values", problems) && ok;
}

if (!ok) process.exit(1);
