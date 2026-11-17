/**
 * An opened Plan row's body (FTR-043): its items, read now that the row is open (R-8), drawn on the
 * tab's timeline scale where the daemon placed them (their sprint, their own moves or their start;
 * never a made-up date, R-3), then a table of the items with the critical path marked, and the
 * undated ones listed apart. A sprint also shows its burndown; a milestone its items by status,
 * dependencies first, as the Schedule's cards did.
 */
import { Link } from "react-router-dom";
import { useQuery } from "../../data/useQuery";
import { TAGS, urls } from "../../data/urls";
import Burndown from "../../components/Burndown";
import { ErrorState, LoadingState, Timeline } from "../../ui";
import { itemBands } from "../../planTabs";
import { to as link } from "../../overviewLinks";
import type { PlacedItem, PlanLane, RowBody } from "../../types";

type Window = { from: string; to: string; today: string };

/** Reads the row's items; a failed read shows its error inside this row only. */
export function RowItems(p: { project: string; lane: PlanLane } & Window): JSX.Element {
  const { project, lane } = p;
  const body = useQuery<RowBody>(project, urls.planRow(project, lane.key), TAGS.plan);
  if (!body.data) {
    return body.error ? <ErrorState error={body.error} retry={body.reload} /> : <LoadingState />;
  }
  return <RowItemsBody project={project} lane={lane} from={p.from} to={p.to} today={p.today} body={body.data} />;
}

const PLACED: Record<string, string> = {
  sprint: "by its sprint",
  moves: "by its moves",
  start: "by its start",
  undated: "undated",
};

function placedText(i: PlacedItem): string {
  if (i.placed === "undated" || !i.from) return "undated";
  const dates = i.from === i.to ? i.from : `${i.from} to ${i.to}`;
  return `${dates}, ${PLACED[i.placed] ?? i.placed}`;
}

/** The window the items are drawn over: the tab's, else the items' own first and last day. */
function windowOf(p: Window, items: PlacedItem[]): Window | null {
  if (p.from && p.to) return p;
  const days = items.flatMap((i) => [i.from, i.to]).filter((d) => d !== "").sort();
  return days.length ? { from: days[0], to: days[days.length - 1], today: p.today } : null;
}

/** The body once read. Exported so the checks render it without a daemon. */
export function RowItemsBody(p: { project: string; lane: PlanLane; body: RowBody } & Window): JSX.Element {
  const { project, lane, body } = p;
  const lanes = itemBands(body);
  const win = windowOf(p, body.items);
  const undated = body.items.filter((i) => i.placed === "undated" || !i.from);
  const title = new Map(body.items.map((i) => [i.code, i.title]));

  return (
    <div className="plan-row-items">
      {lane.burndown ? <Burndown report={lane.burndown} /> : null}
      {lanes.length > 0 && win ? (
        <Timeline from={win.from} to={win.to} today={win.today || undefined} lanes={lanes} caption={`${lane.label}'s items`} />
      ) : null}
      {body.by_status.length > 0 ? (
        <div className="plan-by-status">
          {body.by_status.map(([status, codes]) => (
            <section key={status} className="plan-status">
              <h3>
                {status} · {codes.length}
              </h3>
              <ol>
                {codes.map((code) => (
                  <li key={code}>
                    <Link to={link.item(project, code)}>{code}</Link> {title.get(code) ?? ""}
                  </li>
                ))}
              </ol>
            </section>
          ))}
        </div>
      ) : null}
      {body.items.length === 0 ? (
        <p className="muted small">No items in {lane.label}.</p>
      ) : (
        <table className="plan-items">
          <caption className="ui-sr-only">{lane.label}'s items</caption>
          <thead>
            <tr>
              <th scope="col">Code</th>
              <th scope="col">Title</th>
              <th scope="col">Status</th>
              <th scope="col" className="plan-col-placed">
                Placed
              </th>
              <th scope="col" className="plan-col-critical">
                Critical path
              </th>
            </tr>
          </thead>
          <tbody>
            {body.items.map((i) => (
              <tr key={i.code}>
                <td>
                  <Link to={link.item(project, i.code)}>{i.code}</Link>
                </td>
                <td>{i.title}</td>
                <td>{i.status}</td>
                <td className="plan-col-placed">{placedText(i)}</td>
                <td className="plan-col-critical">{i.critical ? <span className="ui-pill ui-pill-warn">critical path</span> : null}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {undated.length > 0 ? (
        <div className="plan-undated">
          <h3>Undated ({undated.length})</h3>
          <ul>
            {undated.map((i) => (
              <li key={i.code}>
                <Link to={link.item(project, i.code)}>{i.code}</Link> {i.title}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {body.carried.length > 0 ? (
        <div className="plan-carried">
          <h3>Carried over ({body.carried.length})</h3>
          <ul>
            {body.carried.map((c) => (
              <li key={c.code}>
                <Link to={link.item(project, c.code)}>{c.code}</Link> → {c.to}
                {c.why ? <span className="muted small"> {c.why}</span> : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
