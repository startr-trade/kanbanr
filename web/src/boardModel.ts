/**
 * The Board's model (FTR-078): which group each item of a lane falls in, the lane's flow, insight
 * and finished window, the card's one fact and the next stage's checks in words. Pure, so the page,
 * its components and `scripts/check-board.mjs` use the same code; every rule and constant of the
 * Board lives here.
 */
import { liveDelegated, nextStageHint } from "./cardText.ts";
import type { CardHint } from "./cardText.ts";
import { ALL, UNSCHEDULED, inSprintFilter } from "./sprintFilter.ts";
import { to } from "./overviewLinks.ts";
import { displayedStates } from "./types.ts";
import type {
  AgentScope,
  CardReadiness,
  CheckFamily,
  CompactIndex,
  IndexConfig,
  IndexItem,
  IndexRelease,
  IndexSprint,
  NextStageWire,
  Readiness,
} from "./types.ts";
import type { Segment } from "./ui/progress.ts";
import type { Step } from "./ui/stepper.ts";
import type { Tone } from "./ui/tone.ts";

// ---- constants (chosen against this board's numbers; see the design) ------------------------
export const IDLE_DAYS = 7;
export const IDLE_MIN = 3;
export const IDLE_SHARE = 0.1;
export const WIP_RATIO = 2;
export const FILL_MIN = 5;
export const FILL_RATIO = 2;
export const READY_MIN = 3;
export const STALL_MIN_DAYS = 7;
export const STALL_FACTOR = 2;
export const FLOW_DAYS = 7;
export const RECENT_DONE_DAYS = 14;
export const SEEN_AFTER_MS = 10_000;
export const MILESTONE_CHARS = 60;

const DAY_MS = 86_400_000;

// ---- lanes and groups -----------------------------------------------------------------------
export type LaneKind = "start" | "active" | "finished";
/** `done` is the finished lane's one group. */
export type GroupKey = "you" | "ready" | "agent" | "blocked" | "building" | "define" | "waiting" | "done";
export type SortKey = "close" | "newest" | "priority";

export interface BoardItem {
  item: IndexItem;
  card?: CardReadiness;
  /** The immediate next stage, the one `nextStageHint` speaks of. */
  next?: NextStageWire;
  /** Readiness answered, so an item without a gate is ready rather than unknown. */
  known: boolean;
  daysIn: number | null;
  blockedBy: string | null;
  /** The status of the item it waits on; null when that item is not on this board. */
  blockedByStatus: string | null;
  agent: AgentScope | null;
  critical: boolean;
  /** No change in `IDLE_DAYS` days. */
  idle: boolean;
  changed: boolean;
}

export interface Group {
  key: GroupKey;
  label: string;
  tone: Tone;
  attention: boolean;
  items: BoardItem[];
  oldest: BoardItem | null;
}

export type InsightRule = "idle" | "wip" | "filling" | "ready" | "pace";
export interface Insight {
  rule: InsightRule;
  text: string;
  group?: GroupKey;
}

export interface RestLine {
  count: number;
  /** Distinct releases among the items left out; 0 where releases are off. */
  releases: number;
  text: string;
  to: string;
}

export interface Lane {
  status: string;
  kind: LaneKind;
  total: number;
  groups: Group[];
  flow: { in7: number; out7: number } | null;
  oldestDays: number | null;
  /** The code of the item `oldestDays` is about. */
  oldestCode: string | null;
  /** The median days in the lane over its live items, before filters. */
  usual: number | null;
  insight: Insight | null;
  rest: RestLine | null;
  /** The finished lane's window label, e.g. "Finished in SP-004". */
  window: string | null;
  /** The sprint the finished window is, when it is one. */
  windowSprint: string | null;
}

export interface BoardInput {
  project: string;
  index: CompactIndex;
  readiness: Readiness | null;
  scopes: AgentScope[];
  critical: string[];
  seen: string | null;
  now: Date;
  /** ALL, UNSCHEDULED or a sprint code. */
  sprint: string;
  text: string;
  sort: SortKey;
}

export const GROUP_ICON: Record<GroupKey, string> = {
  you: "●",
  ready: "▶",
  agent: "⚙",
  blocked: "⏸",
  building: "◐",
  define: "✎",
  waiting: "⧗",
  done: "✓",
};

export const GROUP_TONE: Record<GroupKey, Tone> = {
  you: "info",
  ready: "ok",
  agent: "info",
  blocked: "warn",
  waiting: "warn",
  building: "neutral",
  define: "neutral",
  done: "ok",
};

const ATTENTION: GroupKey[] = ["you", "ready", "agent", "blocked"];
const ORDER: Record<LaneKind, GroupKey[]> = {
  start: ["you", "ready", "define", "waiting"],
  active: ["you", "ready", "agent", "blocked", "building"],
  finished: ["done"],
};

/**
 * The statuses that end an item: the config's `terminal_states` when it names any, else what the core
 * falls back to for a board made before they existed (`graph::is_terminal_status`): the status named
 * "Completed" and the no-op states (FTR-078).
 */
export function terminalStates(config: IndexConfig): string[] {
  const named = config.terminal_states ?? [];
  if (named.length > 0) return named;
  const completed = (config.statuses ?? []).filter((s) => s.toLowerCase() === "completed");
  return [...completed, ...(config.no_op_states ?? [])];
}

/** Terminal displayed statuses are finished; the first other displayed status starts; the rest are active. */
export function laneKinds(config: IndexConfig): Record<string, LaneKind> {
  const terminal = terminalStates(config);
  const out: Record<string, LaneKind> = {};
  let started = false;
  for (const s of displayedStates(config)) {
    if (terminal.includes(s)) out[s] = "finished";
    else if (!started) {
      out[s] = "start";
      started = true;
    } else out[s] = "active";
  }
  return out;
}

export function groupLabel(key: GroupKey, kind: LaneKind, window?: string | null): string {
  switch (key) {
    case "you":
      return "Waiting on you";
    case "ready":
      return kind === "start" ? "Ready to start" : "Ready to move";
    case "agent":
      return "An agent is on it now";
    case "blocked":
      return "Blocked or stalled";
    case "building":
      return "Building";
    case "define":
      return "Needs definition or design";
    case "waiting":
      return "Waiting on another item";
    case "done":
      return window ?? "Finished";
  }
}

const hintOf = (b: BoardItem): CardHint | null => (b.next ? nextStageHint(b.next) : null);

/** The group an item falls in; the first rule that holds (the design's table). */
export function groupOf(b: BoardItem, kind: LaneKind, idle: boolean): GroupKey {
  if (kind === "finished") return "done";
  const hint = hintOf(b);
  if (hint?.kind === "waiting") return "you";
  if (kind === "start") {
    if (b.blockedBy !== null) return "waiting";
    if ((!b.next && b.known) || hint?.kind === "ready") return "ready";
    return "define";
  }
  if (hint?.kind === "ready" && b.blockedBy === null) return "ready";
  if (b.agent !== null) return "agent";
  if (b.blockedBy !== null || idle) return "blocked";
  return "building";
}

// ---- ordering -------------------------------------------------------------------------------
const PRIORITY_RANK: Record<string, number> = { high: 0, medium: 1, low: 2 };
const priorityRank = (b: BoardItem) => PRIORITY_RANK[(b.item.priority ?? "").toLowerCase()] ?? 3;

/** Fewest checks missing first, then fewest open tasks, then longest in the stage, then code. */
export function closeness(a: BoardItem, b: BoardItem): number {
  return (
    (a.next?.missing ?? 0) - (b.next?.missing ?? 0) ||
    (a.next?.tasks_open ?? 0) - (b.next?.tasks_open ?? 0) ||
    (a.daysIn === null ? 1 : 0) - (b.daysIn === null ? 1 : 0) ||
    (b.daysIn ?? 0) - (a.daysIn ?? 0) ||
    a.item.code.localeCompare(b.item.code)
  );
}

export function sortItems(items: BoardItem[], sort: SortKey): BoardItem[] {
  const out = [...items];
  if (sort === "newest") {
    // `featuresNewestFirst`'s order: created newest first, ties by code descending.
    out.sort((a, b) => (b.item.created_at ?? "").localeCompare(a.item.created_at ?? "") || b.item.code.localeCompare(a.item.code));
  } else if (sort === "priority") {
    out.sort((a, b) => priorityRank(a) - priorityRank(b) || closeness(a, b));
  } else out.sort(closeness);
  return out;
}

/** The item that has been in its stage longest; without any ages, the one created first. */
export function oldestOf(items: BoardItem[]): BoardItem | null {
  if (items.length === 0) return null;
  const aged = items.filter((b) => b.daysIn !== null);
  if (aged.length > 0) {
    return aged.reduce((m, b) => ((b.daysIn as number) > (m.daysIn as number) || (b.daysIn === m.daysIn && b.item.code < m.item.code) ? b : m));
  }
  return items.reduce((m, b) => {
    const c = (b.item.created_at ?? "").localeCompare(m.item.created_at ?? "");
    return c < 0 || (c === 0 && b.item.code < m.item.code) ? b : m;
  });
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** A closed group's second half: "oldest FTR-012, 23 days in In Progress". */
export function oldestText(g: Group, laneStatus: string): string | null {
  const o = g.oldest;
  if (!o) return null;
  return o.daysIn === null ? `oldest ${o.item.code}` : `oldest ${o.item.code}, ${plural(o.daysIn, "day")} in ${laneStatus}`;
}

// ---- the finished window --------------------------------------------------------------------
const versionParts = (v: string) => v.split(/[.\-+]/).map((p) => (/^\d+$/.test(p) ? Number(p) : NaN));
function byVersion(a: string, b: string): number {
  const x = versionParts(a);
  const y = versionParts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const p = x[i];
    const q = y[i];
    if (p === undefined && q === undefined) return 0;
    if (p === undefined) return -1;
    if (q === undefined) return 1;
    const pn = Number.isNaN(p);
    const qn = Number.isNaN(q);
    if (pn !== qn) return pn ? 1 : -1; // numbers before others
    if (!pn && p !== q) return p - q;
  }
  return a.localeCompare(b);
}

/** The release in progress: planned, those with a stage first, then by target (empty last), then version. */
export function releaseInProgress(releases: IndexRelease[] | undefined): IndexRelease | null {
  const planned = (releases ?? []).filter((r) => r.state === "planned");
  planned.sort(
    (a, b) =>
      (a.stage ? 0 : 1) - (b.stage ? 0 : 1) ||
      (a.target ? 0 : 1) - (b.target ? 0 : 1) ||
      (a.target ?? "").localeCompare(b.target ?? "") ||
      byVersion(a.version, b.version),
  );
  return planned[0] ?? null;
}

const isoDay = (d: Date) => d.toISOString().slice(0, 10);
const dayMs = (day: string) => Date.parse(`${day.slice(0, 10)}T00:00:00Z`);

/** Which finished items the lane shows: the chosen sprint, else the active sprint, else the release in progress, else the last 14 days. */
export function finishedWindow(index: CompactIndex, sprint: string, now: Date): { label: string; holds(i: IndexItem): boolean; sprint: string | null } {
  const usesSprints = index.config.cadence.sprints === true;
  const usesReleases = index.config.cadence.releases === true;
  if (usesSprints && sprint !== ALL && sprint !== UNSCHEDULED) {
    return { label: `Finished in ${sprint}`, holds: (i) => i.sprint === sprint, sprint };
  }
  const active: IndexSprint | undefined = usesSprints ? (index.sprints ?? []).find((s) => s.state === "active") : undefined;
  if (active) {
    return {
      label: `Finished in ${active.code}`,
      holds: (i) => i.sprint === active.code || (!!i.entered_on && i.entered_on >= active.start.slice(0, 10)),
      sprint: active.code,
    };
  }
  const rel = usesReleases ? releaseInProgress(index.releases) : null;
  if (rel) return { label: `Finished in ${rel.version}`, holds: (i) => i.release === rel.version, sprint: null };
  const since = isoDay(new Date(now.getTime() - RECENT_DONE_DAYS * DAY_MS));
  return { label: `Finished in the last ${RECENT_DONE_DAYS} days`, holds: (i) => !!i.entered_on && i.entered_on >= since, sprint: null };
}

/** The last closed sprint and how many of its items are in a terminal status. */
export function lastClosedSprint(index: CompactIndex): { code: string; finished: number } | null {
  const closed = (index.sprints ?? []).filter((s) => s.state === "closed");
  if (closed.length === 0) return null;
  const last = closed.reduce((m, s) => (s.end > m.end ? s : m));
  const terminal = terminalStates(index.config);
  return { code: last.code, finished: index.items.filter((i) => i.sprint === last.code && terminal.includes(i.status)).length };
}

// ---- flow -----------------------------------------------------------------------------------
/** Moves in and out of a lane over the last `FLOW_DAYS` days ending today (UTC); null without `flow`. */
export function flowCounts(index: CompactIndex, status: string, now: Date): { in7: number; out7: number } | null {
  const lane = index.flow?.find((l) => l.status === status);
  if (!lane) return null;
  const from = isoDay(new Date(now.getTime() - (FLOW_DAYS - 1) * DAY_MS));
  const today = isoDay(now);
  let in7 = 0;
  let out7 = 0;
  for (const d of lane.days) {
    if (d.day >= from && d.day <= today) {
      in7 += d.entered;
      out7 += d.left;
    }
  }
  return { in7, out7 };
}

/** The flow line: "Last 7 days: 12 in · 8 out · oldest 23 days (FTR-012)"; null when there is nothing to say. */
export function flowText(lane: Lane): string | null {
  if (lane.kind === "finished") return lane.flow ? `Last ${FLOW_DAYS} days: ${lane.flow.in7} finished` : null;
  const oldest = lane.oldestDays !== null && lane.oldestCode ? `oldest ${plural(lane.oldestDays, "day")} (${lane.oldestCode})` : null;
  const moves = lane.flow ? `${lane.flow.in7} in · ${lane.flow.out7} out` : null;
  if (moves && oldest) return `Last ${FLOW_DAYS} days: ${moves} · ${oldest}`;
  if (moves) return `Last ${FLOW_DAYS} days: ${moves}`;
  if (oldest) return `Oldest ${plural(lane.oldestDays as number, "day")} (${lane.oldestCode})`;
  return null;
}

// ---- insight --------------------------------------------------------------------------------
const ratioText = (r: number) => String(r < 10 ? Number(r.toFixed(1)) : Math.round(r));

/** One line a lane says about itself, chosen by rule; null when nothing is worth saying. */
export function laneInsight(
  lane: Lane,
  ctx: { lastSprintFinished: number | null; kind: LaneKind; lastSprint?: string | null; windowSprint?: string | null },
): Insight | null {
  const count = (k: GroupKey) => lane.groups.find((g) => g.key === k)?.items.length ?? 0;
  if (ctx.kind === "finished") {
    const f = ctx.lastSprintFinished;
    const here = count("done");
    if (f !== null && f >= 1 && ctx.windowSprint && ctx.windowSprint !== ctx.lastSprint && here >= f) {
      return {
        rule: "pace",
        text: `${here} finished in ${ctx.windowSprint}, already ${here === f ? "as many as" : "more than"} the ${f} ${ctx.lastSprint ?? "the last sprint"} finished.`,
      };
    }
    return null;
  }
  if (ctx.kind === "active") {
    const idle = lane.groups.reduce((n, g) => n + g.items.filter((b) => b.idle).length, 0);
    if (idle >= Math.max(IDLE_MIN, Math.ceil(IDLE_SHARE * lane.total))) {
      return { rule: "idle", text: `${idle} of ${lane.total} have had no change in ${IDLE_DAYS} days.`, group: "blocked" };
    }
    const f = ctx.lastSprintFinished;
    if (f !== null && f >= 1 && lane.total >= WIP_RATIO * f) {
      return { rule: "wip", text: `${lane.status} holds ${ratioText(lane.total / f)}× what the last sprint finished (${lane.total} against ${f}).` };
    }
  }
  const fl = lane.flow;
  if (fl && fl.in7 >= FILL_MIN && fl.in7 >= FILL_RATIO * fl.out7) {
    return { rule: "filling", text: `${fl.in7} came in during the last ${FLOW_DAYS} days and ${fl.out7} left: the lane is filling.` };
  }
  const ready = count("ready");
  if (ready >= READY_MIN) {
    return {
      rule: "ready",
      text: ctx.kind === "active" ? `${ready} are ready to move — nothing stops them but a move.` : `${ready} are ready to start — every check met and nothing they wait on open.`,
      group: "ready",
    };
  }
  return null;
}

// ---- the card's one fact --------------------------------------------------------------------
export type FactKind = "critical" | "blocked" | "agent" | "stalled" | "changed" | "proof";
export interface Fact {
  kind: FactKind;
  icon: string;
  text: string;
}

/** The first fact that holds, in priority order; never two. `laneUsual` is the lane's median days in it. */
export function cardFact(b: BoardItem, laneUsual: number | null): Fact | null {
  if (b.critical) return { kind: "critical", icon: "◆", text: "On the critical path" };
  if (b.blockedBy !== null) {
    return { kind: "blocked", icon: "⏸", text: `Waiting on ${b.blockedBy} (${b.blockedByStatus ?? "not on this board"})` };
  }
  if (b.agent !== null) {
    const d = b.card?.doing;
    const key = d ? d.task.slice(d.task.lastIndexOf("/") + 1) : "";
    return { kind: "agent", icon: "⚙", text: d ? `An agent is on it: ${key} ${d.text}` : "An agent is on it now" };
  }
  if (b.daysIn !== null && b.daysIn >= Math.max(STALL_MIN_DAYS, STALL_FACTOR * (laneUsual ?? 0))) {
    const usual = laneUsual === null ? "" : `; the lane's usual is ${laneUsual}`;
    return { kind: "stalled", icon: "⌛", text: `${plural(b.daysIn, "day")} in ${b.item.status}${usual}` };
  }
  if (b.changed) return { kind: "changed", icon: "✱", text: "Changed since you last looked" };
  const total = b.card?.req_total ?? 0;
  if (total > 0) return { kind: "proof", icon: "✓", text: `Requirements proven ${b.card?.req_proven ?? 0} of ${total}` };
  return null;
}

// ---- the next stage's checks, in words ------------------------------------------------------
const FAMILY_ORDER: CheckFamily[] = ["definition", "approval", "signoffs", "tests", "docs"];
const FAMILY_LABEL: Record<CheckFamily, string> = {
  definition: "Definition",
  approval: "Approval",
  signoffs: "Sign-offs",
  tests: "Tests green",
  docs: "Docs",
};
const FAMILY_NOUN: Record<CheckFamily, [string, string]> = {
  definition: ["definition item", "definition items"],
  approval: ["approval", "approvals"],
  signoffs: ["sign-off", "sign-offs"],
  tests: ["test to turn green", "tests to turn green"],
  docs: ["doc", "docs"],
};

export const CHECK_WORDS: Record<Step["state"], string> = { done: "met", current: "now", next: "to do", skipped: "not asked" };

/** How many of a family's missing gaps a delegation in force holds (DEF-104); only the two families a delegation can cover. */
function heldIn(next: NextStageWire, family: CheckFamily, now: Date): { held: number; ids: string[] } {
  if (family !== "approval" && family !== "signoffs") return { held: 0, ids: [] };
  const gaps = liveDelegated(next, now).filter((g) =>
    family === "approval" ? g.check === "approved" : g.check === "signoff" && g.name !== "docs-folded",
  );
  return { held: gaps.length, ids: [...new Set(gaps.map((g) => g.delegation))] };
}

/** The five check families as steps; empty without `checks` (an older daemon). */
export function stepperSteps(next: NextStageWire, now: Date = new Date()): Step[] {
  if (!next.checks || next.checks.length === 0) return [];
  return FAMILY_ORDER.map((family) => {
    const t = next.checks!.find((c) => c.family === family);
    const missing = t?.missing ?? 0;
    const asked = (t?.asked ?? false) || missing > 0;
    const state: Step["state"] = missing > 0 ? "next" : asked ? "done" : "skipped";
    const step: Step = { key: family, label: missing > 0 ? `${FAMILY_LABEL[family]} ${missing}` : FAMILY_LABEL[family], state };
    // A step whose open gaps are all Claude's under a delegation says so, in words (decision 7).
    if (missing > 0 && heldIn(next, family, now).held >= missing) step.word = "Agent holds";
    return step;
  });
}

/** "18 checks left before Completed: 9 tests to turn green, 6 docs, 3 sign-offs"; null when nothing is left. */
/** A piece of the checks-left text; `delegation` marks "Claude, under DLG-n", which the card links (DEF-104). */
export interface LeftSegment {
  text: string;
  delegation?: string;
}

/** The checks-left text as segments, so the card can link each delegation it names; null when nothing is left. */
export function checksLeftSegments(next: NextStageWire, now: Date = new Date()): LeftSegment[] | null {
  if (next.missing === 0) return null;
  const head = `${plural(next.missing, "check")} left before ${next.status}`;
  const parts: LeftSegment[][] = (next.checks ?? [])
    .filter((c) => c.missing > 0)
    .map((c) => ({ c, order: FAMILY_ORDER.indexOf(c.family) }))
    .sort((a, b) => b.c.missing - a.c.missing || a.order - b.order)
    .map(({ c }) => {
      const words = `${c.missing} ${FAMILY_NOUN[c.family][c.missing === 1 ? 0 : 1]}`;
      const { held, ids } = heldIn(next, c.family, now);
      if (held === 0) return [{ text: words }];
      const holds = ids.flatMap((id, k): LeftSegment[] => [
        ...(k > 0 ? [{ text: " and " }] : []),
        { text: `the agent, under ${id}`, delegation: id },
      ]);
      return [{ text: `${words} (${held >= c.missing ? "" : `${held} `}` }, ...holds, { text: ")" }];
    });
  if (parts.length === 0) return [{ text: head }];
  const out: LeftSegment[] = [{ text: `${head}: ` }];
  parts.forEach((p, k) => out.push(...(k > 0 ? [{ text: ", " }] : []), ...p));
  return out;
}

export function checksLeftText(next: NextStageWire, now: Date = new Date()): string | null {
  return checksLeftSegments(next, now)?.map((s) => s.text).join("") ?? null;
}

// ---- words, not codes -----------------------------------------------------------------------
export function badgeWords(i: IndexItem): { key: string; text: string; title: string }[] {
  const out: { key: string; text: string; title: string }[] = [];
  if (i.kind) out.push({ key: "kind", text: i.kind.toLowerCase(), title: "Kind" });
  if (i.priority) out.push({ key: "priority", text: `${i.priority.toLowerCase()} priority`, title: "Priority" });
  if (i.assignee) out.push({ key: "assignee", text: `assigned to ${i.assignee}`, title: "Assignee" });
  if (i.team) out.push({ key: "team", text: `team ${i.team}`, title: "Team" });
  for (const l of i.labels ?? []) out.push({ key: `label:${l}`, text: `label: ${l}`, title: "Label" });
  return out;
}

/** A milestone's full name; its code when the milestone is unknown or unnamed. */
export function milestoneFullName(index: CompactIndex, code: string): string {
  const m = index.milestones.find((x) => x.code === code);
  return m?.name ? m.name : code;
}

/** The milestone's name cut to `MILESTONE_CHARS` characters; the full name goes in the title. */
export function milestoneName(index: CompactIndex, code: string): string {
  const full = milestoneFullName(index, code);
  const chars = [...full];
  return chars.length > MILESTONE_CHARS ? `${chars.slice(0, MILESTONE_CHARS - 1).join("")}…` : full;
}

// ---- open groups, bar filters, last look ----------------------------------------------------
const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-");
export const groupId = (laneStatus: string, key: GroupKey): string => `g-${slug(laneStatus)}-${key}`;

/** An attention group is open unless shut; a quiet one is closed unless opened. */
export function isGroupOpen(laneStatus: string, g: Group, open: Set<string>, shut: Set<string>): boolean {
  const id = groupId(laneStatus, g.key);
  return g.attention ? !shut.has(id) : open.has(id);
}

/** The two lists after a toggle: only the list that differs from the default holds the id. */
export function toggleGroup(open: Set<string>, shut: Set<string>, id: string, nowOpen: boolean, byDefault: boolean): { open: Set<string>; shut: Set<string> } {
  const o = new Set(open);
  const s = new Set(shut);
  o.delete(id);
  s.delete(id);
  if (nowOpen !== byDefault) (nowOpen ? o : s).add(id);
  return { open: o, shut: s };
}

/** The groups a lane draws: those named in `only` alone (and forced open), else all. */
export function shownGroups(lane: Lane, only: Set<string>): { groups: Group[]; filtered: boolean } {
  const picked = lane.groups.filter((g) => only.has(groupId(lane.status, g.key)));
  return picked.length > 0 ? { groups: picked, filtered: true } : { groups: lane.groups, filtered: false };
}

export const seenKey = (project: string): string => `kanbanr-board-seen:${project}`;

/** The bar's segments, one per non-empty group in display order. */
export function laneSegments(lane: Lane): Segment[] {
  return lane.groups.filter((g) => g.items.length > 0).map((g) => ({ key: g.key, label: g.label, value: g.items.length, tone: g.tone }));
}

// ---- assembling the lanes -------------------------------------------------------------------
function median(xs: number[]): number | null {
  if (xs.length === 0) return null;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : Math.round((s[m - 1] + s[m]) / 2);
}

function daysSince(day: string | undefined, now: Date): number | null {
  if (!day) return null;
  const t = dayMs(day);
  return Number.isNaN(t) ? null : Math.max(0, Math.floor((now.getTime() - t) / DAY_MS));
}

/** Every lane of the board, grouped; no paging, so every item of a lane is in exactly one group. */
export function boardLanes(input: BoardInput): Lane[] {
  const { project, index, readiness, scopes, critical, seen, now, sprint, text, sort } = input;
  const config = index.config;
  const usesSprints = config.cadence.sprints === true;
  const terminal = terminalStates(config);
  const kinds = laneKinds(config);
  const byCode = new Map(index.items.map((i) => [i.code, i]));
  const agents = new Map<string, AgentScope>();
  for (const s of scopes) if (s.state === "active" && !agents.has(s.item)) agents.set(s.item, s);
  const criticalSet = new Set(critical);
  const needle = text.trim().toLowerCase();
  const matchesText = (f: IndexItem) =>
    !needle || [f.code, f.title, f.kind ?? "", f.priority ?? "", ...(f.labels ?? [])].join(" ").toLowerCase().includes(needle);
  const idleBefore = now.getTime() - IDLE_DAYS * DAY_MS;
  const last = usesSprints ? lastClosedSprint(index) : null;

  const build = (f: IndexItem): BoardItem => {
    const card = readiness?.[f.code];
    const next = card?.next.find((n) => n.immediate !== false);
    let blockedBy: string | null = null;
    for (const d of f.depends_on ?? []) {
      if (d.includes(":")) continue;
      const dep = byCode.get(d);
      if (!dep || !terminal.includes(dep.status)) {
        blockedBy = d;
        break;
      }
    }
    const updated = card?.updated_at ? Date.parse(card.updated_at) : NaN;
    return {
      item: f,
      card,
      next,
      known: readiness !== null,
      daysIn: daysSince(f.entered_on, now),
      blockedBy,
      blockedByStatus: blockedBy ? (byCode.get(blockedBy)?.status ?? null) : null,
      agent: agents.get(f.code) ?? null,
      critical: criticalSet.has(`${project}:${f.code}`),
      idle: !Number.isNaN(updated) && updated < idleBefore,
      changed: seen !== null && !!card?.updated_at && card.updated_at > seen,
    };
  };

  return displayedStates(config).map((status): Lane => {
    const kind = kinds[status];
    const inLane = index.items.filter((f) => f.status === status);
    const flow = flowCounts(index, status, now);
    const lane: Lane = { status, kind, total: 0, groups: [], flow, oldestDays: null, oldestCode: null, usual: null, insight: null, rest: null, window: null, windowSprint: null };

    if (kind === "finished") {
      const win = finishedWindow(index, sprint, now);
      const texted = inLane.filter(matchesText);
      const shown = texted.filter((f) => win.holds(f) && inSprintFilter(f, usesSprints, sprint));
      const shownSet = new Set(shown.map((f) => f.code));
      const items = shown
        .map(build)
        .sort((a, b) => (b.item.entered_on ?? "").localeCompare(a.item.entered_on ?? "") || a.item.code.localeCompare(b.item.code));
      lane.total = texted.length;
      lane.window = win.label;
      lane.windowSprint = win.sprint;
      if (items.length > 0) {
        lane.groups = [{ key: "done", label: win.label, tone: GROUP_TONE.done, attention: false, items, oldest: null }];
      }
      const left = texted.filter((f) => !shownSet.has(f.code));
      if (left.length > 0) {
        const usesReleases = config.cadence.releases === true;
        const k = usesReleases ? new Set(left.map((f) => f.release).filter((r): r is string => !!r)).size : 0;
        const n = left.length;
        lane.rest = usesReleases
          ? { count: n, releases: k, text: k > 0 ? `${n} finished earlier, across ${plural(k, "release")} →` : `${n} finished earlier →`, to: to.releases(project) }
          : { count: n, releases: 0, text: `${n} finished earlier →`, to: to.items(project, { status }) };
      }
      lane.insight = laneInsight(lane, { lastSprintFinished: last?.finished ?? null, kind, lastSprint: last?.code ?? null, windowSprint: win.sprint });
      return lane;
    }

    const all = inLane.map(build);
    lane.usual = median(all.map((b) => b.daysIn).filter((d): d is number => d !== null));
    const kept = all.filter((b) => inSprintFilter(b.item, usesSprints, sprint) && matchesText(b.item));
    lane.total = kept.length;
    const buckets = new Map<GroupKey, BoardItem[]>();
    for (const b of kept) {
      const k = groupOf(b, kind, b.idle);
      (buckets.get(k) ?? buckets.set(k, []).get(k)!).push(b);
    }
    lane.groups = ORDER[kind]
      .filter((k) => (buckets.get(k)?.length ?? 0) > 0)
      .map((k) => {
        const items = sortItems(buckets.get(k)!, sort);
        return { key: k, label: groupLabel(k, kind), tone: GROUP_TONE[k], attention: ATTENTION.includes(k), items, oldest: oldestOf(items) };
      });
    const oldest = oldestOf(kept.filter((b) => b.daysIn !== null));
    if (oldest) {
      lane.oldestDays = oldest.daysIn;
      lane.oldestCode = oldest.item.code;
    }
    lane.insight = laneInsight(lane, { lastSprintFinished: last?.finished ?? null, kind });
    return lane;
  });
}
