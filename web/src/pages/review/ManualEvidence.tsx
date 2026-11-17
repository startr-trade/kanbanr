/**
 * A manual check's evidence on the Review page (FTR-038): the item, the requirement and the check,
 * its last result, and how to record it. A check not yet due (DEF-076) offers no result. A result
 * Claude prepared is confirmed or rejected in its panel; with nothing prepared the check is the
 * person's own observation, so Passed and Failed stay here in the row and never in the group's
 * header (R-11).
 */
import { useState } from "react";
import { Link } from "react-router-dom";
import { PreparedPanel } from "../../components/PreparedPanel";
import { lastResultLine, manualCommand, notDueLine, type ManualRow, type Outcome } from "../../manualTests";
import type { PreparedView } from "../../types";
import { OutcomeChip } from "./EntryEvidence";

export function ManualEvidence({
  project,
  m,
  writable,
  busy,
  outcome,
  onRetry,
  onRecord,
  onConfirm,
  onReject,
}: {
  project: string;
  m: ManualRow;
  writable: boolean;
  busy: boolean;
  outcome?: Outcome;
  onRetry: () => void;
  onRecord: (passed: boolean, note: string) => void;
  onConfirm: (view: PreparedView, edits: Record<string, unknown>) => Promise<void>;
  onReject: (view: PreparedView, reason: string) => Promise<void>;
}) {
  const { row, prepared } = m;
  const [noting, setNoting] = useState(false);
  const [note, setNote] = useState("");
  const last = lastResultLine(row.last);
  // Before its item reaches its build stage a check offers no result (DEF-076).
  const notDue = notDueLine(row);
  // Any outcome, a rejection or an error included, replaces the form (DEF-090).
  const finished = outcome != null;
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
      {finished ? (
        <OutcomeChip outcome={outcome} onRetry={onRetry} />
      ) : notDue ? (
        <p className="small muted">{notDue}</p>
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
    </div>
  );
}
