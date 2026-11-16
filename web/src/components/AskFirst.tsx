import { useEffect, useState } from "react";

/**
 * A button that asks before it acts (DEF-034): the first press replaces it with the question
 * `ask`, a button of the same `label` and Cancel, inline (no form, no browser dialog); the second
 * press calls `onConfirm`; Cancel returns to the button. A change of `label` or `ask` while the
 * question shows drops it, so a press never confirms a question the page no longer asks. Shared by
 * the Releases page's direct promotions and the group actions that reuse it (FTR-038, FTR-049).
 */
export function AskFirst(p: {
  label: string;
  ask: string;
  onConfirm(): void | Promise<unknown>;
  disabled?: boolean;
  disabledWhy?: string;
  /** The button's class; "btn" when not given. */
  className?: string;
}): JSX.Element {
  const [armed, setArmed] = useState(false);
  useEffect(() => setArmed(false), [p.label, p.ask]);
  const className = p.className ?? "btn";
  const title = p.disabled ? p.disabledWhy : undefined;
  if (!armed) {
    return (
      <button className={className} onClick={() => setArmed(true)} disabled={p.disabled} title={title}>
        {p.label}
      </button>
    );
  }
  return (
    <span className="ask-first" role="group" aria-label={p.ask}>
      <span className="ask-first-question">{p.ask}</span>{" "}
      <button
        className={className}
        disabled={p.disabled}
        title={title}
        onClick={() => {
          setArmed(false);
          void p.onConfirm();
        }}
      >
        {p.label}
      </button>{" "}
      <button className="btn" onClick={() => setArmed(false)}>
        Cancel
      </button>
    </span>
  );
}
