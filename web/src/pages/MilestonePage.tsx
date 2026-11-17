import { Link, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import type { CompactIndex, Readiness } from "../types";
import { ErrorBox, FeatureCard, Loading, LiveDot } from "../components/bits";

/** Milestone detail: the milestones it depends on, plus its feature items as tiles. */
export default function MilestonePage() {
  const { project = "", code = "" } = useParams();
  // The milestone and its cards come from the compact index, never the whole board (DEF-081).
  const { data, error, loading } = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);
  const readiness = useQuery<Readiness>(project, urls.readiness(project), TAGS.readiness);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const milestone = data.milestones.find((m) => m.code === code);
  if (!milestone) return <ErrorBox error={`Milestone ${code} not found`} />;
  const features = data.items.filter((f) => f.milestone === code);
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
        {(milestone.depends_on ?? []).length === 0 ? (
          <p className="muted">No dependencies.</p>
        ) : (
          <ul className="dep-list">
            {(milestone.depends_on ?? []).map((dep) => {
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
              <FeatureCard
                    key={f.code}
                    project={project}
                    feature={f}
                    siblings={data.items}
                    gaps={readiness.data?.[f.code]?.gaps}
                    next={readiness.data?.[f.code]?.next}
                  />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
