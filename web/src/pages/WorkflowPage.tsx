import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { conditionText } from "../types";
import type { Gate, GitModelView, ProcessStatus } from "../types";
import { cadenceSection, doctorWarning, gitModelRows, guardRows } from "../workflowSections";
import type { CadenceSection, GuardRow, Row, Tone } from "../workflowSections";

/**
 * Read-only Workflow view (FEAT-039, FEAT-117). The per-project config is the single source of
 * truth. The diagram is drawn by the daemon — the same one `config workflow --to-mermaid` prints,
 * gates as notes — rather than by a copy of the exporter here, which had to be kept in step by
 * hand. Below it, each stage with what entering it asks for; then how work moves from an item
 * branch to a release, and every guard rail that refuses a step on the way (FEAT-213).
 */
export default function WorkflowPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);
  const diagram = useAsync(() => api.getWorkflowDiagram(project), [project, tick]);
  const process = useAsync(() => api.getProcess(project), [project, tick]);
  // What each stage actually asks (FEAT-199): the raw config misses role-addressed gates.
  const effective = useAsync(() => api.getGates(project), [project, tick]);
  const usesSprints = data?.config.cadence?.sprints === true;
  const usesReleases = data?.config.cadence?.releases === true;
  const sprints = useAsync(
    () => (usesSprints ? api.getSprints(project) : Promise.resolve([])),
    [project, tick, usesSprints],
  );
  const releases = useAsync(
    () => (usesReleases ? api.getReleases(project) : Promise.resolve([])),
    [project, tick, usesReleases],
  );
  // From the code repository when the daemon was started in it; otherwise the model the cadence
  // alone gives, with no branches read (FEAT-213).
  const gitModel = useAsync(
    () =>
      api
        .getGitModel(project)
        .then((view) => ({ view, fromRepo: true }))
        .catch(() =>
          api.getGitModelPreview(project, usesSprints, usesReleases).then((view) => ({ view, fromRepo: false })),
        ),
    [project, tick, usesSprints, usesReleases],
  );
  const fromRepo = gitModel.data?.fromRepo === true;
  const gitDoctor = useAsync(
    () => (fromRepo ? api.getGitDoctor(project) : Promise.resolve([])),
    [project, tick, fromRepo],
  );
  const guards = useAsync(() => api.getGuards(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const config = data.config;
  const gates = effective.data ?? {};
  const staged = config.statuses.filter((s) => gates[s]);

  return (
    <div className="workflow-page">
      <div className="page-head">
        <h1>Workflow</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        Generated from the project config (statuses, transitions, default/terminal/no-op states, and
        the gates on each stage).
      </p>
      {process.data ? <ProcessLine status={process.data} /> : null}
      {diagram.error ? (
        <ErrorBox error={diagram.error} />
      ) : diagram.data ? (
        <Markdown source={"```mermaid\n" + diagram.data + "```\n"} />
      ) : (
        <Loading />
      )}

      <section className="section">
        <h2>Stages</h2>
        {staged.length === 0 ? (
          <p className="muted">
            This workflow declares no gates, so kanbanr's built-in rule applies: moving an item into
            a status that means work has started needs an approved definition.
          </p>
        ) : (
          <div className="tiles">
            {staged.map((status) => (
              <StageTile key={status} status={status} gate={gates[status]} />
            ))}
          </div>
        )}
      </section>

      <CadenceView cadence={cadenceSection(project, config, sprints.data ?? [], releases.data ?? [])} />

      <section className="section">
        <h2>Git model</h2>
        {gitModel.error ? (
          <ErrorBox error={gitModel.error} />
        ) : gitModel.data ? (
          <GitModelSection
            view={gitModel.data.view}
            fromRepo={gitModel.data.fromRepo}
            warning={doctorWarning(gitDoctor.data ?? [])}
          />
        ) : (
          <Loading />
        )}
      </section>

      <section className="section">
        <h2>Guard rails</h2>
        <p className="muted small">
          What refuses a step on the way from an item branch to a release. Each line names the command
          that changes it; the monitor changes nothing.
        </p>
        {guards.error ? (
          <ErrorBox error={guards.error} />
        ) : guards.data ? (
          <GuardList rows={guardRows(guards.data)} />
        ) : (
          <Loading />
        )}
      </section>
    </div>
  );
}

const TONE_CHIP: Record<Tone, string> = { on: "chip done", off: "chip", warn: "chip warn", unknown: "chip tasks" };

/** A state chip and the command that changes it. */
function StateAndCommand({ row }: { row: Row }) {
  return (
    <>
      {row.tone ? <span className={TONE_CHIP[row.tone]}>{row.value}</span> : <span>{row.value}</span>}
      {row.command ? (
        <span className="muted small">
          {" "}
          <code>{row.command}</code>
        </span>
      ) : null}
    </>
  );
}

function Facts({ rows }: { rows: Row[] }) {
  return (
    <dl className="facts">
      {rows.map((r) => (
        <div className="facts-row" key={r.label}>
          <dt>{r.label}</dt>
          <dd>
            <StateAndCommand row={r} />
          </dd>
        </div>
      ))}
    </dl>
  );
}

/** Sprints and releases, when the project has either (FEAT-213). */
function CadenceView({ cadence }: { cadence: CadenceSection | null }) {
  if (!cadence) return null;
  return (
    <section className="section">
      <h2>Cadence</h2>
      <div className="tiles">
        {cadence.sprints ? (
          <div className="tile">
            <div className="tile-title">Sprints</div>
            <Facts rows={cadence.sprints.rows} />
            {cadence.sprints.active ? (
              <p className="small">
                Running: <Link to={cadence.sprints.active.to}>{cadence.sprints.active.code}</Link>
                {cadence.sprints.active.name ? ` ${cadence.sprints.active.name}` : ""}{" "}
                <span className="muted">({cadence.sprints.active.dates})</span>
              </p>
            ) : (
              <p className="muted small">
                No sprint is running: <code>kanbanr sprint start &lt;SP&gt;</code>
              </p>
            )}
          </div>
        ) : null}
        {cadence.releases ? (
          <div className="tile">
            <div className="tile-title">Releases</div>
            <Facts rows={cadence.releases.rows} />
            <div className="tile-states">
              {cadence.releases.stages.map((s) => (
                <span className={s.cut ? "chip done" : "chip"} key={s.name} title={s.cut ? "the cut" : undefined}>
                  {s.name}
                  {s.cut ? " · the cut" : ""}
                </span>
              ))}
            </div>
            {cadence.releases.open.length === 0 ? (
              <p className="muted small">
                No release planned: <code>kanbanr release add &lt;version&gt;</code>
              </p>
            ) : (
              <ul className="guard-list">
                {cadence.releases.open.map((r) => (
                  <li key={r.version}>
                    <Link to={r.to}>{r.version}</Link>
                    <span className="chip">{r.stage}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        ) : null}
      </div>
    </section>
  );
}

/** The git model in words, and what git doctor found (FEAT-213). */
function GitModelSection({
  view,
  fromRepo,
  warning,
}: {
  view: GitModelView;
  fromRepo: boolean;
  warning: { count: number; first: string[] } | null;
}) {
  return (
    <>
      {fromRepo ? null : (
        <p className="muted small">
          From the cadence alone: this monitor was not started in the project's code repository, so no
          branches were read. Start <code>kanbanr serve</code> in the project folder to see them.
        </p>
      )}
      {warning ? (
        <div className="warn-box">
          <strong>
            git doctor finds {warning.count} problem{warning.count === 1 ? "" : "s"}
          </strong>
          <ul>
            {warning.first.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
          <span className="muted small">
            All of them: <code>kanbanr git doctor</code>
          </span>
        </div>
      ) : null}
      <Facts rows={gitModelRows(view)} />
    </>
  );
}

/** Every guard rail, grouped, each with its state and command (FEAT-213). */
function GuardList({ rows }: { rows: GuardRow[] }) {
  const groups = [...new Set(rows.map((r) => r.group))];
  return (
    <div className="tiles">
      {groups.map((group) => (
        <div className="tile" key={group}>
          <div className="tile-title">{group}</div>
          <ul className="guard-list">
            {rows
              .filter((r) => r.group === group)
              .map((r) => (
                <li key={r.label}>
                  <span>{r.label}</span>
                  <StateAndCommand row={r} />
                </li>
              ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

function StageTile({ status, gate }: { status: string; gate: Gate }) {
  const requires = gate.requires ?? [];
  const warns = gate.warns ?? [];
  const signoffs = gate.signoffs ?? [];
  return (
    <div className="tile">
      <div className="tile-title">{status}</div>
      {gate.purpose ? <div className="tile-desc">{gate.purpose}</div> : null}
      <div className="tile-states">
        {requires.map((c) => (
          <span className="chip" key={`r-${conditionText(c)}`}>
            {conditionText(c)}
          </span>
        ))}
        {signoffs.map((s) => (
          <span className="chip tasks" key={`s-${s}`}>
            sign-off {s}
          </span>
        ))}
        {warns.map((c) => (
          <span className="chip warn" key={`w-${conditionText(c)}`} title="reported, not enforced">
            warns: {conditionText(c)}
          </span>
        ))}
        {gate.enforce === "warn" ? <span className="chip warn">warn only</span> : null}
        {(gate.on_enter ?? []).includes("branch") ? <span className="chip">branch here</span> : null}
        {(gate.roles ?? []).map((r) => (
          <span className="chip tasks" key={`role-${r}`} title="what this stage is for, in any process">
            role: {r}
          </span>
        ))}
        {(gate.affects_docs ?? []).map((d) => (
          <span className="chip tasks" key={`doc-${d}`} title="every item entering lists this doc in affects_docs">
            lists {d}
          </span>
        ))}
        {requires.length + signoffs.length + warns.length === 0 ? (
          <span className="muted small">asks nothing to enter</span>
        ) : null}
      </div>
    </div>
  );
}

/**
 * The saved process this workflow came from, and whether it has moved on (FEAT-170). Told, never
 * applied: `kanbanr process update` brings the new version when the project's user decides to.
 */
function ProcessLine({ status }: { status: ProcessStatus }) {
  const source = status.source;
  if (!source) return null;
  const library = source.library === "builtin" ? "built-in" : source.library;
  const version = source.version > 0 ? ` v${source.version}` : "";
  const personal = source.library === "personal";
  return (
    <div className="section">
      <p className="small">
        Process: <strong>{source.name}</strong>
        {version} · {library}{" "}
        {personal ? (
          <span className="muted">— a personal process, compared on its owner's machine (kanbanr process status)</span>
        ) : status.messages.length === 0 ? (
          <span className="chip done">current</span>
        ) : (
          <span className="chip warn">changed</span>
        )}
      </p>
      {status.messages.map((m) => (
        <p className="muted small" key={m}>
          {m.split("`").map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : part))}
        </p>
      ))}
    </div>
  );
}
