#!/usr/bin/env node
//
// FTR-076: the Charter page, checked without a browser. Each check prints its name.
//   web::check_charter_since_last_visit_counts_what_changed (R-7) — the pure half: sinceLastVisit
//     counts the finished items serving a goal, the releases shipped and the lessons recorded after
//     the time this browser last looked, gives each count a link, and answers null for a first
//     visit, an unreadable time and no change. The render half: the page says that line, its counts
//     linking to their lists, and says nothing on a first visit or when nothing changed.
//   web::check_charter_model_rows_routes_and_cells (R-2, R-4) — goalRows links each goal's items and
//     proven counts and its lessons, flags an unserved goal; routeSteps draws shipped stops done and
//     the next one with its target or "no date set"; matrixCell links a non-zero cell to its items.
//   web::check_charter_model_series_themes_and_folds (R-5, R-6) — seriesFor and sharedMax, themeRows
//     with its kinds line, fold, and the page's constants.
//
//   web::check_charter_page_shows_every_charter_field (R-1) — CharterView over a fixture charter with
//     every field and lessons with every field: each value is in the markup; a 400-character
//     constraint is whole inside a <details>.
//   web::check_charter_goal_rows_link_to_their_items (R-2) — each goal's Items cell links to its items,
//     each Proven bar to the proven ones; a goal no item serves says so in words.
//   web::check_charter_goal_rows_show_their_lessons (R-7) — a goal's Lessons cell counts the lessons
//     that name it and links to them, and each lesson shows its goals.
//   web::check_charter_lists_are_tables_with_nothing_dropped (R-3) — non-goals, stakeholders and
//     constraints are captioned tables with one row per entry.
//   web::check_charter_fresh_project_says_so_plainly (R-9) — an empty project: every empty section says
//     what is missing in a sentence and no chart is drawn.
//   web::check_charter_charts_have_tables_and_links (R-10) — every chart has its numbers in a table,
//     every count is a link, chart colours are accent only.
//   web::check_charter_says_when_it_was_read (R-10) — "Read HH:MM from board 1234567".
//
// Usage: npm run check:charter-page
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { closeRender, render } from "./lib/render.mjs";
import { FOLD_ENTRY, FOLD_STATEMENT, SEEN_AFTER_MS, fold, goalRows, matrixCell, routeSteps, seenKey, seriesFor, sharedMax, sinceLastVisit, themeRows, todayLine } from "../src/charterModel.ts";

let failed = false;
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);

/** Run one named check: its problems, or one line saying what it proved. */
async function check(name, proved, body) {
  const problems = [];
  const expect = (what, got, want) => {
    if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  };
  try {
    await body(expect, problems);
  } catch (e) {
    problems.push(`threw: ${e?.stack ?? e}`);
  }
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else console.log(`${name} — ${proved}`);
}

const lesson = (id, at, kind, tags, goals = []) => ({
  id,
  lesson: `Lesson ${id}`,
  kind,
  status: "active",
  at,
  confidence_now: 0.8,
  in_force: true,
  tags,
  goals,
  affirmed: 0,
  contradicted: 0,
});

const progress = {
  project: "x",
  generated_at: "2026-11-17T10:00:00Z",
  releases_on: true,
  goals: [
    { id: "G-1", title: "Pay", items_done: 3, items_open: 1, req_proven: 5, req_total: 8, lessons: ["L-1", "L-2"] },
    { id: "G-2", title: "Cart", items_done: 0, items_open: 0, req_proven: 0, req_total: 0 },
  ],
  route: [
    { version: "v0.6.0", state: "shipped", shipped_at: "2026-10-01T10:00:00Z", items: 2 },
    { version: "v0.7.0", state: "shipped", shipped_at: "2026-11-14T10:00:00Z", items: 3 },
    { version: "v0.8.0", state: "next", target: "2026-12-18", items: 1 },
  ],
  marks: [
    { goal: "G-1", release: "v0.6.0", items: 1 },
    { goal: "G-1", release: "v0.7.0", items: 2 },
  ],
  series: [
    {
      goal: "G-1",
      points: [
        { release: "v0.6.0", items: 1, req_proven: 2, req_total: 3 },
        { release: "v0.7.0", items: 3, req_proven: 5, req_total: 8 },
      ],
    },
  ],
  lessons: {
    in_force: 3,
    total: 3,
    retired: 0,
    themes: [
      { key: "testing", label: "testing", lessons: ["L-1", "L-2", "L-3"] },
      { key: "untagged", label: "untagged", lessons: [] },
    ],
    rows: [
      lesson("L-1", "2026-10-02T00:00:00Z", "pitfall", ["testing"], ["G-1"]),
      lesson("L-2", "2026-11-15T00:00:00Z", "pitfall", ["testing"], ["G-1"]),
      lesson("L-3", "2026-11-16T00:00:00Z", "practice", ["testing"]),
    ],
  },
  finished: [
    { code: "FTR-040", at: "2026-11-10T00:00:00Z", goals: ["G-1"] },
    { code: "FTR-041", at: "2026-11-15T00:00:00Z", goals: ["G-1", "G-2"] },
    { code: "FTR-042", at: "2026-11-16T00:00:00Z", goals: ["G-2"] },
  ],
};
const charter = {
  purpose: "Sell",
  goals: [
    { id: "G-1", statement: "Customers can pay", measure: "orders", horizon: "2026" },
    { id: "G-2", statement: "x".repeat(200) },
  ],
};

await check("web::check_charter_model_rows_routes_and_cells", "goal rows, route steps and matrix cells link to their lists", (expect) => {
  const rows = goalRows(progress, charter);
  expect("ids", rows.map((r) => r.id), ["G-1", "G-2"]);
  expect("G-1 links", [rows[0].itemsTo, rows[0].provenTo], ["/p/x/items?goal=G-1", "/p/x/items?goal=G-1&evidence=proven"]);
  expect("G-1 counts", [rows[0].itemsDone, rows[0].itemsOpen, rows[0].proven, rows[0].total, rows[0].unserved], [3, 1, 5, 8, false]);
  expect("G-1 lessons", [rows[0].lessons, rows[0].lessonThemes], [["L-1", "L-2"], ["testing"]]);
  expect("G-2 unserved", rows[1].unserved, true);
  expect("G-2 folds", [rows[1].head.length <= FOLD_STATEMENT + 1, rows[1].rest], [true, "x".repeat(200)]);
  expect("no progress", goalRows(null, charter).map((r) => r.unserved), [true, true]);
  expect(
    "steps",
    routeSteps(progress),
    [
      { key: "v0.6.0", label: "v0.6.0", state: "done", at: "2026-10-01T10:00:00Z" },
      { key: "v0.7.0", label: "v0.7.0", state: "done", at: "2026-11-14T10:00:00Z" },
      { key: "v0.8.0", label: "v0.8.0 · target 2026-12-18", state: "next" },
    ],
  );
  const undated = { ...progress, route: [{ version: "v0.8.0", state: "next", items: 0 }] };
  expect("undated", routeSteps(undated)[0].label, "v0.8.0 · no date set");
  expect("today", todayLine(progress), "Today: after v0.7.0, working toward v0.8.0.");
  expect("cell", matrixCell(progress, "G-1", "v0.7.0"), { value: 2, to: "/p/x/items?goal=G-1&release=v0.7.0" });
  expect("empty cell", matrixCell(progress, "G-2", "v0.7.0"), { value: 0 });
});

await check("web::check_charter_model_series_themes_and_folds", "series, shared maximum, themes, folding and constants", (expect) => {
  expect("series", seriesFor(progress, "G-1").map((p) => p.req_proven), [2, 5]);
  expect("no series", seriesFor(progress, "G-2"), []);
  expect("max", sharedMax(progress), 8);
  expect("max floor", sharedMax({ ...progress, series: [] }), 1);
  const themes = themeRows(progress);
  expect("themes", themes.map((t) => [t.label, t.count, t.kinds]), [["testing", 3, "2 pitfalls, 1 practice"], ["untagged", 0, ""]]);
  expect("short", fold("short", 10), { head: "short", rest: "" });
  expect("first line", fold("one\ntwo", 10), { head: "one", rest: "two" });
  expect("long", fold("a".repeat(30), 10).rest, "a".repeat(30));
  expect("constants", [FOLD_STATEMENT, FOLD_ENTRY, SEEN_AFTER_MS, seenKey("x")], [120, 160, 10000, "kanbanr-charter-seen:x"]);
});

// ---- The rendered page (T7) ----
// The purpose is Markdown, sanitised by DOMPurify, which needs a window to bind to; give it one
// before the first render loads the page (a dependency is imported once). React's DOM build must
// load first, without a window, so a trivial render goes before it.
await render("src/ui/States.tsx", "LoadingState", {});
globalThis.window ??= new JSDOM("<!DOCTYPE html>").window;
const PAGE = "src/pages/CharterPage.tsx";
const longConstraint = `Must stay local-first ${"and plain-text ".repeat(26)}end.`;
const fullCharter = {
  purpose: "Keep the plan where Claude can read it",
  vision: "Every session continues the plan with no recap",
  goals: [
    { id: "G-1", statement: "A session recovers the full plan", measure: "zero recaps", horizon: "2026" },
    { id: "G-2", statement: "Every item records why it exists", measure: "reasons on file", horizon: "v1.0" },
    { id: "G-3", statement: "Nothing built before agreed" },
  ],
  non_goals: ["An enterprise PM platform", "Two-way sync"],
  stakeholders: [
    { name: "The developer", role: "owner", interest: "keeps the plan across sessions" },
    { name: "Claude", role: "agent", interest: "reads and records the plan" },
  ],
  constraints: ["Local-first", longConstraint],
  adopted_at: "2026-09-14T08:00:00Z",
};
const fullLesson = (id, o) => ({
  ...lesson(id, "2026-11-02T00:00:00Z", "pitfall", ["testing"], ["G-1"]),
  confidence_now: 0.73,
  from_item: "FTR-011",
  from_retro: "SP-002",
  retro_kind: "sprint",
  evidence: `Evidence for ${id}`,
  affirmed: 2,
  contradicted: 1,
  cited_by: ["FTR-020", "FTR-021"],
  ...o,
});
const fullProgress = {
  ...progress,
  board_rev: "1234567890abcdef",
  goals: [
    { id: "G-1", title: "A session recovers the full plan", items_done: 3, items_open: 1, req_proven: 5, req_total: 8, lessons: ["L-1", "L-2"] },
    { id: "G-2", title: "Every item records why", items_done: 0, items_open: 0, req_proven: 0, req_total: 0 },
    { id: "G-3", title: "Nothing built", items_done: 1, items_open: 0, req_proven: 1, req_total: 1 },
  ],
  lessons: {
    in_force: 2,
    total: 3,
    retired: 1,
    themes: [{ key: "testing", label: "testing", lessons: ["L-1", "L-2", "L-3"] }],
    cycles: [
      { key: "v0.6.0", label: "v0.6.0", recorded: 1, cumulative: 1 },
      { key: "v0.7.0", label: "v0.7.0", recorded: 2, cumulative: 3 },
    ],
    rows: [
      fullLesson("L-1", {}),
      fullLesson("L-2", { at: "2026-11-15T00:00:00Z", kind: "practice", tags: ["testing", "docs"], from_item: "", from_retro: "SP-002" }),
      fullLesson("L-3", { at: "2026-11-16T00:00:00Z", status: "retired", in_force: false, goals: [], from_retro: "MS-001", retro_kind: "milestone" }),
    ],
  },
  wins: [{ code: "FTR-041", title: "The Releases page", at: "2026-11-16T10:00:00Z", goals: ["G-1", "G-3"], release: "v0.7.0" }],
};
const open = "/p/x/charter#open=theme:testing";
const view = (props, url = open) => render(PAGE, "CharterView", { project: "x", charter: fullCharter, progress: fullProgress, seen: null, ...props }, { url });
const cell = (document, text) => [...document.querySelectorAll("td, th")].find((c) => c.textContent.trim().startsWith(text));
const hrefs = (el) => [...el.querySelectorAll("a")].map((a) => a.getAttribute("href"));

await check("web::check_charter_page_shows_every_charter_field", "every charter field and every lesson field is on the page, long entries whole in a <details>", async (expect, problems) => {
  const { html, document } = await view({});
  const want = [
    fullCharter.purpose, fullCharter.vision, "2026-09-14",
    ...fullCharter.goals.flatMap((g) => [g.id, g.statement, g.measure ?? "", g.horizon ?? ""]),
    ...fullCharter.non_goals,
    ...fullCharter.stakeholders.flatMap((s) => [s.name, s.role, s.interest]),
    "Local-first",
    "Evidence for L-1", "FTR-011", "SP-002", "MS-001", "pitfall", "practice", "73%", "docs", "retired", "FTR-020", "FTR-021",
  ].filter((t) => t !== "");
  for (const t of want) if (!html.includes(t)) problems.push(`missing on the page: ${t}`);
  expect("one h1", document.querySelectorAll("h1").length, 1);
  const inDetails = [...document.querySelectorAll("details")].some((d) => d.textContent.includes(longConstraint));
  expect("long constraint whole in a details", inDetails, true);
  expect("lessons section keeps its anchor", document.getElementById("lessons") !== null, true);
});

await check("web::check_charter_goal_rows_link_to_their_items", "each goal's Items cell links to its items and its Proven bar to the proven ones; an unserved goal says so", async (expect) => {
  const { document } = await view({});
  const g1 = cell(document, "G-1").closest("tr");
  expect("G-1 links", hrefs(g1).filter((h) => h.startsWith("/p/x/items")), ["/p/x/items?goal=G-1", "/p/x/items?goal=G-1&evidence=proven"]);
  expect("G-1 words", g1.textContent.includes("3 done") && g1.textContent.includes("1 open"), true);
  const g2 = cell(document, "G-2").closest("tr");
  expect("G-2 says unserved", g2.textContent.includes("no item serves this goal yet"), true);
  expect("G-2 has no items link", hrefs(g2).filter((h) => h.startsWith("/p/x/items")), []);
});

await check("web::check_charter_goal_rows_show_their_lessons", "a goal's Lessons cell counts and links the lessons naming it; each lesson shows its goals", async (expect) => {
  const { document } = await view({});
  const g1 = cell(document, "G-1").closest("tr");
  expect("count and link", [g1.textContent.includes("2 lessons"), hrefs(g1).includes("/p/x/charter#lessons")], [true, true]);
  expect("themes on hover", [...g1.querySelectorAll("[title]")].some((e) => e.getAttribute("title").includes("testing")), true);
  const l1 = cell(document, "L-1").closest("tr");
  expect("lesson goals", l1.textContent.includes("G-1"), true);
});

await check("web::check_charter_lists_are_tables_with_nothing_dropped", "non-goals, stakeholders and constraints are captioned tables, one row per entry", async (expect) => {
  const { document } = await view({});
  const table = (cap) => [...document.querySelectorAll("table")].find((t) => t.querySelector("caption")?.textContent.trim() === cap);
  for (const [cap, n] of [["Non-goals", 2], ["Stakeholders", 2], ["Constraints", 2]]) {
    const t = table(cap);
    expect(`${cap} rows`, t ? t.querySelectorAll("tbody tr").length : "no table", n);
  }
  const s = table("Stakeholders");
  const row = s?.querySelector("tbody tr")?.textContent ?? "";
  expect("stakeholder fields", ["The developer", "owner", "keeps the plan across sessions"].every((t) => row.includes(t)), true);
});

await check(
  "web::check_charter_since_last_visit_counts_what_changed",
  "counts items, goals, releases and lessons after the last visit and draws them as one line of links; null for a first visit, a bad time and no change",
  async (expect) => {
    const s = sinceLastVisit(progress, "2026-11-12T08:00:00Z");
    expect("text", s?.text, "Since 12 Nov: 2 items finished toward G-1 and G-2, v0.7.0 shipped, 2 lessons recorded.");
    expect(
      "links",
      s?.links,
      [
        { label: "2 items", to: "/p/x/items" },
        { label: "G-1", to: "/p/x/items?goal=G-1" },
        { label: "G-2", to: "/p/x/items?goal=G-2" },
        { label: "v0.7.0", to: "/p/x/releases?tab=current#v0.7.0" },
        { label: "2 lessons", to: "/p/x/charter#lessons" },
      ],
    );
    expect("first visit", sinceLastVisit(progress, null), null);
    expect("unreadable", sinceLastVisit(progress, "yesterday"), null);
    expect("no change", sinceLastVisit(progress, "2026-11-17T09:00:00Z"), null);
    const one = sinceLastVisit(progress, "2026-11-15T12:00:00Z");
    expect("singular", one?.text, "Since 15 Nov: 1 item finished toward G-2, 1 lesson recorded.");

    // The rendered half.
  const seen = await view({ seen: "2026-11-12T08:00:00Z" });
  const line = seen.document.querySelector(".charter-since");
  expect("text", line?.textContent.trim(), "Since 12 Nov: 2 items finished toward G-1 and G-2, v0.7.0 shipped, 2 lessons recorded.");
  expect("links", line ? hrefs(line) : [], ["/p/x/items", "/p/x/items?goal=G-1", "/p/x/items?goal=G-2", "/p/x/releases?tab=current#v0.7.0", "/p/x/charter#lessons"]);
  expect("first visit", (await view({ seen: null })).document.querySelector(".charter-since"), null);
  expect("no change", (await view({ seen: "2026-11-17T09:00:00Z" })).document.querySelector(".charter-since"), null);
  },
);

await check("web::check_charter_fresh_project_says_so_plainly", "an empty project answers each empty section in a sentence and draws no chart", async (expect, problems) => {
  const emptyProgress = { project: "x", generated_at: "2026-11-17T10:00:00Z", releases_on: false, goals: [], lessons: { in_force: 0, total: 0, retired: 0 } };
  const { document, html } = await view({ charter: { purpose: "", goals: [] }, progress: emptyProgress }, "/p/x/charter");
  expect("no svg", document.querySelectorAll("svg").length, 0);
  for (const t of [
    "No charter yet. Ask Claude to write the project's charter: its purpose, goals and what it will not do.",
    "No goals yet: items have nothing to serve.",
    "This project does not ship releases, so there is no line of releases to draw. Finished work shows under Recent wins.",
    "No lessons recorded yet. Ask Claude to record one when something teaches the project.",
    "Nothing that serves a goal has finished yet.",
  ]) if (!html.includes(t.replace(/'/g, "&#x27;")) && !html.includes(t)) problems.push(`missing sentence: ${t}`);
  const shipping = await view({ charter: fullCharter, progress: { ...emptyProgress, releases_on: true, goals: [], route: [{ version: "v0.1.0", state: "next", items: 0 }] } }, "/p/x/charter");
  expect("nothing shipped", shipping.html.includes("Nothing has shipped yet. The first release will start the line."), true);
  const old = await view({ progress: null }, "/p/x/charter");
  expect("older daemon", old.html.includes("This kanbanr does not report progress yet: update it to see the way travelled"), true);
});

await check("web::check_charter_charts_have_tables_and_links", "every chart has its numbers in a table, every count is a link, marks use only accent", async (expect, problems) => {
  const { document } = await view({});
  const charts = [...document.querySelectorAll("svg[role=img]")];
  if (charts.length < 2) problems.push(`expected a chart per goal with points and the lessons chart, found ${charts.length}`);
  for (const svg of charts) {
    const fig = svg.closest("figure");
    const cells = fig ? [...fig.querySelectorAll("table td")].map((c) => c.textContent.trim()) : [];
    if (cells.length === 0) problems.push(`chart "${svg.getAttribute("aria-label")}" has no table view`);
  }
  const g1 = cell(document, "G-1").closest("tr");
  for (const c of g1.querySelectorAll("td")) {
    if (/\d/.test(c.textContent) && !c.querySelector("a") && !c.textContent.includes("G-1") && !c.textContent.includes("2026")) problems.push(`goal cell with a number and no link: ${c.textContent.trim()}`);
  }
  const m = [...document.querySelectorAll("table.ui-matrix td a")].map((a) => a.getAttribute("href"));
  expect("matrix links", m, ["/p/x/items?goal=G-1&release=v0.6.0", "/p/x/items?goal=G-1&release=v0.7.0"]);
  const wins = document.querySelector("#recent-wins");
  expect("win links", wins ? hrefs(wins) : [], ["/p/x/feature/FTR-041"]);
  const css = readFileSync(new URL("../src/ui/ui.css", import.meta.url), "utf8") + readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
  const bad = (css.match(/\.charter-[^{]*\{[^}]*\}/g) ?? []).filter((r) => /#[0-9a-f]{3,6}\b|rgb\(/i.test(r));
  expect("charter css uses tokens", bad, []);
});

await check("web::check_charter_says_when_it_was_read", "the page says when it was read and from which board revision", async (expect) => {
  const { document } = await view({});
  const d = new Date(fullProgress.generated_at);
  const hhmm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  expect("read line", document.querySelector(".charter-read")?.textContent.trim(), `Read ${hhmm} from board 1234567`);
});

await closeRender();
if (failed) process.exit(1);
