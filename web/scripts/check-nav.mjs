#!/usr/bin/env node
//
// FTR-047: the monitor's grouped navigation, checked without React or a browser.
//   web::check_nav_lists_every_page_route_once — every project page route in main.tsx is the
//     path of exactly one NAV entry, and no two entries share a path or a key (R-1);
//   web::check_nav_hides_releases_without_cadence_and_empty_areas — Releases shows only once the
//     index says the project uses releases, and an area with no entry is not shown (R-1);
//   web::check_nav_marks_the_active_area — an item, milestone, status or document page marks the
//     entry it was reached from, compared on whole path segments (R-2);
//   web::check_nav_menus_open_one_at_a_time_and_close_on_escape — the menu reducer (R-3);
//   web::check_shell_links_are_not_duplicated — no path twice in NAV plus the global nav (R-5);
//   web::check_nav_puts_review_second_under_work — Work lists Board, then Review (R-7).
//
// Usage: npm run check:nav
import { readFileSync } from "node:fs";
import { NAV, activeEntry, visibleAreas } from "../src/nav.ts";
import { navReducer } from "../src/navState.ts";

let failed = false;
function report(name, problems) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}

const P = "x";
const root = `/p/${P}`;
const entries = NAV.flatMap((a) => a.entries.map((e) => ({ area: a.key, ...e, at: e.path(P) })));

// R-1: every top-level project page route is exactly one entry's path.
{
  const problems = [];
  const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
  // Detail pages are reached from an entry, not listed in the nav.
  const detail = [/^feature\/:/, /^milestone\/:/, /^state\/:/, /^docs\/file\//, /^docs\/folder\//, /^ui(\/|$)/];
  const routes = [...main.matchAll(/path:\s*"p\/:project(?:\/([^"]*))?"/g)]
    .map((m) => m[1] ?? "")
    .filter((r) => !detail.some((d) => d.test(r)));
  if (routes.length === 0) problems.push("found no project routes in main.tsx");
  for (const r of routes) {
    const url = r === "" ? root : `${root}/${r}`;
    const hits = entries.filter((e) => e.at === url);
    if (hits.length !== 1) problems.push(`route p/:project/${r} is the path of ${hits.length} NAV entries, expected 1`);
  }
  for (const e of entries) {
    const r = e.at === root ? "" : e.at.slice(root.length + 1);
    if (!routes.includes(r)) problems.push(`NAV entry ${e.key} (${e.at}) has no page route in main.tsx`);
  }
  const dup = (xs) => xs.filter((x, i) => xs.indexOf(x) !== i);
  for (const k of new Set(dup(entries.map((e) => e.key)))) problems.push(`two NAV entries have the key ${k}`);
  for (const p of new Set(dup(entries.map((e) => e.at)))) problems.push(`two NAV entries have the path ${p}`);
  report("web::check_nav_lists_every_page_route_once", problems);
}

// R-1: conditional entries and empty areas.
{
  const problems = [];
  const ix = (releases) => ({ config: { cadence: { sprints: false, releases, git_model: false } } });
  const keys = (areas) => areas.flatMap((a) => a.entries.map((e) => e.key));
  const cases = [
    ["the index is not loaded", null, false],
    ["the project does not use releases", ix(false), false],
    ["the project uses releases", ix(true), true],
  ];
  for (const [what, index, shown] of cases) {
    const areas = visibleAreas(NAV, index);
    if (keys(areas).includes("releases") !== shown) problems.push(`Releases ${shown ? "hidden" : "shown"} when ${what}`);
    for (const a of areas) if (a.entries.length === 0) problems.push(`area ${a.key} shown with no entries when ${what}`);
    if (areas.some((a) => a.key === "evidence")) problems.push(`Evidence shown with no entries when ${what}`);
    if (!keys(areas).includes("board")) problems.push(`Board missing when ${what}`);
  }
  const fixture = [{ key: "plan", label: "Plan", entries: [{ key: "releases", label: "Releases", path: (p) => `/p/${p}/releases`, when: (i) => i.config.cadence.releases }] }];
  if (visibleAreas(fixture, ix(false)).length !== 0) problems.push("an area whose only entry is hidden is still shown");
  report("web::check_nav_hides_releases_without_cadence_and_empty_areas", problems);
}

// R-2: the active area and entry.
{
  const problems = [];
  const cases = [
    ["/p/x", { area: "work", entry: "board" }],
    ["/p/x/", { area: "work", entry: "board" }],
    ["/p/x/feature/F-1", { area: "work", entry: "board" }],
    ["/p/x/state/Planned", { area: "work", entry: "board" }],
    ["/p/x/review", { area: "work", entry: "review" }],
    ["/p/x/milestones", { area: "work", entry: "milestones" }],
    ["/p/x/milestone/MS-1", { area: "work", entry: "milestones" }],
    ["/p/x/releases", { area: "plan", entry: "releases" }],
    ["/p/x/docs", { area: "records", entry: "docs" }],
    ["/p/x/docs/file/a.md", { area: "records", entry: "docs" }],
    ["/p/x/docs/folder/design", { area: "records", entry: "docs" }],
    ["/p/x/docsx", null],
    ["/p/xy/review", null],
    ["/portfolio", null],
    ["/", null],
  ];
  for (const [path, want] of cases) {
    const got = activeEntry(NAV, P, path);
    if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${path} → ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
  report("web::check_nav_marks_the_active_area", problems);
}

// R-3: one menu at a time; Escape closes and returns focus; choosing a page closes.
{
  const problems = [];
  const expect = (what, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  };
  const closed = { open: null, mobile: false, returnFocus: null };
  let s = navReducer(closed, { type: "toggle", area: "work" });
  expect("toggle work", s, { open: "work", mobile: false, returnFocus: null });
  s = navReducer(s, { type: "toggle", area: "plan" });
  expect("toggle work then plan", s, { open: "plan", mobile: false, returnFocus: null });
  s = navReducer(s, { type: "escape" });
  expect("escape from plan", s, { open: null, mobile: false, returnFocus: "plan" });
  s = navReducer({ ...s, returnFocus: null }, { type: "toggle", area: "plan" });
  s = navReducer(s, { type: "toggle", area: "plan" });
  expect("toggle plan twice", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "navigate" });
  expect("navigate", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "outside" });
  expect("outside click", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "close" });
  expect("close", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "work" }), { type: "toggle-mobile" });
  expect("toggle-mobile with a menu open", s, { open: null, mobile: true, returnFocus: null });
  expect("escape from the mobile menu", navReducer(s, { type: "escape" }), { open: null, mobile: false, returnFocus: "mobile" });
  expect("toggle-mobile twice", navReducer(s, { type: "toggle-mobile" }), closed);
  expect("navigate from the mobile menu", navReducer(s, { type: "navigate" }), closed);
  expect("escape with nothing open", navReducer(closed, { type: "escape" }), closed);
  report("web::check_nav_menus_open_one_at_a_time_and_close_on_escape", problems);
}

// R-5: each destination once across NAV and the global nav (the literal NavLinks in App.tsx).
{
  const problems = [];
  const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
  const global = [...app.matchAll(/<NavLink\b[^>]*?\bto="([^"]+)"/g)].map((m) => m[1]);
  if (global.length === 0) problems.push("found no global nav links in App.tsx");
  const all = [...entries.map((e) => e.at), ...global];
  const seen = new Map();
  for (const p of all) seen.set(p, (seen.get(p) ?? 0) + 1);
  for (const [p, n] of seen) if (n > 1) problems.push(`${p} appears ${n} times in NAV and the global nav`);
  report("web::check_shell_links_are_not_duplicated", problems);
}

// R-7: Review is Work's second entry, after Board.
{
  const problems = [];
  const work = NAV.find((a) => a.key === "work");
  const keys = work ? work.entries.map((e) => e.key) : [];
  if (keys[0] !== "board" || keys[1] !== "review") problems.push(`Work lists ${JSON.stringify(keys)}, expected board then review first`);
  report("web::check_nav_puts_review_second_under_work", problems);
}

if (failed) process.exit(1);
