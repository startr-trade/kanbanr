import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";

export default function MilestonesPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  return (
    <div className="milestones-page">
      <div className="page-head">
        <h1>Milestones</h1>
        <LiveDot />
      </div>
      {data.milestones.length === 0 && <p className="muted">No milestones.</p>}
      <div className="ms-list">
        {data.milestones.map((m) => {
          const features = data.features.filter((f) => f.milestone === m.code);
          return (
            <section className="ms-card" key={m.code}>
              <div className="ms-head">
                <Link to={`/p/${encodeURIComponent(project)}/milestone/${encodeURIComponent(m.code)}`}>
                  <b>{m.code}</b> {m.name}
                </Link>
                {m.depends_on.length > 0 && (
                  <span className="muted small">depends on {m.depends_on.join(", ")}</span>
                )}
              </div>
              {m.description && <p className="muted small">{m.description}</p>}
              <ul className="ms-features">
                {features.map((f) => (
                  <li key={f.code}>
                    <span className="mono">{f.code}</span> {f.title}
                    <span className="chip status small">{f.status}</span>
                  </li>
                ))}
                {features.length === 0 && <li className="muted small">No feature items.</li>}
              </ul>
            </section>
          );
        })}
      </div>
    </div>
  );
}
