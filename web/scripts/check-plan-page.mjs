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
//
// Usage: npm run check:plan-page
import { defaultTab, itemBands, legacyRedirect, openLanes, planTabs, toTimeline } from "../src/planTabs.ts";
import { TAGS, urls } from "../src/data/urls.ts";

let failed = false;
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);

/** Run one named check: its problems, or one line saying what it proved. */
function check(name, proved, body) {
  const problems = [];
  const expect = (what, got, want) => {
    if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  };
  body(expect, problems);
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

check(
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

check(
  "web::check_plan_url_names_the_tab_and_open_rows",
  "the tab and the open rows come from the URL, a bare sprint code opens that sprint, and /schedule and /gantt go to the Plan page.",
  (expect, problems) => {
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
  },
);

if (failed) process.exit(1);
