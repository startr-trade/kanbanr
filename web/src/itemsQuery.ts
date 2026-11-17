// The Items page's filters, read from its URL (FTR-048, R-9). Pure, so node checks import it: the
// page's query string becomes the `/items` API query, so a link from the Overview
// (`/p/x/items?kind=defect&evidence=red`) opens the list it counted.
import type { ItemFilter } from "./types";

/** The page's filter keys, each empty when unset: the defaults `useUrlState` reads them against. */
export const ITEM_FILTERS = {
  kind: "",
  status: "",
  milestone: "",
  goal: "",
  sprint: "",
  release: "",
  evidence: "",
} as const;

export type ItemFilterState = { [K in keyof typeof ITEM_FILTERS]: string };

/** The evidence values `/items?evidence=` accepts (core `items::Evidence`). */
export const EVIDENCE = ["red", "planned", "stale", "unproven", "proven", "untested"] as const;

/** The filter keys in the order the API query writes them (the same as `urls.items`). */
const ORDER = ["kind", "status", "milestone", "goal", "sprint", "release", "evidence"] as const;

/**
 * The filters `search` sets: each of the seven keys that is present and not empty. Other
 * parameters (a table's sort and filter text) are ignored, and an evidence value the API would
 * refuse is dropped, so a mistyped link lists every item rather than failing.
 */
export function itemsFilter(search: string): ItemFilter {
  let params: URLSearchParams;
  try {
    params = new URLSearchParams(search);
  } catch {
    return {};
  }
  const out: Record<string, string> = {};
  for (const key of ORDER) {
    const v = params.get(key);
    if (v === null || v === "") continue;
    if (key === "evidence" && !(EVIDENCE as readonly string[]).includes(v)) continue;
    out[key] = v;
  }
  return out as ItemFilter;
}

/**
 * The `/items` API query for the page's `search`: `?kind=defect&evidence=red`, keys in a fixed
 * order, or "" when no filter is set. Reading its own output gives it back unchanged.
 */
export function itemsQuery(search: string): string {
  const f = itemsFilter(search) as Record<string, string | undefined>;
  const kept = ORDER.filter((k) => f[k] !== undefined).map((k) => `${encodeURIComponent(k)}=${encodeURIComponent(f[k] as string)}`);
  return kept.length ? `?${kept.join("&")}` : "";
}

/** The CSV file's name for the filters set: `items-defect-red`, or `items` with none. */
export function itemsCsvName(search: string): string {
  const f = itemsFilter(search) as Record<string, string | undefined>;
  const parts = ORDER.map((k) => f[k]).filter((v): v is string => v !== undefined);
  const name = ["items", ...parts].join("-").replace(/[^A-Za-z0-9._-]+/g, "_");
  return name;
}
