/**
 * An accordion row (FTR-046, R-2): one line (R-11) whose header is a real heading holding the
 * toggle button, with properties (`meta`) and `actions` on the same line. The body is rendered only
 * while the row is open — `children` is a function, so a page fetches the body inside it.
 *
 * Controlled with `open` and `onToggle` (a page usually passes `useOpenRows()`'s state, so open
 * rows live in the URL hash); without `open` the row keeps its own state, starting closed.
 */
import { useState, type ReactNode } from "react";

export function AccordionRow(p: {
  /** The row's id: the section's `id` (an anchor) and the key `useOpenRows` keeps. */
  id: string;
  /** The heading level of the header; default 3. */
  level?: 2 | 3 | 4;
  title: ReactNode;
  /** Properties on the header line, drawn as `SeverityPill`s or `.ui-pill` chips. */
  meta?: ReactNode;
  actions?: ReactNode;
  open?: boolean;
  onToggle?(open: boolean): void;
  /** A checkbox before the heading, labelled by the title. `disabled` is the reason, shown as text. */
  select?: { checked: boolean; disabled?: string; onChange(v: boolean): void };
  children: () => ReactNode;
}): JSX.Element {
  const [own, setOwn] = useState(false);
  const open = p.open ?? own;
  const level = p.level ?? 3;
  const Heading = `h${level}` as "h2" | "h3" | "h4";
  const headId = `${p.id}-head`;
  const bodyId = `${p.id}-body`;
  const reasonId = `${p.id}-reason`;

  const toggle = () => {
    const next = !open;
    if (p.open === undefined) setOwn(next);
    p.onToggle?.(next);
  };

  return (
    <section className="ui-row" id={p.id}>
      <div className="ui-row-head">
        {p.select && (
          <span className="ui-row-select">
            <input
              type="checkbox"
              checked={p.select.checked}
              disabled={p.select.disabled ? true : undefined}
              aria-labelledby={headId}
              aria-describedby={p.select.disabled ? reasonId : undefined}
              onChange={(e) => p.select?.onChange(e.currentTarget.checked)}
            />
            {p.select.disabled && (
              <span className="ui-row-reason" id={reasonId}>
                {p.select.disabled}
              </span>
            )}
          </span>
        )}
        <Heading className="ui-row-title">
          <button type="button" id={headId} aria-expanded={open} aria-controls={bodyId} onClick={toggle}>
            {p.title}
          </button>
        </Heading>
        {p.meta && <span className="ui-row-meta">{p.meta}</span>}
        {p.actions && <span className="ui-row-actions">{p.actions}</span>}
      </div>
      <div className="ui-row-body" id={bodyId} hidden={!open}>
        {open ? p.children() : null}
      </div>
    </section>
  );
}
