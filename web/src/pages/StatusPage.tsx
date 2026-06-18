import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot, TaskBadge, progress } from "../components/bits";
import { featureProgress, listsNewestFirst, listFullyCompleted } from "../types";

/**
 * A single feature status, grouped by feature item. Under each feature it shows only the
 * todo-lists that are NOT fully completed (newest first), with their tasks.
 */
export default function StatusPage() {
  const { project = "", state = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const features = data.features.filter((f) => f.status === state);

  return (
    <div className="status-page">
      <div className="page-head">
        <h1>
          Status: <span className="status-name">{state}</span>
        </h1>
        <LiveDot />
      </div>
      <p className="muted small">
        {features.length} feature{features.length === 1 ? "" : "s"} in this status
      </p>

      {features.length === 0 && <p className="muted">No features in {state}.</p>}

      {features.map((f) => {
        const p = featureProgress(f);
        // Only the open todo-lists (not fully completed), newest first.
        const openLists = listsNewestFirst(f.todo_lists).filter((l) => !listFullyCompleted(l));
        return (
          <section className="status-group" key={f.code}>
            <div className="status-group-head">
              <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(f.code)}`}>
                <b>{f.code}</b> {f.title}
              </Link>
              <span className="muted small">
                {p.done}/{p.total} tasks · {openLists.length} open list{openLists.length === 1 ? "" : "s"}
              </span>
            </div>
            {openLists.length === 0 ? (
              <div className="muted small pad">No open todo-lists.</div>
            ) : (
              openLists.map((l) => {
                const lp = progress(l.tasks);
                return (
                  <div className="status-todo" key={l.code}>
                    <div className="status-todo-head">
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
                  </div>
                );
              })
            )}
          </section>
        );
      })}
    </div>
  );
}
