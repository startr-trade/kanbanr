// Every URL the shared cache keys a read by, spelled once (DEF-081), and the slice tags each read
// depends on, so a live change re-reads only what it touched.
import { apiUrl } from "../runtimeConfig.ts";

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
};

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
  /** Sent only by the daemon's background worker, after a drift run (FTR-045). */
  driftLatest: ["drift"],
} as const;
