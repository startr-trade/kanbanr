import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { byAgentText } from "../agentScopes";
import { boardLanes, SEEN_AFTER_MS, seenKey, toggleGroup } from "../boardModel";
import type { Lane, SortKey } from "../boardModel";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { displayedStates } from "../types";
import type { Activity, AgentScope, CompactIndex, CriticalPath, IndexConfig, Readiness, SprintReport } from "../types";
import { ALL, UNSCHEDULED } from "../sprintFilter";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Burndown from "../components/Burndown";
import { ErrorState, LoadingState } from "../ui/States";
import { parseOpen, useOpenRows, useUrlState, writeOpen } from "../ui/urlState";
import { LaneGroups } from "./board/LaneGroups";
import { LaneHeader } from "./board/LaneHeader";
import "./board/board.css";

const SORTS: { value: SortKey; label: string }[] = [
  { value: "close", label: "Closest to next stage" },
  { value: "newest", label: "Newest" },
  { value: "priority", label: "Priority" },
];

/** The one line an older daemon earns: it reports neither the days in a stage nor what each stage lacks. */
export const OLDER_DAEMON_NOTICE =
  "This Kanbanr does not report how long items have been in their stage or what each stage still lacks: update it to see them.";

/** The notice, or null when the daemon's answers carry the new fields (or there is nothing to judge). */
export function olderDaemonNotice(index: CompactIndex, readiness: Readiness | null): string | null {
  const noAges = index.items.length > 0 && !index.items.some((i) => !!i.entered_on);
  const cards = readiness ? Object.values(readiness) : [];
  const noChecks = cards.length > 0 && !cards.some((c) => c.next.some((n) => !!n.checks) || !!c.updated_at);
  return noAges || noChecks ? OLDER_DAEMON_NOTICE : null;
}

/** The Board, pure over what the page read, so a check renders it without a daemon. */
export function BoardView(p: {
  project: string;
  /** The compact index the lanes were built from: cards name their milestones from it. */
  index: CompactIndex;
  lanes: Lane[];
  config: IndexConfig;
  sprintPicker: ReactNode;
  sort: SortKey;
  onSort(s: SortKey): void;
  open: Set<string>;
  shut: Set<string>;
  onToggle(id: string, open: boolean, byDefault: boolean): void;
  filters: Set<string>;
  onFilter(id: string, on: boolean): void;
  notice: string | null;
  otherStates: ReactNode;
  activity: ReactNode;
  /** The text filter, drawn in the page head. */
  textFilter?: ReactNode;
  /** The chosen sprint's header. */
  sprintHeader?: ReactNode;
  /** Readiness has not answered yet: the lanes show a loading state instead of groups. */
  loading?: boolean;
  /** Readiness failed: items are grouped from the index alone and this says why. */
  readError?: string | null;
  /** Clears the bar filters of a lane (several ids at once). */
  onClearFilters?(ids: string[]): void;
}): JSX.Element {
  const clear = p.onClearFilters ?? ((ids: string[]) => ids.forEach((id) => p.onFilter(id, false)));
  return (
    <div className="board-page">
      <div className="page-head">
        <h1>{p.config.name || p.project}</h1>
        <LiveDot />
        <span className="spacer" />
        {p.sprintPicker}
        <label className="per-page">
          Order
          <select value={p.sort} onChange={(e) => p.onSort(e.target.value as SortKey)}>
            {SORTS.map((s) => (
              <option key={s.value} value={s.value}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
        {p.textFilter}
      </div>
      {p.config.description && <p className="muted">{p.config.description}</p>}
      {p.notice && (
        <p className="board-notice" role="status">
          {p.notice}
        </p>
      )}
      {p.readError && <ErrorState error={`Could not read what each item needs next: ${p.readError}`} />}
      {p.sprintHeader}
      <div className="board">
        {p.lanes.map((lane) => (
          <section className="column" key={lane.status}>
            <h2 className="col-title">
              <Link className="column-head" to={`/p/${encodeURIComponent(p.project)}/state/${encodeURIComponent(lane.status)}`}>
                {lane.status} <span className="count">{lane.total}</span>
              </Link>
            </h2>
            <div className="column-body">
              {p.loading && lane.kind !== "finished" ? (
                <LoadingState />
              ) : (
                <>
                  <LaneHeader lane={lane} only={p.filters} onFilter={p.onFilter} />
                  <LaneGroups
                    project={p.project}
                    lane={lane}
                    index={p.index}
                    open={p.open}
                    shut={p.shut}
                    only={p.filters}
                    onToggle={p.onToggle}
                    onClear={clear}
                  />
                </>
              )}
            </div>
          </section>
        ))}
      </div>
      {p.otherStates}
      {p.activity}
    </div>
  );
}

export default function BoardPage() {
  const { project = "" } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  // The compact index has every card, the config and the sprints; never the whole board (DEF-081).
  const { data, error, loading } = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);
  const readiness = useQuery<Readiness>(project, urls.readiness(project), TAGS.readiness);
  // Agent scopes and the critical path only add a fact to a card; if they fail the fact is left out.
  const scopes = useQuery<AgentScope[]>(project, urls.scopes(project), TAGS.scopes);
  const critical = useQuery<CriticalPath>(project, urls.criticalPath(project), TAGS.criticalPath);
  // Sprints only where the project uses them (FEAT-119, FEAT-123); everywhere else the board is as
  // it always was.
  const usesSprints = data?.config.cadence.sprints === true;
  const sprints = usesSprints ? (data?.sprints ?? []) : [];
  const active = sprints.find((s) => s.state === "active");
  const [chosen, setChosen] = useState<string | null>(null);
  const sprintCode = chosen ?? active?.code ?? ALL;
  // A real sprint has a report; All items and Unscheduled (FTR-032) don't.
  const isSprint = sprintCode !== ALL && sprintCode !== UNSCHEDULED;
  const sprint = useQuery<SprintReport>(
    project,
    usesSprints && isSprint ? urls.sprint(project, sprintCode) : null,
    TAGS.sprints,
  );
  const [q, setQ] = useState("");
  // The sort lives in the query string (?bsort=newest), the open groups and bar filters in the hash.
  const [{ sort }, setUrl] = useUrlState({ sort: "close" }, "b");
  const [open] = useOpenRows("open");
  const [shut] = useOpenRows("shut");
  const [only] = useOpenRows("only");

  // When the person last looked, kept in this browser only: read once on arrival, written after a
  // while on the page and when it is left, never earlier, so this visit compares with the last one.
  const [seen] = useState<string | null>(() => {
    try {
      return localStorage.getItem(seenKey(project));
    } catch {
      return null;
    }
  });
  useEffect(() => {
    const write = () => {
      try {
        localStorage.setItem(seenKey(project), new Date().toISOString());
      } catch {
        /* a browser that refuses storage just forgets the visit */
      }
    };
    const timer = window.setTimeout(write, SEEN_AFTER_MS);
    window.addEventListener("pagehide", write);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("pagehide", write);
    };
  }, [project]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const items = data.items;
  const states = displayedStates(data.config);
  // Non-displayed statuses (includes no-op) — still reachable via status-page links.
  const otherStates = (data.config.statuses ?? []).filter((s) => !states.includes(s));
  const isNoOp = (s: string) => data.config.no_op_states?.includes(s);
  const countIn = (s: string) => items.filter((f) => f.status === s).length;

  const lanes = boardLanes({
    project,
    index: data,
    readiness: readiness.data,
    scopes: scopes.data ?? [],
    critical: critical.data?.critical_path ?? [],
    seen,
    now: new Date(),
    sprint: sprintCode,
    text: q,
    sort: sort as SortKey,
  });

  // The hash lists change together (a toggle moves an id between two lists), so one navigation writes them.
  const writeHash = (lists: Record<string, Set<string>>) => {
    let hash = location.hash;
    for (const [param, ids] of Object.entries(lists)) hash = writeOpen(hash, ids, param);
    navigate({ pathname: location.pathname, search: location.search, hash }, { replace: true, preventScrollReset: true });
  };
  const onToggle = (id: string, nowOpen: boolean, byDefault: boolean) => {
    const next = toggleGroup(parseOpen(location.hash, "open"), parseOpen(location.hash, "shut"), id, nowOpen, byDefault);
    writeHash({ open: next.open, shut: next.shut });
  };
  const onFilter = (id: string, on: boolean) => {
    const next = new Set(parseOpen(location.hash, "only"));
    if (on) next.add(id);
    else next.delete(id);
    writeHash({ only: next });
  };
  const onClearFilters = (ids: string[]) => {
    const next = new Set(parseOpen(location.hash, "only"));
    for (const id of ids) next.delete(id);
    writeHash({ only: next });
  };

  const sprintPicker = (
    <>
      {usesSprints && (
        <label className="per-page">
          Sprint
          <select value={sprintCode} onChange={(e) => setChosen(e.target.value)}>
            <option value={ALL}>All items</option>
            <option value={UNSCHEDULED}>Unscheduled</option>
            {sprints.map((s) => (
              <option key={s.code} value={s.code}>
                {s.code}
                {s.state === "active" ? " (active)" : s.state === "closed" ? " (closed)" : ""}
              </option>
            ))}
          </select>
        </label>
      )}
    </>
  );

  return (
    <BoardView
      project={project}
      index={data}
      lanes={lanes}
      config={data.config}
      sprintPicker={sprintPicker}
      sort={sort as SortKey}
      onSort={(s) => setUrl({ sort: s })}
      open={open}
      shut={shut}
      onToggle={onToggle}
      filters={only}
      onFilter={onFilter}
      onClearFilters={onClearFilters}
      notice={olderDaemonNotice(data, readiness.data)}
      loading={readiness.loading && !readiness.data && !readiness.error}
      readError={readiness.error}
      textFilter={
        <input
          className="board-search"
          aria-label="Filter items"
          placeholder="Filter… (text · kind · priority · label)"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      }
      sprintHeader={sprint.data ? <SprintHeader report={sprint.data} /> : null}
      otherStates={
        otherStates.length > 0 ? (
          <div className="other-states">
            <span className="muted small">Other states:</span>
            {otherStates.map((s) => (
              <Link
                key={s}
                to={`/p/${encodeURIComponent(project)}/state/${encodeURIComponent(s)}`}
                className={`statelink${isNoOp(s) ? " noop" : ""}`}
                title={isNoOp(s) ? "no-op state" : "non-displayed state"}
              >
                {s} <b>{countIn(s)}</b>
              </Link>
            ))}
          </div>
        ) : null
      }
      activity={<ActivityPanel project={project} />}
    />
  );
}

/** Recent activity (from the project's changelog), re-read when the activity changes. */
function ActivityPanel({ project }: { project: string }) {
  const { data } = useQuery<Activity[]>(project, urls.activity(project, {}), TAGS.activity);
  if (!data || data.length === 0) return null;
  const when = (iso: string) => {
    const d = new Date(iso);
    return isNaN(d.getTime()) ? iso : d.toLocaleString();
  };
  return (
    <section className="activity">
      <h2>Recent activity</h2>
      <ul className="activity-list">
        {data.slice(0, 12).map((a, i) => (
          <li key={i}>
            <span className="activity-msg">{a.message}</span>
            <span className="activity-meta">
              {byAgentText(a)} · {when(a.time)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** What the chosen sprint is for, where it stands, and how it burned down (FEAT-123). */
function SprintHeader({ report }: { report: SprintReport }) {
  const unit = report.unit;
  return (
    <section className="section sprint-header">
      <div className="tile">
        <div className="tile-title">
          {report.code} {report.name ? `· ${report.name}` : ""}{" "}
          <span className="chip">{report.state}</span>
        </div>
        {report.goal ? <div className="tile-desc">{report.goal}</div> : null}
        <div className="tile-states">
          <span className="chip">
            {report.start} → {report.end}
          </span>
          <span className="chip">{report.days_left} day(s) left</span>
          <span className="chip tasks">
            {report.done} of {report.committed} {unit} done
          </span>
          {report.capacity != null ? (
            <span className={`chip ${report.committed > report.capacity ? "warn" : ""}`}>
              capacity {report.capacity}
            </span>
          ) : null}
          {report.unestimated.length > 0 ? (
            <span className="chip warn" title={report.unestimated.join(", ")}>
              {report.unestimated.length} unestimated
            </span>
          ) : null}
        </div>
        <Burndown report={report} />
      </div>
    </section>
  );
}
