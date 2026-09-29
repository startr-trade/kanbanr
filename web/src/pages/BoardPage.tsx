import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { displayedStates, featuresNewestFirst } from "../types";
import type { Feature } from "../types";
import { ErrorBox, FeatureCard, Loading, LiveDot } from "../components/bits";

const PAGE_SIZES = [10, 25, 50] as const;
type PageSize = number | "all";

/** Remembered page-size choice (localStorage), defaulting to 10. */
function initialPageSize(): PageSize {
  const v = localStorage.getItem("kanbanr-board-page-size");
  if (v === "all") return "all";
  const n = Number(v);
  return (PAGE_SIZES as readonly number[]).includes(n) ? n : 10;
}

export default function BoardPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);
  const readiness = useAsync(() => api.getReadiness(project), [project, tick]);
  const [q, setQ] = useState("");
  const [pageSize, setPageSize] = useState<PageSize>(initialPageSize);
  // Per-status current page (1-based); each column paginates independently.
  const [pages, setPages] = useState<Record<string, number>>({});

  useEffect(() => {
    localStorage.setItem("kanbanr-board-page-size", String(pageSize));
  }, [pageSize]);
  // Reset every column to page 1 when the page size or the filter changes.
  useEffect(() => {
    setPages({});
  }, [pageSize, q]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const states = displayedStates(data.config);
  // Non-displayed statuses (includes no-op) — still reachable via status-page links.
  const otherStates = data.config.statuses.filter((s) => !states.includes(s));
  const isNoOp = (s: string) => data.config.no_op_states?.includes(s);
  const countIn = (s: string) => data.features.filter((f) => f.status === s).length;

  // Filter/search across code, title, kind, priority, labels.
  const needle = q.trim().toLowerCase();
  const matches = (f: Feature) =>
    !needle ||
    [f.code, f.title, f.kind ?? "", f.priority ?? "", ...(f.labels ?? [])]
      .join(" ")
      .toLowerCase()
      .includes(needle);

  const setPage = (state: string, p: number) => setPages((prev) => ({ ...prev, [state]: p }));

  return (
    <div className="board-page">
      <div className="page-head">
        <h1>{data.config.name || data.id}</h1>
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
        <input
          className="board-search"
          placeholder="Filter… (text · kind · priority · label)"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      </div>
      {data.config.description && <p className="muted">{data.config.description}</p>}
      <div className="board">
        {states.map((state) => {
          // Reverse-chronological within the column, then paginate with the shared page size.
          const all = featuresNewestFirst(data.features.filter((f) => f.status === state && matches(f)));
          const total = all.length;
          const size = pageSize === "all" ? Math.max(total, 1) : pageSize;
          const pageCount = Math.max(1, Math.ceil(total / size));
          const page = Math.min(Math.max(1, pages[state] ?? 1), pageCount);
          const shown = pageSize === "all" ? all : all.slice((page - 1) * size, page * size);
          return (
            <section className="column" key={state}>
              <Link className="column-head" to={`/p/${encodeURIComponent(project)}/state/${encodeURIComponent(state)}`}>
                {state} <span className="count">{total}</span>
              </Link>
              <div className="column-body">
                {shown.map((f) => (
                  <FeatureCard
                    key={f.code}
                    project={project}
                    feature={f}
                    siblings={data.features}
                    gaps={readiness.data?.[f.code]}
                  />
                ))}
                {total === 0 && <div className="muted small pad">—</div>}
              </div>
              {pageCount > 1 && (
                <div className="pager">
                  <button
                    type="button"
                    disabled={page <= 1}
                    onClick={() => setPage(state, page - 1)}
                    aria-label={`Previous page of ${state}`}
                  >
                    ‹
                  </button>
                  <span className="muted small">
                    {page} / {pageCount}
                  </span>
                  <button
                    type="button"
                    disabled={page >= pageCount}
                    onClick={() => setPage(state, page + 1)}
                    aria-label={`Next page of ${state}`}
                  >
                    ›
                  </button>
                </div>
              )}
            </section>
          );
        })}
      </div>

      {otherStates.length > 0 && (
        <div className="other-states">
          <span className="muted small">Other states:</span>
          {otherStates.map((s) => (
            <Link
              key={s}
              to={`/p/${encodeURIComponent(project)}/state/${encodeURIComponent(s)}`}
              className={`statelink${isNoOp(s) ? " noop" : ""}`}
              title={isNoOp(s) ? "no-op state" : "non-displayed state"}
            >
              {s} <b>{countIn(s)}</b>
            </Link>
          ))}
        </div>
      )}

      <ActivityPanel project={project} tick={tick} />
    </div>
  );
}

/** Recent activity (from the project's changelog), refreshed on each live tick. */
function ActivityPanel({ project, tick }: { project: string; tick: number }) {
  const { data } = useAsync(() => api.getActivity(project), [project, tick]);
  if (!data || data.length === 0) return null;
  const when = (iso: string) => {
    const d = new Date(iso);
    return isNaN(d.getTime()) ? iso : d.toLocaleString();
  };
  return (
    <section className="activity">
      <h2>Recent activity</h2>
      <ul className="activity-list">
        {data.slice(0, 12).map((a, i) => (
          <li key={i}>
            <span className="activity-msg">{a.message}</span>
            <span className="activity-meta">
              {a.actor} · {when(a.time)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
