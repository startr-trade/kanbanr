/**
 * The timeline model (FTR-046, R-5): lanes of bands and marks over a window of ISO dates. Pure, so
 * the checks import it directly; `Timeline.tsx` is the thin component over it.
 *
 * Dates are ISO `YYYY-MM-DD` (a time part is allowed and kept to the day), read as UTC, so a window
 * is the same on every machine.
 */
import type { Tone } from "./tone.ts";

/**
 * A span on a lane. `progress` (0..1) draws the done part filled and the rest at 12% of the tone;
 * `pending` draws it dashed and labelled "pending"; `to_url` makes it a link.
 */
export interface Band {
  from: string;
  to: string;
  label: string;
  tone?: Tone;
  pending?: boolean;
  to_url?: string;
  progress?: number;
}

/** A point on a lane: a release is a `diamond` (FTR-043 puts releases on their own lane). */
export interface Mark {
  at: string;
  label: string;
  kind: "diamond" | "dot";
}

/** One row of the timeline. */
export interface Lane {
  key: string;
  label: string;
  bands: Band[];
  marks: Mark[];
}

const DAY = 86_400_000;

/** Milliseconds since the epoch of an ISO date's day (UTC), or NaN when it is not a date. */
export function dayOf(iso: string): number {
  const t = Date.parse(iso.length === 10 ? `${iso}T00:00:00Z` : iso);
  return Number.isNaN(t) ? NaN : Math.floor(t / DAY) * DAY;
}

/** The ISO `YYYY-MM-DD` of a day number from `dayOf`. */
export function isoDay(t: number): string {
  return new Date(t).toISOString().slice(0, 10);
}

/**
 * Where `at` falls in the window `from`..`to`: 0 at `from`, 1 at `to`, clamped to 0..1. A window
 * of no length, or a date that cannot be read, gives 0.
 */
export function scale(from: string, to: string, at: string): number {
  const a = dayOf(from);
  const b = dayOf(to);
  const t = dayOf(at);
  if (Number.isNaN(a) || Number.isNaN(b) || Number.isNaN(t) || b <= a) return 0;
  return Math.min(1, Math.max(0, (t - a) / (b - a)));
}

/**
 * The window to draw. When `today` is inside `from`..`to`, the window is widened so it keeps at
 * least `minDays` around today (half before, half after), never cut: the result covers both the
 * given window and today's. When today is outside, or a date cannot be read, the window is as given.
 */
export function clampWindow(from: string, to: string, today: string, minDays: number): { from: string; to: string } {
  const a = dayOf(from);
  const b = dayOf(to);
  const t = dayOf(today);
  if (Number.isNaN(a) || Number.isNaN(b) || Number.isNaN(t) || t < a || t > b) return { from, to };
  const before = Math.floor(minDays / 2);
  const after = minDays - before;
  return { from: isoDay(Math.min(a, t - before * DAY)), to: isoDay(Math.max(b, t + after * DAY)) };
}

/** A band's progress as a whole percentage, clamped to 0..100; `null` when it has none. */
export function percentDone(band: Band): number | null {
  if (band.progress == null || Number.isNaN(band.progress)) return null;
  return Math.round(Math.min(1, Math.max(0, band.progress)) * 100);
}

/** The text the hidden list gives a band: its dates, and whether it is pending or how far done. */
export function bandText(band: Band): string {
  const pct = percentDone(band);
  let text = `${band.label}: ${band.from} to ${band.to}`;
  if (band.pending) text += ", pending";
  if (pct !== null) text += `, ${pct}% done`;
  return text;
}

/** The text the hidden list gives a mark. */
export function markText(mark: Mark): string {
  return `${mark.label}: ${mark.at}`;
}
