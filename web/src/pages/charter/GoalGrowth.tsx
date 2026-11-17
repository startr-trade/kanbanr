/**
 * Progress per goal (FTR-076): one small step chart a goal of the requirements proven across the
 * releases shipped, every chart on one y scale so the panels compare. Read from today's evidence;
 * the caption says so. A goal with no points is named in one line, never drawn empty.
 */
import { GrowthChart } from "../../ui";
import { seriesFor, sharedMax } from "../../charterModel";
import type { CharterProgress } from "../../types";

/** "G-4 and G-7", "G-1, G-4 and G-7". */
function listOf(ids: string[]): string {
  return ids.length <= 1 ? ids.join("") : `${ids.slice(0, -1).join(", ")} and ${ids[ids.length - 1]}`;
}

export function GoalGrowth(p: { progress: CharterProgress; goalIds: string[] }): JSX.Element {
  const max = sharedMax(p.progress);
  const drawn = p.goalIds.filter((g) => seriesFor(p.progress, g).length > 0);
  const bare = p.goalIds.filter((g) => seriesFor(p.progress, g).length === 0);
  return (
    <>
      <p className="muted small">Requirements proven of items shipped by each release, as tested today.</p>
      {drawn.length > 0 && (
        <div className="charter-growth">
          {drawn.map((g) => (
            <GrowthChart
              key={g}
              stateKey={`gc-${g}-`}
              max={max}
              unit="requirements proven"
              caption={`${g}: requirements proven by release`}
              tableCaption={`${g} requirements proven by release`}
              points={seriesFor(p.progress, g).map((pt) => ({ label: pt.release, value: pt.req_proven, total: pt.req_total }))}
            />
          ))}
        </div>
      )}
      {bare.length > 0 && (
        <p className="muted">
          {listOf(bare)} {bare.length === 1 ? "has" : "have"} nothing shipped yet.
        </p>
      )}
    </>
  );
}
