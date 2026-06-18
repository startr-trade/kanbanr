import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { BoardLane, Counts, Disposition } from "../types";

/** Portfolio overview (FEAT-030): cross-project rollups (milestone → project → program →
 * portfolio %) plus a cross-project board grouping every feature into normalized lanes. */
export default function PortfolioPage() {
  const tick = useLiveTick(api.allEvents());
  const rollups = useAsync(() => api.getPortfolioRollups(), [tick]);
  const board = useAsync(() => api.getPortfolioBoard(), [tick]);

  if (rollups.loading && !rollups.data) return <Loading />;
  if (rollups.error) return <ErrorBox error={rollups.error} />;
  const report = rollups.data;

  return (
    <div className="portfolio">
      <div className="page-head">
        <h1>{report?.portfolio ?? "Portfolio"}</h1>
        <LiveDot />
      </div>

      {report && (
        <section className="portfolio-rollups">
          <h2>
            Rollups <Pct counts={report.counts} />
          </h2>
          {report.programs.length === 0 && (
            <p className="muted">No programs yet. Declare one with the CLI: <code>kanbanr portfolio add-program prog --projects a,b</code>.</p>
          )}
          {report.programs.map((prog) => (
            <div className="rollup-program" key={prog.id}>
              <div className="rollup-row program">
                <span className="rollup-name">{prog.name}</span>
                <Bar counts={prog.counts} />
              </div>
              {prog.projects.map((proj) => (
                <div className="rollup-project" key={proj.id}>
                  <div className="rollup-row project">
                    <Link className="rollup-name" to={`/p/${encodeURIComponent(proj.id)}`}>
                      {proj.name}
                    </Link>
                    <Bar counts={proj.counts} />
                  </div>
                  {proj.milestones.map((ms) => (
                    <div className="rollup-row milestone" key={ms.code}>
                      <span className="rollup-name muted small">{ms.name || ms.code}</span>
                      <Bar counts={ms.counts} />
                    </div>
                  ))}
                </div>
              ))}
            </div>
          ))}
        </section>
      )}

      <section className="portfolio-board">
        <h2>Cross-project board</h2>
        {board.error && <ErrorBox error={board.error} />}
        {board.data && (
          <div className="board-lanes">
            {board.data.lanes.map((lane) => (
              <Lane key={lane.disposition} lane={lane} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

const LANE_LABEL: Record<Disposition, string> = {
  "not-started": "Not started",
  "in-progress": "In progress",
  done: "Done",
};

function Lane({ lane }: { lane: BoardLane }) {
  return (
    <div className={`board-lane lane-${lane.disposition}`}>
      <div className="lane-head">
        <span>{LANE_LABEL[lane.disposition]}</span>
        <b>{lane.cards.length}</b>
      </div>
      <div className="lane-cards">
        {lane.cards.map((c) => (
          <Link
            key={`${c.project}:${c.code}`}
            to={`/p/${encodeURIComponent(c.project)}/feature/${encodeURIComponent(c.code)}`}
            className="board-card"
          >
            <div className="card-top">
              <span className="card-proj">{c.project}</span>
              <span className="card-code">{c.code}</span>
            </div>
            <div className="card-title">{c.title}</div>
            <div className="card-meta muted small">
              <span>{c.status}</span>
              {c.tasks_total > 0 && (
                <span>
                  {c.tasks_done}/{c.tasks_total}
                </span>
              )}
              {c.assignee && <span>@{c.assignee}</span>}
            </div>
          </Link>
        ))}
        {lane.cards.length === 0 && <p className="muted small">—</p>}
      </div>
    </div>
  );
}

function Pct({ counts }: { counts: Counts }) {
  return <span className="muted">{counts.percent}%</span>;
}

function Bar({ counts }: { counts: Counts }) {
  return (
    <span className="rollup-bar" title={`${counts.tasks_done}/${counts.tasks_total} tasks`}>
      <span className="rollup-bar-fill" style={{ width: `${counts.percent}%` }} />
      <span className="rollup-pct">{counts.percent}%</span>
    </span>
  );
}
