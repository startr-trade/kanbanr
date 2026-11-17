// Whether the Needs-you drawer is open (FTR-048): navReducer's pattern (FTR-047) with its own
// state, pure so node checks it without React. Escape closes it and asks for focus back on the
// badge; a press outside, reaching another page, or following a link closes it without moving focus.
import type { Attention, AttentionLine, AttentionSection, DelegatedDecision, DelegationEnding, DelegationHold, DriftLatest } from "./types";
import { dayMonth } from "./cardText.ts";
import { driftLabel } from "./driftAge.ts";
import { to } from "./overviewLinks.ts";

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
export function drawerSections(attention: Attention, drift: DriftLatest | null, now: Date): DrawerSection[] {
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

// ---- Needs you in delegated mode (DEF-104) -------------------------------------------------

/** How long a look lasts before it counts as seen (the Board's and Charter's value too). */
export const SEEN_AFTER_MS = 10_000;
/** The most fresh decisions Needs you lists; the rest are "more on the Review page". */
export const DECIDED_SHOWN = 10;

/** Where this browser keeps when Needs you was last looked at; one key for the drawer and the Overview. */
export const seenKey = (project: string): string => `kanbanr-needs-you-seen:${project}`;

export interface DrawerSection {
  key: AttentionSection;
  title: string;
  lines: DrawerLine[];
}
/** A delegation that ends soon or just ended; `state` picks the pill's word. */
export interface EndingLine {
  delegation: string;
  state: DelegationEnding["state"];
  text: string;
  to: string;
}
/** What one delegation in force still holds, in words. */
export interface HoldLine {
  delegation: string;
  text: string;
  to: string;
}
export interface NeedsYouParts {
  /** A delegation is in force: the drawer reads in three parts. */
  delegated: boolean;
  /** The person's own lines; without a delegation, every section as `drawerSections` gives them. */
  yours: DrawerSection[];
  /** Delegations ending soon, ended or revoked: shown first under Yours, never counted. */
  ending: EndingLine[];
  /** What Yours says when it has no line, naming the delegations in force. */
  emptyYours: string | null;
  decided: { fresh: DelegatedDecision[]; earlier: number; earlierList: DelegatedDecision[]; more: number };
  holds: HoldLine[];
  heldTotal: number;
  /** Health and Records, after the three parts. */
  others: DrawerSection[];
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
const andList = (ids: string[]) => (ids.length <= 1 ? ids.join("") : `${ids.slice(0, -1).join(", ")} and ${ids[ids.length - 1]}`);

/** "approval", "design review", "docs-folded sign-off": what a held verdict is called, counted. */
export function verdictNoun(verdict: string, n: number): string {
  if (verdict === "approve") return n === 1 ? "approval" : "approvals";
  if (verdict === "ratify") return n === 1 ? "ratification" : "ratifications";
  const name = verdict.startsWith("signoff:") ? verdict.slice("signoff:".length) : verdict;
  if (name === "design-review") return n === 1 ? "design review" : "design reviews";
  return `${name} ${n === 1 ? "sign-off" : "sign-offs"}`;
}

/** "DLG-001: 4 design reviews, 2 approvals · until 4 Dec". */
export function heldWords(h: DelegationHold): string {
  const what = h.verdicts.map((v) => `${v.count} ${verdictNoun(v.verdict, v.count)}`).join(", ");
  return `${h.delegation}: ${what} · until ${dayMonth(h.until)}`;
}

/** "DLG-001 ends on 4 Dec: 6 decisions return to you", and the ended and revoked forms. */
export function endingWords(e: DelegationEnding): string {
  const day = dayMonth(e.at);
  const head = e.state === "granted" ? `${e.delegation} ends on ${day}` : e.state === "revoked" ? `${e.delegation} was revoked on ${day}` : `${e.delegation} ended on ${day}`;
  if (e.returns === 0) return `${head}: nothing returns to you`;
  const tail = e.state === "granted" ? plural(e.returns, "decision returns", "decisions return") : plural(e.returns, "decision came", "decisions came");
  return `${head}: ${tail}${e.state === "granted" ? "" : " back"} to you`;
}

/** The two halves of a decided line: what Claude did, and when ("16 Nov 14:02", UTC). */
export function decidedParts(d: DelegatedDecision): { what: string; when: string } {
  const what =
    d.verdict === "approve" ? `approved ${d.code}` : d.verdict === "ratify" ? `ratified ${d.code}` : `signed off ${d.signoff || "a sign-off"} on ${d.code}`;
  const t = new Date(Date.parse(d.at));
  const when = Number.isNaN(t.getTime()) ? d.at : `${dayMonth(d.at)} ${t.toISOString().slice(11, 16)}`;
  return { what, when };
}

/** "approved FTR-041 · 16 Nov 14:02 · DLG-001". */
export function decidedWords(d: DelegatedDecision): string {
  const { what, when } = decidedParts(d);
  return `${what} · ${when} · ${d.delegation}`;
}

/**
 * The drawer's and the Overview's content. Without a delegation in force it is today's sections
 * (with any ending line first). With one it is three parts: Yours (what no delegation covers),
 * what Claude decided since `seen`, and what each delegation still holds; Health and Records
 * follow, uncounted. `seen` is the ISO time of the last look in this browser, or null for none.
 */
export function needsYouParts(project: string, attention: Attention, drift: DriftLatest | null, now: Date, seen: string | null): NeedsYouParts {
  const d = attention.delegated;
  const ending: EndingLine[] = (d?.ending ?? []).map((e) => ({ delegation: e.delegation, state: e.state, text: endingWords(e), to: to.delegation(project, e.delegation) }));
  const sections = drawerSections(attention, drift, now);
  const none = { fresh: [], earlier: 0, earlierList: [], more: 0 };
  if (!d || d.in_force.length === 0) {
    return { delegated: false, yours: sections, ending, emptyYours: null, decided: none, holds: [], heldTotal: 0, others: [] };
  }
  const yours = sections
    .filter((s) => s.key === "decisions" || s.key === "releases")
    .map((s) => ({ ...s, lines: s.lines.filter((l) => l.kind !== "delegated") }))
    .filter((s) => s.lines.length > 0);
  const others = sections.filter((s) => s.key === "health" || s.key === "records");
  const seenAt = seen === null ? Number.NEGATIVE_INFINITY : Date.parse(seen);
  const isFresh = (x: DelegatedDecision) => !(Date.parse(x.at) <= seenAt);
  const fresh = d.decided.filter(isFresh);
  const shown = fresh.slice(0, DECIDED_SHOWN);
  const earlierList = d.decided.filter((x) => !isFresh(x));
  const holds: HoldLine[] = d.holds.map((h) => ({ delegation: h.delegation, text: heldWords(h), to: to.delegation(project, h.delegation) }));
  return {
    delegated: true,
    yours,
    ending,
    emptyYours: yours.length === 0 && ending.length === 0 ? `Nothing needs you \u2014 Claude is working under ${andList(d.in_force)}` : null,
    decided: { fresh: shown, earlier: earlierList.length, earlierList, more: fresh.length - shown.length + Math.max(0, d.decided_total - d.decided.length) },
    holds,
    heldTotal: d.holds.reduce((n, h) => n + h.total, 0),
    others,
  };
}
