import { useEffect, useMemo, useRef } from "react";
import { Link, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { useScrollToHash } from "../useBoardIndex";
import { LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { Empty, ErrorState, LoadingState, Section } from "../ui";
import { SEEN_AFTER_MS, goalRows, seenKey, seriesFor, sinceLastVisit } from "../charterModel";
import { CharterLists } from "./charter/CharterLists";
import { GoalGrowth } from "./charter/GoalGrowth";
import { GoalsTable } from "./charter/GoalsTable";
import { LessonsSection } from "./charter/LessonsSection";
import { RecentWins } from "./charter/RecentWins";
import { WayTravelled } from "./charter/WayTravelled";
import type { Charter, CharterProgress } from "../types";

/**
 * The project's charter (FEAT-046, FTR-076): why it exists, what it commits to, for whom, what it
 * will not do, and how far the project has travelled toward its goals. Two reads: the charter, and
 * the derived progress (`GET /charter/progress`); everything drawn here comes from those two.
 *
 * When this browser last looked is kept in `localStorage` only, never on the board.
 */
export default function CharterPage() {
  const { project = "" } = useParams();
  const charter = useQuery<Charter>(project, urls.charter(project), TAGS.charter);
  const progress = useQuery<CharterProgress>(project, urls.charterProgress(project), TAGS.charterProgress);
  useScrollToHash(!!charter.data && !!progress.data);

  // What was true when this browser last looked, read once per project.
  const seen = useMemo(() => readSeen(project), [project]);
  const loaded = useRef(false);
  loaded.current = !!progress.data;
  useEffect(() => {
    const write = () => {
      if (loaded.current) writeSeen(project);
    };
    const timer = setTimeout(write, SEEN_AFTER_MS);
    window.addEventListener("pagehide", write);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("pagehide", write);
      write();
    };
  }, [project]);

  if (charter.loading && !charter.data) return <LoadingState />;
  if (charter.error) return <ErrorState error={charter.error} retry={charter.reload} />;
  if (!charter.data) return null;
  // An older daemon has no progress route: the charter's tables still draw, the derived parts say so.
  const missing = !!progress.error && progress.error.startsWith("404");
  return (
    <>
      {progress.error && !missing ? <ErrorState error={progress.error} retry={progress.reload} /> : null}
      <CharterView
        project={project}
        charter={charter.data}
        progress={progress.data}
        seen={seen}
        state={progress.data ? "ready" : progress.error ? "unavailable" : "pending"}
      />
    </>
  );
}

function readSeen(project: string): string | null {
  try {
    return window.localStorage.getItem(seenKey(project));
  } catch {
    return null;
  }
}

function writeSeen(project: string): void {
  try {
    window.localStorage.setItem(seenKey(project), new Date().toISOString());
  } catch {
    /* a browser that keeps nothing simply has no last visit */
  }
}

const pad = (n: number): string => String(n).padStart(2, "0");

/** "14:05" in the reader's own time. */
function clock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const NOT_YET = "This kanbanr does not report progress yet: update it to see the way travelled.";

/**
 * The page drawn from its data, exported so the checks render it without a daemon. `progress` null
 * is a daemon that has no progress route (`state` "unavailable", the default) or one still being
 * read (`"pending"`): the charter's own tables draw and each derived section says why it is missing.
 */
export function CharterView(p: {
  project: string;
  charter: Charter;
  progress: CharterProgress | null;
  seen: string | null;
  state?: "ready" | "pending" | "unavailable";
}): JSX.Element {
  const { project, charter: c, progress } = p;
  const state = progress ? "ready" : (p.state ?? "unavailable");
  const goals = c.goals ?? [];
  const goalIds = goals.map((g) => g.id);
  const empty = !c.purpose?.trim() && goals.length === 0 && !(c.non_goals ?? []).length && !(c.stakeholders ?? []).length;
  const since = progress ? sinceLastVisit(progress, p.seen) : null;
  const trends: Record<string, number[]> = {};
  if (progress) for (const g of goalIds) trends[g] = seriesFor(progress, g).map((pt) => pt.req_proven);
  /** What stands in a derived section while its read is missing. */
  const derived = (children: JSX.Element | null): JSX.Element | null =>
    progress ? children : state === "pending" ? <LoadingState /> : <p className="muted">{NOT_YET}</p>;

  return (
    <div className="charter-page">
      <div className="page-head">
        <h1>Charter</h1>
        <LiveDot />
        {progress && (
          <span className="charter-read muted small">
            Read {clock(progress.generated_at)}
            {progress.board_rev ? ` from board ${progress.board_rev.slice(0, 7)}` : ""}
          </span>
        )}
      </div>

      {empty && <Empty title="No charter yet. Ask your agent to write the project's charter: its purpose, goals and what it will not do." />}

      <Section title="Purpose" id="purpose">
        {c.purpose?.trim() ? <Markdown source={c.purpose} /> : <span className="ui-pill ui-pill-warn">[MISSING: purpose]</span>}
        {c.vision?.trim() ? <p className="muted">{c.vision}</p> : null}
        {c.adopted_at ? <p className="muted small">Adopted {c.adopted_at.slice(0, 10)}. Items created before this predate the method.</p> : null}
      </Section>

      {since && (
        <p className="charter-since" role="status">
          {sinceText(since)}
        </p>
      )}

      <Section title="Goals" id="goals">
        {goals.length === 0 ? (
          <Empty title="No goals yet: items have nothing to serve." />
        ) : (
          <GoalsTable rows={goalRows(progress, c)} trends={trends} withProgress={progress !== null} />
        )}
      </Section>

      <Section title="The way travelled" id="way">
        {derived(progress && <WayTravelled progress={progress} goalIds={goalIds} />)}
      </Section>

      {(state !== "ready" || (progress?.releases_on && goalIds.length > 0)) && (
        <Section title="Progress per goal" id="growth">
          {derived(
            progress &&
              ((progress.route ?? []).some((s) => s.state === "shipped") ? (
                <GoalGrowth progress={progress} goalIds={goalIds} />
              ) : (
                <p className="muted">Nothing has shipped yet, so there is no progress to draw.</p>
              )),
          )}
        </Section>
      )}

      <Section title="Lessons" id="lessons">
        {derived(progress && <LessonsSection project={project} progress={progress} />)}
      </Section>

      <Section title="Recent wins" id="recent-wins">
        {derived(progress && <RecentWins project={project} wins={progress.wins ?? []} />)}
      </Section>

      <Section title="What the project will not do" id="limits">
        <CharterLists nonGoals={c.non_goals ?? []} stakeholders={c.stakeholders ?? []} constraints={c.constraints ?? []} />
      </Section>
    </div>
  );
}

/** The since-last-visit sentence with each count a link to the list behind it. */
function sinceText(s: NonNullable<ReturnType<typeof sinceLastVisit>>): JSX.Element {
  // The sentence is the model's; each link label is found in it and wrapped, in order.
  const out: (string | JSX.Element)[] = [];
  let rest = s.text;
  for (const l of s.links) {
    const at = rest.indexOf(l.label);
    if (at < 0) continue;
    if (at > 0) out.push(rest.slice(0, at));
    out.push(
      <Link key={out.length} to={l.to}>
        {l.label}
      </Link>,
    );
    rest = rest.slice(at + l.label.length);
  }
  out.push(rest);
  return <>{out}</>;
}
