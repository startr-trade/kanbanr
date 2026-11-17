#!/usr/bin/env node
//
// FTR-048: the project Overview, rendered, and the landing it replaces.
//   web::check_project_root_is_the_overview_and_the_board_moved (R-4) — main.tsx maps p/:project to
//     OverviewPage and p/:project/board to BoardPage; nav.ts has an Overview entry at /p/{p} and the
//     Board at /p/{p}/board; no link labelled Board or "dashboard" still points at /p/{p} without
//     /board; the Overview's h2s come in the stated order.
//   web::check_overview_numbers_all_link (R-9) — every numbered tile under Now and What the board
//     holds is a link with an href (each to its list), except the flow tiles, which carry a Copy
//     "Ask Claude" sentence; every goal row links its items; more than five waiting lines fold the
//     last card into "N more in Needs you".
//   web::check_overview_empty_states_ask_claude (R-10) — with no sprint, no release and no goals each
//     Empty has its sentence and a Copy button; while loading, role=status and never "Nothing needs
//     you".
//   web::check_home_redirects_to_the_only_project (R-12) — homeTarget gives /p/a for one project and
//     null for none or two; HomePage renders <Navigate replace> to it when it is set.
//   web::check_flow_tiles_offer_the_flow_report_sentence (R-13) — the cycle-time and rework tiles are
//     not links and each has a Copy button with "Ask Claude for the flow report."
//
// The page renders through scripts/lib/render.mjs (FTR-046).
//
// Usage: npm run check:overview
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { NAV, activeEntry } from "../src/nav.ts";
import { homeTarget } from "../src/homeTarget.ts";
import { closeRender, render } from "./lib/render.mjs";

let failed = false;
function report(name, problems) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
const srcDir = new URL("../src/", import.meta.url).pathname;
const read = (rel) => readFileSync(join(srcDir, rel), "utf8");

const FLOW = "Ask Claude for the flow report.";
const noop = () => {};

const attention = {
  total: 3,
  lines: [
    { kind: "approve", section: "decisions", label: "Approve", count: 2, to: "/p/x/review?group=approve", tone: "info", yours: true },
    { kind: "release", section: "releases", label: "v0.8.0 at candidate", count: 1, to: "/p/x/releases?tab=current#v0.8.0", tone: "info", yours: true },
    { kind: "red-tests", section: "health", label: "Red tests", count: 4, to: "/p/x/items?evidence=red", tone: "error", yours: false },
    { kind: "doctor-errors", section: "health", label: "Doctor errors", count: 22, ask: "Ask Claude to fix the 22 doctor errors.", tone: "error", yours: false },
  ],
};
const full = {
  project: "x",
  board_rev: "abcdef0123456789",
  generated_at: "2026-11-17T10:00:00Z",
  attention,
  now: {
    sprint: { code: "SP-004", name: "Monitor", goal: "Pages", start: "2026-11-10", end: "2026-11-23", days_left: 7, committed: 12, done: 5 },
    releases: [
      {
        version: "v0.8.0",
        name: "Monitor",
        stage: "candidate",
        waits_on_tag: false,
        steps: [
          { key: "main", label: "main", state: "done", at: "2026-11-15T09:00:00Z", by: "claude" },
          { key: "candidate", label: "candidate", state: "current" },
          { key: "stable", label: "stable", state: "next" },
        ],
        items: 20,
        finished: 14,
      },
    ],
    milestones: [{ code: "MS-004", name: "Monitor", done: 3, total: 8 }],
  },
  holds: {
    items: { total: 54, by_kind: { feature: 30, defect: 20 }, untyped: 4 },
    requirements: { total: 120, functional: 100, nfr: 20, by_pattern: { event: 40 }, iso_tagged: 12 },
    tests: { total: 200, by_state: { green: 180, red: 3, planned: 17 }, by_kind: { unit: 200 } },
    coverage: { proven: 102, total: 120, stale_items: 6 },
    flow: { completed: 40, cycle_p50_days: 0.03, cycle_p90_days: 1.5, rework: 2, defects: 20, escaped: 7 },
    architecture: { adrs: { accepted: 18, proposed: 2 }, design_docs: 9, component_docs: 14 },
    records: { retros: 3, release_notes: 7, lessons: 46, releases: 9 },
  },
  goals: [
    { id: "G-1", title: "A session recovers the plan", items_done: 4, items_open: 2, req_proven: 10, req_total: 12 },
    { id: "G-2", title: "Every item records why", items_done: 1, items_open: 0, req_proven: 3, req_total: 3 },
  ],
  recent: [
    { at: "2026-11-16T12:00:00Z", kind: "approval", code: "FTR-043", words: "FTR-043 One Plan page approved on the monitor", to: "/p/x/feature/FTR-043" },
    { at: "2026-11-14T08:00:00Z", kind: "adr", code: "ADR-0020", words: "ADR-0020 accepted: Plan replaces Gantt", to: "/p/x/docs/file/decisions/0020-plan.md" },
  ],
  activity: [{ at: "2026-11-17T09:55:00Z", item: "FTR-048", words: "FTR-048 A project Overview edited" }],
};
const cadence = { sprints: true, releases: true };
const ok = (data) => ({ data, loading: false, error: null, reload: noop });
const body = (overview, extra = {}) =>
  render("src/pages/OverviewPage.tsx", "OverviewBody", { project: "x", name: "X project", overview, drift: null, cadence, today: "2026-11-17", ...extra }, { url: "/p/x" });

try {
  // R-4: the project's root is the Overview; the Board moved to /board.
  {
    const problems = [];
    const main = read("main.tsx");
    if (!/path:\s*"p\/:project",\s*element:\s*<OverviewPage\s*\/>/.test(main)) problems.push("main.tsx does not route p/:project to OverviewPage");
    if (!/path:\s*"p\/:project\/board",\s*element:\s*<BoardPage\s*\/>/.test(main)) problems.push("main.tsx does not route p/:project/board to BoardPage");
    const entries = NAV.flatMap((a) => a.entries.map((e) => ({ area: a.key, ...e })));
    const overview = entries.find((e) => e.key === "overview");
    if (!overview) problems.push("nav.ts has no Overview entry");
    else {
      if (overview.area !== "overview") problems.push(`the Overview entry sits in ${overview.area}, not the Overview area`);
      if (overview.path("x") !== "/p/x") problems.push(`the Overview entry's path is ${overview.path("x")}, expected /p/x`);
    }
    const board = entries.find((e) => e.key === "board");
    if (!board) problems.push("nav.ts has no Board entry");
    else if (board.path("x") !== "/p/x/board") problems.push(`the Board entry's path is ${board.path("x")}, expected /p/x/board`);
    const cases = [
      ["/p/x", { area: "overview", entry: "overview" }],
      ["/p/x/board", { area: "work", entry: "board" }],
      ["/p/x/feature/F-1", { area: "work", entry: "board" }],
      ["/p/x/state/Planned", { area: "work", entry: "board" }],
    ];
    for (const [path, want] of cases) {
      const got = activeEntry(NAV, "x", path);
      if (!same(got, want)) problems.push(`${path} marks ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
    }
    // No link that reads Board or dashboard still points at the project's root.
    const files = [];
    const walk = (d) => {
      for (const f of readdirSync(d)) {
        const p = join(d, f);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(tsx?|mts)$/.test(f)) files.push(p);
      }
    };
    walk(srcDir);
    for (const f of files) {
      const src = readFileSync(f, "utf8");
      for (const m of src.matchAll(/<Link\b[^>]*?\bto=\{((?:[^{}]|\{[^{}]*\})*)\}[^>]*>([^<]*)<\/Link>/g)) {
        if (/board|dashboard/i.test(m[2]) && !/board/.test(m[1])) problems.push(`${f.slice(srcDir.length)}: the link "${m[2].trim()}" goes to {${m[1]}}, not the Board`);
      }
    }
    const home = read("pages/HomePage.tsx");
    if (/Open dashboard/.test(home) && !/\/board`/.test(home)) problems.push("HomePage's Open dashboard link does not go to /board");
    // The running sprint on the Workflow page links the Board, which opens on it.
    if (/to:\s*base\s*,/.test(read("workflowSections.ts"))) problems.push("workflowSections.ts links the running sprint to the project's root, not the Board");
    const { document } = await body(ok(full));
    const h2 = [...document.querySelectorAll("h2")].map(text);
    const want = ["Needs you", "Now", "What the board holds", "Goals", "Recent decisions and ADRs", "Activity"];
    if (!same(h2, want)) problems.push(`the Overview's h2s are ${JSON.stringify(h2)}, expected ${JSON.stringify(want)}`);
    const h1 = [...document.querySelectorAll("h1")].map(text);
    if (h1.length !== 1 || h1[0] !== "X project") problems.push(`the Overview's h1 is ${JSON.stringify(h1)}, expected "X project"`);
    if (!text(document.body).includes("as of abcdef0")) problems.push("the Overview does not say which board revision it shows (as of abcdef0)");
    report("web::check_project_root_is_the_overview_and_the_board_moved", problems);
  }

  // R-9: every number links to its list.
  {
    const problems = [];
    const { document } = await body(ok(full));
    const section = (title) => [...document.querySelectorAll("section")].find((s) => text(s.querySelector("h2")) === title);
    for (const title of ["Now", "What the board holds"]) {
      const s = section(title);
      if (!s) {
        problems.push(`no ${title} section`);
        continue;
      }
      for (const tile of s.querySelectorAll(".ui-tile")) {
        if (!/\d/.test(text(tile.querySelector(".ui-tile-value")))) continue;
        const label = text(tile.querySelector(".ui-tile-label"));
        const flow = /cycle time|rework/i.test(label);
        if (flow) continue;
        if (tile.tagName !== "A" || !tile.getAttribute("href")) problems.push(`${title}: the tile "${text(tile)}" is not a link`);
      }
    }
    const holds = section("What the board holds");
    const hrefs = holds ? [...holds.querySelectorAll("a")].map((a) => a.getAttribute("href")) : [];
    const expected = [
      "/p/x/items",
      "/p/x/items?kind=feature",
      "/p/x/items?kind=defect",
      "/p/x/items?kind=none",
      "/p/x/items?evidence=unproven",
      "/p/x/items?evidence=red",
      "/p/x/items?evidence=planned",
      "/p/x/items?evidence=stale",
      "/p/x/docs/folder/decisions",
      "/p/x/docs/folder/design",
      "/p/x/docs/folder/design/components",
      "/p/x/docs/folder/retros",
      "/p/x/docs/folder/releases",
      "/p/x/charter#lessons",
      "/p/x/releases",
    ];
    for (const h of expected) if (!hrefs.includes(h)) problems.push(`What the board holds has no link to ${h}`);
    const untyped = holds && [...holds.querySelectorAll("a.ui-tile")].find((a) => a.getAttribute("href") === "/p/x/items?kind=none");
    if (untyped && !untyped.classList.contains("ui-tile-warn")) problems.push("the untyped tile is not in the warn tone");
    const red = holds && [...holds.querySelectorAll("a.ui-tile")].find((a) => a.getAttribute("href") === "/p/x/items?evidence=red");
    if (red && !red.classList.contains("ui-tile-error")) problems.push("the red tests tile is not in the error tone");
    const now = section("Now");
    const nowHrefs = now ? [...now.querySelectorAll("a")].map((a) => a.getAttribute("href")) : [];
    for (const h of ["/p/x/plan?tab=current#SP-004", "/p/x/releases?tab=current#v0.8.0", "/p/x/milestone/MS-004"])
      if (!nowHrefs.includes(h)) problems.push(`Now has no link to ${h}`);
    if (now && !now.querySelector('[aria-current="step"]')) problems.push("Now draws no stage stepper for the release in flight");
    const goals = section("Goals");
    const rows = goals ? [...goals.querySelectorAll("tbody tr")] : [];
    if (rows.length !== 2) problems.push(`Goals shows ${rows.length} rows, expected 2`);
    for (const [i, g] of ["G-1", "G-2"].entries()) {
      const a = rows[i] && [...rows[i].querySelectorAll("a")].find((x) => x.getAttribute("href") === `/p/x/items?goal=${g}`);
      if (!a) problems.push(`the ${g} row does not link /p/x/items?goal=${g}`);
    }
    const needs = section("Needs you");
    const cardLinks = needs ? [...needs.querySelectorAll("a.ui-tile")].map((a) => a.getAttribute("href")) : [];
    if (!cardLinks.includes("/p/x/review?group=approve")) problems.push("Needs you has no card linking the Approve group");
    if (needs && !text(needs).includes("Ask Claude to fix the 22 doctor errors.")) problems.push("the doctor card does not offer its sentence");
    const recent = section("Recent decisions and ADRs");
    if (recent && ![...recent.querySelectorAll("a")].some((a) => a.getAttribute("href") === "/p/x/feature/FTR-043")) problems.push("Recent does not link its lines");
    const activity = section("Activity");
    if (activity && ![...activity.querySelectorAll("a")].some((a) => a.getAttribute("href") === "/p/x/feature/FTR-048")) problems.push("Activity does not link the item code");
    // More than five lines: four cards, then "N more in Needs you".
    const many = { ...full, attention: { total: 7, lines: Array.from({ length: 7 }, (_, i) => ({ kind: `g${i}`, section: "decisions", label: `Group ${i}`, count: 10 - i, to: `/p/x/review?group=g${i}`, tone: "info", yours: true })) } };
    const { document: d } = await body(ok(many));
    const n = [...d.querySelectorAll("section")].find((s) => text(s.querySelector("h2")) === "Needs you");
    const cards = n ? n.querySelectorAll(".overview-card").length : 0;
    if (cards !== 5) problems.push(`with seven waiting lines Needs you shows ${cards} cards, expected 5`);
    const more = n && [...n.querySelectorAll("button")].find((b) => /3 more in Needs you/.test(text(b)));
    if (!more) problems.push('the fifth card is not a "3 more in Needs you" button');
    report("web::check_overview_numbers_all_link", problems);
  }

  // R-10: empty states ask Claude; loading is never empty.
  {
    const problems = [];
    const empty = {
      project: "x",
      board_rev: "abcdef0",
      generated_at: "2026-11-17T10:00:00Z",
      attention: { total: 0 },
      now: {},
      holds: full.holds,
    };
    const { document } = await body(ok(empty));
    const empties = [...document.querySelectorAll(".ui-empty")];
    const find = (t) => empties.find((e) => text(e.querySelector(".ui-empty-title")) === t);
    const cases = [
      ["Nothing needs you right now.", null],
      ["No sprint is running.", "Ask Claude to plan the next sprint."],
      ["No release is in flight.", "Ask Claude what the next release should hold."],
      ["No milestone is in progress.", null],
      ["The charter has no goals yet.", "Ask Claude to draft the charter's goals with you."],
      ["No decisions recorded yet.", null],
      ["No activity yet.", null],
    ];
    for (const [title, ask] of cases) {
      const e = find(title);
      if (!e) {
        problems.push(`no empty state "${title}"`);
        continue;
      }
      if (ask) {
        if (text(e.querySelector(".ui-empty-sentence")) !== ask) problems.push(`"${title}" does not ask "${ask}"`);
        if (![...e.querySelectorAll("button")].some((b) => text(b) === "Copy")) problems.push(`"${title}" has no Copy button`);
      }
    }
    // Sprints and releases off: neither empty state is drawn.
    const { document: off } = await body(ok(empty), { cadence: { sprints: false, releases: false } });
    if (/No sprint is running|No release is in flight/.test(text(off.body))) problems.push("with sprints and releases off the Overview still says none is running");
    const { document: loading } = await body({ data: null, loading: true, error: null, reload: noop });
    if (!loading.querySelector('[role="status"]')) problems.push("while loading there is no role=status");
    if (/Nothing needs you/.test(text(loading.body))) problems.push('while loading the Overview says "Nothing needs you"');
    const { document: failed } = await body({ data: null, loading: false, error: "boom", reload: noop });
    if (!failed.querySelector('[role="alert"]')) problems.push("a failed read shows no role=alert");
    if (/Nothing needs you/.test(text(failed.body))) problems.push('a failed read says "Nothing needs you"');
    report("web::check_overview_empty_states_ask_claude", problems);
  }

  // R-12: Home with one project opens it.
  {
    const problems = [];
    if (homeTarget([{ id: "a" }]) !== "/p/a") problems.push(`homeTarget([a]) is ${homeTarget([{ id: "a" }])}, expected /p/a`);
    if (homeTarget([{ id: "a b" }]) !== "/p/a%20b") problems.push("homeTarget does not encode the project id");
    if (homeTarget([]) !== null) problems.push("homeTarget([]) is not null");
    if (homeTarget([{ id: "a" }, { id: "b" }]) !== null) problems.push("homeTarget of two projects is not null");
    const home = read("pages/HomePage.tsx");
    if (!/homeTarget\(/.test(home)) problems.push("HomePage.tsx does not decide through homeTarget");
    if (!/<Navigate\s+replace\s+to=\{/.test(home)) problems.push("HomePage.tsx does not render <Navigate replace to={…}/>");
    report("web::check_home_redirects_to_the_only_project", problems);
  }

  // R-13: the flow tiles are not links and offer the flow report sentence.
  {
    const problems = [];
    const { document } = await body(ok(full));
    const tiles = [...document.querySelectorAll(".ui-tile")];
    for (const label of [/cycle time/i, /rework/i]) {
      const tile = tiles.find((t) => label.test(text(t.querySelector(".ui-tile-label"))));
      if (!tile) {
        problems.push(`no ${label.source} tile`);
        continue;
      }
      if (tile.tagName === "A" || tile.closest("a")) problems.push(`the ${label.source} tile is a link`);
      const box = tile.closest(".overview-ask-tile");
      if (!box) {
        problems.push(`the ${label.source} tile has no sentence beside it`);
        continue;
      }
      if (!text(box).includes(FLOW)) problems.push(`the ${label.source} tile does not say "${FLOW}"`);
      if (![...box.querySelectorAll("button")].some((b) => text(b) === "Copy")) problems.push(`the ${label.source} tile has no Copy button`);
    }
    report("web::check_flow_tiles_offer_the_flow_report_sentence", problems);
  }
  // R-8 (T10): marks on one day draw as one mark.
  {
    const problems = [];
    const marks = Array.from({ length: 8 }, (_, i) => ({ at: "2026-11-15", label: i === 0 ? "X" : `Y${i}`, kind: "dot" }));
    const lanes = [{ key: "d", label: "Decisions", bands: [], marks }];
    const { document } = await render("src/ui/Timeline.tsx", "Timeline", { from: "2026-11-10", to: "2026-11-23", lanes, caption: "c" });
    const drawn = [...document.querySelectorAll("svg .ui-tl-mark")];
    if (drawn.length !== 1) problems.push(`eight marks on one day drew ${drawn.length} marks, not 1`);
    else {
      if (text(drawn[0].querySelector("text")) !== "X +7") problems.push(`the label is "${text(drawn[0].querySelector("text"))}", not "X +7"`);
      const title = text(drawn[0].querySelector("title"));
      for (const m of marks) if (!title.includes(m.label)) problems.push(`the title does not name ${m.label}`);
    }
    // Apart marks stay separate; a diamond in a group makes it a diamond; a crowded label is dropped.
    const mixed = [
      { at: "2026-11-12", label: "A", kind: "dot" },
      { at: "2026-11-20", label: "A very long label that cannot fit", kind: "dot" },
      { at: "2026-11-20", label: "R", kind: "diamond" },
      { at: "2026-11-21", label: "C", kind: "dot" },
    ];
    const r2 = await render("src/ui/Timeline.tsx", "Timeline", { from: "2026-11-10", to: "2026-11-23", lanes: [{ key: "d", label: "D", bands: [], marks: mixed }], caption: "c" });
    const m2 = [...r2.document.querySelectorAll("svg .ui-tl-mark")];
    if (m2.length !== 3) problems.push(`mixed lane drew ${m2.length} marks, not 3`);
    else {
      if (!m2[1].classList.contains("ui-tl-diamond")) problems.push("a group holding a diamond is not a diamond");
      if (m2[2].querySelector("text")) problems.push("a label that would overprint its neighbour was not dropped");
    }
    report("web::check_timeline_marks_on_one_day_draw_as_one_mark", problems);
  }
} catch (e) {
  failed = true;
  console.error(`check-overview: ${e?.stack ?? e}`);
}

await closeRender();
process.exit(failed ? 1 : 0);
