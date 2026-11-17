/**
 * One quiet item as one line (FTR-078): the code, the title as the only link, then what is left,
 * the tasks and the days in the stage, each as text. Quiet groups and the finished lane use it.
 */
import { Link } from "react-router-dom";
import type { BoardItem } from "../../boardModel.ts";

const days = (n: number) => `${n} ${n === 1 ? "day" : "days"}`;

export function BoardRow(p: { project: string; b: BoardItem; finished?: boolean }): JSX.Element {
  const { b } = p;
  const i = b.item;
  const left = p.finished || !b.next ? null : b.next.missing > 0 ? `${b.next.missing} ${b.next.missing === 1 ? "check" : "checks"} left` : "ready";
  return (
    <li className="board-row">
      <span className="board-code">{i.code}</span>
      <Link className="board-row-title" to={`/p/${encodeURIComponent(p.project)}/feature/${encodeURIComponent(i.code)}`}>
        {i.title}
      </Link>
      {left && <span className="board-row-fact">{left}</span>}
      {i.total > 0 && (
        <span className="board-row-fact">
          {i.done}/{i.total} tasks
        </span>
      )}
      {!p.finished && b.daysIn !== null && <span className="board-row-fact">{days(b.daysIn)}</span>}
    </li>
  );
}
