#!/usr/bin/env node
//
// The monitor's markdown is sanitised (FEAT-140): render a document carrying the usual ways to run
// script — a <script>, event-handler attributes, javascript: links, an <iframe> — through the SAME
// function the monitor uses, and fail if any survives or if ordinary markdown stops rendering.
//
// Usage: npm run check:markdown
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { readFileSync } from "node:fs";
import { render } from "pele";
import { markdownToSafeHtml, safeSvg } from "../src/markdownHtml.ts";
import { find, isBroken, label, makeIndex } from "../src/boardRefs.ts";

const { window } = new JSDOM("");
const purifier = createDOMPurify(window);

const hostile = [
  "# Release notes",
  "",
  "<script>fetch('/api/projects/x/features/FEAT-1/approve', {method: 'POST'})</script>",
  "",
  '<img src="x.png" onerror="alert(1)">',
  "",
  "[click me](javascript:alert(1))",
  "",
  '<a href="https://example.com" onclick="alert(1)">a link</a>',
  "",
  '<iframe src="https://example.com"></iframe>',
  "",
  "| a | b |",
  "|---|---|",
  "| 1 | 2 |",
  "",
  "```mermaid",
  "flowchart LR",
  "  A --> B",
  "```",
  "",
  "- item, **bold**, `code` and ![diagram](diagram.png)",
].join("\n");

const html = markdownToSafeHtml(hostile, (src) => `/docs/raw/${src}`, purifier);
const problems = [];
const mustNot = [
  ["<script", "a <script> element"],
  ["onerror", "an onerror= attribute"],
  ["onclick", "an onclick= attribute"],
  ["javascript:", "a javascript: URL"],
  ["<iframe", "an <iframe>"],
];
for (const [needle, what] of mustNot) {
  if (html.toLowerCase().includes(needle)) problems.push(`kept ${what}`);
}
const must = [
  ["<h1", "the heading"],
  ["<table", "the table"],
  ['class="language-mermaid"', "the mermaid block, for the diagram renderer"],
  ["<strong>bold</strong>", "bold text"],
  ["<code>code</code>", "inline code"],
  ['src="/docs/raw/diagram.png"', "the resolved image"],
  ['href="https://example.com"', "the ordinary link"],
];
for (const [needle, what] of must) {
  if (!html.includes(needle)) problems.push(`lost ${what}`);
}

// A diagram: labels inside <foreignObject> survive, an injected handler does not.
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><foreignObject width="10" height="10">' +
  '<div xmlns="http://www.w3.org/1999/xhtml"><span class="nodeLabel">Claude</span></div>' +
  "</foreignObject><script>alert(1)</script></svg>";
const cleanSvg = safeSvg(svg, purifier);
if (cleanSvg.includes("onload") || cleanSvg.includes("<script")) problems.push("the SVG kept a script");
if (!cleanSvg.includes("nodeLabel")) problems.push("the SVG lost its label");

// monitor_diagrams_render_with_pele (FEAT-203 R-2): the Workflow and Gantt pages' sources, as the
// daemon serves them, and a document's block are drawn by Pele and come through the sanitiser as
// a diagram: shapes and labels kept, nothing that runs script.
const diagrams = JSON.parse(readFileSync(new URL("./diagrams-fixture.json", import.meta.url), "utf8"));
const docBlock = /<code class="language-mermaid">([\s\S]*?)<\/code>/.exec(html);
const sources = [
  ["the Workflow page", diagrams.workflow, "Business Arch"],
  ["the Gantt page", diagrams.gantt, "demo/FEAT-001"],
  ["a document's block", docBlock ? window.document.createRange().createContextualFragment(docBlock[1]).textContent : "", "B"],
];
for (const [what, source, label] of sources) {
  try {
    const drawn = safeSvg(render(source, { idPrefix: "t-" }).svg, purifier);
    if (!drawn.startsWith("<svg")) problems.push(`${what}: Pele's diagram did not survive the sanitiser`);
    if (!drawn.includes(label)) problems.push(`${what}: the diagram lost its label ${label}`);
    if (!/<(path|rect|line|polygon)\b/.test(drawn)) problems.push(`${what}: the diagram lost its shapes`);
    if (!drawn.includes("var(--pele-")) problems.push(`${what}: the diagram lost its theme colours`);
    if (/<script|\son[a-z]+=/i.test(drawn)) problems.push(`${what}: the diagram kept a script`);
  } catch (e) {
    problems.push(`${what}: Pele could not draw it — ${e.message}`);
  }
}

// Board references (FEAT-181). First, the finder agrees with the Rust one on the shared fixture.
const fixture = JSON.parse(readFileSync(new URL("./refs-fixture.json", import.meta.url), "utf8"));
const fx = fixture.index;
const index = makeIndex({
  project: fx.project,
  items: Object.entries(fx.items).map(([code, requirements]) => ({ code, requirements })),
  milestones: fx.milestones,
  adrs: Object.entries(fx.adrs).map(([id, path]) => ({ id, path })),
  goals: fx.goals,
  docs: fx.docs,
});
for (const c of fixture.cases) {
  const got = find(c.markdown, index).map((f) => [f.reference.kind, label(f.reference), f.line, f.explicit, isBroken(index, f)]);
  if (JSON.stringify(got) !== JSON.stringify(c.expect)) {
    problems.push(`refs fixture "${c.name}": expected ${JSON.stringify(c.expect)}, got ${JSON.stringify(got)}`);
  }
}

// Then: markdownHtml autolinks board references (R-1) — each kind, not inside code, existing links
// kept, foreign tokens untouched, a broken reference classed.
const linked = markdownToSafeHtml(
  [
    "FEAT-012 and FEAT-012/R-2 serve G-1 in MS-004 under ADR-0005; see architecture/data.md.",
    "",
    "`FEAT-013` stays code, and so does UTF-8.",
    "",
    "```",
    "FEAT-013 in a fence",
    "```",
    "",
    "[the cart](/p/shop/feature/FEAT-013) and [data](architecture/data.md) and [web](https://example.com).",
    "",
    "FEAT-999 is not on this board.",
  ].join("\n"),
  undefined,
  purifier,
  index,
);
const linkMust = [
  ['<a class="board-ref" href="/p/shop/feature/FEAT-012">FEAT-012</a>', "the item"],
  ['<a class="board-ref" href="/p/shop/feature/FEAT-012#R-2">FEAT-012/R-2</a>', "the requirement"],
  ['<a class="board-ref" href="/p/shop/charter#G-1">G-1</a>', "the goal"],
  ['<a class="board-ref" href="/p/shop/milestone/MS-004">MS-004</a>', "the milestone"],
  ['<a class="board-ref" href="/p/shop/docs/file/decisions/0005-carts.md">ADR-0005</a>', "the ADR"],
  ['<a class="board-ref" href="/p/shop/docs/file/architecture/data.md">architecture/data.md</a>', "the doc"],
  ["<code>FEAT-013</code>", "the code span, unlinked"],
  ["FEAT-013 in a fence", "the fence, unlinked"],
  ['<a class="board-ref" href="/p/shop/feature/FEAT-013">the cart</a>', "the explicit link, classed"],
  ['<a class="board-ref" href="/p/shop/docs/file/architecture/data.md">data</a>', "the relative doc link, routed"],
  ['<a href="https://example.com">web</a>', "the outside link, as it was"],
  ['class="board-ref broken"', "the broken reference, classed"],
];
for (const [needle, what] of linkMust) {
  if (!linked.includes(needle)) problems.push(`board refs: lost ${what}`);
}
if (/<a[^>]*>UTF-8/.test(linked)) problems.push("board refs: linked UTF-8, which is not this board's");
if (/<a[^>]*>(?:<a|[^<]*<a)/.test(linked)) problems.push("board refs: nested a link inside a link");

if (problems.length) {
  for (const p of problems) console.error(`check:markdown — ${p}`);
  console.error(html);
  console.error(linked);
  process.exit(1);
}
console.log(
  "check:markdown — scripts, handlers and javascript: URLs are removed; markdown and diagram labels render; " +
    "the Workflow, Gantt and document diagrams are drawn by Pele and sanitised; " +
    "board references link, and match the Rust finder on the shared fixture.",
);
