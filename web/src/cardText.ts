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
}

export interface CardHint {
  text: string;
  title: string;
  /** "needs" and "waiting" are things to do; "ready" is not. */
  kind: "needs" | "waiting" | "ready";
}

/** The hint for one next stage, or null when the card should say nothing about it. */
export function nextStageHint(n: NextStage): CardHint | null {
  const signoffs = n.signoffs ?? [];
  const conditions = Math.max(0, n.missing - signoffs.length);
  if (conditions > 0) {
    return {
      text: `${n.status} needs ${conditions}`,
      title: `${conditions} thing(s) to settle before ${n.status}${signoffs.length ? `, and sign-off ${signoffs.join(", ")}` : ""}`,
      kind: "needs",
    };
  }
  if ((n.tasks_open ?? 0) > 0) return null;
  if (signoffs.length > 0) {
    return {
      text: `waiting for sign-off ${signoffs.join(", ")}`,
      title: `${n.status} needs the sign-off ${signoffs.join(", ")}`,
      kind: "waiting",
    };
  }
  return {
    text: n.ends ? "ready to complete" : `ready for ${n.status}`,
    title: `nothing stands between this item and ${n.status}`,
    kind: "ready",
  };
}
