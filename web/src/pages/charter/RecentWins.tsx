/** Recent wins (FTR-076): the latest finished items that serve a goal, one line each, linked. */
import { Link } from "react-router-dom";
import { Empty } from "../../ui";
import { shortDate } from "../../charterModel";
import { to } from "../../overviewLinks";
import type { Win } from "../../types";

export function RecentWins(p: { project: string; wins: Win[] }): JSX.Element {
  if (p.wins.length === 0) return <Empty title="Nothing that serves a goal has finished yet." />;
  return (
    <ul className="charter-wins">
      {p.wins.map((w) => (
        <li key={w.code}>
          <Link to={to.item(p.project, w.code)}>{w.code}</Link> <em>{w.title}</em>{" "}
          <span className="muted">
            finished {shortDate(w.at)}, toward {w.goals.join(", ")}
            {w.release ? ` · ${w.release}` : ""}
          </span>
        </li>
      ))}
    </ul>
  );
}
