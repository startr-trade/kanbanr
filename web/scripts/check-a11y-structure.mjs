#!/usr/bin/env node
//
// DEF-082: the monitor's pages are structured for a screen reader.
//   web::check_markdown_demotes_headings_below_the_page_title (R-4) — markdownToSafeHtml shifts
//     headings down by headingOffset (default 1, capped at h6), so a spec's `# Title` sits under
//     the page's own h1; `Markdown` passes its `headingOffset` prop through.
//   web::check_one_h1_per_page (R-4) — at most one `<h1` per returned tree (`return (` block) in
//     each page file, and DocPage draws a fallback h1 when its doc has none.
//   web::check_cards_and_rows_have_headings (R-5) — FeatureCard's title is an h3, the Board's
//     column header is an h2, and AccordionRow's header is a heading.
//   web::check_text_alternatives_for_dot_icons_and_inputs (R-6) — LiveDot shows visible text, no
//     emoji in a .tsx is read aloud (each sits in an aria-hidden span), and every <input>, <select>
//     and <textarea> has an aria-label, an aria-labelledby, a wrapping <label> or a <label htmlFor>
//     naming its id.
//   web::check_live_dot_says_live_or_reconnecting (R-9) — LiveDot says "Live" or "Reconnecting…"
//     inside role=status, and live.ts's connectionReducer goes live → reconnecting on `error` and
//     back on `open`.
//
// Usage: npm run check:a11y-structure
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { markdownToSafeHtml } from "../src/markdownHtml.ts";
import { closeRender, render } from "./lib/render.mjs";

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)],
  );
}
/** Blank out comments but keep line breaks, so an index's line number is the file's. */
const stripComments = (text) =>
  text.replace(/\/\*[\s\S]*?\*\/|(^|[^:"'`])\/\/[^\n]*/gm, (c) => c.replace(/[^\n]/g, " "));
const lineOf = (text, index) => text.slice(0, index).split("\n").length;

/** The JSX opening tag that starts at `start` (`<input …>` or `<input … />`), braces respected. */
function openingTag(src, start) {
  let depth = 0;
  for (let i = start; i < src.length; i++) {
    const c = src[i];
    if (c === "{") depth++;
    else if (c === "}") depth--;
    else if (c === ">" && depth === 0) return src.slice(start, i + 1);
  }
  return src.slice(start);
}

const srcDir = new URL("../src", import.meta.url).pathname;
const read = (rel) => readFileSync(join(srcDir, rel), "utf8");
const purifier = createDOMPurify(new JSDOM("").window);

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

/** The heading tags of `html`, in order: ["h2", "h3"]. */
const headings = (html) => [...html.matchAll(/<(h[1-6])\b/g)].map((m) => m[1]);

const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
};

let ok = true;

try {
  // R-4: markdown headings are demoted below the page's own title.
  {
    const name = "web::check_markdown_demotes_headings_below_the_page_title";
    const problems = [];
    eq(problems, 'markdownToSafeHtml("# A\\n## B") headings', headings(markdownToSafeHtml("# A\n## B", undefined, purifier)), ["h2", "h3"]);
    eq(problems, "with headingOffset 0", headings(markdownToSafeHtml("# A\n## B", undefined, purifier, null, 0)), ["h1", "h2"]);
    eq(problems, "with headingOffset 2", headings(markdownToSafeHtml("# A\n## B", undefined, purifier, null, 2)), ["h3", "h4"]);
    eq(problems, '"###### F" stays h6', headings(markdownToSafeHtml("###### F", undefined, purifier)), ["h6"]);
    const text = markdownToSafeHtml("# A heading", undefined, purifier);
    if (!/<h2[^>]*>A heading<\/h2>/.test(text)) problems.push(`the demoted heading lost its text: ${text}`);
    const markdown = read("components/Markdown.tsx");
    if (!/headingOffset\?:\s*0\s*\|\s*1\s*\|\s*2/.test(markdown)) problems.push("Markdown.tsx has no `headingOffset?: 0 | 1 | 2` prop");
    if (!/markdownToSafeHtml\([^)]*headingOffset/.test(markdown)) problems.push("Markdown.tsx does not pass headingOffset to markdownToSafeHtml");
    if (!/<Markdown[^>]*headingOffset=\{1\}/.test(read("pages/FeaturePage.tsx"))) problems.push("FeaturePage does not render its spec with headingOffset={1}");
    ok = report(name, problems) && ok;
  }

  // R-4: one h1 per returned tree in every page.
  {
    const name = "web::check_one_h1_per_page";
    const problems = [];
    const pages = readdirSync(join(srcDir, "pages")).filter((f) => f.endsWith(".tsx"));
    for (const file of pages) {
      const src = read(`pages/${file}`);
      const blocks = src.split(/\breturn \(/);
      blocks.forEach((block, i) => {
        const n = (block.match(/<h1\b/g) ?? []).length;
        if (n > 1) problems.push(`pages/${file}: ${n} <h1 in ${i === 0 ? "the code before the first return" : `return block ${i}`}, expected at most 1`);
      });
    }
    const doc = read("pages/DocPage.tsx");
    if (!/<h1\b/.test(doc)) problems.push("pages/DocPage.tsx draws no fallback <h1 for a doc that starts without one");
    if (!/headingOffset=\{/.test(doc)) problems.push("pages/DocPage.tsx does not choose the Markdown headingOffset");
    ok = report(name, problems) && ok;
  }

  // R-5: cards, columns and rows are headed.
  {
    const name = "web::check_cards_and_rows_have_headings";
    const problems = [];
    const feature = { code: "DEF-082", title: "The monitor fails contrast", done: 1, total: 2 };
    const card = await render("src/components/bits.tsx", "FeatureCard", { project: "p", feature });
    const h3 = card.document.querySelector("h3.card-title");
    if (!h3) problems.push(`FeatureCard draws no h3.card-title: ${card.html}`);
    else if (!h3.textContent.includes(feature.title)) problems.push(`FeatureCard's h3 says "${h3.textContent}", expected the title`);
    const board = read("pages/BoardPage.tsx");
    if (!/<h2 className="col-title">\s*<Link className="column-head"/.test(board)) problems.push('BoardPage\'s column header is not an <h2 className="col-title"> around the column link');
    const row = await render("src/ui/AccordionRow.tsx", "AccordionRow", { id: "r1", title: "A row", children: () => "body" });
    const head = row.document.querySelector("h1, h2, h3, h4, h5, h6");
    if (!head || !head.textContent.includes("A row")) problems.push(`AccordionRow's header is not a heading holding its title: ${row.html}`);
    ok = report(name, problems) && ok;
  }

  // R-6: the dot, icons and inputs have text a screen reader can use.
  {
    const name = "web::check_text_alternatives_for_dot_icons_and_inputs";
    const problems = [];
    const dot = await render("src/components/bits.tsx", "LiveDot", { state: "live" });
    if (!(dot.document.body.textContent ?? "").trim()) problems.push(`LiveDot draws no visible text: ${dot.html}`);
    const tsx = walk(srcDir).filter((f) => f.endsWith(".tsx"));
    for (const file of tsx) {
      const rel = file.slice(srcDir.length + 1);
      const src = stripComments(readFileSync(file, "utf8"));
      for (const m of src.matchAll(/\p{Emoji_Presentation}/gu)) {
        const before = src.slice(0, m.index);
        const opens = [...before.matchAll(/<span\b[^>]*\baria-hidden="true"[^>]*>/g)];
        const open = opens.length ? opens[opens.length - 1].index : -1;
        const hidden = open >= 0 && before.lastIndexOf("</span>") < open;
        if (!hidden) problems.push(`${rel}:${lineOf(src, m.index)}: emoji ${m[0]} is read aloud; wrap it in <span aria-hidden="true">`);
      }
      for (const m of src.matchAll(/<(input|select|textarea)\b/g)) {
        const tag = openingTag(src, m.index);
        if (/\baria-label(ledby)?=/.test(tag)) continue;
        const before = src.slice(0, m.index);
        if (before.lastIndexOf("<label") > before.lastIndexOf("</label>")) continue;
        // A <label htmlFor> naming the control's id labels it as well as wrapping does.
        const id = /\bid=(\{[^}]*\}|"[^"]*")/.exec(tag)?.[1];
        if (id && src.includes(`htmlFor=${id}`)) continue;
        problems.push(`${rel}:${lineOf(src, m.index)}: <${m[1]}> has no aria-label, aria-labelledby or wrapping <label>`);
      }
    }
    ok = report(name, problems) && ok;
  }

  // R-9: the live dot says what the stream is doing.
  {
    const name = "web::check_live_dot_says_live_or_reconnecting";
    const problems = [];
    for (const [state, want] of [["live", "Live"], ["reconnecting", "Reconnecting…"]]) {
      const { html, document } = await render("src/components/bits.tsx", "LiveDot", { state });
      const status = document.querySelectorAll('[role="status"]');
      if (status.length !== 1) problems.push(`state=${state}: ${status.length} role=status elements, expected 1: ${html}`);
      else if ((status[0].textContent ?? "").trim() !== want) problems.push(`state=${state}: the status says "${status[0].textContent}", expected "${want}"`);
    }
    const live = await import("../src/data/live.ts").catch((e) => ({ error: e }));
    if (typeof live.connectionReducer !== "function") problems.push("src/data/live.ts exports no connectionReducer");
    else {
      eq(problems, 'connectionReducer("live", "error")', live.connectionReducer("live", "error"), "reconnecting");
      eq(problems, 'connectionReducer("reconnecting", "error")', live.connectionReducer("reconnecting", "error"), "reconnecting");
      eq(problems, 'connectionReducer("reconnecting", "open")', live.connectionReducer("reconnecting", "open"), "live");
      eq(problems, 'connectionReducer("live", "open")', live.connectionReducer("live", "open"), "live");
    }
    if (typeof live.useConnection !== "function") problems.push("src/data/live.ts exports no useConnection");
    ok = report(name, problems) && ok;
  }
} finally {
  await closeRender();
}

process.exit(ok ? 0 : 1);
