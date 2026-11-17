/**
 * A lane's groups as one stacked bar (FTR-078): the kit's bar markup and widths, with a legend
 * whose entries are buttons that show one group alone, and a table of the same counts.
 * The kit's `ProgressBar` draws a plain legend, so this draws the bar from the kit's pure parts.
 */
import { useState } from "react";
import { GROUP_ICON, groupId, laneSegments, oldestText } from "../../boardModel.ts";
import type { Lane } from "../../boardModel.ts";
import { progressLabel } from "../../ui/ProgressBar.tsx";
import { percentages } from "../../ui/progress.ts";

export function LaneBar(p: { lane: Lane; only: Set<string>; onFilter(id: string, on: boolean): void }): JSX.Element | null {
  const [table, setTable] = useState(false);
  const { lane } = p;
  const segments = laneSegments(lane);
  if (segments.length === 0) return null;
  const widths = percentages(segments, lane.total);
  const name = `${lane.status} by group`;
  return (
    <div className="board-lane-bar">
      <div className="ui-progress-bar" role="img" aria-label={progressLabel(name, segments, lane.total)}>
        {segments.map((s, k) => (widths[k] > 0 ? <span key={s.key} className={`ui-progress-seg ui-tone-${s.tone}`} style={{ width: `${widths[k]}%` }} /> : null))}
      </div>
      <ul className="board-legend">
        {lane.groups
          .filter((g) => g.items.length > 0)
          .map((g) => {
            const id = groupId(lane.status, g.key);
            const on = p.only.has(id);
            return (
              <li key={g.key}>
                <button type="button" aria-pressed={on} onClick={() => p.onFilter(id, !on)}>
                  <span className={`board-key ui-tone-${g.tone}`} aria-hidden="true" />
                  <span aria-hidden="true">{GROUP_ICON[g.key]}</span> {g.label} {g.items.length}
                </button>
              </li>
            );
          })}
      </ul>
      <button type="button" className="board-table-toggle" aria-pressed={table} onClick={() => setTable(!table)}>
        Show as table
      </button>
      {table && (
        <table className="board-table">
          <caption>{name}</caption>
          <thead>
            <tr>
              <th scope="col">Group</th>
              <th scope="col">Items</th>
              <th scope="col">Oldest (days)</th>
            </tr>
          </thead>
          <tbody>
            {lane.groups.map((g) => (
              <tr key={g.key} title={oldestText(g, lane.status) ?? undefined}>
                <th scope="row">{g.label}</th>
                <td>{g.items.length}</td>
                <td>{g.oldest?.daysIn ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
