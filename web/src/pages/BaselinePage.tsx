/**
 * The project's engineering baseline (FTR-033): the practice packs it follows, the revisions Claude
 * proposed for the person to accept, every area with its standing answer and the entries it
 * recorded, the charter's own constraints, and the history of accepted revisions. Accepting a
 * revision is the person's alone (R-3): FTR-034's prepared panel with Accept, or on a read-only
 * monitor the command for their own terminal. Texts are shown as text, never as markdown.
 */
import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync } from "../live";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { apiUrl, isReadOnly } from "../runtimeConfig";
import { useScrollToHash } from "../useBoardIndex";
import { LiveDot } from "../components/bits";
import { PreparedPanel } from "../components/PreparedPanel";
import { Empty, ErrorState, LoadingState, Section } from "../ui";
import { adrHref, answerText, areaRows, history, revisionAsPrepared } from "../baseline";
import type { BaselineArea, BaselineEntryView, BaselinePack, BaselineRevision, BaselineView, CompactIndex } from "../types";

/** The watcher's slices this page reads: the baseline and its packs, the charter, the items naming entries, the ADRs. */
const BASELINE_TAGS = ["baseline", "charter", "items", "adrs"] as const;

export default function BaselinePage() {
  const { project = "" } = useParams();
  const baseline = useQuery<BaselineView>(project, apiUrl(`/projects/${encodeURIComponent(project)}/baseline`), BASELINE_TAGS);
  // ADR paths come from the compact index, never the whole board (DEF-081).
  const index = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);
  const meta = useAsync(() => api.getMeta(), []);
  // Who an acceptance here is recorded as: the board's commit identity, as on the Review page.
  const who = api.approver(meta.data ?? null);
  // A read-only monitor (FEAT-178) offers no decision; the panel shows the command instead.
  const writable = !isReadOnly() && meta.data?.writes === true && who != null;
  useScrollToHash(!!baseline.data);

  if (baseline.loading && !baseline.data) return <LoadingState />;
  if (baseline.error && !baseline.data) return <ErrorState error={baseline.error} retry={baseline.reload} />;
  if (!baseline.data) return null;
  return (
    <BaselineBody
      project={project}
      view={baseline.data}
      adrs={index.data?.adrs ?? []}
      writable={writable}
      by={who}
      onChanged={baseline.reload}
    />
  );
}

/** The page without its reads, so web::baseline_page_shows_areas_and_accepts_a_revision can render it. */
export function BaselineBody({
  project,
  view,
  adrs,
  writable,
  by,
  onChanged,
}: {
  project: string;
  view: BaselineView;
  adrs: { id: string; path: string }[];
  writable: boolean;
  by: string | null;
  onChanged?: () => void;
}) {
  const [said, setSaid] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const run = async (id: string, f: () => Promise<unknown>, ok: string) => {
    setBusy(id);
    try {
      await f();
      setSaid((d) => ({ ...d, [id]: ok }));
      onChanged?.();
    } catch (e) {
      setSaid((d) => ({ ...d, [id]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const p = `/p/${encodeURIComponent(project)}`;
  const proposed = view.revisions.filter((r) => r.state === "proposed");
  const groups = areaRows(view);
  const accepted = history(view);
  const empty = view.packs.length === 0 && view.areas.length === 0 && view.revisions.length === 0;

  return (
    <div className="baseline-page">
      <div className="page-head">
        <h1>Baseline</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        The project's engineering baseline: the stack and engineering questions its practice packs ask, answered
        once, and what each answer records. Every design that touches an entry names it.
      </p>

      {empty ? (
        <Empty
          title="No baseline yet."
          ask="Ask Claude to set up the project's engineering baseline: which practice packs to follow and their questions."
        />
      ) : null}

      <Section title="Packs">
        {view.packs.length === 0 ? (
          <p className="muted small">The project follows no practice pack.</p>
        ) : (
          view.packs.map((pack) => <PackLine key={pack.name} pack={pack} />)
        )}
      </Section>

      <Section title="Proposed revisions">
        {proposed.length === 0 ? (
          <p className="muted small">No revision waits for you.</p>
        ) : (
          proposed.map((rev) => (
            <div className="section" id={rev.id} key={rev.id}>
              <h3>
                {rev.id}
                {rev.item ? (
                  <>
                    {" "}
                    <span className="muted small">
                      asked by <Link to={`${p}/feature/${encodeURIComponent(rev.item)}`}>{rev.item}</Link>'s design
                    </span>
                  </>
                ) : (
                  <span className="muted small"> asked at setup</span>
                )}
              </h3>
              {(rev.warnings ?? []).map((w) => (
                <p className="muted small" key={w}>
                  {w}
                </p>
              ))}
              {said[rev.id] ? (
                <p className="small">{said[rev.id]}</p>
              ) : (
                <PreparedPanel
                  project={project}
                  view={revisionAsPrepared(rev)}
                  writable={writable}
                  busy={busy === rev.id}
                  label="Accept"
                  onConfirm={() => run(rev.id, () => api.acceptBaseline(project, rev.id, rev.rev, by ?? ""), "accepted")}
                  onReject={(why) =>
                    run(
                      rev.id,
                      () => api.withdrawBaseline(project, rev.id, by ?? "", why || "withdrawn on the Baseline page"),
                      "withdrawn",
                    )
                  }
                />
              )}
            </div>
          ))
        )}
      </Section>

      <Section title="Areas">
        {groups.length === 0 ? (
          <p className="muted small">The followed packs ask nothing yet.</p>
        ) : (
          groups.map((g) => (
            <div className="section" key={g.group}>
              <h3>{g.title}</h3>
              {g.areas.map((a) => (
                <AreaBlock key={a.id} area={a} project={project} adrs={adrs} />
              ))}
            </div>
          ))
        )}
      </Section>

      <Section title="Also in the charter">
        {view.charter_constraints.length === 0 ? (
          <p className="muted small">
            The <Link to={`${p}/charter`}>charter</Link> lists no constraints.
          </p>
        ) : (
          <>
            <ul className="small">
              {view.charter_constraints.map((c) => (
                <li key={c}>{c}</li>
              ))}
            </ul>
            <p className="muted small">
              Read-only here: these stay in the <Link to={`${p}/charter`}>charter</Link> until it is edited.
            </p>
          </>
        )}
      </Section>

      <Section title="History">
        {accepted.length === 0 ? (
          <p className="muted small">No revision has been accepted yet.</p>
        ) : (
          <ul className="small">
            {accepted.map((r) => (
              <HistoryLine key={r.id} rev={r} />
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

/** One pack: name, version, library, and whether its library's copy has moved on (as the Workflow page's ProcessLine). */
function PackLine({ pack }: { pack: BaselinePack }) {
  const library = pack.library === "builtin" ? "built-in" : pack.library;
  const version = pack.version > 0 ? ` v${pack.version}` : "";
  const messages = pack.messages ?? [];
  return (
    <div className="section">
      <p className="small">
        <strong>{pack.name}</strong>
        {version} · {library}{" "}
        {pack.removed ? (
          <span className="chip warn">removed</span>
        ) : messages.length === 0 ? (
          <span className="chip done">current</span>
        ) : (
          <span className="chip warn">changed</span>
        )}
      </p>
      {messages.map((m) => (
        <p className="muted small" key={m}>
          {m.split("`").map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : part))}
        </p>
      ))}
    </div>
  );
}

/** One area: its title, pack and question, its answer, and the entries that answer recorded. */
function AreaBlock({ area, project, adrs }: { area: BaselineArea; project: string; adrs: { id: string; path: string }[] }) {
  const p = `/p/${encodeURIComponent(project)}`;
  let answer = "";
  if (area.status === "answered") answer = answerText({ options: area.options });
  else if (area.status === "later") answer = "decide later";
  else if (area.status === "not_applicable") answer = answerText({ not_applicable: area.reason });
  else if (area.status === "proposed") answer = `proposed in ${area.revision || "a revision"}`;
  return (
    <div className="section" id={`area-${area.id}`}>
      <h4>
        {area.title} <span className="muted small">· pack {area.pack}</span>
      </h4>
      {area.question ? <p className="muted small">{area.question}</p> : null}
      {answer ? (
        <p className="small">
          {area.status === "later" ? <span className="chip warn">decide later</span> : answer}
          {area.value ? ` · ${area.value}` : ""}
          {area.item ? (
            <>
              {" "}
              · asked by <Link to={`${p}/feature/${encodeURIComponent(area.item)}`}>{area.item}</Link>
            </>
          ) : null}
        </p>
      ) : (
        <p className="muted small">Not asked yet: Claude asks it at the first design that touches it.</p>
      )}
      {area.entries.length ? (
        <ul className="small">
          {area.entries.map((e) => (
            <EntryLine key={e.id} entry={e} project={project} adrs={adrs} />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function EntryLine({ entry, project, adrs }: { entry: BaselineEntryView; project: string; adrs: { id: string; path: string }[] }) {
  const p = `/p/${encodeURIComponent(project)}`;
  const adr = entry.adr ? adrHref(project, entry.adr, adrs) : null;
  return (
    <li>
      <span className="chip">{entry.kind === "non_goal" ? "non-goal" : entry.kind}</span> <code>{entry.id}</code> {entry.text}
      {entry.adr ? (
        <>
          {" · "}
          {adr ? <Link to={adr}>{entry.adr}</Link> : entry.adr}
        </>
      ) : null}
      {entry.superseded_by ? <span className="muted"> · superseded by {entry.superseded_by}</span> : null}
      {entry.pack_removed ? (
        <>
          {" "}
          <span className="chip warn">pack removed</span>
        </>
      ) : null}
      {entry.not_accepted_through_kanbanr ? (
        <>
          {" "}
          <span className="chip warn">not accepted through kanbanr</span>
        </>
      ) : null}
      {entry.named_by.length ? (
        <span className="muted">
          {" · named by "}
          {entry.named_by.map((code, i) => (
            <span key={code}>
              {i ? ", " : ""}
              <Link to={`${p}/feature/${encodeURIComponent(code)}`}>{code}</Link>
            </span>
          ))}
        </span>
      ) : null}
    </li>
  );
}

/** One accepted revision: what it answered, who accepted it and when. */
function HistoryLine({ rev }: { rev: BaselineRevision }) {
  const a = rev.accepted;
  return (
    <li>
      <code>{rev.id}</code> {rev.answers.map((x) => `${x.area}: ${answerText(x)}`).join("; ")}
      {a ? (
        <span className="muted">
          {" "}
          · accepted by {a.by} on {a.at.slice(0, 10)}
          {a.origin ? ` via the ${a.origin}` : ""}
        </span>
      ) : null}
      {rev.acceptance_unverifiable ? <span className="muted"> · {rev.acceptance_unverifiable}</span> : null}
    </li>
  );
}
