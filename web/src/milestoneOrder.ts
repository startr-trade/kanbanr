/**
 * Milestones newest first (FTR-031): the highest code first. Codes compare by the number after
 * their last `-` (`MS-10` above `MS-9`), equal numbers by the whole code as text; a code with no
 * number comes last, among themselves by text; all descending. Returns a new array.
 *
 * The twin of `Milestone::newest_first` in kanbanr-core (models.rs): the same rule, written in
 * each language so a custom code sorts the same on the CLI and in the monitor.
 */
function rank(code: string): number | null {
  const tail = code.slice(code.lastIndexOf("-") + 1);
  return code.includes("-") && /^[0-9]+$/.test(tail) ? Number(tail) : null;
}

export function newestFirst<T extends { code: string }>(list: readonly T[]): T[] {
  return [...list].sort((a, b) => {
    const ra = rank(a.code);
    const rb = rank(b.code);
    if (ra !== rb) {
      if (ra === null) return 1;
      if (rb === null) return -1;
      return rb - ra;
    }
    return a.code < b.code ? 1 : a.code > b.code ? -1 : 0;
  });
}
