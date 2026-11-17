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
//
// Usage: npm run check:a11y-structure
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { markdownToSafeHtml } from "../src/markdownHtml.ts";
import { closeRender, render } from "./lib/render.mjs";

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
} finally {
  await closeRender();
}

process.exit(ok ? 0 : 1);
