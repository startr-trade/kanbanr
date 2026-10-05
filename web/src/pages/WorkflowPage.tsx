import { useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { conditionText } from "../types";
import type { Gate, ProcessStatus } from "../types";

/**
 * Read-only Workflow view (FEAT-039, FEAT-117). The per-project config is the single source of
 * truth. The diagram is drawn by the daemon — the same one `config workflow --to-mermaid` prints,
 * gates as notes — rather than by a copy of the exporter here, which had to be kept in step by
 * hand. Below it, each stage with what entering it asks for.
 */
export default function WorkflowPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);
  const diagram = useAsync(() => api.getWorkflowDiagram(project), [project, tick]);
  const process = useAsync(() => api.getProcess(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const config = data.config;
  const gates = config.gates ?? {};
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
