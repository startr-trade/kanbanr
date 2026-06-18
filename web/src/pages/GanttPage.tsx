import { useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";

/**
 * Read-only Gantt view (FEAT-035). The daemon computes the schedule (dates / dependency sequencing
 * + the critical path) server-side and returns it as a Mermaid `gantt` diagram; this page just
 * fetches that text and renders it through the Mermaid-capable Markdown component.
 */
export default function GanttPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getGantt(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  return (
    <div className="gantt-page">
      <div className="page-head">
        <h1>Gantt</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        Schedule derived from feature start dates, estimates, and the dependency DAG. Tasks on the
        critical path (the longest dependency chain) are highlighted.
      </p>
      <Markdown source={"```mermaid\n" + data + "```\n"} />
    </div>
  );
}
