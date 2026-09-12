import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import Markdown from "../components/Markdown";
import { ErrorBox, FeatureBadges, Loading, LiveDot, TaskBadge, progress } from "../components/bits";
import { featureProgress, listsNewestFirst, listFullyCompleted } from "../types";
import type { Feature } from "../types";

/** Dedicated page for one feature (epic): Specification + a tile per todo-list (newest first). */
export default function FeaturePage() {
  const { project = "", code = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const feature = data.features.find((f) => f.code === code);
  if (!feature) return <ErrorBox error={`Feature ${code} not found`} />;
  const p = featureProgress(feature);
  const lists = listsNewestFirst(feature.todo_lists);

  return (
    <div className="feature-page">
      <div className="page-head">
        <h1>
          <span className="mono">{feature.code}</span> — {feature.title}
        </h1>
        <LiveDot />
      </div>

      <div className="feature-meta">
        <span className="chip status">{feature.status}</span>
        {feature.milestone && (
          <Link
            className="chip"
            to={`/p/${encodeURIComponent(project)}/milestone/${encodeURIComponent(feature.milestone)}`}
          >
            {feature.milestone}
          </Link>
        )}
        <span className="chip tasks">
          {p.done}/{p.total} tasks
        </span>
        <FeatureBadges feature={feature} />
        {feature.due && <span className="chip">due {feature.due}</span>}
        <span className="spacer" />
        <a className="btn" href={api.exportUrl(project, feature.code, "md")} target="_blank" rel="noreferrer">
          Export .md
        </a>
        <a className="btn" href={api.exportUrl(project, feature.code, "json")} target="_blank" rel="noreferrer">
          Export .json
        </a>
      </div>

      {(feature.depends_on ?? []).length > 0 && (
        <div className="depends">
          <span className="muted small">Blocked by:</span>
          {(feature.depends_on ?? []).map((dep) => {
            const known = data.features.some((f) => f.code === dep);
            return known ? (
              <Link
                key={dep}
                className="chip blocked"
                to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(dep)}`}
              >
                {dep}
              </Link>
            ) : (
              <span key={dep} className="chip blocked">{dep}</span>
            );
          })}
        </div>
      )}

      <Provenance feature={feature} />

      <section className="section">
        <h2>Specification</h2>
        {feature.specification.trim() ? (
          <Markdown source={feature.specification} />
        ) : (
          <p className="muted">No specification provided.</p>
        )}
      </section>

      <section className="section">
        <h2>
          Todo-lists <span className="muted small">(newest first)</span>
        </h2>
        {lists.length === 0 ? (
          <p className="muted">No todo-lists yet.</p>
        ) : (
          <div className="todo-tiles">
            {lists.map((l) => {
              const lp = progress(l.tasks);
              return (
                <section className={`todo-tile${listFullyCompleted(l) ? " done" : ""}`} key={l.code}>
                  <div className="todo-tile-head">
                    <code className="taskkey">{l.code}</code>
                    <span className="todo-desc">{l.description || "(no description)"}</span>
                    <span className="muted small">
                      {lp.done}/{lp.total}
                    </span>
                  </div>
                  {l.tasks.length === 0 ? (
                    <div className="muted small pad">No tasks.</div>
                  ) : (
                    <ul className="tasklist">
                      {l.tasks.map((t) => (
                        <li key={t.key}>
                          <code className="taskkey">{t.key}</code>
                          <span className="tasktext">{t.text}</span>
                          <TaskBadge state={t.state} />
                        </li>
                      ))}
                    </ul>
                  )}
                </section>
              );
            })}
          </div>
        )}
      </section>

      <FeatureActivity project={project} code={feature.code} tick={tick} />
    </div>
  );
}

const day = (ts?: string | null) => (ts ?? "").slice(0, 10);

/** Where the feature was imported from, and the issue it is mirrored to. The source is history:
 *  its original text lives in the spec, so a vanished source is labeled, never a broken link. */
function Provenance({ feature }: { feature: Feature }) {
  const { source, issue } = feature;
  if (!source && !issue) return null;
  return (
    <div className="provenance">
      {source && (
        <span className="muted small">
          Imported from <code>{source.ref}</code> ({source.system})
          {source.revision && (
            <>
              {" "}at commit <code>{source.revision.slice(0, 7)}</code>
            </>
          )}{" "}
          on {day(source.imported_at)}
          {source.url && !source.missing_since && (
            <>
              {" · "}
              <a href={source.url} target="_blank" rel="noreferrer">
                open ↗
              </a>
            </>
          )}
        </span>
      )}
      {source?.missing_since && (
        <span className="chip warn" title="The original text is preserved in the specification below.">
          source no longer present (since {day(source.missing_since)})
        </span>
      )}
      {issue && (
        <a className="chip" href={issue.url} target="_blank" rel="noreferrer">
          {issue.system === "github" ? "GitHub" : issue.system} issue #{issue.number} ↗
        </a>
      )}
      {issue && (
        <span className="muted small">
          {issue.synced_at ? `mirrored ${day(issue.synced_at)}` : "not yet mirrored"}
        </span>
      )}
    </div>
  );
}

/** This feature's recent activity (the changelog filtered to its code). */
function FeatureActivity({ project, code, tick }: { project: string; code: string; tick: number }) {
  const { data } = useAsync(() => api.getActivity(project, `?ref=${encodeURIComponent(code)}`), [project, code, tick]);
  if (!data || data.length === 0) return null;
  return (
    <section className="activity">
      <h2>Activity</h2>
      <ul className="activity-list">
        {data.slice(0, 10).map((a, i) => (
          <li key={i}>
            <span className="activity-msg">{a.message}</span>
            <span className="activity-meta">
              {a.actor} · {new Date(a.time).toLocaleString()}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
