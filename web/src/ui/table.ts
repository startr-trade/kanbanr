/**
 * The table model (FTR-046, R-3): columns, sorting, filtering and CSV. Pure, so the checks import it
 * directly; `DataTable.tsx` is the thin component over it.
 */
import type { ReactNode } from "react";

/**
 * One column. `value` is what the column sorts, filters and exports by (`null` for no value);
 * `render` draws the cell when the text of `value` is not enough. `narrow: "hide"` drops the column
 * below 720 px, moving its value into the row's detail.
 */
export interface Column<R> {
  key: string;
  label: string;
  value(r: R): string | number | null;
  render?(r: R): ReactNode;
  sortable?: boolean;
  align?: "start" | "end";
  narrow?: "hide" | "keep";
}

/** Two non-null values: numbers numerically, otherwise as text with numeric runs (`FTR-9 < FTR-10`). */
function compareValues(a: string | number, b: string | number): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b), undefined, { numeric: true });
}

/**
 * `rows` sorted by `col` in `dir`, as a new array. Stable: rows with equal values keep their input
 * order in both directions. A `null` value is always last, ascending or descending.
 */
export function sortRows<R>(rows: R[], col: Column<R>, dir: "asc" | "desc"): R[] {
  const sign = dir === "desc" ? -1 : 1;
  return rows
    .map((r, i) => ({ r, i, v: col.value(r) }))
    .sort((x, y) => {
      const xNull = x.v === null || x.v === undefined;
      const yNull = y.v === null || y.v === undefined;
      if (xNull || yNull) {
        if (xNull && yNull) return x.i - y.i;
        return xNull ? 1 : -1;
      }
      const c = compareValues(x.v as string | number, y.v as string | number) * sign;
      return c !== 0 ? c : x.i - y.i;
    })
    .map((d) => d.r);
}

/**
 * The rows where every whitespace-separated token of `text` is found, case-insensitively, in the
 * joined values of `cols`. Empty text keeps every row. Returns a new array in input order.
 */
export function filterRows<R>(rows: R[], cols: Column<R>[], text: string): R[] {
  const tokens = text.toLowerCase().split(/\s+/).filter((t) => t !== "");
  if (tokens.length === 0) return rows.slice();
  return rows.filter((r) => {
    const hay = cols
      .map((c) => c.value(r))
      .filter((v) => v !== null && v !== undefined)
      .map(String)
      .join(" ")
      .toLowerCase();
    return tokens.every((t) => hay.includes(t));
  });
}

/** One CSV cell: a formula-leading cell gets a leading `'` (CSV injection); then RFC 4180 quoting. */
function csvCell(value: string | number | null | undefined): string {
  let text = value === null || value === undefined ? "" : String(value);
  if (/^[=+\-@\t\r]/.test(text)) text = `'${text}`;
  if (/[",\r\n]/.test(text)) text = `"${text.replace(/"/g, '""')}"`;
  return text;
}

/**
 * `rows` as CSV (RFC 4180): a header of the columns' labels, then one line per row of their
 * `value()`s. A cell holding `,`, `"`, CR or LF is quoted with `"` doubled; a cell starting with `=`,
 * `+`, `-`, `@`, tab or CR is prefixed with `'` so a spreadsheet does not run it. Every line ends in
 * CRLF.
 */
export function toCsv<R>(rows: R[], cols: Column<R>[]): string {
  const lines = [cols.map((c) => csvCell(c.label)).join(",")];
  for (const r of rows) lines.push(cols.map((c) => csvCell(c.value(r))).join(","));
  return lines.map((l) => `${l}\r\n`).join("");
}
