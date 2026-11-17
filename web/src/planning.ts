// The person's planning decisions on the pages (FTR-049), as pure functions: the question a page
// asks, the request a target or a drop makes, and the pending decisions keyed for the Plan page's
// lanes and rows. A page records a decision; Claude carries it out with `--decision` or declines
// it. Nothing here posts. web/scripts/check-planning.mjs checks every function.
import type { PlanningRequest, PlanningView } from "./types";

/** "A", "A and B", "A, B and C". */
function list(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** The question a page asks before it records a decision, e.g. "Plan FTR-044 and DEF-076 into SP-004?". */
export function requestText(action: PlanningRequest["action"], target: string, items: string[]): string {
  if (action === "into-sprint") return `Plan ${list(items)} into ${target}?`;
  if (action === "into-release") return `Move ${list(items)} to ${target}?`;
  return `Take ${list(items)} out of ${items.length > 1 ? "their sprints" : "its sprint"}?`;
}

/** The button's words for a request: "Plan into SP-004", "Move to v0.9.0", "Take out of its sprint". */
export function actionLabel(action: PlanningRequest["action"], target: string): string {
  if (action === "into-sprint") return `Plan into ${target}`;
  if (action === "into-release") return `Move to ${target}`;
  return "Take out of its sprint";
}

/**
 * The request a target key makes for `items`: `sprint:SP-004` plans into the sprint,
 * `release:v0.9.0` into the release. Any other key, or no items, gives null.
 */
export function targetRequest(key: string, items: string[]): PlanningRequest | null {
  if (items.length === 0) return null;
  const at = key.indexOf(":");
  const kind = at < 0 ? "" : key.slice(0, at);
  const target = at < 0 ? "" : key.slice(at + 1);
  if (!target) return null;
  if (kind === "sprint") return { action: "into-sprint", target, items: [...items] };
  if (kind === "release") return { action: "into-release", target, items: [...items] };
  return null;
}

/** The lane a decision is pending on: `sprint:SP-004`, `release:v0.9.0`; none for out-of-sprint. */
function laneOf(v: PlanningView): string | null {
  const d = v.decision;
  if (d.action === "into-sprint") return `sprint:${d.target}`;
  if (d.action === "into-release") return `release:${d.target}`;
  return null;
}

/** Pending decisions by the lane they plan into, keyed `sprint:SP-004` or `release:v0.9.0`, oldest first. */
export function pendingByTarget(views: PlanningView[]): Map<string, PlanningView[]> {
  const out = new Map<string, PlanningView[]>();
  for (const v of views) {
    if (v.state !== "pending") continue;
    const key = laneOf(v);
    if (!key) continue;
    out.set(key, [...(out.get(key) ?? []), v]);
  }
  return out;
}

/** Pending decisions by each item they name, oldest first. */
export function pendingByItem(views: PlanningView[]): Map<string, PlanningView[]> {
  const out = new Map<string, PlanningView[]>();
  for (const v of views) {
    if (v.state !== "pending") continue;
    for (const code of new Set(v.decision.items)) out.set(code, [...(out.get(code) ?? []), v]);
  }
  return out;
}

/** The request for a declined decision's nearest valid move, for the same items; null when it has none. */
export function nearestRequest(v: PlanningView): PlanningRequest | null {
  const n = v.state === "declined" ? v.outcome?.nearest : undefined;
  if (!n) return null;
  if (n.action !== "into-sprint" && n.action !== "into-release" && n.action !== "out-of-sprint") return null;
  return { action: n.action, target: n.target, items: [...v.decision.items] };
}

/** The one-press offer's words, e.g. "Ask for v0.8.0 instead"; null when there is no nearest move. */
export function nearestLabel(v: PlanningView): string | null {
  const r = nearestRequest(v);
  if (!r) return null;
  return `Ask for ${r.action === "out-of-sprint" ? "it out of its sprint" : r.target} instead`;
}

/**
 * The request a drop of `item` on a lane drafts (the drag, R-7): a sprint lane is `into-sprint`, a
 * release lane `into-release`; any other lane gives null. It drafts only: the page asks first.
 */
export function dropRequest(item: string, laneKey: string): PlanningRequest | null {
  return targetRequest(laneKey, [item]);
}
