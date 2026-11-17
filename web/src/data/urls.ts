// Every URL the shared cache keys a read by, spelled once (DEF-081), and the slice tags each read
// depends on, so a live change re-reads only what it touched.
import { apiUrl } from "../runtimeConfig.ts";
import type { ItemFilter } from "../types";

const enc = encodeURIComponent;
const project = (p: string) => `/projects/${enc(p)}`;

/** `?a=1&b=2` from the parameters that are set, in a fixed order; "" when none is. */
function query(params: [string, string | number | boolean | undefined][]): string {
  const kept = params.filter(([, v]) => v !== undefined && v !== "" && v !== false);
  if (kept.length === 0) return "";
  return "?" + kept.map(([k, v]) => `${enc(k)}=${enc(String(v))}`).join("&");
}

export const urls = {
  /** The compact index: items as cards, milestones, sprints, releases, ADRs, docs and goals. */
  index: (p: string) => apiUrl(`${project(p)}/index`),
  /** The project's configuration. */
  config: (p: string) => apiUrl(`${project(p)}/config`),
  /** One item, with its specification. */
  feature: (p: string, code: string) => apiUrl(`${project(p)}/features/${enc(code)}`),
  /** Items, without specifications, filtered by status and milestone when given. */
  features: (p: string, q: { status?: string; milestone?: string }) =>
    apiUrl(`${project(p)}/features${query([["milestone", q.milestone], ["status", q.status]])}`),
  readiness: (p: string) => apiUrl(`${project(p)}/readiness`),
  activity: (p: string, q: { ref?: string; ongoing?: boolean; limit?: number }) =>
    apiUrl(`${project(p)}/activity${query([["limit", q.limit], ["ongoing", q.ongoing], ["ref", q.ref]])}`),
  charter: (p: string) => apiUrl(`${project(p)}/charter`),
  sprints: (p: string) => apiUrl(`${project(p)}/sprints`),
  sprint: (p: string, s: string) => apiUrl(`${project(p)}/sprints/${enc(s)}`),
  gitModel: (p: string) => apiUrl(`${project(p)}/git/model`),
  docTree: (p: string) => apiUrl(`${project(p)}/docs`),
  /** One doc's markdown: text, not JSON. */
  doc: (p: string, path: string) => apiUrl(`${project(p)}/docs/content?path=${enc(path)}`),
  /** The lessons in force, or those that bear on one item when `for` is given. */
  lessons: (p: string, q: { for?: string } = {}) => apiUrl(`${project(p)}/lessons${query([["for", q.for]])}`),
  /** The project's agent scopes with their state (FEAT-214). */
  scopes: (p: string) => apiUrl(`${project(p)}/scopes`),
  /** Agent branches ahead of their items' tasks (FEAT-214); reads the code repository. */
  scopeDrift: (p: string) => apiUrl(`${project(p)}/scopes/drift`),
  releases: (p: string) => apiUrl(`${project(p)}/releases`),
  /** The workflow as a mermaid state diagram: text, not JSON. */
  workflowDiagram: (p: string) => apiUrl(`${project(p)}/workflow?format=mermaid`),
  process: (p: string) => apiUrl(`${project(p)}/process`),
  gates: (p: string) => apiUrl(`${project(p)}/gates`),
  /** The git model a cadence gives, with no repository read (FEAT-194). */
  gitModelPreview: (p: string, sprints: boolean, releases: boolean) =>
    apiUrl(`${project(p)}/git/model?sprints=${sprints ? "on" : "off"}&releases=${releases ? "on" : "off"}`),
  gitDoctor: (p: string) => apiUrl(`${project(p)}/git/doctor`),
  guards: (p: string) => apiUrl(`${project(p)}/guards`),
  /** Items whose definition is not currently agreed (FEAT-067). */
  pendingReviews: (p: string) => apiUrl(`${project(p)}/review`),
  /** What this daemon can do; a root read, so no board change re-reads it. */
  meta: () => apiUrl(`/meta`),
  /** The Review page's decision groups, counted (FTR-038). */
  reviewSummary: (p: string) => apiUrl(`${project(p)}/review/summary`),
  /** One review group's rows (FTR-038). */
  reviewGroup: (p: string, key: string) => apiUrl(`${project(p)}/review/groups/${enc(key)}`),
  /** One row's full evidence (FTR-038); a manual row's key `<CODE>/<R-n>/<test>` is one segment. */
  reviewRow: (p: string, key: string, row: string) =>
    apiUrl(`${project(p)}/review/groups/${enc(key)}/rows/${enc(row)}`),
  /** The latest background drift result, with the flags unless `flags` is false (FTR-045). */
  driftLatest: (p: string, flags = true) =>
    apiUrl(`${project(p)}/drift/latest${flags ? "" : "?flags=false"}`),
  /** What needs the person: review groups, releases at a stage, health and records (FTR-048). */
  attention: (p: string) => apiUrl(`${project(p)}/attention`),
  /** The project's Overview (FTR-048). */
  overview: (p: string) => apiUrl(`${project(p)}/overview`),
  /** Items as rows, filtered by every filter that is set (FTR-048). */
  items: (p: string, f: ItemFilter) =>
    apiUrl(
      `${project(p)}/items${query([
        ["kind", f.kind],
        ["status", f.status],
        ["milestone", f.milestone],
        ["goal", f.goal],
        ["sprint", f.sprint],
        ["release", f.release],
        ["evidence", f.evidence],
      ])}`,
    ),
  /** The person's planning decisions, oldest first, in one state when given (FTR-049). */
  planning: (p: string, state?: "pending" | "carried" | "declined") =>
    apiUrl(`${project(p)}/planning${query([["state", state]])}`),
  /** One planning decision checked against frozen scope, dependencies, capacity and dates (FTR-049). */
  planningCheck: (p: string, id: string) => apiUrl(`${project(p)}/planning/${enc(id)}/check`),
};

/** What `/attention` reads (FTR-048): items, releases, prepared records, manual tests, decisions, gates, docs, ADRs and config. */
const ATTENTION = ["items", "releases", "prepared", "manual", "decisions", "gates", "docs", "adrs", "config"] as const;

/** The slice tags each read depends on (A1 shared API §2). */
export const TAGS = {
  index: ["items", "config", "milestones", "sprints", "releases", "adrs", "docs", "charter"],
  config: ["config"],
  feature: (c: string) => [`item:${c}`, `spec:${c}`],
  features: ["items"],
  readiness: ["items", "config", "gates", "prepared"],
  activity: ["activity"],
  charter: ["charter"],
  sprints: ["sprints", "items"],
  docTree: ["docs"],
  /** One doc: the watcher names each changed doc by its path under docs/. */
  doc: (path: string) => [`doc:${path}`],
  lessons: ["lessons"],
  /** `lessons?for=`: matched against the item's labels, kind and goals. */
  lessonsFor: (c: string) => ["lessons", `item:${c}`],
  scopes: ["scopes"],
  releases: ["releases", "items"],
  workflowDiagram: ["config"],
  /** The config against the packs it follows. */
  process: ["config", "baseline"],
  gates: ["config"],
  gitModelPreview: ["config", "sprints"],
  /**
   * Reads that answer from the code repository, which sends no events: re-read on every change to
   * the board, as before DEF-081.
   */
  scopeDrift: ["*"],
  gitModel: ["*"],
  gitDoctor: ["*"],
  guards: ["*"],
  pendingReviews: ["*"],
  /** A root read: never invalidated by a project's change. */
  meta: [],
  /** Sent only by the daemon's background worker, after a drift run (FTR-045). */
  driftLatest: ["drift"],
  /** The Needs-you badge and drawer (FTR-048). */
  attention: ATTENTION,
  /** The Overview: what attention reads, plus sprints, milestones, the charter, lessons and activity (FTR-048). */
  overview: [...ATTENTION, "sprints", "milestones", "charter", "lessons", "activity"],
  /** The Items list (FTR-048). */
  items: ["items"],
  /** The planning decisions (FTR-049): `planning/*` in the watcher's table. */
  planning: ["planning"],
  /** A decision's check reads the items, sprints and releases it names. */
  planningCheck: ["planning", "items", "sprints", "releases"],
} as const;
