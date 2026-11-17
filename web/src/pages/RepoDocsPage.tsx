import { Link, useParams } from "react-router-dom";
import { useQuery, type Query } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { api } from "../api";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { PositionPicker, usePick } from "../components/PositionPicker";
import { positionLine, repoDocHref } from "../position";
import type { PickView, RepoTree } from "../types";

/**
 * The repository's docs under `docs/` at the position picked (FTR-004). They are read from git or
 * from the working tree with nothing checked out, and the banner says which. The board's own docs
 * are on the Docs page; this is the code repository's.
 */
export default function RepoDocsPage() {
  const project = useParams().project ?? "";
  const pick = usePick();
  const tree = useQuery<RepoTree>(project, urls.repoDocs(project, pick), TAGS.repoDocs, () =>
    api.getRepoTree(project, pick),
  );
  const picks = useQuery<PickView[]>(project, urls.positions(project), TAGS.positions);
  return <RepoDocsBody project={project} pick={pick} tree={tree} picks={picks.data} />;
}

/** What the page draws, given its reads. The picker is in the header, whatever the reads answer. */
export function RepoDocsBody({
  project,
  pick,
  tree,
  picks,
}: {
  project: string;
  pick: string;
  tree: Pick<Query<RepoTree>, "data" | "error" | "loading">;
  picks: PickView[] | null;
}) {
  const board = `/p/${encodeURIComponent(project)}/docs`;
  return (
    <div className="docs-page">
      <div className="page-head">
        <h1>Repository docs</h1>
        <PositionPicker picks={picks} pick={pick} />
        <LiveDot />
      </div>
      <p className="muted small">
        The code repository's docs at the position picked. <Link to={board}>The board's own docs</Link> have one version.
      </p>
      {tree.error ? (
        <ErrorBox error={tree.error} />
      ) : tree.loading && !tree.data ? (
        <Loading />
      ) : tree.data ? (
        <>
          <p className="ui-position-banner" role="status">
            {positionLine(tree.data, tree.data.root)}
          </p>
          {tree.data.files.length === 0 ? (
            <p className="muted">No markdown files under {tree.data.root} at this position.</p>
          ) : (
            <ul className="doc-files">
              {tree.data.files.map((f) => (
                <li key={f}>
                  <Link to={repoDocHref(project, f, pick)}>
                    <span aria-hidden="true">📄</span> {f}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </>
      ) : null}
    </div>
  );
}
