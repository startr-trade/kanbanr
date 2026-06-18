import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import Markdown from "../components/Markdown";
import { ErrorBox, Loading, LiveDot } from "../components/bits";

/** Render a single documentation markdown file. */
export default function DocPage() {
  const params = useParams();
  const project = params.project ?? "";
  const path = params["*"] ?? "";
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getDoc(project, path), [project, path, tick]);

  const base = `/p/${encodeURIComponent(project)}/docs`;
  const parentPath = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";

  if (loading && data === null) return <Loading />;
  if (error) return <ErrorBox error={error} />;

  return (
    <div className="doc-page">
      <div className="page-head">
        <nav className="docs-crumbs">
          <Link to={base}>docs</Link>
          {parentPath && (
            <>
              <span className="sep">/</span>
              <Link to={`${base}/folder/${parentPath}`}>{parentPath}</Link>
            </>
          )}
          <span className="sep">/</span>
          <span className="mono">{path.split("/").pop()}</span>
        </nav>
        <LiveDot />
      </div>
      <article className="doc-article">
        <Markdown
          source={data ?? ""}
          resolveImage={(src) => api.docRawUrl(project, parentPath ? `${parentPath}/${src}` : src)}
        />
      </article>
    </div>
  );
}
