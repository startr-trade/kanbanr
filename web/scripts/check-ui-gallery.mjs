#!/usr/bin/env node
//
// The UI kit's gallery and layout (FTR-046):
//   web::check_gallery_shows_every_exported_component (R-8) — each component exported from
//     ui/index.ts is drawn in UiGalleryPage.tsx, main.tsx routes /ui with no build-mode condition,
//     and the gallery renders with one h1, both themes side by side;
//   web::check_pages_and_markdown_are_uncapped_and_rows_at_36px (R-11) — --page-max is none,
//     --row-h 36px, no --prose-max token and no max-width on .markdown; styles.css has no 1,200 px
//     cap; ui.css's .ui-row-head uses
//     min-height: var(--row-h) and white-space: nowrap.
//
// Usage: npm run check:ui-gallery
import { existsSync, readFileSync } from "node:fs";
import { closeRender, render } from "./lib/render.mjs";

const src = (path) => new URL(`../src/${path}`, import.meta.url).pathname;
const read = (path) => (existsSync(src(path)) ? readFileSync(src(path), "utf8") : null);
const stripComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

/** The components ui/index.ts exports: capitalised value names re-exported from a `.tsx` file. */
function exportedComponents(index) {
  const out = [];
  for (const m of stripComments(index).matchAll(/export\s*\{([^}]*)\}\s*from\s*"([^"]+)"/g)) {
    if (!m[2].endsWith(".tsx")) continue;
    for (const part of m[1].split(",")) {
      const name = part.trim();
      if (!name || name.startsWith("type ")) continue;
      // A component is PascalCase; an UPPER_CASE constant (STEP_WORD) is not one.
      if (/^[A-Z][a-z]\w*$/.test(name)) out.push(name);
    }
  }
  return out;
}

let ok = true;

// R-8: the gallery shows every component the kit exports, and /ui is routed in every build.
{
  const name = "web::check_gallery_shows_every_exported_component";
  const problems = [];
  const index = read("ui/index.ts");
  const gallery = read("pages/UiGalleryPage.tsx");
  const main = read("main.tsx");
  if (index === null) problems.push("web/src/ui/index.ts is missing");
  if (gallery === null) problems.push("web/src/pages/UiGalleryPage.tsx is missing");
  if (index !== null && gallery !== null) {
    const components = exportedComponents(index);
    if (components.length < 12) problems.push(`ui/index.ts exports ${components.length} components (${components.join(", ")}), expected at least 12`);
    const body = stripComments(gallery);
    for (const c of components) {
      if (!new RegExp(`<${c}[\\s/>]`).test(body)) problems.push(`UiGalleryPage.tsx does not draw <${c}>, which ui/index.ts exports`);
    }
    if (!/from\s+"\.\.\/ui\/index\.ts"/.test(body)) problems.push("UiGalleryPage.tsx does not import the kit from ui/index.ts");
    if (!/data-theme=\{/.test(body)) problems.push("UiGalleryPage.tsx draws no nested data-theme wrapper for the other theme");
  }
  if (main === null) problems.push("web/src/main.tsx is missing");
  else {
    const body = stripComments(main);
    if (!/\{\s*path:\s*"ui",\s*element:\s*<UiGalleryPage\s*\/>\s*\}/.test(body)) problems.push('main.tsx has no route { path: "ui", element: <UiGalleryPage /> }');
    if (/import\.meta\.env/.test(body)) problems.push("main.tsx reads import.meta.env: /ui must be routed in every build, with no build-mode condition");
  }
  // The gallery renders: one h1, and both themes drawn, the other one inside a data-theme wrapper.
  if (gallery !== null && index !== null) {
    try {
      const { document } = await render("src/pages/UiGalleryPage.tsx", "default", {}, { url: "/ui" });
      const h1 = document.querySelectorAll("h1").length;
      if (h1 !== 1) problems.push(`the gallery renders ${h1} h1 elements, expected 1`);
      const themes = document.querySelectorAll(".ui-gallery-theme");
      if (themes.length !== 2) problems.push(`the gallery renders ${themes.length} theme columns, expected 2`);
      else if (themes[1].getAttribute("data-theme") !== "light") problems.push(`the other theme's column is data-theme="${themes[1].getAttribute("data-theme")}", expected "light" (the server sees dark)`);
      for (const cls of ["ui-tabs", "ui-row", "ui-table", "ui-tile", "ui-progress", "ui-pill", "ui-matrix", "ui-timeline", "ui-stepper", "ui-loading", "ui-error", "ui-empty", "ui-section", "ui-copy"]) {
        const n = document.querySelectorAll(`.${cls}`).length;
        if (n < 2) problems.push(`the gallery renders ${n} .${cls}, expected one in each theme`);
      }
      const ids = [...document.querySelectorAll("[id]")].map((e) => e.id);
      const dup = ids.filter((id, i) => ids.indexOf(id) !== i);
      if (dup.length) problems.push(`ids repeat across the two theme columns: ${[...new Set(dup)].join(", ")}`);
    } catch (e) {
      problems.push(`the gallery does not render: ${e?.stack ?? e}`);
    }
  }
  ok = report(name, problems) && ok;
}

// R-11: no page cap, markdown at full width, single-line rows at 36 px.
{
  const name = "web::check_pages_and_markdown_are_uncapped_and_rows_at_36px";
  const problems = [];
  const { parseTokens } = await import("../src/ui/contrast.ts");
  const themes = parseTokens(read("ui/tokens.css") ?? "");
  const want = { "--page-max": "none", "--row-h": "36px" };
  for (const [theme, t] of Object.entries(themes)) {
    for (const [k, v] of Object.entries(want)) if (t[k] !== v) problems.push(`${theme}: ${k} is ${t[k]}, expected ${v}`);
  }
  const styles = stripComments(read("styles.css") ?? "");
  if (/max-width\s*:\s*1200px/.test(styles)) problems.push("styles.css still caps a page at max-width: 1200px");
  if (/(?:^|[\s,}])\.markdown\s*\{[^}]*max-width/.test(styles)) problems.push(".markdown still has a max-width cap");
  if (/--prose-max/.test(stripComments(read("ui/tokens.css") ?? "")) || /--prose-max/.test(styles)) problems.push("--prose-max is still defined or used");
  const ui = stripComments(read("ui/ui.css") ?? "");
  const head = ui.match(/(?:^|\})\s*\.ui-row-head\s*\{([^}]*)\}/);
  if (!head) problems.push("ui.css has no .ui-row-head rule");
  else {
    if (!/min-height\s*:\s*var\(--row-h\)/.test(head[1])) problems.push(".ui-row-head does not set min-height: var(--row-h)");
    if (!/white-space\s*:\s*nowrap/.test(head[1])) problems.push(".ui-row-head does not set white-space: nowrap");
  }
  ok = report(name, problems) && ok;
}

await closeRender();
if (!ok) process.exit(1);
