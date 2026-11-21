#!/usr/bin/env node
//
// The person's planning decisions from the pages (FTR-049).
//
// web::check_planning_action_asks_first_and_posts_one_decision (R-1): `requestText` words the
// question; PlanningAction draws a select of targets and one button named for the chosen target,
// is hidden on a read-only monitor, asks first on the first press (DEF-034's AskFirst), posts one
// decision on the second press (guardInFlight, DEF-090) and then shows that it is pending.
//
// web::check_a_declined_planning_decision_offers_the_nearest_move (R-5): `nearestRequest` turns a
// declined decision's nearest move into a request for the same items, and DeclinedPlanning shows
// the reason and posts that request on one press.
//
// web::check_plan_rows_show_pending_planning_decisions (R-3): `pendingByTarget` and
// `pendingByItem` key the pending decisions; `toTimeline` marks a lane's band pending when one
// waits on it, and the Plan tab draws it dashed; an opened row shows "pending: into SP-004 (PD-1)"
// on each item it names, the last declined decision's reason, a checkbox per item and one
// PlanningAction for the selection, offering the open sprints and releases but not the row's own;
// a read-only monitor shows the pills and no selection.
//
// web::check_plan_drag_drafts_a_request_and_moves_nothing (R-7): an item dragged from an opened
// row and dropped on another sprint or release row drafts the request (`dropRequest`) and opens the
// PlanningAction question with that item and target; the drop posts nothing, the second press does.
//
// The components are rendered to markup through scripts/lib/render.mjs, and rendered in jsdom and
// pressed, with `fetch` replaced by a recorder, so the posts are the ones api.ts would send.
//
// Usage: npm run check:planning
import { JSDOM } from "jsdom";
import { createServer } from "vite";
import { render, closeRender } from "./lib/render.mjs";
import {
  requestText,
  pendingByTarget,
  pendingByItem,
  nearestRequest,
  nearestLabel,
  dropRequest,
  targetRequest,
} from "../src/planning.ts";
import { declinedByItem, planTargets, toTimeline } from "../src/planTabs.ts";

const R1 = "web::check_planning_action_asks_first_and_posts_one_decision";
const R5 = "web::check_a_declined_planning_decision_offers_the_nearest_move";
const R3 = "web::check_plan_rows_show_pending_planning_decisions";

const R7 = "web::check_plan_drag_drafts_a_request_and_moves_nothing";

const problems = { [R1]: [], [R3]: [], [R5]: [], [R7]: [] };
const expect = (test, name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems[test].push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};
const ok = (test, name, cond, got) => {
  if (!cond) problems[test].push(`${name}: ${JSON.stringify(got)}`);
};

const decision = (id, action, target, items, extra = {}) => ({
  id,
  action,
  target,
  items,
  by: "Ada",
  origin: "monitor",
  at: "2026-11-16T14:02:00Z",
  ...extra,
});
const pending = (d) => ({ decision: d, state: "pending", outcome: null });
const declined = (d, reason, nearest) => ({
  decision: d,
  state: "declined",
  outcome: { decision: d.id, outcome: "declined", by: "Claude", origin: "conversation", at: "2026-11-16T15:00:00Z", reason, ...(nearest ? { nearest } : {}) },
});

// ---- R-1: the pure model ----
expect(R1, "two items into a sprint", requestText("into-sprint", "SP-004", ["FTR-044", "DEF-076"]), "Plan FTR-044 and DEF-076 into SP-004?");
expect(R1, "one item into a sprint", requestText("into-sprint", "SP-004", ["FTR-044"]), "Plan FTR-044 into SP-004?");
expect(R1, "three items to a release", requestText("into-release", "v0.9.0", ["A-1", "B-2", "C-3"]), "Move A-1, B-2 and C-3 to v0.9.0?");
expect(R1, "out of the sprint", requestText("out-of-sprint", "", ["FTR-044"]), "Take FTR-044 out of its sprint?");
expect(R1, "a sprint key", targetRequest("sprint:SP-004", ["X-1"]), { action: "into-sprint", target: "SP-004", items: ["X-1"] });
expect(R1, "a release key", targetRequest("release:v0.9.0", ["X-1"]), { action: "into-release", target: "v0.9.0", items: ["X-1"] });
expect(R1, "an unknown key", targetRequest("milestone:MS-1", ["X-1"]), null);
expect(R1, "no items", targetRequest("sprint:SP-004", []), null);
// dropRequest is the drag's (T8); its lanes are the same keys.
expect(R1, "a drop on a sprint lane", dropRequest("X-1", "sprint:SP-004"), { action: "into-sprint", target: "SP-004", items: ["X-1"] });
expect(R1, "a drop on another lane", dropRequest("X-1", "backlog"), null);

const views = [
  pending(decision("PD-1", "into-sprint", "SP-004", ["FTR-044", "DEF-076"])),
  pending(decision("PD-2", "into-release", "v0.9.0", ["DEF-076"])),
  declined(decision("PD-3", "into-release", "v0.7.0", ["FTR-044"]), "v0.7.0 is at candidate; its scope is frozen", { action: "into-release", target: "v0.8.0" }),
  { decision: decision("PD-4", "into-sprint", "SP-004", ["X-9"]), state: "carried", outcome: null },
];
const byTarget = pendingByTarget(views);
expect(R1, "pending by target: keys", [...byTarget.keys()], ["sprint:SP-004", "release:v0.9.0"]);
expect(R1, "pending by target: the sprint's", byTarget.get("sprint:SP-004")?.map((v) => v.decision.id), ["PD-1"]);
const byItem = pendingByItem(views);
expect(R1, "pending by item", byItem.get("DEF-076")?.map((v) => v.decision.id), ["PD-1", "PD-2"]);
ok(R1, "a declined or carried one is not pending", !byItem.has("X-9") && byItem.get("FTR-044")?.length === 1, [...byItem.keys()]);

// ---- R-5: the pure model ----
expect(R5, "nearest of a declined decision", nearestRequest(views[2]), { action: "into-release", target: "v0.8.0", items: ["FTR-044"] });
expect(R5, "a pending one has no nearest", nearestRequest(views[0]), null);
expect(R5, "a declined one with no nearest", nearestRequest(declined(decision("PD-5", "into-sprint", "SP-1", ["A-1"]), "closed")), null);
expect(R5, "the offer's words", nearestLabel(views[2]), "Ask for v0.8.0 instead");

// ---- the components, rendered to markup ----
const targets = [
  { key: "sprint:SP-004", label: "SP-004" },
  { key: "release:v0.9.0", label: "v0.9.0" },
];
const base = { project: "demo", items: ["FTR-044", "DEF-076"], targets, writable: true, who: "Ada" };
{
  const { document } = await render("src/components/PlanningAction.tsx", "PlanningAction", base);
  const options = [...document.querySelectorAll("select option")].map((o) => o.textContent);
  expect(R1, "a select of the targets", options, ["SP-004", "v0.9.0"]);
  const buttons = [...document.querySelectorAll("button")].map((b) => b.textContent);
  expect(R1, "at rest: one button, named for the first target", buttons, ["Plan into SP-004"]);
  const ro = await render("src/components/PlanningAction.tsx", "PlanningAction", { ...base, writable: false });
  expect(R1, "a read-only monitor shows no action", ro.html, "");
  const none = await render("src/components/PlanningAction.tsx", "PlanningAction", { ...base, items: [] });
  ok(R1, "no selection: the button is disabled and says why", none.document.querySelector("button")?.disabled === true && /select/i.test(none.document.querySelector("button")?.title ?? ""), none.html);
}
{
  const { document } = await render("src/components/PlanningAction.tsx", "DeclinedPlanning", { project: "demo", view: views[2], writable: true, who: "Ada" });
  const text = document.body.textContent;
  ok(R5, "the reason is shown", text.includes("declined: v0.7.0 is at candidate; its scope is frozen"), text);
  expect(R5, "one button, the nearest move", [...document.querySelectorAll("button")].map((b) => b.textContent), ["Ask for v0.8.0 instead"]);
  const ro = await render("src/components/PlanningAction.tsx", "DeclinedPlanning", { project: "demo", view: views[2], writable: false, who: "Ada" });
  ok(R5, "read-only: the reason, no button", ro.document.querySelectorAll("button").length === 0 && ro.document.body.textContent.includes("declined:"), ro.html);
}

// ---- R-3: pending decisions on the Plan page's lanes and rows ----
{
  expect(R3, "pending by target: the sprint and the release", [...byTarget.keys()], ["sprint:SP-004", "release:v0.9.0"]);
  expect(R3, "pending by item: both decisions naming DEF-076", byItem.get("DEF-076")?.map((v) => v.decision.id), ["PD-1", "PD-2"]);
  ok(R3, "a declined or carried decision is not pending", !byItem.has("X-9") && !byTarget.has("release:v0.7.0"), [...byTarget.keys()]);

  const laneOf = (key, kind, state, bands = []) => ({ key, kind, label: key.slice(key.indexOf(":") + 1), state, from: "2026-11-16", to: "2026-11-29", bands, marks: [], done: 0, total: 2, points_done: null, points_total: null, carried: 0, burndown: null });
  const sprintBand = { from: "2026-11-16", to: "2026-11-29", label: "SP-004", tone: "info", progress: 0.5 };
  const slice = {
    scope: "current",
    today: "2026-11-17",
    from: "2026-11-03",
    to: "2026-12-01",
    lanes: [
      laneOf("sprint:SP-004", "sprint", "active", [sprintBand]),
      laneOf("sprint:SP-005", "sprint", "planned", [{ ...sprintBand, label: "SP-005" }]),
      laneOf("release:v0.8.0", "release", "candidate"),
      laneOf("release:v0.9.0", "release", "planned"),
      laneOf("sprint:SP-003", "sprint", "closed", [{ ...sprintBand, label: "SP-003" }]),
    ],
    undated: [],
  };
  const t = toTimeline(slice, byTarget);
  const pendingOf = (key) => t.lanes.find((l) => l.key === key)?.bands.map((b) => b.pending);
  expect(R3, "the lane a decision plans into is pending", pendingOf("sprint:SP-004"), [true]);
  expect(R3, "a lane with none is not", pendingOf("sprint:SP-005"), [false]);
  expect(R3, "toTimeline with no decisions marks nothing", toTimeline(slice).lanes.flatMap((l) => l.bands.map((b) => b.pending)), [false, false, false]);

  const indexSprints = [
    { code: "SP-003", name: "", start: "2026-11-02", end: "2026-11-15", state: "closed" },
    { code: "SP-004", name: "", start: "2026-11-16", end: "2026-11-29", state: "active" },
    { code: "SP-005", name: "", start: "2026-11-30", end: "2026-12-13", state: "planned" },
  ];
  const indexReleases = [
    { version: "v0.7.0", state: "shipped" },
    { version: "v0.8.0", state: "planned", stage: "candidate" },
    { version: "v0.9.0", state: "planned" },
    { version: "v0.6.9", state: "superseded" },
  ];
  expect(R3, "targets: open sprints and planned releases, never frozen or closed", planTargets(indexSprints, indexReleases), [
    { key: "sprint:SP-004", label: "SP-004" },
    { key: "sprint:SP-005", label: "SP-005" },
    { key: "release:v0.9.0", label: "v0.9.0" },
  ]);
  const old = declined(decision("PD-6", "into-sprint", "SP-003", ["DEF-076"]), "SP-003 is closed", null);
  old.outcome.at = "2026-10-01T10:00:00Z";
  const recent = declinedByItem([...views, old], "2026-11-17");
  expect(R3, "declined by item: the last 14 days only", [...recent.keys()], ["FTR-044"]);

  const planning = { views, targets: planTargets(indexSprints, indexReleases), writable: true, who: "Ada" };
  const page = await render("src/pages/PlanPage.tsx", "PlanTabBody", { project: "demo", scope: "current", slice, planning }, { url: "/p/demo/plan" });
  const dashed = [...page.document.querySelectorAll(".ui-tl-band.ui-tl-pending")].map((b) => b.textContent);
  ok(R3, "the tab draws the pending lane's band dashed", dashed.length === 1 && dashed[0].includes("SP-004"), page.html.slice(0, 400));
  ok(R3, "the row says how many decisions wait on it", page.document.body.textContent.includes("1 pending decision"), page.document.body.textContent);

  const body = {
    key: "sprint:SP-005",
    items: [
      { code: "FTR-044", title: "Forty-four", status: "Planned", from: "2026-11-16", to: "2026-11-29", placed: "sprint", critical: false, pending: ["PD-1"] },
      { code: "DEF-076", title: "Seventy-six", status: "Planned", from: "", to: "", placed: "undated", critical: false, pending: ["PD-1", "PD-2"] },
      { code: "X-1", title: "Other", status: "Planned", from: "", to: "", placed: "undated", critical: false, pending: [] },
    ],
    by_status: [],
    carried: [],
  };
  const row = { project: "demo", lane: slice.lanes[1], from: slice.from, to: slice.to, today: slice.today, body, planning };
  const { document } = await render("src/pages/plan/RowItems.tsx", "RowItemsBody", row, { url: "/p/demo/plan" });
  const cell = (code) => [...document.querySelectorAll("table.plan-items tbody tr")].find((r) => r.textContent.includes(code))?.textContent ?? "";
  ok(R3, "the item's pill names the target and the decision", cell("FTR-044").includes("pending: into SP-004 (PD-1)"), cell("FTR-044"));
  ok(R3, "an item in two decisions shows both", cell("DEF-076").includes("pending: into SP-004 (PD-1)") && cell("DEF-076").includes("pending: into v0.9.0 (PD-2)"), cell("DEF-076"));
  ok(R3, "an item with none shows no pill", !cell("X-1").includes("pending:"), cell("X-1"));
  ok(R3, "the declined one shows its reason", cell("FTR-044").includes("declined: v0.7.0 is at candidate; its scope is frozen"), cell("FTR-044"));
  expect(R3, "a checkbox per item", [...document.querySelectorAll('table.plan-items input[type="checkbox"]')].map((c) => c.getAttribute("aria-label")), ["Select FTR-044", "Select DEF-076", "Select X-1"]);
  expect(R3, "one action, offering every open target but the row's own", [...document.querySelectorAll(".plan-planning select option")].map((o) => o.textContent), ["SP-004", "v0.9.0"]);

  const ro = await render("src/pages/plan/RowItems.tsx", "RowItemsBody", { ...row, planning: { ...planning, writable: false } }, { url: "/p/demo/plan" });
  ok(R3, "read-only: the pills, no selection or action", ro.document.querySelectorAll('input[type="checkbox"], .plan-planning').length === 0 && ro.document.body.textContent.includes("pending: into SP-004 (PD-1)"), ro.html.slice(0, 400));
}

// ---- R-1: "Move to…" on the Releases page's item table (FTR-049 T7) ----
{
  const rel = { version: "v0.8.0", state: "planned", items: [] };
  const rows = [
    { code: "FTR-044", title: "Forty-four", kind: "feature", status: "Planned", sprint: "", tests_green: 0, tests_named: 0, approval: "none", signoff: "" },
    { code: "DEF-076", title: "Seventy-six", kind: "defect", status: "Planned", sprint: "", tests_green: 0, tests_named: 0, approval: "none", signoff: "" },
  ];
  const props = { project: "demo", release: rel, rows, writable: true, who: "Ada", targets: [{ key: "release:v0.9.0", label: "v0.9.0" }], empty: "none" };
  const at = (p) => render("src/pages/releases/ReleaseBody.tsx", "ReleaseItems", p, { url: "/p/demo/releases" });
  const { document } = await at(props);
  expect(R1, "releases: a checkbox per item", [...document.querySelectorAll('table input[type="checkbox"]')].map((c) => c.getAttribute("aria-label")), ["Select FTR-044", "Select DEF-076"]);
  expect(R1, "releases: one action, Move to the other release", [...document.querySelectorAll(".release-move button")].map((b) => b.textContent), ["Move to v0.9.0"]);
  const ro = await at({ ...props, writable: false });
  ok(R1, "releases: read-only shows the plain table", ro.document.querySelectorAll('input[type="checkbox"], .release-move').length === 0 && ro.document.querySelectorAll("table").length === 1, ro.html.slice(0, 300));
  const alone = await at({ ...props, targets: [] });
  ok(R1, "releases: no other planned release, no selection", alone.document.querySelectorAll('input[type="checkbox"], .release-move').length === 0, alone.html.slice(0, 300));
}
await closeRender();

// ---- the components, rendered in jsdom and pressed ----
const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
// The monitor listens for live changes; there is no daemon here.
globalThis.EventSource = class { addEventListener() {} removeEventListener() {} close() {} };

const posts = [];
let answer = () => ({ ...decision("PD-7", "into-sprint", "SP-004", ["FTR-044", "DEF-076"]) });
globalThis.fetch = async (url, init) => {
  posts.push({ url: String(url), method: init?.method, body: init?.body ? JSON.parse(init.body) : null });
  return new Response(JSON.stringify(answer()), { status: 200, headers: { "content-type": "application/json" } });
};

const webRoot = new URL("..", import.meta.url).pathname;
const vite = await createServer({
  root: webRoot,
  logLevel: "error",
  appType: "custom",
  server: { middlewareMode: true, hmr: false, watch: null },
  ssr: { resolve: { externalConditions: ["module-sync"] } },
});
try {
  const mod = await vite.ssrLoadModule("/src/components/PlanningAction.tsx");
  const React = (await import("react")).default;
  const { createRoot } = await import("react-dom/client");
  const act = React.act;
  const root = createRoot(document.getElementById("root"));
  const show = (C, props) => act(() => root.render(React.createElement(C, props)));
  const buttons = () => [...document.querySelectorAll("button")].map((b) => b.textContent);
  const text = () => document.getElementById("root").textContent;
  const press = (label) =>
    act(async () => {
      const b = [...document.querySelectorAll("button")].find((x) => x.textContent === label);
      if (!b) throw new Error(`no button "${label}" among ${JSON.stringify(buttons())}`);
      b.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    });

  await show(mod.PlanningAction, base);
  await press("Plan into SP-004");
  expect(R1, "the first press posts nothing", posts.length, 0);
  ok(R1, "the first press asks", text().includes("Plan FTR-044 and DEF-076 into SP-004?"), text());
  expect(R1, "the question has the same button and Cancel", buttons(), ["Plan into SP-004", "Cancel"]);
  await press("Cancel");
  expect(R1, "Cancel posts nothing", posts.length, 0);

  // Choosing another target renames the button.
  await act(async () => {
    const sel = document.querySelector("select");
    sel.value = "release:v0.9.0";
    sel.dispatchEvent(new window.Event("change", { bubbles: true }));
  });
  expect(R1, "a release target: Move to", buttons(), ["Move to v0.9.0"]);
  await act(async () => {
    const sel = document.querySelector("select");
    sel.value = "sprint:SP-004";
    sel.dispatchEvent(new window.Event("change", { bubbles: true }));
  });

  await press("Plan into SP-004");
  await press("Plan into SP-004");
  expect(R1, "the second press posts one decision", posts.length, 1);
  expect(R1, "to the planning write route", posts[0]?.url, "/api/write/projects/demo/planning");
  expect(R1, "as a POST", posts[0]?.method, "POST");
  expect(R1, "naming the action, target, items and the person", posts[0]?.body, {
    action: "into-sprint",
    target: "SP-004",
    items: ["FTR-044", "DEF-076"],
    by: "Ada",
  });
  ok(R1, "then it shows pending", text().includes("pending: the agent carries it out at its next turn"), text());

  // A refusal is shown, not swallowed.
  posts.length = 0;
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    posts.push({ url: String(url), method: init?.method, body: init?.body ? JSON.parse(init.body) : null });
    return new Response("there is no sprint 'SP-404'", { status: 400 });
  };
  await act(() => root.unmount());
  const root2 = createRoot(document.getElementById("root"));
  await act(() => root2.render(React.createElement(mod.PlanningAction, base)));
  await press("Plan into SP-004");
  await press("Plan into SP-004");
  ok(R1, "a refusal is shown", text().includes("there is no sprint 'SP-404'"), text());
  ok(R1, "and is not called pending", !text().includes("pending:"), text());
  globalThis.fetch = realFetch;
  await act(() => root2.unmount());

  // R-5: one press asks for the nearest move.
  posts.length = 0;
  answer = () => decision("PD-8", "into-release", "v0.8.0", ["FTR-044"]);
  const root3 = createRoot(document.getElementById("root"));
  await act(() => root3.render(React.createElement(mod.DeclinedPlanning, { project: "demo", view: views[2], writable: true, who: "Ada" })));
  await press("Ask for v0.8.0 instead");
  expect(R5, "one press posts one decision", posts.length, 1);
  expect(R5, "the nearest move, for the same items", posts[0]?.body, { action: "into-release", target: "v0.8.0", items: ["FTR-044"], by: "Ada" });
  ok(R5, "then it shows pending", text().includes("pending: the agent carries it out at its next turn"), text());
  await act(() => root3.unmount());

  // R-7: a drag drafts a request and moves nothing.
  // R-7: the pure model.
  const { draftDrop, ITEM_DRAG } = await vite.ssrLoadModule("/src/pages/plan/RowItems.tsx");
  const open = [{ key: "sprint:SP-004" }, { key: "sprint:SP-005" }, { key: "release:v0.9.0" }];
  const carry = (item, from) => JSON.stringify({ item, from });
  expect(R7, "a drop on a sprint lane drafts into-sprint", draftDrop(carry("FTR-044", "sprint:SP-005"), "sprint:SP-004", open), { action: "into-sprint", target: "SP-004", items: ["FTR-044"] });
  expect(R7, "a drop on a release lane drafts into-release", draftDrop(carry("FTR-044", "sprint:SP-005"), "release:v0.9.0", open), { action: "into-release", target: "v0.9.0", items: ["FTR-044"] });
  expect(R7, "a drop on a lane not offered (frozen) drafts nothing", draftDrop(carry("FTR-044", "sprint:SP-005"), "release:v0.8.0", open), null);
  expect(R7, "a drop on its own lane drafts nothing", draftDrop(carry("FTR-044", "sprint:SP-004"), "sprint:SP-004", open), null);
  expect(R7, "a drop on a milestone lane drafts nothing", draftDrop(carry("FTR-044", "sprint:SP-004"), "milestone:MS-1", [...open, { key: "milestone:MS-1" }]), null);
  expect(R7, "data that is not an item's drafts nothing", [draftDrop("", "sprint:SP-004", open), draftDrop("{}", "sprint:SP-004", open), draftDrop("text", "sprint:SP-004", open)], [null, null, null]);
  {
    const { MemoryRouter } = await import("react-router-dom");
    const page = await vite.ssrLoadModule("/src/pages/PlanPage.tsx");
    const laneOf = (key, kind, state, label) => ({ key, kind, label, state, from: "2026-11-16", to: "2026-11-29", bands: [], marks: [], done: 0, total: 2, points_done: null, points_total: null, carried: 0, burndown: null });
    const slice = {
      scope: "planned",
      today: "2026-11-17",
      from: "2026-11-03",
      to: "2026-12-01",
      lanes: [laneOf("sprint:SP-004", "sprint", "active", "SP-004"), laneOf("sprint:SP-005", "sprint", "planned", "SP-005"), laneOf("release:v0.9.0", "release", "planned", "v0.9.0")],
      undated: [],
    };
    const rowBody = { key: "sprint:SP-005", items: [{ code: "FTR-044", title: "Forty-four", status: "Planned", from: "2026-11-30", to: "2026-12-05", placed: "sprint", critical: false, pending: [] }], by_status: [], carried: [] };
    const planning = { views: [], targets: [{ key: "sprint:SP-004", label: "SP-004" }, { key: "sprint:SP-005", label: "SP-005" }, { key: "release:v0.9.0", label: "v0.9.0" }], writable: true, who: "Ada" };
    posts.length = 0;
    globalThis.fetch = async (url, init) => {
      posts.push({ url: String(url), method: init?.method ?? "GET", body: init?.body ? JSON.parse(init.body) : null });
      const body = String(url).includes("/plan/rows/") ? rowBody : decision("PD-9", "into-sprint", "SP-004", ["FTR-044"]);
      return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
    };
    const writes = () => posts.filter((p) => p.method === "POST");
    const root4 = createRoot(document.getElementById("root"));
    await act(async () => root4.render(React.createElement(MemoryRouter, { initialEntries: ["/p/demo/plan#sprint:SP-005"] }, React.createElement(page.PlanTabBody, { project: "demo", scope: "planned", slice, planning }))));
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
    const row = [...document.querySelectorAll("table.plan-items tbody tr")].find((r) => r.textContent.includes("FTR-044"));
    ok(R7, "an item row in an opened row is draggable", row?.getAttribute("draggable") === "true", document.getElementById("root").innerHTML.slice(0, 300));
    const store = new Map();
    const dataTransfer = { types: [], effectAllowed: "", setData(t, v) { store.set(t, v); this.types = [...store.keys()]; }, getData: (t) => store.get(t) ?? "" };
    const fire = (el, type) => {
      const ev = new window.Event(type, { bubbles: true, cancelable: true });
      Object.defineProperty(ev, "dataTransfer", { value: dataTransfer });
      return act(async () => { el.dispatchEvent(ev); });
    };
    await fire(row, "dragstart");
    ok(R7, "the drag carries the item and its lane", store.get(ITEM_DRAG) === JSON.stringify({ item: "FTR-044", from: "sprint:SP-005" }), [...store.entries()]);
    const zones = [...document.querySelectorAll(".plan-drop-zone")];
    expect(R7, "a drop zone per row", zones.length, 3);
    // Its own lane: nothing is drafted.
    await fire(zones[1], "drop");
    ok(R7, "dropped on its own row: no question", !document.querySelector(".plan-drop"), document.getElementById("root").textContent);
    // Another sprint: the question opens, armed, and nothing is posted.
    await fire(zones[0], "drop");
    const drop = document.querySelector(".plan-drop");
    ok(R7, "the drop opens the question for that item and target", !!drop && drop.textContent.includes("Plan FTR-044 into SP-004?"), drop?.textContent);
    expect(R7, "the question has the confirm, Cancel and Dismiss", [...drop.querySelectorAll("button")].map((b) => b.textContent), ["Plan into SP-004", "Cancel", "Dismiss"]);
    expect(R7, "the drop posts nothing", writes().length, 0);
    await act(async () => { [...drop.querySelectorAll("button")].find((b) => b.textContent === "Plan into SP-004").dispatchEvent(new window.MouseEvent("click", { bubbles: true })); });
    expect(R7, "the second press posts one decision", writes().map((p) => p.body), [{ action: "into-sprint", target: "SP-004", items: ["FTR-044"], by: "Ada" }]);
    await act(() => root4.unmount());
    // A read-only monitor: no drag, no drop zone behaviour.
    const root5 = createRoot(document.getElementById("root"));
    await act(async () => root5.render(React.createElement(MemoryRouter, { initialEntries: ["/p/demo/plan#sprint:SP-005"] }, React.createElement(page.PlanTabBody, { project: "demo", scope: "planned", slice, planning: { ...planning, writable: false } }))));
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
    ok(R7, "read-only: nothing is draggable", document.querySelectorAll('[draggable="true"]').length === 0, document.getElementById("root").innerHTML.slice(0, 200));
    await act(() => root5.unmount());
  }
} finally {
  await vite.close();
}

let failed = false;
for (const [name, list] of Object.entries(problems)) {
  if (list.length) {
    failed = true;
    for (const p of list) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}
process.exit(failed ? 1 : 0);
