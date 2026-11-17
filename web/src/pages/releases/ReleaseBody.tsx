import { useState } from "react";
import { api } from "../../api";
import type { PreparedView } from "../../types";
import { PreparedPanel } from "../../components/PreparedPanel";

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
