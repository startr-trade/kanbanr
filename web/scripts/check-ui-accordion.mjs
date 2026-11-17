#!/usr/bin/env node
//
// The UI kit's accordion row (FTR-046):
//   web::check_accordion_body_renders_only_when_open (R-2) — `children` is called only while the
//     row is open, so a page fetches the body on demand;
//   web::check_accordion_header_is_a_heading_with_aria_expanded (R-2) — the header is a heading
//     holding the toggle button, a disabled select shows its reason as text, and the open ids are
//     read from and written to the hash.
//
// Usage: npm run check:ui-accordion
import { parseOpen, writeOpen } from "../src/ui/urlState.ts";
import { closeRender, render } from "./lib/render.mjs";

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
};

let ok = true;

try {
  // R-2: the body renders only when open.
  {
    const name = "web::check_accordion_body_renders_only_when_open";
    const problems = [];
    let calls = 0;
    const children = () => {
      calls += 1;
      return "the row's evidence";
    };
    const closed = await render("src/ui/AccordionRow.tsx", "AccordionRow", { id: "DEF-066", title: "DEF-066 A row", open: false, children });
    eq(problems, "children calls while closed", calls, 0);
    const closedBody = closed.document.getElementById("DEF-066-body");
    if (!closedBody) problems.push(`no #DEF-066-body: ${closed.html}`);
    else {
      if (!closedBody.hasAttribute("hidden")) problems.push("the closed body is not hidden");
      if (closedBody.textContent !== "") problems.push(`the closed body holds "${closedBody.textContent}"`);
    }

    calls = 0;
    const opened = await render("src/ui/AccordionRow.tsx", "AccordionRow", { id: "DEF-066", title: "DEF-066 A row", open: true, children });
    eq(problems, "children calls while open", calls, 1);
    const openBody = opened.document.getElementById("DEF-066-body");
    if (!openBody) problems.push(`no #DEF-066-body: ${opened.html}`);
    else {
      if (openBody.hasAttribute("hidden")) problems.push("the open body is hidden");
      eq(problems, "the open body's text", openBody.textContent, "the row's evidence");
    }

    // Without `open` the row keeps its own state and starts closed.
    calls = 0;
    await render("src/ui/AccordionRow.tsx", "AccordionRow", { id: "x", title: "X", children });
    eq(problems, "children calls for an uncontrolled row (starts closed)", calls, 0);
    ok = report(name, problems) && ok;
  }

  // R-2: the header is a heading with aria-expanded; the select; the hash list.
  {
    const name = "web::check_accordion_header_is_a_heading_with_aria_expanded";
    const problems = [];
    const props = {
      id: "v0.8.0",
      title: "v0.8.0 The monitor",
      meta: "3 items",
      actions: "Promote",
      open: true,
      children: () => "body",
    };
    const { html, document } = await render("src/ui/AccordionRow.tsx", "AccordionRow", props);
    const section = document.querySelector("section.ui-row");
    if (!section) problems.push(`no section.ui-row: ${html}`);
    else if (section.id !== "v0.8.0") problems.push(`the section's id is "${section.id}", expected "v0.8.0"`);
    const button = document.querySelector("section.ui-row .ui-row-head > h3 > button[aria-expanded]");
    if (!button) problems.push(`no h3 > button[aria-expanded] in the header: ${html}`);
    else {
      eq(problems, "aria-expanded when open", button.getAttribute("aria-expanded"), "true");
      eq(problems, "the button's text is the title", button.textContent, "v0.8.0 The monitor");
      eq(problems, "aria-controls names the body", button.getAttribute("aria-controls"), "v0.8.0-body");
      if (button.getAttribute("type") !== "button") problems.push("the toggle is not type=button");
    }
    if (!document.getElementById("v0.8.0-body")) problems.push("aria-controls names no element");
    const meta = document.querySelector(".ui-row-head .ui-row-meta");
    if (!meta || meta.textContent !== "3 items") problems.push("meta is not on the header line");
    const actions = document.querySelector(".ui-row-head .ui-row-actions");
    if (!actions || actions.textContent !== "Promote") problems.push("actions are not on the header line");

    const closed = await render("src/ui/AccordionRow.tsx", "AccordionRow", { ...props, open: false, level: 2 });
    const closedButton = closed.document.querySelector("h2 > button[aria-expanded]");
    if (!closedButton) problems.push(`level 2 gives no h2 > button: ${closed.html}`);
    else eq(problems, "aria-expanded when closed", closedButton.getAttribute("aria-expanded"), "false");

    // A select is a checkbox before the heading, labelled by the title; a disabled one shows why.
    const selected = await render("src/ui/AccordionRow.tsx", "AccordionRow", {
      ...props,
      select: { checked: true, onChange: () => {} },
    });
    const box = selected.document.querySelector('.ui-row-head input[type="checkbox"]');
    if (!box) problems.push(`select renders no checkbox: ${selected.html}`);
    else {
      if (!box.hasAttribute("checked")) problems.push("the checkbox is not checked");
      if (box.hasAttribute("disabled")) problems.push("an enabled select is disabled");
      const label = selected.document.getElementById(box.getAttribute("aria-labelledby") ?? "");
      if (!label || label.textContent !== "v0.8.0 The monitor") problems.push("the checkbox is not labelled by the title");
      const heading = selected.document.querySelector(".ui-row-head h3");
      if (heading && !(box.compareDocumentPosition(heading) & 4 /* FOLLOWING */)) problems.push("the checkbox is not before the heading");
    }
    const reason = "Prepare its evidence first";
    const blocked = await render("src/ui/AccordionRow.tsx", "AccordionRow", {
      ...props,
      select: { checked: false, disabled: reason, onChange: () => {} },
    });
    const blockedBox = blocked.document.querySelector('.ui-row-head input[type="checkbox"]');
    if (!blockedBox) problems.push("a disabled select renders no checkbox");
    else {
      if (!blockedBox.hasAttribute("disabled")) problems.push("a disabled select's checkbox has no disabled");
      const why = blocked.document.getElementById(blockedBox.getAttribute("aria-describedby") ?? "");
      if (!why || why.textContent !== reason) problems.push("the disabled checkbox is not described by its reason");
    }
    const shown = [...blocked.document.querySelectorAll(".ui-row-head *")].some((el) => el.children.length === 0 && el.textContent === reason);
    if (!shown) problems.push(`the disabled reason is not shown as text: ${blocked.html}`);

    // The open ids in the hash.
    eq(problems, 'parseOpen("#DEF-066")', [...parseOpen("#DEF-066")], ["DEF-066"]);
    eq(problems, 'parseOpen("#open=a,b")', [...parseOpen("#open=a,b")], ["a", "b"]);
    eq(problems, 'parseOpen("#DEF-066,FTR-041") splits a plain hash on commas', [...parseOpen("#DEF-066,FTR-041")], ["DEF-066", "FTR-041"]);
    eq(problems, 'parseOpen("#open=v0.8.0,v0.7.0")', [...parseOpen("#open=v0.8.0,v0.7.0")], ["v0.8.0", "v0.7.0"]);
    eq(problems, 'parseOpen("")', [...parseOpen("")], []);
    eq(problems, 'parseOpen("#")', [...parseOpen("#")], []);
    eq(problems, 'parseOpen("#open=")', [...parseOpen("#open=")], []);
    eq(problems, "parseOpen of another key gives no open rows", [...parseOpen("#lanes=a")], []);
    eq(problems, "parseOpen with a param", [...parseOpen("#open=a&lanes=x,y", "lanes")], ["x", "y"]);
    eq(problems, "parseOpen: a malformed escape is kept as written", [...parseOpen("#open=a%E0%A4%A")], ["a%E0%A4%A"]);
    eq(problems, "writeOpen sets the list", writeOpen("", new Set(["v0.8.0", "v0.7.0"])), "#open=v0.8.0,v0.7.0");
    eq(problems, "writeOpen replaces a plain anchor", writeOpen("#DEF-066", new Set(["DEF-066", "FTR-041"])), "#open=DEF-066,FTR-041");
    eq(problems, "writeOpen of nothing is empty", writeOpen("#open=a", new Set()), "");
    eq(problems, "writeOpen keeps another list", writeOpen("#lanes=x&open=a", new Set(["b"])), "#lanes=x&open=b");
    eq(problems, "writeOpen keeps ':' readable", writeOpen("", new Set(["sprint:SP-004"])), "#open=sprint:SP-004");
    const odd = new Set(["a,b", "c&d", "e f"]);
    eq(problems, "writeOpen then parseOpen round-trips ids with , & and spaces", [...parseOpen(writeOpen("", odd))], [...odd]);
    ok = report(name, problems) && ok;
  }
} catch (e) {
  ok = report("web::check_accordion_header_is_a_heading_with_aria_expanded", [`render failed: ${e?.stack ?? e}`]) && ok;
} finally {
  await closeRender();
}

if (!ok) process.exit(1);
