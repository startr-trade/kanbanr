import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, FeatureCard, Loading, LiveDot } from "../components/bits";

/** Milestone detail: the milestones it depends on, plus its feature items as tiles. */
export default function MilestonePage() {
  const { project = "", code = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const milestone = data.milestones.find((m) => m.code === code);
  if (!milestone) return <ErrorBox error={`Milestone ${code} not found`} />;
  const features = data.features.filter((f) => f.milestone === code);
  const base = `/p/${encodeURIComponent(project)}`;

  return (
    <div className="milestone-page">
      <div className="page-head">
        <h1>
          <span className="mono">{milestone.code}</span> — {milestone.name}
        </h1>
        <LiveDot />
      </div>
      {milestone.description && <p className="muted">{milestone.description}</p>}

      <section className="section">
        <h2>Depends on</h2>
        {milestone.depends_on.length === 0 ? (
          <p className="muted">No dependencies.</p>
        ) : (
          <ul className="dep-list">
            {milestone.depends_on.map((dep) => {
              const m = data.milestones.find((mm) => mm.code === dep);
              return (
                <li key={dep}>
                  <Link to={`${base}/milestone/${encodeURIComponent(dep)}`}>
                    <span className="mono">{dep}</span> {m?.name ?? "(unknown)"}
                  </Link>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="section">
        <h2>
          Feature items <span className="muted small">({features.length})</span>
        </h2>
        {features.length === 0 ? (
          <p className="muted">No feature items in this milestone.</p>
        ) : (
          <div className="tiles">
            {features.map((f) => (
              <FeatureCard key={f.code} project={project} feature={f} siblings={data.features} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
