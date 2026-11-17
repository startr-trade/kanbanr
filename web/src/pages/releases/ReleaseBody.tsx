import { useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../../api";
import type { PreparedView, Release, ReleaseItemRow } from "../../types";
import { PreparedPanel } from "../../components/PreparedPanel";
import { useQuery } from "../../data/useQuery";
import { urls } from "../../data/urls";
import { DataTable, ErrorState, LoadingState, filterRows, type Column } from "../../ui";
import { ITEM_COLUMNS } from "../../releaseTabs";
import { to as link } from "../../overviewLinks";
import { unshownFor } from "../../prepared";
import { ReleaseButton } from "./ReleaseActions";
import { ReleaseNotesSection } from "./ReleaseNotesSection";

/**
 * What every release row of a tab shares (FTR-041): who may act, the configured stages and gates,
 * the promotion in flight and its outcome, Claude's prepared records, and what each button shows.
 */
export interface ReleaseCtx {
  project: string;
  stages: string[];
  gates: string[];
  /** The notes path `release prepare` writes, when the project sets one. */
  notesPath: string;
  writable: boolean;
  who: string | null;
  busy: string | null;
  outcome: Record<string, string>;
  onPromote: (r: Release, to: string, expect: string, note?: string, acceptUntested?: string[]) => void;
  removable: (r: Release) => boolean;
  onRemove: (r: Release) => void;
  prepared: PreparedView[];
  /** The prepared records each release's forms show (DEF-095), by version, once its button says. */
  shown: Record<string, string[]>;
  onShown: (version: string, ids: string[]) => void;
  /** Bumped on every live change, for the reads not yet in the shared cache (the notes). */
  tick: number;
  /** The tab's filter text (`?q=`), given to every open table. */
  q: string;
}

/** The item table's columns, its ID linked to the item. */
function itemColumns(project: string): Column<ReleaseItemRow>[] {
  return ITEM_COLUMNS.map((c) =>
    c.key === "code" ? { ...c, render: (r: ReleaseItemRow) => <Link to={link.item(project, r.code)}>{r.code}</Link> } : c,
  );
}

/**
 * An open release's body (FTR-041, R-4): the full-width table of its items, read now that it is
 * open, then the rest of its button (the tag panel, what the step does, the cut preview, the
 * untested acceptance, Needs, Check again), the prepared records no form shows, its notes and the
 * link to its notes doc. Rendered only inside its AccordionRow's `children`.
 */
export function ReleaseBody({
  ctx,
  release: r,
  next,
  accepted,
  onAccepted,
}: {
  ctx: ReleaseCtx;
  release: Release;
  /** The stage its button speaks for; null once it has none (shipped or superseded). */
  next: string | null;
  accepted: boolean;
  onAccepted: (accepted: boolean) => void;
}) {
  const { project } = ctx;
  const items = useQuery<ReleaseItemRow[]>(project, urls.releaseItems(project, r.version), ["releases", "items"]);
  const base = `/p/${encodeURIComponent(project)}`;
  const columns = itemColumns(project);
  return (
    <div className="release-body">
      {items.data ? (
        <DataTable
          rows={filterRows(items.data, ITEM_COLUMNS, ctx.q)}
          columns={columns}
          rowKey={(row) => row.code}
          stateKey="t."
          csvName={`${r.version}-items`}
          caption={`Items in ${r.version}`}
          empty={ctx.q ? `No item in ${r.version} matches "${ctx.q}".` : `No items in ${r.version} yet.`}
        />
      ) : items.error ? (
        <ErrorState error={items.error} retry={items.reload} />
      ) : (
        <LoadingState />
      )}
      {ctx.writable && ctx.removable(r) ? (
        <p>
          <button className="btn" onClick={() => ctx.onRemove(r)} disabled={ctx.busy === r.version}>
            Remove
          </button>
          {ctx.outcome[r.version] ? <span className="muted small"> {ctx.outcome[r.version]}</span> : null}
        </p>
      ) : null}
      {next ? (
        <ReleaseButton
          project={project}
          release={r}
          writable={ctx.writable}
          gates={ctx.gates}
          who={ctx.who}
          busy={ctx.busy}
          onPromote={ctx.onPromote}
          outcome={ctx.outcome[r.version]}
          prepared={ctx.prepared}
          onShown={ctx.onShown}
          variant="body"
          accepted={accepted}
          onAccepted={onAccepted}
        />
      ) : null}
      {/* Until the button has said what its forms show, nothing is listed twice. */}
      {next && ctx.shown[r.version] === undefined ? null : (
        <PreparedNotRequired
          project={project}
          views={unshownFor(ctx.prepared, r.version, new Set(next ? ctx.shown[r.version] : []))}
          writable={ctx.writable}
          who={ctx.who}
        />
      )}
      {r.branch && ctx.notesPath ? (
        <ReleaseNotesSection
          project={project}
          release={r}
          notesPath={ctx.notesPath}
          tick={ctx.tick}
          writable={ctx.writable}
          who={ctx.who}
        />
      ) : null}
      {r.notes_doc ? (
        <p className="small">
          <Link to={`${base}/docs/file/${r.notes_doc.split("/").map(encodeURIComponent).join("/")}`}>Release notes</Link>
        </p>
      ) : null}
    </div>
  );
}

/**
 * The open prepared records about a release that no form on its entry shows (DEF-095): a gate
 * result for a gate no stage asks for, a promotion for a stage that is not the plan's next. Each is
 * shown in its own PreparedPanel, confirmed or rejected there, so nothing waits unseen. On a
 * read-only monitor the panel names the command.
 */
export function PreparedNotRequired({
  project,
  views,
  writable,
  who,
}: {
  project: string;
  views: PreparedView[];
  writable: boolean;
  who: string | null;
}) {
  const [said, setSaid] = useState<Record<string, { ok: boolean; text: string }>>({});
  if (!views.length) return null;
  const answer = async (id: string, go: () => Promise<unknown>, text: string) => {
    try {
      await go();
      setSaid((s) => ({ ...s, [id]: { ok: true, text } }));
    } catch (e) {
      setSaid((s) => ({ ...s, [id]: { ok: false, text: e instanceof Error ? e.message : String(e) } }));
    }
  };
  return (
    <div className="small">
      <p>
        <strong>Prepared, not required</strong>
      </p>
      {views.map((view) => {
        const id = view.id ?? "";
        const gate = view.subject.gate ?? "";
        const isGate = view.kind === "gate-result";
        const passed = view.fields?.passed === true;
        const label = isGate
          ? `Record ${gate} ${passed ? "passed" : "failed"} on ${(view.subject.commit ?? "").slice(0, 7)}`
          : `Promote ${view.subject.release ?? view.release ?? ""} to ${view.subject.stage ?? ""}`;
        return (
          <div key={id}>
            {isGate ? (
              <p className="muted">
                {(view.required_by ?? []).length ? `asked for by ${(view.required_by ?? []).join(", ")}` : `no stage asks for ${gate}`}
              </p>
            ) : null}
            <PreparedPanel
              project={project}
              view={view}
              writable={writable && who != null}
              busy={false}
              label={label}
              onConfirm={(edits) => who && answer(id, () => api.confirmPrepared(project, view, who, edits), `${label}: done`)}
              onReject={(reason) =>
                who && answer(id, () => api.rejectPrepared(project, id, who, reason), "rejected what Claude prepared")
              }
            />
            {said[id] ? <span className={`chip ${said[id].ok ? "done" : "warn"}`}>{said[id].text}</span> : null}
          </div>
        );
      })}
    </div>
  );
}
