import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { Feature, ProjectConfig } from "../types";

/**
 * Releases (FEAT-120, FEAT-123): each release's planned scope, how much of it is finished, its
 * target, and — once cut — its notes. Only for projects that use releases; the tab is not offered
 * elsewhere.
 */
export default function ReleasesPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const board = useAsync(() => api.getProject(project), [project, tick]);
  const releases = useAsync(() => api.getReleases(project), [project, tick]);

  if ((board.loading && !board.data) || (releases.loading && !releases.data)) return <Loading />;
  if (board.error) return <ErrorBox error={board.error} />;
  if (releases.error) return <ErrorBox error={releases.error} />;
  if (!board.data) return null;

  const config = board.data.config;
  if (!config.cadence?.releases) {
    return (
      <div className="releases-page">
        <h1>Releases</h1>
        <p className="muted">
          This project does not use releases. Switch them on with{" "}
          <code>kanbanr config cadence --releases on</code>.
        </p>
      </div>
    );
  }
  const list = releases.data ?? [];
  const base = `/p/${encodeURIComponent(project)}`;

  return (
    <div className="releases-page">
      <div className="page-head">
        <h1>Releases</h1>
        <LiveDot />
      </div>
      {list.length === 0 ? (
        <p className="muted">
          No releases yet — <code>kanbanr release add v0.1.0 --target YYYY-MM-DD</code>.
        </p>
      ) : (
        <div className="tiles">
          {list.map((r) => {
            const planned = board.data!.features.filter(
              (f) => f.release === r.version || (r.shipped ?? []).includes(f.code),
            );
            const finished = planned.filter((f) => isFinished(config, f)).length;
            const pct = planned.length ? Math.round((finished / planned.length) * 100) : 0;
            return (
              <div className="tile" key={r.version}>
                <div className="tile-title">
                  {r.version} {r.name ? `· ${r.name}` : ""}{" "}
                  <span className={`chip ${r.state === "shipped" ? "done" : ""}`}>{r.state}</span>
                </div>
                <div className="tile-states">
                  {r.target ? <span className="chip">target {r.target}</span> : null}
                  {r.shipped_at ? <span className="chip">shipped {r.shipped_at.slice(0, 10)}</span> : null}
                  <span className="chip tasks">
                    {finished}/{planned.length} finished ({pct}%)
                  </span>
                  {(r.carried ?? []).length > 0 ? (
                    <span className="chip warn" title={(r.carried ?? []).map((c) => `${c.code} → ${c.to}`).join(", ")}>
                      {(r.carried ?? []).length} carried over
                    </span>
                  ) : null}
                </div>
                <ul className="dep-list">
                  {planned.map((f) => (
                    <li key={f.code}>
                      <Link to={`${base}/feature/${encodeURIComponent(f.code)}`}>{f.code}</Link> {f.title}{" "}
                      <span className="muted small">({f.status})</span>
                    </li>
                  ))}
                </ul>
                {r.notes_doc ? (
                  <p className="small">
                    <Link to={`${base}/docs/file/${r.notes_doc.split("/").map(encodeURIComponent).join("/")}`}>
                      Release notes
                    </Link>
                  </p>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Finished: at an end status that is not a parking one. */
function isFinished(config: ProjectConfig, f: Feature): boolean {
  const terminal = config.terminal_states?.length
    ? config.terminal_states
    : config.statuses.filter((s) => s.toLowerCase() === "completed");
  return terminal.includes(f.status) && !(config.no_op_states ?? []).includes(f.status);
}
