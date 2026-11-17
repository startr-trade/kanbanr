import { Link, useParams } from "react-router-dom";
import { useQuery, type Query } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { api } from "../api";
import Markdown from "../components/Markdown";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { PositionPicker, usePick } from "../components/PositionPicker";
import { startsWithH1 } from "./DocPage";
import { missingLine, positionLine, repoDocsHref } from "../position";
import type { PickView, RepoDoc } from "../types";

/** `src` resolved against the doc's folder, so `diagram.png` beside `docs/guide/x.md` is `docs/guide/diagram.png`. */
function beside(dir: string, src: string): string {
  const parts = dir ? dir.split("/") : [];
  for (const seg of src.split("/")) {
    if (seg === "..") parts.pop();
    else if (seg !== "." && seg !== "") parts.push(seg);
  }
  return parts.join("/");
}

/**
 * One repository doc at the position picked (FTR-004). The banner above the text says what was
 * read (path, position, git or disk); a doc that is not there at the position says so and whether
 * main has it, and the picker stays so the person can choose another.
 */
export default function RepoDocPage() {
  const params = useParams();
  const project = params.project ?? "";
  const path = params["*"] ?? "";
  const pick = usePick();
  const doc = useQuery<RepoDoc>(project, urls.repoDoc(project, path, pick), TAGS.repoDocs, () =>
    api.getRepoDoc(project, path, pick),
  );
  const picks = useQuery<PickView[]>(project, urls.positions(project), TAGS.positions);
  return <RepoDocBody project={project} path={path} pick={pick} doc={doc} picks={picks.data} />;
}

/** What the page draws, given its reads. The picker is in the header, whatever the reads answer. */
export function RepoDocBody({
  project,
  path,
  pick,
  doc,
  picks,
}: {
  project: string;
  path: string;
  pick: string;
  doc: Pick<Query<RepoDoc>, "data" | "error" | "loading">;
  picks: PickView[] | null;
}) {
  const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const found = doc.data && !doc.data.missing && doc.data.text !== null ? doc.data : null;
  const ownH1 = found !== null && startsWithH1(found.text ?? "");
  return (
    <div className="doc-page">
      <div className="page-head">
        <nav className="docs-crumbs">
          <Link to={repoDocsHref(project, pick)}>repository docs</Link>
          <span className="sep">/</span>
          <span className="mono">{path}</span>
        </nav>
        <PositionPicker picks={picks} pick={pick} />
        <LiveDot />
      </div>
      {!ownH1 && (
        <h1>
          {found
            ? (path.split("/").pop() ?? path).replace(/\.md$/i, "")
            : path.split("/").pop() ?? path}
        </h1>
      )}
      {doc.error ? (
        <ErrorBox error={doc.error} />
      ) : doc.loading && !doc.data ? (
        <Loading />
      ) : doc.data?.missing ? (
        <p className="ui-position-banner" role="status">
          {missingLine(doc.data)} Choose another position above.
        </p>
      ) : found ? (
        <>
          <p className="ui-position-banner" role="status">
            {positionLine(found)}
          </p>
          <article className="doc-article">
            <Markdown
              source={found.text ?? ""}
              at={pick}
              headingOffset={ownH1 ? 0 : 1}
              resolveImage={(src) => api.repoDocRawUrl(project, beside(dir, src), pick)}
            />
          </article>
        </>
      ) : null}
    </div>
  );
}
