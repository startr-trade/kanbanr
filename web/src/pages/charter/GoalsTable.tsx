/**
 * The Goals table (FTR-076): one row a goal, with what serves it and how much of it is proven. Every
 * count links to the list it counted; a goal no item serves says so in words. The numbers come from
 * `charterModel.goalRows`, so the page and its checks read one source.
 */
import { Link } from "react-router-dom";
import { DataTable, ProgressBar, SeverityPill, sparkPoints, type Column } from "../../ui";
import type { GoalRow } from "../../charterModel";

export function GoalsTable(p: { rows: GoalRow[]; trends: Record<string, number[]>; withProgress: boolean }): JSX.Element {
  const base: Column<GoalRow>[] = [
    { key: "id", label: "Goal", value: (r) => r.id, sortable: true, render: (r) => <code className="taskkey" id={r.id}>{r.id}</code> },
    {
      key: "statement",
      label: "Statement",
      value: (r) => r.statement,
      render: (r) => (
        <>
          <span>{r.head}</span>
          {(r.rest || r.lessons.length > 0) && (
            <details className="charter-more">
              <summary>Full statement{r.lessons.length > 0 ? " and lessons" : ""}</summary>
              {r.rest && <p>{r.rest}</p>}
              {r.lessons.length > 0 && (
                <p>
                  Lessons naming this goal: <Link to={r.lessonsTo}>{r.lessons.join(", ")}</Link>
                </p>
              )}
            </details>
          )}
        </>
      ),
    },
    { key: "measure", label: "Measure", value: (r) => r.measure, narrow: "hide" },
    { key: "horizon", label: "Horizon", value: (r) => r.horizon, sortable: true, narrow: "hide" },
  ];
  const progress: Column<GoalRow>[] = [
    {
      key: "items",
      label: "Items",
      value: (r) => r.items,
      sortable: true,
      render: (r) =>
        r.unserved ? (
          <SeverityPill tone="warn">no item serves this goal yet</SeverityPill>
        ) : (
          <Link to={r.itemsTo}>
            {r.itemsDone} done · {r.itemsOpen} open
          </Link>
        ),
    },
    {
      key: "proven",
      label: "Proven",
      value: (r) => (r.total === 0 ? 0 : r.proven / r.total),
      sortable: true,
      render: (r) =>
        r.total === 0 ? (
          <span className="muted">no requirements yet</span>
        ) : (
          <span className="charter-proven">
            <ProgressBar
              label={`${r.id} requirements`}
              total={r.total}
              to={r.provenTo}
              segments={[
                { key: "proven", label: "Proven", value: r.proven, tone: "info" },
                { key: "rest", label: "Not yet proven", value: r.total - r.proven, tone: "neutral" },
              ]}
            />
          </span>
        ),
    },
    {
      key: "lessons",
      label: "Lessons",
      value: (r) => r.lessons.length,
      sortable: true,
      render: (r) =>
        r.lessons.length === 0 ? (
          <span className="muted">none</span>
        ) : (
          <Link to={r.lessonsTo} title={r.lessonThemes.length ? `Themes: ${r.lessonThemes.join(", ")}` : undefined}>
            {r.lessons.length} {r.lessons.length === 1 ? "lesson" : "lessons"}
          </Link>
        ),
    },
    {
      key: "trend",
      label: "Trend",
      value: () => null,
      narrow: "hide",
      render: (r) => {
        const points = sparkPoints(p.trends[r.id] ?? []);
        return points ? (
          <svg className="charter-spark" viewBox="0 0 60 16" width="60" height="16" aria-hidden="true" focusable="false">
            <polyline points={points} fill="none" />
          </svg>
        ) : null;
      },
    },
  ];
  return (
    <DataTable
      rows={p.rows}
      columns={p.withProgress ? [...base, ...progress] : base}
      rowKey={(r) => r.id}
      caption="Goals"
      csvName="charter-goals"
      stateKey="g"
      empty="No goals yet: items have nothing to serve."
    />
  );
}
