/**
 * The Flow panel's words (FTR-061), pure so the checks import them: agent-hours, the stats line,
 * each wait in words (FTR-060's texts, as `kanbanr plan` prints them), the forecast rows, and the
 * chips of the dates a person set. Nothing here makes a date: a forecast is hours from now.
 */
import type { FlowForecast, FlowPlan, TaskWait } from "./types";

/** Agent-hours: minutes under 1 h (`45 min`), else hours to one decimal (`14.0 h`). */
export function hoursText(h: number): string {
  if (h < 1) return `${Math.round(h * 60)} min`;
  return `${h.toFixed(1)} h`;
}

/** "Remaining 46.5 agent-h · critical path 12.0 h over 7 tasks · up to 6 at once". */
export function statsLine(plan: FlowPlan): string {
  const n = plan.critical_path.length;
  return `Remaining ${plan.remaining_h.toFixed(1)} agent-h · critical path ${hoursText(plan.critical_h)} over ${n} ${n === 1 ? "task" : "tasks"} · up to ${plan.widest} at once`;
}

/** What a blocked task (`id`, `CODE/TL/T`) waits on, in words. */
export function waitText(id: string, w: TaskWait): string {
  if ("Dependency" in w) return `waits on ${w.Dependency}`;
  if ("Conflict" in w) {
    const { task, paths } = w.Conflict;
    return paths.length === 0
      ? `${task} is in progress in the same item, and its files are not recorded`
      : `writes ${paths.join(", ")}, which ${task} is writing`;
  }
  if ("ItemNotAtBuild" in w) return `${id.split("/")[0]} is in ${w.ItemNotAtBuild}, not at the build stage`;
  if ("UnknownFootprint" in w) return `it records no files, and ${w.UnknownFootprint} of its item is in progress`;
  return JSON.stringify(w);
}

/** One forecast row's words, and whether the build slots bind it. */
export interface ForecastRow {
  forecast: FlowForecast;
  text: string;
  bound: boolean;
}

/** "4 lanes · 2 build slots → 14.0 h", with "(slots allow 2)" when the slots bind. */
export function forecastRows(plan: FlowPlan): ForecastRow[] {
  return plan.forecasts.map((f) => {
    const bound = f.effective_lanes < f.lanes;
    const base = `${f.lanes} ${f.lanes === 1 ? "lane" : "lanes"} · ${f.slots} build ${f.slots === 1 ? "slot" : "slots"} → ${hoursText(f.finish_h)}`;
    return { forecast: f, text: bound ? `${base} (slots allow ${f.effective_lanes})` : base, bound };
  });
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `YYYY-MM-DD` as `4 Dec 2026`; anything else as it is. */
function dateText(d: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(d);
  const month = m ? Number(m[2]) : 0;
  if (!m || month < 1 || month > 12) return d;
  return `${Number(m[3])} ${MONTHS[month - 1]} ${m[1]}`;
}

/** "SP-004 ends 4 Dec 2026 (set by the person)": the plan's only dates, each a person's. */
export function dateChips(plan: FlowPlan): string[] {
  return plan.dates.map((d) => `${d.what} ${dateText(d.date)} (set by the person)`);
}
