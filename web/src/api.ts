import { apiUrl, isReadOnly } from "./runtimeConfig.ts";
import type { Activity, Adr, BoardReport, Charter, DocFolder, Lesson, PendingReview, PortfolioView, Project, ProjectSummary, RollupReport, Readiness, Sprint, SprintReport, Release, ProcessStatus, Gate, CutPreview, ItemDiagrams, GitModelView, GitDoctorIssue, Guards } from "./types";

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
  listProjects: () => getJson<ProjectSummary[]>(apiUrl("/projects")),
  // Portfolio / program hierarchy (FEAT-030).
  getPortfolio: () => getJson<PortfolioView>(apiUrl("/portfolio")),
  getPortfolioRollups: () => getJson<RollupReport>(apiUrl("/portfolio/rollups")),
  getPortfolioBoard: () => getJson<BoardReport>(apiUrl("/portfolio/board")),
  getProject: (id: string) => getJson<Project>(apiUrl(`/projects/${encodeURIComponent(id)}`)),
  // Scheduling Gantt (FEAT-035): the daemon returns Mermaid `gantt` text.
  getProcess: (id: string) =>
    getJson<ProcessStatus>(apiUrl(`/projects/${encodeURIComponent(id)}/process`)),
  getCharter: (id: string) => getJson<Charter>(apiUrl(`/projects/${encodeURIComponent(id)}/charter`)),
  /** The gates moves are judged by, per status: role-addressed gates resolved, the built-in rule included (FEAT-199). */
  getGates: (id: string) =>
    getJson<Record<string, Gate>>(apiUrl(`/projects/${encodeURIComponent(id)}/gates`)),
  /** A project's sprints (FEAT-119); only projects that switch them on have any. */
  getSprints: (id: string) => getJson<Sprint[]>(apiUrl(`/projects/${encodeURIComponent(id)}/sprints`)),
  /** One sprint with its burndown — `active` names the running one. */
  getSprint: (id: string, code: string) =>
    getJson<SprintReport>(apiUrl(`/projects/${encodeURIComponent(id)}/sprints/${encodeURIComponent(code)}`)),
  /** A project's releases (FEAT-120). */
  getReleases: (id: string) => getJson<Release[]>(apiUrl(`/projects/${encodeURIComponent(id)}/releases`)),
  /**
   * The git model (FEAT-189), read by a daemon started in the code repository (FEAT-213). A
   * daemon started anywhere else refuses it; `getGitModelPreview` then gives the model from the
   * cadence alone.
   */
  getGitModel: (id: string) =>
    getJson<GitModelView>(apiUrl(`/projects/${encodeURIComponent(id)}/git/model`)),
  /** The model a cadence gives, with no repository needed (FEAT-194). */
  getGitModelPreview: (id: string, sprints: boolean, releases: boolean) =>
    getJson<GitModelView>(
      apiUrl(
        `/projects/${encodeURIComponent(id)}/git/model?sprints=${sprints ? "on" : "off"}&releases=${releases ? "on" : "off"}`,
      ),
    ),
  /** Where the board and the branches disagree (FEAT-191); needs the code repository. */
  getGitDoctor: (id: string) =>
    getJson<GitDoctorIssue[]>(apiUrl(`/projects/${encodeURIComponent(id)}/git/doctor`)),
  /** Every guard rail the project has on, in one read (FEAT-213). */
  getGuards: (id: string) => getJson<Guards>(apiUrl(`/projects/${encodeURIComponent(id)}/guards`)),
  /** What each live item is missing, from the one engine every surface uses (FEAT-112). */
  getReadiness: (id: string) =>
    getJson<Readiness>(apiUrl(`/projects/${encodeURIComponent(id)}/readiness`)),
  /**
   * Every Mermaid block of an item's spec and of each doc in its `affects_docs`, with the core
   * lint's findings (FEAT-200). The review page parses each with Pele before offering Approve.
   */
  getItemDiagrams: (id: string, code: string) =>
    getJson<ItemDiagrams>(
      apiUrl(`/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/diagrams`),
    ),
  /** The workflow as Mermaid, drawn by the daemon with its gates as notes (FEAT-117). */
  getWorkflowDiagram: (id: string) =>
    getText(apiUrl(`/projects/${encodeURIComponent(id)}/workflow?format=mermaid`)),
  /** Items whose definition is not currently agreed — computed by the daemon (FEAT-067). */
  getPendingReviews: (id: string) =>
    getJson<PendingReview[]>(apiUrl(`/projects/${encodeURIComponent(id)}/review`)),
  /**
   * What this daemon can do (FEAT-067) and who it would attribute a verdict to (FEAT-077). A
   * read-only monitor answers `writes: false`; a board with no commit identity answers
   * `identity: null`, and the pages then refuse to offer an approval nobody can be named for.
   *
   * A monitor configured `readOnly` (the standalone bundle, FEAT-178) answers `writes: false`
   * whatever the daemon says, so every page that gates a decision on it shows the CLI command.
   */
  getMeta: async () => {
    const meta = await getJson<Meta>(apiUrl(`/meta`));
    return isReadOnly() ? { ...meta, writes: false } : meta;
  },
  /**
   * Record agreement to a definition, or withdraw it (FEAT-067, FEAT-069) — through the same
   * routes and the same content hash the CLI uses. Only reachable on a write-enabled daemon, which
   * is why the pages ask `getMeta` before offering either action.
   */
  approve: (id: string, code: string, by: string, rev = "") =>
    verdict(id, code, "approve", { by, rev }, "approve"),
  /** Record a named sign-off a stage asks for (FEAT-114). */
  signoff: async (id: string, code: string, name: string, by: string, rev = "") => {
    const res = await postWrite(
      apiUrl(`/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/signoff/${encodeURIComponent(name)}`),
      { by, rev },
      `kanbanr signoff ${code} ${name}`,
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
  /** Take back a sign-off given in error, saying why (FEAT-182). */
  withdrawSignoff: async (id: string, code: string, name: string, by: string, reason: string) => {
    const res = await postWrite(
      apiUrl(`/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/signoff/${encodeURIComponent(name)}/withdraw`),
      { by, reason },
      `kanbanr signoff ${code} ${name} --withdraw --reason "…"`,
    );
    if (!res.ok) {
      throw new Error(
        res.status === 404 || res.status === 405
          ? "This monitor is read-only. Start it with `kanbanr review --ui` to withdraw a sign-off from here."
          : `withdrawal failed: ${res.status} ${await res.text()}`,
      );
    }
    return res.json();
  },
  /** Plan an item into a release (FEAT-185), through the route `kanbanr release plan` uses. */
  planIntoRelease: (id: string, version: string, code: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/plan`,
      { items: [code] },
      "plan into a release",
      `kanbanr release plan ${version} ${code}`,
    ),
  /** What cutting `version` now would ship and carry over, nothing written (FEAT-185). */
  getCutPreview: (id: string, version: string) =>
    getJson<CutPreview[]>(
      apiUrl(`/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/cut-preview`),
    ),
  /** Add a planned release (FEAT-185), as `kanbanr release add` does. */
  addRelease: (id: string, version: string, target: string, name = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases`,
      { version, target, name },
      "add a release",
      `kanbanr release add ${version}`,
    ),
  /**
   * Move a release to its next stage, or the named one (FEAT-183, FEAT-185): the same core
   * promotion as `kanbanr release promote`, recorded as `by`. The last stage is the cut.
   */
  promoteRelease: (id: string, version: string, by: string, stage?: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/promote`,
      stage ? { by, stage } : { by },
      "promote a release",
      `kanbanr release promote ${version}${stage ? ` ${stage}` : ""}`,
    ),
  /** Every architecture decision on the board, newest first (FEAT-057). */
  getAdrs: (id: string) => getJson<Adr[]>(apiUrl(`/projects/${encodeURIComponent(id)}/adrs`)),
  /** The user's verdict on a proposed decision (FEAT-153), through the route `kanbanr adr accept`
   * and `adr reject` use. */
  decideAdr: async (id: string, adr: string, verdict: "accept" | "reject", by: string, reason = "") => {
    const res = await postWrite(
      apiUrl(`/write/projects/${encodeURIComponent(id)}/adrs/${encodeURIComponent(adr)}/${verdict}`),
      { by, reason },
      `kanbanr adr ${verdict} ${adr}`,
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
  ratify: (id: string, code: string, by: string, rev = "") =>
    verdict(id, code, "ratify", { by, rev }, "ratify"),
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
      apiUrl(`/projects/${encodeURIComponent(id)}/lessons?for=${encodeURIComponent(code)}`),
    ),
  getLessons: (id: string) =>
    getJson<Lesson[]>(apiUrl(`/projects/${encodeURIComponent(id)}/lessons`)),
  getGantt: (id: string) => getText(apiUrl(`/projects/${encodeURIComponent(id)}/gantt`)),
  getDocTree: (id: string) => getJson<DocFolder>(apiUrl(`/projects/${encodeURIComponent(id)}/docs`)),
  getDoc: (id: string, path: string) =>
    getText(apiUrl(`/projects/${encodeURIComponent(id)}/docs/content?path=${encodeURIComponent(path)}`)),
  getActivity: (id: string, params = "") =>
    getJson<Activity[]>(apiUrl(`/projects/${encodeURIComponent(id)}/activity${params}`)),
  // Raw bytes of a doc asset (e.g. an image), served with a content-type by the daemon.
  docRawUrl: (id: string, path: string) =>
    apiUrl(`/projects/${encodeURIComponent(id)}/docs/raw?path=${encodeURIComponent(path)}`),
  // Export is a plain GET; a direct href download is fine (no auth).
  exportUrl: (id: string, code: string, format: "md" | "json") =>
    apiUrl(`/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/export?format=${format}`),
  projectEvents: (id: string) => apiUrl(`/projects/${encodeURIComponent(id)}/events`),
  allEvents: () => apiUrl("/events"),
};

/** What the daemon's `meta` route answers: this daemon's capabilities and the identity it would record. */
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
  const res = await postWrite(
    apiUrl(`/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/${action}`),
    body,
    `kanbanr ${action} ${code}`,
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

/**
 * POST to a write route — unless this monitor is configured read-only (FEAT-178), in which case
 * nothing is sent and the error names the CLI command that records the same decision. The pages do
 * not offer the action then; this is the guarantee that the standalone bundle never calls a write
 * route even if one did.
 */
async function postWrite(url: string, body: unknown, command: string): Promise<Response> {
  if (isReadOnly()) {
    throw new Error(`This monitor is read-only. Run \`${command}\` instead.`);
  }
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

/**
 * Post to a daemon write route (FEAT-185), through the configured API base and the read-only
 * guard (FEAT-178). A daemon without writes has no such route, so a 404 or 405 says which monitor
 * can; any other refusal is the daemon's own message, shown as it gave it.
 */
async function writePost(path: string, body: unknown, verb: string, command: string) {
  const res = await postWrite(apiUrl(`/write${path}`), body, command);
  if (!res.ok) {
    throw new Error(
      res.status === 404 || res.status === 405
        ? `This monitor is read-only. Start it with \`kanbanr review --ui\` to ${verb} from here.`
        : `${verb} failed: ${res.status} ${await res.text()}`,
    );
  }
  return res.json();
}
