/**
 * The Plan page's model (FTR-043): its tabs from the summary read, the timeline of a tab's slice and
 * of an opened row's items, the open rows in the URL hash, and where the old Gantt and Schedule
 * pages now go. Pure, so the checks import it directly.
 *
 * Nothing here works out a date: the daemon places every row and item, and an undated one is never
 * drawn (no made-up date, R-3).
 */
import { pickTab, type TabDef } from "./ui/tabs.ts";
import { parseOpen } from "./ui/urlState.ts";
import type { Lane, Mark } from "./ui/timeline.ts";
import type { Tone } from "./ui/tone.ts";
import type { IndexRelease, IndexSprint, PlanBand, PlanScope, PlanSlice, PlanSummary, PlanningView, RowBody } from "./types";

/** The three tabs, in the page's order. */
export const PLAN_SCOPES: PlanScope[] = ["current", "planned", "past"];

const LABEL: Record<PlanScope, string> = { current: "Current", planned: "Planned", past: "Past" };

/** The Plan page's tabs (R-1): Current, Planned and Past, each with its count from the summary. */
export function planTabs(s: PlanSummary): TabDef[] {
  return PLAN_SCOPES.map((key) => ({ key, label: LABEL[key], count: s[key] }));
}

/**
 * The tab the page opens on (R-1): the one the URL asks for, else the summary's `open` (Current when
 * anything runs today, else Planned), through `pickTab`.
 */
export function defaultTab(s: PlanSummary, requested: string | null): PlanScope {
  const key = pickTab(planTabs(s), requested, s.open);
  return (PLAN_SCOPES as string[]).includes(key) ? (key as PlanScope) : "current";
}

const TONES: Tone[] = ["ok", "info", "warn", "error", "neutral"];

/** A daemon tone as the monitor's `Tone`; an unknown one is `neutral`. */
function tone(t: string): Tone {
  return (TONES as string[]).includes(t) ? (t as Tone) : "neutral";
}

/**
 * A daemon band as a `Timeline` band: its tone and progress kept, and `pending` when a planning
 * decision waits to plan into its lane (FTR-049).
 */
function band(b: PlanBand, pending: boolean) {
  return {
    from: b.from,
    to: b.to,
    label: b.label,
    tone: tone(b.tone),
    pending,
    ...(b.progress == null ? {} : { progress: b.progress }),
  };
}

/** A daemon mark as a `Timeline` mark: a release's target is a `diamond`, a stage change a `dot`. */
function mark(m: { at: string; label: string; kind: string }): Mark {
  return { at: m.at, label: m.label, kind: m.kind === "diamond" ? "diamond" : "dot" };
}

/**
 * A tab's slice as `Timeline` props (R-10): its window and today as the daemon gave them, and one
 * lane per dated row (`key` is the row's key). A release keeps its own lane of diamonds and dots,
 * with no band. Undated rows are not drawn. A lane that `pending` (planning.ts's `pendingByTarget`)
 * holds has its bands marked pending: a decision waits for Claude to plan into it (FTR-049).
 */
export function toTimeline(
  slice: PlanSlice,
  pending: ReadonlyMap<string, PlanningView[]> = new Map(),
): { from: string; to: string; today: string; lanes: Lane[] } {
  return {
    from: slice.from,
    to: slice.to,
    today: slice.today,
    lanes: slice.lanes.map((l) => ({
      key: l.key,
      label: l.label,
      bands: l.bands.map((b) => band(b, (pending.get(l.key)?.length ?? 0) > 0)),
      marks: l.marks.map(mark),
    })),
  };
}

/**
 * An opened row's items as `Timeline` lanes (R-3): one lane per placed item, its band over the
 * dates the daemon placed it at; an item on the critical path is drawn `warn`. Undated items get no
 * lane: the page lists them apart.
 */
export function itemBands(body: RowBody): Lane[] {
  return body.items
    .filter((i) => i.placed !== "undated" && i.from !== "" && i.to !== "")
    .map((i) => ({
      key: i.code,
      label: i.code,
      bands: [{ from: i.from, to: i.to, label: i.title, tone: i.critical ? "warn" : "info", pending: false }],
      marks: [],
    }));
}

/**
 * The open rows in the URL hash (R-6), as row keys: `#sprint:SP-003` opens that row, and a bare
 * code (the Overview's `#SP-004`) is read as a sprint.
 */
export function openLanes(hash: string): Set<string> {
  return new Set([...parseOpen(hash)].map((id) => (id.includes(":") ? id : `sprint:${id}`)));
}

/**
 * Where an old page's path goes (R-6): `…/schedule` to `…/plan?tab=planned`, `…/gantt` to
 * `…/plan`; `null` for any other path.
 */
export function legacyRedirect(path: string): string | null {
  const m = /^(.*\/)(schedule|gantt)\/?$/.exec(path);
  if (!m) return null;
  return m[2] === "schedule" ? `${m[1]}plan?tab=planned` : `${m[1]}plan`;
}

/** How long a declined planning decision stays on its items' rows (FTR-049). */
export const DECLINED_DAYS = 14;

/**
 * The latest declined planning decision per item, declined within `DECLINED_DAYS` of `today`
 * (`YYYY-MM-DD`), so the row can show the reason and offer the nearest move (FTR-049, R-5).
 */
export function declinedByItem(views: PlanningView[], today: string): Map<string, PlanningView> {
  const out = new Map<string, PlanningView>();
  const day = Date.parse(`${today}T00:00:00Z`);
  for (const v of views) {
    if (v.state !== "declined" || !v.outcome) continue;
    const at = Date.parse(v.outcome.at);
    if (Number.isFinite(day) && Number.isFinite(at) && day - at > DECLINED_DAYS * 86_400_000) continue;
    for (const code of v.decision.items) {
      const seen = out.get(code);
      if (!seen || (seen.outcome?.at ?? "") <= v.outcome.at) out.set(code, v);
    }
  }
  return out;
}

/**
 * Where the Plan page offers to plan items (FTR-049), from the compact index: the sprints that have
 * not closed, then the releases still planned (a release at a stage, shipped or superseded is
 * frozen), each in the index's order. `PlanningAction`'s targets: key `sprint:SP-004` or
 * `release:v0.9.0`, label the code or version.
 */
export function planTargets(sprints: IndexSprint[], releases: IndexRelease[]): { key: string; label: string }[] {
  return [
    ...sprints.filter((s) => s.state !== "closed").map((s) => ({ key: `sprint:${s.code}`, label: s.code })),
    ...releases.filter((r) => r.state === "planned" && !r.stage).map((r) => ({ key: `release:${r.version}`, label: r.version })),
  ];
}
