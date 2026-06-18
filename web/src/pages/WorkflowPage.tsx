import { useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import type { ProjectConfig } from "../types";

/**
 * Read-only Workflow view (FEAT-039). The per-project config is the single source of truth; this
 * page renders it as a Mermaid `stateDiagram-v2` *client-side* (the same flat format the CLI's
 * `config workflow --to-mermaid` exports) and draws it via the Mermaid-capable Markdown component.
 */
export default function WorkflowPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getProject(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const diagram = toStateDiagram(data.config);

  return (
    <div className="workflow-page">
      <div className="page-head">
        <h1>Workflow</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        Generated from the project config (statuses, transitions, default/terminal/no-op states).
      </p>
      <Markdown source={"```mermaid\n" + diagram + "```\n"} />
    </div>
  );
}

/** True when `s` is usable verbatim as a Mermaid state id (alphanumerics or `_`). */
function isPlainId(s: string): boolean {
  return s.length > 0 && /^[A-Za-z0-9_]+$/.test(s);
}

/** A stable, valid Mermaid id for a status name (matches the Rust `id_for`). */
function idFor(name: string): string {
  if (isPlainId(name)) return name;
  let id = name.replace(/[^A-Za-z0-9]/g, "_");
  if (id.length === 0 || /^[0-9]/.test(id)) id = "s" + id;
  return id;
}

/**
 * Render a project config as a Mermaid `stateDiagram-v2`. Mirrors the Rust `mermaid::to_state_diagram`
 * so the web view matches the CLI export: start edge, transitions, terminal edges, no-op notes, and
 * `state "Name" as id` aliasing for status names that aren't plain ids.
 */
function toStateDiagram(config: ProjectConfig): string {
  const statuses = config.statuses ?? [];
  const terminals = config.terminal_states ?? [];
  const noOps = config.no_op_states ?? [];
  const transitions = config.transitions ?? {};

  const ids = new Map<string, string>();
  for (const s of statuses) if (!ids.has(s)) ids.set(s, idFor(s));
  const idOf = (name: string) => ids.get(name) ?? idFor(name);

  let out = "stateDiagram-v2\n";

  // Alias declarations for non-plain names, sorted for stable output.
  for (const name of [...ids.keys()].sort()) {
    const id = idOf(name);
    if (id !== name) out += `    state "${name}" as ${id}\n`;
  }

  if (config.default_state) out += `    [*] --> ${idOf(config.default_state)}\n`;

  for (const from of Object.keys(transitions).sort()) {
    for (const to of transitions[from]) out += `    ${idOf(from)} --> ${idOf(to)}\n`;
  }

  for (const t of terminals) out += `    ${idOf(t)} --> [*]\n`;

  for (const n of noOps) out += `    note right of ${idOf(n)}: no-op (inert disposition)\n`;

  return out;
}
