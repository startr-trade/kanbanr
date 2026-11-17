// The system baseline's pure part (FTR-006): the Review page's group card, how a row reads in a
// design-review entry, and the chip a Catalog row carries. No React and no browser API, so
// web::system_baseline (scripts/check-system-baseline.mjs) imports it.
import type { Prepared, SystemBaselineRow, SystemBaselineState } from "./types.ts";

/** The group's key on the Review page: the acceptance is its own group, never a page-level panel. */
export const GROUP_KEY = "system-baseline";

/** What the person runs in a terminal to accept it: shown in place of the button on a read-only monitor. */
export const ACCEPT_COMMAND = "kanbanr catalog baseline accept";

/** What accepting commits the person to, in the group's own words. */
export const ACCEPT_SENTENCE =
  "Accepting says the system design is agreed before the first build: it pins each system row's source as shown, and the build stage waits until it is accepted.";

/** The first four characters of a pin with an ellipsis: "91ab…"; "none" when empty. */
export function shortRev(rev: string): string {
  return rev ? `${rev.slice(0, 4)}…` : "none";
}

/** A row in one line: "context (pinned 91ab… → now 4c12…)". An empty pin or source says so. */
export function rowLine(r: Pick<SystemBaselineRow, "row" | "pinned" | "now">): string {
  const pinned = r.pinned ? `pinned ${shortRev(r.pinned)}` : "not pinned before";
  const now = r.now ? `now ${shortRev(r.now)}` : "no source now";
  return `${r.row} (${pinned} → ${now})`;
}

/** The design-review entry's rows as one sentence, or "" when the design changes none. */
export function entryRowsLine(rows: SystemBaselineRow[] | undefined): string {
  if (!rows || rows.length === 0) return "";
  return `Changes the system baseline: ${rows.map(rowLine).join("; ")}`;
}

/** What the group's action is: a button that posts, the command to run, or why neither is offered yet. */
export type CardAction =
  | { kind: "button"; label: string; rev: string }
  | { kind: "command"; command: string }
  | { kind: "blocked"; why: string };

/** The card the `system-baseline` group's row opens to. */
export interface BaselineCard {
  heading: string;
  /** "System baseline — 3 rows, 2 pinned; accepting says the system design is agreed before the first build". */
  body: string;
  /** Each row of the baseline, as the card lists it. */
  rows: { row: string; text: string }[];
  /** The prepared acceptance's id and its note, when one waits; the note is the person's to edit. */
  prepared: string;
  note: string;
  action: CardAction;
}

/**
 * The group's card, or null while there is nothing to accept: the group shows only while the
 * baseline awaits its first acceptance or an acceptance is prepared (an accepted or changed baseline
 * shows none). `writable` false gives the command, never a button (check:read-only).
 */
export function cardFor(
  state: SystemBaselineState,
  prepared: Prepared | null | undefined,
  writable: boolean,
): BaselineCard | null {
  if (state.kind !== "awaiting" && !prepared) return null;
  const unpinned = new Set(state.unpinned ?? []);
  const missing = new Set(state.missing ?? []);
  const pinned = state.rows.filter((r) => !unpinned.has(r) && !missing.has(r)).length;
  const rows = state.rows.map((row) => ({
    row,
    text: missing.has(row) ? `${row}: no source yet` : unpinned.has(row) ? `${row}: not pinned (kept by its own tool)` : `${row}: pinned`,
  }));
  const note = typeof prepared?.fields?.note === "string" ? (prepared.fields.note as string) : "";
  const action: CardAction = !state.can_accept
    ? { kind: "blocked", why: state.cannot_accept_why ?? "the system docs are not all written yet" }
    : writable
      ? { kind: "button", label: "Accept", rev: state.pins_rev }
      : { kind: "command", command: ACCEPT_COMMAND };
  return {
    heading: "System baseline",
    body: `System baseline — ${state.rows.length} ${state.rows.length === 1 ? "row" : "rows"}, ${pinned} pinned; ${ACCEPT_SENTENCE.charAt(0).toLowerCase()}${ACCEPT_SENTENCE.slice(1)}`,
    rows,
    prepared: prepared?.id ?? "",
    note,
    action,
  };
}

/** A chip on a Catalog row: its words, its tone and what it means. */
export interface BaselineChip {
  label: string;
  tone: "ok" | "warn" | "plain";
  title: string;
}

/** The day of an RFC 3339 instant. */
const day = (at: string) => at.slice(0, 10);

/**
 * The baseline chip of Catalog row `id`: `pinned`, `changed since <date>`, `not pinned` or
 * `awaiting`; null for a row the baseline does not hold (or before it is initialised).
 */
export function catalogChip(id: string, state: SystemBaselineState | null | undefined): BaselineChip | null {
  if (!state || state.kind === "not_initialised" || !state.rows.includes(id)) return null;
  if ((state.unpinned ?? []).includes(id)) {
    return { label: "not pinned", tone: "plain", title: "This row's source is kept by another tool, so the baseline cannot pin it." };
  }
  if (state.kind === "awaiting") {
    return { label: "awaiting", tone: "warn", title: "The system baseline has not been accepted yet." };
  }
  const changed = (state.changes ?? []).some((c) => c.row === id);
  if (changed) {
    const since = state.accepted ? ` ${day(state.accepted.at)}` : "";
    return { label: `changed since${since}`, tone: "warn", title: "This row's source differs from the accepted baseline." };
  }
  return { label: "pinned", tone: "ok", title: "This row's source reads as it did when the baseline was accepted." };
}

/**
 * The catalog rows an item's design changes, for the item page's "Views changed" chips: those the
 * review queue derived from its docs, then any the definition names that the queue did not list;
 * each once, in that order.
 */
export function viewsChanged(derived: string[] | undefined, named: string[] | undefined): string[] {
  return [...new Set([...(derived ?? []), ...(named ?? [])])];
}

/** Where a catalog row is on the Catalog page: its anchor (`#row-<id>`). */
export function catalogRowHref(project: string, row: string): string {
  return `/p/${encodeURIComponent(project)}/catalog#row-${encodeURIComponent(row)}`;
}
