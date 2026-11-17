/**
 * A growth chart (FTR-076, R-5): one series as a step line with a marker at every point, a faint
 * track for the total it is part of, and one y axis from 0 to `max`. The SVG is the picture
 * (`role="img"`, named by the caption); a visually hidden table holds every point, and "Show as
 * table" swaps the picture for a real, sortable one. Accent hue only: the line, markers and track
 * carry no meaning by colour, the numbers are in the labels, the titles and the table.
 */
import { useState } from "react";
import { DataTable } from "./DataTable.tsx";
import { growthPath, pointText, pointX, pointY, type GrowthPoint } from "./growth.ts";
import type { Column } from "./table.ts";

export type { GrowthPoint } from "./growth.ts";

/** The drawing: 320 by 120, with room left of the plot for the axis and right of it for the last label. */
const WIDTH = 320;
const HEIGHT = 120;
const LEFT = 30;
const RIGHT = 46;
const TOP = 10;
const BOTTOM = 22;
const PLOT_W = WIDTH - LEFT - RIGHT;
const PLOT_H = HEIGHT - TOP - BOTTOM;
/** Marker radius: a 9 px dot, over the 8 px minimum. */
const MARKER = 4.5;

export function GrowthChart(p: {
  points: GrowthPoint[];
  max: number;
  caption: string;
  unit: string;
  tableCaption: string;
  /** Prefix of the table view's URL keys, so two charts on one page keep separate state. */
  stateKey?: string;
  /** Start as the table (for a check; the person presses the button). */
  asTable?: boolean;
}): JSX.Element {
  const [table, setTable] = useState(p.asTable ?? false);
  const hasTotal = p.points.some((pt) => pt.total !== undefined);
  const n = p.points.length;
  const { line, track } = growthPath(p.points, p.max, PLOT_W, PLOT_H);
  const grid = [0, 0.5, 1].map((f) => ({ f, y: TOP + pointY(f * p.max, p.max, PLOT_H) }));
  const last = p.points[n - 1];

  const columns: Column<GrowthPoint>[] = [
    { key: "label", label: "Point", value: (r) => r.label },
    { key: "value", label: p.unit, value: (r) => r.value, align: "end" },
    ...(hasTotal ? [{ key: "total", label: "Of", value: (r: GrowthPoint) => r.total ?? null, align: "end" as const }] : []),
  ];

  return (
    <figure className="ui-growth">
      {table ? (
        <DataTable
          rows={p.points}
          columns={columns}
          rowKey={(r) => r.label}
          stateKey={p.stateKey ?? "gc"}
          csvName="growth"
          empty="No points yet."
          caption={p.tableCaption}
        />
      ) : (
        <>
          <svg role="img" aria-label={p.caption} viewBox={`0 0 ${WIDTH} ${HEIGHT}`} width="100%" preserveAspectRatio="xMinYMin meet">
            <g className="ui-growth-grid">
              {grid.map((g) => (
                <g key={g.f}>
                  <line x1={LEFT} x2={LEFT + PLOT_W} y1={g.y} y2={g.y} />
                  {g.f !== 0.5 && (
                    <text x={LEFT - 4} y={g.y} textAnchor="end" dominantBaseline="middle">
                      {g.f === 0 ? 0 : p.max}
                    </text>
                  )}
                </g>
              ))}
            </g>
            <g transform={`translate(${LEFT} ${TOP})`}>
              {track && <path className="ui-growth-track" d={track} fill="none" />}
              {line && <path className="ui-growth-line" d={line} fill="none" strokeWidth={2} />}
              {p.points.map((pt, i) => (
                <g key={pt.label} className="ui-growth-point">
                  <title>{pointText(pt, p.unit)}</title>
                  <circle cx={pointX(i, n, PLOT_W)} cy={pointY(pt.value, p.max, PLOT_H)} r={MARKER} />
                </g>
              ))}
              {last && (
                <text className="ui-growth-last" x={pointX(n - 1, n, PLOT_W) + MARKER + 3} y={pointY(last.value, p.max, PLOT_H)} dominantBaseline="middle">
                  {last.value}
                </text>
              )}
            </g>
            <g className="ui-growth-x">
              {p.points.map((pt, i) =>
                i === 0 || i === n - 1 ? (
                  <text key={pt.label} x={LEFT + pointX(i, n, PLOT_W)} y={HEIGHT - 6} textAnchor={i === 0 && n > 1 ? "start" : n === 1 ? "middle" : "end"}>
                    {pt.label}
                  </text>
                ) : null,
              )}
            </g>
          </svg>
          <table className="ui-sr-only">
            <caption>{p.tableCaption}</caption>
            <thead>
              <tr>
                <th scope="col">Point</th>
                <th scope="col">{p.unit}</th>
                {hasTotal && <th scope="col">Of</th>}
              </tr>
            </thead>
            <tbody>
              {p.points.map((pt) => (
                <tr key={pt.label}>
                  <th scope="row">{pt.label}</th>
                  <td>{pt.value}</td>
                  {hasTotal && <td>{pt.total ?? ""}</td>}
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      <figcaption className="ui-growth-caption">{p.caption}</figcaption>
      <button type="button" className="ui-growth-toggle" aria-pressed={table} onClick={() => setTable((t) => !t)}>
        {table ? "Show as chart" : "Show as table"}
      </button>
    </figure>
  );
}
