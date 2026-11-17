/**
 * The growth chart's model (FTR-076, R-5): a step line of one series over a few ordered points, on
 * one y axis from 0 to `max`. Pure, so the checks import it directly; `GrowthChart.tsx` draws it.
 */

/** One point: its label (a release or a cycle), its value and, optionally, the total it is part of. */
export interface GrowthPoint {
  label: string;
  value: number;
  total?: number;
}

/** The paths of a `w` by `h` plot area, and where each point's label goes. */
export interface GrowthPaths {
  /** The step line of `value`; "" for no points. */
  line: string;
  /** The faint step line of `total`; "" when no point has one. */
  track: string;
  ticks: { x: number; label: string }[];
}

const num = (n: number): string => String(Math.round(n * 100) / 100);

/** A point's x in a plot `w` wide: one point sits in the middle, the rest are spread edge to edge. */
export function pointX(i: number, n: number, w: number): number {
  return n <= 1 ? w / 2 : (i * w) / (n - 1);
}

/** A value's y in a plot `h` tall, 0 at the bottom and `max` at the top; clamped to the plot. */
export function pointY(v: number, max: number, h: number): number {
  const m = max > 0 ? max : 1;
  const f = Math.min(Math.max(Number.isFinite(v) ? v : 0, 0), m) / m;
  return h - f * h;
}

/** A step path through `(x, y)` points: along, then up or down, so a value holds until the next point. */
function step(xy: [number, number][]): string {
  return xy.map(([x, y], i) => (i === 0 ? `M${num(x)} ${num(y)}` : `H${num(x)} V${num(y)}`)).join(" ");
}

/** The step line, the faint track of the totals and the label positions, in a `w` by `h` plot area. */
export function growthPath(points: GrowthPoint[], max: number, w: number, h: number): GrowthPaths {
  const n = points.length;
  const line = step(points.map((p, i) => [pointX(i, n, w), pointY(p.value, max, h)]));
  const totals = points.map((p, i): [number, number] | null => (p.total === undefined ? null : [pointX(i, n, w), pointY(p.total, max, h)]));
  const track = totals.some((t) => t === null) ? "" : step(totals as [number, number][]);
  return { line, track, ticks: points.map((p, i) => ({ x: pointX(i, n, w), label: p.label })) };
}

/** A point's hover and table text: "v0.7.0: 41 of 52 requirements proven". */
export function pointText(p: GrowthPoint, unit: string): string {
  return p.total === undefined ? `${p.label}: ${p.value} ${unit}` : `${p.label}: ${p.value} of ${p.total} ${unit}`;
}
