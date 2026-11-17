/**
 * A copy affordance (FTR-046): the "Ask Claude" sentence or a terminal line, copied with one click.
 * It writes through `navigator.clipboard` and announces "Copied" in a live region. Where the
 * clipboard is unavailable (a non-secure origin, or the write is refused), it shows the text in a
 * read-only input and selects it, so the person can copy it themselves.
 */
import { useEffect, useRef, useState } from "react";

export function CopyText(p: { text: string; label: string }): JSX.Element {
  const [status, setStatus] = useState("");
  const [manual, setManual] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (manual) input.current?.select();
  }, [manual]);

  const fallBack = () => {
    setManual(true);
    setStatus("Selected: copy it with your keyboard");
    input.current?.select();
  };

  const copy = () => {
    const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
    if (!clip?.writeText || (typeof window !== "undefined" && window.isSecureContext === false)) {
      fallBack();
      return;
    }
    clip.writeText(p.text).then(() => setStatus("Copied"), fallBack);
  };

  return (
    <span className="ui-copy">
      <button type="button" className="btn ui-btn" onClick={copy}>
        {p.label}
      </button>
      {manual && <input ref={input} className="ui-copy-text" readOnly value={p.text} aria-label={p.label} onFocus={(e) => e.currentTarget.select()} />}
      <span className="ui-copy-status" aria-live="polite">
        {status}
      </span>
    </span>
  );
}
