#!/usr/bin/env node
//
// FTR-043: the Plan page, checked without a browser. Each check prints its name.
//   web::check_plan_tabs_carry_counts_and_open_current_when_work_runs (R-1) — planTabs gives
//     Current, Planned and Past with the summary's counts, in that order; defaultTab opens Current
//     when something runs, Planned when nothing does, and the tab the URL asks for when it names
//     one of the three.
//   web::check_plan_url_names_the_tab_and_open_rows (R-6), the pure part — `?tab=past#sprint:SP-003`
//     opens Past and that row; a bare `#SP-004` (the Overview's link) is read as `sprint:SP-004`;
//     the old `/schedule` and `/gantt` paths go to the Plan page; the Plan URLs are the daemon's.
//     The rendered part — the page rendered at `?tab=past#sprint:SP-003` selects Past and opens
//     SP-003's row only; at `#SP-004` the Current tab opens SP-004's row.
//   web::check_plan_page_reads_only_what_is_open (R-8) — the page reads the summary, one tab's
//     slice and an opened row's body through the shared cache with the Plan tags; a row's body is
//     drawn only inside AccordionRow's `children`, so a closed row reads nothing; nothing under the
//     page calls api.getProject, getGantt or useAsync. An empty Current tab asks Claude to plan.
//   web::check_plan_timeline_draws_tall_bands_with_progress_a_release_lane_and_a_today_line (R-10)
//     — toTimeline gives a sprint 3 of 6 done progress 0.5 and a release a lane of diamond and dot
//     marks with no band; rendered, the bands are 24 px tall with a filled part half their width,
//     and today is one solid line in --accent labelled "today".
//   web::check_plan_replaces_the_gantt_and_schedule_pages (R-6) — main.tsx routes `p/:project/plan`
//     to the Plan page and turns `schedule` and `gantt` into redirects (to Planned, and to the
//     page); GanttPage.tsx and SchedulePage.tsx are gone; NAV's Plan area has one `plan` entry,
//     active on the old paths too, and no Schedule or Gantt entry.
//   web::check_portfolio_timeline_has_one_row_per_project (R-7) — the portfolio page reads
//     `GET /portfolio/plan` through the shared cache, and its "Now" timeline, rendered, has one lane
//     per project, in the daemon's order: a project running a sprint has its band, its release in
//     flight a diamond, a project with no cadence a lane with nothing on it; today is drawn once.
//
// Usage: npm run check:plan-page
import { defaultTab, itemBands, legacyRedirect, openLanes, planTabs, toTimeline } from "../src/planTabs.ts";
import { TAGS, urls } from "../src/data/urls.ts";
import { activeEntry, NAV } from "../src/nav.ts";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { closeRender, render } from "./lib/render.mjs";

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

const summary = (current, planned, past) => ({
  current,
  planned,
  past,
  open: current > 0 ? "current" : "planned",
  today: "2026-11-17",
});

const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
const src = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
/** Each row's key and whether its header says it is open. */
const expanded = (document) =>
  Object.fromEntries([...document.querySelectorAll("section.ui-row")].map((s) => [s.id, s.querySelector("button[aria-expanded]")?.getAttribute("aria-expanded") === "true"]));

const lane = (key, kind, extra) => ({ key, kind, label: key.split(":")[1], state: "", from: "", to: "", bands: [], marks: [], done: 0, total: 0, carried: 0, ...extra });
const CURRENT = {
  scope: "current",
  today: "2026-11-17",
  from: "2026-11-03",
  to: "2026-12-01",
  lanes: [
    lane("sprint:SP-004", "sprint", {
      state: "active",
      from: "2026-11-15",
      to: "2026-11-28",
      bands: [{ from: "2026-11-15", to: "2026-11-28", label: "SP-004", tone: "info", progress: 0.5 }],
      done: 3,
      total: 6,
      burndown: {
        code: "SP-004",
        name: "",
        start: "2026-11-15",
        end: "2026-11-28",
        state: "active",
        unit: "days",
        items: ["FTR-1"],
        committed: 6,
        done: 3,
        unestimated: [],
        days_left: 11,
        burndown: [{ date: "2026-11-15", remaining: 6 }, { date: "2026-11-16", remaining: 4 }, { date: "2026-11-17", remaining: 3 }],
      },
    }),
    lane("release:v0.8.0", "release", {
      state: "candidate",
      from: "2026-11-16",
      to: "2026-11-24",
      marks: [
        { at: "2026-11-24", label: "v0.8.0", kind: "diamond" },
        { at: "2026-11-16", label: "candidate", kind: "dot" },
      ],
      done: 4,
      total: 9,
    }),
  ],
  undated: [],
};
const PAST = {
  scope: "past",
  today: "2026-11-17",
  from: "2026-08-18",
  to: "2026-11-17",
  lanes: [
    lane("sprint:SP-003", "sprint", { state: "closed", from: "2026-11-14", to: "2026-11-20", done: 9, total: 11, carried: 2 }),
    lane("release:v0.7.0", "release", { state: "shipped", from: "2026-11-13", to: "2026-11-13", done: 8, total: 8 }),
    lane("sprint:SP-002", "sprint", { state: "closed", from: "2026-10-28", to: "2026-11-03", done: 5, total: 5 }),
  ],
  undated: [],
};
const MILESTONE = lane("milestone:MS-004", "milestone", { state: "open", done: 1, total: 3 });
const ROW = {
  key: "sprint:SP-004",
  items: [
    { code: "FTR-1", title: "One", status: "Done", from: "2026-11-15", to: "2026-11-16", placed: "moves", critical: true, pending: [] },
    { code: "FTR-2", title: "Two", status: "In Progress", from: "2026-11-15", to: "2026-11-28", placed: "sprint", critical: false, pending: [] },
    { code: "FTR-3", title: "Three", status: "Planned", from: "", to: "", placed: "undated", critical: false, pending: [] },
  ],
  by_status: [],
  carried: [],
};

await check(
  "web::check_plan_tabs_carry_counts_and_open_current_when_work_runs",
  "the tabs carry the summary's counts, and the page opens on Current when work runs, else Planned.",
  (expect) => {
    const s = summary(2, 4, 3);
    expect(
      "the tabs, in order, with their counts",
      planTabs(s).map((t) => [t.key, t.label, t.count]),
      [
        ["current", "Current", 2],
        ["planned", "Planned", 4],
        ["past", "Past", 3],
      ],
    );
    expect("work runs: Current", defaultTab(s, null), "current");
    // Past has more rows, yet the page opens where the summary says, not on the largest tab.
    expect("work runs, Past larger: still Current", defaultTab(summary(1, 0, 9), null), "current");
    expect("nothing runs: Planned", defaultTab(summary(0, 4, 3), null), "planned");
    expect("nothing runs, nothing planned: Planned", defaultTab(summary(0, 0, 3), null), "planned");
    expect("the URL's tab wins", defaultTab(s, "past"), "past");
    expect("an empty tab can still be asked for", defaultTab(summary(0, 4, 3), "current"), "current");
    expect("an unknown tab falls back to open", defaultTab(s, "later"), "current");
    expect("an empty board: Planned, all zero", planTabs(summary(0, 0, 0)).map((t) => t.count), [0, 0, 0]);
  },
);

await check(
  "web::check_plan_url_names_the_tab_and_open_rows",
  "the tab and the open rows come from the URL, a bare sprint code opens that sprint, the rendered page opens on them, and /schedule and /gantt go to the Plan page.",
  async (expect, problems) => {
    expect("?tab=past opens Past", defaultTab(summary(2, 4, 3), new URLSearchParams("?tab=past").get("tab")), "past");
    expect("#sprint:SP-003 opens that row", [...openLanes("#sprint:SP-003")], ["sprint:SP-003"]);
    expect("a bare #SP-004 is a sprint", [...openLanes("#SP-004")], ["sprint:SP-004"]);
    expect(
      "an open list keeps each key",
      [...openLanes("#open=sprint:SP-004,release:v0.8.0,milestone:MS-004")],
      ["sprint:SP-004", "release:v0.8.0", "milestone:MS-004"],
    );
    expect("no hash, no open row", [...openLanes("")], []);

    expect("/schedule goes to Planned", legacyRedirect("/p/kanbanr/schedule"), "/p/kanbanr/plan?tab=planned");
    expect("/gantt goes to the Plan page", legacyRedirect("/p/kanbanr/gantt"), "/p/kanbanr/plan");
    expect("a trailing slash is the same page", legacyRedirect("/p/kanbanr/gantt/"), "/p/kanbanr/plan");
    expect("another page is not redirected", legacyRedirect("/p/kanbanr/releases"), null);
    expect("a page that only ends in the word is not redirected", legacyRedirect("/p/kanbanr/my-gantt"), null);

    const end = (what, url, want) => {
      if (!url.endsWith(want)) problems.push(`${what} is ${url}, wanted …${want}`);
    };
    end("urls.planSummary", urls.planSummary("x"), "/projects/x/plan/summary");
    end("urls.plan", urls.plan("x", "past"), "/projects/x/plan?scope=past");
    end("urls.planRow", urls.planRow("x", "sprint:SP-004"), "/projects/x/plan/rows/sprint%3ASP-004");
    end("urls.portfolioPlan", urls.portfolioPlan(), "/portfolio/plan");
    expect("the Plan reads' tags", [...TAGS.plan], ["sprints", "releases", "milestones", "items"]);

    // The slice's own window and today reach the timeline untouched, and rows keep their keys.
    const slice = {
      scope: "current",
      today: "2026-11-17",
      from: "2026-11-03",
      to: "2026-12-01",
      lanes: [
        {
          key: "sprint:SP-004",
          kind: "sprint",
          label: "SP-004",
          state: "active",
          from: "2026-11-15",
          to: "2026-11-28",
          bands: [{ from: "2026-11-15", to: "2026-11-28", label: "SP-004", tone: "info", progress: 0.5 }],
          marks: [],
          done: 3,
          total: 6,
          carried: 0,
        },
      ],
      undated: [],
    };
    const t = toTimeline(slice);
    expect("the window and today", [t.from, t.to, t.today], ["2026-11-03", "2026-12-01", "2026-11-17"]);
    expect("the lane keys are the URL's row keys", t.lanes.map((l) => l.key), ["sprint:SP-004"]);

    // An undated item gets no lane on the row's timeline.
    const body = {
      key: "sprint:SP-004",
      items: [
        { code: "A", title: "a", status: "Done", from: "2026-11-15", to: "2026-11-16", placed: "moves", critical: true, pending: [] },
        { code: "U", title: "u", status: "Planned", from: "", to: "", placed: "undated", critical: false, pending: [] },
      ],
      by_status: [],
      carried: [],
    };
    expect("undated items are not drawn", itemBands(body).map((l) => l.key), ["A"]);

    // Rendered: the tab from `?tab=`, the open rows from the hash.
    const pastUrl = "/p/shop/plan?tab=past#sprint:SP-003";
    const page = await render("src/pages/PlanPage.tsx", "PlanBody", { project: "shop", summary: summary(1, 2, 2) }, { url: pastUrl });
    const selected = [...page.document.querySelectorAll('[role="tab"][aria-selected="true"]')].map((t) => text(t));
    expect("?tab=past selects the Past tab", selected.map((t) => t.split(" ")[0]), ["Past"]);
    expect("the page has one h1, Plan", [...page.document.querySelectorAll("h1")].map(text), ["Plan"]);

    const past = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "past", slice: PAST }, { url: pastUrl });
    expect("#sprint:SP-003 opens SP-003 and nothing else", expanded(past.document), { "sprint:SP-003": true, "release:v0.7.0": false, "sprint:SP-002": false });

    const bare = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "current", slice: CURRENT }, { url: "/p/shop/plan?tab=current#SP-004" });
    expect("a bare #SP-004 opens the sprint's row", expanded(bare.document), { "sprint:SP-004": true, "release:v0.8.0": false });
  },
);

await check(
  "web::check_plan_page_reads_only_what_is_open",
  "the page reads the summary, one tab and an opened row through the shared cache with the Plan tags, a closed row draws and reads nothing, nothing reads the whole board or the Gantt, and an empty Current tab asks Claude to plan.",
  async (expect, problems) => {
    const sources = { "pages/PlanPage.tsx": src("pages/PlanPage.tsx") };
    for (const f of readdirSync(new URL("../src/pages/plan", import.meta.url))) sources[`pages/plan/${f}`] = src(`pages/plan/${f}`);
    expect("the files under pages/plan", Object.keys(sources).sort(), ["pages/PlanPage.tsx", "pages/plan/FlowPanel.tsx", "pages/plan/PastRow.tsx", "pages/plan/PlanRow.tsx", "pages/plan/RowItems.tsx"]);
    for (const [rel, text] of Object.entries(sources)) {
      for (const bad of [/\bapi\.getProject\s*\(/, /\bgetGantt\b/, /\buseAsync\b/, /\bapi\.getPlan/]) {
        if (bad.test(text)) problems.push(`${rel} has ${bad}: reads go through useQuery and urls`);
      }
    }
    const page = sources["pages/PlanPage.tsx"];
    if (!/useQuery<PlanSummary>\(\s*project\s*,\s*urls\.planSummary\(project\)\s*,\s*TAGS\.plan\s*\)/.test(page)) problems.push("PlanPage.tsx does not read urls.planSummary(project) with TAGS.plan");
    if (!/useQuery<PlanSlice>\(\s*project\s*,\s*urls\.plan\(project\s*,\s*scope\)\s*,\s*TAGS\.plan\s*\)/.test(page)) problems.push("PlanPage.tsx does not read one tab, urls.plan(project, scope), with TAGS.plan");
    if (!/<Tabs\b[^>]*param="tab"/.test(page)) problems.push("PlanPage.tsx does not keep its tab in ?tab=");
    const items = sources["pages/plan/RowItems.tsx"];
    if (!/useQuery<RowBody>\(\s*project\s*,\s*urls\.planRow\(project\s*,\s*lane\.key\)\s*,\s*TAGS\.plan\s*\)/.test(items)) problems.push("RowItems.tsx does not read urls.planRow(project, lane.key) with TAGS.plan");
    for (const row of ["pages/plan/PlanRow.tsx", "pages/plan/PastRow.tsx"]) {
      if (!/<AccordionRow\b/.test(sources[row])) problems.push(`${row} is not an AccordionRow`);
      if (!/\{\(\)\s*=>\s*<RowItems\b/.test(sources[row])) problems.push(`${row} does not draw RowItems only inside AccordionRow's children function`);
    }

    // Rendered with no row open: no row has a body, so none reads one.
    const closed = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "current", slice: CURRENT }, { url: "/p/shop/plan?tab=current" });
    const bodies = [...closed.document.querySelectorAll(".ui-row-body")];
    expect("one row per lane", bodies.length, CURRENT.lanes.length);
    expect("closed rows draw nothing", bodies.map((b) => b.innerHTML), CURRENT.lanes.map(() => ""));
    // One row open: only its body is drawn, waiting for its own read.
    const opened = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "current", slice: CURRENT }, { url: "/p/shop/plan?tab=current#release:v0.8.0" });
    const drawn = [...opened.document.querySelectorAll(".ui-row-body")].filter((b) => b.innerHTML !== "").map((b) => b.id);
    expect("only the open row has a body", drawn, ["release:v0.8.0-body"]);
    const wait = opened.document.getElementById("release:v0.8.0-body")?.querySelector('[role="status"]');
    if (!wait) problems.push("the open row does not show its own read loading");

    // The body, once read: the items on the row's timeline, a table, the critical path, the undated.
    const body = await render("src/pages/plan/RowItems.tsx", "RowItemsBody", { project: "shop", lane: CURRENT.lanes[0], from: CURRENT.from, to: CURRENT.to, body: ROW }, { url: "/p/shop/plan" });
    const tl = body.document.querySelector(".ui-timeline svg");
    if (!tl) problems.push("the row's items have no timeline");
    expect("one item lane per dated item", [...body.document.querySelectorAll(".ui-tl-lane-label")].map(text), ["FTR-1", "FTR-2"]);
    const rows = [...body.document.querySelectorAll(".plan-items tbody tr")].map((r) => text(r));
    expect("the table lists every item", rows.length, 3);
    if (!rows.some((r) => r.startsWith("FTR-1") && /critical path/.test(r))) problems.push(`FTR-1 has no critical-path pill: ${JSON.stringify(rows)}`);
    if (rows.some((r) => r.startsWith("FTR-2") && /critical path/.test(r))) problems.push("FTR-2 is marked critical");
    const undated = text(body.document.querySelector(".plan-undated"));
    if (!/^Undated \(1\)/.test(undated) || !/FTR-3/.test(undated)) problems.push(`the undated list is "${undated}"`);
    if (!body.document.querySelector("figure.burndown")) problems.push("the sprint's body has no burndown");
    const ms = await render("src/pages/plan/RowItems.tsx", "RowItemsBody", { project: "shop", lane: MILESTONE, from: "", to: "", body: { ...ROW, key: MILESTONE.key, by_status: [["In Progress", ["FTR-1"]], ["Done", ["FTR-2", "FTR-4"]]] } }, { url: "/p/shop/plan" });
    expect("a milestone's body groups its items by status", [...ms.document.querySelectorAll(".plan-by-status h3")].map(text), ["In Progress · 1", "Done · 2"]);

    // An empty Current tab asks Claude to plan.
    const empty = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "current", slice: { ...CURRENT, lanes: [], undated: [] } }, { url: "/p/shop/plan" });
    expect("the empty Current tab", text(empty.document.querySelector(".ui-empty-title")), "Nothing is running today.");
    expect("its ask", text(empty.document.querySelector(".ui-empty-sentence")), "Ask Claude to plan the next sprint.");
  },
);

await check(
  "web::check_plan_timeline_draws_tall_bands_with_progress_a_release_lane_and_a_today_line",
  "a sprint band carries 0.5 for 3 of 6 and draws 24 px tall with its done half filled, a release is a lane of diamonds and dots with no band, and today is one solid --accent line labelled today.",
  async (expect, problems) => {
    const t = toTimeline(CURRENT);
    const sprint = t.lanes.find((l) => l.key === "sprint:SP-004");
    const release = t.lanes.find((l) => l.key === "release:v0.8.0");
    expect("the sprint band's progress for 3 of 6", sprint?.bands.map((b) => b.progress), [0.5]);
    expect("the release has no band", release?.bands.length, 0);
    expect("the release's marks", release?.marks.map((m) => m.kind), ["diamond", "dot"]);

    const { document } = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "shop", scope: "current", slice: CURRENT }, { url: "/p/shop/plan" });
    const svg = document.querySelector(".ui-timeline svg");
    if (!svg) return problems.push("the tab draws no timeline");
    const band = svg.querySelector(".ui-tl-band");
    const rest = band?.querySelector("rect.ui-tl-rest");
    const done = band?.querySelector("rect.ui-tl-done");
    if (!rest || !done) return problems.push(`the sprint band has no filled part: ${band?.outerHTML}`);
    expect("the band's height", [rest.getAttribute("height"), done.getAttribute("height")], ["24", "24"]);
    const half = Number(done.getAttribute("width")) / Number(rest.getAttribute("width"));
    if (Math.abs(half - 0.5) > 0.01) problems.push(`the filled part is ${half} of the band, wanted 0.5`);
    expect("the release lane draws diamonds and dots, no band", [svg.querySelectorAll(".ui-tl-diamond").length, svg.querySelectorAll(".ui-tl-dot").length, svg.querySelectorAll(".ui-tl-band").length], [1, 1, 1]);
    const today = [...svg.querySelectorAll(".ui-tl-today line")];
    expect("one today line", today.length, 1);
    if (today[0]?.getAttribute("stroke-dasharray")) problems.push("the today line is dashed");
    expect("labelled today", text(svg.querySelector(".ui-tl-today text")), "today");
    const css = readFileSync(new URL("../src/ui/ui.css", import.meta.url), "utf8");
    if (!/\.ui-tl-today line\s*\{[^}]*stroke:\s*var\(--accent\)/.test(css)) problems.push("the today line is not drawn in --accent");
  },
);

await check(
  "web::check_plan_replaces_the_gantt_and_schedule_pages",
  "main.tsx routes the Plan page and redirects /schedule to Planned and /gantt to the page, the two old pages are gone, and the nav has one Plan entry that the old paths also mark.",
  (expect, problems) => {
    expect("/schedule goes to Planned", legacyRedirect("/p/kanbanr/schedule"), "/p/kanbanr/plan?tab=planned");
    expect("/gantt goes to the Plan page", legacyRedirect("/p/kanbanr/gantt"), "/p/kanbanr/plan");

    const main = src("main.tsx");
    if (!/\{\s*path:\s*"p\/:project\/plan",\s*element:\s*<PlanPage\s*\/>\s*\}/.test(main)) problems.push("main.tsx does not route p/:project/plan to <PlanPage />");
    // `..` is resolved against the path, not the route tree, so /p/x/schedule lands on /p/x/plan.
    const redirect = (path, to) =>
      new RegExp(`\\{\\s*path:\\s*"p\\/:project\\/${path}",\\s*element:\\s*<Navigate\\s+to="${to.replace(/[?.]/g, "\\$&")}"\\s+relative="path"\\s+replace\\s*\\/>\\s*\\}`).test(main);
    if (!redirect("schedule", "../plan?tab=planned")) problems.push("main.tsx does not redirect p/:project/schedule to ../plan?tab=planned");
    if (!redirect("gantt", "../plan")) problems.push("main.tsx does not redirect p/:project/gantt to ../plan");
    if (/SchedulePage|GanttPage/.test(main)) problems.push("main.tsx still names SchedulePage or GanttPage");
    for (const f of ["pages/GanttPage.tsx", "pages/SchedulePage.tsx"]) {
      if (existsSync(new URL(`../src/${f}`, import.meta.url))) problems.push(`${f} still exists`);
    }

    const plan = NAV.find((a) => a.key === "plan");
    expect("the Plan area's entries", plan?.entries.map((e) => e.key), ["plan", "releases"]);
    const entry = plan?.entries.find((e) => e.key === "plan");
    expect("the plan entry", [entry?.label, entry?.path("x"), entry?.match], ["Plan", "/p/x/plan", ["/schedule", "/gantt"]]);
    for (const path of ["/p/x/plan", "/p/x/schedule", "/p/x/gantt"]) {
      expect(`${path} marks Plan active`, activeEntry(NAV, "x", path), { area: "plan", entry: "plan" });
    }
  },
);

await check(
  "web::check_portfolio_timeline_has_one_row_per_project",
  "the portfolio reads /portfolio/plan through the shared cache, and its Now timeline draws one lane per project: a running sprint's band, a release's diamond, an empty lane for a project with no cadence, and one today line.",
  async (expect, problems) => {
    const page = src("pages/PortfolioPage.tsx");
    if (!/useQuery<PortfolioPlan>\(\s*null\s*,\s*urls\.portfolioPlan\(\)\s*,\s*\[\s*"\*"\s*\]\s*\)/.test(page)) problems.push('PortfolioPage.tsx does not read urls.portfolioPlan() with useQuery(null, …, ["*"])');
    if (/\bgetGantt\b|portfolio\/gantt/.test(page)) problems.push("PortfolioPage.tsx reads the Mermaid Gantt");

    const plan = {
      from: "2026-11-03",
      to: "2026-12-01",
      today: "2026-11-17",
      lanes: [
        {
          project: "shop",
          label: "Shop",
          bands: [{ from: "2026-11-15", to: "2026-11-28", label: "SP-004", tone: "info", progress: 0.5 }],
          marks: [{ at: "2026-11-24", label: "v0.8.0", kind: "diamond" }],
        },
        { project: "notes", label: "Notes", bands: [], marks: [] },
      ],
    };
    const { document } = await render("src/pages/PortfolioPage.tsx", "PortfolioNow", { plan }, { url: "/portfolio" });
    const svg = document.querySelector(".ui-timeline svg");
    if (!svg) return problems.push("the Now section draws no timeline");
    expect("one lane per project, in order", [...svg.querySelectorAll(".ui-tl-lane-label")].map(text), ["Shop", "Notes"]);
    expect("the running sprint's band", [...svg.querySelectorAll(".ui-tl-band")].length, 1);
    expect("the release in flight", svg.querySelectorAll(".ui-tl-diamond").length, 1);
    expect("one today line", svg.querySelectorAll(".ui-tl-today line").length, 1);

    const none = await render("src/pages/PortfolioPage.tsx", "PortfolioNow", { plan: { ...plan, lanes: [] } }, { url: "/portfolio" });
    if (none.document.querySelector(".ui-timeline")) problems.push("an empty portfolio still draws a timeline");
    if (!none.document.querySelector(".ui-empty-title")) problems.push("an empty portfolio says nothing");
  },
);

await closeRender();

if (failed) process.exit(1);
