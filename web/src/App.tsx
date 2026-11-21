import { useEffect, useReducer, useRef } from "react";
import { Link, NavLink, Outlet, useLocation } from "react-router-dom";
import { ThemeToggle } from "./components/bits";
import { NeedsYouBadge } from "./components/NeedsYou";
import { api } from "./api";
import { useAsync } from "./live";
import { useQuery } from "./data/useQuery";
import { TAGS, urls } from "./data/urls";
import type { CompactIndex } from "./types";
import { behindWarning, buildLabel } from "./buildInfo";
import { NAV, activeEntry, hereLabel, visibleAreas, type AreaKey, type NavArea } from "./nav";
import { navReducer, type NavState } from "./navState";

/** Persistent shell: the brand, the navigation (global views, the project, and the project's pages
 * grouped into areas, FTR-047), the build label and the theme toggle. */
export default function App() {
  const { pathname } = useLocation();
  const segs = pathname.split("/").filter(Boolean);
  const inProject = segs[0] === "p" && segs.length >= 2;
  const project = inProject ? decodeURIComponent(segs[1]) : null;
  // The index decides the conditional entries, such as Releases only where the project uses
  // releases (FEAT-123): a page for data a project does not have is a door into an empty room.
  // Read from the compact index, never the whole board (DEF-081); a failed read hides only those
  // entries, as before.
  const index = useQuery<CompactIndex>(project, project ? urls.index(project) : null, TAGS.index);
  // Which kanbanr this monitor is, and whether the board has moved on past it (FTR-036). A failed
  // call shows nothing: the pages already say when the daemon is down.
  const meta = useAsync(() => api.getMeta().catch(() => null), []);
  const label = buildLabel(meta.data ?? null);
  const behind = behindWarning(meta.data ?? null);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <Link to="/" aria-label="Kanbanr — all projects">
            <BrandMark />
            <span className="brand-word">kanbanr</span>
          </Link>
        </div>
        <ShellNav project={project} index={index.data} />
        {project && <NeedsYouBadge project={project} />}
        <span className="spacer" />
        {label && (
          <span className="build" title="The Kanbanr this monitor runs">
            {label}
          </span>
        )}
        <ThemeToggle />
      </header>
      {behind && (
        <div className="warn-box" role="alert">
          <strong>Behind the board.</strong> {behind}
        </div>
      )}
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}

const CLOSED: NavState = { open: null, mobile: false, returnFocus: null };

/**
 * The top bar's navigation (FTR-047): one `<nav>` that serves both widths, so each destination is
 * in the shell once (FEAT-083).
 *
 * Above 720 px it is one row: the global views, the project, then each area — a plain link named
 * for the area when it has one page (R-8), else a disclosure button ("Work ▾") opening a list under
 * it (R-6). At 720 px and below, `.nav-areas` folds behind the ≡ button into a full-width panel
 * where every area is a heading with all its pages, and the global views move to its foot under
 * "All projects"; CSS only reorders them. `nav` is a prop so a check can pass a fixture.
 */
export function ShellNav({
  project,
  index,
  nav = NAV,
}: {
  project: string | null;
  index: CompactIndex | null;
  nav?: NavArea[];
}) {
  const { pathname } = useLocation();
  const [state, dispatch] = useReducer(navReducer, CLOSED);
  const navRef = useRef<HTMLElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const areaRefs = useRef<Partial<Record<AreaKey, HTMLButtonElement | null>>>({});

  const areas = project ? visibleAreas(nav, index) : [];
  const active = project ? activeEntry(nav, project, pathname) : null;
  const here = project ? hereLabel(nav, project, pathname) : "";
  const anyOpen = state.open !== null || state.mobile;

  // Reaching another page closes whatever menu was open.
  const lastPath = useRef(pathname);
  useEffect(() => {
    if (lastPath.current === pathname) return;
    lastPath.current = pathname;
    dispatch({ type: "navigate" });
  }, [pathname]);

  // While a menu is open: a press outside the nav closes it, and Escape closes it from anywhere.
  useEffect(() => {
    if (!anyOpen) return;
    const onPointerDown = (e: PointerEvent) => {
      if (navRef.current && e.target instanceof Node && navRef.current.contains(e.target)) return;
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
  }, [anyOpen]);

  // After Escape, focus goes back to the button that opened the menu; then the request is cleared.
  useEffect(() => {
    if (state.returnFocus === null) return;
    const el = state.returnFocus === "mobile" ? toggleRef.current : areaRefs.current[state.returnFocus];
    el?.focus();
    dispatch({ type: "close" });
  }, [state.returnFocus]);

  const linkProps = (key: string, areaKey: AreaKey) =>
    active?.area === areaKey && active.entry === key ? { "aria-current": "page" as const } : {};
  const onNavigate = () => dispatch({ type: "navigate" });

  return (
    <nav
      ref={navRef}
      className={["nav", project ? null : "no-project", state.mobile ? "open" : null].filter(Boolean).join(" ")}
      aria-label={project ? "Project" : "Global views"}
    >
      {project && (
        <span className="nav-here">
          <span aria-hidden="true">· </span>
          {here}
        </span>
      )}
      {project && (
        <button
          ref={toggleRef}
          type="button"
          className="nav-toggle"
          aria-expanded={state.mobile}
          aria-controls="nav-areas"
          aria-label="Menu"
          onClick={() => dispatch({ type: "toggle-mobile" })}
        >
          ≡
        </button>
      )}
      <div className="nav-areas" id="nav-areas">
        <div className="global-nav" role="group" aria-labelledby="nav-global-heading">
          <h2 className="nav-heading" id="nav-global-heading">
            All projects
          </h2>
          <NavLink to="/" end className={({ isActive }) => (isActive ? "tab active" : "tab")} onClick={onNavigate}>
            Projects
          </NavLink>
          <NavLink to="/portfolio" className={({ isActive }) => (isActive ? "tab active" : "tab")} onClick={onNavigate}>
            Portfolio
          </NavLink>
        </div>
        {/* Which project you are in — not a breadcrumb (FEAT-083). It used to lead with a
            "Projects" link, which duplicated the one in the global nav two elements to its left:
            the same word, twice, pointing at the same place. The global nav is the persistent way
            up, so this says where you are and nothing more. */}
        {project && (
          <div className="crumbs" role="group" aria-label="Current project">
            <span className="sep">/</span>
            <Link to={`/p/${encodeURIComponent(project)}`} className="crumb-project" onClick={onNavigate}>
              {project}
            </Link>
          </div>
        )}
        {areas.map((area) => {
          const current = active?.area === area.key;
          const cls = current ? "nav-area current" : "nav-area";
          if (area.entries.length === 1) {
            // One page: a plain link named for the area, never a one-entry menu (R-8).
            const only = area.entries[0];
            return (
              <div className="nav-item nav-single" key={area.key}>
                <Link to={only.path(project!)} className={cls} onClick={onNavigate} {...linkProps(only.key, area.key)}>
                  {area.label}
                </Link>
              </div>
            );
          }
          const open = state.open === area.key;
          return (
            <div className="nav-item" key={area.key}>
              <h2 className="nav-heading">{area.label}</h2>
              <button
                ref={(el) => {
                  areaRefs.current[area.key] = el;
                }}
                type="button"
                className={cls}
                aria-expanded={open}
                aria-controls={`nav-${area.key}`}
                onClick={() => dispatch({ type: "toggle", area: area.key })}
              >
                {area.label}
                <span aria-hidden="true"> ▾</span>
              </button>
              <ul className="nav-menu" id={`nav-${area.key}`} hidden={!open}>
                {area.entries.map((e) => (
                  <li key={e.key}>
                    <Link to={e.path(project!)} className="nav-link" onClick={onNavigate} {...linkProps(e.key, area.key)}>
                      {e.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          );
        })}
      </div>
    </nav>
  );
}

/**
 * The kanbanr mark (FEAT-133): a `k` whose stem is a kanban column of three cards and whose arms
 * are the thread the work runs along — one to the next item, one landing on done. Drawn with the
 * theme's brand tokens, so it follows the light/dark toggle; assets/brand/ holds the same mark.
 */
function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 160 160" aria-hidden="true" focusable="false">
      <path className="arm" d="M70 80 L124 32 M70 80 L124 128" />
      <rect className="card" x="26" y="20" width="44" height="30" rx="7" />
      <rect className="card" x="26" y="65" width="44" height="30" rx="7" />
      <rect className="card" x="26" y="110" width="44" height="30" rx="7" />
      <circle className="next" cx="124" cy="32" r="13" />
      <circle className="done" cx="124" cy="128" r="16" />
    </svg>
  );
}
