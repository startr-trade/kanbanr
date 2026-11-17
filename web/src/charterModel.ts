// The Charter page's model (FTR-076): pure functions over the derived read
// (`GET /projects/{p}/charter/progress`) and the charter, so node checks import them without a
// browser. The page components only draw what these return.
import { to } from "./overviewLinks.ts";
import type { Charter, CharterProgress, GoalProgress, LessonRow, RouteStop, SeriesPoint } from "./types";
import type { MatrixCell } from "./ui/Matrix.tsx";
import type { Step } from "./ui/stepper.ts";

/** A goal's statement longer than this keeps its first line in the row and the rest in `<details>`. */
export const FOLD_STATEMENT = 120;
/** A non-goal, stakeholder or constraint longer than this folds the same way. */
export const FOLD_ENTRY = 160;
/** How long the page stays open before the last-visit time is written. */
export const SEEN_AFTER_MS = 10_000;

/** The `localStorage` key that keeps when this browser last looked at a project's charter. */
export const seenKey = (p: string): string => `kanbanr-charter-seen:${p}`;

/** `text` cut into the part shown and the part kept in `<details>`; `rest` is "" when it fits. */
export function fold(text: string, limit: number): { head: string; rest: string } {
  const line = text.split("\n")[0] ?? "";
  if (text.length <= limit && line === text) return { head: text, rest: "" };
  if (line.length <= limit && line !== text) return { head: line, rest: text.slice(line.length).trim() };
  const cut = line.length > limit ? line.slice(0, limit).replace(/\s+\S*$/, "") || line.slice(0, limit) : line;
  return { head: cut + (line.length > limit ? "…" : ""), rest: text };
}

/** One row of the Goals table. */
export interface GoalRow {
  id: string;
  statement: string;
  head: string;
  rest: string;
  measure: string;
  horizon: string;
  itemsDone: number;
  itemsOpen: number;
  items: number;
  proven: number;
  total: number;
  /** No item serves the goal. */
  unserved: boolean;
  /** Ids of the lessons that name the goal. */
  lessons: string[];
  /** The themes of those lessons, for the hover. */
  lessonThemes: string[];
  itemsTo: string;
  provenTo: string;
  lessonsTo: string;
}

/** The charter's goals in order, each with its counts from the progress read (zeros when it is absent). */
export function goalRows(progress: CharterProgress | null, charter: Charter): GoalRow[] {
  const p = progress?.project ?? "";
  const byId = new Map<string, GoalProgress>((progress?.goals ?? []).map((g) => [g.id, g]));
  const lessonRows = new Map<string, LessonRow>((progress?.lessons.rows ?? []).map((r) => [r.id, r]));
  return charter.goals.map((g) => {
    const f = byId.get(g.id);
    const { head, rest } = fold(g.statement, FOLD_STATEMENT);
    const lessons = f?.lessons ?? [];
    const themes = new Set<string>();
    for (const id of lessons) for (const t of lessonRows.get(id)?.tags ?? []) themes.add(t);
    const done = f?.items_done ?? 0;
    const open = f?.items_open ?? 0;
    return {
      id: g.id,
      statement: g.statement,
      head,
      rest,
      measure: g.measure ?? "",
      horizon: g.horizon ?? "",
      itemsDone: done,
      itemsOpen: open,
      items: done + open,
      proven: f?.req_proven ?? 0,
      total: f?.req_total ?? 0,
      unserved: done + open === 0,
      lessons,
      lessonThemes: [...themes].sort(),
      itemsTo: to.items(p, { goal: g.id }),
      provenTo: to.items(p, { goal: g.id, evidence: "proven" }),
      lessonsTo: `${to.lessons(p)}`,
    };
  });
}

/**
 * The stops of the way travelled as the stepper draws them: each shipped release `done` with its ship
 * date, the next one `next` (labelled with the target the person set, or "no date set").
 */
export function routeSteps(progress: CharterProgress): Step[] {
  return (progress.route ?? []).map((s): Step => {
    if (s.state === "shipped") {
      return s.shipped_at ? { key: s.version, label: s.version, state: "done", at: s.shipped_at } : { key: s.version, label: s.version, state: "done" };
    }
    return { key: s.version, label: `${s.version} · ${s.target ? `target ${s.target}` : "no date set"}`, state: "next" };
  });
}

/** "Today: after v0.7.0, working toward v0.8.0." */
export function todayLine(progress: CharterProgress): string {
  const route = progress.route ?? [];
  const shipped = route.filter((s) => s.state === "shipped");
  const next = route.find((s) => s.state === "next");
  const after = shipped.length ? `after ${shipped[shipped.length - 1].version}` : "before the first release";
  return next ? `Today: ${after}, working toward ${next.version}.` : `Today: ${after}.`;
}

/** One cell of the goals-by-releases matrix: the items serving `goal` in `version`, linking to them. */
export function matrixCell(progress: CharterProgress, goal: string, version: string): MatrixCell {
  const mark = (progress.marks ?? []).find((m) => m.goal === goal && m.release === version);
  const value = mark?.items ?? 0;
  return value > 0 ? { value, to: to.items(progress.project, { goal, release: version }) } : { value };
}

/** A goal's points, one per shipped release; empty when it has none. */
export function seriesFor(progress: CharterProgress, goal: string): SeriesPoint[] {
  return (progress.series ?? []).find((s) => s.goal === goal)?.points ?? [];
}

/** The y maximum every goal's chart shares, so the panels compare: the largest `req_total`, at least 1. */
export function sharedMax(progress: CharterProgress): number {
  let max = 1;
  for (const s of progress.series ?? []) for (const pt of s.points) max = Math.max(max, pt.req_total, pt.req_proven);
  return max;
}

/** One theme with its lessons and the kinds as words ("4 pitfalls, 2 practices"). */
export interface ThemeRow {
  key: string;
  label: string;
  count: number;
  kinds: string;
  rows: LessonRow[];
}

const KIND_WORD: Record<LessonRow["kind"], [string, string]> = {
  practice: ["practice", "practices"],
  pitfall: ["pitfall", "pitfalls"],
  decision: ["decision", "decisions"],
};

/** The themes in the server's order, each with its lesson rows and a kinds line. */
export function themeRows(progress: CharterProgress): ThemeRow[] {
  const byId = new Map((progress.lessons.rows ?? []).map((r) => [r.id, r]));
  return (progress.lessons.themes ?? []).map((t) => {
    const rows = t.lessons.map((id) => byId.get(id)).filter((r): r is LessonRow => r !== undefined);
    const counts = new Map<LessonRow["kind"], number>();
    for (const r of rows) counts.set(r.kind, (counts.get(r.kind) ?? 0) + 1);
    const kinds = [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([k, n]) => `${n} ${KIND_WORD[k][n === 1 ? 0 : 1]}`)
      .join(", ");
    return { key: t.key, label: t.label, count: rows.length, kinds, rows };
  });
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "12 Nov" from an RFC 3339 time, in UTC so the same answer reads the same everywhere. */
export function shortDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** What changed since the person last looked: a sentence and a link for each count. */
export interface Since {
  text: string;
  links: { label: string; to: string }[];
}

/**
 * One line of what changed since `seenIso`, or null for a first visit (`seenIso` null), an unreadable
 * time, or no change. Counts the finished items that serve a goal, the releases shipped and the
 * lessons recorded after that time.
 */
export function sinceLastVisit(progress: CharterProgress, seenIso: string | null): Since | null {
  if (seenIso === null) return null;
  const seen = Date.parse(seenIso);
  if (Number.isNaN(seen)) return null;
  const after = (iso: string | undefined): boolean => !!iso && Date.parse(iso) > seen;
  const p = progress.project;
  const finished = (progress.finished ?? []).filter((f) => after(f.at));
  const shipped = (progress.route ?? []).filter((s: RouteStop) => s.state === "shipped" && after(s.shipped_at));
  const lessons = (progress.lessons.rows ?? []).filter((l) => after(l.at));
  const parts: string[] = [];
  const links: Since["links"] = [];
  if (finished.length) {
    const goals = [...new Set(finished.flatMap((f) => f.goals))].sort();
    const label = plural(finished.length, "item", "items");
    parts.push(`${label} finished toward ${goals.join(goals.length === 2 ? " and " : ", ")}`);
    links.push({ label, to: to.items(p) });
    for (const g of goals) links.push({ label: g, to: to.items(p, { goal: g }) });
  }
  for (const s of shipped) {
    parts.push(`${s.version} shipped`);
    links.push({ label: s.version, to: to.release(p, s.version) });
  }
  if (lessons.length) {
    const label = plural(lessons.length, "lesson", "lessons");
    parts.push(`${label} recorded`);
    links.push({ label, to: to.lessons(p) });
  }
  if (parts.length === 0) return null;
  return { text: `Since ${shortDate(seenIso)}: ${parts.join(", ")}.`, links };
}
