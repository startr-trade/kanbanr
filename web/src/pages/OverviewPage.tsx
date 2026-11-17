/**
 * The project Overview (FTR-048): the page a project opens on. In order: what needs the person,
 * what is happening now, how much the board holds, the goals, recent decisions and ADRs, and the
 * activity. Every number links to the list it counted, through `overviewLinks.ts`; a number with no
 * list page (the flow figures) offers the sentence to ask Claude instead. One read, `/overview`,
 * plus the daemon's latest drift result and the compact index the shell already holds.
 */
import { Link, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { LiveDot } from "../components/bits";
import { CopyText, DataTable, Empty, ErrorState, LoadingState, ProgressBar, Section, StageStepper, StatTile, Timeline, type Column, type Step, type Tone } from "../ui";
import { drawerSections, type DrawerLine } from "../needsYouState";
import { ago } from "../driftAge";
import { to } from "../overviewLinks";
import type { CompactIndex, DriftLatest, GoalFacts, Overview, Recent } from "../types";

/** The most action cards Needs you shows; past it, the last card opens the drawer. */
const MAX_CARDS = 5;
/** What a flow number with no list page offers instead (R-13). */
const FLOW_ASK = "Ask Claude for the flow report.";

const TONES: Tone[] = ["ok", "info", "warn", "error", "neutral"];
const toneOf = (t: string): Tone => (TONES.includes(t as Tone) ? (t as Tone) : "neutral");

export default function OverviewPage() {
  const { project = "" } = useParams();
  const overview = useQuery<Overview>(project, urls.overview(project), TAGS.overview);
  const drift = useQuery<DriftLatest>(project, urls.driftLatest(project, false), TAGS.driftLatest);
  const index = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);
  const cadence = index.data ? { sprints: index.data.config.cadence.sprints === true, releases: index.data.config.cadence.releases === true } : null;
  return (
    <OverviewBody
      project={project}
      name={index.data?.config.name || project}
      overview={overview}
      drift={drift.data}
      cadence={cadence}
      today={new Date().toISOString().slice(0, 10)}
    />
  );
}

/** Opens the Needs-you drawer from the page: the badge in the top bar owns it. */
function openDrawer() {
  if (typeof document === "undefined") return;
  const badge = document.querySelector<HTMLButtonElement>('button[aria-controls="needs-you"]');
  if (badge && badge.getAttribute("aria-expanded") !== "true") badge.click();
  badge?.focus();
}

/** A tile with no page behind it, with its "Ask Claude" sentence and a Copy button under it. */
function AskTile(p: { label: string; value: string | number; ask: string; tone?: Tone; sub?: string }): JSX.Element {
  return (
    <div className="overview-ask-tile">
      <StatTile label={p.label} value={p.value} tone={p.tone} sub={p.sub} />
      <p className="overview-ask">
        <q>{p.ask}</q> <CopyText text={p.ask} label="Copy" />
      </p>
    </div>
  );
}

/** The attention lines (and the drift line) as cards: by section, then the largest count first. */
export function cardLines(lines: DrawerLine[]): { shown: DrawerLine[]; more: number } {
  if (lines.length <= MAX_CARDS) return { shown: lines, more: 0 };
  return { shown: lines.slice(0, MAX_CARDS - 1), more: lines.length - (MAX_CARDS - 1) };
}

function NeedsYouCards(p: { overview: Overview; drift: DriftLatest | null }): JSX.Element {
  const sections = drawerSections(p.overview.attention, p.drift, new Date());
  const lines = sections.flatMap((s) => [...s.lines].sort((a, b) => b.count - a.count));
  if (lines.length === 0) return <Empty title="Nothing needs you right now." />;
  const { shown, more } = cardLines(lines);
  return (
    <ul className="overview-cards">
      {shown.map((l, i) => (
        <li key={`${l.kind}-${i}`} className="overview-card">
          {l.to ? (
            <StatTile label={l.label} value={l.count} to={l.to} tone={toneOf(l.tone)} sub={l.note} />
          ) : (
            <AskTile label={l.label} value={l.count} tone={toneOf(l.tone)} sub={l.note} ask={l.ask ?? ""} />
          )}
        </li>
      ))}
      {more > 0 && (
        <li className="overview-card">
          <button type="button" className="ui-tile overview-more" onClick={openDrawer}>
            <span className="ui-tile-value">{more}</span> <span className="ui-tile-label">more in Needs you</span>
          </button>
        </li>
      )}
    </ul>
  );
}

function NowSection(p: { project: string; overview: Overview; cadence: { sprints: boolean; releases: boolean } | null }): JSX.Element {
  const { sprint, releases = [], milestones = [] } = p.overview.now;
  const pr = p.project;
  return (
    <div className="overview-now">
      {(sprint || p.cadence?.sprints) && (
      <div className="overview-col">
        <h3>Sprint</h3>
        {sprint ? (
          <StatTile
            label={`done in ${sprint.code}`}
            value={`${sprint.done} of ${sprint.committed}`}
            to={to.sprint(pr, sprint.code)}
            sub={`${sprint.days_left} day${sprint.days_left === 1 ? "" : "s"} left${sprint.name ? ` · ${sprint.name}` : ""}`}
          />
        ) : (
          <Empty title="No sprint is running." ask="Ask Claude to plan the next sprint." />
        )}
      </div>
      )}
      {(releases.length > 0 || p.cadence?.releases) && (
      <div className="overview-col">
        <h3>Releases</h3>
        {releases.length > 0 ? (
          releases.map((r) => (
            <div key={r.version} className="overview-release">
              <StatTile
                label={`items finished in ${r.version}`}
                value={`${r.finished} of ${r.items}`}
                to={to.release(pr, r.version)}
                sub={r.waits_on_tag ? "tagged, waiting for its workflow" : r.stage ? `at ${r.stage}` : undefined}
              />
              <StageStepper stages={r.steps.map((s) => ({ ...s, state: s.state as Step["state"] }))} label={`${r.version} stages`} />
            </div>
          ))
        ) : (
          <Empty title="No release is in flight." ask="Ask Claude what the next release should hold." />
        )}
      </div>
      )}
      <div className="overview-col">
        <h3>Milestones</h3>
        {milestones.length > 0 ? (
          milestones.map((m) => (
            <ProgressBar
              key={m.code}
              label={m.name ? `${m.code} ${m.name}` : m.code}
              total={m.total}
              to={to.milestone(pr, m.code)}
              segments={[
                { key: "done", label: "Done", value: m.done, tone: "ok" },
                { key: "open", label: "Open", value: Math.max(0, m.total - m.done), tone: "neutral" },
              ]}
            />
          ))
        ) : (
          <Empty title="No milestone is in progress." />
        )}
      </div>
    </div>
  );
}

const days = (d: number | undefined) => (d === undefined ? "—" : d < 1 ? `${Math.round(d * 24 * 60)} min` : `${d.toFixed(1)} d`);
const sum = (m: Record<string, number> | undefined) => Object.values(m ?? {}).reduce((a, b) => a + b, 0);

function HoldsSection(p: { project: string; overview: Overview }): JSX.Element {
  const h = p.overview.holds;
  const pr = p.project;
  const items = (q: Record<string, string> = {}) => to.items(pr, q);
  const pct = h.coverage.total > 0 ? Math.round((h.coverage.proven / h.coverage.total) * 100) : 0;
  const red = h.tests.by_state?.red ?? 0;
  const planned = h.tests.by_state?.planned ?? 0;
  const adrs = sum(h.architecture.adrs);
  const adrSub = Object.entries(h.architecture.adrs ?? {})
    .map(([s, n]) => `${s} ${n}`)
    .join(" · ");
  return (
    <>
      <h3>Items</h3>
      <div className="overview-tiles">
        <StatTile label="items" value={h.items.total} to={items()} />
        {Object.entries(h.items.by_kind ?? {}).map(([k, n]) => (
          <StatTile key={k} label={k} value={n} to={items({ kind: k })} />
        ))}
        {h.items.untyped > 0 && <StatTile label="untyped" value={h.items.untyped} to={items({ kind: "none" })} tone="warn" />}
      </div>
      <h3>Requirements and tests</h3>
      <div className="overview-tiles">
        <StatTile label="requirements" value={h.requirements.total} to={items({ evidence: "unproven" })} sub={`${h.requirements.functional} functional · ${h.requirements.nfr} NFR`} />
        <StatTile label="requirements proven" value={`${pct}%`} to={items({ evidence: "unproven" })} sub={`${h.coverage.proven} of ${h.coverage.total}`} />
        <StatTile label="red tests" value={red} to={items({ evidence: "red" })} tone={red > 0 ? "error" : undefined} />
        <StatTile label="planned tests" value={planned} to={items({ evidence: "planned" })} />
        <StatTile label="items with stale evidence" value={h.coverage.stale_items} to={items({ evidence: "stale" })} tone={h.coverage.stale_items > 0 ? "warn" : undefined} />
      </div>
      <h3>Flow</h3>
      <div className="overview-tiles">
        <StatTile
          label="defects escaped"
          value={`${h.flow.escaped} of ${h.flow.defects}`}
          to={items({ kind: "defect" })}
          tone={h.flow.escaped > 0 ? "error" : undefined}
        />
        <AskTile label="cycle time, median" value={days(h.flow.cycle_p50_days)} sub={`p90 ${days(h.flow.cycle_p90_days)}`} ask={FLOW_ASK} />
        <AskTile label="items reworked" value={h.flow.rework} ask={FLOW_ASK} />
      </div>
      <h3>Architecture and records</h3>
      <div className="overview-tiles">
        <StatTile label="ADRs" value={adrs} to={to.docsFolder(pr, "decisions")} sub={adrSub || undefined} />
        <StatTile label="design docs" value={h.architecture.design_docs} to={to.docsFolder(pr, "design")} />
        <StatTile label="component docs" value={h.architecture.component_docs} to={to.docsFolder(pr, "design/components")} />
        <StatTile label="retros" value={h.records.retros} to={to.docsFolder(pr, "retros")} />
        <StatTile label="release notes" value={h.records.release_notes} to={to.docsFolder(pr, "releases")} />
        <StatTile label="lessons" value={h.records.lessons} to={to.lessons(pr)} />
        <StatTile label="releases" value={h.records.releases} to={to.releases(pr)} />
      </div>
    </>
  );
}

function goalColumns(project: string): Column<GoalFacts>[] {
  return [
    { key: "goal", label: "Goal", value: (g) => g.id, render: (g) => <Link to={to.items(project, { goal: g.id })}>{g.id}</Link>, sortable: true, narrow: "keep" },
    { key: "title", label: "Title", value: (g) => g.title, sortable: true, narrow: "keep" },
    { key: "done", label: "Done", value: (g) => g.items_done, sortable: true, align: "end" },
    { key: "open", label: "Open", value: (g) => g.items_open, sortable: true, align: "end" },
    {
      key: "proven",
      label: "Requirements proven",
      value: (g) => (g.req_total > 0 ? g.req_proven / g.req_total : null),
      render: (g) => (
        <ProgressBar
          label={`${g.id} requirements`}
          total={g.req_total}
          segments={[
            { key: "proven", label: "Proven", value: g.req_proven, tone: "ok" },
            { key: "open", label: "Not yet", value: Math.max(0, g.req_total - g.req_proven), tone: "neutral" },
          ]}
        />
      ),
    },
  ];
}

function RecentSection(p: { recent: Recent[]; today: string }): JSX.Element {
  if (p.recent.length === 0) return <Empty title="No decisions recorded yet." />;
  const oldest = p.recent.reduce((a, r) => (r.at < a ? r.at : a), p.recent[0].at).slice(0, 10);
  const marks = p.recent.map((r) => ({ at: r.at.slice(0, 10), label: r.code ?? r.kind, kind: (r.kind === "release-stage" || r.kind === "shipped" ? "diamond" : "dot") as "diamond" | "dot" }));
  return (
    <div className="overview-recent">
      <Timeline from={oldest} to={p.today} today={p.today} caption="Recent decisions and ADRs" lanes={[{ key: "recent", label: "Decisions", bands: [], marks }]} />
      <ul className="overview-list">
        {p.recent.map((r, i) => (
          <li key={`${r.at}-${i}`}>
            <time dateTime={r.at}>{r.at.slice(0, 10)}</time> <Link to={r.to}>{r.words}</Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ActivitySection(p: { project: string; overview: Overview }): JSX.Element {
  const lines = p.overview.activity ?? [];
  if (lines.length === 0) return <Empty title="No activity yet." />;
  const now = new Date();
  return (
    <ul className="overview-list">
      {lines.map((l, i) => {
        const code = l.item && l.words.startsWith(l.item) ? l.item : null;
        return (
          <li key={`${l.at}-${i}`}>
            {code ? (
              <>
                <Link to={to.item(p.project, code)}>{code}</Link>
                {l.words.slice(code.length)}
              </>
            ) : (
              l.words
            )}{" "}
            <time className="muted small" dateTime={l.at}>
              {ago(l.at, now)}
            </time>
          </li>
        );
      })}
    </ul>
  );
}

/** The page without its reads, so web::check_overview_* can render it from a fixture. */
export function OverviewBody(p: {
  project: string;
  name: string;
  overview: { data: Overview | null; loading: boolean; error: string | null; reload?(): void };
  drift: DriftLatest | null;
  /** Whether the project runs sprints and releases; null while the index is unknown. */
  cadence: { sprints: boolean; releases: boolean } | null;
  /** Today's ISO date, the end of the Recent timeline. */
  today: string;
}): JSX.Element {
  const o = p.overview.data;
  const head = (
    <div className="page-head">
      <h1>{p.name}</h1>
      <LiveDot />
    </div>
  );
  // A section is drawn only once the read has data and no error (A1 shared API §3), so loading is
  // never shown as "Nothing needs you".
  if (p.overview.loading || (p.overview.error && !o) || !o) {
    return (
      <div className="overview-page">
        {head}
        {p.overview.error ? <ErrorState error={p.overview.error} retry={p.overview.reload} /> : <LoadingState />}
      </div>
    );
  }
  const asOf = [o.board_rev ? `as of ${o.board_rev.slice(0, 7)}` : null, new Date(o.generated_at).toLocaleString()].filter(Boolean).join(", ");
  return (
    <div className="overview-page">
      {head}
      <p className="muted small overview-asof">{asOf}</p>
      <Section title="Needs you">
        <NeedsYouCards overview={o} drift={p.drift} />
      </Section>
      <Section title="Now">
        <NowSection project={p.project} overview={o} cadence={p.cadence} />
      </Section>
      <Section title="What the board holds">
        <HoldsSection project={p.project} overview={o} />
      </Section>
      <Section title="Goals">
        {(o.goals ?? []).length > 0 ? (
          <DataTable rows={o.goals ?? []} columns={goalColumns(p.project)} rowKey={(g) => g.id} caption="Goals" csvName="goals" stateKey="goals" empty={<Empty title="No goals match." />} />
        ) : (
          <Empty title="The charter has no goals yet." ask="Ask Claude to draft the charter's goals with you." />
        )}
      </Section>
      <Section title="Recent decisions and ADRs">
        <RecentSection recent={o.recent ?? []} today={p.today} />
      </Section>
      <Section title="Activity">
        <ActivitySection project={p.project} overview={o} />
      </Section>
    </div>
  );
}
