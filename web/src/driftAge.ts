// The one wording for a background drift result's age (FTR-045): pages show the latest result the
// daemon kept, with the code revision and the time it was computed at, never a fresh computation.
import type { DriftLatest } from "./types";

/** The text the monitor shows when the daemon has no code repository (R-8). */
export const UNAVAILABLE = "Drift unavailable: this monitor has no code repository";

/** A commit's short form, as the monitor shows revisions. */
const short = (rev: string) => rev.slice(0, 7);

/** "just now" (< 45 s), "N min ago", "N h ago", "N d ago". A time in the future reads "just now". */
export function ago(iso: string, now: Date): string {
  const secs = Math.floor((now.getTime() - new Date(iso).getTime()) / 1000);
  if (!Number.isFinite(secs) || secs < 45) return "just now";
  if (secs < 3600) return `${Math.max(1, Math.floor(secs / 60))} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86400)} d ago`;
}

/** "as of 3f2a1c9, 4 min ago" · "as of 3f2a1c9, 4 min ago · recomputing…" · "computing…" ·
 * "Drift unavailable: this monitor has no code repository" · "failed 2 min ago: <reason>".
 * The revision is the code repository's. */
export function driftLabel(latest: DriftLatest, now: Date): string {
  switch (latest.state) {
    case "unavailable":
      return UNAVAILABLE;
    case "failed":
      return `failed ${ago(latest.failed_at ?? "", now)}: ${latest.reason ?? ""}`;
    case "computing":
      return "computing…";
    default: {
      const asOf = latest.as_of;
      if (!asOf) return "computing…";
      const label = `as of ${short(asOf.code_rev)}, ${ago(asOf.finished_at, now)}`;
      return latest.recomputing ? `${label} · recomputing…` : label;
    }
  }
}

/** The label's title attribute: the board revision the result was computed at, shown on hover;
 * "" before the first result. */
export function driftTitle(latest: DriftLatest): string {
  return latest.as_of ? `board ${short(latest.as_of.board_rev)}` : "";
}
