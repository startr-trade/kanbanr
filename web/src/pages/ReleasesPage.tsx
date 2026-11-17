import { useCallback, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { api, type Meta } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { LiveDot } from "../components/bits";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { Empty, ErrorState, LoadingState, Tabs, useOpenRows, useUrlState, writeOpen } from "../ui";
import { TAB_OF, defaultTab, initiallyOpen, releaseTabs, type ReleaseTab } from "../releaseTabs";
import type { ProjectConfig, Release, ReleasesSummary, Scope } from "../types";
import { ReleaseRow } from "./releases/ReleaseRow";
import type { ReleaseCtx } from "./releases/ReleaseBody";
import { BaselineStageGates } from "./releases/BaselineStageGates";

/** The stages a release moves through when the project names none (FEAT-183). */
const DEFAULT_STAGES = ["candidate", "test", "staging", "production"];

/**
 * Releases (FEAT-120, FEAT-123, FTR-041): three tabs, Current, Planned and Released, each listing
 * its releases newest first, one accordion each. A header carries the release's chips, its stage
 * stepper and its next step; the body, read only when opened, is a full-width table of its items
 * with the tag, CI and notes below. Only for projects that use releases.
 *
 * It loads only what is shown (R-6): the summary gives the tabs and their counts, the selected tab
 * reads its own slice, and an open release reads its table. The tab is `?tab=`, the open releases
 * the hash (`#v0.8.0,v0.7.1`), and the filter `?q=`.
 */
export default function ReleasesPage() {
  const { project = "" } = useParams();
  const summary = useQuery<ReleasesSummary>(project, urls.releasesSummary(project), ["releases", "items"]);
  const config = useQuery<ProjectConfig>(project, urls.config(project), TAGS.config);
  const meta = useQuery<Meta>(null, urls.meta(), TAGS.meta, () => api.getMeta());
  const error = summary.error ?? config.error;
  if (!summary.data || !config.data) {
    return (
      <div className="releases-page">
        <div className="page-head">
          <h1>Releases</h1>
          <LiveDot />
        </div>
        {error ? <ErrorState error={error} retry={() => (summary.reload(), config.reload())} /> : <LoadingState />}
      </div>
    );
  }
  const cadence = config.data.cadence;
  if (!cadence?.releases) {
    return (
      <div className="releases-page">
        <h1>Releases</h1>
        <p className="muted">
          This project does not use releases. Switch them on with{" "}
          <code>kanbanr config cadence --releases on</code>.
        </p>
      </div>
    );
  }
  // Recorded as the board's commit identity, as `kanbanr release promote` records it.
  const who = api.approver(meta.data ?? null);
  return (
    <ReleasesBody
      project={project}
      summary={summary.data}
      stages={cadence.release_stages?.length ? cadence.release_stages : DEFAULT_STAGES}
      gates={cadence.commit_gates ?? []}
      notesPath={cadence.release_prepare?.notes ?? ""}
      // A read-only copy of the monitor (FEAT-178) never offers a promotion, whatever the daemon says.
      writable={!isReadOnly() && meta.data?.writes === true && who != null}
      who={who}
      cadence={cadence}
    />
  );
}

/** What a tab's list needs from the page. */
interface PageProps {
  project: string;
  stages: string[];
  gates: string[];
  notesPath: string;
  writable: boolean;
  who: string | null;
  /** The project's cadence, for the baseline's stage gates (FTR-033 P13); absent in checks that don't need them. */
  cadence?: ProjectConfig["cadence"];
}

/**
 * The page once its summary is read: the heading, the filter box and the three tabs, only the
 * selected one drawn. Exported so the checks render it without a daemon.
 */
export function ReleasesBody(p: PageProps & { summary: ReleasesSummary }): JSX.Element {
  const [state, setState] = useUrlState({ q: "" });
  const { summary, ...page } = p;
  const none = summary.current + summary.planned + summary.past === 0;
  return (
    <div className="releases-page">
      <div className="page-head">
        <h1>Releases</h1>
        <LiveDot />
      </div>
      {p.cadence ? <BaselineStageGates project={p.project} cadence={p.cadence} /> : null}
      {none ? (
        <p className="muted">
          No releases yet — <code>kanbanr release add v0.1.0 --target YYYY-MM-DD</code>.
        </p>
      ) : (
        <>
          <div className="releases-filter ui-table-tools">
            <label>
              Filter{" "}
              <input
                type="search"
                value={state.q}
                placeholder="version, name or item"
                aria-label="Filter releases and their items"
                onChange={(e) => setState({ q: e.currentTarget.value })}
              />
            </label>
          </div>
          <Tabs tabs={releaseTabs(summary)} param="tab" defaultKey={defaultTab(summary, null)} label="Releases">
            {(key) => <ReleaseList {...page} scope={TAB_OF[key as ReleaseTab]} />}
          </Tabs>
        </>
      )}
    </div>
  );
}

/** One tab: its own slice, read when the tab is selected. */
function ReleaseList(p: PageProps & { scope: Scope }): JSX.Element {
  const { project, scope } = p;
  const slice = useQuery<Release[]>(project, urls.releases(project, scope), ["releases", "items", "gates"]);
  if (!slice.data) {
    return slice.error ? <ErrorState error={slice.error} retry={slice.reload} /> : <LoadingState />;
  }
  return <ReleaseListBody {...p} slice={slice.data} />;
}

const EMPTY: Record<Scope, string> = {
  current: "No release is at a stage.",
  planned: "No release is planned.",
  past: "Nothing has shipped yet.",
};

/** Whether a release's version or name holds the filter text, case ignored. */
const named = (r: Release, q: string) => `${r.version} ${r.name ?? ""}`.toLowerCase().includes(q.trim().toLowerCase());

/**
 * A tab's releases drawn, newest first as the slice comes: one `ReleaseRow` each. The first is open
 * unless the hash names others in this tab (R-2). The filter hides a closed release whose version
 * or name does not match and filters the open releases' tables (R-10). Exported so the checks render
 * it without a daemon.
 */
export function ReleaseListBody(p: PageProps & { scope: Scope; slice: Release[] }): JSX.Element {
  const { project, scope, slice, stages } = p;
  const tick = useLiveTick(api.projectEvents(project));
  // What Claude prepared for these decisions (FTR-034): CI results and confirmations after the tag.
  const prepared = useAsync(() => api.getPrepared(project), [project, tick]);
  // Promote (FEAT-185): which release is being promoted, and what came of it.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});
  // The prepared records each release's forms already show (DEF-095), by version, as the release's
  // button reports them; the rest are listed under "Prepared, not required".
  const [shown, setShown] = useState<Record<string, string[]>>({});
  const onShown = useCallback(
    (version: string, ids: string[]) =>
      setShown((s) => (s[version]?.join(",") === ids.join(",") ? s : { ...s, [version]: ids })),
    [],
  );
  const [{ q }] = useUrlState({ q: "" });
  const [hashOpen] = useOpenRows();
  const location = useLocation();
  const navigate = useNavigate();
  // Until a row is toggled, the first release is open unless the hash names others here; after,
  // exactly the hash's.
  const [touched, setTouched] = useState(false);
  const open = touched ? hashOpen : initiallyOpen(slice, hashOpen);
  const toggle = (version: string, isOpen: boolean) => {
    const next = new Set(open);
    if (isOpen) next.add(version);
    else next.delete(version);
    setTouched(true);
    navigate(
      { pathname: location.pathname, search: location.search, hash: writeOpen(location.hash, next) },
      { replace: true, preventScrollReset: true },
    );
  };

  // The stage after the one it is at; the first, before it has one. None once it is at the last.
  const nextStage = (r: Release) => {
    // A superseded release never ships: its tag stays, and the patch carries on (FTR-026).
    if (r.state === "shipped" || r.superseded_by) return null;
    const at = r.stage ? stages.indexOf(r.stage) : -1;
    // Tagged at the last stage, it waits for its tag workflow's result and ships on it (DEF-049):
    // the stage it is at is the one the button still speaks for.
    if (r.tagged_at && at >= 0 && at + 1 === stages.length) return stages[at];
    return at + 1 < stages.length ? stages[at + 1] : null;
  };
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183), carrying
  // the release tip the plan showed (FEAT-212).
  const promote = async (r: Release, to: string, expect: string, note = "", acceptUntested?: string[]) => {
    if (!p.who) return;
    setBusy(r.version);
    try {
      const done = (await api.promoteRelease(project, r.version, p.who, to, expect, note, acceptUntested)) as {
        next?: string;
      };
      // A stage taken by mistake is stepped back from the CLI, by the person (DEF-034): the page
      // offers no Undo button, so the chip names the command.
      const undo = to ? ` — undo with \`kanbanr release undo ${r.version} --reason …\` while nothing is pushed` : "";
      setOutcome((o) => ({ ...o, [r.version]: `${to ? `promoted to ${to}` : `promoted: ${r.version} shipped`}${done?.next ? ` — ${done.next}` : ""}${undo}` }));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({
        ...o,
        [r.version]: `${message} — from the project folder: kanbanr release promote ${r.version} ${to}${note ? ` --note "${note}"` : ""}`,
      }));
    } finally {
      setBusy(null);
    }
  };

  // Offered only for a release that holds nothing and has not started (FTR-022): the same rule the
  // daemon applies, so the button never offers what would be refused. A patch of a planned release
  // would be listed in this tab too.
  const removable = (r: Release) =>
    r.state === "planned" &&
    !r.stage &&
    !r.branch &&
    !r.notes_doc &&
    (r.items ?? []).length === 0 &&
    !slice.some((o) => o.parent === r.version);
  const remove = async (r: Release) => {
    if (!window.confirm(`Remove release ${r.version}? It holds nothing.`)) return;
    setBusy(r.version);
    try {
      await api.removeRelease(project, r.version);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({ ...o, [r.version]: `${message} — from the project folder: kanbanr release remove ${r.version}` }));
    } finally {
      setBusy(null);
    }
  };

  if (slice.length === 0) return <Empty title={EMPTY[scope]} />;
  const ctx: ReleaseCtx = {
    project,
    stages,
    gates: p.gates,
    notesPath: p.notesPath,
    writable: p.writable,
    who: p.who,
    busy,
    outcome,
    onPromote: promote,
    removable,
    onRemove: remove,
    prepared: prepared.data ?? [],
    shown,
    onShown,
    tick,
    q,
  };
  const shownRows = slice.filter((r) => open.has(r.version) || named(r, q));
  return (
    <div className={`release-list release-${scope}`}>
      {shownRows.length === 0 ? (
        <Empty title={`No release here matches "${q}".`} />
      ) : (
        shownRows.map((r) => (
          <ReleaseRow
            key={r.version}
            ctx={ctx}
            release={r}
            scope={scope}
            next={nextStage(r)}
            open={open.has(r.version)}
            onToggle={(v) => toggle(r.version, v)}
          />
        ))
      )}
    </div>
  );
}
