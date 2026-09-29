import type { SprintReport } from "../types";

/**
 * A sprint's burndown (FEAT-123): what was still to do at the end of each day, against the
 * straight line from everything committed to nothing by the last day.
 *
 * One series, so no legend; the heading names it. The line is 2px in the accent colour (checked
 * against both surfaces), the ideal line is a dashed reference in muted ink, and every day has a
 * hover target with its value. A table gives the same numbers without the picture.
 */
export default function Burndown({ report }: { report: SprintReport }) {
  const days = report.burndown;
  const total = Math.max(1, spanDays(report.start, report.end));
  const top = Math.max(report.committed, 1);
  const W = 600;
  const H = 140;
  const pad = { l: 36, r: 12, t: 12, b: 22 };
  const x = (i: number) => pad.l + (i / Math.max(total - 1, 1)) * (W - pad.l - pad.r);
  const y = (v: number) => pad.t + (1 - v / top) * (H - pad.t - pad.b);
  const points = days.map((d, i) => `${x(i)},${y(d.remaining)}`).join(" ");
  const unit = report.unit;

  return (
    <figure className="burndown">
      <figcaption className="muted small">Burndown — {unit} remaining</figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Burndown of ${report.code}`}>
        {/* Recessive frame: a baseline and the committed level, labelled at the axis. */}
        <line x1={pad.l} x2={W - pad.r} y1={y(0)} y2={y(0)} className="bd-axis" />
        <text x={pad.l - 6} y={y(top) + 4} className="bd-label" textAnchor="end">
          {report.committed}
        </text>
        <text x={pad.l - 6} y={y(0) + 4} className="bd-label" textAnchor="end">
          0
        </text>
        <text x={pad.l} y={H - 6} className="bd-label">
          {report.start}
        </text>
        <text x={W - pad.r} y={H - 6} className="bd-label" textAnchor="end">
          {report.end}
        </text>
        {/* The ideal: all of it, gone by the last day. A reference, not a series. */}
        <line x1={x(0)} y1={y(top)} x2={x(total - 1)} y2={y(0)} className="bd-ideal" />
        {days.length > 1 ? <polyline points={points} className="bd-line" /> : null}
        {days.map((d, i) => (
          <g key={d.date}>
            <circle cx={x(i)} cy={y(d.remaining)} r={4} className="bd-dot" />
            {/* A hit target bigger than the mark. */}
            <circle cx={x(i)} cy={y(d.remaining)} r={10} className="bd-hit">
              <title>
                {d.date}: {d.remaining} {unit} remaining
              </title>
            </circle>
          </g>
        ))}
      </svg>
      <details className="muted small">
        <summary>As a table</summary>
        <table className="bd-table">
          <thead>
            <tr>
              <th>Day</th>
              <th>Remaining ({unit})</th>
            </tr>
          </thead>
          <tbody>
            {days.map((d) => (
              <tr key={d.date}>
                <td>{d.date}</td>
                <td>{d.remaining}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}

/** Days from start to end, both included. */
function spanDays(start: string, end: string): number {
  const a = Date.parse(start);
  const b = Date.parse(end);
  if (Number.isNaN(a) || Number.isNaN(b)) return 1;
  return Math.round((b - a) / 86_400_000) + 1;
}
