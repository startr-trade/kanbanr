/**
 * A sortable, filterable table with CSV export (FTR-046, R-3). The sort column, its direction and
 * the filter text live in the URL (`<stateKey>sort`, `<stateKey>dir`, `<stateKey>q`), written with
 * `replace`, so a link opens the table as the person left it.
 *
 * Sorting and filtering are in memory (`table.ts`). A table over 500 rows draws the first 200 and a
 * "Show all N" button; this is not pagination. Below 720 px, columns with `narrow: "hide"` are not
 * drawn and their values move into a `<details>` in the row's first cell.
 */
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { filterRows, sortRows, toCsv, type Column } from "./table.ts";
import { useUrlState } from "./urlState.ts";

export type { Column } from "./table.ts";

/** Above this many rows, only the first `FIRST_ROWS` are drawn until "Show all" is pressed. */
const MANY_ROWS = 500;
const FIRST_ROWS = 200;
/** The breakpoint `styles.css` already uses. */
const NARROW_QUERY = "(max-width: 720px)";

/** Whether the window is below the breakpoint now; false on the server and where `matchMedia` is missing. */
function narrowNow(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia(NARROW_QUERY).matches;
}

/**
 * True below the 720 px breakpoint; false on the server and where `matchMedia` is missing. The
 * first draw already knows the width, so a narrow screen never draws the wide table first.
 */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(narrowNow);
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia(NARROW_QUERY);
    const update = () => setNarrow(mq.matches);
    update();
    mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, []);
  return narrow;
}

/** A checkbox whose `indeterminate` follows the prop (it has no HTML attribute). */
function TriCheckbox(p: { checked: boolean; indeterminate: boolean; disabled: boolean; label: string; onChange(v: boolean): void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = p.indeterminate;
  }, [p.indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={p.checked}
      disabled={p.disabled ? true : undefined}
      aria-label={p.label}
      onChange={(e) => p.onChange(e.currentTarget.checked)}
    />
  );
}

export function DataTable<R>(p: {
  rows: R[];
  columns: Column<R>[];
  rowKey(r: R): string;
  filterPlaceholder?: string;
  /** Prefix of the URL keys, so two tables on one page keep separate state. */
  stateKey?: string;
  /** The CSV file's name, without `.csv`; default `table`. */
  csvName?: string;
  /** Shown when there are no rows, or none after filtering. */
  empty: ReactNode;
  caption: string;
  /**
   * A checkbox column. Select-all covers the filtered, enabled rows. `disabled` returns why a row
   * cannot be selected (shown as text next to its checkbox), or null.
   */
  selectable?: { selected: Set<string>; onChange(next: Set<string>): void; disabled?(r: R): string | null };
}): JSX.Element {
  const prefix = p.stateKey ?? "";
  const [state, setState] = useUrlState({ sort: "", dir: "asc", q: "" }, prefix);
  const [showAll, setShowAll] = useState(false);
  const [csvError, setCsvError] = useState<string | null>(null);
  const narrow = useNarrow();
  const base = useId();
  const filterId = `${base}-filter`;

  const sortCol = p.columns.find((c) => c.key === state.sort && c.sortable) ?? null;
  const dir: "asc" | "desc" = state.dir === "desc" ? "desc" : "asc";

  const shownRows = useMemo(() => {
    const filtered = filterRows(p.rows, p.columns, state.q);
    return sortCol ? sortRows(filtered, sortCol, dir) : filtered;
  }, [p.rows, p.columns, state.q, sortCol, dir]);

  if (p.rows.length === 0) {
    return <div className="ui-table-empty">{p.empty}</div>;
  }

  const drawn = !showAll && shownRows.length > MANY_ROWS ? shownRows.slice(0, FIRST_ROWS) : shownRows;
  const columns = narrow ? p.columns.filter((c) => c.narrow !== "hide") : p.columns;
  const folded = narrow ? p.columns.filter((c) => c.narrow === "hide") : [];
  const sel = p.selectable;
  const span = columns.length + (sel ? 1 : 0);

  const enabledShown = sel ? shownRows.filter((r) => !sel.disabled?.(r)) : [];
  const enabledKeys = enabledShown.map((r) => p.rowKey(r));
  const selectedShown = sel ? enabledKeys.filter((k) => sel.selected.has(k)).length : 0;
  const allSelected = enabledKeys.length > 0 && selectedShown === enabledKeys.length;

  const toggleAll = (on: boolean) => {
    if (!sel) return;
    const next = new Set(sel.selected);
    for (const k of enabledKeys) {
      if (on) next.add(k);
      else next.delete(k);
    }
    sel.onChange(next);
  };

  const toggleOne = (key: string, on: boolean) => {
    if (!sel) return;
    const next = new Set(sel.selected);
    if (on) next.add(key);
    else next.delete(key);
    sel.onChange(next);
  };

  const onSort = (col: Column<R>) => {
    if (sortCol?.key === col.key) setState({ sort: col.key, dir: dir === "asc" ? "desc" : "asc" });
    else setState({ sort: col.key, dir: "asc" });
  };

  const exportCsv = () => {
    try {
      if (typeof Blob === "undefined" || typeof URL.createObjectURL !== "function") {
        throw new Error("this browser cannot build a file to download");
      }
      const blob = new Blob([toCsv(shownRows, p.columns)], { type: "text/csv;charset=utf-8" });
      const href = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = href;
      a.download = `${p.csvName ?? "table"}.csv`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(href), 0);
      setCsvError(null);
    } catch (e) {
      setCsvError(`Export failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const ariaSort = (col: Column<R>) => {
    if (!col.sortable) return undefined;
    if (sortCol?.key !== col.key) return "none" as const;
    return dir === "asc" ? ("ascending" as const) : ("descending" as const);
  };

  return (
    <div className="ui-table-wrap">
      <div className="ui-table-tools">
        <label htmlFor={filterId}>Filter</label>
        <input
          id={filterId}
          type="search"
          value={state.q}
          placeholder={p.filterPlaceholder}
          onChange={(e) => setState({ q: e.currentTarget.value })}
        />
        <span className="ui-table-count" aria-live="polite">
          {shownRows.length} of {p.rows.length}
        </span>
        <button type="button" className="btn ui-btn" onClick={exportCsv}>
          Export CSV
        </button>
      </div>
      {csvError && (
        <p className="ui-table-error" role="alert">
          {csvError}
        </p>
      )}
      <table className="ui-table">
        <caption>{p.caption}</caption>
        <thead>
          <tr>
            {sel && (
              <th scope="col" className="ui-table-select">
                <TriCheckbox
                  checked={allSelected}
                  indeterminate={selectedShown > 0 && !allSelected}
                  disabled={enabledKeys.length === 0}
                  label="Select all shown rows"
                  onChange={toggleAll}
                />
              </th>
            )}
            {columns.map((c) => (
              <th key={c.key} scope="col" aria-sort={ariaSort(c)} className={c.align === "end" ? "ui-end" : undefined}>
                {c.sortable ? (
                  <button type="button" className="ui-sort" onClick={() => onSort(c)}>
                    {c.label}
                  </button>
                ) : (
                  c.label
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {drawn.length === 0 ? (
            <tr>
              <td colSpan={span} className="ui-table-empty">
                {p.empty}
              </td>
            </tr>
          ) : (
            drawn.map((r) => {
              const key = p.rowKey(r);
              const why = sel?.disabled?.(r) ?? null;
              const reasonId = `${base}-why-${key}`;
              return (
                <tr key={key}>
                  {sel && (
                    <td className="ui-table-select">
                      <input
                        type="checkbox"
                        checked={sel.selected.has(key)}
                        disabled={why ? true : undefined}
                        aria-label={`Select ${key}`}
                        aria-describedby={why ? reasonId : undefined}
                        onChange={(e) => toggleOne(key, e.currentTarget.checked)}
                      />
                      {why && (
                        <span className="ui-table-reason" id={reasonId}>
                          {why}
                        </span>
                      )}
                    </td>
                  )}
                  {columns.map((c, i) => (
                    <td key={c.key} className={c.align === "end" ? "ui-end" : undefined}>
                      {c.render ? c.render(r) : (c.value(r) ?? "")}
                      {i === 0 && folded.length > 0 && (
                        <details className="ui-table-detail">
                          <summary>More</summary>
                          <dl>
                            {folded.map((f) => (
                              <div key={f.key}>
                                <dt>{f.label}</dt>
                                <dd>{f.render ? f.render(r) : (f.value(r) ?? "")}</dd>
                              </div>
                            ))}
                          </dl>
                        </details>
                      )}
                    </td>
                  ))}
                </tr>
              );
            })
          )}
        </tbody>
      </table>
      {drawn.length < shownRows.length && (
        <button type="button" className="btn ui-btn" onClick={() => setShowAll(true)}>
          Show all {shownRows.length}
        </button>
      )}
    </div>
  );
}
