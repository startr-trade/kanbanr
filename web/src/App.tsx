import { Link, NavLink, Outlet, useLocation } from "react-router-dom";
import { ThemeToggle } from "./components/bits";
import { api } from "./api";
import { useAsync } from "./live";

/** Persistent shell: a simple breadcrumb + within-project tab bar that keeps navigation
 * between projects, the dashboard, status, and documentation one click away. */
export default function App() {
  const { pathname } = useLocation();
  const segs = pathname.split("/").filter(Boolean);
  const inProject = segs[0] === "p" && segs.length >= 2;
  const project = inProject ? decodeURIComponent(segs[1]) : null;
  // The Releases tab only where the project uses releases (FEAT-123): a tab for data a project
  // does not have is a door into an empty room.
  const config = useAsync(
    () => (project ? api.getProject(project).then((p) => p.config) : Promise.resolve(null)),
    [project],
  );
  const usesReleases = config.data?.cadence?.releases === true;

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <Link to="/" aria-label="kanbanr — all projects">
            <BrandMark />
            kanbanr
          </Link>
        </div>
        <nav className="global-nav" aria-label="Global views">
          <NavLink to="/" end className={({ isActive }) => (isActive ? "tab active" : "tab")}>
            Projects
          </NavLink>
          <NavLink to="/portfolio" className={({ isActive }) => (isActive ? "tab active" : "tab")}>
            Portfolio
          </NavLink>
        </nav>
        {/* Which project you are in — not a breadcrumb (FEAT-083). It used to lead with a
            "Projects" link, which duplicated the one in the global nav two elements to its left:
            the same word, twice, pointing at the same place. The global nav is the persistent way
            up, so this says where you are and nothing more. */}
        {project && (
          <nav className="crumbs" aria-label="Current project">
            <span className="sep">/</span>
            <Link to={`/p/${encodeURIComponent(project)}`} className="crumb-project">
              {project}
            </Link>
          </nav>
        )}
        {project && (
          <nav className="tabs" aria-label="Project views">
            <ProjectTab to={`/p/${encodeURIComponent(project)}`} end>
              Board
            </ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/charter`}>Charter</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/review`}>Review</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/milestones`}>Milestones</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/schedule`}>Schedule</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/gantt`}>Gantt</ProjectTab>
            {usesReleases ? (
              <ProjectTab to={`/p/${encodeURIComponent(project)}/releases`}>Releases</ProjectTab>
            ) : null}
            <ProjectTab to={`/p/${encodeURIComponent(project)}/workflow`}>Workflow</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/docs`}>Docs</ProjectTab>
          </nav>
        )}
        <span className="spacer" />
        <ThemeToggle />
      </header>
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}

function ProjectTab({ to, end, children }: { to: string; end?: boolean; children: React.ReactNode }) {
  return (
    <NavLink to={to} end={end} className={({ isActive }) => (isActive ? "tab active" : "tab")}>
      {children}
    </NavLink>
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
