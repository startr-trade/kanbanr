#!/usr/bin/env node
//
// FTR-076: the Charter page, checked without a browser. Each check prints its name.
//   web::check_charter_since_last_visit_counts_what_changed (R-7) — the pure half: sinceLastVisit
//     counts the finished items serving a goal, the releases shipped and the lessons recorded after
//     the time this browser last looked, gives each count a link, and answers null for a first
//     visit, an unreadable time and no change. (The render half joins this file with the page.)
//   web::check_charter_model_rows_routes_and_cells (R-2, R-4) — goalRows links each goal's items and
//     proven counts and its lessons, flags an unserved goal; routeSteps draws shipped stops done and
//     the next one with its target or "no date set"; matrixCell links a non-zero cell to its items.
//   web::check_charter_model_series_themes_and_folds (R-5, R-6) — seriesFor and sharedMax, themeRows
//     with its kinds line, fold, and the page's constants.
//
// Usage: npm run check:charter-page
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

await check(
  "web::check_charter_since_last_visit_counts_what_changed",
  "counts items, goals, releases and lessons after the last visit; null for a first visit, a bad time and no change",
  (expect) => {
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
  },
);

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

if (failed) process.exit(1);
