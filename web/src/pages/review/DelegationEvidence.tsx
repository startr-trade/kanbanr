/**
 * The evidence of a delegation row on the Review page (FTR-038), moved from Decisions here
 * (FTR-028): a delegation Claude proposed, shown as FTR-034's prepared panel with Grant and its
 * discard; or a decision Claude took under a delegation, with Keep and Undo and the optional
 * reason. Undo with a reason is taken here in the row (R-11); the group's header keeps or undoes
 * several at once, without one.
 */
import { useState } from "react";
import { api } from "../../api";
import { PreparedPanel } from "../../components/PreparedPanel";
import type { DelegatedDecision, DelegationView, PreparedView } from "../../types";

/** A proposed delegation as FTR-034's prepared panel shows a prepared record (FTR-028). */
function asPrepared(g: DelegationView): PreparedView {
  const fields: Record<string, unknown> = { verdicts: g.verdicts.join(", ") };
  if (g.steps?.length) fields.steps = g.steps.join(", ");
  if (g.items?.length) fields.items = g.items.join(", ");
  if (g.kinds?.length) fields.kinds = g.kinds.join(", ");
  if (g.milestone) fields.milestone = g.milestone;
  if (g.all) fields.covers = "every item";
  fields.until = g.until;
  if (g.note) fields.note = g.note;
  return {
    id: g.id,
    kind: "delegation",
    subject: {},
    fields,
    sources: [],
    by: g.proposed.by,
    origin: g.proposed.origin,
    at: g.proposed.at,
    rev: g.rev,
    state: "open",
    resolution: null,
    stale_why: "",
    page: "review",
    command: `kanbanr decisions grant ${g.id}`,
  };
}

export function DelegationEvidence({
  project,
  delegation,
  delegated,
  writable,
  who,
}: {
  project: string;
  /** A delegation Claude proposed (the `delegation` group). */
  delegation?: DelegationView | null;
  /** A decision Claude took under a delegation (the `delegated` group). */
  delegated?: DelegatedDecision | null;
  writable: boolean;
  who: string | null;
}) {
  const [said, setSaid] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const run = async (key: string, f: () => Promise<unknown>, ok: string) => {
    setBusy(key);
    try {
      await f();
      setSaid((d) => ({ ...d, [key]: ok }));
    } catch (e) {
      setSaid((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const by = who ?? "";
  const reason = (k: string) => reasons[k] ?? "";
  const setReason = (k: string, v: string) => setReasons((r) => ({ ...r, [k]: v }));

  if (delegation) {
    const g = delegation;
    return (
      <div className="decisions-here">
        <p className="small">
          <strong>{g.id}</strong> proposed
        </p>
        {said[g.id] ? (
          <p className="small">{said[g.id]}</p>
        ) : (
          <PreparedPanel
            project={project}
            view={asPrepared(g)}
            writable={writable}
            busy={busy === g.id}
            label="Grant"
            onConfirm={() => run(g.id, () => api.grantDelegation(project, g.id, g.rev, by), "granted")}
            onReject={(why) =>
              run(
                g.id,
                () => api.revokeDelegation(project, g.id, by, why || "discarded on the Review page"),
                "discarded",
              )
            }
          />
        )}
      </div>
    );
  }

  if (!delegated) return <p className="muted small">Nothing is recorded for this row.</p>;
  const d = delegated;
  return (
    <div className="decisions-here small">
      <p>
        <strong>{d.code}</strong> {d.title}: {d.verdict}
        {d.signoff ? ` ${d.signoff}` : ""}, under {d.delegation}, on {d.at.slice(0, 10)}
      </p>
      {said[d.decision] ? (
        <p>{said[d.decision]}</p>
      ) : writable ? (
        <p>
          <input
            aria-label={`Reason for ${d.decision}`}
            placeholder="reason (optional)"
            value={reason(d.decision)}
            onChange={(e) => setReason(d.decision, e.target.value)}
          />{" "}
          <button
            disabled={busy === d.decision}
            onClick={() =>
              run(d.decision, () => api.reviewDelegated(project, d.decision, "keep", by, reason(d.decision)), "kept")
            }
          >
            Keep
          </button>{" "}
          <button
            disabled={busy === d.decision}
            onClick={() =>
              run(d.decision, () => api.reviewDelegated(project, d.decision, "undo", by, reason(d.decision)), "undone")
            }
          >
            Undo
          </button>
        </p>
      ) : (
        <p className="muted">
          <code>kanbanr decisions review {d.decision} --keep</code> or <code>--undo</code>
        </p>
      )}
    </div>
  );
}
