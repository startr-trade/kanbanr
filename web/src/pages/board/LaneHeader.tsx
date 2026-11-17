/**
 * What a lane says about itself (FTR-078): its bar, its flow over the last 7 days and at most one
 * thought-provoking line. Nothing is drawn for a quiet lane that has nothing to say.
 */
import { flowText, groupId } from "../../boardModel.ts";
import type { Lane } from "../../boardModel.ts";
import { SeverityPill } from "../../ui/SeverityPill.tsx";
import { LaneBar } from "./LaneBar.tsx";

const PILL: Record<string, { tone: "warn" | "info"; word: string }> = {
  idle: { tone: "warn", word: "Stalled" },
  wip: { tone: "info", word: "Heavy" },
  filling: { tone: "info", word: "Filling" },
  ready: { tone: "info", word: "Ready" },
  pace: { tone: "info", word: "Pace" },
};

export function LaneHeader(p: { lane: Lane; only: Set<string>; onFilter(id: string, on: boolean): void }): JSX.Element {
  const { lane } = p;
  const flow = flowText(lane);
  const ins = lane.insight;
  const pill = ins ? PILL[ins.rule] : null;
  const target = ins?.group ? groupId(lane.status, ins.group) : null;
  return (
    <div className="board-lane-head">
      {lane.kind !== "finished" && <LaneBar lane={lane} only={p.only} onFilter={p.onFilter} />}
      {flow && <p className="board-flow">{flow}</p>}
      {ins && pill && (
        <p className="board-insight">
          <SeverityPill tone={pill.tone}>{pill.word}</SeverityPill> {ins.text}
          {target && (
            <>
              {" "}
              <button type="button" className="board-link-button" aria-pressed={p.only.has(target)} onClick={() => p.onFilter(target, !p.only.has(target))}>
                Show them
              </button>
            </>
          )}
        </p>
      )}
    </div>
  );
}
