import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { releaseAction } from "../releaseAction";
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
  // Promote (FEAT-185): which release is being promoted, and what came of it.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});

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
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183), carrying
  // the release tip the plan showed (FEAT-212).
  const promote = async (r: Release, to: string, expect: string) => {
    if (!who) return;
    setBusy(r.version);
    try {
      const done = (await api.promoteRelease(project, r.version, who, to, expect)) as { next?: string };
      setOutcome((o) => ({ ...o, [r.version]: `promoted to ${to}${done?.next ? ` — ${done.next}` : ""}` }));
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
                  <ReleaseButton
                    project={project}
                    release={r}
                    tick={tick}
                    writable={writable}
                    busy={busy}
                    onPromote={promote}
                    outcome={outcome[r.version]}
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

/**
 * A release's one button (FEAT-212): named for its next step, with what the step will do beside it,
 * the cut's preview inline, and — until the plan is ready — what is missing and how to fix it.
 */
function ReleaseButton({
  project,
  release,
  tick,
  writable,
  busy,
  onPromote,
  outcome,
}: {
  project: string;
  release: Release;
  tick: number;
  writable: boolean;
  busy: string | null;
  onPromote: (r: Release, to: string, expect: string) => void;
  outcome?: string;
}) {
  // "Check again" re-reads the plan: a `git push` updates the remote-tracking ref locally, so no
  // fetch is needed (the monitor never fetches).
  const [again, setAgain] = useState(0);
  // The last promotion's note says what came next ("push main…"); once the plan has been read
  // again it may no longer hold, so it is put away until the next promotion.
  const [hideOutcome, setHideOutcome] = useState(false);
  useEffect(() => setHideOutcome(false), [outcome]);
  const plan = useAsync(() => api.getReleasePlan(project, release.version), [project, release.version, tick, again]);
  if (plan.error) return <ErrorBox error={plan.error} />;
  if (!plan.data) return <Loading />;
  const action = releaseAction(plan.data, writable);
  if (action.note) {
    return (
      <p className="muted small">
        <Inline text={action.note} />
      </p>
    );
  }
  return (
    <div className="release-action">
      <div className="review-actions">
        {action.button ? (
          <button
            className="btn btn-primary"
            onClick={() => onPromote(release, plan.data!.to, plan.data!.release_tip)}
            disabled={!action.enabled || busy != null}
            title={action.enabled ? action.does.join("; ") : "not ready: see what is missing"}
          >
            {busy === release.version ? "promoting…" : action.label}
          </button>
        ) : (
          <code>{action.command}</code>
        )}
        {outcome && !hideOutcome ? (
          <span className={`chip ${outcome.startsWith("promoted") ? "done" : "warn"}`}>
            <Inline text={outcome} />
          </span>
        ) : null}
      </div>
      {action.does.length ? (
        <ul className="dep-list small">
          {action.does.map((d) => (
            <li key={d}>
              <Inline text={d} />
            </li>
          ))}
        </ul>
      ) : null}
      {action.preview ? <CutPreviewList preview={action.preview} /> : null}
      {action.missing.length ? (
        <div className="small">
          <p>
            <strong>Needs:</strong>
          </p>
          <ul className="dep-list">
            {action.missing.map((m) => (
              <li key={m.text}>
                <Inline text={m.text} />
                {m.fix ? (
                  <>
                    {" — "}
                    <Inline text={m.fix} />
                  </>
                ) : null}
              </li>
            ))}
          </ul>
          <button
            className="btn"
            onClick={() => {
              setHideOutcome(true);
              setAgain((n) => n + 1);
            }}
            disabled={plan.loading}
          >
            Check again
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** Exactly what the cut would tag, ship and carry over, from the plan (FEAT-185, FEAT-212). */
function CutPreviewList({
  preview,
}: {
  preview: { tag: string; ships: string[]; carries: { code: string; why: string }[] };
}) {
  return (
    <div className="small">
      {preview.tag ? (
        <p>
          Tags: <code>{preview.tag}</code>
        </p>
      ) : null}
      <p>Ships: {preview.ships.length ? preview.ships.join(", ") : "nothing"}</p>
      {preview.carries.length ? (
        <ul className="dep-list">
          {preview.carries.map((p) => (
            <li key={p.code}>
              carries {p.code}: {p.why}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** A message from the plan, with its `code` spans shown as code rather than as backticks. */
function Inline({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>))}
    </>
  );
}
