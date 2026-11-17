// The monitor's project navigation as data (FTR-047, A1 shared API §8). A project's pages are
// grouped into areas; the shell renders whatever this table says, so a page item adds its entry
// here, in one place, and never edits App.tsx. Pure: no React and no browser API, so node checks
// import it directly (web/scripts/check-nav.mjs).
import type { CompactIndex } from "./types.ts";

export type AreaKey = "overview" | "work" | "plan" | "evidence" | "architecture" | "records";

export interface NavEntry {
  key: string;
  label: string;
  /** The entry's URL for project `p` (the project id, not yet encoded). */
  path: (p: string) => string;
  /** Shown only when this holds for the project's index; hidden while the index is unknown. */
  when?: (ix: CompactIndex) => boolean;
  /** Extra path prefixes, beyond `/p/{p}`, that also mark this entry active. */
  match?: string[];
}

export interface NavArea {
  key: AreaKey;
  label: string;
  entries: NavEntry[];
}

const base = (p: string) => `/p/${encodeURIComponent(p)}`;

/** The v0.8.0 navigation. Each page item edits only its own line. */
export const NAV: NavArea[] = [
  // FTR-048 adds the Overview entry.
  { key: "overview", label: "Overview", entries: [] },
  {
    key: "work",
    label: "Work",
    entries: [
      // FTR-048 moves Board to /p/{p}/board.
      { key: "board", label: "Board", path: (p) => base(p), match: ["/feature/", "/state/"] },
      // Review is Work's second entry, after Board (R-7).
      { key: "review", label: "Review", path: (p) => `${base(p)}/review` },
      { key: "milestones", label: "Milestones", path: (p) => `${base(p)}/milestones`, match: ["/milestone/"] },
    ],
  },
  {
    key: "plan",
    label: "Plan",
    entries: [
      // FTR-043 replaces Schedule and Gantt with `plan`.
      { key: "schedule", label: "Schedule", path: (p) => `${base(p)}/schedule` },
      { key: "gantt", label: "Gantt", path: (p) => `${base(p)}/gantt` },
      {
        key: "releases",
        label: "Releases",
        path: (p) => `${base(p)}/releases`,
        when: (ix) => ix.config.cadence.releases === true,
      },
    ],
  },
  // The v0.9.0 views add Evidence's entries.
  { key: "evidence", label: "Evidence", entries: [] },
  {
    key: "architecture",
    label: "Architecture",
    entries: [
      { key: "workflow", label: "Workflow", path: (p) => `${base(p)}/workflow` },
      // The project's engineering baseline, beside its other technical decisions (FTR-033 R-6).
      { key: "baseline", label: "Baseline", path: (p) => `${base(p)}/baseline` },
    ],
  },
  {
    key: "records",
    label: "Records",
    entries: [
      { key: "charter", label: "Charter", path: (p) => `${base(p)}/charter` },
      { key: "docs", label: "Docs", path: (p) => `${base(p)}/docs`, match: ["/docs/"] },
      // FTR-005 T6 adds Catalog here.
    ],
  },
];

/**
 * The areas to show for a project: entries whose `when` is false are dropped, and so is an entry
 * with a `when` while the index is still `null`; then areas left with no entry are dropped.
 */
export function visibleAreas(nav: NavArea[], ix: CompactIndex | null): NavArea[] {
  const out: NavArea[] = [];
  for (const area of nav) {
    const entries = area.entries.filter((e) => !e.when || (ix !== null && e.when(ix)));
    if (entries.length > 0) out.push({ ...area, entries });
  }
  return out;
}

/** A path prefix without its trailing slashes: "/feature/" → "/feature", "/" → "". */
function trimSlash(s: string): string {
  return s.replace(/\/+$/, "");
}

/** Whether `rest` is `prefix` or lies under it, compared on whole path segments (L-23). */
function underPrefix(rest: string, prefix: string): boolean {
  if (prefix === "") return rest === "";
  return rest === prefix || rest.startsWith(prefix + "/");
}

/**
 * The entry a URL belongs to: the path beyond `/p/{project}` is matched against each entry's own
 * path and its `match` prefixes, and the longest match wins. An entry at exactly `/p/{project}`
 * matches only that page. `null` outside the project.
 */
export function activeEntry(
  nav: NavArea[],
  project: string,
  pathname: string,
): { area: AreaKey; entry: string } | null {
  const root = base(project);
  const path = trimSlash(pathname);
  if (path !== root && !path.startsWith(root + "/")) return null;
  const rest = path.slice(root.length);
  let best: { area: AreaKey; entry: string; len: number } | null = null;
  for (const area of nav) {
    for (const entry of area.entries) {
      const own = entry.path(project);
      const prefixes = [own.startsWith(root) ? trimSlash(own.slice(root.length)) : null, ...(entry.match ?? []).map(trimSlash)];
      for (const prefix of prefixes) {
        if (prefix === null || !underPrefix(rest, prefix)) continue;
        if (!best || prefix.length > best.len) best = { area: area.key, entry: entry.key, len: prefix.length };
      }
    }
  }
  return best ? { area: best.area, entry: best.entry } : null;
}

/** The active page's label ("Board", "Milestones", …), shown beside the mobile menu button; "" when none. */
export function hereLabel(nav: NavArea[], project: string, pathname: string): string {
  const at = activeEntry(nav, project, pathname);
  if (!at) return "";
  const area = nav.find((a) => a.key === at.area);
  return area?.entries.find((e) => e.key === at.entry)?.label ?? "";
}
