import { Link, NavLink, Outlet, useLocation } from "react-router-dom";
import { ThemeToggle } from "./components/bits";

/** Persistent shell: a simple breadcrumb + within-project tab bar that keeps navigation
 * between projects, the dashboard, status, and documentation one click away. */
export default function App() {
  const { pathname } = useLocation();
  const segs = pathname.split("/").filter(Boolean);
  const inProject = segs[0] === "p" && segs.length >= 2;
  const project = inProject ? decodeURIComponent(segs[1]) : null;

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <Link to="/">▦ kanbanr</Link>
        </div>
        <nav className="global-nav" aria-label="Global views">
          <NavLink to="/" end className={({ isActive }) => (isActive ? "tab active" : "tab")}>
            Projects
          </NavLink>
          <NavLink to="/portfolio" className={({ isActive }) => (isActive ? "tab active" : "tab")}>
            Portfolio
          </NavLink>
        </nav>
        {project && (
          <nav className="crumbs">
            <Link to="/">Projects</Link>
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
            <ProjectTab to={`/p/${encodeURIComponent(project)}/milestones`}>Milestones</ProjectTab>
            <ProjectTab to={`/p/${encodeURIComponent(project)}/schedule`}>Schedule</ProjectTab>
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
