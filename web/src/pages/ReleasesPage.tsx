import { useCallback, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, viaOrigin } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { Release } from "../types";
import { unshownFor } from "../prepared";
import { ReleaseButton } from "./releases/ReleaseActions";
import { ReleaseNotesSection } from "./releases/ReleaseNotesSection";
import { PreparedNotRequired } from "./releases/ReleaseBody";

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
  // What Claude prepared for these decisions (FTR-034): CI results and confirmations after the tag.
  const prepared = useAsync(() => api.getPrepared(project), [project, tick]);
  // Promote (FEAT-185): which release is being promoted, and what came of it.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});
  // The prepared records each release's forms already show (DEF-095), by version, as the release's
  // button reports them; the rest are listed under "Prepared, not required".
  const [shown, setShown] = useState<Record<string, string[]>>({});
  const onShown = useCallback(
    (version: string, ids: string[]) =>
      setShown((s) => (s[version]?.join(",") === ids.join(",") ? s : { ...s, [version]: ids })),
    [],
  );

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
    // A superseded release never ships: its tag stays, and the patch carries on (FTR-026).
    if (r.state === "shipped" || r.superseded_by) return null;
    const at = r.stage ? stages.indexOf(r.stage) : -1;
    // Tagged at the last stage, it waits for its tag workflow's result and ships on it (DEF-049):
    // the stage it is at is the one the button still speaks for.
    if (r.tagged_at && at >= 0 && at + 1 === stages.length) return stages[at];
    return at + 1 < stages.length ? stages[at + 1] : null;
  };
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183), carrying
  // the release tip the plan showed (FEAT-212).
  const promote = async (r: Release, to: string, expect: string, note = "", acceptUntested?: string[]) => {
    if (!who) return;
    setBusy(r.version);
    try {
      const done = (await api.promoteRelease(project, r.version, who, to, expect, note, acceptUntested)) as {
        next?: string;
      };
      // A stage taken by mistake is stepped back from the CLI, by the person (DEF-034): the page
      // offers no Undo button, so the chip names the command.
      const undo = to ? ` — undo with \`kanbanr release undo ${r.version} --reason …\` while nothing is pushed` : "";
      setOutcome((o) => ({ ...o, [r.version]: `${to ? `promoted to ${to}` : `promoted: ${r.version} shipped`}${done?.next ? ` — ${done.next}` : ""}${undo}` }));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({
        ...o,
        [r.version]: `${message} — from the project folder: kanbanr release promote ${r.version} ${to}${note ? ` --note "${note}"` : ""}`,
      }));
    } finally {
      setBusy(null);
    }
  };

  // Offered only for a release that holds nothing and has not started (FTR-022): the same rule the
  // daemon applies, so the button never offers what would be refused.
  const removable = (r: Release) =>
    r.state === "planned" &&
    !r.stage &&
    !r.branch &&
    !r.notes_doc &&
    (r.items ?? []).length === 0 &&
    !list.some((o) => o.parent === r.version);
  const remove = async (r: Release) => {
    if (!window.confirm(`Remove release ${r.version}? It holds nothing.`)) return;
    setBusy(r.version);
    try {
      await api.removeRelease(project, r.version);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({ ...o, [r.version]: `${message} — from the project folder: kanbanr release remove ${r.version}` }));
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
                            .map((c) => `${c.stage} · ${c.by} · ${c.at.slice(0, 10)}${viaOrigin(c.origin)}${c.reason ? ` — ${c.reason}` : ""}`)
                            .join("\n")
                        : "not yet promoted"
                    }
                  >
                    {r.stage ? `stage ${r.stage}` : "not yet a candidate"}
                  </span>
                </div>
                <div className="tile-states">
                  {r.target ? <span className="chip">target {r.target}</span> : null}
                  {r.tagged_at && r.state !== "shipped" && !r.superseded_by ? (
                    <span className="chip warn">tagged, not released yet</span>
                  ) : null}
                  {r.superseded_by ? (
                    <span className="chip warn">Superseded by {r.superseded_by}</span>
                  ) : null}
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
                {writable && removable(r) ? (
                  <p>
                    <button className="btn" onClick={() => remove(r)} disabled={busy === r.version}>
                      Remove
                    </button>
                    {outcome[r.version] ? <span className="muted small"> {outcome[r.version]}</span> : null}
                  </p>
                ) : null}
                {next ? (
                  <ReleaseButton
                    project={project}
                    release={r}
                    tick={tick}
                    writable={writable}
                    gates={config.cadence?.commit_gates ?? []}
                    who={who}
                    busy={busy}
                    onPromote={promote}
                    outcome={outcome[r.version]}
                    prepared={prepared.data ?? []}
                    onShown={onShown}
                  />
                ) : null}
                {/* Until the button has said what its forms show, nothing is listed twice. */}
                {next && shown[r.version] === undefined ? null : (
                  <PreparedNotRequired
                    project={project}
                    views={unshownFor(prepared.data, r.version, new Set(next ? shown[r.version] : []))}
                    writable={writable}
                    who={who}
                  />
                )}
                {r.branch && config.cadence?.release_prepare?.notes ? (
                  <ReleaseNotesSection
                    project={project}
                    release={r}
                    notesPath={config.cadence.release_prepare.notes}
                    tick={tick}
                    writable={writable}
                    who={who}
                  />
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
