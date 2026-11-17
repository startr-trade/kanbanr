import { Link, useParams } from "react-router-dom";
import { useQuery, type Query } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { PositionPicker, usePick } from "../components/PositionPicker";
import { repoDocsHref } from "../position";
import type { DocFolder, PickView } from "../types";

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
  // The doc tree through the shared cache, re-read only when a doc changes (DEF-081).
  const tree = useQuery<DocFolder>(project, urls.docTree(project), TAGS.docTree);
  // The position the repository's docs are read at (FTR-004); the board's docs have one version.
  const pick = usePick();
  const picks = useQuery<PickView[]>(project, urls.positions(project), TAGS.positions);
  return <DocsBody project={project} folderPath={folderPath} tree={tree} pick={pick} picks={picks.data} />;
}

/** What the page draws, given its reads. The picker is in the header, whatever the reads answer. */
export function DocsBody({
  project,
  folderPath,
  tree,
  pick,
  picks,
}: {
  project: string;
  folderPath: string;
  tree: Pick<Query<DocFolder>, "data" | "error" | "loading">;
  pick: string;
  picks: PickView[] | null;
}) {
  const { data, error, loading } = tree;
  const header = (
    <div className="page-head">
      <h1>Documentation</h1>
      <PositionPicker picks={picks} pick={pick} />
      <LiveDot />
    </div>
  );
  const folder = data ? findFolder(data, folderPath) : null;
  if (loading && !data) return <>{header}<Loading /></>;
  if (error) return <>{header}<ErrorBox error={error} /></>;
  if (!data) return header;
  if (!folder) return <>{header}<ErrorBox error={`Folder '${folderPath}' not found`} /></>;

  const base = `/p/${encodeURIComponent(project)}/docs`;
  const crumbs = folderPath ? folderPath.split("/") : [];

  return (
    <div className="docs-page">
      {header}
      <p className="muted small">
        These are the board's docs, one version each. <Link to={repoDocsHref(project, pick)}>The repository's docs at {pick}</Link>{" "}
        are read from the position picked.
      </p>

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
              <span className="folder-icon" aria-hidden="true">📁</span>
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
              <Link to={`${base}/file/${d.path}`}><span aria-hidden="true">📄</span> {d.title}</Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
