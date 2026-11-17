#!/usr/bin/env node
//
// The UI kit's tabs (FTR-046):
//   web::check_tabs_pick_requested_then_preferred_then_largest (R-1) — the opening tab, and the
//     URL state it is read from;
//   web::check_tabs_markup_is_a_tablist_with_counts (R-1) — the rendered tab list;
//   web::check_tabs_are_underlined_with_an_accent_bar (R-12) — the selected tab is an underline,
//     never an accent fill.
//
// Usage: npm run check:ui-tabs
import { readFileSync } from "node:fs";
import { pickTab, visibleTabs } from "../src/ui/tabs.ts";
import { parseState, writeState } from "../src/ui/urlState.ts";
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

// R-1: pickTab, and the URL state the requested tab comes from.
{
  const problems = [];
  const tabs = [
    { key: "a", label: "A", count: 0 },
    { key: "b", label: "B", count: 5 },
    { key: "c", label: "C", count: 9, hidden: true },
    { key: "d", label: "D", count: 5 },
    { key: "e", label: "E", count: 2, disabled: "nothing to show yet" },
  ];
  eq(problems, "a requested visible tab", pickTab(tabs, "d", "a"), "d");
  eq(problems, "a requested hidden tab falls through to the preferred", pickTab(tabs, "c", "a"), "a");
  eq(problems, "a requested absent tab falls through to the preferred", pickTab(tabs, "zz", "a"), "a");
  eq(problems, "a hidden preferred tab falls through to the largest", pickTab(tabs, null, "c"), "b");
  eq(problems, "no request and no preference: the largest non-empty (first on a tie)", pickTab(tabs, null), "b");
  eq(problems, "all empty: the first visible", pickTab([{ key: "x", label: "X", hidden: true }, { key: "y", label: "Y", count: 0 }, { key: "z", label: "Z" }], null), "y");
  eq(problems, "no tabs", pickTab([], "a", "b"), "");
  eq(problems, "only hidden tabs", pickTab([{ key: "x", label: "X", hidden: true }], "x"), "");
  // A disabled tab is never selected: requested, preferred or largest, it falls through like a hidden one.
  const withDisabled = [
    { key: "a", label: "A", count: 1 },
    { key: "b", label: "B", count: 4 },
    { key: "x", label: "X", count: 9, disabled: "not yet" },
  ];
  eq(problems, "a requested disabled tab falls through to the preferred", pickTab(withDisabled, "x", "a"), "a");
  eq(problems, "a preferred disabled tab falls through to the largest", pickTab(withDisabled, null, "x"), "b");
  eq(problems, "a disabled largest tab falls through to the largest enabled", pickTab(withDisabled, null), "b");
  eq(problems, "all empty with the first disabled: the first enabled", pickTab([{ key: "x", label: "X", disabled: "no" }, { key: "y", label: "Y" }], null), "y");
  eq(problems, "only disabled tabs", pickTab([{ key: "x", label: "X", count: 3, disabled: "no" }], "x", "x"), "");
  eq(problems, "visibleTabs drops hidden and keeps disabled", visibleTabs(tabs).map((t) => t.key), ["a", "b", "d", "e"]);

  eq(problems, "parseState reads a key and ignores unknown ones", parseState("?tab=b&x=1", { tab: "" }), { tab: "b" });
  eq(problems, "parseState with a prefix", parseState("?tsort=id&tdir=desc&sort=no", { sort: "", dir: "asc" }, "t"), { sort: "id", dir: "desc" });
  eq(problems, "parseState: an empty value is its default", parseState("?dir=", { dir: "asc" }), { dir: "asc" });
  eq(problems, "parseState: a malformed value is read, not thrown", parseState("?tab=%E0%A4%A", { tab: "" }).tab.length > 0, true);
  eq(problems, "writeState sets and keeps others", writeState("?x=1", { tab: "b" }, { tab: "" }), "?x=1&tab=b");
  eq(problems, "writeState removes a value equal to its default", writeState("?x=1&dir=desc", { dir: "asc" }, { dir: "asc" }), "?x=1");
  eq(problems, "writeState with a prefix, nothing left", writeState("?tq=foo", { q: "" }, { q: "" }, "t"), "");
  ok = report("web::check_tabs_pick_requested_then_preferred_then_largest", problems) && ok;
}

// R-1: the rendered markup.
try {
  const problems = [];
  const tabs = [
    { key: "open", label: "Open", count: 3 },
    { key: "done", label: "Done", count: 12 },
    { key: "secret", label: "Secret", count: 1, hidden: true },
    { key: "later", label: "Later", disabled: "Nothing is planned yet" },
  ];
  const props = { tabs, label: "Items", children: (key) => `panel for ${key}` };
  const { html, document } = await render("src/ui/Tabs.tsx", "Tabs", props, { url: "/p/x?tab=open" });
  const lists = document.querySelectorAll('[role="tablist"]');
  if (lists.length !== 1) problems.push(`${lists.length} role=tablist, expected 1: ${html}`);
  else if (lists[0].getAttribute("aria-label") !== "Items") problems.push(`the tablist's aria-label is ${lists[0].getAttribute("aria-label")}, expected "Items"`);
  const tabEls = [...document.querySelectorAll('[role="tab"]')];
  eq(problems, "one role=tab per visible tab", tabEls.map((t) => t.textContent), ["Open 3", "Done 12", "Later"]);
  if (tabEls.some((t) => t.tagName !== "BUTTON")) problems.push("every tab is a button");
  if (/Secret/.test(html)) problems.push("a hidden tab is rendered");
  eq(problems, "aria-selected follows ?tab=open", tabEls.map((t) => t.getAttribute("aria-selected")), ["true", "false", "false"]);
  const counts = [...document.querySelectorAll('[role="tab"] .ui-count')].map((c) => c.textContent);
  eq(problems, "counts are in .ui-count, none for an undefined count", counts, ["3", "12"]);
  const later = tabEls[2];
  if (later && later.getAttribute("aria-disabled") !== "true") problems.push("a disabled tab has no aria-disabled");
  if (later && later.getAttribute("title") !== "Nothing is planned yet") problems.push(`a disabled tab's title is ${later?.getAttribute("title")}, expected its reason`);
  const panels = document.querySelectorAll('[role="tabpanel"]');
  if (panels.length !== 1) problems.push(`${panels.length} role=tabpanel, expected 1 (only the selected one)`);
  else {
    const panel = panels[0];
    const sel = tabEls[0];
    if (panel.textContent !== "panel for open") problems.push(`the panel shows "${panel.textContent}", expected "panel for open"`);
    if (!sel || panel.getAttribute("aria-labelledby") !== sel.id) problems.push("the panel's aria-labelledby is not the selected tab's id");
    if (!sel || sel.getAttribute("aria-controls") !== panel.id) problems.push("the selected tab's aria-controls is not the panel's id");
  }

  // With no tab in the URL, the largest non-empty opens; a custom `param` is read.
  const largest = await render("src/ui/Tabs.tsx", "Tabs", props, { url: "/" });
  const picked = [...largest.document.querySelectorAll('[role="tab"][aria-selected="true"]')].map((t) => t.textContent);
  eq(problems, "with no ?tab the largest non-empty is selected", picked, ["Done 12"]);
  const custom = await render("src/ui/Tabs.tsx", "Tabs", { ...props, param: "view" }, { url: "/?tab=done&view=open" });
  const pickedCustom = [...custom.document.querySelectorAll('[role="tab"][aria-selected="true"]')].map((t) => t.textContent);
  eq(problems, "param=view reads ?view", pickedCustom, ["Open 3"]);

  // Every tab disabled: none is selected and no panel shows, but the first keeps tabIndex=0 so the
  // tab list stays reachable by keyboard.
  const allOff = await render(
    "src/ui/Tabs.tsx",
    "Tabs",
    { tabs: [{ key: "a", label: "A", disabled: "not yet" }, { key: "b", label: "B", disabled: "not yet" }], label: "Off", children: (key) => `panel for ${key}` },
    { url: "/?tab=a" },
  );
  const offTabs = [...allOff.document.querySelectorAll('[role="tab"]')];
  eq(problems, "every tab disabled: the first keeps tabIndex=0", offTabs.map((t) => t.getAttribute("tabindex")), ["0", "-1"]);
  eq(problems, "every tab disabled: none is selected", offTabs.map((t) => t.getAttribute("aria-selected")), ["false", "false"]);
  eq(problems, "every tab disabled: each has aria-disabled", offTabs.map((t) => t.getAttribute("aria-disabled")), ["true", "true"]);
  eq(problems, "every tab disabled: no panel", allOff.document.querySelectorAll('[role="tabpanel"]').length, 0);
  ok = report("web::check_tabs_markup_is_a_tablist_with_counts", problems) && ok;
} catch (e) {
  ok = report("web::check_tabs_markup_is_a_tablist_with_counts", [`render failed: ${e?.stack ?? e}`]) && ok;
} finally {
  await closeRender();
}

// R-12: underlined tabs, never an accent fill.
{
  const problems = [];
  const css = readFileSync(new URL("../src/ui/ui.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const rules = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({ selector: m[1].trim().replace(/\s+/g, " "), body: m[2] }));
  const tabRules = rules.filter((r) => /\.ui-tab\b/.test(r.selector));
  const selectedRule = tabRules.find((r) => r.selector.split(",").some((s) => s.trim() === '.ui-tab[aria-selected="true"]'));
  if (!selectedRule) problems.push('ui.css has no .ui-tab[aria-selected="true"] rule');
  else if (!/box-shadow\s*:\s*inset\s+0\s+-2px\s+0\s+var\(--accent\)/.test(selectedRule.body)) {
    problems.push(`the selected-tab rule does not draw a 2 px --accent bar: ${selectedRule.body.trim()}`);
  }
  const baseRule = tabRules.find((r) => r.selector.split(",").some((s) => s.trim() === ".ui-tab"));
  if (!baseRule) problems.push("ui.css has no .ui-tab rule");
  else {
    if (!/background(-color)?\s*:\s*transparent/.test(baseRule.body)) problems.push(".ui-tab's background is not transparent");
    if (!/(^|[;\s])color\s*:\s*var\(--text\)/.test(baseRule.body)) problems.push(".ui-tab's text is not var(--text)");
  }
  for (const r of tabRules) {
    for (const m of r.body.matchAll(/background(?:-color)?\s*:\s*([^;]+)/g)) {
      if (/var\(--(accent|accent-strong|on-accent|accent-2)\)/.test(m[1])) problems.push(`${r.selector} sets background to ${m[1].trim()} — tabs are underlined, not filled`);
    }
  }
  ok = report("web::check_tabs_are_underlined_with_an_accent_bar", problems) && ok;
}

if (!ok) process.exit(1);
