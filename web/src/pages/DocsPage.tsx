import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { DocFolder } from "../types";

function findFolder(root: DocFolder, path: string): DocFolder | null {
  if (path === "" || path === root.path) return root;
  if (root.path === path) return root;
  for (const f of root.folders) {
    if (f.path === path) return f;
    const hit = findFolder(f, path);
    if (hit) return hit;
  }
  return null;
}

/** Documentation index + folder drilldown: sub-folders render as tiles, files as links. */
export default function DocsPage() {
  const params = useParams();
  const project = params.project ?? "";
  const folderPath = (params["*"] ?? "").replace(/\/$/, "");
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getDocTree(project), [project, tick]);

  if (loading && !data) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  if (!data) return null;

  const folder = findFolder(data, folderPath);
  if (!folder) return <ErrorBox error={`Folder '${folderPath}' not found`} />;

  const base = `/p/${encodeURIComponent(project)}/docs`;
  const crumbs = folderPath ? folderPath.split("/") : [];

  return (
    <div className="docs-page">
      <div className="page-head">
        <h1>Documentation</h1>
        <LiveDot />
      </div>

      <nav className="docs-crumbs">
        <Link to={base}>docs</Link>
        {crumbs.map((c, i) => {
          const sub = crumbs.slice(0, i + 1).join("/");
          return (
            <span key={sub}>
              <span className="sep">/</span>
              <Link to={`${base}/folder/${sub}`}>{c}</Link>
            </span>
          );
        })}
      </nav>

      {folder.path !== "" && (
        <p className="muted">{folder.description || `Folder: ${folder.name}`}</p>
      )}

      {folder.folders.length === 0 && folder.docs.length === 0 && (
        <p className="muted">
          Empty. Add docs with the CLI: <code>kanbanr doc add {folderPath ? folderPath + "/" : ""}notes.md --file notes.md</code>
        </p>
      )}

      {folder.folders.length > 0 && (
        <div className="tiles">
          {folder.folders.map((sub) => (
            <Link className="tile folder-tile" key={sub.path} to={`${base}/folder/${sub.path}`}>
              <span className="folder-icon">📁</span>
              <span className="tile-title">{sub.name}</span>
              {sub.description && <p className="tile-desc">{sub.description}</p>}
              <span className="muted small">
                {sub.folders.length} folder{sub.folders.length === 1 ? "" : "s"}, {sub.docs.length} doc
                {sub.docs.length === 1 ? "" : "s"}
              </span>
            </Link>
          ))}
        </div>
      )}

      {folder.docs.length > 0 && (
        <ul className="doc-files">
          {folder.docs.map((d) => (
            <li key={d.path}>
              <Link to={`${base}/file/${d.path}`}>📄 {d.title}</Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
