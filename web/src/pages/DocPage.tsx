import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import Markdown from "../components/Markdown";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import type { DocFolder } from "../types";

/** Whether a doc's first heading is an h1 (`# Title`, or `Title` over `===`), outside code fences. */
export function startsWithH1(source: string): boolean {
  let fenced = false;
  const lines = source.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s{0,3}(```|~~~)/.test(line)) fenced = !fenced;
    if (fenced) continue;
    const atx = /^\s{0,3}(#{1,6})(\s|$)/.exec(line);
    if (atx) return atx[1].length === 1;
    if (line.trim() && i + 1 < lines.length) {
      if (/^\s{0,3}=+\s*$/.test(lines[i + 1])) return true;
      if (/^\s{0,3}-+\s*$/.test(lines[i + 1])) return false;
    }
  }
  return false;
}

/** A doc's title in the doc tree, if the tree lists it. */
function treeTitle(folder: DocFolder | null, path: string): string | null {
  if (!folder) return null;
  const hit = folder.docs.find((d) => d.path === path);
  if (hit) return hit.title;
  for (const sub of folder.folders) {
    const t = treeTitle(sub, path);
    if (t) return t;
  }
  return null;
}

/**
 * Render a single documentation markdown file. The page has one h1 (DEF-082): a doc that starts
 * with its own `# Title` provides it, and is rendered with its headings as written; any other doc
 * gets an h1 from its title in the doc tree, or its file name, with its own headings shifted down.
 */
export default function DocPage() {
  const params = useParams();
  const project = params.project ?? "";
  const path = params["*"] ?? "";
  const tick = useLiveTick(api.projectEvents(project));
  const { data, error, loading } = useAsync(() => api.getDoc(project, path), [project, path, tick]);
  const ownH1 = data !== null && startsWithH1(data);
  // The doc tree is read only for a doc that brings no h1 of its own.
  const tree = useAsync(
    () => (data !== null && !ownH1 ? api.getDocTree(project) : Promise.resolve(null)),
    [project, data !== null && !ownH1, tick],
  );

  const base = `/p/${encodeURIComponent(project)}/docs`;
  const parentPath = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";

  if (loading && data === null) return <Loading />;
  if (error) return <ErrorBox error={error} />;
  const fileName = path.split("/").pop() ?? path;
  const title = treeTitle(tree.data, path) ?? fileName.replace(/\.md$/i, "");

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
        {!ownH1 && <h1>{title}</h1>}
        <Markdown
          source={data ?? ""}
          headingOffset={ownH1 ? 0 : 1}
          resolveImage={(src) => api.docRawUrl(project, parentPath ? `${parentPath}/${src}` : src)}
        />
      </article>
    </div>
  );
}
