/**
 * The `system-baseline` group's row on the Review page (FTR-006): the acceptance card. It reads the
 * baseline (`GET …/catalog/baseline`) for the row alone, lists each system row with where it stands,
 * lets the person edit the note of a prepared acceptance, and accepts it as they saw it (`rev` is the
 * pins' hash this card showed, so a source that changed since is refused). A read-only monitor shows
 * the command in place of the button. The pure model is `systemBaselineView.ts`; never a page-level panel.
 */
import { useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../../api";
import { AskFirst } from "../../components/AskFirst";
import { TAGS, urls } from "../../data/urls";
import { useQuery } from "../../data/useQuery";
import { catalogRowHref, cardFor } from "../../systemBaselineView";
import { ErrorState, LoadingState } from "../../ui";
import type { SystemBaselineReview } from "../../types";

export function SystemBaselineEvidence({
  project,
  writable,
  who,
  onDecided,
}: {
  project: string;
  writable: boolean;
  who: string | null;
  onDecided?(): void;
}) {
  const q = useQuery<SystemBaselineReview>(project, urls.systemBaseline(project), TAGS.systemBaseline);
  const [note, setNote] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ ok: boolean; text: string } | null>(null);
  if (q.error && !q.data) return <ErrorState error={`The system baseline could not be read: ${q.error}`} retry={q.reload} />;
  if (!q.data) return <LoadingState label="Reading the system baseline…" />;
  const card = cardFor(q.data.state, q.data.prepared, writable && who != null);
  if (!card) return <p className="small">Nothing is waiting to be accepted: the baseline is {q.data.state.kind.replace("_", " ")}.</p>;
  const text = note ?? card.note;
  const accept = async (rev: string) => {
    try {
      await api.acceptSystemBaseline(project, who ?? "", text, card.prepared, rev);
      setOutcome({ ok: true, text: "accepted" });
      onDecided?.();
    } catch (e) {
      setOutcome({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  };
  return (
    <div className="review-body">
      <p className="small">{card.body}</p>
      <ul className="small">
        {card.rows.map((r) => (
          <li key={r.row}>
            <Link to={catalogRowHref(project, r.row)}>{r.text}</Link>
          </li>
        ))}
      </ul>
      {card.action.kind === "button" && !outcome?.ok ? (
        <div className="review-actions">
          <label className="small">
            Note{" "}
            <input
              className="review-reason"
              type="text"
              value={text}
              placeholder="what you agreed (optional)"
              onChange={(e) => setNote(e.target.value)}
            />
          </label>{" "}
          <AskFirst
            label={card.action.label}
            ask="Accept the system baseline as shown?"
            className="btn btn-primary"
            onConfirm={() => accept((card.action as { rev: string }).rev)}
          />
        </div>
      ) : null}
      {card.action.kind === "command" ? (
        <p className="muted small">
          To accept it, run <code>{card.action.command}</code> in your own terminal.
        </p>
      ) : null}
      {card.action.kind === "blocked" ? <p className="muted small">{card.action.why}</p> : null}
      {outcome ? <span className={`chip ${outcome.ok ? "done" : "warn"}`}>{outcome.text}</span> : null}
    </div>
  );
}
