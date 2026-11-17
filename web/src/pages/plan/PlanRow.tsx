/**
 * One row of the Plan page's Current or Planned tab (FTR-043): a sprint, a release or a milestone,
 * as an `AccordionRow` whose header carries its state and how much of it is done. Its items are
 * read only when it is opened (R-8), inside the row's `children`.
 */
import { AccordionRow } from "../../ui";
import type { PlanLane } from "../../types";
import { RowItems } from "./RowItems";

const KIND: Record<string, string> = { sprint: "Sprint", release: "Release", milestone: "Milestone" };

/** "3 of 6 days done": a sprint counts its estimate in the burndown's unit, the rest count items. */
export function doneText(lane: PlanLane): string {
  const unit = lane.burndown?.unit ? ` ${lane.burndown.unit}` : lane.kind === "sprint" ? "" : " items";
  return `${lane.done} of ${lane.total}${unit} done`;
}

export function PlanRow(p: {
  project: string;
  lane: PlanLane;
  /** The tab's window, so the row's items share its timeline's scale. */
  from: string;
  to: string;
  today: string;
  open: boolean;
  onToggle(open: boolean): void;
}): JSX.Element {
  const { lane } = p;
  return (
    <AccordionRow
      id={lane.key}
      level={2}
      open={p.open}
      onToggle={p.onToggle}
      title={
        <>
          <span className="plan-row-kind">{KIND[lane.kind] ?? lane.kind}</span> <code className="taskkey">{lane.label}</code>
          {lane.from ? (
            <span className="plan-row-dates">
              {" "}
              {lane.from === lane.to ? lane.from : `${lane.from} to ${lane.to}`}
            </span>
          ) : null}
        </>
      }
      meta={
        <>
          {lane.state ? <span className="ui-pill ui-pill-neutral">{lane.state}</span> : null}
          {lane.from ? null : <span className="ui-pill ui-pill-neutral">undated</span>}
          <span className="ui-pill ui-pill-info">{doneText(lane)}</span>
          {lane.carried > 0 ? <span className="ui-pill ui-pill-warn">{lane.carried} carried</span> : null}
        </>
      }
    >
      {() => <RowItems project={p.project} lane={lane} from={p.from} to={p.to} today={p.today} />}
    </AccordionRow>
  );
}
