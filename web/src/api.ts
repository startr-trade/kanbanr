import type {
  Activity,
  BoardReport,
  Charter,
  DocFolder,
  Lesson,
  PendingReview,
  PortfolioView,
  Project,
  ProjectSummary,
  RollupReport,
} from "./types";

// The monitor is a read-only view of the local data folder served by the view daemon — no auth.

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText}: ${await res.text()}`);
  }
  return res.json() as Promise<T>;
}

async function getText(url: string): Promise<string> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText}: ${await res.text()}`);
  }
  return res.text();
}

export const api = {
  listProjects: () => getJson<ProjectSummary[]>("/api/projects"),
  // Portfolio / program hierarchy (FEAT-030).
  getPortfolio: () => getJson<PortfolioView>("/api/portfolio"),
  getPortfolioRollups: () => getJson<RollupReport>("/api/portfolio/rollups"),
  getPortfolioBoard: () => getJson<BoardReport>("/api/portfolio/board"),
  getProject: (id: string) => getJson<Project>(`/api/projects/${encodeURIComponent(id)}`),
  // Scheduling Gantt (FEAT-035): the daemon returns Mermaid `gantt` text.
  getCharter: (id: string) => getJson<Charter>(`/api/projects/${encodeURIComponent(id)}/charter`),
  /** Items whose definition is not currently agreed — computed by the daemon (FEAT-067). */
  getPendingReviews: (id: string) =>
    getJson<PendingReview[]>(`/api/projects/${encodeURIComponent(id)}/review`),
  /** What this daemon can do (FEAT-067). A read-only monitor answers `writes: false`. */
  getMeta: () => getJson<{ writes: boolean }>(`/api/meta`),
  /**
   * Record agreement to a definition, through the same route and the same content hash the CLI
   * uses. Only reachable when the daemon was started with `--allow-writes`.
   */
  approve: async (id: string, code: string, by: string) => {
    const res = await fetch(
      `/api/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/approve`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ by }),
      },
    );
    if (!res.ok) {
      throw new Error(
        res.status === 404
          ? "This monitor is read-only. Start it with `kanbanr review --ui` to approve from here."
          : `approve failed: ${res.status} ${await res.text()}`,
      );
    }
    return res.json();
  },
  getLessons: (id: string) =>
    getJson<Lesson[]>(`/api/projects/${encodeURIComponent(id)}/lessons`),
  getGantt: (id: string) => getText(`/api/projects/${encodeURIComponent(id)}/gantt`),
  getDocTree: (id: string) => getJson<DocFolder>(`/api/projects/${encodeURIComponent(id)}/docs`),
  getDoc: (id: string, path: string) =>
    getText(`/api/projects/${encodeURIComponent(id)}/docs/content?path=${encodeURIComponent(path)}`),
  getActivity: (id: string, params = "") =>
    getJson<Activity[]>(`/api/projects/${encodeURIComponent(id)}/activity${params}`),
  // Raw bytes of a doc asset (e.g. an image), served with a content-type by the daemon.
  docRawUrl: (id: string, path: string) =>
    `/api/projects/${encodeURIComponent(id)}/docs/raw?path=${encodeURIComponent(path)}`,
  // Export is a plain GET; a direct href download is fine (no auth).
  exportUrl: (id: string, code: string, format: "md" | "json") =>
    `/api/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/export?format=${format}`,
  projectEvents: (id: string) => `/api/projects/${encodeURIComponent(id)}/events`,
  allEvents: () => "/api/events",
};
