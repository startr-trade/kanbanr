import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { BoardLane, Counts, Disposition } from "../types";

/** The same page sizes the project board offers, and the same remembered choice. */
const PAGE_SIZES = [10, 25, 50, 100];

/** Portfolio overview (FEAT-030): cross-project rollups (milestone → project → program →
 * portfolio %) plus a cross-project board grouping every feature into normalized lanes. */
export default function PortfolioPage() {
  const tick = useLiveTick(api.allEvents());
  // A portfolio lane holds EVERY project's work in that disposition, so it is the longest column
  // in the product — a "Not started" lane across several boards runs to hundreds of cards, and
  // the page became a scroll with no shape (FEAT-091). Paginated per lane, exactly as the project
  // board's columns are, down to the remembered page size.
  const [pageSize, setPageSize] = useState<number | "all">(() => {
    const saved = localStorage.getItem("kanbanr-board-page-size");
    if (saved === "all") return "all";
    const n = Number(saved);
    return PAGE_SIZES.includes(n) ? n : 25;
  });
  const [pages, setPages] = useState<Record<string, number>>({});
  useEffect(() => {
    // Shared with the project board on purpose: it is one reader's preference, not one page's.
    try {
      localStorage.setItem("kanbanr-board-page-size", String(pageSize));
    } catch {
      /* private window, blocked storage — the choice just does not persist */
    }
  }, [pageSize]);
  useEffect(() => {
    setPages({});
  }, [pageSize]);
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
        <span className="spacer" />
        <label className="per-page">
          Per page
          <select
            value={String(pageSize)}
            onChange={(e) => setPageSize(e.target.value === "all" ? "all" : Number(e.target.value))}
          >
            {PAGE_SIZES.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
            <option value="all">All</option>
          </select>
        </label>
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
              <Lane
                key={lane.disposition}
                lane={lane}
                pageSize={pageSize}
                page={pages[lane.disposition] ?? 1}
                onPage={(p) => setPages((prev) => ({ ...prev, [lane.disposition]: p }))}
              />
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

function Lane({
  lane,
  pageSize,
  page,
  onPage,
}: {
  lane: BoardLane;
  pageSize: number | "all";
  page: number;
  onPage: (p: number) => void;
}) {
  const total = lane.cards.length;
  const size = pageSize === "all" ? Math.max(total, 1) : pageSize;
  const pageCount = Math.max(1, Math.ceil(total / size));
  const current = Math.min(Math.max(1, page), pageCount);
  const shown = pageSize === "all" ? lane.cards : lane.cards.slice((current - 1) * size, current * size);
  return (
    <div className={`board-lane lane-${lane.disposition}`}>
      <div className="lane-head">
        <span>{LANE_LABEL[lane.disposition]}</span>
        {/* The count is the WHOLE lane, not the page — a header that counted the page would make
            a paginated lane look smaller than it is. */}
        <b>{total}</b>
      </div>
      <div className="lane-cards">
        {shown.map((c) => (
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
        {total === 0 && <p className="muted small">—</p>}
      </div>
      {pageCount > 1 && (
        <div className="pager">
          <button
            type="button"
            disabled={current <= 1}
            onClick={() => onPage(current - 1)}
            aria-label={`Previous page of ${LANE_LABEL[lane.disposition]}`}
          >
            ‹
          </button>
          <span className="muted small">
            {current} / {pageCount}
          </span>
          <button
            type="button"
            disabled={current >= pageCount}
            onClick={() => onPage(current + 1)}
            aria-label={`Next page of ${LANE_LABEL[lane.disposition]}`}
          >
            ›
          </button>
        </div>
      )}
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
