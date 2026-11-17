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

/** One piece of a task's text: plain, `code`, or a link (only to `/p/…`, `http://` or `https://`). */
export interface TaskTextPart {
  kind: "text" | "code" | "link";
  text: string;
  href?: string;
}

/** The index of the `)` that closes a target starting at `from` (nested parens allowed), else -1. */
function closingParen(text: string, from: number): number {
  let depth = 0;
  for (let j = from; j < text.length; j++) {
    if (text[j] === "(") depth += 1;
    else if (text[j] === ")") {
      if (depth === 0) return j;
      depth -= 1;
    }
  }
  return -1;
}

/**
 * A task's text as parts: `x` is code, [label](href) a link when href starts with `/p/`, `http://`
 * or `https://` (any other target keeps only its label as text), unbalanced marks stay literal.
 * Nothing is built as HTML: the caller renders each part as an element.
 */
export function taskTextParts(text: string): TaskTextPart[] {
  const parts: TaskTextPart[] = [];
  let plain = "";
  const flush = () => {
    if (plain) parts.push({ kind: "text", text: plain });
    plain = "";
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i + 1) {
        flush();
        parts.push({ kind: "code", text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    } else if (c === "[") {
      const mid = text.indexOf("](", i + 1);
      const close = mid > i ? closingParen(text, mid + 2) : -1;
      const label = mid > i ? text.slice(i + 1, mid) : "";
      if (close > 0 && label.length > 0 && !label.includes("[") && !label.includes("]")) {
        const href = text.slice(mid + 2, close);
        if (/^(\/p\/|https?:\/\/)/.test(href)) {
          flush();
          parts.push({ kind: "link", text: label, href });
        } else plain += label;
        i = close + 1;
        continue;
      }
    }
    plain += c;
    i += 1;
  }
  flush();
  return parts;
}
