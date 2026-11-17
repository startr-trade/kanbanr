#!/usr/bin/env node
//
// The Board (FTR-078): each lane is grouped by what its items need next, not paged. The pure model
// the page renders with (`src/boardModel.ts`) is checked case by case:
//   web::check_board_every_item_is_in_exactly_one_group (R-1, model half)
//   web::check_board_start_lane_groups_follow_their_rules (R-2)
//   web::check_board_waiting_on_you_agrees_with_the_card_hint (R-2)
//   web::check_board_active_lane_groups_follow_their_rules (R-3)
//   web::check_board_finished_lane_shows_the_current_window_only (R-4)
//   web::check_board_flow_line_counts_the_last_seven_days (R-8)
//   web::check_board_insight_is_one_line_chosen_by_rule (R-9)
//   web::check_board_card_fact_follows_the_priority_order (R-12, model half)
//   web::check_board_last_look_is_kept_in_this_browser (R-13, model half)
//   web::check_board_groups_order_by_closeness_newest_or_priority (R-15)
//   web::check_board_renders_450_items_within_budget (R-19, the 20 ms half)
//
// Usage: npm run check:board
import {
  boardLanes,
  checksLeftText,
  cardFact,
  closeness,
  finishedWindow,
  groupId,
  groupOf,
  isGroupOpen,
  laneInsight,
  laneKinds,
  releaseInProgress,
  seenKey,
  sortItems,
  stepperSteps,
  toggleGroup,
  flowText,
  SEEN_AFTER_MS,
} from "../src/boardModel.ts";
import { nextStageHint } from "../src/cardText.ts";

let ok = true;
function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    ok = false;
  } else console.log(`${name} — ok`);
}
const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
};

// ---- fixtures -------------------------------------------------------------------------------
const NOW = new Date("2026-11-17T12:00:00Z");
const day = (n) => new Date(NOW.getTime() - n * 86_400_000).toISOString().slice(0, 10);
const iso = (n) => new Date(NOW.getTime() - n * 86_400_000).toISOString();

const config = (over = {}) => ({
  name: "demo",
  description: "",
  statuses: ["Planned", "In Progress", "Completed"],
  displayed_states: ["Planned", "In Progress", "Completed"],
  terminal_states: ["Completed"],
  cadence: { sprints: false, releases: false, git_model: false },
  codes: {},
  ...over,
});
const item = (code, status, over = {}) => ({ code, title: `Title ${code}`, status, milestone: "MS-1", done: 0, total: 0, created_at: iso(30), entered_on: day(3), ...over });
const next = (over = {}) => ({ status: "Completed", missing: 3, signoffs: [], tasks_open: 0, ends: true, immediate: true, for_you: 0, ...over });
const card = (nexts, over = {}) => ({ gaps: [], next: nexts, updated_at: iso(1), req_proven: 0, req_total: 0, ...over });
const index = (items, over = {}) => ({ project: "p", config: config(), items, milestones: [], docs: [], goals: [], ...over });
const input = (idx, readiness, over = {}) => ({ project: "p", index: idx, readiness, scopes: [], critical: [], seen: null, now: NOW, sprint: "all", text: "", sort: "close", ...over });
const laneOf = (lanes, status) => lanes.find((l) => l.status === status);
const groupItems = (lane, key) => lane.groups.find((g) => g.key === key)?.items.map((b) => b.item.code) ?? [];
const WAITING = next({ status: "In Progress", missing: 1, signoffs: ["design-review"], for_you: 1 });
const READY = next({ status: "In Progress", missing: 0, ends: false });
const NEEDS = next({ status: "In Progress", missing: 4, ends: false });

/** The 450-item board: three lanes, every group reachable, gates and ages varied. */
function makeBoard(n) {
  const items = [];
  const readiness = {};
  for (let i = 0; i < n; i++) {
    const code = `FTR-${String(i + 1).padStart(3, "0")}`;
    const status = ["Planned", "In Progress", "Completed"][i % 3];
    const dep = i % 11 === 0 && i > 3 ? { depends_on: [`FTR-${String(i - 2).padStart(3, "0")}`] } : {};
    items.push(item(code, status, { entered_on: day(i % 40), created_at: iso(60 - (i % 50)), priority: ["high", "medium", "low", ""][i % 4], ...dep }));
    const nx = [WAITING, READY, NEEDS][i % 3];
    readiness[code] = card([nx], { updated_at: iso(i % 17) });
  }
  return { items, readiness };
}

// ---- R-1: every item in exactly one group ---------------------------------------------------
{
  const problems = [];
  const { items, readiness } = makeBoard(450);
  const idx = index(items);
  const lanes = boardLanes(input(idx, readiness));
  for (const lane of lanes) {
    const seen = new Map();
    for (const g of lane.groups) for (const b of g.items) seen.set(b.item.code, (seen.get(b.item.code) ?? 0) + 1);
    const dup = [...seen].filter(([, c]) => c !== 1);
    if (dup.length) problems.push(`${lane.status}: in more than one group ${JSON.stringify(dup.slice(0, 3))}`);
    const inLane = items.filter((f) => f.status === lane.status).length;
    const counted = seen.size + (lane.rest?.count ?? 0);
    eq(problems, `${lane.status} total`, lane.total, inLane);
    eq(problems, `${lane.status} items plus the rest line`, counted, inLane);
    // Readiness failed: every item is still grouped, none Waiting on you or Ready.
  }
  const blind = boardLanes(input(idx, null));
  for (const lane of blind) {
    const n = lane.groups.reduce((s, g) => s + g.items.length, 0) + (lane.rest?.count ?? 0);
    eq(problems, `${lane.status} without readiness`, n, items.filter((f) => f.status === lane.status).length);
    if (lane.kind !== "finished" && lane.groups.some((g) => g.key === "you" || g.key === "ready")) problems.push(`${lane.status}: you/ready without readiness`);
  }
  report("web::check_board_every_item_is_in_exactly_one_group", problems);
}

// ---- R-2: start lane ------------------------------------------------------------------------
{
  const problems = [];
  const items = [
    item("A-1", "Planned"), // waiting on you
    item("A-2", "Planned", { depends_on: ["A-9"] }), // waiting on you AND blocked -> you
    item("A-3", "Planned", { depends_on: ["A-9"] }), // ready and blocked -> waiting
    item("A-4", "Planned"), // ready
    item("A-5", "Planned"), // no gate -> ready
    item("A-6", "Planned"), // needs -> define
    item("A-7", "Planned"), // tasks open -> hint null -> define
    item("A-9", "In Progress"),
  ];
  const r = {
    "A-1": card([WAITING]),
    "A-2": card([WAITING]),
    "A-3": card([READY]),
    "A-4": card([READY]),
    "A-5": card([]),
    "A-6": card([NEEDS]),
    "A-7": card([next({ missing: 2, tasks_open: 1 })]),
  };
  const lane = laneOf(boardLanes(input(index(items), r)), "Planned");
  eq(problems, "kind", lane.kind, "start");
  eq(problems, "you", groupItems(lane, "you"), ["A-1", "A-2"]);
  eq(problems, "ready", groupItems(lane, "ready"), ["A-4", "A-5"]);
  eq(problems, "define (closest first)", groupItems(lane, "define"), ["A-7", "A-6"]);
  eq(problems, "waiting", groupItems(lane, "waiting"), ["A-3"]);
  eq(problems, "display order", lane.groups.map((g) => g.key), ["you", "ready", "define", "waiting"]);
  eq(problems, "labels", lane.groups.map((g) => g.label), ["Waiting on you", "Ready to start", "Needs definition or design", "Waiting on another item"]);
  eq(problems, "attention", lane.groups.map((g) => g.attention), [true, true, false, false]);
  report("web::check_board_start_lane_groups_follow_their_rules", problems);
}

// ---- R-2: Waiting on you agrees with the card's hint ----------------------------------------
{
  const problems = [];
  const cases = [WAITING, READY, NEEDS, next({ missing: 1, for_you: 1, tasks_open: 1 }), next({ missing: 2, for_you: 1 }), next({ missing: 1, for_you: 1, immediate: false }), next({ missing: 1, for_you: 1, signoffs: ["x"] })];
  for (const [i, n] of cases.entries()) {
    for (const kind of ["start", "active"]) {
      const b = { item: item("C-1", "Planned"), card: card([n]), next: n.immediate === false ? undefined : n, known: true, daysIn: 1, blockedBy: null, blockedByStatus: null, agent: null, critical: false, idle: false, changed: false };
      const you = groupOf(b, kind, false) === "you";
      const waits = nextStageHint(n)?.kind === "waiting" && n.immediate !== false;
      if (you !== waits) problems.push(`case ${i} (${kind}): groupOf says you=${you}, the hint says waiting=${waits}`);
    }
  }
  report("web::check_board_waiting_on_you_agrees_with_the_card_hint", problems);
}

// ---- R-3: active lane -----------------------------------------------------------------------
{
  const problems = [];
  const scope = (item) => ({ id: "SC-1", project: "p", role: "build", item, operations: [], issued_by: "x", issued_at: "", expires_at: "", state: "active" });
  const items = [
    item("B-1", "In Progress"), // you
    item("B-2", "In Progress"), // ready and agent -> ready
    item("B-3", "In Progress"), // agent and blocked -> agent
    item("B-4", "In Progress", { depends_on: ["B-9"] }), // blocked
    item("B-5", "In Progress"), // idle alone -> blocked
    item("B-6", "In Progress"), // building
    item("B-7", "In Progress", { depends_on: ["B-9"] }), // ready but blocked -> blocked
    item("B-8", "Completed"),
    item("B-9", "Planned"),
    item("B-0", "In Progress"), // ready
  ];
  const r = {
    "B-1": card([WAITING]),
    "B-2": card([READY]),
    "B-3": card([NEEDS]),
    "B-4": card([NEEDS]),
    "B-5": card([NEEDS], { updated_at: iso(9) }),
    "B-6": card([NEEDS]),
    "B-7": card([READY]),
    "B-0": card([READY]),
  };
  const lane = laneOf(boardLanes(input(index(items), r, { scopes: [scope("B-2"), scope("B-3")] })), "In Progress");
  eq(problems, "kind", lane.kind, "active");
  eq(problems, "you", groupItems(lane, "you"), ["B-1"]);
  eq(problems, "ready", groupItems(lane, "ready").sort(), ["B-0", "B-2"]);
  eq(problems, "agent", groupItems(lane, "agent"), ["B-3"]);
  eq(problems, "blocked", groupItems(lane, "blocked").sort(), ["B-4", "B-5", "B-7"]);
  eq(problems, "building", groupItems(lane, "building"), ["B-6"]);
  eq(problems, "display order", lane.groups.map((g) => g.key), ["you", "ready", "agent", "blocked", "building"]);
  eq(problems, "labels", lane.groups.map((g) => g.label), ["Waiting on you", "Ready to move", "An agent is on it now", "Blocked or stalled", "Building"]);
  eq(problems, "a revoked scope is no agent", groupItems(laneOf(boardLanes(input(index(items), r, { scopes: [{ ...scope("B-6"), state: "expired" }] })), "In Progress"), "agent"), []);
  report("web::check_board_active_lane_groups_follow_their_rules", problems);
}

// ---- R-4: the finished lane's window --------------------------------------------------------
{
  const problems = [];
  const sprints = [
    { code: "SP-003", name: "", start: day(60), end: day(30), state: "closed" },
    { code: "SP-004", name: "", start: day(10), end: day(-4), state: "active" },
  ];
  const done = [
    item("D-1", "Completed", { sprint: "SP-004", entered_on: day(2), release: "v0.7.0" }),
    item("D-2", "Completed", { entered_on: day(5), release: "v0.7.0" }), // after the sprint's start
    item("D-3", "Completed", { sprint: "SP-003", entered_on: day(40), release: "v0.6.0" }),
    item("D-4", "Completed", { entered_on: day(20) }),
    item("D-5", "Completed", { entered_on: day(1) }),
  ];
  const cad = (sp, rel) => ({ sprints: sp, releases: rel, git_model: false });
  const lanesFor = (idx, over) => laneOf(boardLanes(input(idx, {}, over)), "Completed");

  // A sprint chosen: its items only; the rest ignores the choice.
  let lane = lanesFor(index(done, { config: config({ cadence: cad(true, false) }), sprints }), { sprint: "SP-003" });
  eq(problems, "chosen sprint", groupItems(lane, "done"), ["D-3"]);
  eq(problems, "chosen sprint label", lane.window, "Finished in SP-003");
  eq(problems, "chosen sprint rest", lane.rest?.count, 4);
  eq(problems, "chosen sprint rest link", lane.rest?.to, "/p/p/items?status=Completed");

  // All items with an active sprint: by sprint and by entered_on >= start.
  lane = lanesFor(index(done, { config: config({ cadence: cad(true, false) }), sprints }), {});
  eq(problems, "active sprint", groupItems(lane, "done"), ["D-5", "D-1", "D-2"]);
  eq(problems, "active sprint label", lane.window, "Finished in SP-004");
  eq(problems, "active sprint rest", [lane.rest?.count, lane.rest?.text], [2, "2 finished earlier →"]);

  // Releases on, two planned: the one with a stage is current.
  const releases = [
    { version: "v0.9.0", state: "planned", target: "2026-12-01" },
    { version: "v0.8.0", state: "planned", stage: "candidate", target: "2026-12-20" },
    { version: "v0.7.0", state: "released" },
  ];
  eq(problems, "release in progress", releaseInProgress(releases)?.version, "v0.8.0");
  eq(problems, "release order without a stage", releaseInProgress(releases.map((r) => ({ ...r, stage: undefined })))?.version, "v0.9.0");
  eq(problems, "version order", releaseInProgress([{ version: "v0.10.0", state: "planned" }, { version: "v0.9.0", state: "planned" }])?.version, "v0.9.0");
  const relDone = [item("E-1", "Completed", { release: "v0.8.0" }), item("E-2", "Completed", { release: "v0.7.0" }), item("E-3", "Completed", { release: "v0.6.0" }), item("E-4", "Completed")];
  lane = lanesFor(index(relDone, { config: config({ cadence: cad(false, true) }), releases }), {});
  eq(problems, "release window", groupItems(lane, "done"), ["E-1"]);
  eq(problems, "release window label", lane.window, "Finished in v0.8.0");
  eq(problems, "release rest", [lane.rest?.count, lane.rest?.releases, lane.rest?.text, lane.rest?.to], [3, 2, "3 finished earlier, across 2 releases →", "/p/p/releases"]);
  lane = lanesFor(index([item("E-4", "Completed", { entered_on: day(40) }), item("E-5", "Completed", { entered_on: day(41) })], { config: config({ cadence: cad(false, true) }), releases: [] }), {});
  eq(problems, "releases on, none in progress: 14 days gives none, rest names no release", [lane.groups.length, lane.rest?.text], [0, "2 finished earlier →"]);

  // Neither: the last 14 days.
  lane = lanesFor(index(done), {});
  eq(problems, "14 days", groupItems(lane, "done"), ["D-5", "D-1", "D-2"]);
  eq(problems, "14 days label", lane.window, "Finished in the last 14 days");
  eq(problems, "14 days rest", [lane.rest?.count, lane.rest?.text], [2, "2 finished earlier →"]);
  eq(problems, "window helper", finishedWindow(index(done), "all", NOW).holds(done[3]), false);

  // The text filter is honoured by the rest; the Sprint choice is not.
  lane = lanesFor(index(done, { config: config({ cadence: cad(true, false) }), sprints }), { text: "D-3" });
  eq(problems, "text filter shows", groupItems(lane, "done"), []);
  eq(problems, "text filter rest", lane.rest?.count, 1);
  lane = lanesFor(index(done, { config: config({ cadence: cad(true, false) }), sprints }), { sprint: "unscheduled" });
  eq(problems, "unscheduled shows the unscheduled in the window", groupItems(lane, "done"), ["D-5", "D-2"]);
  eq(problems, "unscheduled rest ignores the choice", lane.rest?.count, 3);

  // An older daemon: no entered_on, the window falls back to the sprint or release fields only.
  const old = done.map(({ entered_on, ...f }) => f);
  lane = lanesFor(index(old, { config: config({ cadence: cad(true, false) }), sprints }), {});
  eq(problems, "older daemon, sprint field", groupItems(lane, "done"), ["D-1"]);
  lane = lanesFor(index(old), {});
  eq(problems, "older daemon, no window", lane.groups.length, 0);
  report("web::check_board_finished_lane_shows_the_current_window_only", problems);
}

// ---- R-8: the flow line ---------------------------------------------------------------------
{
  const problems = [];
  const items = [item("F-1", "In Progress", { entered_on: day(23) }), item("F-2", "In Progress", { entered_on: day(2) })];
  const flow = [
    { status: "Planned", days: [] },
    { status: "In Progress", days: [{ day: day(8), entered: 9, left: 9 }, { day: day(6), entered: 2, left: 1 }, { day: day(0), entered: 1, left: 0 }, { day: day(2), entered: 3, left: 2 }] },
    { status: "Completed", days: [{ day: day(1), entered: 6, left: 0 }, { day: day(7), entered: 4, left: 0 }] },
  ];
  const lanes = boardLanes(input(index(items, { flow }), { "F-1": card([NEEDS]), "F-2": card([NEEDS]) }));
  const lane = laneOf(lanes, "In Progress");
  eq(problems, "in/out over today and the six days before", lane.flow, { in7: 6, out7: 3 });
  eq(problems, "oldest", [lane.oldestDays, lane.oldestCode], [23, "F-1"]);
  eq(problems, "text", flowText(lane), "Last 7 days: 6 in · 3 out · oldest 23 days (F-1)");
  eq(problems, "finished text", flowText(laneOf(lanes, "Completed")), "Last 7 days: 6 finished");
  const noFlow = laneOf(boardLanes(input(index(items), {})), "In Progress");
  eq(problems, "without flow: the oldest part only", [noFlow.flow, flowText(noFlow)], [null, "Oldest 23 days (F-1)"]);
  const bare = laneOf(boardLanes(input(index([item("F-9", "In Progress", { entered_on: undefined })]), {})), "In Progress");
  eq(problems, "without either: no line", flowText(bare), null);
  report("web::check_board_flow_line_counts_the_last_seven_days", problems);
}

// ---- R-9: the insight -----------------------------------------------------------------------
{
  const problems = [];
  const mk = (n, over) => Array.from({ length: n }, (_, i) => ({ code: `I-${i}`, ...over(i) }));
  const laneWith = (n, idleN, readyN, flow, sprintsOn = false, extra = []) => {
    const items = [...mk(n, (i) => ({ status: "In Progress" })).map((f) => item(f.code, "In Progress"))];
    const r = {};
    items.forEach((f, i) => {
      r[f.code] = card([i < readyN ? READY : NEEDS], { updated_at: i >= readyN && i < readyN + idleN ? iso(9) : iso(1) });
    });
    const idx = index([...items, ...extra], { flow: flow ? [{ status: "In Progress", days: [{ day: day(1), entered: flow[0], left: flow[1] }] }] : undefined });
    const l = laneOf(boardLanes(input(idx, r)), "In Progress");
    return l;
  };
  const ins = (l, ctx = {}) => laneInsight(l, { lastSprintFinished: null, kind: "active", ...ctx });

  eq(problems, "idle 3 of 20", ins(laneWith(20, 3, 0, null))?.rule, "idle");
  eq(problems, "idle sentence", ins(laneWith(20, 3, 0, null))?.text, "3 of 20 have had no change in 7 days.");
  eq(problems, "idle links the blocked group", ins(laneWith(20, 3, 0, null))?.group, "blocked");
  eq(problems, "2 idle of 20 is none", ins(laneWith(20, 2, 0, null)), null);
  eq(problems, "the share grows with the lane (3 of 40 is none, 4 is idle)", [ins(laneWith(40, 3, 0, null)), ins(laneWith(40, 4, 0, null))?.rule], [null, "idle"]);

  const wip = ins(laneWith(62, 0, 0, null), { lastSprintFinished: 21 });
  eq(problems, "wip", [wip?.rule, wip?.text], ["wip", "In Progress holds 3× what the last sprint finished (62 against 21)."]);
  eq(problems, "ratio 1.9 is none", ins(laneWith(19, 0, 0, null), { lastSprintFinished: 10 }), null);
  eq(problems, "ratio 2 is wip", ins(laneWith(20, 0, 0, null), { lastSprintFinished: 10 })?.text, "In Progress holds 2× what the last sprint finished (20 against 10).");
  eq(problems, "no finished sprint, no wip", ins(laneWith(62, 0, 0, null), { lastSprintFinished: 0 }), null);

  const fill = ins(laneWith(10, 0, 0, [12, 3]));
  eq(problems, "filling", [fill?.rule, fill?.text], ["filling", "12 came in during the last 7 days and 3 left: the lane is filling."]);
  eq(problems, "filling needs 5 in", ins(laneWith(10, 0, 0, [4, 0])), null);
  eq(problems, "filling needs twice what left", ins(laneWith(10, 0, 0, [6, 4])), null);

  const rdy = ins(laneWith(10, 0, 4, null));
  eq(problems, "ready (active)", [rdy?.rule, rdy?.text, rdy?.group], ["ready", "4 are ready to move — nothing stops them but a move.", "ready"]);
  eq(problems, "ready needs 3", ins(laneWith(10, 0, 2, null)), null);
  eq(problems, "ready (start)", ins(laneWith(10, 0, 4, null), { kind: "start" })?.text, "4 are ready to start — every check met and nothing they wait on open.");
  eq(problems, "idle and ready together give idle", ins(laneWith(20, 4, 4, null))?.rule, "idle");
  eq(problems, "a quiet lane gives null", ins(laneWith(10, 0, 0, null)), null);
  eq(problems, "a start lane has no idle rule", ins(laneWith(20, 4, 0, null), { kind: "start" }), null);

  // The finished lane's one rule: pace.
  const doneLane = (n) => ({ status: "Completed", kind: "finished", total: n, groups: [{ key: "done", label: "x", tone: "ok", attention: false, items: Array.from({ length: n }, () => ({})), oldest: null }], flow: null, oldestDays: null, oldestCode: null, usual: null, insight: null, rest: null, window: "x", windowSprint: "SP-004" });
  const pace = laneInsight(doneLane(18), { lastSprintFinished: 12, kind: "finished", lastSprint: "SP-003", windowSprint: "SP-004" });
  eq(problems, "pace", [pace?.rule, pace?.text], ["pace", "18 finished in SP-004, already more than the 12 SP-003 finished."]);
  eq(problems, "pace needs the last sprint's count", laneInsight(doneLane(18), { lastSprintFinished: 20, kind: "finished", lastSprint: "SP-003", windowSprint: "SP-004" }), null);
  eq(problems, "pace needs a sprint window", laneInsight(doneLane(18), { lastSprintFinished: 12, kind: "finished", lastSprint: "SP-003", windowSprint: null }), null);
  report("web::check_board_insight_is_one_line_chosen_by_rule", problems);
}

// ---- R-12: the card's one fact --------------------------------------------------------------
{
  const problems = [];
  const b = (over = {}) => ({
    item: item("G-1", "In Progress"),
    card: card([NEEDS], { req_proven: 7, req_total: 9, doing: { task: "TL-001/T3", text: "Build the route" } }),
    next: NEEDS,
    known: true,
    daysIn: 23,
    blockedBy: "FTR-012",
    blockedByStatus: "In Progress",
    agent: { id: "SC-1" },
    critical: true,
    idle: false,
    changed: true,
    ...over,
  });
  const f = (x, usual = 6) => cardFact(x, usual);
  eq(problems, "critical first", [f(b())?.kind, f(b())?.text], ["critical", "On the critical path"]);
  eq(problems, "blocked", [f(b({ critical: false }))?.kind, f(b({ critical: false }))?.text], ["blocked", "Waiting on FTR-012 (In Progress)"]);
  eq(problems, "blocked, not on this board", f(b({ critical: false, blockedByStatus: null }))?.text, "Waiting on FTR-012 (not on this board)");
  eq(problems, "agent", f(b({ critical: false, blockedBy: null }))?.text, "An agent is on it: T3 Build the route");
  eq(problems, "agent without doing", f(b({ critical: false, blockedBy: null, card: card([NEEDS]) }))?.text, "An agent is on it now");
  const base = { critical: false, blockedBy: null, agent: null };
  eq(problems, "stalled", [f(b(base))?.kind, f(b(base))?.text], ["stalled", "23 days in In Progress; the lane's usual is 6"]);
  eq(problems, "stalled needs twice the usual", f(b({ ...base, daysIn: 11 }))?.kind, "changed");
  eq(problems, "stalled needs 7 days", f(b({ ...base, daysIn: 6 }), 1)?.kind, "changed");
  eq(problems, "changed", [f(b({ ...base, daysIn: 3 }))?.kind, f(b({ ...base, daysIn: 3 }))?.text], ["changed", "Changed since you last looked"]);
  eq(problems, "proof", [f(b({ ...base, daysIn: 3, changed: false }))?.kind, f(b({ ...base, daysIn: 3, changed: false }))?.text], ["proof", "Requirements proven 7 of 9"]);
  eq(problems, "nothing to say", f(b({ ...base, daysIn: null, changed: false, card: card([NEEDS]) })), null);
  eq(problems, "icons", ["critical", "blocked", "agent"].map(() => null), [null, null, null]);
  const kinds = [];
  let x = b();
  for (const patch of [{ critical: false }, { blockedBy: null }, { agent: null }, { daysIn: 3 }, { changed: false }]) {
    kinds.push(f(x)?.kind);
    x = { ...x, ...patch };
  }
  kinds.push(f(x)?.kind);
  eq(problems, "each removed in turn shows the next", kinds, ["critical", "blocked", "agent", "stalled", "changed", "proof"]);
  report("web::check_board_card_fact_follows_the_priority_order", problems);
}

// ---- R-13: the last look --------------------------------------------------------------------
{
  const problems = [];
  const items = [item("H-1", "In Progress"), item("H-2", "In Progress"), item("H-3", "In Progress")];
  const r = {
    "H-1": card([NEEDS], { updated_at: "2026-11-17T08:00:00Z" }),
    "H-2": card([NEEDS], { updated_at: "2026-11-16T08:00:00Z" }),
    "H-3": card([NEEDS], { updated_at: undefined }),
  };
  const changed = (seen) => laneOf(boardLanes(input(index(items), r, { seen })), "In Progress").groups.flatMap((g) => g.items).filter((x) => x.changed).map((x) => x.item.code).sort();
  eq(problems, "changed after the last look", changed("2026-11-16T12:00:00Z"), ["H-1"]);
  eq(problems, "first visit marks nothing", changed(null), []);
  eq(problems, "a look after every change marks nothing", changed("2026-11-17T09:00:00Z"), []);
  eq(problems, "the key is per project", seenKey("kanbanr"), "kanbanr-board-seen:kanbanr");
  eq(problems, "the visit is kept after 10 s", SEEN_AFTER_MS, 10_000);
  report("web::check_board_last_look_is_kept_in_this_browser", problems);
}

// ---- R-15: ordering -------------------------------------------------------------------------
{
  const problems = [];
  const mkb = (code, missing, tasks, daysIn, over = {}) => ({
    item: item(code, "In Progress", over),
    next: missing === null ? undefined : next({ missing, tasks_open: tasks }),
    known: true,
    daysIn,
    blockedBy: null,
    blockedByStatus: null,
    agent: null,
    critical: false,
    idle: false,
    changed: false,
  });
  const set = [
    mkb("J-1", 3, 0, 5, { created_at: "2026-10-01T00:00:00Z", priority: "low" }),
    mkb("J-2", 1, 2, 5, { created_at: "2026-10-03T00:00:00Z", priority: "High" }),
    mkb("J-3", 1, 0, 2, { created_at: "2026-10-02T00:00:00Z" }),
    mkb("J-4", 1, 0, 9, { created_at: "2026-10-02T00:00:00Z", priority: "medium" }),
    mkb("J-5", 1, 0, null, { created_at: "2026-10-04T00:00:00Z", priority: "high" }),
    mkb("J-6", null, 0, 4, { created_at: "2026-10-05T00:00:00Z" }),
    mkb("J-7", 1, 0, 9, { created_at: "2026-10-02T00:00:00Z" }),
  ];
  const codes = (s) => sortItems(set, s).map((x) => x.item.code);
  eq(problems, "close: missing, tasks open, days in desc (null last), code", codes("close"), ["J-6", "J-4", "J-7", "J-3", "J-5", "J-2", "J-1"]);
  eq(problems, "newest: created desc, ties by code desc", codes("newest"), ["J-6", "J-5", "J-2", "J-7", "J-4", "J-3", "J-1"]);
  eq(problems, "priority: high, medium, low, unset last, then close", codes("priority"), ["J-5", "J-2", "J-4", "J-1", "J-6", "J-7", "J-3"]);
  eq(problems, "closeness is a comparator", Math.sign(closeness(set[1], set[0])), -1);
  const lane = laneOf(boardLanes(input(index(set.map((x) => x.item)), Object.fromEntries(set.filter((x) => x.next).map((x) => [x.item.code, card([x.next])])), { sort: "newest" })), "In Progress");
  eq(problems, "a lane sorts each group by the choice", lane.groups.flatMap((g) => g.items.map((y) => y.item.code)).includes("J-1"), true);
  // The finished lane always orders newest finished first.
  const fin = laneOf(boardLanes(input(index([item("K-1", "Completed", { entered_on: day(3) }), item("K-3", "Completed", { entered_on: day(1) }), item("K-2", "Completed", { entered_on: day(1) })]), {}, { sort: "priority" })), "Completed");
  eq(problems, "finished: newest first, ties by code", groupItems(fin, "done"), ["K-2", "K-3", "K-1"]);
  report("web::check_board_groups_order_by_closeness_newest_or_priority", problems);
}

// ---- helpers the page draws with (kept with the model) --------------------------------------
{
  const problems = [];
  eq(problems, "lane kinds", laneKinds(config()), { Planned: "start", "In Progress": "active", Completed: "finished" });
  eq(problems, "two lanes", laneKinds(config({ displayed_states: ["Open", "Closed"], terminal_states: ["Closed"] })), { Open: "start", Closed: "finished" });
  eq(problems, "group id", groupId("In Progress", "you"), "g-in-progress-you");
  const g = (key, attention) => ({ key, label: "", tone: "ok", attention, items: [], oldest: null });
  const none = new Set();
  eq(problems, "attention group open by default", isGroupOpen("In Progress", g("you", true), none, none), true);
  eq(problems, "attention group shut when listed", isGroupOpen("In Progress", g("you", true), none, new Set(["g-in-progress-you"])), false);
  eq(problems, "quiet group shut by default", isGroupOpen("In Progress", g("building", false), none, none), false);
  eq(problems, "quiet group open when listed", isGroupOpen("In Progress", g("building", false), new Set(["g-in-progress-building"]), none), true);
  let t = toggleGroup(none, none, "g-a-you", false, true);
  eq(problems, "closing an attention group writes shut", [[...t.open], [...t.shut]], [[], ["g-a-you"]]);
  t = toggleGroup(t.open, t.shut, "g-a-you", true, true);
  eq(problems, "reopening it clears the list", [[...t.open], [...t.shut]], [[], []]);
  t = toggleGroup(none, none, "g-a-building", true, false);
  eq(problems, "opening a quiet group writes open", [[...t.open], [...t.shut]], [["g-a-building"], []]);
  const steps = stepperSteps({ ...next({ missing: 18 }), checks: [
    { family: "definition", asked: true, missing: 0 },
    { family: "approval", asked: false, missing: 0 },
    { family: "signoffs", asked: true, missing: 3 },
    { family: "tests", asked: true, missing: 9 },
    { family: "docs", asked: true, missing: 6 },
  ] });
  eq(problems, "steps", steps.map((s) => [s.label, s.state]), [["Definition", "done"], ["Approval", "skipped"], ["Sign-offs 3", "next"], ["Tests green 9", "next"], ["Docs 6", "next"]]);
  eq(problems, "checks left", checksLeftText({ ...next({ missing: 18 }), checks: steps.map((s) => ({ family: s.key, asked: true, missing: Number(s.label.split(" ").pop()) || 0 })) }), "18 checks left before Completed: 9 tests to turn green, 6 docs, 3 sign-offs");
  eq(problems, "without checks", checksLeftText(next({ missing: 18 })), "18 checks left before Completed");
  eq(problems, "nothing left", checksLeftText(next({ missing: 0 })), null);
  eq(problems, "older daemon draws no stepper", stepperSteps(next()), []);
  report("web::check_board_model_helpers", problems);
}

// ---- R-19: 450 items within 20 ms -----------------------------------------------------------
{
  const problems = [];
  const { items, readiness } = makeBoard(450);
  const idx = index(items, { flow: [{ status: "In Progress", days: [{ day: day(1), entered: 3, left: 1 }] }] });
  const run = () => {
    const t = performance.now();
    boardLanes(input(idx, readiness));
    return performance.now() - t;
  };
  run();
  const times = Array.from({ length: 20 }, run).sort((a, b) => a - b);
  const median = (times[9] + times[10]) / 2;
  console.log(`web::check_board_renders_450_items_within_budget — boardLanes median of 20: ${median.toFixed(2)} ms`);
  if (median >= 20) problems.push(`boardLanes took ${median.toFixed(1)} ms (median of 20), the budget is 20 ms`);
  report("web::check_board_renders_450_items_within_budget", problems);
}

process.exit(ok ? 0 : 1);
