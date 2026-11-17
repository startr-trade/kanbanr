/**
 * The three states a page shows instead of its data (FTR-046, R-6): loading, an error, and empty.
 * They never render together; a page picks one from `useQuery`'s `loading` and `error` (an empty
 * state only when `loading === false && error === null`), so loading is never drawn as empty.
 */
import type { ReactNode } from "react";
import { CopyText } from "./CopyText.tsx";

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
