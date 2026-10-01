#!/usr/bin/env node
//
// The monitor's markdown is sanitised (FEAT-140): render a document carrying the usual ways to run
// script — a <script>, event-handler attributes, javascript: links, an <iframe> — through the SAME
// function the monitor uses, and fail if any survives or if ordinary markdown stops rendering.
//
// Usage: npm run check:markdown
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { markdownToSafeHtml, safeSvg } from "../src/markdownHtml.ts";

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

if (problems.length) {
  for (const p of problems) console.error(`check:markdown — ${p}`);
  console.error(html);
  process.exit(1);
}
console.log("check:markdown — scripts, handlers and javascript: URLs are removed; markdown and diagram labels render.");
