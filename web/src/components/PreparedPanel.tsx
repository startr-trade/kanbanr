// What Claude prepared for a decision, filled in beside the decision's one button (FTR-034). The
// person reads it, and confirms it, changes a field first, or rejects it; their click makes the
// verdict as them. Labels and fields are shown as text, never as markdown or HTML, and a source is
// a link only when sourceHref says it is one.
import { useState } from "react";
import { Link } from "react-router-dom";
import type { PreparedView } from "../types";
import { EDITABLE, fieldLabel, preparedLine, sourceHref } from "../prepared";

type Mode = "show" | "edit" | "reject";

function shown(value: unknown): string {
  if (value === true) return "passed";
  if (value === false) return "failed";
  if (value == null) return "";
  return String(value);
}

export function PreparedPanel({
  project,
  view,
  writable,
  busy,
  label,
  onConfirm,
  onReject,
}: {
  project: string;
  view: PreparedView;
  writable: boolean;
  busy: boolean;
  label: string;
  onConfirm: (edits: Record<string, unknown>) => void;
  onReject: (reason: string) => void;
}) {
  const kind = view.kind ?? "";
  const fields = view.fields ?? {};
  const editable = EDITABLE[kind] ?? [];
  const [mode, setMode] = useState<Mode>("show");
  const [edits, setEdits] = useState<Record<string, unknown>>({});
  const [reason, setReason] = useState("");
  const names = Array.from(new Set([...Object.keys(fields), ...(mode === "edit" ? editable : [])]));
  const value = (k: string) => (k in edits ? edits[k] : fields[k]);
  const set = (k: string, v: unknown) => setEdits((e) => ({ ...e, [k]: v }));
  const open = view.state === "open";

  return (
    <div className="prepared-panel">
      <p className="small">
        <span className="chip">prepared</span> {preparedLine(view)}
      </p>
      {mode !== "edit" ? (
        <dl className="small">
          {names.map((k) => (
            <div key={k}>
              <dt>{fieldLabel(kind, k)}</dt>
              <dd>{shown(fields[k]) || "—"}</dd>
            </div>
          ))}
        </dl>
      ) : null}
      {mode === "edit" ? (
        <div className="small">
          {editable.map((k) =>
            typeof fields[k] === "boolean" || k === "passed" ? (
              <p key={k}>
                <label>
                  {fieldLabel(kind, k)}{" "}
                  <select
                    value={value(k) === false ? "failed" : "passed"}
                    onChange={(e) => set(k, e.target.value === "passed")}
                    aria-label={fieldLabel(kind, k)}
                  >
                    <option value="passed">Passed</option>
                    <option value="failed">Failed</option>
                  </select>
                </label>
              </p>
            ) : (
              <p key={k}>
                <label>
                  {fieldLabel(kind, k)}{" "}
                  <input
                    value={shown(value(k))}
                    onChange={(e) => set(k, e.target.value)}
                    aria-label={fieldLabel(kind, k)}
                  />
                </label>
              </p>
            ),
          )}
        </div>
      ) : null}
      {view.sources?.length ? (
        <ul className="prepared-sources small">
          {view.sources.map((s, i) => {
            const to = sourceHref(project, s);
            const words = s.label || s.url || "";
            return (
              <li key={`${i}-${s.url ?? ""}`}>
                {to == null ? (
                  s.url?.startsWith("commit:") ? (
                    <>
                      {s.label && s.label !== "commit" ? `${s.label} ` : ""}
                      <code>{s.url.slice(7, 14)}</code>
                    </>
                  ) : (
                    words
                  )
                ) : to.external ? (
                  <a href={to.href} target="_blank" rel="noreferrer">
                    {words}
                  </a>
                ) : (
                  <Link to={to.href}>{words}</Link>
                )}
              </li>
            );
          })}
        </ul>
      ) : null}
      {!open ? (
        <p className="small">
          <span className="chip warn">{view.state.replace("_", " ")}</span> {view.stale_why}
        </p>
      ) : !writable ? (
        <p className="small">
          To take it in your own terminal: <code>{view.command}</code>
        </p>
      ) : (
        <>
          <p className="review-actions">
            <button
              className="btn btn-primary"
              disabled={busy || mode === "reject"}
              onClick={() => onConfirm(mode === "edit" ? edits : {})}
            >
              {busy ? "recording…" : label}
            </button>
            {mode === "show" ? (
              <>
                <button className="btn" disabled={busy} onClick={() => setMode("edit")}>
                  Edit
                </button>
                <button className="btn" disabled={busy} onClick={() => setMode("reject")}>
                  Reject
                </button>
              </>
            ) : (
              <button
                className="btn"
                disabled={busy}
                onClick={() => {
                  setMode("show");
                  setEdits({});
                  setReason("");
                }}
              >
                Cancel
              </button>
            )}
          </p>
          {mode === "reject" ? (
            <p className="small review-actions">
              <label>
                Why (optional){" "}
                <input value={reason} onChange={(e) => setReason(e.target.value)} aria-label="Why reject it" />
              </label>
              <button className="btn" disabled={busy} onClick={() => onReject(reason.trim())}>
                Reject
              </button>
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
