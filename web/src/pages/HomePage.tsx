import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";

export default function HomePage() {
  const tick = useLiveTick(api.allEvents());
  const { data, error, loading } = useAsync(() => api.listProjects(), [tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  const projects = data ?? [];

  return (
    <div className="home">
      <div className="page-head">
        <h1>Projects</h1>
        <LiveDot />
      </div>
      {projects.length === 0 && (
        <p className="muted">
          No projects yet. Create one with the Claude skill / CLI: <code>kanbanr project init my-project</code>.
        </p>
      )}
      <div className="tiles">
        {projects.map((p) => {
          const base = `/p/${encodeURIComponent(p.id)}`;
          return (
            <div className="tile" key={p.id}>
              <Link to={base} className="tile-title">
                {p.name || p.id}
              </Link>
              {p.description && <p className="tile-desc">{p.description}</p>}
              <Link to={base} className="tile-primary">
                Open dashboard →
              </Link>
              <div className="tile-states">
                {p.displayed_states.map((s) => (
                  <Link key={s} to={`${base}/state/${encodeURIComponent(s)}`} className="statelink">
                    {s} <b>{p.counts[s] ?? 0}</b>
                  </Link>
                ))}
              </div>
              {p.other_states.length > 0 && (
                <div className="tile-states other">
                  {p.other_states.map((s) => (
                    <Link
                      key={s}
                      to={`${base}/state/${encodeURIComponent(s)}`}
                      className={`statelink muted${p.no_op_states.includes(s) ? " noop" : ""}`}
                      title={p.no_op_states.includes(s) ? "no-op state" : "non-displayed state"}
                    >
                      {s} <b>{p.counts[s] ?? 0}</b>
                    </Link>
                  ))}
                </div>
              )}
              <div className="tile-foot">
                <Link to={`${base}/docs`}>📄 Documentation</Link>
                <span className="muted small">{p.total_features} features</span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
