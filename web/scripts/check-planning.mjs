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

const R1 = "web::check_planning_action_asks_first_and_posts_one_decision";
const R5 = "web::check_a_declined_planning_decision_offers_the_nearest_move";

const problems = { [R1]: [], [R5]: [] };
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
await closeRender();

// ---- the components, rendered in jsdom and pressed ----
const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

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
  ok(R1, "then it shows pending", text().includes("pending: Claude carries it out at its next turn"), text());

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
  ok(R5, "then it shows pending", text().includes("pending: Claude carries it out at its next turn"), text());
  await act(() => root3.unmount());
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
