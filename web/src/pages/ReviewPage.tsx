import { useState } from "react";
import { useParams } from "react-router-dom";
import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { zachmanColumns } from "../types";
import type { PendingReview } from "../types";

/**
 * What is waiting for agreement, and the one action that gives it (FEAT-067).
 *
 * The approval gate exists to stop work being built on reasoning nobody agreed to. Its own premise
 * is that agreeing has to be **cheap** — a gate that costs a page of terminal output per item gets
 * escaped instead of used, which is exactly what happened here. So this page is the brief, rendered
 * for a reader, with the button beside it.
 *
 * The daemon decides what is pending and what "agreed" means; this page only renders and posts. It
 * offers the button only when the daemon accepts writes, because a button that fails on click is
 * worse than no button.
 */
export default function ReviewPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const [done, setDone] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const meta = useAsync(() => api.getMeta(), []);
  const pending = useAsync(() => api.getPendingReviews(project), [project, tick]);

  if (pending.loading && !pending.data) return <Loading />;
  if (pending.error) return <ErrorBox error={pending.error} />;
  const items = pending.data ?? [];
  const writable = meta.data?.writes === true;

  const approve = async (code: string) => {
    setBusy(code);
    try {
      await api.approve(project, code, "reviewed in the monitor");
      setDone((d) => ({ ...d, [code]: "approved" }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="review-page">
      <div className="page-head">
        <h1>Review</h1>
        <LiveDot />
      </div>

      {items.length === 0 ? (
        <p className="muted">
          Nothing is waiting for agreement. An item appears here when its definition has never been
          approved, or when the definition changed after a yes — which lapses the approval rather
          than silently keeping it.
        </p>
      ) : (
        <p className="muted">
          {items.length} item{items.length === 1 ? "" : "s"} waiting. Approving records agreement
          pinned to the definition below: change it afterwards and the approval lapses.
          {!writable && (
            <>
              {" "}
              This monitor is <strong>read-only</strong> — restart it with{" "}
              <code>kanbanr review --ui</code> to approve from here, or run{" "}
              <code>kanbanr approve &lt;CODE&gt;</code>.
            </>
          )}
        </p>
      )}

      {items.map((item) => (
        <Brief
          key={item.code}
          project={project}
          item={item}
          writable={writable}
          busy={busy === item.code}
          outcome={done[item.code]}
          onApprove={() => approve(item.code)}
        />
      ))}
    </div>
  );
}

function Brief({
  project,
  item,
  writable,
  busy,
  outcome,
  onApprove,
}: {
  project: string;
  item: PendingReview;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  onApprove: () => void;
}) {
  const def = item.definition ?? {};
  const requirements = def.requirements ?? [];
  const dimensions = zachmanColumns(def.zachman).filter((d) => d.answer.trim());
  const approved = outcome === "approved";

  return (
    <section className="section" id={item.code}>
      <h2>
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(item.code)}`}>
          <code className="taskkey">{item.code}</code>
        </Link>{" "}
        {item.title}
        <span className={`chip ${item.approval === "lapsed" ? "warn" : ""}`} style={{ marginLeft: 8 }}>
          {item.approval === "lapsed" ? "approval lapsed — the definition changed" : "never approved"}
        </span>
      </h2>

      {def.statement ? <Markdown source={`> ${def.statement}`} /> : null}
      {(def.goals ?? []).length > 0 && (
        <div className="tile-states">
          {(def.goals ?? []).map((g) => (
            <span className="chip tasks" key={g}>
              serves {g}
            </span>
          ))}
        </div>
      )}
      {item.started_unapproved?.trim() ? (
        <p className="muted small">
          Started without approval: {item.started_unapproved}
        </p>
      ) : null}

      {dimensions.length > 0 && (
        <ul className="dep-list">
          {dimensions.map((d) => (
            <li key={d.column}>
              <strong>{d.column}.</strong> {d.answer}
            </li>
          ))}
        </ul>
      )}

      {requirements.length > 0 && (
        <div className="tiles">
          {requirements.map((r) => (
            <div className="tile" key={r.id}>
              <div className="tile-title">
                <code className="taskkey">{r.id}</code> {r.text}
              </div>
              {r.scenario?.measure ? (
                <div className="tile-desc">Measure: {r.scenario.measure}</div>
              ) : null}
              <div className="tile-states">
                <span className="chip">{r.kind ?? "functional"}</span>
                {(r.iso25010 ?? []).map((q) => (
                  <span className="chip" key={q}>
                    {q}
                  </span>
                ))}
                {(r.tests ?? []).map((t) => (
                  <span className={`chip ${t.state === "green" ? "tasks" : "warn"}`} key={t.name}>
                    {t.state} · {t.name}
                  </span>
                ))}
                {(r.tests ?? []).length === 0 && <span className="chip warn">no test</span>}
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="tile-states" style={{ marginTop: 12 }}>
        {approved ? (
          <span className="chip tasks">approved — it will leave this list on the next refresh</span>
        ) : writable ? (
          <button className="chip" onClick={onApprove} disabled={busy}>
            {busy ? "recording…" : `Approve ${item.code}`}
          </button>
        ) : (
          <code>kanbanr approve {item.code}</code>
        )}
        {outcome && !approved ? <span className="chip warn">{outcome}</span> : null}
      </div>
    </section>
  );
}
