import type {
  Activity,
  BoardReport,
  DocFolder,
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
