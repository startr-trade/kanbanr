import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { displayedStates } from "../types";
import type { Project } from "../types";

/** Order a set of milestone codes by their dependency DAG (dependencies first). */
function orderByDeps(project: Project, codes: string[]): string[] {
  const set = new Set(codes);
  const depsOf = (c: string) =>
    (project.milestones.find((m) => m.code === c)?.depends_on ?? []).filter((d) => set.has(d));
  const out: string[] = [];
  const seen = new Set<string>();
  const visit = (c: string, stack: Set<string>) => {
    if (seen.has(c) || stack.has(c)) return;
    stack.add(c);
    depsOf(c).forEach((d) => visit(d, stack));
    stack.delete(c);
    seen.add(c);
    out.push(c);
  };
  codes.forEach((c) => visit(c, new Set()));
  return out;
}

/**
 * Derived schedule: for each displayed status, the milestones that contain feature items in
 * that status (ordered by their dependency DAG). Nothing here is stored — it is computed from
 * the current feature statuses, so it can never drift from reality.
 */
export default function SchedulePage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const states = displayedStates(data.config);

  return (
    <div className="schedule-page">
      <div className="page-head">
        <h1>Schedule</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        Milestones grouped by the status of their feature items (dependency-ordered).
      </p>

      {states.map((state) => {
        const involved = data.milestones
          .filter((m) => data.features.some((f) => f.milestone === m.code && f.status === state))
          .map((m) => m.code);
        const ordered = orderByDeps(data, involved);
        return (
          <section className="sched-card" key={state}>
            <div className="ms-head">
              <b>{state}</b>
              <span className="muted small">
                {ordered.length} milestone{ordered.length === 1 ? "" : "s"}
              </span>
            </div>
            {ordered.length === 0 ? (
              <p className="muted small">No milestones have feature items in {state}.</p>
            ) : (
              <ol className="sched-list">
                {ordered.map((code) => {
                  const m = data.milestones.find((mm) => mm.code === code)!;
                  const count = data.features.filter(
                    (f) => f.milestone === code && f.status === state
                  ).length;
                  return (
                    <li key={code}>
                      <Link to={`/p/${encodeURIComponent(project)}/milestone/${encodeURIComponent(code)}`}>
                        <span className="mono">{code}</span> {m.name}
                      </Link>
                      <span className="chip tasks small">
                        {count} in {state}
                      </span>
                      {m.depends_on.length > 0 && (
                        <span className="muted small">needs {m.depends_on.join(", ")}</span>
                      )}
                    </li>
                  );
                })}
              </ol>
            )}
          </section>
        );
      })}
    </div>
  );
}
