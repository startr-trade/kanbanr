/**
 * A lane's groups as disclosure rows, never pages (FTR-078): attention groups open with full cards,
 * quiet ones closed to a line with their count and their oldest item and, opened, one-line rows. The
 * finished lane shows its window and one line for what the window leaves out.
 */
import { Link } from "react-router-dom";
import { groupId, isGroupOpen, oldestText, shownGroups } from "../../boardModel.ts";
import type { Group, Lane } from "../../boardModel.ts";
import type { CompactIndex } from "../../types.ts";
import { AccordionRow } from "../../ui/AccordionRow.tsx";
import { Empty } from "../../ui/States.tsx";
import { BoardCard } from "./BoardCard.tsx";
import { BoardRow } from "./BoardRow.tsx";

export function LaneGroups(p: {
  project: string;
  lane: Lane;
  index: CompactIndex;
  open: Set<string>;
  shut: Set<string>;
  only: Set<string>;
  onToggle(id: string, open: boolean, byDefault: boolean): void;
  /** Clears the bar filters of this lane. */
  onClear(ids: string[]): void;
}): JSX.Element {
  const { lane } = p;
  const { groups, filtered } = shownGroups(lane, p.only);
  const finished = lane.kind === "finished";
  const list = (g: Group) => {
    const label = `${g.label}, ${g.items.length} ${g.items.length === 1 ? "item" : "items"}`;
    return (
      <div className="board-group-list" role="region" aria-label={label} tabIndex={0}>
        {g.attention && !finished ? (
          g.items.map((b) => <BoardCard key={b.item.code} project={p.project} b={b} index={p.index} usual={lane.usual} />)
        ) : (
          <ul className="board-rows">
            {g.items.map((b) => (
              <BoardRow key={b.item.code} project={p.project} b={b} finished={finished} />
            ))}
          </ul>
        )}
      </div>
    );
  };
  return (
    <div className="board-groups">
      {filtered && (
        <button type="button" className="board-link-button" onClick={() => p.onClear(lane.groups.map((g) => groupId(lane.status, g.key)))}>
          Show every group
        </button>
      )}
      {groups.map((g) => {
        const id = groupId(lane.status, g.key);
        const open = filtered || isGroupOpen(lane.status, g, p.open, p.shut);
        const oldest = finished ? null : oldestText(g, lane.status);
        return (
          <AccordionRow
            key={g.key}
            id={id}
            level={3}
            title={`${g.label} · ${g.items.length}`}
            meta={oldest ? <span className="board-oldest">{oldest}</span> : undefined}
            actions={g.key === "you" ? <Link to={`/p/${encodeURIComponent(p.project)}/review`}>Review →</Link> : undefined}
            open={open}
            onToggle={(o) => p.onToggle(id, o, g.attention)}
          >
            {() => list(g)}
          </AccordionRow>
        );
      })}
      {lane.rest && (
        <p className="board-rest">
          <Link to={lane.rest.to}>{lane.rest.text}</Link>
        </p>
      )}
      {groups.length === 0 && !lane.rest && <Empty title="Nothing here." />}
    </div>
  );
}
