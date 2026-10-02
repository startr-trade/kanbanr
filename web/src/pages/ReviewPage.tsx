import { useState } from "react";
import { useParams } from "react-router-dom";
import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { zachmanColumns } from "../types";
import type { Adr, PendingReview } from "../types";

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
  // Decisions waiting for their owner (FEAT-153): proposed, and nothing else.
  const adrs = useAsync(() => api.getAdrs(project), [project, tick]);

  if (pending.loading && !pending.data) return <Loading />;
  if (pending.error) return <ErrorBox error={pending.error} />;
  const items = pending.data ?? [];
  const proposed = (adrs.data ?? []).filter((a) => a.status.toLowerCase() === "proposed");
  // Who a yes here is recorded as (FEAT-077). The board's commit identity, exactly as
  // `kanbanr approve` defaults it — not the string "reviewed in the monitor", which named the place
  // the click happened and left twenty-eight approvals on this board attributable to nobody.
  const who = api.approver(meta.data ?? null);
  const writable = meta.data?.writes === true && who != null;

  // One verdict per item, and which one follows from how it got here: work built under a recorded
  // bypass is ratified — agreed after the fact, and recorded as such (FEAT-109) — anything else is
  // approved before the work.
  const agree = async (item: PendingReview) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      if (item.approval === "unratified") {
        await api.ratify(project, code, who);
        setDone((d) => ({ ...d, [code]: "ratified" }));
      } else {
        await api.approve(project, code, who);
        setDone((d) => ({ ...d, [code]: "approved" }));
      }
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A verdict on a proposed decision (FEAT-153), recorded as the same identity.
  const decide = async (adr: Adr, verdict: "accept" | "reject", reason = "") => {
    if (!who) return;
    setBusy(adr.id);
    try {
      await api.decideAdr(project, adr.id, verdict, who, reason);
      setDone((d) => ({ ...d, [adr.id]: verdict === "accept" ? "accepted" : "rejected" }));
    } catch (e) {
      setDone((d) => ({ ...d, [adr.id]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A named sign-off the item's next stage is waiting on (FEAT-117), recorded as the same identity.
  const signoff = async (item: PendingReview, name: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      await api.signoff(project, code, name, who);
      setDone((d) => ({ ...d, [code]: `signed off ${name}` }));
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

      {items.length === 0 && proposed.length > 0 ? (
        <p className="muted">No definitions are waiting for agreement.</p>
      ) : items.length === 0 ? (
        <p className="muted">
          Nothing is waiting for agreement. An item appears here when its definition has never been
          approved, or when the definition changed after a yes — which lapses the approval rather
          than silently keeping it.
        </p>
      ) : (
        <p className="muted">
          {items.length} item{items.length === 1 ? "" : "s"} waiting. Approving records agreement
          pinned to the definition below: change it afterwards and the approval lapses.
          {items.some((i) => i.approval === "unratified")
            ? " Work already finished under a recorded bypass is listed first, to ratify."
            : null}
          {meta.data?.writes !== true ? (
            <>
              {" "}
              This monitor is <strong>read-only</strong> — restart it with{" "}
              <code>kanbanr review --ui</code> to approve from here, or run{" "}
              <code>kanbanr approve &lt;CODE&gt;</code>.
            </>
          ) : who == null ? (
            <>
              {" "}
              This board has <strong>no commit identity</strong>, so an approval could not say who
              gave it — and one that cannot is not evidence of agreement. Set one with{" "}
              <code>kanbanr identity --name "You" --email you@example.com</code>, then reload.
            </>
          ) : (
            <>
              {" "}
              A yes is recorded as <strong>{who}</strong>.
            </>
          )}
        </p>
      )}

      {items.map((item, i) => (
        <Brief
          key={item.code}
          project={project}
          item={item}
          writable={writable}
          busy={busy === item.code}
          outcome={done[item.code]}
          // The first is open so the page shows what a brief looks like; the rest are collapsed,
          // because a queue of twelve read as one unbroken column (FEAT-076).
          defaultOpen={i === 0}
          onApprove={() => agree(item)}
          onSignoff={(name) => signoff(item, name)}
        />
      ))}

      {proposed.length > 0 && (
        <>
          <h2>Decisions</h2>
          <p className="muted">
            {proposed.length} architecture decision{proposed.length === 1 ? "" : "s"} proposed and
            waiting for a verdict. Accepting needs every section answered; rejecting needs a
            reason, which stays on the record.
          </p>
          {proposed.map((adr) => (
            <DecisionBrief
              key={adr.id}
              project={project}
              adr={adr}
              writable={writable}
              busy={busy === adr.id}
              outcome={done[adr.id]}
              onDecide={(verdict, reason) => decide(adr, verdict, reason)}
            />
          ))}
        </>
      )}
    </div>
  );
}

/** One proposed decision: its text, what rests on it, and the two verdicts (FEAT-153). */
function DecisionBrief({
  project,
  adr,
  writable,
  busy,
  outcome,
  onDecide,
}: {
  project: string;
  adr: Adr;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  onDecide: (verdict: "accept" | "reject", reason?: string) => void;
}) {
  const [reason, setReason] = useState("");
  const decided = outcome === "accepted" || outcome === "rejected";
  const missing = adr.missing ?? [];
  // The title is the summary's; the body below starts after it.
  const body = (adr.body ?? "").replace(/^#\s[^\n]*\n/, "");

  return (
    <details className="review-item" id={adr.id}>
      <summary>
        <code className="taskkey">{adr.id}</code>
        <span className="review-title">{adr.title}</span>
        <span className={`chip ${missing.length > 0 ? "warn" : ""}`}>
          {missing.length > 0 ? `unanswered: ${missing.join(", ")}` : "proposed"}
        </span>
        {decided ? <span className="chip done">{outcome}</span> : null}
      </summary>

      <div className="review-body">
        {adr.path ? (
          <p className="muted small">
            <Link
              to={`/p/${encodeURIComponent(project)}/docs/file/${adr.path.split("/").map(encodeURIComponent).join("/")}`}
            >
              open {adr.path}
            </Link>
            {adr.date ? ` · proposed ${adr.date}` : null}
          </p>
        ) : null}
        {(adr.affects ?? []).length > 0 && (
          <div className="tile-states">
            {(adr.affects ?? []).map((code) => (
              <span className="chip tasks" key={code}>
                affects {code}
              </span>
            ))}
          </div>
        )}
        <Markdown source={body} />

        <div className="review-actions">
          {decided ? (
            <span className="chip done">{outcome} — it leaves this list on the next refresh</span>
          ) : writable ? (
            <>
              <button
                className="btn btn-primary"
                onClick={() => onDecide("accept")}
                disabled={busy || missing.length > 0}
                title={missing.length > 0 ? `Answer ${missing.join(", ")} first` : undefined}
              >
                {busy ? "recording…" : `Accept ${adr.id}`}
              </button>
              <input
                className="review-reason"
                placeholder="Why reject it?"
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                aria-label={`Reason for rejecting ${adr.id}`}
              />
              <button
                className="btn"
                onClick={() => onDecide("reject", reason)}
                disabled={busy || reason.trim() === ""}
              >
                {busy ? "recording…" : `Reject ${adr.id}`}
              </button>
            </>
          ) : (
            <code>
              kanbanr adr accept {adr.id} · kanbanr adr reject {adr.id} --reason "…"
            </code>
          )}
          {outcome && !decided ? <span className="chip warn">{outcome}</span> : null}
        </div>
      </div>
    </details>
  );
}

function Brief({
  project,
  item,
  writable,
  busy,
  outcome,
  defaultOpen,
  onApprove,
  onSignoff,
}: {
  project: string;
  item: PendingReview;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  defaultOpen: boolean;
  onApprove: () => void;
  onSignoff: (name: string) => void;
}) {
  const def = item.definition ?? {};
  const requirements = def.requirements ?? [];
  const dimensions = zachmanColumns(def.zachman).filter((d) => d.answer.trim());
  const ratify = item.approval === "unratified";
  const waitingOnSignoff = item.approval === "signoff";
  const verb = ratify ? "Ratify" : "Approve";
  const approved =
    outcome === "approved" || outcome === "ratified" || (outcome ?? "").startsWith("signed off");
  const signoffs = item.signoffs_needed ?? [];

  return (
    <details className="review-item" id={item.code} open={defaultOpen}>
      {/* The summary is the row you scan; the brief is what you expand to read. */}
      <summary>
        <code className="taskkey">{item.code}</code>
        <span className="review-title">{item.title}</span>
        <span className={`chip ${item.approval === "missing" ? "" : "warn"}`}>
          {ratify
            ? `${item.status} without agreement`
            : waitingOnSignoff
              ? `waiting on sign-off: ${signoffs.join(", ")}`
              : item.approval === "lapsed"
                ? "approval lapsed"
                : "never approved"}
        </span>
        {approved ? <span className="chip done">{outcome}</span> : null}
      </summary>

      <div className="review-body">
      <p className="muted small">
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(item.code)}`}>
          open {item.code}
        </Link>
        {item.approval === "lapsed"
          ? " · approved once, then the definition changed — agreeing again covers what it says now"
          : ratify
            ? " · built under a recorded bypass — ratifying agrees to it after the fact, and the record says so"
            : null}
      </p>

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

      {/* The action lives in the body, not the summary: approving should follow reading. */}
      <div className="review-actions">
        {approved ? (
          <span className="chip done">{outcome} — it leaves this list on the next refresh</span>
        ) : writable ? (
          <>
            {waitingOnSignoff ? null : (
              <button className="btn btn-primary" onClick={onApprove} disabled={busy}>
                {busy ? "recording…" : `${verb} ${item.code}`}
              </button>
            )}
            {signoffs.map((name) => (
              <button
                key={name}
                className={`btn ${waitingOnSignoff ? "btn-primary" : ""}`}
                onClick={() => onSignoff(name)}
                disabled={busy}
              >
                {busy ? "recording…" : `Sign off ${name}`}
              </button>
            ))}
          </>
        ) : (
          <code>
            {waitingOnSignoff
              ? signoffs.map((n) => `kanbanr signoff ${item.code} ${n}`).join("  ·  ")
              : `kanbanr ${verb.toLowerCase()} ${item.code}`}
          </code>
        )}
        {outcome && !approved ? <span className="chip warn">{outcome}</span> : null}
      </div>
      </div>
    </details>
  );
}
