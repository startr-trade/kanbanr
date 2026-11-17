/**
 * A stat tile (FTR-046, R-4): one number and what it counts, linking to the list behind it. Its
 * accessible name is "<value> <label>". A `warn` or `error` tone adds its word visibly before the
 * label, so the tone is never carried by colour alone. `spark` draws a small trend line that is
 * hidden from screen readers and adds nothing to the name.
 */
import { useId, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { TONE_WORD, type Tone } from "./tone.ts";

/** The polyline points of a sparkline in a 60 × 16 box; empty when there are fewer than two. */
export function sparkPoints(values: number[]): string {
  if (values.length < 2) return "";
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  const span = hi - lo || 1;
  return values.map((v, i) => `${((i / (values.length - 1)) * 60).toFixed(1)},${(15 - ((v - lo) / span) * 14).toFixed(1)}`).join(" ");
}

export function StatTile(p: { label: string; value: string | number; to?: string; sub?: ReactNode; tone?: Tone; spark?: number[] }): JSX.Element {
  const subId = useId();
  const tone = p.tone ?? "neutral";
  const word = tone === "warn" || tone === "error" ? TONE_WORD[tone] : "";
  const label = word ? `${word} ${p.label}` : p.label;
  const points = p.spark ? sparkPoints(p.spark) : "";
  const body = (
    <>
      <span className="ui-tile-value">{p.value}</span> <span className="ui-tile-label">{label}</span>
      {points && (
        <svg className="ui-tile-spark" viewBox="0 0 60 16" aria-hidden="true" focusable="false">
          <polyline points={points} fill="none" />
        </svg>
      )}
      {p.sub != null && (
        <span className="ui-tile-sub" id={subId}>
          {p.sub}
        </span>
      )}
    </>
  );
  const className = `ui-tile ui-tile-${tone}`;
  return p.to ? (
    <Link className={className} to={p.to} aria-label={`${p.value} ${label}`} aria-describedby={p.sub != null ? subId : undefined}>
      {body}
    </Link>
  ) : (
    <div className={className}>{body}</div>
  );
}
