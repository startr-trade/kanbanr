/**
 * The tab model (FTR-046): which tabs are shown and which one a page opens on. Pure, so the checks
 * import it directly.
 */

/** One tab. `count` is shown in the label; `hidden` drops it; `disabled` says why it cannot open. */
export interface TabDef {
  key: string;
  label: string;
  count?: number;
  hidden?: boolean;
  disabled?: string;
}

/** The tabs that are drawn: hidden ones are dropped, disabled ones kept (they say why). */
export function visibleTabs(tabs: TabDef[]): TabDef[] {
  return tabs.filter((t) => !t.hidden);
}

/**
 * The tab a page opens on: the requested one if it is visible; else the preferred one if visible;
 * else the visible tab with the largest count above zero (the first on a tie); else the first
 * visible tab; `""` when there is none. A requested tab that is hidden or absent falls through.
 */
export function pickTab(tabs: TabDef[], requested: string | null, preferred?: string): string {
  const shown = visibleTabs(tabs);
  if (shown.length === 0) return "";
  const has = (key: string | null | undefined) => key != null && shown.some((t) => t.key === key);
  if (has(requested)) return requested as string;
  if (has(preferred)) return preferred as string;
  let best: TabDef | null = null;
  for (const t of shown) {
    if ((t.count ?? 0) > 0 && (best === null || (t.count ?? 0) > (best.count ?? 0))) best = t;
  }
  return (best ?? shown[0]).key;
}
