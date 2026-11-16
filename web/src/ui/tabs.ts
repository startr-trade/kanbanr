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
 * The tab a page opens on: the requested one if it is visible and enabled; else the preferred one if
 * visible and enabled; else the enabled visible tab with the largest count above zero (the first on a
 * tie); else the first enabled visible tab; `""` when there is none. A requested, preferred or
 * largest tab that is hidden, disabled or absent falls through: a disabled tab is never selected.
 */
export function pickTab(tabs: TabDef[], requested: string | null, preferred?: string): string {
  const open = visibleTabs(tabs).filter((t) => !t.disabled);
  if (open.length === 0) return "";
  const has = (key: string | null | undefined) => key != null && open.some((t) => t.key === key);
  if (has(requested)) return requested as string;
  if (has(preferred)) return preferred as string;
  let best: TabDef | null = null;
  for (const t of open) {
    if ((t.count ?? 0) > 0 && (best === null || (t.count ?? 0) > (best.count ?? 0))) best = t;
  }
  return (best ?? open[0]).key;
}
