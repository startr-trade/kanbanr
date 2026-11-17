/**
 * The `recode` group's row on the Review page (FTR-056): the plan Claude prepared for bringing old
 * items to their kinds' code prefixes, as a two-column from → to table with the count of renames in
 * its heading. The table scrolls, since a board can have a hundred renames. It reads the printed plan
 * the prepared record holds (`fields.plan`, the text `kanbanr feature recode` prints), so what the
 * person reads is what the record's rev names. The decision itself is the group's Confirm button.
 */
import type { PreparedView } from "../../types";

/** One rename of the plan: `FEAT-150 → DEF-095  bug  The board drops a field`. */
export interface RecodeLine {
  from: string;
  to: string;
  kind: string;
  title: string;
}

/** The plan's renames in print order, and the closing line (`N items; plan <rev>`) when present. */
export function parsePlan(text: string): { lines: RecodeLine[]; summary: string } {
  const lines: RecodeLine[] = [];
  let summary = "";
  for (const raw of text.split("\n")) {
    const row = raw.match(/^(\S+) → (\S+) {2}(\S*) {2}(.*)$/);
    if (row) lines.push({ from: row[1], to: row[2], kind: row[3], title: row[4] });
    else if (/^\d+ items?; plan /.test(raw)) summary = raw;
  }
  return { lines, summary };
}

/** The plan text of a prepared `recode` record, or "" when it carries none. */
export function planOf(view: PreparedView): string {
  const plan = view.fields?.plan;
  return typeof plan === "string" ? plan : "";
}

export function RecodeEvidence({ plan }: { plan: string }): JSX.Element {
  const { lines, summary } = parsePlan(plan);
  const n = lines.length;
  return (
    <div className="review-body recode-plan">
      <h4>{`Renames ${n} ${n === 1 ? "item" : "items"} to their kinds' prefixes`}</h4>
      <p className="muted small">
        Each item keeps its old code as an alias, so links, commit trailers and records made under it still reach it. {summary}
      </p>
      <div className="ui-table-wrap" style={{ maxHeight: "24rem", overflowY: "auto" }}>
        <table className="ui-table">
          <caption className="ui-sr-only">{`${n} renames, from the old code to the new`}</caption>
          <thead>
            <tr>
              <th scope="col">From</th>
              <th scope="col">To</th>
            </tr>
          </thead>
          <tbody>
            {lines.map((l) => (
              <tr key={l.from}>
                <td title={l.title}>
                  <code>{l.from}</code>
                </td>
                <td title={l.title}>
                  <code>{l.to}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
