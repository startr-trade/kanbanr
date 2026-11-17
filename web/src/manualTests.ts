// The manual tests waiting for a person (DEF-040), as the Review page shows them. Pure, so the
// page and web::manual_tests (scripts/check-manual-tests.mjs) use the same code.
import { openFor } from "./prepared.ts";
import type { ManualResult, ManualWaiting, PreparedView } from "./types";

/** One manual test waiting, with the open record Claude prepared for it, if any. */
export interface ManualRow {
  row: ManualWaiting;
  prepared?: PreparedView;
}

/**
 * Each waiting test paired with its newest open `manual-test` record. A stale, superseded or
 * answered record is never offered, so a result already recorded does not come back.
 */
export function manualRows(waiting: ManualWaiting[] | null | undefined, prepared: PreparedView[] | null | undefined): ManualRow[] {
  return (waiting ?? []).map((row) => ({
    row,
    prepared: openFor(prepared, "manual-test", { item: row.code, requirement: row.requirement, test: row.test }),
  }));
}

/**
 * Why a check is not due yet (DEF-076): "Not yet: FEAT-003 is at System Design. Its check opens at
 * Build, once the code it checks exists.", or "" when it is due. Before its item reaches the build
 * stage nothing exists to check, so the page offers no result and kanbanr refuses one.
 */
export function notDueLine(row: ManualWaiting): string {
  if (!row.opens_at) return "";
  return `Not yet: ${row.code} is at ${row.status}. Its check opens at ${row.opens_at}, once the code it checks exists.`;
}

/** The `kanbanr test` command for the person's own terminal, where the monitor cannot write. */
export function manualCommand(row: ManualWaiting, passed: boolean): string {
  const quote = (s: string) => (/^[A-Za-z0-9._/:@=+,-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
  return `kanbanr test ${row.code} ${row.requirement} ${quote(row.test)} ${passed ? "green" : "red"}`;
}

/** One item's manual checks on the Review page (FTR-057): its heading, its rows and their counts. */
export interface ItemGroup {
  code: string;
  title: string;
  status: string;
  release: string;
  rows: ManualRow[];
  /** The rows with an open prepared record. */
  prepared: number;
  /** The rows that hold the item's release's next promotion. */
  holds: number;
}

/** The manual checks in the order the Review page shows them (FTR-057). */
export interface ManualSections {
  /** One section per release being shipped, in the order the server gives (newest first). */
  holding: { version: string; items: ItemGroup[] }[];
  /** Every other item with a due check. */
  other: ItemGroup[];
  /** Checks whose item has not reached its build stage (DEF-076). */
  notDue: ManualRow[];
}

/**
 * The due checks grouped by item, keeping the server's row order within an item; the items of
 * each holding release under it, in `holding`'s order; within a section, items with a check that
 * holds a promotion first, then the server's order; every other item in `other`. A check that is
 * not due yet (`opens_at`, DEF-076) goes to `notDue`.
 */
export function manualGroups(rows: ManualRow[], holding: string[]): ManualSections {
  const notDue: ManualRow[] = [];
  const items: ItemGroup[] = [];
  const byCode = new Map<string, ItemGroup>();
  for (const r of rows) {
    if (r.row.opens_at) {
      notDue.push(r);
      continue;
    }
    let g = byCode.get(r.row.code);
    if (!g) {
      g = { code: r.row.code, title: r.row.title, status: r.row.status, release: r.row.release ?? "", rows: [], prepared: 0, holds: 0 };
      byCode.set(r.row.code, g);
      items.push(g);
    }
    g.rows.push(r);
    if (r.prepared) g.prepared += 1;
    if (r.row.holds) g.holds += 1;
  }
  // Array.prototype.sort is stable, so equal items keep the server's order.
  const holdsFirst = (list: ItemGroup[]) => list.sort((a, b) => Number(b.holds > 0) - Number(a.holds > 0));
  const sections = holding.map((version) => ({ version, items: holdsFirst(items.filter((g) => g.release !== "" && g.release === version)) }));
  const other = holdsFirst(items.filter((g) => !holding.includes(g.release) || g.release === ""));
  return { holding: sections, other, notDue };
}

/** "failed by Ada on 12 Nov 2026", or "" when nothing was recorded. */
export function lastResultLine(last: ManualResult | null | undefined): string {
  if (!last) return "";
  const d = new Date(last.at);
  const day = Number.isNaN(d.getTime())
    ? last.at.slice(0, 10)
    : d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
  return `${last.state === "green" ? "passed" : "failed"} by ${last.by || "someone"} on ${day}`;
}

/**
 * What a decision on the Review page came to once its call returned (DEF-090): `ok` for a verdict
 * recorded, a rejection included, and the error text otherwise. Any outcome hides the form.
 */
export interface Outcome {
  ok: boolean;
  text: string;
}

/** The line shown where the form was: "Rejected — it leaves this list on the next refresh", or the error. */
export function outcomeLine(o: Outcome): string {
  return o.ok ? `${o.text} — it leaves this list on the next refresh` : o.text;
}
