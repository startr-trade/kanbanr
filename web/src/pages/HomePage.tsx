import { Link, Navigate } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { homeTarget } from "../homeTarget";
import { groupByBoard } from "../boards";
import type { ProjectSummary } from "../types";

export default function HomePage() {
  const tick = useLiveTick(api.allEvents());
  const { data, error, loading } = useAsync(() => api.listProjects(), [tick]);
  const boardsRead = useAsync(() => api.getBoards(), [tick]);

  if ((loading && !data) || (boardsRead.loading && !boardsRead.data)) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  const projects = data ?? [];
  // One project: open its Overview rather than a list of one (FTR-048 R-12).
  // It applies only to one project on one board, which groupByBoard decides (FTR-040).
  const groups = groupByBoard(projects, boardsRead.data ?? []);
  const only = groups.length === 1 && groups[0].board === null ? homeTarget(projects) : null;
  if (only) return <Navigate replace to={only} />;

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
      {groups.map((g) =>
        g.board === null ? (
          <Tiles key="all" projects={g.projects} />
        ) : (
          <section key={g.board.name} className="board-group">
            <h2>{g.board.name}</h2>
            <p className="muted small">
              {g.board.head_at
                ? `last changed ${new Date(g.board.head_at).toLocaleString()}`
                : `history can't be read: ${g.board.history_unreadable ?? "unknown"}`}
            </p>
            <Tiles projects={g.projects} />
            <p>
              <Link to={`/portfolio?board=${encodeURIComponent(g.board.name)}`}>Portfolio</Link>
            </p>
          </section>
        ),
      )}
    </div>
  );
}

function Tiles({ projects }: { projects: ProjectSummary[] }) {
  return (
      <div className="tiles">
        {projects.map((p) => {
          const base = `/p/${encodeURIComponent(p.id)}`;
          return (
            <div className="tile" key={p.id}>
              <Link to={base} className="tile-title">
                {p.name || p.id}
              </Link>
              {p.description && <p className="tile-desc">{p.description}</p>}
              <Link to={`${base}/board`} className="tile-primary">
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
                <Link to={`${base}/docs`}><span aria-hidden="true">📄</span> Documentation</Link>
                <span className="muted small">{p.total_features} features</span>
              </div>
            </div>
          );
        })}
      </div>
  );
}
