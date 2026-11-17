import { useState } from "react";
import { Link } from "react-router-dom";
import Markdown from "../../components/Markdown";
import type { Outcome } from "../../manualTests";
import type { Adr } from "../../types";
import { OutcomeChip } from "./EntryEvidence";

/**
 * A proposed ADR's evidence on the Review page (FTR-038): its text, what rests on it, and the two
 * verdicts (FEAT-153). Accept needs every section answered; Reject needs a reason, one per ADR, so
 * it is taken here in the row and never from the group's header (R-11).
 */
export function AdrEvidence({
  project,
  adr,
  writable,
  busy,
  outcome,
  onRetry,
  onDecide,
}: {
  project: string;
  adr: Adr;
  writable: boolean;
  busy: boolean;
  outcome?: Outcome;
  onRetry: () => void;
  onDecide: (verdict: "accept" | "reject", reason?: string) => void;
}) {
  const [reason, setReason] = useState("");
  const decided = outcome != null;
  const missing = adr.missing ?? [];
  // The title is the row's; the body below starts after it.
  const body = (adr.body ?? "").replace(/^#\s[^\n]*\n/, "");

  return (
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
          <OutcomeChip outcome={outcome} onRetry={onRetry} />
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
      </div>
    </div>
  );
}
