/**
 * What a board card says about an item's next stage (FEAT-205), in plain words.
 *
 * The pill used to read "→ In Progress ✓", which looked like the item's status and claimed ready
 * while tasks or sign-offs were still outstanding. Now: what the next stage still needs, that it is
 * waiting for a sign-off, or that it is ready — and nothing at all while the item's tasks are open,
 * since the tasks chip already says so and "ready" would not be true.
 */
import type { CheckTally, DelegatedGap } from "./types";

export interface NextStage {
  status: string;
  /** Everything the stage still lacks, its sign-offs included. */
  missing: number;
  /** Sign-offs the stage still needs. */
  signoffs?: string[];
  /** The item's tasks not yet done. */
  tasks_open?: number;
  /** The stage ends the work. */
  ends?: boolean;
  /** The stage the item reaches next. Absent counts as true, for an older daemon (DEF-020). */
  immediate?: boolean;
  /** How many of `missing` the user settles: sign-offs and approval. Absent counts as 0. */
  for_you?: number;
  /** What the stage lacks by family; absent from an older daemon (FTR-078). */
  checks?: CheckTally[];
  /** The gaps a delegation in force holds; absent from an older daemon (DEF-104). */
  delegated?: DelegatedGap[];
}

export interface CardHint {
  text: string;
  title: string;
  /** "needs" and "waiting" are things to do; "ready" is not. */
  kind: "needs" | "waiting" | "ready";
}

/** "4 Dec": a day and a month in UTC, for "until 4 Dec" (DEF-104). */
export function dayMonth(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return new Date(t).toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" });
}

/** The gaps a delegation still holds at `now`; one whose end has passed is the person's again (R-2). */
export function liveDelegated(n: NextStage, now: Date): DelegatedGap[] {
  return (n.delegated ?? []).filter((g) => Date.parse(g.until) > now.getTime());
}

const asWords = (g: DelegatedGap): string => (g.check === "approved" ? "approval" : g.name.replace(/-/g, " "));

/** The hint for one next stage, or null when the card should say nothing about it. */
export function nextStageHint(n: NextStage, now: Date = new Date()): CardHint | null {
  if (n.immediate === false) return null;
  const signoffs = n.signoffs ?? [];
  const held = liveDelegated(n, now);
  // A delegation that ended since the read: its gaps are the person's again.
  const lapsed = (n.delegated?.length ?? 0) - held.length;
  const yours = (n.for_you ?? 0) + lapsed;
  const rest = n.missing - yours - held.length;
  if (n.missing > 0) {
    if (rest > 0) {
      const mine = yours > 0 ? `, ${yours} yours` : "";
      const ids = [...new Set(held.map((g) => g.delegation))].join(" and ");
      const claudes = held.length > 0 ? `, and ${held.length} Claude holds under ${ids}` : "";
      return {
        text: `${n.status} needs ${n.missing}${mine}`,
        title:
          yours > 0 || held.length > 0
            ? `${n.status} needs ${n.missing} things: ${rest} for Claude to do, ${yours} for you (a sign-off or approval)${claudes}`
            : `${n.status} needs ${n.missing} thing(s) done by Claude`,
        kind: "needs",
      };
    }
    // Only held gaps are left: the delegated hint speaks.
    if (yours === 0) return null;
    // Only the user's part remains.
    if ((n.tasks_open ?? 0) > 0) return null;
    const heldNames = new Set(held.filter((g) => g.check === "signoff").map((g) => g.name));
    const mine = signoffs.filter((s) => !heldNames.has(s));
    const names = [...mine, ...(yours > mine.length ? ["approval"] : [])];
    return {
      text: `waiting on you: ${names.join(", ")}`,
      title: `${n.status} waits on you: ${names.join(", ")}`,
      kind: "waiting",
    };
  }
  if ((n.tasks_open ?? 0) > 0) return null;
  return {
    text: n.ends ? "ready to complete" : `ready for ${n.status}`,
    title: `nothing stands between this item and ${n.status}`,
    kind: "ready",
  };
}

/** What Claude holds on a card, as a link to the delegation (DEF-104). */
export interface DelegatedHint {
  delegation: string;
  text: string;
  title: string;
  kind: "delegated";
}

/** One hint per delegation that holds a gap of the immediate next stage, in first-seen order. */
export function delegatedHints(n: NextStage, now: Date = new Date()): DelegatedHint[] {
  if (n.immediate === false) return [];
  const byId = new Map<string, DelegatedGap[]>();
  for (const g of liveDelegated(n, now)) byId.set(g.delegation, [...(byId.get(g.delegation) ?? []), g]);
  return [...byId].map(([delegation, gaps]) => ({
    delegation,
    text: `${[...new Set(gaps.map(asWords))].join(", ")}: Claude, under ${delegation}`,
    title: `Claude may take these under ${delegation} until ${dayMonth(gaps[0].until)}; you can still take them on the Review page`,
    kind: "delegated" as const,
  }));
}
