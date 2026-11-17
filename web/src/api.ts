import { apiUrl, isReadOnly } from "./runtimeConfig.ts";
import { urls } from "./data/urls.ts";
import type { CompactIndex, DriftLatest, Feature, ProjectConfig } from "./types";
import type { BaselineRevision, BaselineView, ProfileStatus } from "./types";
import type { CatalogStatus, FactsOrEmpty, SystemBaselineReview, TestTiming } from "./types";
import type { Attention, ItemFilter, ItemRow, Overview } from "./types";
import type { FlowPlan, PlanScope, PlanSlice, PlanSummary, PortfolioPlan, RowBody } from "./types";
import type { ReleaseItemRow, ReleasesSummary, Scope } from "./types";
import type { Activity, Adr, BoardReport, Charter, DocFolder, Lesson, PendingReview, PortfolioView, Project, ProjectSummary, BoardInfo, RollupReport, Readiness, Sprint, SprintReport, Release, ProcessStatus, Gate, CutPreview, ItemDiagrams, GitModelView, GitDoctorIssue, Guards, AgentScope, ScopeDrift, ReleasePlan, ReleaseNotes, PlannedRelease, DriftChecked, PreparedView, Resolution, DecisionsView, ModeChanged, Delegation, ManualWaiting, GroupCount, GroupSlice, RowEvidence, GroupDecision, GroupDecisionAnswer } from "./types";
import type { PickView, Picked, RepoDoc, RepoTree } from "./types";
import type { DocViews, KatalogrStatus } from "./types";
import type { PlanningAssessment, PlanningDecision, PlanningRequest, PlanningView } from "./types";
import type { CharterProgress } from "./types";

// The monitor is a read-only view of the local data folder served by the view daemon — no auth.

/** GET a JSON read; a non-2xx answer is an error carrying its status and body. */
export async function getJson<T>(url: string): Promise<T> {
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

/** `?board=<name>` for a global read on a server with several boards (FTR-040); empty otherwise. */
function boardQuery(board?: string | null): string {
  return board ? `?board=${encodeURIComponent(board)}` : "";
}

export const api = {
  listProjects: () => getJson<ProjectSummary[]>(apiUrl("/projects")),
  // Portfolio / program hierarchy (FEAT-030).
  // The boards a server shows; an older daemon has no such route, and that is one board (FTR-040).
  getBoards: () => getJson<BoardInfo[]>(apiUrl("/boards")).catch(() => [] as BoardInfo[]),
  getPortfolio: (board?: string | null) => getJson<PortfolioView>(apiUrl(`/portfolio${boardQuery(board)}`)),
  getPortfolioRollups: (board?: string | null) =>
    getJson<RollupReport>(apiUrl(`/portfolio/rollups${boardQuery(board)}`)),
  getPortfolioBoard: (board?: string | null) =>
    getJson<BoardReport>(apiUrl(`/portfolio/board${boardQuery(board)}`)),
  getProject: (id: string) => getJson<Project>(apiUrl(`/projects/${encodeURIComponent(id)}`)),
  // The monitor's slices (DEF-081): pages read these through useQuery and `urls`, not the whole board.
  /** The compact index: items as cards, milestones, sprints, releases, ADRs, docs and goals. */
  getIndex: (id: string) => getJson<CompactIndex>(urls.index(id)),
  /** The project's configuration. */
  getConfig: (id: string) => getJson<ProjectConfig>(urls.config(id)),
  /** One item, with its specification. */
  getFeature: (id: string, code: string) => getJson<Feature>(urls.feature(id, code)),
  /** Items without specifications, by status and milestone when given. */
  getFeatures: (id: string, q: { status?: string; milestone?: string }) => getJson<Feature[]>(urls.features(id, q)),
  /** The latest background drift result, never computed on request (FTR-045); `flags: false` leaves the flags out. */
  getDriftLatest: (id: string, q: { flags?: boolean } = {}) =>
    getJson<DriftLatest>(urls.driftLatest(id, q.flags ?? true)),
  /** What needs the person, for the Needs-you badge and drawer (FTR-048). */
  getAttention: (id: string) => getJson<Attention>(urls.attention(id)),
  /** The project's Overview (FTR-048). */
  getOverview: (id: string) => getJson<Overview>(urls.overview(id)),
  /** Items as rows, filtered (FTR-048). */
  getItems: (id: string, f: ItemFilter) => getJson<ItemRow[]>(urls.items(id, f)),
  /** The Plan page's counts per tab and the tab it opens on (FTR-043). */
  getPlanSummary: (id: string) => getJson<PlanSummary>(urls.planSummary(id)),
  /** One Plan tab: its timeline's span and its rows (FTR-043). */
  getPlan: (id: string, scope: PlanScope) => getJson<PlanSlice>(urls.plan(id, scope)),
  /** One Plan row's items, read when it is opened (FTR-043). */
  getPlanRow: (id: string, key: string) => getJson<RowBody>(urls.planRow(id, key)),
  /** The flow plan in agent-hours for a scope, the running sprint's by default (FTR-061). */
  getPlanFlow: (id: string, scope?: string) => getJson<FlowPlan>(urls.planFlow(id, scope)),
  /** The portfolio's timeline, one row per project, with its window (FTR-043). */
  getPortfolioPlan: (board?: string | null) => getJson<PortfolioPlan>(urls.portfolioPlan(board)),
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
  /** One Releases tab's releases, newest first (FTR-041). */
  getReleaseSlice: (id: string, scope: Scope) => getJson<Release[]>(urls.releases(id, scope)),
  /** The Releases page's counts per tab and the tab it opens on (FTR-041). */
  getReleasesSummary: (id: string) => getJson<ReleasesSummary>(urls.releasesSummary(id)),
  charterProgress: (p: string) => getJson<CharterProgress>(urls.charterProgress(p)),
  /** One release's item table (FTR-041). */
  getReleaseItems: (id: string, version: string) => getJson<ReleaseItemRow[]>(urls.releaseItems(id, version)),
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
  /** A project's agent scopes with their state (FEAT-214). */
  getScopes: (id: string) => getJson<AgentScope[]>(apiUrl(`/projects/${encodeURIComponent(id)}/scopes`)),
  /** Agent branches ahead of their items' tasks (FEAT-214); needs the code repository. */
  getScopeDrift: (id: string) =>
    getJson<ScopeDrift[]>(apiUrl(`/projects/${encodeURIComponent(id)}/scopes/drift`)),
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
  signoff: async (
    id: string,
    code: string,
    name: string,
    by: string,
    rev = "",
    specRev = "",
    checked: DriftChecked[] = [],
  ) => {
    const res = await postWrite(
      apiUrl(`/write/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/signoff/${encodeURIComponent(name)}`),
      checked.length ? { by, rev, spec_rev: specRev, checked } : { by, rev, spec_rev: specRev },
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
    ) as Promise<PlannedRelease>,
  /** What cutting `version` now would ship and carry over, nothing written (FEAT-185). */
  getCutPreview: (id: string, version: string) =>
    getJson<CutPreview[]>(
      apiUrl(`/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/cut-preview`),
    ),
  /**
   * What promoting `version` to its next stage would do, nothing changed (FEAT-212): the button,
   * each step and condition, what is missing and, for the cut, what ships. Never fetches.
   */
  getReleasePlan: (id: string, version: string) => getJson<ReleasePlan>(urls.releasePlan(id, version)),
  /**
   * A release's notes as they are on `release/<V>` (FEAT-216), read from the code repository the
   * daemon was started in.
   */
  getReleaseNotes: (id: string, version: string) =>
    getJson<ReleaseNotes>(
      apiUrl(`/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/notes`),
    ),
  /** Commit `text` as a release's notes onto `release/<V>` (FEAT-216): the new sha and the notes. */
  saveReleaseNotes: (id: string, version: string, text: string, by: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/notes`,
      { text, by },
      "save release notes",
      `kanbanr release notes ${version} --edit`,
      "PUT",
    ) as Promise<{ sha: string; notes: ReleaseNotes; main_moved?: string }>,
  /** Add a planned release (FEAT-185), as `kanbanr release add` does. */
  addRelease: (id: string, version: string, target: string, name = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases`,
      { version, target, name },
      "add a release",
      `kanbanr release add ${version}`,
    ),
  /**
   * Remove a release that holds nothing (FTR-022), as `kanbanr release remove` does; the daemon
   * refuses any other and says why.
   */
  removeRelease: (id: string, version: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}`,
      undefined,
      "remove a release",
      `kanbanr release remove ${version}`,
      "DELETE",
    ),
  /**
   * Move a release to its next stage, or the named one (FEAT-183, FEAT-185): the same core
   * promotion as `kanbanr release promote`, recorded as `by`. The last stage is the cut.
   */
  promoteRelease: (
    id: string,
    version: string,
    by: string,
    stage?: string,
    expect?: string,
    reason?: string,
    acceptUntested?: string[],
  ) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(version)}/promote`,
      // `expect`: the release tip the plan showed (FEAT-212) — refused if the branch moved since.
      // `reason`: for a stage after the tag, what the user checked (FTR-026).
      // `accept_untested`: at the cut, what the person accepts shipping unverified (DEF-043).
      {
        by,
        ...(stage ? { stage } : {}),
        ...(expect ? { expect } : {}),
        ...(reason ? { reason } : {}),
        ...(acceptUntested?.length ? { accept_untested: acceptUntested } : {}),
      },
      "promote a release",
      `kanbanr release promote ${version}${stage ? ` ${stage}` : ""}${reason ? ` --note "${reason}"` : ""}${
        acceptUntested?.length ? ` --accept-untested ${acceptUntested.join(",")}` : ""
      }`,
    ),
  /**
   * Start a patch release from a release whose tag's recorded result is failed (FTR-026): a defect
   * carrying the run's link and the notes, planned with the release's items into the patch.
   */
  patchFromFailedTag: (id: string, parent: string, version: string, by: string, link: string, notes: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/releases/${encodeURIComponent(parent)}/patch-from-tag`,
      { version, by, ...(link ? { link } : {}), ...(notes ? { notes } : {}) },
      "start a patch release",
      `kanbanr release add ${version} --patch-of ${parent} --from-failed-tag`,
    ) as Promise<{ release: Release; defect: string; planned: string[] }>,
  /**
   * Record a gate's verdict for a commit (FTR-002), through the route `kanbanr gate record` uses.
   * What your CI said, as a person saw it: kanbanr asks no server.
   */
  recordGate: (id: string, name: string, commit: string, passed: boolean, by: string, url = "", note = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/gate-results`,
      { name, commit, passed, by, ...(url ? { url } : {}), ...(note ? { note } : {}) },
      "record a gate result",
      `kanbanr gate record ${name} ${passed ? "--passed" : "--failed"} --commit ${commit}`,
    ),
  /** The manual tests waiting for a person, live work first (DEF-040). */
  getManualTests: (id: string) =>
    getJson<ManualWaiting[]>(apiUrl(`/projects/${encodeURIComponent(id)}/manual-tests`)),
  /**
   * Record a manual test as passed or failed, as the person who checked it (DEF-040), through the
   * route `kanbanr test` uses; the daemon keeps who, when and the note beside the test's state.
   */
  recordManual: (id: string, code: string, requirement: string, test: string, passed: boolean, by: string, note = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/tests/${encodeURIComponent(requirement)}/${encodeURIComponent(test)}`,
      { state: passed ? "green" : "red", by, ...(note ? { note } : {}) },
      "record a manual test",
      `kanbanr test ${code} ${requirement} "${test}" ${passed ? "green" : "red"}`,
      "PUT",
    ),
  /**
   * The records Claude prepared for the person to confirm (FTR-034), newest first: the open ones,
   * or with `all` every one. Each says where it stands and the equivalent verdict command.
   */
  getPrepared: (id: string, opts: { all?: boolean; item?: string; release?: string } = {}) => {
    const q = [`state=${opts.all ? "all" : "open"}`];
    if (opts.item) q.push(`item=${encodeURIComponent(opts.item)}`);
    if (opts.release) q.push(`release=${encodeURIComponent(opts.release)}`);
    return getJson<PreparedView[]>(apiUrl(`/projects/${encodeURIComponent(id)}/prepared?${q.join("&")}`));
  },
  /**
   * Confirm a prepared record (FTR-034): the server makes the decision's own verdict as `by`, with
   * `edits` to the editable fields and the pins the page showed. A read-only monitor names the
   * equivalent command for the person's own terminal.
   */
  confirmPrepared: (
    id: string,
    view: PreparedView,
    by: string,
    edits: Record<string, unknown>,
    pins: { rev?: string; spec_rev?: string; expect?: string } = {},
  ) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/prepared/${encodeURIComponent(view.id ?? "")}/confirm`,
      {
        by,
        ...(Object.keys(edits).length ? { edits } : {}),
        ...(pins.rev ? { rev: pins.rev } : {}),
        ...(pins.spec_rev ? { spec_rev: pins.spec_rev } : {}),
        ...(pins.expect ? { expect: pins.expect } : {}),
      },
      "confirm what Claude prepared",
      view.command || "kanbanr prepare list",
    ),
  /** Turn a prepared record down (FTR-034); the reason is optional. Nothing else changes. */
  rejectPrepared: (id: string, prepared: string, by: string, reason = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/prepared/${encodeURIComponent(prepared)}/reject`,
      { by, ...(reason ? { reason } : {}) },
      "reject what Claude prepared",
      "kanbanr prepare list",
    ) as Promise<Resolution>,
  /**
   * Confirm every open record of a batch with one click (FTR-037): a walkthrough run's checks. The
   * server makes each record's verdict as `by`, in one commit, and says which went through.
   */
  confirmPreparedBatch: (id: string, batch: string, by: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/prepared/batch/${encodeURIComponent(batch)}/confirm`,
      { by },
      "confirm the batch Claude prepared",
      "kanbanr prepare list",
    ) as Promise<BatchAnswer>,
  /** Turn a whole batch down at once (FTR-037); the reason is optional. */
  rejectPreparedBatch: (id: string, batch: string, by: string, reason = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/prepared/batch/${encodeURIComponent(batch)}/reject`,
      { by, ...(reason ? { reason } : {}) },
      "reject the batch Claude prepared",
      "kanbanr prepare list",
    ) as Promise<BatchAnswer>,
  /**
   * Where this project's decisions are taken (FTR-028): the mode, the delegations, the decisions
   * Claude took under one for the person to keep or undo, and what waits where.
   */
  getDecisions: (id: string) => getJson<DecisionsView>(apiUrl(`/projects/${encodeURIComponent(id)}/decisions`)),
  /** The person sets the decision mode; the answer lists what waits where now. */
  setDecisionMode: (id: string, mode: string, by: string, reason = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/decisions/mode`,
      { mode, by, ...(reason ? { reason } : {}) },
      "change where decisions are taken",
      `kanbanr decisions mode ${mode}`,
      "PUT",
    ) as Promise<ModeChanged>,
  /** The person grants a proposed delegation, pinned to the `rev` the page showed. */
  grantDelegation: (id: string, delegation: string, rev: string, by: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/decisions/delegations/${encodeURIComponent(delegation)}/grant`,
      { rev, by },
      "grant a delegation",
      `kanbanr decisions grant ${delegation}`,
    ) as Promise<Delegation>,
  /** Revoke a delegation, or discard a proposal; a reason is required. */
  revokeDelegation: (id: string, delegation: string, by: string, reason: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/decisions/delegations/${encodeURIComponent(delegation)}/revoke`,
      { reason, by },
      "revoke a delegation",
      `kanbanr decisions revoke ${delegation} --reason "…"`,
    ) as Promise<Delegation>,
  /** The person keeps or undoes a decision Claude took under a delegation. */
  reviewDelegated: (id: string, decision: string, verdict: "keep" | "undo", by: string, reason = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/decisions/reviews`,
      { decision, verdict, by, ...(reason ? { reason } : {}) },
      `${verdict} a delegated decision`,
      `kanbanr decisions review ${decision} --${verdict}`,
    ),
  /** The Review page's decision groups, each counted; empty groups are absent (FTR-038). */
  getReviewSummary: (id: string) => getJson<GroupCount[]>(urls.reviewSummary(id)),
  /** One review group's rows, in the page's default order (FTR-038). */
  getReviewGroup: (id: string, key: string) => getJson<GroupSlice>(urls.reviewGroup(id, key)),
  /** One row's full evidence, read when the row is opened (FTR-038). */
  getReviewRow: (id: string, key: string, row: string) => getJson<RowEvidence>(urls.reviewRow(id, key, row)),
  /**
   * One decision over the rows the person selected in one group (FTR-038): each row gets its own
   * verdict, pinned to what the page showed, in one commit. A refused row is named in its result
   * and the rest still go through.
   */
  decideGroup: (id: string, body: GroupDecision) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/review/decide`,
      body,
      "take a group decision",
      "kanbanr review",
    ) as Promise<GroupDecisionAnswer>,
  /**
   * The person's planning decision (FTR-049): recorded as theirs, in one commit, and changing no
   * plan; Claude checks it and carries it out with `--decision`, or declines it with a reason. On
   * a read-only monitor the person can plan in a terminal instead (R-9).
   */
  recordPlanning: (id: string, body: PlanningRequest & { by: string; note?: string }) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/planning`,
      body,
      "record a planning decision",
      planCommand(body),
    ) as Promise<PlanningDecision>,
  /** The planning decisions, oldest first, in one state when given (FTR-049). */
  getPlanning: (id: string, state?: "pending" | "carried" | "declined") =>
    getJson<PlanningView[]>(urls.planning(id, state)),
  /** One decision's problems, warnings and nearest valid move (FTR-049). */
  checkPlanning: (id: string, decision: string) => getJson<PlanningAssessment>(urls.planningCheck(id, decision)),
  // The engineering baseline (FTR-033): read by the Baseline page only, never by the whole-board fetch.
  /** The baseline: the packs followed, the areas and their answers, every revision, the charter's constraints. */
  getBaseline: (id: string) => getJson<BaselineView>(apiUrl(`/projects/${encodeURIComponent(id)}/baseline`)),
  /** The packs the project follows, each against its library's current copy. */
  getProfile: (id: string) => getJson<ProfileStatus>(apiUrl(`/projects/${encodeURIComponent(id)}/profile`)),
  /** The person accepts a proposed baseline revision, pinned to the `rev` the page showed: theirs alone. */
  acceptBaseline: (id: string, revision: string, rev: string, by: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/baseline/revisions/${encodeURIComponent(revision)}/accept`,
      { rev, by },
      "accept a baseline revision",
      `kanbanr baseline accept ${revision}`,
    ) as Promise<BaselineRevision>,
  /** Withdraw a proposed baseline revision; a reason is required. */
  withdrawBaseline: (id: string, revision: string, by: string, reason: string) =>
    writePost(
      `/projects/${encodeURIComponent(id)}/baseline/revisions/${encodeURIComponent(revision)}/withdraw`,
      { reason, by },
      "withdraw a baseline revision",
      `kanbanr baseline withdraw ${revision} --reason "…"`,
    ) as Promise<BaselineRevision>,
  // The project's facts and its content catalog (FTR-005): read by the Catalog page only.
  /** The catalog's derived status: every row with its source state and protections, and the gaps. */
  getCatalog: (id: string) => getJson<CatalogStatus>(apiUrl(`/projects/${encodeURIComponent(id)}/catalog`)),
  /** The system baseline's state and the open prepared acceptance (FTR-006). */
  systemBaseline: (id: string) => getJson<SystemBaselineReview>(urls.systemBaseline(id)),
  /**
   * The person accepts the system baseline as they saw it (FTR-006): `rev` is the pins' hash the page
   * showed, so a source that changed since is refused rather than accepted unread. `prepared` is the
   * record being confirmed, when there is one. Theirs alone; there is no `init` here, the CLI does it.
   */
  acceptSystemBaseline: (id: string, by: string, note: string, prepared = "", rev = "") =>
    writePost(
      `/projects/${encodeURIComponent(id)}/catalog/baseline/accept`,
      { by, note, ...(prepared ? { prepared } : {}), ...(rev ? { rev } : {}) },
      "accept the system baseline",
      note ? `kanbanr catalog baseline accept --note ${JSON.stringify(note)}` : "kanbanr catalog baseline accept",
    ) as Promise<SystemBaselineReview>,
  /** The project's facts record, or `{}` when none is set. */
  getFacts: (id: string) => getJson<FactsOrEmpty>(apiUrl(`/projects/${encodeURIComponent(id)}/facts`)),
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
  /** The last kept duration of each of one item's tests (FTR-058). */
  getTimings: (id: string, code: string) =>
    getJson<{ tests: TestTiming[] }>(
      apiUrl(`/projects/${encodeURIComponent(id)}/timings?code=${encodeURIComponent(code)}`),
    ),
  getLessons: (id: string) =>
    getJson<Lesson[]>(apiUrl(`/projects/${encodeURIComponent(id)}/lessons`)),
  getGantt: (id: string) => getText(apiUrl(`/projects/${encodeURIComponent(id)}/gantt`)),
  getDocTree: (id: string) => getJson<DocFolder>(apiUrl(`/projects/${encodeURIComponent(id)}/docs`)),
  getDoc: (id: string, path: string) =>
    getText(apiUrl(`/projects/${encodeURIComponent(id)}/docs/content?path=${encodeURIComponent(path)}`)),
  // A doc's `katalogr-query` blocks asked of katalogr by the daemon, and the link's own state (FTR-016).
  getDocViews: (id: string, path: string) =>
    getJson<DocViews>(apiUrl(`/projects/${encodeURIComponent(id)}/docs/views?path=${encodeURIComponent(path)}`)),
  getKatalogr: (id: string) => getJson<KatalogrStatus>(apiUrl(`/projects/${encodeURIComponent(id)}/katalogr`)),
  getActivity: (id: string, params = "") =>
    getJson<Activity[]>(apiUrl(`/projects/${encodeURIComponent(id)}/activity${params}`)),
  // Docs read at a position (FTR-004): the picker's entries, a pick resolved, the repository's docs
  // and one doc at a pick, and a doc's raw bytes for its images.
  getPositions: (id: string) => getJson<PickView[]>(urls.positions(id)),
  getPosition: (id: string, at: string) =>
    getJson<Picked>(apiUrl(`/projects/${encodeURIComponent(id)}/positions/resolve?at=${encodeURIComponent(at)}`)),
  getRepoTree: (id: string, at: string) => getJson<RepoTree>(urls.repoDocs(id, at)),
  getRepoDoc: (id: string, path: string, at: string) => getJson<RepoDoc>(urls.repoDoc(id, path, at)),
  repoDocRawUrl: (id: string, path: string, at: string) => urls.repoDocRaw(id, path, at),
  // Raw bytes of a doc asset (e.g. an image), served with a content-type by the daemon.
  docRawUrl: (id: string, path: string) =>
    apiUrl(`/projects/${encodeURIComponent(id)}/docs/raw?path=${encodeURIComponent(path)}`),
  // Export is a plain GET; a direct href download is fine (no auth).
  exportUrl: (id: string, code: string, format: "md" | "json") =>
    apiUrl(`/projects/${encodeURIComponent(id)}/features/${encodeURIComponent(code)}/export?format=${format}`),
  projectEvents: (id: string) => apiUrl(`/projects/${encodeURIComponent(id)}/events`),
  allEvents: () => apiUrl("/events"),
};

/** The answer to a batch confirmation or rejection (FTR-037): each record and whether it went through. */
export type BatchAnswer = {
  batch: string;
  results: { id: string; kind: string; verdict: "confirmed" | "rejected"; made?: string; error?: string }[];
};

/** What the daemon's `meta` route answers: this daemon's capabilities and the identity it would record. */
export type Meta = {
  writes: boolean;
  schema_version?: number;
  identity: { name: string; email: string } | null;
  /** The kanbanr this daemon runs, and the commit it was built from (FTR-036). */
  version?: string;
  commit?: string;
  /** This daemon was built from a modified tree. */
  dirty?: boolean;
  /** The newest kanbanr that wrote the board, and whether it is newer than this daemon. */
  board_writer?: { version: string; commit: string; at: string; dirty?: boolean } | null;
  writer_newer?: boolean;
};

/**
 * Where a verdict was taken, as words that end a line: " via the monitor", or " · origin unknown"
 * for a record written before origin existed (DEF-028). A value this version does not know is
 * shown as it is.
 */
export function viaOrigin(origin?: string | null): string {
  if (!origin) return " · origin unknown";
  return origin === "monitor" || origin === "conversation" || origin === "terminal"
    ? ` · via the ${origin}`
    : ` · via ${origin}`;
}

/**
 * The sentence of an `already_recorded` answer: the verdict repeated the latest one, nothing was
 * recorded, and this says who recorded it first, when and from where (DEF-028). Null for any other
 * answer. The write itself succeeded, so callers treat it as done.
 */
export function alreadySaid(answer: unknown): string | null {
  const said = (answer as { already_recorded?: { sentence?: string } } | null)?.already_recorded;
  return said ? (said.sentence ?? "already recorded") : null;
}

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
/** The token the monitor gave its own page at this start (DEF-024); a write is refused without it.
 * Outside a page (the web checks run this module under Node) there is none to send. */
function token(): string {
  if (typeof document === "undefined") return "";
  return document.querySelector<HTMLMetaElement>('meta[name="kanbanr-token"]')?.content ?? "";
}

async function postWrite(url: string, body: unknown, command: string, method = "POST"): Promise<Response> {
  if (isReadOnly()) {
    throw new Error(`This monitor is read-only. Run \`${command}\` instead.`);
  }
  return fetch(url, {
    method,
    headers: { "content-type": "application/json", "x-kanbanr-token": token() },
    body: JSON.stringify(body),
  });
}

/** The CLI command that makes the same plan change as a planning request (FTR-049). */
function planCommand(r: PlanningRequest): string {
  // Built before the post is sent, so a malformed body still reaches the daemon's own refusal.
  const items = Array.isArray(r?.items) ? r.items.join(" ") : "";
  if (r?.action === "into-sprint") return `kanbanr sprint plan ${r.target} ${items}`;
  if (r?.action === "into-release") return `kanbanr release plan ${r.target} ${items}`;
  return `kanbanr sprint unplan ${items}`;
}

/**
 * Post to a daemon write route (FEAT-185), through the configured API base and the read-only
 * guard (FEAT-178). A daemon without writes has no such route, so a 404 or 405 says which monitor
 * can; any other refusal is the daemon's own message, shown as it gave it.
 */
async function writePost(path: string, body: unknown, verb: string, command: string, method = "POST") {
  const res = await postWrite(apiUrl(`/write${path}`), body, command, method);
  if (!res.ok) {
    throw new Error(
      res.status === 404 || res.status === 405
        ? `This monitor is read-only. Start it with \`kanbanr review --ui\` to ${verb} from here.`
        : `${verb} failed: ${res.status} ${await res.text()}`,
    );
  }
  return res.json();
}
