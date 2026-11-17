import { Link, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import type { CompactIndex } from "../types";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { newestFirst } from "../milestoneOrder";

export default function MilestonesPage() {
  const { project = "" } = useParams();
  // Milestones and their item rows from the compact index, never the whole board (DEF-081).
  const { data, error, loading } = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);

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
        {newestFirst(data.milestones).map((m) => {
          const features = data.items.filter((f) => f.milestone === m.code);
          return (
            <section className="ms-card" key={m.code}>
              <div className="ms-head">
                <Link to={`/p/${encodeURIComponent(project)}/milestone/${encodeURIComponent(m.code)}`}>
                  <b>{m.code}</b> {m.name}
                </Link>
                {(m.depends_on ?? []).length > 0 && (
                  <span className="muted small">depends on {(m.depends_on ?? []).join(", ")}</span>
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
