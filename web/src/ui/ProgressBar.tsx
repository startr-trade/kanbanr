/**
 * A progress bar (FTR-046, R-4): segments of a total side by side. The bar is a picture named in
 * words ("Items: Done 12, In progress 3, of 20"), and a visible legend lists each non-zero segment
 * as "label value", so no part is told by colour alone. `to` makes the whole bar a link.
 */
import { Link } from "react-router-dom";
import { percentages, type Segment } from "./progress.ts";

export type { Segment } from "./progress.ts";

/** The bar's accessible name: every segment's label and value, then the total. */
export function progressLabel(label: string, segments: Segment[], total: number): string {
  return `${label}: ${segments.map((s) => `${s.label} ${s.value}`).join(", ")}, of ${total}`;
}

export function ProgressBar(p: { segments: Segment[]; total: number; label: string; to?: string }): JSX.Element {
  const widths = percentages(p.segments, p.total);
  const bar = (
    <>
      <div className="ui-progress-bar" role="img" aria-label={progressLabel(p.label, p.segments, p.total)}>
        {p.segments.map((s, i) =>
          widths[i] > 0 ? <span key={s.key} className={`ui-progress-seg ui-tone-${s.tone}`} style={{ width: `${widths[i]}%` }} /> : null,
        )}
      </div>
      <ul className="ui-progress-legend">
        {p.segments
          .filter((s) => s.value !== 0)
          .map((s) => (
            <li key={s.key}>
              <span className={`ui-progress-key ui-tone-${s.tone}`} aria-hidden="true" />
              {`${s.label} ${s.value}`}
            </li>
          ))}
      </ul>
    </>
  );
  return p.to ? (
    <Link className="ui-progress" to={p.to}>
      {bar}
    </Link>
  ) : (
    <div className="ui-progress">{bar}</div>
  );
}
