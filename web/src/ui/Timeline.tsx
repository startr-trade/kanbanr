/**
 * A timeline (FTR-046, R-5; FTR-043's choices): lanes as rows, bands 24 px tall with their label
 * beside them, marks as diamonds or dots, and today as a solid 2 px accent line labelled "today".
 * A band's `progress` draws its done part filled and the rest faint; a pending band is dashed and
 * labelled "pending"; a band with `to_url` is a link.
 *
 * The SVG is the picture (`role="img"`, named by the caption). A visually hidden list beside it
 * gives every band and mark with its dates, so the data is never in the picture alone.
 */
import { Link } from "react-router-dom";
import { bandText, clampWindow, dayOf, markText, percentDone, scale, type Band, type Lane, type Mark } from "./timeline.ts";

export type { Band, Lane, Mark } from "./timeline.ts";

/** The drawing's coordinate system: the lane labels on the left, then the window. */
const WIDTH = 960;
const LABEL_W = 160;
const PAD = 16;
const TOP = 20;
const LANE_H = 36;
const BAND_H = 24;
/** The fewest days drawn around today. */
const MIN_DAYS = 14;

export function Timeline(p: { from: string; to: string; today?: string; lanes: Lane[]; caption: string }): JSX.Element {
  const win = p.today ? clampWindow(p.from, p.to, p.today, MIN_DAYS) : { from: p.from, to: p.to };
  const span = WIDTH - LABEL_W - PAD;
  const x = (at: string) => LABEL_W + scale(win.from, win.to, at) * span;
  const height = TOP + p.lanes.length * LANE_H + 4;
  // Today is drawn only inside the window: `scale` clamps, so a date outside would sit on an edge.
  const t = p.today ? dayOf(p.today) : NaN;
  const todayX = p.today && t >= dayOf(win.from) && t <= dayOf(win.to) ? x(p.today) : null;

  const band = (b: Band, i: number, y: number) => {
    const x1 = x(b.from);
    const x2 = Math.max(x(b.to), x1 + 2);
    const w = x2 - x1;
    const tone = b.tone ?? "info";
    const pct = percentDone(b);
    const text = b.pending ? `${b.label} · pending` : b.label;
    const after = x2 + 6 < WIDTH - 80;
    const drawn = (
      <g className={`ui-tl-band ui-tl-${tone}${b.pending ? " ui-tl-pending" : ""}`}>
        <title>{bandText(b)}</title>
        {pct === null ? (
          <rect x={x1} y={y} width={w} height={BAND_H} rx={4} strokeDasharray={b.pending ? "4 3" : undefined} />
        ) : (
          <>
            <rect className="ui-tl-rest" x={x1} y={y} width={w} height={BAND_H} rx={4} strokeDasharray={b.pending ? "4 3" : undefined} />
            <rect className="ui-tl-done" x={x1} y={y} width={(w * pct) / 100} height={BAND_H} rx={4} />
          </>
        )}
        <text className="ui-tl-label" x={after ? x2 + 6 : x1 - 6} y={y + BAND_H / 2} dominantBaseline="middle" textAnchor={after ? "start" : "end"}>
          {text}
        </text>
      </g>
    );
    return b.to_url ? (
      <Link key={`b${i}`} to={b.to_url}>
        {drawn}
      </Link>
    ) : (
      <g key={`b${i}`}>{drawn}</g>
    );
  };

  /** Marks whose x fall within this many px draw as one (a day at the window's scale). */
  const MERGE_PX = 4;
  /** An estimate of one label character's width, to see whether two labels would overprint. */
  const CHAR_PX = 6.5;
  /** Group a lane's marks that sit on one spot; draw each group once, dropping a label that would overprint. */
  const marks = (lane: Lane, y: number) => {
    const sorted = lane.marks.map((m) => ({ m, mx: x(m.at) })).sort((a, b) => a.mx - b.mx);
    const groups: { mx: number; members: Mark[] }[] = [];
    for (const { m, mx } of sorted) {
      const last = groups[groups.length - 1];
      if (last && mx - last.mx <= MERGE_PX) last.members.push(m);
      else groups.push({ mx, members: [m] });
    }
    let labelEnd = -Infinity;
    return groups.map((g, i) => {
      const my = y + BAND_H / 2;
      const first = g.members[0]!;
      const n = g.members.length;
      const diamond = g.members.some((m) => m.kind === "diamond");
      const label = n > 1 ? `${first.label} +${n - 1}` : first.label;
      const start = g.mx + 9;
      const showLabel = start >= labelEnd;
      if (showLabel) labelEnd = start + label.length * CHAR_PX + 4;
      return (
        <g key={`m${i}`} className={`ui-tl-mark ui-tl-${diamond ? "diamond" : "dot"}`}>
          <title>{n > 1 ? g.members.map(markText).join("\n") : markText(first)}</title>
          {diamond ? (
            <polygon points={`${g.mx},${my - 7} ${g.mx + 7},${my} ${g.mx},${my + 7} ${g.mx - 7},${my}`} />
          ) : (
            <circle cx={g.mx} cy={my} r={5} />
          )}
          {showLabel && (
            <text className="ui-tl-label" x={start} y={my} dominantBaseline="middle">
              {label}
            </text>
          )}
        </g>
      );
    });
  };

  return (
    <div className="ui-timeline">
      <svg role="img" aria-label={p.caption} viewBox={`0 0 ${WIDTH} ${height}`} width="100%" preserveAspectRatio="xMinYMin meet">
        {p.lanes.map((lane, li) => {
          const y = TOP + li * LANE_H + (LANE_H - BAND_H) / 2;
          return (
            <g key={lane.key} className="ui-tl-lane">
              <text className="ui-tl-lane-label" x={0} y={y + BAND_H / 2} dominantBaseline="middle">
                {lane.label}
              </text>
              {lane.bands.map((b, i) => band(b, i, y))}
              {marks(lane, y)}
            </g>
          );
        })}
        {todayX !== null && (
          <g className="ui-tl-today">
            <line x1={todayX} x2={todayX} y1={TOP - 4} y2={height} strokeWidth={2} />
            <text x={todayX} y={TOP - 8} textAnchor="middle">
              today
            </text>
          </g>
        )}
      </svg>
      <ul className="ui-sr-only">
        {p.lanes.map((lane) => (
          <li key={lane.key}>
            {lane.label}
            <ul>
              {lane.bands.map((b, i) => (
                <li key={`b${i}`}>{b.to_url ? <Link to={b.to_url}>{bandText(b)}</Link> : bandText(b)}</li>
              ))}
              {lane.marks.map((m, i) => (
                <li key={`m${i}`}>{markText(m)}</li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  );
}
