// Whether the Needs-you drawer is open (FTR-048): navReducer's pattern (FTR-047) with its own
// state, pure so node checks it without React. Escape closes it and asks for focus back on the
// badge; a press outside, reaching another page, or following a link closes it without moving focus.
import type { Attention, AttentionLine, AttentionSection, DriftLatest } from "./types";
import { driftLabel } from "./driftAge.ts";

export type NeedsYouEvent = { type: "toggle" } | { type: "escape" } | { type: "outside" } | { type: "navigate" } | { type: "close" };

export interface NeedsYouState {
  open: boolean;
  /** Set after Escape: the shell moves focus back to the badge, then sends `close`. */
  returnFocus: boolean;
}

export const CLOSED: NeedsYouState = { open: false, returnFocus: false };

export function needsYouReducer(s: NeedsYouState, e: NeedsYouEvent): NeedsYouState {
  switch (e.type) {
    case "toggle":
      return { open: !s.open, returnFocus: false };
    case "escape":
      return s.open ? { open: false, returnFocus: true } : s;
    case "outside":
    case "navigate":
    case "close":
      return CLOSED;
  }
}

/** The drawer's sections, in the order it shows them, with their headings. */
export const SECTIONS: { key: AttentionSection; title: string }[] = [
  { key: "decisions", title: "Decisions" },
  { key: "releases", title: "Releases" },
  { key: "health", title: "Health" },
  { key: "records", title: "Records" },
];

/** The sentence the drift line offers: there is no drift page in v0.8.0. */
export const DRIFT_ASK = "Ask Claude to settle the drifted docs.";

/** A drawer line: an attention line, with a note after its count (the drift result's age). */
export type DrawerLine = AttentionLine & { note?: string };

/**
 * The Health line the drawer adds from the daemon's latest drift result (FTR-045): "Drifted docs N ·
 * as of …", only when there is a result with flags.
 */
export function driftLine(drift: DriftLatest | null, now: Date): DrawerLine | null {
  if (!drift || drift.count <= 0 || !drift.as_of) return null;
  if (drift.state !== "fresh" && drift.state !== "stale") return null;
  return {
    kind: "drift",
    section: "health",
    label: "Drifted docs",
    note: driftLabel(drift, now),
    count: drift.count,
    ask: DRIFT_ASK,
    tone: "warn",
    yours: false,
  };
}

/** The lines of each section in the drawer's order, the drift line last under Health; empty sections left out. */
export function drawerSections(attention: Attention, drift: DriftLatest | null, now: Date): { key: AttentionSection; title: string; lines: DrawerLine[] }[] {
  const all: DrawerLine[] = [...(attention.lines ?? [])];
  const d = driftLine(drift, now);
  if (d) all.push(d);
  return SECTIONS.map((s) => ({ ...s, lines: all.filter((l) => l.section === s.key) })).filter((s) => s.lines.length > 0);
}

/** The badge's accessible name: "Needs you: 3 decisions waiting". */
export function badgeLabel(total: number): string {
  if (total <= 0) return "Needs you: no decisions waiting";
  return `Needs you: ${total} decision${total === 1 ? "" : "s"} waiting`;
}
