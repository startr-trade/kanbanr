import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { Link } from "react-router-dom";
import { alreadySaid, api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { zachmanColumns } from "../types";
import type { Adr, DriftChecked, DriftFlag, PendingReview, PlannedRelease, SpecBrief } from "../types";
import { approveBlocker, checkBlocks, type DiagramProblem } from "../diagramCheck";
import { PreparedPanel } from "../components/PreparedPanel";
import { openFor } from "../prepared";
import { lastResultLine, manualCommand, manualRows, type ManualRow } from "../manualTests";
import type { PreparedView } from "../types";

/**
 * What to show once a verdict went through. A repeat of the latest verdict records nothing; the
 * answer then says who recorded it first and from where, and that replaces "approved" (DEF-028).
 */
function alreadyOr(answer: unknown, done: string): string {
  const said = alreadySaid(answer);
  return said ? `already ${said}` : done;
}

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
/** "planned into v1", and, when a sprint runs that the item is not in, the command that plans it in (DEF-029). */
function plannedText(version: string, code: string, planned: PlannedRelease): string {
  const out = planned.not_in_sprint?.find((o) => o.code === code);
  return out
    ? `planned into ${version}. ${code} is in no sprint and ${out.sprint} is running: kanbanr sprint plan ${out.sprint} ${code}`
    : `planned into ${version}`;
}

export default function ReviewPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project), 400);
  const [done, setDone] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const meta = useAsync(() => api.getMeta(), []);
  const pending = useAsync(() => api.getPendingReviews(project), [project, tick]);
  // What Claude prepared for these decisions (FTR-034), shown filled in beside their buttons.
  const prepared = useAsync(() => api.getPrepared(project), [project, tick]);
  // Decisions waiting for their owner (FEAT-153): proposed, and nothing else.
  const adrs = useAsync(() => api.getAdrs(project), [project, tick]);
  // Manual tests waiting for a person (DEF-040): recorded here, not in the terminal.
  const manual = useAsync(() => api.getManualTests(project), [project, tick]);

  if (pending.loading && !pending.data) return <Loading />;
  if (pending.error) return <ErrorBox error={pending.error} />;
  const items = pending.data ?? [];
  const proposed = (adrs.data ?? []).filter((a) => a.status.toLowerCase() === "proposed");
  const manualWaiting = manual.data ?? [];
  // Who a yes here is recorded as (FEAT-077). The board's commit identity, exactly as
  // `kanbanr approve` defaults it — not the string "reviewed in the monitor", which named the place
  // the click happened and left twenty-eight approvals on this board attributable to nobody.
  const who = api.approver(meta.data ?? null);
  // A monitor configured read-only (the standalone bundle, FEAT-178) offers no decision at all; each
  // brief shows the CLI command that records it instead.
  const writable = !isReadOnly() && meta.data?.writes === true && who != null;

  // One verdict per item, and which one follows from how it got here: work built under a recorded
  // bypass is ratified — agreed after the fact, and recorded as such (FEAT-109) — anything else is
  // approved before the work.
  const agree = async (item: PendingReview) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      if (item.approval === "unratified") {
        // Pinned to the definition this page showed (FEAT-159): if it changed meanwhile, the
        // daemon refuses rather than recording a yes to text nobody read.
        const answer = await api.ratify(project, code, who, item.rev ?? "");
        setDone((d) => ({ ...d, [code]: alreadyOr(answer, "ratified") }));
      } else {
        const answer = await api.approve(project, code, who, item.rev ?? "");
        setDone((d) => ({ ...d, [code]: alreadyOr(answer, "approved") }));
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
  const signoff = async (item: PendingReview, name: string, checked: DriftChecked[] = []) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      const answer = await api.signoff(project, code, name, who, item.rev ?? "", item.spec_rev ?? "", checked);
      setDone((d) => ({ ...d, [code]: alreadyOr(answer, `signed off ${name}`) }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A record Claude prepared, confirmed as the same identity (FTR-034): the server makes the
  // verdict it prepares, pinned to the definition and spec this page showed.
  const confirmPrepared = async (item: PendingReview, view: PreparedView, edits: Record<string, unknown>, done: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      const answer = await api.confirmPrepared(project, view, who, edits, { rev: item.rev ?? "", spec_rev: item.spec_rev ?? "" });
      setDone((d) => ({ ...d, [code]: alreadyOr(answer, done) }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const rejectPrepared = async (item: PendingReview, view: PreparedView, reason: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      await api.rejectPrepared(project, view.id ?? "", who, reason);
      setDone((d) => ({ ...d, [code]: "rejected what Claude prepared; ask Claude to prepare it again" }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A manual test's result, recorded as the person who checked it (DEF-040): the route
  // `kanbanr test` uses, so the board's record is the same.
  const recordManual = async (m: ManualRow, passed: boolean, note: string) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.recordManual(project, m.row.code, m.row.requirement, m.row.test, passed, who, note);
      setDone((d) => ({ ...d, [key]: passed ? "recorded as passed" : "recorded as failed" }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const confirmManual = async (m: ManualRow, view: PreparedView, edits: Record<string, unknown>) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.confirmPrepared(project, view, who, edits);
      setDone((d) => ({ ...d, [key]: "recorded" }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const rejectManual = async (m: ManualRow, view: PreparedView, reason: string) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.rejectPrepared(project, view.id ?? "", who, reason);
      setDone((d) => ({ ...d, [key]: "rejected what Claude prepared" }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // Which release a ready item ships in (FEAT-185): the same plan route `kanbanr release plan` uses.
  const planRelease = async (item: PendingReview, version: string) => {
    const code = item.code;
    setBusy(code);
    try {
      const planned = await api.planIntoRelease(project, version, code);
      setDone((d) => ({ ...d, [code]: plannedText(version, code, planned) }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A release that does not exist yet: added, as `kanbanr release add` adds it, then planned into.
  const newRelease = async (item: PendingReview, version: string, target: string) => {
    const code = item.code;
    setBusy(code);
    try {
      await api.addRelease(project, version, target);
      const planned = await api.planIntoRelease(project, version, code);
      setDone((d) => ({ ...d, [code]: plannedText(version, code, planned) }));
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

      {items.length === 0 && (proposed.length > 0 || manualWaiting.length > 0) ? (
        <p className="muted">No definitions are waiting for agreement.</p>
      ) : items.length === 0 ? (
        <p className="muted">
          Nothing is waiting for agreement. An item appears here when its definition has never been
          approved, or when approved text was changed or removed, which lapses the approval, or when
          text was added once the item is being built.
        </p>
      ) : (
        <p className="muted">
          {items.length} item{items.length === 1 ? "" : "s"} waiting. Approving records agreement
          pinned to the definition below: change or remove approved text afterwards and the approval
          lapses; added text is reviewed at the next decision.
          {items.some((i) => i.approval === "unratified")
            ? " Work already finished under a recorded bypass is listed first, to ratify."
            : null}
          {items.some((i) => i.finished)
            ? " Finished work whose definition changed after approval is listed last."
            : null}
          {isReadOnly() ? (
            <>
              {" "}
              This copy of the monitor is <strong>read-only</strong> — record a verdict in Claude
              Code, or run <code>kanbanr approve &lt;CODE&gt;</code>.
            </>
          ) : meta.data?.writes !== true ? (
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
          tick={tick}
          writable={writable}
          busy={busy === item.code}
          outcome={done[item.code]}
          // The first is open so the page shows what a brief looks like; the rest are collapsed,
          // because a queue of twelve read as one unbroken column (FEAT-076).
          defaultOpen={i === 0}
          onApprove={() => agree(item)}
          onSignoff={(name, checked) => signoff(item, name, checked)}
          prepared={prepared.data ?? []}
          onConfirmPrepared={(view, edits, done) => confirmPrepared(item, view, edits, done)}
          onRejectPrepared={(view, reason) => rejectPrepared(item, view, reason)}
          onRelease={(version) => planRelease(item, version)}
          onNewRelease={(version, target) => newRelease(item, version, target)}
        />
      ))}

      {manualWaiting.length > 0 && (
        <ManualChecks
          project={project}
          rows={manualRows(manualWaiting, prepared.data)}
          writable={writable}
          busy={busy}
          done={done}
          onRecord={recordManual}
          onConfirm={confirmManual}
          onReject={rejectManual}
        />
      )}

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

const manualKey = (m: ManualRow) => `manual:${m.row.code}:${m.row.requirement}:${m.row.test}`;

/**
 * The manual tests waiting for a person (DEF-040): each with its requirement and the check to make,
 * and Passed / Failed for the person who did it. Nothing has to be typed; a note is behind a link.
 * Where the monitor cannot write, the command for the person's own terminal is shown instead.
 */
function ManualChecks({
  project,
  rows,
  writable,
  busy,
  done,
  onRecord,
  onConfirm,
  onReject,
}: {
  project: string;
  rows: ManualRow[];
  writable: boolean;
  busy: string | null;
  done: Record<string, string>;
  onRecord: (m: ManualRow, passed: boolean, note: string) => void;
  onConfirm: (m: ManualRow, view: PreparedView, edits: Record<string, unknown>) => void;
  onReject: (m: ManualRow, view: PreparedView, reason: string) => void;
}) {
  return (
    <>
      <h2>Manual checks</h2>
      <p className="muted">
        {rows.length} manual check{rows.length === 1 ? "" : "s"} waiting for someone to do them. Do the
        check, then record whether it passed.
      </p>
      {rows.map((m) => (
        <ManualCheck
          key={manualKey(m)}
          project={project}
          m={m}
          writable={writable}
          busy={busy === manualKey(m)}
          outcome={done[manualKey(m)]}
          onRecord={(passed, note) => onRecord(m, passed, note)}
          onConfirm={(view, edits) => onConfirm(m, view, edits)}
          onReject={(view, reason) => onReject(m, view, reason)}
        />
      ))}
    </>
  );
}

function ManualCheck({
  project,
  m,
  writable,
  busy,
  outcome,
  onRecord,
  onConfirm,
  onReject,
}: {
  project: string;
  m: ManualRow;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  onRecord: (passed: boolean, note: string) => void;
  onConfirm: (view: PreparedView, edits: Record<string, unknown>) => void;
  onReject: (view: PreparedView, reason: string) => void;
}) {
  const { row, prepared } = m;
  const [noting, setNoting] = useState(false);
  const [note, setNote] = useState("");
  const last = lastResultLine(row.last);
  const recorded = outcome != null && outcome.startsWith("recorded");
  const failed = row.state === "red";
  return (
    <div className="review-item manual-check">
      <p>
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(row.code)}`}>
          <code className="taskkey">{row.code}</code> {row.title}
        </Link>{" "}
        <code className="taskkey">{row.requirement}</code>{" "}
        <span className={`chip ${failed ? "warn" : ""}`}>{failed ? "failed" : "not run"}</span>
      </p>
      <p className="small">{row.requirement_text}</p>
      <p>
        <strong>Check:</strong> {row.test}
      </p>
      {last ? <p className="small muted">Last result: {last}.</p> : null}
      {failed ? (
        <p className="small muted">It is recorded as failed. To have a defect filed, ask Claude to file one.</p>
      ) : null}
      {recorded ? (
        <span className="chip done">{outcome} — it leaves this list on the next refresh</span>
      ) : prepared && writable ? (
        <PreparedPanel
          project={project}
          view={prepared}
          writable={writable}
          busy={busy}
          label={prepared.fields?.state === "red" ? "Record failed" : "Record passed"}
          onConfirm={(edits) => onConfirm(prepared, edits)}
          onReject={(reason) => onReject(prepared, reason)}
        />
      ) : writable ? (
        <>
          <div className="review-actions">
            <button className="btn btn-primary" disabled={busy} onClick={() => onRecord(true, note.trim())}>
              Passed
            </button>
            <button className="btn" disabled={busy} onClick={() => onRecord(false, note.trim())}>
              Failed
            </button>
            {noting ? null : (
              <a
                href="#add-a-note"
                className="small"
                onClick={(e) => {
                  e.preventDefault();
                  setNoting(true);
                }}
              >
                Add a note
              </a>
            )}
          </div>
          {noting ? (
            <p className="small">
              <label>
                Note (optional){" "}
                <input value={note} onChange={(e) => setNote(e.target.value)} maxLength={4000} aria-label="Note" />
              </label>
            </p>
          ) : null}
        </>
      ) : (
        <p className="small">
          To record it in your own terminal: <code>{manualCommand(row, true)}</code> (or <code>red</code>)
        </p>
      )}
      {outcome && !recorded ? <p className="small warn">{outcome}</p> : null}
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

/**
 * Pele's parse of every diagram an item's brief covers — its spec and each doc in `affects_docs`
 * (FEAT-200) — re-run on the live tick, so fixing the spec re-enables Approve without a reload.
 */
function useDiagramCheck(project: string, code: string, tick: number) {
  const diagrams = useAsync(() => api.getItemDiagrams(project, code), [project, code, tick]);
  const [parsed, setParsed] = useState<{ problems: DiagramProblem[]; failed: string | null; done: boolean }>({
    problems: [],
    failed: null,
    done: false,
  });
  useEffect(() => {
    const d = diagrams.data;
    if (diagrams.error) {
      setParsed({ problems: [], failed: diagrams.error, done: true });
      return;
    }
    if (!d) return;
    setParsed((p) => ({ ...p, done: false }));
    let cancelled = false;
    import("pele")
      .then(({ parse }) => {
        if (!cancelled) setParsed({ problems: checkBlocks(d, parse), failed: null, done: true });
      })
      .catch((e) => {
        if (!cancelled) setParsed({ problems: [], failed: String(e), done: true });
      });
    return () => {
      cancelled = true;
    };
  }, [diagrams.data, diagrams.error]);
  // Fails closed: a refetch in flight counts as loading, so a spec broken since the last check is
  // never approved on the strength of the check before it.
  const loading = diagrams.loading || !parsed.done;
  return { ...parsed, loading };
}

/** Where a diagram problem's source is: the item for its spec, the doc page for a doc. */
function problemHref(project: string, code: string, p: DiagramProblem): string {
  const enc = encodeURIComponent;
  return p.doc
    ? `/p/${enc(project)}/docs/file/${p.doc.split("/").map(enc).join("/")}`
    : `/p/${enc(project)}/feature/${enc(code)}`;
}

/** Context kept around each change, and the shortest run of unchanged lines worth folding (FTR-023). */
const DIFF_CONTEXT = 2;

type DiffRow = { op: "same" | "added" | "removed" | "fold"; text: string };

/** The changed lines with two lines of context; each longer unchanged run folds to a count. */
function foldDiff(diff: SpecBrief["diff"]): DiffRow[] {
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < diff.length) {
    if (diff[i].op !== "same") {
      rows.push(diff[i]);
      i++;
      continue;
    }
    let j = i;
    while (j < diff.length && diff[j].op === "same") j++;
    const run = diff.slice(i, j);
    const head = i === 0 ? 0 : DIFF_CONTEXT;
    const tail = j === diff.length ? 0 : DIFF_CONTEXT;
    if (run.length > 4) {
      rows.push(...run.slice(0, head));
      const hidden = run.length - head - tail;
      rows.push({ op: "fold", text: `… ${hidden} unchanged ${hidden === 1 ? "line" : "lines"}` });
      rows.push(...run.slice(run.length - tail));
    } else {
      rows.push(...run);
    }
    i = j;
  }
  return rows;
}

/** The spec a sign-off pins: only what changed since the last sign-off, the whole spec on request. */
function SpecDiff({ spec, changed }: { spec: SpecBrief; changed: boolean }) {
  const whole = <Markdown source={spec.text} />;
  if (spec.diff.length > 0) {
    return (
      <div className="spec-diff">
        <h3>The spec, changed since sign-off</h3>
        <pre className="diff">
          {foldDiff(spec.diff).map((r, n) => (
            <div key={n} className={`diff-${r.op}`} style={{ color: r.op === "added" ? "var(--ok, #1a7f37)" : r.op === "removed" ? "var(--bad, #cf222e)" : undefined }}>
              {r.op === "added" ? "+ " : r.op === "removed" ? "− " : "  "}
              {r.text}
            </div>
          ))}
        </pre>
        <details>
          <summary>Show the whole spec</summary>
          {whole}
        </details>
      </div>
    );
  }
  return (
    <div className="spec-diff">
      <h3>The spec this sign-off covers</h3>
      {changed && !spec.previous_kept ? (
        <p className="muted small">
          The text signed off on {spec.signed_at.slice(0, 10)} wasn't kept on this board (it was signed before
          kanbanr kept it), so the whole spec is shown.
        </p>
      ) : null}
      {whole}
    </div>
  );
}

function Brief({
  project,
  item,
  tick,
  writable,
  busy,
  outcome,
  defaultOpen,
  onApprove,
  onSignoff,
  onRelease,
  onNewRelease,
  prepared,
  onConfirmPrepared,
  onRejectPrepared,
}: {
  project: string;
  item: PendingReview;
  tick: number;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  defaultOpen: boolean;
  onApprove: () => void;
  onSignoff: (name: string, checked?: DriftChecked[]) => void;
  onRelease: (version: string) => void;
  onNewRelease: (version: string, target: string) => void;
  prepared: PreparedView[];
  onConfirmPrepared: (view: PreparedView, edits: Record<string, unknown>, done: string) => void;
  onRejectPrepared: (view: PreparedView, reason: string) => void;
}) {
  const [adding, setAdding] = useState(false);
  const [version, setVersion] = useState("");
  const [target, setTarget] = useState("");
  const def = item.definition ?? {};
  const requirements = def.requirements ?? [];
  const dimensions = zachmanColumns(def.zachman).filter((d) => d.answer.trim());
  const ratify = item.approval === "unratified";
  const waitingOnSignoff = item.approval === "signoff";
  // The design was signed off, then the spec changed (FTR-023): signing again is the answer.
  const specChanged = item.approval === "spec_changed";
  // Ready for release and in none (FEAT-185); asked beside any other question this item has.
  const releaseChoices = item.release_choices;
  const whichRelease = item.approval === "release";
  const verb = ratify ? "Ratify" : "Approve";
  const approved =
    outcome === "approved" ||
    outcome === "ratified" ||
    (outcome ?? "").startsWith("already ") ||
    (outcome ?? "").startsWith("signed off") ||
    (outcome ?? "").startsWith("planned into");
  const signoffs = item.signoffs_needed ?? [];
  // The texts that name what this item changed (FTR-013): each is settled before docs-folded can
  // be given, by "still right" or by the code of a correction Claude filed.
  const flags: DriftFlag[] = signoffs.includes("docs-folded") ? (item.to_check ?? []) : [];
  const [settled, setSettled] = useState<Record<string, { confirmed: boolean; correction: string }>>({});
  const settledOf = (f: DriftFlag) => {
    const s = settled[driftKey(f)];
    return s?.confirmed ? "confirmed" : s?.correction.trim() ? "corrected" : null;
  };
  const unsettled = flags.some((f) => settledOf(f) === null);
  const checkedFor = (): DriftChecked[] =>
    flags.map((f) => {
      const s = settled[driftKey(f)];
      return s?.confirmed
        ? { target: driftKey(f), outcome: "confirmed" }
        : { target: driftKey(f), outcome: "corrected", correction: s?.correction.trim() ?? "" };
    });
  const diagrams = useDiagramCheck(project, item.code, tick);
  const blocker = approveBlocker(diagrams);
  // What Claude prepared (FTR-034): a sign-off's note and doc, a ratification's reason. A
  // docs-folded with texts to settle keeps its form: what was checked travels with that button.
  const preparedSignoff = (name: string) =>
    name === "docs-folded" && flags.length ? undefined : openFor(prepared, "signoff", { item: item.code, signoff: name });
  const preparedRatify = ratify ? openFor(prepared, "ratification", { item: item.code }) : undefined;
  const panels = [
    ...(preparedRatify ? [{ view: preparedRatify, label: `Ratify ${item.code}`, done: "ratified" }] : []),
    ...signoffs.flatMap((name) => {
      const view = preparedSignoff(name);
      return view ? [{ view, label: `Sign off ${name}`, done: `signed off ${name}` }] : [];
    }),
  ];

  // Parts added or changed since the last decision lead; the rest is collapsed (FTR-018).
  const changes =
    item.changes && (item.changes.added.length > 0 || item.changes.changed.length > 0 || item.changes.removed.length > 0)
      ? item.changes
      : undefined;
  const definitionBody = (
    <>
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
    </>
  );

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
              : whichRelease
                ? "ready for release: which one?"
              : specChanged
                ? item.finished
                  ? `${item.status}: spec changed since sign-off`
                  : "spec changed since sign-off"
              : item.approval === "added"
                ? item.finished
                  ? `${item.status}: added since approval`
                  : "added since approval"
              : item.approval === "lapsed"
                ? item.finished
                  ? `${item.status}: approval lapsed`
                  : "approval lapsed"
                : "never approved"}
        </span>
        {approved ? <span className="chip done">{outcome}</span> : null}
      </summary>

      <div className="review-body">
      <p className="muted small">
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(item.code)}`}>
          open {item.code}
        </Link>
        {specChanged
          ? ` · signed off by ${item.spec?.signed_by ?? ""} on ${(item.spec?.signed_at ?? "").slice(0, 10)}; since then the spec changed. Signing again covers it as it stands now.`
          : item.approval === "added"
          ? " · approved once; since then text was only added. Approving again covers what it says now."
          : item.approval === "lapsed"
          ? ` · approved once, then the definition changed — agreeing again covers what it says now${
              item.finished ? ". The work is done." : ""
            }`
          : ratify
            ? " · built under a recorded bypass — ratifying agrees to it after the fact, and the record says so"
            : null}
      </p>

      {changes ? (
        <Changes
          item={item}
          changes={changes}
          heading={
            waitingOnSignoff
              ? "Also reviewed by this sign-off: added since approval"
              : "Since your last decision"
          }
        />
      ) : null}
      {changes ? (
        <details className="review-unchanged">
          <summary>The approved text, unchanged</summary>
          {definitionBody}
        </details>
      ) : (
        definitionBody
      )}

      {item.spec ? <SpecDiff spec={item.spec} changed={specChanged} /> : null}

      {diagrams.failed || diagrams.problems.length > 0 ? (
        <div className="diagram-problems">
          <h3>Diagrams</h3>
          {diagrams.failed ? (
            <p className="diagram-error">The diagram check could not run: {diagrams.failed}</p>
          ) : (
            <ul>
              {diagrams.problems.map((p) => (
                <li key={`${p.file}#${p.block}`}>
                  <Link to={problemHref(project, item.code, p)}>{p.file}</Link>, diagram #{p.block}, line{" "}
                  {p.line}: {p.message}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {/* The action lives in the body, not the summary: approving should follow reading. */}
      {item.changes_no_documents && signoffs.includes("docs-folded") ? (
        <p>This item changes no documents; signing off confirms that.</p>
      ) : null}
      {flags.length && !writable ? (
        <p>Give docs-folded on a monitor that takes decisions: it records what you checked beside each text.</p>
      ) : null}
      {signoffs.includes("docs-folded") && item.drift_skipped ? (
        <p className="drift-skipped">Drift check skipped: {item.drift_skipped}.</p>
      ) : null}
      {flags.length ? (
        <div className="drift-flags">
          <h4>Texts that name what this item changed</h4>
          <p>
            Each of these names something {item.code} changed and has not changed since. Read it, then say it is
            still right, or ask Claude to file a correction and enter its code.
          </p>
          <ul>
            {flags.map((f) => {
              const key = driftKey(f);
              const s = settled[key] ?? { confirmed: false, correction: "" };
              return (
                <li key={key}>
                  <Link to={driftHref(project, f)}>{f.target.title || f.target.id}</Link> names{" "}
                  {f.hits.map((h, i) => (
                    <span key={i}>
                      {i ? "; " : ""}
                      <code>{h.name.text}</code> (line {h.lines.join(", ")})
                    </span>
                  ))}
                  {f.commits[0] ? (
                    <span>
                      {" "}
                      — changed by {f.commits[0].item}, {f.commits[0].sha.slice(0, 7)}
                    </span>
                  ) : null}
                  {writable ? (
                    <div className="drift-choice">
                      <label>
                        <input
                          type="checkbox"
                          checked={s.confirmed}
                          onChange={(e) => setSettled({ ...settled, [key]: { ...s, confirmed: e.target.checked } })}
                        />{" "}
                        Still right
                      </label>
                      <label>
                        File a correction: ask Claude to file one, then enter its code{" "}
                        <input
                          type="text"
                          value={s.correction}
                          placeholder="DEF-…"
                          disabled={s.confirmed}
                          onChange={(e) => setSettled({ ...settled, [key]: { ...s, correction: e.target.value } })}
                        />
                      </label>
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </div>
      ) : null}
      {!approved
        ? panels.map((p) => (
            <PreparedPanel
              key={p.view.id}
              project={project}
              view={p.view}
              writable={writable}
              busy={busy}
              label={p.label}
              onConfirm={(edits) => onConfirmPrepared(p.view, edits, p.done)}
              onReject={(reason) => onRejectPrepared(p.view, reason)}
            />
          ))
        : null}
      <div className="review-actions">
        {approved ? (
          <span className="chip done">{outcome} — it leaves this list on the next refresh</span>
        ) : writable ? (
          <>
            {waitingOnSignoff || specChanged || whichRelease || preparedRatify ? null : (
              <button
                className="btn btn-primary"
                onClick={onApprove}
                disabled={busy || blocker != null}
                title={blocker ?? undefined}
              >
                {busy ? "recording…" : `${verb} ${item.code}`}
              </button>
            )}
            {signoffs.filter((name) => !preparedSignoff(name)).map((name) => (
              <button
                key={name}
                className={`btn ${waitingOnSignoff || specChanged ? "btn-primary" : ""}`}
                onClick={() => onSignoff(name, name === "docs-folded" && flags.length ? checkedFor() : undefined)}
                disabled={busy || (name === "docs-folded" && unsettled)}
                title={name === "docs-folded" && unsettled ? "Settle each text above first" : undefined}
              >
                {busy ? "recording…" : `Sign off ${name}`}
              </button>
            ))}
            {releaseChoices
              ? releaseChoices.map((v) => (
                  <button
                    key={v}
                    className={`btn ${whichRelease ? "btn-primary" : ""}`}
                    onClick={() => onRelease(v)}
                    disabled={busy}
                  >
                    {busy ? "recording…" : `In release ${v}`}
                  </button>
                ))
              : null}
            {releaseChoices && !adding ? (
              <button className="btn" onClick={() => setAdding(true)} disabled={busy}>
                New release…
              </button>
            ) : null}
            {releaseChoices && adding ? (
              <>
                <input
                  className="review-reason"
                  placeholder="Version, e.g. v0.2.0"
                  value={version}
                  onChange={(e) => setVersion(e.target.value)}
                  aria-label={`Version of the new release for ${item.code}`}
                />
                <input
                  className="review-reason"
                  type="date"
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  aria-label={`Target date of the new release for ${item.code}`}
                />
                <button
                  className="btn btn-primary"
                  onClick={() => onNewRelease(version.trim(), target)}
                  disabled={busy || version.trim() === ""}
                >
                  {busy ? "recording…" : `Add ${version.trim() || "release"} with ${item.code}`}
                </button>
                <button className="btn" onClick={() => setAdding(false)} disabled={busy}>
                  Cancel
                </button>
              </>
            ) : null}
          </>
        ) : (
          <code>
            {[
              ...(waitingOnSignoff
                ? signoffs
                    .filter((n) => !(n === "docs-folded" && flags.length > 0))
                    .map((n) => `kanbanr signoff ${item.code} ${n}`)
                : whichRelease
                  ? []
                  : [`kanbanr ${verb.toLowerCase()} ${item.code}`]),
              ...(releaseChoices
                ? [
                    releaseChoices.length > 0
                      ? `kanbanr release plan ${releaseChoices.join("|")} ${item.code}`
                      : `kanbanr release add <VERSION> --target YYYY-MM-DD && kanbanr release plan <VERSION> ${item.code}`,
                  ]
                : []),
            ].join("  ·  ")}
          </code>
        )}
        {outcome && !approved ? <span className="chip warn">{outcome}</span> : null}
      </div>
      </div>
    </details>
  );
}

/** The text of one definition part, by its key (FTR-018): `statement`, `goals/<id>`,
 * `zachman/<column>`, `requirements/<id>` and `requirements/<id>/tests/<name>`. */
function partText(def: NonNullable<PendingReview["definition"]>, key: string): string {
  if (key === "statement") return def.statement ?? "";
  if (key.startsWith("goals/")) return key.slice("goals/".length);
  if (key.startsWith("zachman/")) {
    const column = key.slice("zachman/".length);
    const found = zachmanColumns(def.zachman).find((d) => d.column.toLowerCase() === column);
    return found ? `${found.column}. ${found.answer}` : column;
  }
  if (key.startsWith("requirements/")) {
    const rest = key.slice("requirements/".length);
    // A test's name may hold "/", so split on the tests marker and never on every slash.
    const at = rest.indexOf("/tests/");
    const id = at < 0 ? rest : rest.slice(0, at);
    const requirement = (def.requirements ?? []).find((r, i) => (r.id || `#${i}`) === id);
    if (at < 0) return requirement ? `${requirement.id} ${requirement.text}` : id;
    const name = rest.slice(at + "/tests/".length);
    const test = (requirement?.tests ?? []).find((t) => t.name === name);
    return test ? `${id} test ${test.name}${test.kind ? ` (${test.kind})` : ""}` : name;
  }
  return key;
}

/** What was added or changed since the last decision, first on the brief (FTR-018). */
function Changes({
  item,
  changes,
  heading,
}: {
  item: PendingReview;
  changes: NonNullable<PendingReview["changes"]>;
  heading: string;
}) {
  const def = item.definition ?? {};
  return (
    <div className="review-changes">
      <h3>{heading}</h3>
      <ul className="dep-list">
        {changes.added.map((k) => (
          <li key={`a-${k}`}>
            <span className="chip tasks">added</span> {partText(def, k)}
          </li>
        ))}
        {changes.changed.map((k) => (
          <li key={`c-${k}`}>
            <span className="chip warn">changed</span> {partText(def, k)}
          </li>
        ))}
        {changes.removed.map((k) => (
          <li key={`r-${k}`}>
            <code>{k}</code> removed
          </li>
        ))}
      </ul>
    </div>
  );
}

/** How a flag is named in a sign-off: `doc:<path>`, `adr:<id>` or `item:<CODE>` (FTR-013). */
function driftKey(f: DriftFlag): string {
  return `${f.target.kind}:${f.target.id}`;
}

/** Where a flagged text is read: a board doc or an item. */
function driftHref(project: string, f: DriftFlag): string {
  return f.target.kind === "item"
    ? `/p/${project}/feature/${f.target.id}`
    : `/p/${project}/docs/file/${f.target.path ?? f.target.id}`;
}
