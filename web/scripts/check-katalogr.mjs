#!/usr/bin/env node
//
// A doc's `katalogr-query` blocks (FTR-016): drawn from katalogr's answer with Pele, or left as
// their source with one plain sentence under them, never blanked, never an action. Runs the pure
// helpers of `src/docViews.ts` in jsdom with the SAME Pele and sanitiser the monitor uses, against
// the contract's own `views-v1.json`, and reads `styles.css` for the figure's tokens.
//
// Cases, named as the board's tests are: web::doc_views_show_source_when_katalogr_is_absent (R-5),
// web::doc_views_draw_with_pele_and_show_position_and_id (R-6),
// web::doc_views_figure_is_a_section_with_position_and_source_under_it (R-13),
// web::releases_page_names_the_snapshot_repos (R-9).
//
// Usage: npm run check:katalogr
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { readFileSync } from "node:fs";
import { render } from "pele";
import { safeSvg } from "../src/markdownHtml.ts";
import { caption, figureElement, pairBlocks, saidElement, saidFor, snapshotLine, webViewFor } from "../src/docViews.ts";

const { window } = new JSDOM("");
const purifier = createDOMPurify(window);
const document = window.document;
const problems = [];
const fail = (test, what) => problems.push(`web::${test}: ${what}`);

const contract = JSON.parse(readFileSync(new URL("../../fixtures/contract/views-v1.json", import.meta.url), "utf8"));
const blockViewOf = (body) => ({
  mermaid: body.mermaid,
  positions: body.positions,
  exact: body.exact,
  ...(body.view_id ? { view_id: body.view_id } : {}),
  pele: body.pele,
});
const docViews = (blocks, katalogr) => ({
  doc: "arch/views.md",
  position: "0123456789abcdef0123456789abcdef01234567",
  position_from: "the live position of the project's checkout",
  katalogr: katalogr ?? { state: "ready", url: "http://127.0.0.1:7878", katalogr: "0.9.0", contract: 1, capabilities: ["views"] },
  blocks,
});

// ---- R-5: katalogr absent: the source stays, one sentence is added as text ----
{
  const t = "doc_views_show_source_when_katalogr_is_absent";
  const url = "http://127.0.0.1:7878";
  const said = `katalogr is not running at ${url} — the block shows its source`;
  const views = docViews(
    [
      { index: 0, line: 3, source: "structure --level file", said },
      { index: 1, line: 9, source: "nonsense", problem: "`nonsense` is not an operation", said: "this block is not a query: `nonsense` is not an operation" },
    ],
    { state: "absent", url, said: `katalogr is not running at ${url}` },
  );
  const root = document.createElement("div");
  root.innerHTML = views.blocks
    .map((b) => `<pre><code class="language-katalogr-query">${b.source}</code></pre>`)
    .join("");
  const hosts = Array.from(root.querySelectorAll("code.language-katalogr-query")).map((c) => c.closest("pre"));
  for (const pair of pairBlocks(hosts, views)) {
    const what = webViewFor(views, pair);
    if (what.kind !== "say") fail(t, "a block with no view was to be drawn");
    else pair.node.after(saidElement(document, what.text));
  }
  if (root.querySelectorAll("pre").length !== 2) fail(t, "a source <pre> did not stay");
  const saids = Array.from(root.querySelectorAll(".katalogr-said"));
  if (saids.length !== 2) fail(t, `expected one sentence per block, got ${saids.length}`);
  if (!saids[0]?.textContent.includes("katalogr is not running")) fail(t, "the sentence does not say katalogr is not running");
  if (!saids[1]?.textContent.includes("this block is not a query")) fail(t, "a bad block's reason is not shown");
  if (root.querySelector("a, button, input, [onclick]") || Array.from(root.querySelectorAll("*")).some((e) => e.getAttributeNames().some((n) => n.startsWith("on")))) {
    fail(t, "an element with a handler or a control survived");
  }
  if (saidFor(views, { index: 5, line: 1, source: "x" }).length === 0) fail(t, "a block with no said has no fallback sentence");
  // A daemon that could not be asked at all says so too, and still keeps the source.
  const unreached = webViewFor(null, { node: null, block: undefined });
  if (unreached.kind !== "say" || !unreached.text.includes("shows its source")) fail(t, "an unreached daemon says nothing");
}

// ---- R-6: Pele draws katalogr's own answer; the caption names op, position, exactness, ID ----
const exactView = blockViewOf(contract.exact);
const notExactView = blockViewOf(contract.not_exact);
{
  const t = "doc_views_draw_with_pele_and_show_position_and_id";
  const svg = safeSvg(render(contract.exact.mermaid, { idPrefix: "k-" }).svg, purifier);
  if (!svg.startsWith("<svg")) fail(t, "Pele's diagram did not survive the sanitiser");
  if (!svg.includes("app:ring2.tx")) fail(t, "the diagram lost its labels");
  if (/<script|\son[a-z]+=/i.test(svg)) fail(t, "the diagram kept a script");

  const views = docViews([{ index: 0, line: 3, source: "structure --level file", view: { ...exactView, warnings: ["katalogr: the checkout of lib is not here"] } }]);
  const text = caption(views.blocks[0], views);
  for (const need of ["katalogr view structure", "at 0123456 (clean)", "exact", "view v1-0123…cdef"]) {
    if (!text.includes(need)) fail(t, `the caption "${text}" lacks "${need}"`);
  }
  const figure = figureElement(document, views.blocks[0], views, svg);
  if (!figure.querySelector("svg")) fail(t, "the figure holds no SVG");
  if (!figure.textContent.includes("the checkout of lib is not here")) fail(t, "a warning is not shown under the caption");
  if (figure.querySelector(".katalogr-warning svg, .katalogr-warning a")) fail(t, "a warning is more than text");

  const loose = docViews([{ index: 0, line: 3, source: "structure", view: notExactView }]);
  const looseText = caption(loose.blocks[0], loose);
  if (!looseText.includes("not exact")) fail(t, `the not-exact caption "${looseText}" does not say so`);
  if (/view v1-|view /.test(looseText.replace("katalogr view", ""))) fail(t, "a not-exact answer shows a view ID");

  // The open link is only ever an http(s) address.
  const hostile = docViews([{ index: 0, line: 3, source: "structure", view: { ...exactView, open: "javascript:alert(1)" } }]);
  if (figureElement(document, hostile.blocks[0], hostile, svg).querySelector("a")) fail(t, "a javascript: open link was made a link");
  const opened = docViews([{ index: 0, line: 3, source: "structure", view: { ...exactView, open: "http://127.0.0.1:7878/#/graph?at=0123456" } }]);
  if (!figureElement(document, opened.blocks[0], opened, svg).querySelector('a[href^="http://127.0.0.1:7878/"]')) fail(t, "the open link is missing");
}

// ---- R-13: one figure: a Section with an aria-label, the SVG, the position line, the source ----
{
  const t = "doc_views_figure_is_a_section_with_position_and_source_under_it";
  const svg = safeSvg(render(contract.exact.mermaid, { idPrefix: "f-" }).svg, purifier);
  const views = docViews([{ index: 0, line: 3, source: " structure --level file ", view: exactView }]);
  const fig = figureElement(document, views.blocks[0], views, svg);
  if (fig.tagName !== "SECTION" || !fig.classList.contains("ui-section") || !fig.classList.contains("katalogr-view")) {
    fail(t, "the figure is not a Section of the UI kit's class");
  }
  if (!fig.getAttribute("aria-label")?.startsWith("katalogr view")) fail(t, "the figure has no aria-label");
  if (fig.querySelector("h1, h2, h3, h4, h5, h6")) fail(t, "the figure holds a heading, which would change the doc's outline");
  const kids = Array.from(fig.children);
  if (kids.length !== 3 || !kids[0].querySelector("svg") || !kids[1].classList.contains("katalogr-position") || !kids[2].classList.contains("katalogr-source")) {
    fail(t, "the figure's children are not, in order, the SVG, the position line and the source");
  }
  if (kids[2]?.querySelector("code")?.textContent !== "structure --level file") fail(t, "the source line is not the block's one-line query");

  const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((m) => /\.katalogr-(view|said|drawing|position|source|warning)/.test(m[1]));
  if (!rules.some((m) => m[1].includes(".katalogr-view"))) fail(t, "styles.css has no .katalogr-view rule");
  if (!rules.some((m) => m[1].includes(".katalogr-said"))) fail(t, "styles.css has no .katalogr-said rule");
  for (const [, selector, body] of rules) {
    for (const decl of body.split(";").map((d) => d.trim()).filter(Boolean)) {
      const value = decl.slice(decl.indexOf(":") + 1).replace(/var\(--[a-z0-9-]+\)/g, "");
      if (/#[0-9a-f]{3,8}\b|\b(rgb|hsl)a?\(|\d(px|rem|em|pt)\b/i.test(value)) {
        fail(t, `${selector.trim()} { ${decl} } uses a literal, not a token`);
      }
    }
  }
}

// ---- R-9: the Releases page names the snapshot's repos: saved, skipped, absent; never a button ----
{
  const t = "releases_page_names_the_snapshot_repos";
  const saved = {
    version: "v0.8.0",
    snapshot: {
      state: "saved",
      dir: "releases/v0.8.0/views",
      at: "0123456789abcdef0123456789abcdef01234567",
      repos: ["kanbanr", "katalogr"],
      blocks: 2,
      views: 3,
      reason: "",
    },
  };
  const line = snapshotLine(saved);
  if (!line) fail(t, "a saved snapshot gave no line");
  else {
    const text = `${line.before}${line.dir ?? ""}${line.after}`;
    for (const want of ["2 blocks", "3 views", "0123456", "releases/v0.8.0/views", "kanbanr", "katalogr", "before pushing a public board"]) {
      if (!text.includes(want)) fail(t, `the saved line "${text}" lacks "${want}"`);
    }
    if (line.dir !== "releases/v0.8.0/views") fail(t, "the saved line does not carry the folder to link");
    if (text.includes("0123456789abcdef")) fail(t, "the saved line shows the whole commit");
  }
  const one = snapshotLine({ version: "v1", snapshot: { ...saved.snapshot, blocks: 1, views: 1, repos: ["kanbanr"] } });
  if (!one || !`${one.before}${one.after}`.includes("1 block,") || !`${one.before}${one.after}`.includes("1 view ")) fail(t, "one block and one view are not said in the singular");
  const skipped = snapshotLine({ version: "v0.8.0", snapshot: { state: "skipped", dir: "", at: "", repos: [], blocks: 0, views: 0, reason: "katalogr is not installed (nothing named katalogr on PATH)" } });
  if (!skipped || !skipped.before.startsWith("no system views: katalogr is not installed") || skipped.dir) fail(t, "a skipped snapshot does not say no system views and why");
  if (snapshotLine({ version: "v0.7.0" }) !== null) fail(t, "a release with no record shows a line");
  // The line is plain text and a link, never an action.
  if (Object.keys(line ?? {}).some((k) => /click|on[A-Z]|action/.test(k))) fail(t, "the line carries an action");
}

if (problems.length) {
  for (const p of problems) console.error(`check:katalogr — ${p}`);
  process.exit(1);
}
console.log(
  "check:katalogr — a block with no view keeps its source and says why in plain text; a view is drawn by Pele " +
    "with its position, exactness and ID; the figure is a Section of tokens only, with the source under it.",
);
