/**
 * Lessons (FTR-076): how the project's learning grew, release cycle by release cycle, and every
 * lesson grouped by theme. The counts come from the server, by the rules `kanbanr lessons` and
 * `kanbanr lessons --all` use; the believed percentage is the server's number, not a client copy of
 * the decay. A theme opens to a table of its lessons, each linked to where it was learned and the
 * items that cite it; open themes live in the URL hash.
 */
import { Link, useLocation } from "react-router-dom";
import { AccordionRow, DataTable, Empty, GrowthChart, SeverityPill, useOpenRows, type Column } from "../../ui";
import { FOLD_ENTRY, fold, themeRows } from "../../charterModel";
import { to } from "../../overviewLinks";
import type { CharterProgress, LessonRow } from "../../types";

/** The value the status column sorts and filters by; "retired" is every lesson not in force. */
const statusOf = (r: LessonRow): string => (r.in_force ? "in-force" : "retired");

function LearnedFrom(p: { project: string; row: LessonRow }): JSX.Element {
  const { project, row } = p;
  const retro = row.from_retro
    ? row.retro_kind === "sprint"
      ? to.sprint(project, row.from_retro)
      : row.retro_kind === "milestone"
        ? to.milestone(project, row.from_retro)
        : null
    : null;
  return (
    <>
      {row.from_item && <Link to={to.item(project, row.from_item)}>{row.from_item}</Link>}
      {row.from_item && row.from_retro ? " · " : null}
      {row.from_retro && (retro ? <Link to={retro}>{row.from_retro}</Link> : row.from_retro)}
    </>
  );
}

function columns(project: string): Column<LessonRow>[] {
  return [
    { key: "id", label: "Lesson id", value: (r) => r.id, sortable: true, render: (r) => <code className="taskkey">{r.id}</code> },
    {
      key: "lesson",
      label: "Lesson",
      value: (r) => `${r.lesson} ${r.evidence ?? ""}`,
      render: (r) => {
        const { head, rest } = fold(r.lesson, FOLD_ENTRY);
        return (
          <>
            <span className="charter-clamp">{head}</span>
            {(rest || r.evidence) && (
              <details className="charter-more">
                <summary>Full text and evidence</summary>
                {rest && <p>{rest}</p>}
                {r.evidence && <p className="muted">Evidence: {r.evidence}</p>}
              </details>
            )}
          </>
        );
      },
    },
    { key: "kind", label: "Kind", value: (r) => r.kind, sortable: true },
    { key: "believed", label: "Believed", value: (r) => Math.round(r.confidence_now * 100), sortable: true, align: "end", render: (r) => `${Math.round(r.confidence_now * 100)}%` },
    {
      key: "goals",
      label: "Goals",
      value: (r) => (r.goals ?? []).join(" "),
      narrow: "hide",
      render: (r) => (
        <>
          {(r.goals ?? []).map((g, i) => (
            <span key={g}>
              {i > 0 ? ", " : ""}
              <Link to={`/p/${encodeURIComponent(project)}/charter#${encodeURIComponent(g)}`}>{g}</Link>
            </span>
          ))}
        </>
      ),
    },
    { key: "tags", label: "Tags", value: (r) => (r.tags ?? []).join(" "), narrow: "hide", render: (r) => (r.tags ?? []).join(", ") },
    { key: "from", label: "Learned from", value: (r) => `${r.from_item ?? ""} ${r.from_retro ?? ""}`.trim(), render: (r) => <LearnedFrom project={project} row={r} /> },
    {
      key: "cited",
      label: "Cited by",
      value: (r) => (r.cited_by ?? []).length,
      sortable: true,
      align: "end",
      narrow: "hide",
      render: (r) =>
        (r.cited_by ?? []).length === 0 ? (
          "0"
        ) : (
          <details className="charter-more">
            <summary>{(r.cited_by ?? []).length}</summary>
            <ul className="charter-cited">
              {(r.cited_by ?? []).map((c) => (
                <li key={c}>
                  <Link to={to.item(project, c)}>{c}</Link>
                </li>
              ))}
            </ul>
          </details>
        ),
    },
    { key: "verdicts", label: "Affirmed · contradicted", value: (r) => r.affirmed - r.contradicted, narrow: "hide", align: "end", render: (r) => `${r.affirmed} · ${r.contradicted}` },
    {
      key: "status",
      label: "Status",
      value: statusOf,
      sortable: true,
      render: (r) =>
        r.in_force ? (
          "in force"
        ) : (
          <SeverityPill tone="neutral" title={r.status === "retired" ? undefined : "Its believed share fell below 25%"}>
            retired
          </SeverityPill>
        ),
    },
  ];
}

export function LessonsSection(p: { project: string; progress: CharterProgress }): JSX.Element {
  const [open, setOpen] = useOpenRows();
  const location = useLocation();
  const { lessons } = p.progress;
  if (lessons.total === 0) {
    return <Empty title="No lessons recorded yet. Ask Claude to record one when something teaches the project." />;
  }
  const cycles = lessons.cycles ?? [];
  const filter = (q: string) => ({ pathname: location.pathname, search: q ? `?lq=${q}` : "", hash: location.hash });
  const cols = columns(p.project);
  const max = Math.max(1, ...cycles.map((c) => c.cumulative));
  return (
    <>
      {cycles.length > 0 && (
        <GrowthChart
          stateKey="lc-"
          max={max}
          unit="lessons recorded"
          caption="Lessons recorded, cumulative, by release cycle"
          tableCaption="Lessons recorded in each release cycle"
          points={cycles.map((c) => ({ label: c.label, value: c.cumulative }))}
        />
      )}
      <p className="charter-lesson-counts">
        <Link to={filter("")}>{lessons.total} lessons</Link>, <Link to={filter("in-force")}>{lessons.in_force} in force</Link>,{" "}
        <Link to={filter("retired")}>{lessons.retired} retired</Link>
      </p>
      {themeRows(p.progress).map((t) => (
        <AccordionRow
          key={t.key}
          id={`theme:${t.key}`}
          title={`${t.label} · ${t.count}`}
          meta={t.kinds ? <span className="ui-pill ui-pill-neutral">{t.kinds}</span> : undefined}
          open={open.has(`theme:${t.key}`)}
          onToggle={(o) => setOpen(`theme:${t.key}`, o)}
        >
          {() => (
            <DataTable
              rows={t.rows}
              columns={cols}
              rowKey={(r) => r.id}
              caption={`Lessons: ${t.label}`}
              csvName={`lessons-${t.key}`}
              stateKey="l"
              empty="No lessons match."
            />
          )}
        </AccordionRow>
      ))}
    </>
  );
}
