/**
 * What a board card says about an item's next stage (FEAT-205), in plain words.
 *
 * The pill used to read "→ In Progress ✓", which looked like the item's status and claimed ready
 * while tasks or sign-offs were still outstanding. Now: what the next stage still needs, that it is
 * waiting for a sign-off, or that it is ready — and nothing at all while the item's tasks are open,
 * since the tasks chip already says so and "ready" would not be true.
 */
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
}

export interface CardHint {
  text: string;
  title: string;
  /** "needs" and "waiting" are things to do; "ready" is not. */
  kind: "needs" | "waiting" | "ready";
}

/** The hint for one next stage, or null when the card should say nothing about it. */
export function nextStageHint(n: NextStage): CardHint | null {
  if (n.immediate === false) return null;
  const signoffs = n.signoffs ?? [];
  const yours = n.for_you ?? 0;
  if (n.missing > 0) {
    if (yours === 0 || n.missing > yours) {
      const mine = yours > 0 ? `, ${yours} yours` : "";
      return {
        text: `${n.status} needs ${n.missing}${mine}`,
        title:
          yours > 0
            ? `${n.status} needs ${n.missing} things: ${n.missing - yours} for Claude to do, and ${yours} for you (a sign-off or approval)`
            : `${n.status} needs ${n.missing} thing(s) done by Claude`,
        kind: "needs",
      };
    }
    // Only the user's part remains.
    if ((n.tasks_open ?? 0) > 0) return null;
    const names = [...signoffs, ...(yours > signoffs.length ? ["approval"] : [])];
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
