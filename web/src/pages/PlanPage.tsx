/**
 * The Plan page (FTR-043): one page in place of the Gantt and the Schedule. Three tabs, Current,
 * Planned and Past, each a timeline of its sprints, releases and milestones with today in focus,
 * then one row per lane that opens to its items.
 *
 * It loads only what is shown (R-8): the summary read gives the tabs and their counts, the open
 * tab reads its own slice, and a row reads its items only once it is opened. The tab is `?tab=`
 * and the open rows are the hash (`#sprint:SP-004`; a bare `#SP-004` is read as a sprint).
 */
import { useCallback } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { LiveDot } from "../components/bits";
import { Empty, ErrorState, LoadingState, Tabs, Timeline, writeOpen } from "../ui";
import { defaultTab, openLanes, planTabs, toTimeline } from "../planTabs";
import type { PlanScope, PlanSlice, PlanSummary } from "../types";
import { PlanRow } from "./plan/PlanRow";
import { PastRow } from "./plan/PastRow";
import { FlowPanel } from "./plan/FlowPanel";

export default function PlanPage() {
  const { project = "" } = useParams();
  const summary = useQuery<PlanSummary>(project, urls.planSummary(project), TAGS.plan);
  if (!summary.data) {
    return (
      <div className="plan-page">
        <div className="page-head">
          <h1>Plan</h1>
          <LiveDot />
        </div>
        {summary.error ? <ErrorState error={summary.error} retry={summary.reload} /> : <LoadingState />}
      </div>
    );
  }
  return <PlanBody project={project} summary={summary.data} />;
}

/** The page once its summary is read: the heading and the three tabs, only the selected one drawn. */
export function PlanBody(p: { project: string; summary: PlanSummary }): JSX.Element {
  return (
    <div className="plan-page">
      <div className="page-head">
        <h1>Plan</h1>
        <LiveDot />
      </div>
      <Tabs tabs={planTabs(p.summary)} param="tab" defaultKey={defaultTab(p.summary, null)} label="Plan">
        {(key) => (
          <>
            {key === "current" && <FlowPanel project={p.project} />}
            <PlanTab project={p.project} scope={key as PlanScope} />
          </>
        )}
      </Tabs>
    </div>
  );
}

/** One tab: its own slice, read when the tab is selected. */
function PlanTab(p: { project: string; scope: PlanScope }): JSX.Element {
  const { project, scope } = p;
  const slice = useQuery<PlanSlice>(project, urls.plan(project, scope), TAGS.plan);
  if (!slice.data) {
    return slice.error ? <ErrorState error={slice.error} retry={slice.reload} /> : <LoadingState />;
  }
  return <PlanTabBody project={project} scope={scope} slice={slice.data} />;
}

const CAPTION: Record<PlanScope, string> = {
  current: "What runs now",
  planned: "What comes next",
  past: "What closed and shipped",
};

/**
 * A tab's slice drawn: the timeline of its dated lanes, then one row per lane, undated ones last.
 * Past lanes are compact rows. Exported so the checks render it without a daemon.
 */
export function PlanTabBody(p: { project: string; scope: PlanScope; slice: PlanSlice }): JSX.Element {
  const { project, scope, slice } = p;
  const location = useLocation();
  const navigate = useNavigate();
  const open = openLanes(location.hash);
  const toggle = useCallback(
    (key: string, isOpen: boolean) => {
      const next = openLanes(location.hash);
      if (isOpen) next.add(key);
      else next.delete(key);
      navigate(
        { pathname: location.pathname, search: location.search, hash: writeOpen(location.hash, next) },
        { replace: true, preventScrollReset: true },
      );
    },
    [navigate, location.pathname, location.search, location.hash],
  );

  const lanes = [...slice.lanes, ...slice.undated];
  if (lanes.length === 0) {
    if (scope === "current") return <Empty title="Nothing is running today." ask="Ask Claude to plan the next sprint." />;
    if (scope === "planned") return <Empty title="Nothing is planned yet." />;
    return <Empty title="Nothing has closed or shipped yet." />;
  }

  const timeline = toTimeline(slice);
  const drawn = timeline.lanes.filter((l) => l.bands.length > 0 || l.marks.length > 0);
  return (
    <div className={`plan-tab plan-${scope}`}>
      {drawn.length > 0 && (
        <Timeline
          from={timeline.from}
          to={timeline.to}
          today={timeline.today}
          lanes={drawn}
          caption={`${CAPTION[scope]}: ${timeline.from} to ${timeline.to}`}
        />
      )}
      <div className="plan-rows">
        {lanes.map((lane) =>
          scope === "past" ? (
            <PastRow
              key={lane.key}
              project={project}
              lane={lane}
              from={slice.from}
              to={slice.to}
              today={slice.today}
              open={open.has(lane.key)}
              onToggle={(v) => toggle(lane.key, v)}
            />
          ) : (
            <PlanRow
              key={lane.key}
              project={project}
              lane={lane}
              from={slice.from}
              to={slice.to}
              today={slice.today}
              open={open.has(lane.key)}
              onToggle={(v) => toggle(lane.key, v)}
            />
          ),
        )}
      </div>
    </div>
  );
}
