/**
 * The three states a page shows instead of its data (FTR-046, R-6): loading, an error, and empty.
 * They never render together; a page picks one from `useQuery`'s `loading` and `error` (an empty
 * state only when `loading === false && error === null`), so loading is never drawn as empty.
 */
import { useEffect, useState, type ReactNode } from "react";
import { CopyText } from "./CopyText.tsx";
import { announcedSeconds, elapsedText, WORKING_AFTER_MS } from "./working.ts";

export { elapsedText, WORKING_AFTER_MS };

/** The time now, drawn again every `ms`. */
function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}

/**
 * A read that is still going: `since` is when it started (`useQuery`'s `since`), null when none is.
 * Nothing is drawn without one. For the first second only the label; then the label and whole
 * seconds, redrawn every second. A screen reader gets the seconds only at 10, 30 and 60 s and each
 * minute after, so it is not told every second.
 */
export function WorkingState(p: { label: string; since: number | null; after?: number }): JSX.Element | null {
  const now = useNow(1000);
  if (p.since === null) return null;
  if (now - p.since < (p.after ?? WORKING_AFTER_MS)) {
    return (
      <div className="ui-working" role="status">
        {p.label}
      </div>
    );
  }
  const secs = Math.floor((now - p.since) / 1000);
  const heard = announcedSeconds(secs);
  return (
    <div className="ui-working" role="status">
      {p.label}
      <span aria-hidden="true"> · {elapsedText(p.since, now)}</span>
      {heard > 0 && <span className="ui-sr-only"> {elapsedText(p.since, p.since + heard * 1000)}</span>}
    </div>
  );
}

/** Waiting for the first data: `role=status`, "Loading…". */
export function LoadingState(p: { label?: string }): JSX.Element {
  return (
    <div className="ui-loading" role="status">
      {p.label ?? "Loading…"}
    </div>
  );
}

/** A read failed: `role=alert` with the error, and "Try again" when the page can retry. */
export function ErrorState(p: { error: string; retry?(): void }): JSX.Element {
  return (
    <div className="ui-error" role="alert">
      <span className="ui-error-text">{p.error}</span>
      {p.retry && (
        <button type="button" className="btn ui-btn" onClick={() => p.retry?.()}>
          Try again
        </button>
      )}
    </div>
  );
}

/**
 * Nothing to show. `ask` is the "Ask Claude to …" sentence the person can copy, shown in quotes
 * with a Copy button, because the person talks to Claude rather than typing commands.
 */
export function Empty(p: { title: string; ask?: string; children?: ReactNode }): JSX.Element {
  return (
    <div className="ui-empty">
      <p className="ui-empty-title">{p.title}</p>
      {p.children}
      {p.ask && (
        <p className="ui-empty-ask">
          <q className="ui-empty-sentence">{p.ask}</q> <CopyText text={p.ask} label="Copy" />
        </p>
      )}
    </div>
  );
}
