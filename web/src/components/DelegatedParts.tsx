// What Needs you says while a delegation is in force (DEF-104): what Claude decided since the
// person last looked, what each delegation still holds, and the lines for a delegation that ends
// soon. Drawn by the drawer and by the Overview from the same `NeedsYouParts`. Nothing here is
// styled as a warning except an ending line, which asks the person to look; the decided and held
// parts are information, so each line has words and an icon and no tone but info or neutral.
import { useEffect, useMemo } from "react";
import { Link } from "react-router-dom";
import { SeverityPill } from "../ui/SeverityPill.tsx";
import { SEEN_AFTER_MS, decidedParts, seenKey, type EndingLine, type NeedsYouParts } from "../needsYouState.ts";
import { to } from "../overviewLinks.ts";
import type { DelegatedDecision } from "../types";

const ENDING_WORD: Record<EndingLine["state"], string> = { granted: "ends soon", ended: "ended", revoked: "revoked", proposed: "ends soon" };

/** The delegations that end soon or just ended, with what returns to the person. */
export function EndingLines(p: { lines: EndingLine[]; onFollow?(): void }): JSX.Element | null {
  if (p.lines.length === 0) return null;
  return (
    <ul className="needs-you-lines needs-you-ending">
      {p.lines.map((l) => (
        <li key={l.delegation} className="needs-you-line">
          <Link className="needs-you-link" to={l.to} onClick={p.onFollow}>
            <span aria-hidden="true">{"⏳"}</span> <span className="needs-you-label">{l.text}</span>
          </Link>
          <SeverityPill tone="warn">{ENDING_WORD[l.state]}</SeverityPill>
        </li>
      ))}
    </ul>
  );
}

function Decided(p: { d: DelegatedDecision; project: string; fresh: boolean; onFollow?(): void }): JSX.Element {
  const { what, when } = decidedParts(p.d);
  return (
    <li className="needs-you-line">
      {p.fresh && <SeverityPill tone="info">new</SeverityPill>}
      <span aria-hidden="true">{"✓"}</span>
      <Link className="needs-you-link" to={to.item(p.project, p.d.code)} onClick={p.onFollow} title={p.d.title}>
        {what}
      </Link>
      <span className="needs-you-note">
        {when} {"·"}{" "}
        <Link to={to.delegation(p.project, p.d.delegation)} onClick={p.onFollow}>
          {p.d.delegation}
        </Link>
      </span>
    </li>
  );
}

/** Parts two and three: what Claude decided since the last look, and what each delegation holds. */
export function DelegatedParts(p: { parts: NeedsYouParts; project: string; level: 3 | 4; onFollow?(): void }): JSX.Element | null {
  const { decided, holds } = p.parts;
  const decidedAny = decided.fresh.length > 0 || decided.earlier > 0;
  if (!p.parts.delegated || (!decidedAny && holds.length === 0 && decided.more === 0)) return null;
  const H = p.level === 3 ? "h3" : "h4";
  const ids = holds.map((h) => h.delegation).join(", ");
  return (
    <>
      {decidedAny && (
        <section className="needs-you-part needs-you-decided" aria-labelledby="needs-you-decided">
          <H id="needs-you-decided">The agent decided, since you last looked</H>
          {decided.fresh.length > 0 && (
            <ul className="needs-you-lines">
              {decided.fresh.map((d) => (
                <Decided key={d.decision} d={d} project={p.project} fresh onFollow={p.onFollow} />
              ))}
            </ul>
          )}
          {decided.more > 0 && (
            <p className="needs-you-note">
              and {decided.more} more on the{" "}
              <Link to={to.review(p.project, "delegated")} onClick={p.onFollow}>
                Review page
              </Link>
            </p>
          )}
          {decided.earlier > 0 && (
            <details className="needs-you-seen">
              <summary>{decided.fresh.length > 0 ? `${decided.earlier} earlier, already seen` : `${decided.earlier} decided, already seen`}</summary>
              <ul className="needs-you-lines">
                {decided.earlierList.map((d) => (
                  <Decided key={d.decision} d={d} project={p.project} fresh={false} onFollow={p.onFollow} />
                ))}
              </ul>
            </details>
          )}
        </section>
      )}
      {holds.length > 0 && (
        <section className="needs-you-part needs-you-held" aria-labelledby="needs-you-held">
          <H id="needs-you-held">The agent holds</H>
          <details>
            <summary>
              <span aria-hidden="true">{"⚙"}</span> {p.parts.heldTotal} {p.parts.heldTotal === 1 ? "decision" : "decisions"} under {ids}
            </summary>
            <ul className="needs-you-lines">
              {holds.map((h) => (
                <li key={h.delegation} className="needs-you-line">
                  <Link className="needs-you-link" to={h.to} onClick={p.onFollow}>
                    {h.text}
                  </Link>
                </li>
              ))}
            </ul>
          </details>
        </section>
      )}
    </>
  );
}

const readSeen = (project: string): string | null => {
  try {
    return typeof localStorage === "undefined" ? null : localStorage.getItem(seenKey(project));
  } catch {
    return null;
  }
};

/**
 * When Needs you was last looked at in this browser, fixed while a look lasts (so the list does not
 * collapse under the reader). A look is `active`; it ends, and the time is stored, when `active`
 * turns false, the component unmounts, or `SEEN_AFTER_MS` has passed. The one writer of the key.
 * A browser without `localStorage` (a private window) reads as a first visit and never throws.
 */
export function useLastLook(project: string, active: boolean): string | null {
  const seen = useMemo(() => (active ? readSeen(project) : null), [project, active]);
  useEffect(() => {
    if (!active) return;
    const stamp = () => {
      try {
        localStorage.setItem(seenKey(project), new Date().toISOString());
      } catch {
        /* a private window keeps nothing */
      }
    };
    const t = setTimeout(stamp, SEEN_AFTER_MS);
    return () => {
      clearTimeout(t);
      stamp();
    };
  }, [project, active]);
  return seen;
}
