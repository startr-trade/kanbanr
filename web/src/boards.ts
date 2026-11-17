// Several boards on one read-only server (FTR-040). Pure, so node checks import it.
import type { BoardInfo, ProjectSummary } from "./types";

export interface BoardGroup {
  board: BoardInfo | null;
  projects: ProjectSummary[];
}

/** One group per board, in the server's order, or a single group with `board: null` when the
 * server shows no more than one board (Home then renders as it always did). A board with no
 * projects keeps its (empty) group. */
export function groupByBoard(projects: ProjectSummary[], boards: BoardInfo[]): BoardGroup[] {
  if (boards.length <= 1) return [{ board: null, projects }];
  return boards.map((board) => ({
    board,
    projects: projects.filter((p) => p.board === board.name),
  }));
}

/** For a qualified dependency `<project>:<code>` (split on the first `:`), the name of the board
 * holding `<project>` when that is not the board holding `project`; otherwise `null` (bare codes,
 * the same board, an unknown project). Nothing resolves across boards (ADR-0007). */
export function externalBoard(dep: string, project: string, boards: BoardInfo[]): string | null {
  const at = dep.indexOf(":");
  if (at < 0) return null;
  const target = dep.slice(0, at);
  const holder = (p: string) => boards.find((b) => b.projects.includes(p))?.name ?? null;
  const there = holder(target);
  const here = holder(project);
  return there !== null && there !== here ? there : null;
}

/** The chip's target for a qualified dependency: `/p/<project>/feature/<code>`. */
export function dependencyPath(dep: string): string {
  const at = dep.indexOf(":");
  return `/p/${encodeURIComponent(dep.slice(0, at))}/feature/${encodeURIComponent(dep.slice(at + 1))}`;
}
