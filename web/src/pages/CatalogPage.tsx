/**
 * The project's content catalog (FTR-005): what this project must document, by question and
 * perspective, each row with whether it applies, the state of its source, its notations and its
 * drift protections. The gaps come first. Read-only: the page takes no action; the catalog and the
 * facts are kept by Claude through `kanbanr catalog` and `kanbanr facts`.
 */
import { Link, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { apiUrl } from "../runtimeConfig";
import { LiveDot } from "../components/bits";
import { Empty, ErrorState, LoadingState, Section } from "../ui";
import { chips, factsLine, gapSentence, groupRows, hasCatalog, hasFacts, isOff, sourceHref, sourceText, type Chip } from "../catalogView";
import { catalogChip } from "../systemBaselineView";
import type { CatalogStatus, FactsOrEmpty, RowStatus, SystemBaselineReview, SystemBaselineState } from "../types";

/** The watcher's slices the catalog's status is derived from: the catalog, the facts and the docs. */
const CATALOG_TAGS = ["catalog", "facts", "docs", "config"] as const;
const FACTS_TAGS = ["facts"] as const;

export default function CatalogPage() {
  const { project = "" } = useParams();
  const enc = encodeURIComponent(project);
  const catalog = useQuery<CatalogStatus>(project, apiUrl(`/projects/${enc}/catalog`), CATALOG_TAGS);
  const facts = useQuery<FactsOrEmpty>(project, apiUrl(`/projects/${enc}/facts`), FACTS_TAGS);

  // The system baseline's chips on the system rows (FTR-006); no chip while it is not read.
  const baseline = useQuery<SystemBaselineReview>(project, urls.systemBaseline(project), TAGS.systemBaseline);

  if (catalog.loading && !catalog.data) return <LoadingState />;
  if (catalog.error && !catalog.data) return <ErrorState error={catalog.error} retry={catalog.reload} />;
  if (!catalog.data) return null;
  return (
    <CatalogBody
      project={project}
      status={catalog.data}
      facts={facts.loading ? undefined : facts.data}
      baseline={baseline.data?.state ?? null}
    />
  );
}

/** The page without its reads, so web::catalog can render it. `facts` undefined: not read yet. */
export function CatalogBody({
  project,
  status,
  facts,
  baseline = null,
}: {
  project: string;
  status: CatalogStatus;
  facts: FactsOrEmpty | null | undefined;
  /** The system baseline's state, for the system rows' chips (FTR-006); null: none shown. */
  baseline?: SystemBaselineState | null;
}) {
  const groups = groupRows(status.rows);
  const gaps = gapSentence(status.gaps);
  const problems = status.problems ?? [];
  const any = hasCatalog(status);

  return (
    <div className="catalog-page">
      <div className="page-head">
        <h1>Catalog</h1>
        <LiveDot />
      </div>
      <p className="muted small">
        What this project must document, by question (what, how, where, when, who, why) and by perspective (sponsor,
        user, architect, builder, operator): each row says which content answers it, where that content comes from and
        what keeps it true.
      </p>

      {facts === undefined ? null : hasFacts(facts) ? (
        <p className="small">
          <strong>Facts</strong> · {factsLine(facts)}
        </p>
      ) : (
        <Empty
          title="No project facts yet: rows that depend on what the project is read as not applying."
          ask="Ask your agent to set the project's facts: its languages, runtimes, where it runs, its parts and its data."
        >
          <p className="muted small">
            The agent records them with <code>kanbanr facts set</code>.
          </p>
        </Empty>
      )}

      {!any ? (
        <Empty title="No catalog yet." ask="Ask your agent to set up the project's content catalog from the starter.">
          <p className="muted small">
            The agent writes it with <code>kanbanr catalog init</code>.
          </p>
        </Empty>
      ) : (
        <>
          <Section title="Gaps">
            {gaps ? (
              <p className="small">{gaps}</p>
            ) : (
              <p className="muted small">Every question has an applying row for every perspective.</p>
            )}
          </Section>

          {problems.length ? (
            <Section title="Problems">
              <ul className="small">
                {problems.map((p) => (
                  <li key={p}>
                    <span className="chip warn">problem</span> {p}
                  </li>
                ))}
              </ul>
            </Section>
          ) : null}

          {groups.map((g) => (
            <Section key={g.level} title={g.title}>
              {g.rows.map((r) => (
                <RowLine key={r.id} project={project} row={r} baseline={baseline} />
              ))}
            </Section>
          ))}
        </>
      )}
    </div>
  );
}

/** A chip's classes: the tone shows as colour and the label says it in words. */
function chipClass(c: Chip): string {
  if (c.tone === "ok") return "chip done";
  if (c.tone === "warn") return "chip warn";
  return "chip";
}

/** One row: its title and id, what it answers for whom, its source (linked where it has a page), its chips. */
function RowLine({ project, row, baseline }: { project: string; row: RowStatus; baseline: SystemBaselineState | null }) {
  const pill = catalogChip(row.id, baseline);
  const href = sourceHref(project, row);
  const source = sourceText(row);
  return (
    <div className={isOff(row) ? "section muted" : "section"} id={`row-${row.id}`}>
      <h3>
        {row.title} <code className="small">{row.id}</code>
      </h3>
      <p className="small">
        {row.answers.join(", ")} · for {row.for.join(", ")} ·{" "}
        {href ? <Link to={href}>{source}</Link> : <span>{source}</span>}
      </p>
      <p className="small">
        {chips(row).map((c, i) => (
          <span key={`${c.kind}-${i}`}>
            {i ? " " : ""}
            <span className={chipClass(c)} title={c.title}>
              {c.label}
            </span>
          </span>
        ))}
        {pill ? (
          <>
            {" "}
            <span className={pill.tone === "ok" ? "chip done" : pill.tone === "warn" ? "chip warn" : "chip"} title={pill.title}>
              baseline: {pill.label}
            </span>
          </>
        ) : null}
      </p>
    </div>
  );
}
