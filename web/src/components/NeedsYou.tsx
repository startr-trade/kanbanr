// The Needs-you badge in the top bar and its drawer (FTR-047 placed the slot, FTR-048 fills it).
// The shell renders the badge between the project navigation and the build label, outside the
// menu, so it is always visible; this file never needs App.tsx edited.
//
// The badge's number counts only the person's own decisions (review groups and releases at a
// stage); health and records lines are Claude's to act on, so they are in the drawer but not in the
// number (FTR-048 R-2). The drawer opens and closes as the navigation's menus do (FTR-047): Escape
// and a press outside close it.
import { useEffect, useReducer, useRef, type Ref } from "react";
import { Link, useLocation } from "react-router-dom";
import { useQuery } from "../data/useQuery.ts";
import { TAGS, urls } from "../data/urls.ts";
import { CopyText } from "../ui/CopyText.tsx";
import { SeverityPill } from "../ui/SeverityPill.tsx";
import { ErrorState, LoadingState } from "../ui/States.tsx";
import type { Tone } from "../ui/tone.ts";
import { CLOSED, badgeLabel, drawerSections, needsYouReducer, type DrawerLine } from "../needsYouState.ts";
import type { Attention, DriftLatest } from "../types";

const TONES: Tone[] = ["ok", "info", "warn", "error", "neutral"];
const toneOf = (t: string): Tone => (TONES.includes(t as Tone) ? (t as Tone) : "neutral");

/** The badge, reading `/attention` and the latest drift through the shared cache. */
export function NeedsYouBadge(p: { project: string }): JSX.Element | null {
  const attention = useQuery<Attention>(p.project, urls.attention(p.project), TAGS.attention);
  const drift = useQuery<DriftLatest>(p.project, urls.driftLatest(p.project, false), TAGS.driftLatest);
  const [state, dispatch] = useReducer(needsYouReducer, CLOSED);
  const { pathname } = useLocation();
  const wrap = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  // Reaching another page closes the drawer.
  const lastPath = useRef(pathname);
  useEffect(() => {
    if (lastPath.current === pathname) return;
    lastPath.current = pathname;
    dispatch({ type: "navigate" });
  }, [pathname]);

  // While open: a press outside the badge and drawer closes it, and Escape closes it from anywhere.
  useEffect(() => {
    if (!state.open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (wrap.current && e.target instanceof Node && wrap.current.contains(e.target)) return;
      dispatch({ type: "outside" });
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") dispatch({ type: "escape" });
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [state.open]);

  // After Escape, focus goes back to the badge; then the request is cleared.
  useEffect(() => {
    if (!state.returnFocus) return;
    button.current?.focus();
    dispatch({ type: "close" });
  }, [state.returnFocus]);

  return (
    <NeedsYouView
      wrapRef={wrap}
      buttonRef={button}
      attention={attention.data}
      error={attention.error}
      drift={drift.data}
      open={state.open}
      onToggle={() => dispatch({ type: "toggle" })}
      onClose={() => dispatch({ type: "close" })}
    />
  );
}

/**
 * What the badge draws, from its data and whether the drawer is open: the button, with the total in
 * a pill when it is above 0, and the drawer below it when open. `attention` is null while loading,
 * when the button shows no number and never says nothing is waiting.
 */
export function NeedsYouView(p: {
  attention: Attention | null;
  error?: string | null;
  drift: DriftLatest | null;
  open: boolean;
  onToggle(): void;
  onClose(): void;
  wrapRef?: Ref<HTMLSpanElement>;
  buttonRef?: Ref<HTMLButtonElement>;
}): JSX.Element {
  const total = p.attention?.total ?? 0;
  return (
    <span className="needs-you" ref={p.wrapRef}>
      <button
        ref={p.buttonRef}
        type="button"
        className="needs-you-badge"
        aria-expanded={p.open}
        aria-controls="needs-you"
        aria-label={p.attention ? badgeLabel(total) : "Needs you"}
        onClick={p.onToggle}
      >
        Needs you
        {total > 0 && <span className="needs-you-count">{total}</span>}
      </button>
      {p.open &&
        (p.attention ? (
          <NeedsYouDrawer attention={p.attention} drift={p.drift} onClose={p.onClose} />
        ) : (
          <div id="needs-you" className="needs-you-drawer" role="dialog" aria-modal="false" aria-labelledby="needs-you-title">
            <h2 id="needs-you-title">Needs you</h2>
            {p.error ? <ErrorState error={p.error} /> : <LoadingState />}
          </div>
        ))}
    </span>
  );
}

/** The drawer: per section a heading and its lines, as links or as a sentence to copy. */
export function NeedsYouDrawer(p: { attention: Attention; drift: DriftLatest | null; onClose(): void }): JSX.Element {
  const sections = drawerSections(p.attention, p.drift, new Date());
  return (
    <div id="needs-you" className="needs-you-drawer" role="dialog" aria-modal="false" aria-labelledby="needs-you-title">
      <h2 id="needs-you-title">Needs you</h2>
      {p.attention.total === 0 && <p className="needs-you-none">Nothing needs you right now.</p>}
      {sections.map((s) => (
        <section key={s.key} className="needs-you-section" aria-labelledby={`needs-you-${s.key}`}>
          <h3 id={`needs-you-${s.key}`}>{s.title}</h3>
          <ul className="needs-you-lines">
            {s.lines.map((l, i) => (
              <DrawerItem key={`${l.kind}-${i}`} line={l} onClose={p.onClose} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function DrawerItem(p: { line: DrawerLine; onClose(): void }): JSX.Element {
  const l = p.line;
  const count = <SeverityPill tone={toneOf(l.tone)}>{l.count}</SeverityPill>;
  return (
    <li className="needs-you-line">
      {l.to ? (
        <Link className="needs-you-link" to={l.to} onClick={p.onClose}>
          <span className="needs-you-label">{l.label}</span> {count}
        </Link>
      ) : (
        <span className="needs-you-link">
          <span className="needs-you-label">{l.label}</span> {count}
        </span>
      )}
      {l.note && <span className="needs-you-note">{l.note}</span>}
      {l.ask && (
        <span className="needs-you-ask">
          <q>{l.ask}</q> <CopyText text={l.ask} label="Copy" />
        </span>
      )}
    </li>
  );
}
