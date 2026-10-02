import type { Activity, Adr, BoardReport, Charter, DocFolder, Lesson, PendingReview, PortfolioView, Project, ProjectSummary, RollupReport, Readiness, Sprint, SprintReport, Release } from "./types";

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
  /** A project's sprints (FEAT-119); only projects that switch them on have any. */
  getSprints: (id: string) => getJson<Sprint[]>(`/api/projects/${encodeURIComponent(id)}/sprints`),
  /** One sprint with its burndown — `active` names the running one. */
  getSprint: (id: string, code: string) =>
    getJson<SprintReport>(`/api/projects/${encodeURIComponent(id)}/sprints/${encodeURIComponent(code)}`),
  /** A project's releases (FEAT-120). */
  getReleases: (id: string) => getJson<Release[]>(`/api/projects/${encodeURIComponent(id)}/releases`),
  /** What each live item is missing, from the one engine every surface uses (FEAT-112). */
  getReadiness: (id: string) =>
    getJson<Readiness>(`/api/projects/${encodeURIComponent(id)}/readiness`),
  /** The workflow as Mermaid, drawn by the daemon with its gates as notes (FEAT-117). */
  getWorkflowDiagram: (id: string) =>
    getText(`/api/projects/${encodeURIComponent(id)}/workflow?format=mermaid`),
  /** Items whose definition is not currently agreed — computed by the daemon (FEAT-067). */
  getPendingReviews: (id: string) =>
    getJson<PendingReview[]>(`/api/projects/${encodeURIComponent(id)}/review`),
  /**
   * What this daemon can do (FEAT-067) and who it would attribute a verdict to (FEAT-077). A
   * read-only monitor answers `writes: false`; a board with no commit identity answers
   * `identity: null`, and the pages then refuse to offer an approval nobody can be named for.
   */
  getMeta: () => getJson<Meta>(`/api/meta`),
  /**
   * Record agreement to a definition, or withdraw it (FEAT-067, FEAT-069) — through the same
   * routes and the same content hash the CLI uses. Only reachable on a write-enabled daemon, which
   * is why the pages ask `getMeta` before offering either action.
   */
  approve: (id: string, code: string, by: string) =>
    verdict(id, code, "approve", { by }, "approve"),
  /** Record a named sign-off a stage asks for (FEAT-114). */
  signoff: async (id: string, code: string, name: string, by: string) => {
    const res = await fetch(
      `/api/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/signoff/${encodeURIComponent(name)}`,
      { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ by }) },
    );
    if (!res.ok) {
      throw new Error(
        res.status === 404 || res.status === 405
          ? "This monitor is read-only. Start it with `kanbanr review --ui` to sign off from here."
          : `sign-off failed: ${res.status} ${await res.text()}`,
      );
    }
    return res.json();
  },
  /** Every architecture decision on the board, newest first (FEAT-057). */
  getAdrs: (id: string) => getJson<Adr[]>(`/api/projects/${encodeURIComponent(id)}/adrs`),
  /** The user's verdict on a proposed decision (FEAT-153), through the route `kanbanr adr accept`
   * and `adr reject` use. */
  decideAdr: async (id: string, adr: string, verdict: "accept" | "reject", by: string, reason = "") => {
    const res = await fetch(
      `/api/write/projects/${encodeURIComponent(id)}/adrs/${encodeURIComponent(adr)}/${verdict}`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ by, reason }),
      },
    );
    if (!res.ok) {
      throw new Error(
        res.status === 404 || res.status === 405
          ? `This monitor is read-only. Start it with \`kanbanr review --ui\` to ${verdict} from here.`
          : `${verdict} failed: ${res.status} ${await res.text()}`,
      );
    }
    return res.json();
  },
  /** Agree after the fact to work built under a recorded bypass (FEAT-109). */
  ratify: (id: string, code: string, by: string) => verdict(id, code, "ratify", { by }, "ratify"),
  unapprove: (id: string, code: string, by: string, reason: string) =>
    verdict(id, code, "unapprove", { by, reason }, "withdraw"),
  /**
   * The name a verdict recorded from this monitor is attributed to. The board's commit identity is
   * the same source `kanbanr approve` defaults to, so a verdict reads identically whichever surface
   * gave it. Null when the board has no identity configured — the caller must then not offer the
   * action at all rather than invent one.
   */
  approver: (meta: Meta | null) => meta?.identity?.name?.trim() || null,
  /** The lessons that bear on one item — matched by the daemon on its labels, kind and goals. */
  getLessonsFor: (id: string, code: string) =>
    getJson<Lesson[]>(
      `/api/projects/${encodeURIComponent(id)}/lessons?for=${encodeURIComponent(code)}`,
    ),
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

/** What `/api/meta` answers: this daemon's capabilities and the identity it would record. */
export type Meta = {
  writes: boolean;
  schema_version?: number;
  identity: { name: string; email: string } | null;
};

/**
 * Post an approval verdict. A read-only daemon has no write route at all, so a 404 or 405 means
 * "this monitor cannot do that" rather than "something went wrong" — and says which command can.
 */
async function verdict(
  id: string,
  code: string,
  action: "approve" | "unapprove" | "ratify",
  body: Record<string, string>,
  verb: string,
) {
  const res = await fetch(
    `/api/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/${action}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
  if (!res.ok) {
    throw new Error(
      res.status === 404 || res.status === 405
        ? `This monitor is read-only. Start it with \`kanbanr review --ui\` to ${verb} from here.`
        : `${verb} failed: ${res.status} ${await res.text()}`,
    );
  }
  return res.json();
}
