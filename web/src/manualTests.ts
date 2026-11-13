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

/** The `kanbanr test` command for the person's own terminal, where the monitor cannot write. */
export function manualCommand(row: ManualWaiting, passed: boolean): string {
  const quote = (s: string) => (/^[A-Za-z0-9._/:@=+,-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
  return `kanbanr test ${row.code} ${row.requirement} ${quote(row.test)} ${passed ? "green" : "red"}`;
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
