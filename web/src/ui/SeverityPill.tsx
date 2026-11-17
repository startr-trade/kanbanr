/**
 * A severity pill (FTR-046, R-4): a short label in its tone's colours. Tone is never carried by
 * colour alone: when the pill's own text does not say its tone, a visually hidden `TONE_WORD` is
 * read first ("error DEF-066 blocked").
 */
import { Children, isValidElement, type ReactNode } from "react";
import { TONE_WORD, type Tone } from "./tone.ts";

export type { Tone } from "./tone.ts";

/** The plain text of a React node, as a screen reader would read it. */
export function textOf(node: ReactNode): string {
  let out = "";
  Children.forEach(node, (child) => {
    if (typeof child === "string" || typeof child === "number") out += String(child);
    else if (isValidElement<{ children?: ReactNode }>(child)) out += textOf(child.props.children);
  });
  return out;
}

/** Whether `text` already says `tone`'s word, so no hidden word is needed. */
export function saysTone(text: string, tone: Tone): boolean {
  const word = TONE_WORD[tone];
  return word === "" || text.toLowerCase().includes(word);
}

export function SeverityPill(p: { tone: Tone; children: ReactNode; title?: string }): JSX.Element {
  const hidden = saysTone(textOf(p.children), p.tone) ? null : <span className="ui-sr-only">{`${TONE_WORD[p.tone]} `}</span>;
  return (
    <span className={`ui-pill ui-pill-${p.tone}`} title={p.title}>
      {hidden}
      {p.children}
    </span>
  );
}
