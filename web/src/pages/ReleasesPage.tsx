import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { Release } from "../types";

/** The stages a release moves through when the project names none (FEAT-183). */
const DEFAULT_STAGES = ["candidate", "test", "staging", "production"];

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
  const meta = useAsync(() => api.getMeta(), []);
  // Promote (FEAT-185): which release is being promoted, what came of it, and which is waiting on
  // the production confirmation.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});
  const [confirming, setConfirming] = useState<string | null>(null);

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
  const stages = config.cadence.release_stages?.length ? config.cadence.release_stages : DEFAULT_STAGES;
  const last = stages[stages.length - 1];
  // Recorded as the board's commit identity, as `kanbanr release promote` records it.
  const who = api.approver(meta.data ?? null);
  // A read-only copy of the monitor (FEAT-178) never offers a promotion, whatever the daemon says.
  const writable = !isReadOnly() && meta.data?.writes === true && who != null;
  // The stage after the one it is at; the first, before it has one. None once it is at the last.
  const nextStage = (r: Release) => {
    if (r.state === "shipped") return null;
    const at = r.stage ? stages.indexOf(r.stage) : -1;
    return at + 1 < stages.length ? stages[at + 1] : null;
  };
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183).
  const promote = async (r: Release, to: string) => {
    if (!who) return;
    setBusy(r.version);
    setConfirming(null);
    try {
      await api.promoteRelease(project, r.version, who, to);
      setOutcome((o) => ({ ...o, [r.version]: `promoted to ${to}` }));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({
        ...o,
        [r.version]: `${message} — from the project folder: kanbanr release promote ${r.version} ${to}`,
      }));
    } finally {
      setBusy(null);
    }
  };

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
            const holds = new Set(r.items ?? []);
            const planned = board.data!.features.filter((f) => holds.has(f.code));
            // Counted by the daemon, by the same rule as the sprint's (FEAT-138): this page used
            // to keep its own copy, which counted only the end status.
            const finished = r.finished ?? 0;
            const pct = planned.length ? Math.round((finished / planned.length) * 100) : 0;
            const next = nextStage(r);
            return (
              <div className="tile" key={r.version}>
                <div className="tile-title">
                  {r.version} {r.name ? `· ${r.name}` : ""}{" "}
                  <span className={`chip ${r.state === "shipped" ? "done" : ""}`}>{r.state}</span>{" "}
                  <span
                    className={`chip ${r.stage === last ? "done" : ""}`}
                    title={
                      (r.stages ?? []).length > 0
                        ? (r.stages ?? [])
                            .map((c) => `${c.stage} · ${c.by} · ${c.at.slice(0, 10)}${c.reason ? ` — ${c.reason}` : ""}`)
                            .join("\n")
                        : "not yet promoted"
                    }
                  >
                    {r.stage ? `stage ${r.stage}` : "not yet a candidate"}
                  </span>
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
                {next ? (
                  <div className="review-actions">
                    {!writable ? (
                      <code>kanbanr release promote {r.version}</code>
                    ) : confirming === r.version ? (
                      <div>
                        <p className="small">
                          <strong>Promoting {r.version} to {next} is the cut.</strong> It tags the code
                          repository <code>{r.version}</code> and fast-forwards main to{" "}
                          <code>release/{r.version}</code> (a patch of an older release is tagged
                          without moving main).
                        </p>
                        <CutPreviewList project={project} version={r.version} />
                        <div className="review-actions">
                          <button className="btn btn-primary" onClick={() => promote(r, next)} disabled={busy != null}>
                            {busy === r.version ? "promoting…" : `Cut ${r.version}`}
                          </button>
                          <button className="btn" onClick={() => setConfirming(null)} disabled={busy != null}>
                            Cancel
                          </button>
                        </div>
                      </div>
                    ) : (
                      <button
                        className={`btn ${next === last ? "" : "btn-primary"}`}
                        onClick={() => (next === last ? setConfirming(r.version) : promote(r, next))}
                        disabled={busy != null}
                        title={next === last ? "production is the cut: a tag and a merge to main" : undefined}
                      >
                        {busy === r.version ? "promoting…" : `Promote to ${next}${next === last ? " (cut)" : ""}`}
                      </button>
                    )}
                    {outcome[r.version] ? (
                      <span className={`chip ${outcome[r.version].startsWith("promoted") ? "done" : "warn"}`}>
                        {outcome[r.version]}
                      </span>
                    ) : null}
                  </div>
                ) : null}
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

/** Exactly what the cut would ship and carry over, by its own rule (FEAT-185). */
function CutPreviewList({ project, version }: { project: string; version: string }) {
  const preview = useAsync(() => api.getCutPreview(project, version), [project, version]);
  if (preview.error) return <ErrorBox error={preview.error} />;
  if (!preview.data) return <Loading />;
  const ships = preview.data.filter((p) => p.ships);
  const carries = preview.data.filter((p) => !p.ships);
  return (
    <div className="small">
      <p>
        Ships: {ships.length ? ships.map((p) => (p.to ? `${p.code} (→ ${p.to})` : p.code)).join(", ") : "nothing"}
      </p>
      {carries.length ? (
        <ul className="dep-list">
          {carries.map((p) => (
            <li key={p.code}>
              carries {p.code}: {p.why}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
