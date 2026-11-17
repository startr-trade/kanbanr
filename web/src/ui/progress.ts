/**
 * The progress bar's model (FTR-046, R-4): each segment's share of a total, as whole percentages.
 * Pure, so the checks import it directly; `ProgressBar.tsx` draws the segments.
 */
import type { Tone } from "./tone.ts";

/** One part of a bar: `value` of the bar's total, drawn in `tone` and listed as `label value`. */
export interface Segment {
  key: string;
  label: string;
  value: number;
  tone: Tone;
}

/**
 * Each segment's share of `total` as a whole percentage, rounded by largest remainder so the shares
 * add up to the rounded whole they cover (100 when the segments fill the total): `[1,1,1]` of 3 gives
 * `[34,33,33]`. Segments are clamped in order to what is left of the total, so segments over the
 * total never draw past 100; a negative value counts as 0. A total of 0 (or less) gives all zeros.
 */
export function percentages(segments: Segment[], total: number): number[] {
  if (!(total > 0)) return segments.map(() => 0);
  let left = total;
  const raw = segments.map((s) => {
    const v = Math.min(Math.max(0, Number.isFinite(s.value) ? s.value : 0), left);
    left -= v;
    return (v / total) * 100;
  });
  const floors = raw.map(Math.floor);
  const want = Math.round(raw.reduce((a, b) => a + b, 0));
  let spare = want - floors.reduce((a, b) => a + b, 0);
  // The spare points go to the largest remainders; on a tie, the earlier segment.
  const order = raw.map((r, i) => ({ i, rem: r - floors[i] })).sort((a, b) => b.rem - a.rem || a.i - b.i);
  for (const { i, rem } of order) {
    if (spare <= 0) break;
    if (rem <= 0) continue;
    floors[i] += 1;
    spare -= 1;
  }
  return floors;
}
