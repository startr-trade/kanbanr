#!/usr/bin/env node
//
// FTR-061: the Plan page's Flow panel, checked without a browser. Each check prints its name.
//   web::check_plan_flow_shows_running_ready_blocked_and_the_forecast (R-6) — planFlow.ts's helpers
//     word the stats line, each wait (FTR-060's texts), the forecast rows ("4 lanes · 2 build slots
//     → 14 h", "(slots allow 2)" when the slots bind) and agent-hours (minutes under 1 h); the panel
//     rendered with a fixture plan shows the stats, Running (with "1.5 of 2 h" and an "over" pill),
//     Ready (a critical pill and the chain), Blocked (each wait in words), the forecast table with
//     the critical path as its floor, the notes, and asks Claude for tasks when there are none; it
//     reads urls.planFlow through the shared cache, and the Current tab draws it first.
//   web::check_plan_flow_quotes_no_calendar_days (R-7) — the panel's text says no "day" or "days"
//     and holds no date except the chips of dates a person set, each labelled so.
//
// Usage: npm run check:plan-flow
import { readFileSync } from "node:fs";
import { dateChips, forecastRows, hoursText, statsLine, waitText } from "../src/planFlow.ts";
import { TAGS, urls } from "../src/data/urls.ts";
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

const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
const src = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");

const PLAN = {
  scope: "sprint:SP-004",
  total_h: 52,
  done_h: 5.5,
  remaining_h: 46.5,
  critical_path: ["FTR-1/TL-001/T1", "FTR-1/TL-001/T2", "FTR-2/TL-001/T1", "FTR-2/TL-001/T2", "FTR-2/TL-001/T3", "FTR-3/TL-001/T1", "FTR-3/TL-001/T2"],
  critical_h: 12,
  widest: 6,
  profile: [
    { at_h: 0, running: 3 },
    { at_h: 2, running: 6 },
    { at_h: 9, running: 1 },
  ],
  running: [
    { id: "FTR-1/TL-001/T1", text: "The core module", scope: "SC-301", elapsed_h: 1.5, estimate_h: 2, over: false },
    { id: "FTR-2/TL-001/T1", text: "The route", scope: null, elapsed_h: 3, estimate_h: 2, over: true },
  ],
  ready: [
    { id: "FTR-1/TL-001/T2", text: "The CLI", estimate_h: 1.5, critical: true, chain_h: 10.5 },
    { id: "FTR-4/TL-001/T1", text: "The docs", estimate_h: 0.75, critical: false, chain_h: 0.75 },
  ],
  blocked: [
    {
      id: "FTR-2/TL-001/T2",
      waits: [{ Dependency: "FTR-2/TL-001/T1" }, { Conflict: { task: "FTR-1/TL-001/T1", paths: ["api/src/lib.rs"] } }],
    },
    { id: "FTR-5/TL-001/T1", waits: [{ ItemNotAtBuild: "Design" }] },
  ],
  forecasts: [
    { lanes: 1, slots: 2, effective_lanes: 1, finish_h: 46.5 },
    { lanes: 4, slots: 2, effective_lanes: 4, finish_h: 14 },
    { lanes: 8, slots: 2, effective_lanes: 8, finish_h: 12 },
  ],
  build_share: 0.25,
  build_share_measured: false,
  unestimated: ["FTR-4/TL-001/T2", "FTR-4/TL-001/T3"],
  untasked_items: ["FTR-6"],
  outside: [],
  ratios: [],
  dates: [
    { what: "SP-004 ends", date: "2026-12-04" },
    { what: "v0.8.0 target", date: "2026-12-10" },
  ],
};

const EMPTY = {
  ...PLAN,
  total_h: 0,
  done_h: 0,
  remaining_h: 0,
  critical_path: [],
  critical_h: 0,
  widest: 0,
  profile: [],
  running: [],
  ready: [],
  blocked: [],
  forecasts: [
    { lanes: 1, slots: 2, effective_lanes: 1, finish_h: 0 },
    { lanes: 4, slots: 2, effective_lanes: 4, finish_h: 0 },
  ],
  unestimated: [],
  untasked_items: ["FTR-6"],
  dates: [],
};

await check(
  "web::check_plan_flow_shows_running_ready_blocked_and_the_forecast",
  "the helpers word the stats, each wait, the forecast rows and agent-hours; the panel shows running, ready, blocked, the forecast with the critical path as its floor and the notes, asks Claude for tasks when there are none, reads urls.planFlow through the cache, and is drawn first on the Current tab.",
  async (expect, problems) => {
    // The pure helpers.
    expect("hours to one decimal", hoursText(14), "14.0 h");
    expect("minutes under an hour", hoursText(0.75), "45 min");
    expect("an hour", hoursText(1), "1.0 h");
    expect("the stats line", statsLine(PLAN), "Remaining 46.5 agent-h · critical path 12.0 h over 7 tasks · up to 6 at once");
    expect("a dependency", waitText("FTR-2/TL-001/T2", { Dependency: "FTR-2/TL-001/T1" }), "waits on FTR-2/TL-001/T1");
    expect(
      "a write conflict",
      waitText("FTR-2/TL-001/T2", { Conflict: { task: "FTR-1/TL-001/T1", paths: ["api/src/lib.rs", "web/src/api.ts"] } }),
      "writes api/src/lib.rs, web/src/api.ts, which FTR-1/TL-001/T1 is writing",
    );
    expect(
      "a conflict with no files named",
      waitText("FTR-2/TL-001/T2", { Conflict: { task: "FTR-2/TL-001/T1", paths: [] } }),
      "FTR-2/TL-001/T1 is in progress in the same item, and its files are not recorded",
    );
    expect("an item not at build", waitText("FTR-5/TL-001/T1", { ItemNotAtBuild: "Design" }), "FTR-5 is in Design, not at the build stage");
    expect(
      "an unknown footprint",
      waitText("FTR-5/TL-001/T2", { UnknownFootprint: "FTR-5/TL-001/T1" }),
      "it records no files, and FTR-5/TL-001/T1 of its item is in progress",
    );
    expect(
      "the forecast rows",
      forecastRows({ ...PLAN, forecasts: [...PLAN.forecasts, { lanes: 8, slots: 1, effective_lanes: 2, finish_h: 24.3 }] }).map((r) => r.text),
      ["1 lane · 2 build slots → 46.5 h", "4 lanes · 2 build slots → 14.0 h", "8 lanes · 2 build slots → 12.0 h", "8 lanes · 1 build slot → 24.3 h (slots allow 2)"],
    );
    expect(
      "the date chips",
      dateChips(PLAN),
      ["SP-004 ends 4 Dec 2026 (set by the person)", "v0.8.0 target 10 Dec 2026 (set by the person)"],
    );

    // The URL and the cache.
    const end = (what, url, want) => {
      if (!url.endsWith(want)) problems.push(`${what} is ${url}, wanted …${want}`);
    };
    end("urls.planFlow with no scope", urls.planFlow("x"), "/projects/x/plan/flow");
    end("urls.planFlow with a scope", urls.planFlow("x", "sprint:SP-004"), "/projects/x/plan/flow?scope=sprint%3ASP-004");
    expect("the flow read's tags", [...TAGS.planFlow], ["items", "sprints", "releases", "tasklog"]);
    const panel = src("pages/plan/FlowPanel.tsx");
    if (!/useQuery<FlowPlan>\(\s*project\s*,\s*urls\.planFlow\(project\)\s*,\s*TAGS\.planFlow\s*\)/.test(panel)) problems.push("FlowPanel.tsx does not read urls.planFlow(project) with TAGS.planFlow");
    for (const bad of [/\bapi\.getProject\s*\(/, /\buseAsync\b/]) if (bad.test(panel)) problems.push(`FlowPanel.tsx has ${bad}: reads go through useQuery and urls`);
    const page = src("pages/PlanPage.tsx");
    if (!/key === "current"[^\n]*<FlowPanel project=\{p\.project\} \/>/.test(page)) problems.push("PlanPage.tsx does not draw <FlowPanel/> on the Current tab");
    if (!src("api.ts").includes("getJson<FlowPlan>(urls.planFlow(")) problems.push("api.ts has no getPlanFlow read of FlowPlan");

    // The panel rendered.
    const { document } = await render("src/pages/plan/FlowPanel.tsx", "FlowPanelBody", { project: "shop", plan: PLAN }, { url: "/p/shop/plan" });
    const root = document.querySelector(".plan-flow");
    if (!root) return problems.push("the panel draws no .plan-flow");
    expect("the panel's heading", text(root.querySelector("h2")), "Flow");
    expect("the stats line", text(root.querySelector(".plan-flow-stats")), statsLine(PLAN));
    if (!/over tasks, not items/.test(text(root))) problems.push("the panel does not say its critical path is over tasks");
    const rows = (sel) => [...root.querySelectorAll(`${sel} li`)].map(text);
    const running = rows(".plan-flow-running");
    expect("running rows", running.length, 2);
    if (!/^FTR-1\/TL-001\/T1 The core module SC-301 1\.5 h of 2\.0 h$/.test(running[0] ?? "")) problems.push(`the first running row is "${running[0]}"`);
    if (!/over$/.test(running[1] ?? "") || !root.querySelector(".plan-flow-running .ui-pill")) problems.push(`the over-running task has no "over" pill: "${running[1]}"`);
    const ready = rows(".plan-flow-ready");
    expect("ready rows", ready, ["FTR-1/TL-001/T2 The CLI critical path chain 10.5 h", "FTR-4/TL-001/T1 The docs chain 45 min"]);
    expect(
      "blocked waits in words",
      [...root.querySelectorAll(".plan-flow-blocked li li")].map(text),
      ["waits on FTR-2/TL-001/T1", "writes api/src/lib.rs, which FTR-1/TL-001/T1 is writing", "FTR-5 is in Design, not at the build stage"],
    );
    const forecast = [...root.querySelectorAll(".plan-flow-forecast tbody tr")].map(text);
    expect("one forecast row per lane count", forecast.length, 3);
    if (!forecast.some((r) => /^4 lanes · 2 build slots → 14\.0 h/.test(r))) problems.push(`no 4-lane row: ${JSON.stringify(forecast)}`);
    if (!/critical path 12\.0 h: the least it can take however many lanes run/.test(text(root.querySelector(".plan-flow-forecast")))) problems.push("the forecast does not name the critical path as its floor");
    const notes = text(root.querySelector(".plan-flow-notes"));
    for (const want of ["2 tasks have no estimate (counted as 1 h)", "1 item has no tasks", "build share 0.25 (assumed)"]) {
      if (!notes.includes(want)) problems.push(`the notes lack "${want}": "${notes}"`);
    }

    const empty = await render("src/pages/plan/FlowPanel.tsx", "FlowPanelBody", { project: "shop", plan: EMPTY }, { url: "/p/shop/plan" });
    expect("no tasks: the empty title", text(empty.document.querySelector(".ui-empty-title")), "No tasks to plan.");
    expect("its ask", text(empty.document.querySelector(".ui-empty-sentence")), "Ask Claude to break the sprint's items into tasks.");
  },
);

await check(
  "web::check_plan_flow_quotes_no_calendar_days",
  "the panel's text has no day or days, and the only dates in it are the chips of dates a person set.",
  async (expect, problems) => {
    for (const plan of [PLAN, { ...PLAN, dates: [] }, EMPTY]) {
      const { document } = await render("src/pages/plan/FlowPanel.tsx", "FlowPanelBody", { project: "shop", plan }, { url: "/p/shop/plan" });
      const root = document.querySelector(".plan-flow");
      const all = text(root);
      if (/\bdays?\b/i.test(all)) problems.push(`the panel says day or days: "${all}"`);
      const chips = [...root.querySelectorAll(".plan-flow-dates li")].map(text);
      expect("the chips are the plan's dates", chips, dateChips(plan));
      // Everything but the chips: no ISO date and no "4 Dec 2026"-style date.
      const clone = root.cloneNode(true);
      for (const c of clone.querySelectorAll(".plan-flow-dates")) c.remove();
      const rest = text(clone);
      const date = /\b\d{4}-\d{2}-\d{2}\b|\b\d{1,2} (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4}\b/;
      if (date.test(rest)) problems.push(`a date outside the person's chips: "${rest.match(date)[0]}"`);
      for (const c of chips) if (!/\(set by the person\)$/.test(c)) problems.push(`a chip not labelled as the person's: "${c}"`);
    }
  },
);

await closeRender();

if (failed) process.exit(1);
