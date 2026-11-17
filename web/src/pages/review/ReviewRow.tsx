/**
 * One row of a Review group (FTR-038, R-9, R-10): FTR-046's `AccordionRow`, so its header is a
 * heading holding the toggle. One line: the code and title (cut with an ellipsis), what is waiting,
 * and the row's properties as pills (prepared, its batch, the release and the sprint); the
 * `records` line, what a yes records, sits under the title in small muted text and is the only
 * second line. The body is the row's evidence, drawn only while the row is open.
 *
 * Also here: the diagram check the approve and ratify rows wait on (FEAT-200), shared by the row's
 * group and the brief in its body.
 */
import { useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "../../data/useQuery";
import { TAGS, urls } from "../../data/urls";
import { checkBlocks, type DiagramProblem } from "../../diagramCheck";
import { AccordionRow } from "../../ui";
import type { ItemDiagrams, ReviewRow as Row } from "../../types";

/**
 * The row's `what` text as drawn: empty when a pill already says it (DEF-112). The state is named
 * once, as the pill, because the pills are what FTR-080 filters by; the served row is unchanged,
 * so the What column and the evidence sentence still read `row.what`. Exact match only.
 */
export function rowWhat(row: Pick<Row, "what" | "pills">): string {
  return row.pills.includes(row.what) ? "" : row.what;
}

export function ReviewRow(p: {
  project: string;
  row: Row;
  /** The row's heading level; 3 unless it sits under an item's heading (FTR-057's Manual group: 4). */
  level?: 3 | 4;
  /** A pill drawn before the row's own, such as the check that holds a promotion (FTR-057). */
  badge?: ReactNode;
  open: boolean;
  onToggle(open: boolean): void;
  /** The row's checkbox; absent on a read-only monitor. `disabled` is why it cannot be selected. */
  select?: { checked: boolean; disabled?: string; onChange(v: boolean): void };
  children: () => ReactNode;
}): JSX.Element {
  const { row } = p;
  const enc = encodeURIComponent;
  // A delegation, a delegated decision and an ADR are not items; their code links nowhere.
  const isItem = /^[A-Z][A-Z0-9]*-\d+$/.test(row.code) && !/^(DLG|ADR)-/.test(row.code);
  const what = rowWhat(row);
  const pills = [...row.pills, ...(row.release ? [row.release] : []), ...(row.sprint ? [row.sprint] : [])];
  return (
    <AccordionRow
      id={row.key}
      level={p.level}
      open={p.open}
      onToggle={p.onToggle}
      select={p.select}
      title={
        <>
          <span className="review-row-title">
            <code className="taskkey">{row.code}</code> {row.title}
          </span>
          <span className="review-row-records">{row.records}</span>
        </>
      }
      meta={
        <>
          {what !== "" && <span className="review-row-what">{what}</span>}
          {p.badge}
          {pills.map((text, i) => (
            <span className="ui-pill ui-pill-neutral" key={`${i}-${text}`}>
              {text}
            </span>
          ))}
        </>
      }
      actions={
        isItem ? (
          <Link className="small" to={`/p/${enc(p.project)}/feature/${enc(row.code)}`} aria-label={`open ${row.code}`}>
            open
          </Link>
        ) : undefined
      }
    >
      {p.children}
    </AccordionRow>
  );
}

/** Where an item's diagram check stands: still running, could not run, or its problems. */
export interface DiagramState {
  problems: DiagramProblem[];
  failed: string | null;
  loading: boolean;
}

/**
 * Pele's parse of every diagram an item's brief covers — its spec and each doc in `affects_docs`
 * (FEAT-200) — read through the shared cache and re-read when the item or its spec changes, so fixing the spec re-enables Approve without a reload.
 */
export function useDiagramCheck(project: string, code: string): DiagramState & { done: boolean } {
  const diagrams = useQuery<ItemDiagrams>(project, urls.itemDiagrams(project, code), TAGS.itemDiagrams(code));
  const [parsed, setParsed] = useState<{ problems: DiagramProblem[]; failed: string | null; done: boolean }>({
    problems: [],
    failed: null,
    done: false,
  });
  useEffect(() => {
    const d = diagrams.data;
    if (diagrams.error) {
      setParsed({ problems: [], failed: diagrams.error, done: true });
      return;
    }
    if (!d) return;
    setParsed((p) => ({ ...p, done: false }));
    let cancelled = false;
    import("pele")
      .then(({ parse }) => {
        if (!cancelled) setParsed({ problems: checkBlocks(d, parse), failed: null, done: true });
      })
      .catch((e) => {
        if (!cancelled) setParsed({ problems: [], failed: String(e), done: true });
      });
    return () => {
      cancelled = true;
    };
  }, [diagrams.data, diagrams.error]);
  // Fails closed: a refetch in flight counts as loading, so a spec broken since the last check is
  // never approved on the strength of the check before it.
  const loading = diagrams.loading || diagrams.refreshing || !parsed.done;
  return { ...parsed, loading };
}

/**
 * Runs one row's diagram check and reports it (approve and ratify groups): the group keeps a row
 * unselectable until its check has finished clean. Draws nothing.
 */
export function DiagramWatch(p: {
  project: string;
  rowKey: string;
  code: string;
  onState(key: string, state: DiagramState): void;
}): null {
  const check = useDiagramCheck(p.project, p.code);
  const { onState, rowKey } = p;
  const signature = `${check.loading}|${check.failed ?? ""}|${check.problems.length}`;
  useEffect(() => {
    onState(rowKey, { problems: check.problems, failed: check.failed, loading: check.loading });
    // Reported when what it says changes, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rowKey, signature]);
  return null;
}
