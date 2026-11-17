/**
 * The stage stepper's model (FTR-046, R-5): which stages are done, which is current and which come
 * next. Pure, so the checks import it directly; `StageStepper.tsx` draws the steps.
 */

/** A stage as the release cadence names it. */
export interface StageInput {
  key: string;
  label: string;
}

/** A recorded move into a stage: when, and by whom. */
export interface History {
  stage: string;
  at: string;
  by: string;
}

/** One drawn step. A done step carries when it was reached and by whom, when that is recorded. */
export interface Step {
  key: string;
  label: string;
  state: "done" | "current" | "next" | "skipped";
  /** Words for this step alone, instead of its state's (DEF-104: "Claude holds"). */
  word?: string;
  at?: string;
  by?: string;
}

/** The latest recorded move into `stage`, if any. */
function reached(history: History[], stage: string): History | undefined {
  let latest: History | undefined;
  for (const h of history) {
    if (h.stage === stage && (latest === undefined || h.at >= latest.at)) latest = h;
  }
  return latest;
}

/**
 * The steps for `stages` in order: every stage before `current` is done (with its date and who from
 * `history`, when present), `current` is current, and the rest are next. With `shipped`, every stage
 * is done. A `current` that is `null`, or names no stage, leaves every stage next.
 */
export function stepsFromStages(stages: StageInput[], current: string | null, history: History[], shipped: boolean): Step[] {
  const at = shipped ? stages.length : current === null ? -1 : stages.findIndex((s) => s.key === current);
  return stages.map((s, i): Step => {
    if (at >= 0 && i < at) {
      const h = reached(history, s.key);
      return h ? { key: s.key, label: s.label, state: "done", at: h.at, by: h.by } : { key: s.key, label: s.label, state: "done" };
    }
    if (i === at) return { key: s.key, label: s.label, state: "current" };
    return { key: s.key, label: s.label, state: "next" };
  });
}
