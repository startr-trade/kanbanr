/**
 * A stage stepper (FTR-046, R-5; FTR-041's choice): dots joined by a line, one per stage, with the
 * date and who under each done step. The state is said in text ("done", "now", "next"), never by
 * the dot's colour alone, and the current step carries `aria-current="step"`.
 */
import type { Step } from "./stepper.ts";

export type { History, StageInput, Step } from "./stepper.ts";

/** The text each state is shown with. */
export const STEP_WORD: Record<Step["state"], string> = {
  done: "done",
  current: "now",
  next: "next",
  skipped: "skipped",
};

/**
 * `words` replaces the text of the states it names (FTR-078): the Board says "met", "to do" and
 * "not asked" for a stage's checks. A state it leaves out keeps its `STEP_WORD`.
 */
export function StageStepper(p: { stages: Step[]; label: string; words?: Partial<Record<Step["state"], string>> }): JSX.Element {
  return (
    <ol className="ui-stepper" aria-label={p.label}>
      {p.stages.map((s) => (
        <li key={s.key} className={`ui-step ui-step-${s.state}`} aria-current={s.state === "current" ? "step" : undefined}>
          <span className="ui-step-dot" aria-hidden="true" />
          <span className="ui-step-label">{s.label}</span>
          <span className="ui-step-state">{p.words?.[s.state] ?? STEP_WORD[s.state]}</span>
          {s.state === "done" && (s.at || s.by) && (
            <span className="ui-step-when">{[s.at ? s.at.slice(0, 10) : null, s.by || null].filter(Boolean).join(" · ")}</span>
          )}
        </li>
      ))}
    </ol>
  );
}
