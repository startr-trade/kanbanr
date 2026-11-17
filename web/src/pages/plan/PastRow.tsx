/**
 * One row of the Plan page's Past tab (FTR-043, R-5): compact, on one line, planned against
 * delivered and carried over: "SP-003 · 14–20 Nov · 9 of 11 delivered · 2 carried". It opens to
 * its items, read only then (R-8); the body names where each carried item went.
 */
import { AccordionRow } from "../../ui";
import type { PlanLane } from "../../types";
import { RowItems } from "./RowItems";

const MONTH = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `YYYY-MM-DD` as [year, month index, day]; null when it is not one. */
function parts(iso: string): [number, number, number] | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return m ? [Number(m[1]), Number(m[2]) - 1, Number(m[3])] : null;
}

/** "14–20 Nov", "28 Oct – 3 Nov", "13 Nov", or "28 Dec 2026 – 3 Jan 2027" across a year. */
export function dayRange(from: string, to: string): string {
  const a = parts(from);
  const b = parts(to || from);
  if (!a || !b) return "undated";
  const [ay, am, ad] = a;
  const [by, bm, bd] = b;
  if (ay !== by) return `${ad} ${MONTH[am]} ${ay} – ${bd} ${MONTH[bm]} ${by}`;
  if (am !== bm) return `${ad} ${MONTH[am]} – ${bd} ${MONTH[bm]}`;
  if (ad !== bd) return `${ad}–${bd} ${MONTH[am]}`;
  return `${ad} ${MONTH[am]}`;
}

/** The row's one line. */
export function pastLine(lane: PlanLane): string {
  const carried = lane.carried > 0 ? ` · ${lane.carried} carried` : "";
  return `${lane.label} · ${dayRange(lane.from, lane.to)} · ${lane.done} of ${lane.total} delivered${carried}`;
}

export function PastRow(p: {
  project: string;
  lane: PlanLane;
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
      title={<span className="plan-past-line">{pastLine(lane)}</span>}
      meta={lane.state ? <span className="ui-pill ui-pill-neutral">{lane.state}</span> : undefined}
    >
      {() => <RowItems project={p.project} lane={lane} from={p.from} to={p.to} today={p.today} />}
    </AccordionRow>
  );
}
