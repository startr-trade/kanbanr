export type TaskState = "NotStarted" | "InProgress" | "Completed";

/** One entry in a project's activity changelog. */
export interface Activity {
  time: string;
  actor: string;
  message: string;
  /** Made under an agent scope (FEAT-214): the person, by the agent for its item. */
  by_agent?: ByAgent | null;
}

/** Who a change under an agent scope was made by (FEAT-214). */
export interface ByAgent {
  scope: string;
  role: string;
  item: string;
}

/** An agent scope with its derived state (FEAT-214): `GET …/scopes`. Internal. */
export interface AgentScope {
  id: string;
  project: string;
  role: "design" | "build";
  item: string;
  operations: string[];
  branch?: string | null;
  base?: string | null;
  issued_by: string;
  issued_at: string;
  expires_at: string;
  note?: string;
  worktree?: string | null;
  claimed_at?: string | null;
  revoked_at?: string | null;
  revoked_by?: string | null;
  revoked_reason?: string | null;
  state: "issued" | "active" | "expired" | "revoked";
}

/** A build scope's branch ahead of its item's tasks (FEAT-214): `GET …/scopes/drift`. Internal. */
export interface ScopeDrift {
  scope: string;
  item: string;
  branch: string;
  since: string;
  commits: { sha: string; subject: string }[];
}

export interface Task {
  key: string;
  text: string;
  state: TaskState;
  /** The tasks it waits on: `T3`, `TL-002/T3`, `CODE/TL-001/T3` (FTR-060). */
  depends_on?: string[];
  /** The files it writes, repository-relative; a folder ends in `/` (FTR-060). */
  writes?: string[];
  /** The files it only reads (FTR-060). */
  reads?: string[];
  /** Its estimate in agent-hours (FTR-060). */
  estimate_h?: number;
}

export interface TodoList {
  code: string;
  description: string;
  tasks: Task[];
  created_at: string;
}

export interface Feature {
  code: string;
  /** Codes the item had before a rename; a link or a bookmark under one still reaches it (FTR-056). */
  aliases?: string[];
  title: string;
  specification: string;
  status: string;
  /** Required — every feature belongs to a milestone. */
  milestone: string;
  /** Work kind (feature / chore / bug / refactor / docs / recurring …). */
  kind?: string | null;
  /** Priority (low / medium / high …). */
  priority?: string | null;
  /** Optional due date. */
  due?: string | null;
  /** Assignee (the person/agent owning this feature). */
  assignee?: string | null;
  /** Owning team. */
  team?: string | null;
  /** Free-form labels/tags. */
  labels?: string[];
  /** Other feature codes this one is blocked by. */
  depends_on?: string[];
  /** Story points, for projects that estimate in them (FEAT-121). */
  points?: number | null;
  /** The sprint it is planned into (FEAT-119). */
  sprint?: string | null;
  /** The release it is planned into (FEAT-120). */
  release?: string | null;
  /** Persistent todo-lists (an epic can hold many, added across sessions). */
  todo_lists: TodoList[];
  /** Why this item exists, what must be true, and how it is verified (FEAT-047). */
  definition?: FeatureDefinition | null;
  /** Where this feature was imported from, if it was. */
  source?: Source | null;
  /** The external issue this feature is mirrored to, if any. */
  issue?: IssueLink | null;
  created_at: string;
  updated_at: string;
}

/** The project charter: why the project exists and what it commits to (FEAT-046). */
export interface Charter {
  purpose: string;
  vision?: string;
  goals: Goal[];
  non_goals?: string[];
  stakeholders?: Stakeholder[];
  constraints?: string[];
  /** When the charter was first written; items created earlier predate the method. */
  adopted_at?: string;
}

/**
 * Something this project learned, and how much it still believes it (FEAT-055). Confidence decays
 * with age unless reaffirmed, so this list stays short without anyone pruning it.
 */
export interface Lesson {
  id: string;
  lesson: string;
  kind: "practice" | "pitfall" | "decision";
  at: string;
  from_item?: string;
  from_retro?: string;
  evidence?: string;
  tags?: string[];
  goals?: string[];
  confidence: number;
  last_affirmed?: string;
  status: "candidate" | "active" | "retired";
}

/** An outcome the project commits to. Work items link these by id. */
export interface Goal {
  id: string;
  statement: string;
  measure?: string;
  horizon?: string;
}

export interface Stakeholder {
  name: string;
  role?: string;
  interest?: string;
}

/** Import provenance: a record of where an item came from, not a live pointer. */
export interface Source {
  system: string;
  ref: string;
  revision?: string | null;
  url?: string | null;
  imported_at: string;
  key: string;
  /** Set when the source was found to be gone (deleted file, retired tracker). */
  missing_since?: string | null;
}

/** The reasoning and evidence for a work item (FEAT-047/048/049). */
/** One agreement, or one withdrawal of it (FEAT-069). */
export interface ApprovalEvent {
  at: string;
  by: string;
  verdict: "approved" | "ratified" | "withdrawn";
  reason?: string;
  rev?: string;
  /** Where it was taken: monitor, conversation, terminal or delegated (DEF-028). Absent means unknown. */
  origin?: string;
  /** The delegation it was taken under, such as DLG-001, when origin is delegated (FTR-028). */
  delegation?: string;
  /** The person's answer in the conversation it was taken from (FTR-035). */
  heard?: Heard;
  /** `tests` when the approval agreed to a change to tests only (FTR-039); absent otherwise. */
  scope?: string;
}

export interface FeatureDefinition {
  statement?: string;
  /** Charter goal ids this item serves — the link that makes "why" checkable. */
  goals?: string[];
  zachman?: Zachman;
  design_doc?: string;
  /** Board docs this item's design changes (FEAT-180); `docs_updated` checks each changed. */
  affects_docs?: string[];
  /** Why this item changes no doc, recorded when the person confirms it (DEF-075); absent if unset. */
  affects_no_docs?: string;
  requirements?: Requirement[];
  approval?: Approval | null;
  /** Every agreement and withdrawal, oldest first (FEAT-069). Nothing is ever removed. */
  approvals?: ApprovalEvent[];
  /** A recorded reason work started without approval. */
  started_unapproved?: string;
  /** A recorded reason this item is exempt from gap reporting. */
  exempt?: string;
  /** Named sign-offs, given and withdrawn, oldest first (FEAT-114, FEAT-182). */
  signoffs?: Signoff[];
}

/** One sign-off entry: given, or a withdrawal of it (FEAT-182). The latest per name decides. */
export interface Signoff {
  id: string;
  by: string;
  at: string;
  /** The definition content it covered; empty on a withdrawal. */
  rev: string;
  /** A hash per part of the definition it covered (FTR-018). */
  parts?: Record<string, string>;
  /** A hash of the whole spec it covered, when its gate asks for spec sections (FTR-023). */
  spec_rev?: string;
  note?: string;
  doc?: string;
  status?: string;
  /** Absent means given. */
  verdict?: "given" | "withdrawn";
  reason?: string;
  /** Where it was taken (DEF-028). Absent means unknown. */
  origin?: string;
  /** The delegation it was taken under, such as DLG-001, when origin is delegated (FTR-028). */
  delegation?: string;
  /** The person's answer in the conversation it was taken from (FTR-035). */
  heard?: Heard;
  /** The position (`<commit>` or `<commit>+<dirty>`) the item resolved to when it was given (FTR-004). */
  position?: string;
  /** Each repository doc the item lists that existed at `position`, with its blob id (FTR-004). */
  docs_read?: Record<string, string>;
}

/** The six completeness dimensions, one line each. */
export interface Zachman {
  what?: string;
  how?: string;
  where?: string;
  when?: string;
  who?: string;
  why?: string;
}

export interface Approval {
  by: string;
  at: string;
  rev: string;
  /** A hash per part of the definition the approval covered (FTR-018). */
  parts?: Record<string, string>;
  /** Where it was taken (DEF-028). Absent means unknown. */
  origin?: string;
  /** The delegation it was taken under, such as DLG-001, when origin is delegated (FTR-028). */
  delegation?: string;
  /** The person's answer in the conversation it was taken from (FTR-035). */
  heard?: Heard;
}

export interface Requirement {
  id: string;
  kind: "functional" | "nfr";
  text: string;
  iso25010?: string[];
  scenario?: QualityScenario | null;
  tests?: TestRef[];
  /** For a defect: the requirement it violates. */
  violates?: string;
}

export interface QualityScenario {
  stimulus?: string;
  environment?: string;
  response?: string;
  measure?: string;
}

/** One test's last kept duration (FTR-058): whole milliseconds, from the newest capture that timed it. */
export interface TestTiming {
  code: string;
  requirement: string;
  test: string;
  ms: number;
  at: string;
}

/** A duration as the CLI prints it: under 1 s `N ms`, under 60 s `S.ss s`, else `M min S s`. */
export function durationText(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(Math.floor(ms / 10) / 100).toFixed(2)} s`;
  const secs = Math.floor(ms / 1000);
  return `${Math.floor(secs / 60)} min ${secs % 60} s`;
}

/** The last kept duration of one requirement's test, or undefined when none was kept. */
export function lastDuration(
  timings: readonly TestTiming[] | null | undefined,
  requirement: string,
  test: string,
): number | undefined {
  return timings?.find((t) => t.requirement === requirement && t.test === test)?.ms;
}

export interface TestRef {
  name: string;
  kind?: string;
  state: "planned" | "red" | "green";
  checked_rev?: string;
  /** When it can be checked (DEF-043): "pushed", "cut" or a release stage; absent at the item's gates. */
  at?: string;
}

/** The six dimensions in order, with blanks named — derived here exactly as the CLI derives it. */
export function zachmanColumns(z?: Zachman): { column: string; answer: string }[] {
  const source = z ?? {};
  return [
    ["What", source.what],
    ["How", source.how],
    ["Where", source.where],
    ["When", source.when],
    ["Who", source.who],
    ["Why", source.why],
  ].map(([column, answer]) => ({ column: column as string, answer: (answer as string) ?? "" }));
}

/** What an item has not said yet: the same rules doctor applies, for a chip on the board. */
/**
 * An item waiting for agreement (FEAT-067). Whether an approval is *current* or has *lapsed*
 * depends on a content hash computed in core, so the daemon decides and sends the answer — the
 * rule lives in one place rather than being reimplemented here where it could drift.
 */
export interface PendingReview {
  code: string;
  title: string;
  status: string;
  /** `unratified`: finished under a recorded bypass and never agreed to (FEAT-109). */
  /** `release`: agreed, and its next stage is the end — which planned release is it in (FEAT-185)? */
  /** `added`: approved, then text was added that now needs a decision (FTR-018). */
  /** `spec_changed`: the spec a sign-off pinned changed after it was given (FTR-023). */
  approval: "missing" | "lapsed" | "unratified" | "signoff" | "release" | "added" | "spec_changed";
  /** Parts added, changed and removed since the last decision (FTR-018), by part key. */
  changes?: { added: string[]; changed: string[]; removed: string[] };
  /** Finished work listed only because its approval lapsed (DEF-013). */
  finished?: boolean;
  /** Sign-offs the item's next stage is waiting on (FEAT-117). */
  signoffs_needed?: string[];
  /** The item lists no documents, so signing off docs-folded confirms that it changes none (DEF-033). */
  changes_no_documents?: boolean;
  /** The item's branch got a commit after its earlier docs-folded sign-off, which therefore lapsed:
   * why, naming the branch (DEF-046). */
  signoff_lapsed?: string;
  /** Texts that name something the item changed since they last changed, for the person to
   * confirm or correct before docs-folded (FTR-013). Absent when the check was skipped. */
  to_check?: DriftFlag[];
  /** The flagged texts Claude settled, with what it recorded (DEF-039). */
  settled?: SettledText[];
  /** Why the drift check did not run; never to be read as "no drift" (FTR-013). */
  drift_skipped?: string;
  /** Present when the item is ready for release and in none: the planned releases, in file order
   * (FEAT-185). Empty means there is none yet, and only a new release can be chosen. */
  release_choices?: string[];
  /** The definition revision this brief shows; a verdict sent with it is refused if it moved on (FEAT-159). */
  rev?: string;
  started_unapproved?: string;
  definition: FeatureDefinition;
  /** The hash of the spec's current text, when the sign-off asked pins the spec (FTR-023). */
  spec_rev?: string;
  /** The spec that sign-off pins, with what changed since the last one (FTR-023). */
  spec?: SpecBrief;
  /** Claude's overrides on the item that wait for the person to ratify, oldest first (DEF-043). */
  overrides?: OverrideView[];
  /** The change since the last decision touched only tests (FTR-039); absent when false. */
  tests_only?: boolean;
  /** The release the item is planned into (FTR-038); absent when none. */
  release?: string;
  /** The sprint the item is in (FTR-038); absent when none. */
  sprint?: string;
  /** When the item last changed: its `updated_at` (FTR-038). */
  changed_at?: string;
}

/** One decision group, counted: a tab on the Review page (FTR-038, `GET /review/summary`). */
export interface GroupCount {
  /** approve, ratify, design-review, docs-folded, release-signoff, signoff-<name>, which-release,
   * manual, delegation, delegated or adr. */
  key: string;
  label: string;
  /** The rows in the group. */
  count: number;
  /** The group button's verb: "Approve", "Ratify", "Sign off", "Plan into", "Confirm", "Grant",
   * "Keep" or "Accept". */
  action: string;
  /** The rows that can be decided now. */
  ready: number;
  /** The rows with an open prepared record. */
  prepared: number;
}

/** One row of a group: one decision waiting on one thing (FTR-038). */
export interface ReviewRow {
  /** Unique within the group: the item's code, `<CODE>/<R-n>/<test>` for a manual check, the
   * delegation's id, the delegated decision's id, or the ADR's id. */
  key: string;
  code: string;
  title: string;
  /** What is waiting, in a few words. */
  what: string;
  /** What a yes records. */
  records: string;
  release: string;
  sprint: string;
  changed_at: string;
  /** Empty when the row can be decided; else one sentence naming who provides what is missing. */
  blocked: string;
  pills: string[];
  /** The open prepared record's id; empty when none. */
  prepared: string;
  /** That record's batch; empty when none. */
  batch: string;
  /** What the group decision sends back. */
  rev: string;
  spec_rev: string;
  signoff: string;
  requirement: string;
  test: string;
}

/** One group's rows, in the page's default order (FTR-038, `GET /review/groups/{key}`). */
export interface GroupSlice {
  key: string;
  label: string;
  action: string;
  rows: ReviewRow[];
  /** The planned releases, in file order: for `which-release` only. */
  choices: string[];
  /** The releases whose items the manual group lists first, newest first (FTR-057): for `manual` only. */
  holding?: string[];
}

/** One row's full evidence, read when the row is opened (FTR-038, `GET /review/groups/{key}/rows/{row}`). */
export interface RowEvidence {
  entry: PendingReview | null;
  manual: ManualWaiting | null;
  adr: Adr | null;
  delegated: DelegatedDecision | null;
  delegation: DelegationView | null;
  prepared: PreparedView | null;
}

/** One selected row of a group decision, with the pins the slice showed (FTR-038). */
export interface DecisionItem {
  key: string;
  code: string;
  rev: string;
  spec_rev: string;
  signoff: string;
  requirement: string;
  test: string;
  prepared: string;
}

/** What `POST /review/decide` takes: one decision over the rows the person selected (FTR-038). */
export interface GroupDecision {
  group: string;
  /** The group's action; `undo` for the delegated group's second button; `reject` to turn down
   * the selected prepared records. */
  action: string;
  by: string;
  /** The release, for `which-release`; empty otherwise. */
  target: string;
  reason: string;
  items: DecisionItem[];
}

/** What a group decision answers: each row, and the verdict it made or why it was refused. */
export interface GroupDecisionAnswer {
  group: string;
  action: string;
  results: { key: string; made?: string; error?: string }[];
}

/** One override Claude made past a gate, as the Review page shows it (DEF-043). */
export interface OverrideView {
  at: string;
  from: string;
  to: string;
  reason: string;
  origin: string;
}

export interface SpecBrief {
  signoff: string;
  text: string;
  /** False when the earlier sign-off's text is missing from the board. */
  previous_kept: boolean;
  signed_by: string;
  signed_at: string;
  diff: { op: "same" | "added" | "removed"; text: string }[];
}

/** One thing an item is missing, as the daemon's readiness engine reports it (FEAT-112). The rules
 * live in the engine only; the monitor used to carry its own copy, and it drifted. */
export interface Gap {
  check: string;
  requirement?: string;
  level: "warning" | "error";
  label: string;
  message: string;
}

/** One fenced `mermaid` block of an item's spec or of a doc it names (FEAT-200). */
export interface DiagramBlock {
  /** `"FEAT-012 spec"` or `"docs/<key>"`. */
  file: string;
  /** The doc key, for a link to it; null for a spec. */
  doc: string | null;
  /** 1-based index among the file's mermaid blocks. */
  block: number;
  /** The file line of the block's first source line. */
  line: number;
  source: string;
}

/** What the diagram lint found wrong, by rule name. */
export type LintRule =
  | "empty_block"
  | "unknown_type"
  | "semicolon_in_text"
  | "label_punctuation"
  | "unbalanced_label"
  | "end_as_id"
  | "unbalanced_block"
  | "unknown_statement";

/** One problem the diagram lint found, located in its file (FEAT-200). */
export interface DiagramFinding {
  file: string;
  block: number;
  line: number;
  rule: LintRule;
  message: string;
}

/** Every diagram of an item's spec and its `affects_docs`, with the lint's findings (FEAT-200). */
export interface ItemDiagrams {
  code: string;
  blocks: DiagramBlock[];
  findings: DiagramFinding[];
}

/** Per live item, keyed by code: what its card counts, and the next stage with how much it lacks. */
export type Readiness = Record<
  string,
  { gaps: Gap[]; next: { status: string; missing: number; signoffs: string[]; tasks_open: number; ends: boolean; immediate?: boolean; for_you?: number }[] }
>;

/** A mirrored external issue (e.g. a GitHub issue kept in step with this feature). */
export interface IssueLink {
  system: string;
  repo: string;
  number: number;
  url: string;
  synced_hash?: string | null;
  synced_at?: string | null;
}

/** True when a todo-list has tasks and all are completed. */
export function listFullyCompleted(list: TodoList): boolean {
  return list.tasks.length > 0 && list.tasks.every((t) => t.state === "Completed");
}

/** Todo-lists newest-first (reverse chronological). */
export function listsNewestFirst(lists: TodoList[]): TodoList[] {
  return [...lists].sort((a, b) =>
    b.created_at.localeCompare(a.created_at) || b.code.localeCompare(a.code)
  );
}

/**
 * Feature items newest-first (reverse chronological). Sorted by creation time descending so the
 * order is stable across edits; ties break on last-update then code. (Switch the first comparison
 * to `updated_at` if you'd rather surface most-recently-active items first.)
 */
export function featuresNewestFirst<T extends { code: string; created_at?: string; updated_at?: string }>(
  features: T[],
): T[] {
  return [...features].sort(
    (a, b) =>
      (b.created_at ?? "").localeCompare(a.created_at ?? "") ||
      (b.updated_at ?? "").localeCompare(a.updated_at ?? "") ||
      b.code.localeCompare(a.code)
  );
}

/** Task progress across all of a feature's todo-lists. */
export function featureProgress(feature: Feature): { done: number; total: number } {
  let done = 0;
  let total = 0;
  for (const l of feature.todo_lists) {
    for (const t of l.tasks) {
      total++;
      if (t.state === "Completed") done++;
    }
  }
  return { done, total };
}

export interface Milestone {
  code: string;
  name: string;
  description: string;
  depends_on: string[];
}

/** A project's codes (FEAT-217): kind (lowercase) to prefix, and the prefix of any other kind. */
export interface CodesConfig {
  default?: string;
  kinds?: Record<string, string>;
}

export interface ProjectConfig {
  name: string;
  description: string;
  statuses: string[];
  default_state: string;
  transitions: Record<string, string[]>;
  displayed_states: string[];
  /** Statuses flagged as functionally inert (no-op); always non-displayed. */
  no_op_states: string[];
  /** Explicit terminal (end) states — a feature here is "done" (FEAT-039). */
  terminal_states?: string[];
  /** Entry criteria per status (FEAT-113). Absent: kanbanr's built-in rule applies. */
  gates?: Record<string, Gate>;
  /** Which prefix an item's code gets from its kind (FEAT-217). Absent: FEAT for every item. */
  codes?: CodesConfig;
  /** A shorter path through the stages for some kinds, keyed by kind (FTR-019). Absent: none. */
  paths?: Record<string, KindPath>;
  /** Days or story points (FEAT-121). */
  estimate_unit?: "days" | "points";
  /** Whether the project uses sprints and releases at all (FEAT-121). */
  cadence?: {
    sprints?: boolean;
    releases?: boolean;
    sprint_length_days?: number;
    release?: string;
    /** The stages a release moves through; the last is the cut (FEAT-183). */
    release_stages?: string[];
    /** The gates a stage needing `gate_passed` requires a passed result of (FTR-002). */
    commit_gates?: string[];
    integration_branch?: string;
    main_branch?: string;
    /** Branching follows the git model the cadence implies (FEAT-189). */
    git_model?: boolean;
    /** Each release stage's conditions and entry actions, keyed by stage name (FEAT-212). */
    release_gates?: Record<
      string,
      {
        purpose?: string;
        requires?: string[];
        after?: string[];
        on_enter?: string[];
        /** Gates the engineering baseline requires here, each with the entry that promised it (FTR-033). */
        gates?: { gate: string; from: string }[];
      }
    >;
    /** What a release's `prepare` action commits onto `release/<V>` (FEAT-212). */
    release_prepare?: {
      version_files?: { path: string; match?: string }[];
      changelog?: string;
      changelog_copies?: string[];
      notes?: string;
      notes_listings?: { path: string; after: string; entry: string }[];
    };
  };
}

/** A timebox (FEAT-119). */
export interface Sprint {
  code: string;
  name?: string;
  goal?: string;
  start: string;
  end: string;
  capacity?: number | null;
  state: "planned" | "active" | "closed";
  carried?: { code: string; to: string }[];
}

/** A sprint with what it holds and how it burned down, derived by the daemon. */
export interface SprintReport extends Sprint {
  unit: string;
  items: string[];
  committed: number;
  done: number;
  unestimated: string[];
  days_left: number;
  burndown: { date: string; remaining: number }[];
}

/** A release, planned up front and cut from finished work (FEAT-120). */
/** What `release plan` answers: the release, and the planned items that are in no running sprint (DEF-029). */
export type PlannedRelease = Release & { not_in_sprint?: { code: string; sprint: string }[] };

export interface Release {
  version: string;
  name?: string;
  target?: string;
  state: "planned" | "shipped";
  shipped_at?: string;
  shipped?: string[];
  notes_doc?: string;
  carried?: { code: string; to: string; why?: string }[];
  /** Planned into it or shipped in it, as the daemon reports (FEAT-138). */
  items?: string[];
  /** How many of `items` are finished — by the daemon's rule, the same as the sprint's. */
  finished?: number;
  /** Where it is now, from the project's release stages; absent until first promoted (FEAT-183). */
  stage?: string;
  /** Every stage change, oldest first. A move back carries its reason. */
  stages?: {
    stage: string;
    by: string;
    at: string;
    reason?: string;
    item?: string;
    origin?: string;
    /** The requirements accepted shipping untested at the cut, as CODE/R-n (DEF-043). */
    accepted_untested?: string[];
    /** The stage this change undid (`kanbanr release undo`, DEF-034); absent on every other change. */
    undoes?: string;
  }[];
  /** Who cut the release and from where (DEF-028). Absent means unknown. */
  shipped_by?: string;
  shipped_origin?: string;
  /** The code branch, made on entering the first stage. */
  branch?: string;
  /** The cut release this patches (FEAT-184); absent for an ordinary release. */
  parent?: string;
  /** When the stage that tags was entered (FTR-026); a tagged release still `planned` is waiting on its later stages. */
  tagged_at?: string;
  /** The commit the tag names. */
  tag_commit?: string;
  /** The patch release that replaced it after its tag's CI run failed; its tag stays in git. */
  superseded_by?: string;
  /** For a patch: what its parent shipped, which the patch keeps (FEAT-184). */
  included?: string[];
}

/** The Releases page's tab a release falls in (FTR-041): the route's words; the page labels `past` Released. */
export type Scope = "current" | "planned" | "past";

/** The Releases page's counts per tab and the tab it opens on: `GET /projects/{p}/releases/summary` (FTR-041). */
export interface ReleasesSummary {
  current: number;
  planned: number;
  past: number;
  /** `current` when any release is in flight, else `planned` when any is planned, else `past`. */
  open: string;
}

/** One row of a release's item table: `GET /projects/{p}/releases/{v}/items` (FTR-041). */
export interface ReleaseItemRow {
  code: string;
  title: string;
  kind: string;
  status: string;
  sprint: string;
  /** Tests named across the definition's requirements that are green. */
  tests_green: number;
  /** Every test named across the definition's requirements. */
  tests_named: number;
  /** `approved` (ratified counts), `lapsed`, `added` or `none`. */
  approval: string;
  /** `signed`, `lapsed`, or empty when never given. */
  design_review: string;
  /** `signed`, `lapsed`, or empty when never given. */
  docs_folded: string;
  /** The release a carried item moved to, or `unplanned`; empty for an item not carried. */
  carried_to: string;
  /** An end status, or a stage marked `done` (FEAT-137). */
  finished: boolean;
}

/** A release's notes as they are on its branch `release/<V>` (FEAT-216). */
export interface ReleaseNotes {
  version: string;
  branch: string;
  /** The notes file in the code repository. */
  path: string;
  text: string;
  /** The release branch's tip they were read at, seven characters. */
  at: string;
  /** The release has a branch and has not shipped. */
  editable: boolean;
  /** Why not, when not editable. */
  why_not?: string;
  /** Items planned into the release whose code the notes do not mention (DEF-031). */
  unmentioned?: string[];
  /** The changelog file, when one is configured (DEF-044). */
  changelog_path?: string;
  /** This version's section in it, at the same tip; absent when empty. */
  changelog_section?: string;
}

/** A check a gate names, the Zachman check narrowed to some columns, the spec's named sections
 * (FEAT-179), or the coverage check narrowed to some of the project's rules (FEAT-197). */
export type Condition =
  | string
  | { zachman: string[] }
  | { spec_sections: string[] }
  | { docs_cover: string[] };

/** What entering a status asks for (FEAT-113, FEAT-114). */
/** What a cut would do with one planned item (FEAT-183, FEAT-185). */
export interface CutPreview {
  code: string;
  ships: boolean;
  to?: string;
  why?: string;
}

/** A gate's verdict recorded for one commit (FTR-002). */
export interface GateResult {
  name: string;
  commit: string;
  passed: boolean;
  by: string;
  at: string;
  source: "person" | "local";
  url?: string;
  note?: string;
  /** Where it was recorded (FTR-034): monitor, conversation or terminal; absent on older results. */
  origin?: string;
}

/** The answer to recording a gate result: the version it shipped, when it was the pass of the tag
 * workflow a tagged release waited on (DEF-049). */
export type GateResultResponse = GateResult & { settled?: string };

/** Where a piece of a prepared record's evidence came from (FTR-034): an http(s) link or a board
 * reference (`doc:<path>`, `item:<CODE>`, `commit:<sha>`), or no link at all. */
export interface PreparedSource {
  label?: string;
  url?: string;
}

/** What a prepared record's decision is about; which keys a kind uses is the server's. */
export interface PreparedSubject {
  item?: string;
  signoff?: string;
  requirement?: string;
  test?: string;
  release?: string;
  stage?: string;
  gate?: string;
  commit?: string;
  target?: string;
}

/** The evidence for one decision, prepared by Claude for the person to confirm (FTR-034). */
export interface Prepared {
  id?: string;
  /** signoff, promotion, gate-result, ratification, … */
  kind?: string;
  subject: PreparedSubject;
  /** What the verdict will carry. */
  fields?: Record<string, unknown>;
  sources?: PreparedSource[];
  by?: string;
  origin?: string;
  at?: string;
  rev?: string;
  /** The batch it was prepared in (FTR-037): a walkthrough run's checks, confirmed with one click. */
  batch?: string;
}

/** One field the person changed before confirming. */
export interface FieldChange {
  field: string;
  prepared: unknown;
  confirmed: unknown;
}

/** The person's answer to a prepared record. */
export interface Resolution {
  id?: string;
  verdict: "confirmed" | "rejected";
  by?: string;
  origin?: string;
  at?: string;
  changed?: FieldChange[];
  reason?: string;
  made?: string;
}

export type PreparedState = "open" | "confirmed" | "rejected" | "superseded" | "stale" | "unknown_kind";

/** A prepared record as the monitor lists it: where it stands, the answer, the equivalent command. */
export interface PreparedView extends Prepared {
  state: PreparedState;
  resolution: Resolution | null;
  /** Why it no longer waits, when it doesn't. */
  stale_why: string;
  /** The page its decision is taken on. */
  page: string;
  /** The verdict command a person would run in their own terminal instead. */
  command: string;
  /** The release the record is about (DEF-095), derived: a promotion's own; for a gate result,
   * the release its commit belongs to. Absent when none. */
  release?: string;
  /** The release stages that ask for a gate result's gate (DEF-095). Absent when none does. */
  required_by?: string[];
}

/** A kept manual-test result: who checked, when, from where, and any note (DEF-040). */
export interface ManualResult {
  code: string;
  requirement: string;
  test: string;
  /** green | red */
  state: string;
  by: string;
  at: string;
  /** monitor | terminal */
  origin?: string;
  checked_rev?: string;
  note?: string;
  prepared?: string;
}

/** A manual test waiting for a person, with its requirement and the check to make (DEF-040). */
export interface ManualWaiting {
  code: string;
  title: string;
  status: string;
  requirement: string;
  requirement_text: string;
  test: string;
  /** planned | red */
  state: string;
  checked_rev: string;
  last: ManualResult | null;
  /** The build stage the check opens at (DEF-076); absent or empty when the check is due. */
  opens_at?: string;
  /** The item's release (FTR-057); absent when it has none. */
  release?: string;
  /** The button this check holds, e.g. "Promote v0.8.0 to main" (FTR-057); absent when none. */
  holds?: string;
}

/** One condition of a release stage a promotion enters, and whether it holds (FEAT-212). */
export interface PlanCondition {
  check:
    | "items_finished"
    | "prepared"
    | "main_at_release"
    | "pushed"
    | "gate_passed"
    | "tag_passed"
    | "items_merged"
    | "stage_passed"
    | "merged_items_ship"
    | "later_tests_green"
    | "untested_accepted";
  stage: string;
  holds: boolean;
  detail: string;
  fix?: string;
  /** The requirements it names, as CODE/R-n (DEF-043): for untested_accepted, exactly what the
   * person accepts shipping unverified. */
  refs?: string[];
  /** kanbanr asks it of this stage beyond the stage's own requires (DEF-077): shown "(implied)". */
  implied?: boolean;
}

/** One step a promotion takes (FEAT-212). */
export interface PlanStep {
  action: "branch" | "prepare" | "move_main" | "tag";
  stage: string;
  does: string;
  noop: boolean;
  blocked?: string;
}

/** The newest result recorded for a release's tag (FTR-026). */
export interface TagResult {
  commit: string;
  gate: string;
  state: "passed" | "failed" | "none";
  result?: GateResult;
}

/** What promoting a release would do, with nothing changed (FEAT-212). */
export interface ReleasePlan {
  version: string;
  from: string;
  to: string;
  direction: string;
  /** This promotion enters the stage that tags. */
  cut: boolean;
  /** This promotion enters a stage after the cut: a confirmation by the user (FTR-026). */
  confirm: boolean;
  /** This promotion enters the last stage and ships the release (FTR-026). */
  ships: boolean;
  /** The release waits at its last stage for the tag workflow's result and ships on a pass (DEF-049). */
  awaiting?: boolean;
  /** The tag's recorded CI result, while the release is tagged and not shipped (FTR-026). */
  tag?: TagResult;
  /** The merge that sent the release back and the steps from there, until it is cut (DEF-031). */
  returned?: { item: string; by: string; at: string; from: string; steps: string[] };
  /** What the one button says: "Promote v0.2.0 to main", "Cut v0.2.1". */
  button: string;
  conditions: PlanCondition[];
  steps: PlanStep[];
  preview?: CutPreview[];
  tag_commit?: { sha: string; subject: string };
  release_tip: string;
  missing: string[];
  ready: boolean;
  needs_reason: boolean;
  origin?: { tracking: string; fetched: boolean; fetch_error?: string };
  after?: string;
  /** Whether the daemon had the code repository to read. */
  repo: boolean;
  /** Nothing is planned into the release (FEAT-212 R-17). */
  empty: boolean;
  /** Worth knowing, not stopping: e.g. the notes carry an earlier date (DEF-001). */
  warnings?: string[];
  /** Entering the stage it is at again would redo something (DEF-057): only on the default plan. */
  again?: AgainOffer;
}

/** Entering a release's current stage again: what the second button says and does (DEF-057). */
export interface AgainOffer {
  stage: string;
  button: string;
  does: string[];
  ready: boolean;
  missing: string[];
}

/** The stages an item of one kind passes, what its first move asks, and the method parts left out. */
export interface KindPath {
  stages: string[];
  requires?: Condition[];
  method_off?: string[];
}

export interface Gate {
  purpose?: string;
  requires?: Condition[];
  warns?: Condition[];
  signoffs?: string[];
  enforce?: "block" | "warn";
  kinds?: string[];
  on_enter?: string[];
  /** The stage roles this status holds: design, build, integrate, release (FEAT-188). */
  roles?: string[];
  /** The docs every item entering this stage must list in affects_docs (FEAT-197). */
  affects_docs?: string[];
}

/** Readable text for a gate condition. */
export function conditionText(c: Condition): string {
  if (typeof c === "string") return c.replace(/_/g, " ");
  if ("spec_sections" in c) return `spec sections ${c.spec_sections.join(", ")}`;
  if ("zachman" in c) return `zachman ${c.zachman.join("/")}`;
  if ("docs_cover" in c) return `docs cover ${c.docs_cover.join(", ")}`;
  // A condition a newer kanbanr writes: shown as it is rather than breaking the page (FEAT-199).
  return JSON.stringify(c);
}

export interface Project {
  id: string;
  config: ProjectConfig;
  features: Feature[];
  milestones: Milestone[];
}

export interface ProjectSummary {
  id: string;
  name: string;
  description: string;
  displayed_states: string[];
  /** Non-displayed statuses (includes no-op), still linkable to their status pages. */
  other_states: string[];
  no_op_states: string[];
  /** Feature count per status (all statuses). */
  counts: Record<string, number>;
  total_features: number;
  has_docs: boolean;
  /** The board this project is read from; present when the server runs over boards (FTR-040). */
  board?: string;
}

/** One board a read-only server shows, as `GET /api/boards` lists it (FTR-040). */
export interface BoardInfo {
  name: string;
  projects: string[];
  head?: string;
  head_at?: string;
  writer?: { version: string; commit: string; at: string; dirty: boolean };
  history_unreadable?: string;
  shadowed?: string[];
}

export interface DocFile {
  path: string;
  title: string;
}

export interface DocFolder {
  path: string;
  name: string;
  description: string;
  folders: DocFolder[];
  docs: DocFile[];
}

/** The states displayed for a project (falls back to all statuses). */
export function displayedStates(config: { displayed_states?: string[]; statuses?: string[] }): string[] {
  return config.displayed_states?.length ? config.displayed_states : (config.statuses ?? []);
}

// ---- portfolio / program hierarchy (FEAT-030) ----------------------------------------------

/** A program's project membership within the portfolio index. */
export interface ProgramView {
  id: string;
  name: string;
  description: string;
  projects: string[];
  /** True when this is the synthesized implicit default program. */
  implicit: boolean;
}

export interface PortfolioView {
  name: string;
  description: string;
  programs: ProgramView[];
}

/** Task counts + a derived done/total percentage. */
export interface Counts {
  features: number;
  tasks_total: number;
  tasks_done: number;
  percent: number;
}

export interface MilestoneRollup {
  code: string;
  name: string;
  counts: Counts;
}

export interface ProjectRollup {
  id: string;
  name: string;
  counts: Counts;
  milestones: MilestoneRollup[];
}

export interface ProgramRollup {
  id: string;
  name: string;
  counts: Counts;
  projects: ProjectRollup[];
}

export interface RollupReport {
  portfolio: string;
  counts: Counts;
  programs: ProgramRollup[];
}

export type Disposition = "done" | "in-progress" | "not-started";

export interface BoardCard {
  project: string;
  code: string;
  title: string;
  status: string;
  milestone: string;
  assignee?: string | null;
  team?: string | null;
  tasks_done: number;
  tasks_total: number;
  disposition: Disposition;
}

export interface BoardLane {
  disposition: Disposition;
  cards: BoardCard[];
}

export interface BoardReport {
  lanes: BoardLane[];
}

/** An architecture decision as the daemon lists it (FEAT-057, FEAT-153). */
export type Adr = {
  id: string;
  status: string;
  title?: string;
  date?: string;
  decided?: string;
  /** Where the verdict was taken (DEF-028). Absent means unknown. */
  decided_origin?: string;
  reason?: string;
  deciders?: string[];
  affects?: string[];
  driven_by?: string[];
  body?: string;
  path?: string;
  /** Sections still unanswered; a decision with any cannot be accepted. */
  missing?: string[];
};

/** Which saved process a project's workflow came from (FEAT-169). */
export interface ProcessSource {
  name: string;
  library: "board" | "personal" | "builtin";
  version: number;
  rev: string;
}

/** How a project stands against its saved process (FEAT-170). */
export interface ProcessStatus {
  source: ProcessSource | null;
  drift: { edited: boolean; newer: number | null; gone: boolean } | null;
  messages: string[];
}

/**
 * The git model the cadence implies (FEAT-189, FEAT-194), as `GET …/git/model` answers it: the
 * summary in words, the model itself, and, from a daemon started in the code repository, the
 * branches it read. Internal: not in the published API.
 */
export interface GitModelView {
  topology: "trunk" | "sprints" | "releases" | "sprints_and_releases";
  /** Where finished items are merged: `sprint/<SP>` stands for the running sprint's branch. */
  line: string;
  /** Where an item branch is made. */
  branch_from: string;
  production_at: "integrate_merge" | "sprint_close" | "release_cut";
  /** The merges kanbanr makes; only when `automated` is on. */
  kanbanr_merges: string[];
  automated: boolean;
  /** Who pushes the code: always "you". */
  pushes: string;
  model: {
    main: string;
    integration_branch?: string;
    line: string | null;
    item_patterns: string[];
    protected: string[];
  };
  /** The branches read from the code repository; absent from a preview. */
  facts?: { main: string } | null;
}

/** One problem `GET …/git/doctor` finds between the board and the branches (FEAT-191). Internal. */
export interface GitDoctorIssue {
  severity: "error" | "warning";
  project: string;
  code: string | null;
  message: string;
}

/** Every guard rail a project has, in one read (FEAT-213): `GET …/guards`. Internal. */
export interface Guards {
  method: { part: string; on: boolean; status?: string; means: string }[];
  commits: {
    item_branch: boolean;
    branch_pattern: string;
    trailer: string;
    escape: string;
    protected: string[];
    /** Null where it cannot be told: the daemon does not read the code repository's hooks. */
    hooks_installed: boolean | null;
  };
  diagrams: { lint_on_write: boolean; strict: boolean };
  coverage: { id: string; subjects: number; uncovered: number; problem?: string }[];
  push_policy: { policy: string; source: "environment" | "board" | "default" };
  code_pushes: string;
  /** Agent scopes (FEAT-214): the roles, the scopes in force, and the pre-push hook. */
  agents?: {
    roles: { role: string; operations: string[] }[];
    scopes: AgentScope[];
    /** Null where it cannot be told: the daemon does not read the code repository's hooks. */
    push_hook_installed: boolean | null;
  };
}

/** A name a change put in the code, of a kind that decides how a text must hold it (FTR-013). */
export interface DriftName {
  kind: "flag" | "command" | "env" | "route" | "config_key" | "name" | "file";
  text: string;
}

/** A linked text that names something an item changed after the text last changed (FTR-013).
 * It says which text names what, never that the text is wrong. */
export interface DriftFlag {
  target: { kind: "doc" | "adr" | "item"; id: string; title: string; path?: string };
  via: string[];
  hits: { name: DriftName; lines: number[] }[];
  commits: { sha: string; at: string; item: string; names: DriftName[] }[];
  target_changed_at: string;
  /** Edits to the text since the first change that opened the flag, oldest first. The flag stays
   * open after an edit; only Claude's check settles it (DEF-050). `sha` is empty for an edit not
   * yet committed. */
  edits?: { sha: string; at: string; subject: string }[];
  /** Unified diff from the text as it stood when the flag opened to the text now. */
  diff?: string;
  diff_truncated?: boolean;
}

/** How a flagged text was settled at the sign-off (FTR-013). */
export interface DriftChecked {
  /** `doc:<path>`, `adr:<id>` or `item:<CODE>`. */
  target: string;
  outcome: "confirmed" | "corrected";
  correction?: string;
  /** Who settled it, where, when, and against which commit (DEF-039). */
  by?: string;
  origin?: string;
  at?: string;
  rev?: string;
  text_changed_at?: string;
  /** Why the text is still right, for `confirmed`. */
  why?: string;
  /** The `text-check` prepared record it came from. */
  prepared?: string;
  /** The shas of the edits made while the text was flagged, which the check rested on (DEF-050). */
  edits?: string[];
}

/** A flagged text and the check Claude recorded for it (DEF-039). */
export interface SettledText {
  flag: DriftFlag;
  check: DriftChecked;
}

/** Where a project's decisions are taken (FTR-028). */
export type DecisionMode = "monitor" | "delegated" | "conversation";

/** Who did something on the decisions record, when and from where; a grant pins its `rev`. */
export interface DecisionStamp {
  by: string;
  at: string;
  origin: string;
  rev?: string;
  reason?: string;
}

/** One entry of the decision mode's history. */
export interface ModeChange {
  mode: DecisionMode;
  by: string;
  at: string;
  origin: string;
  reason?: string;
}

/** The person's answer a conversation verdict was recorded from (FTR-035). */
export interface Heard {
  question: string;
  answer: string;
  pass: string;
  heard_at: string;
  tool_use_id?: string;
  session_id?: string;
}

/** A delegation: the verdicts Claude may take, on which items, until when (FTR-028). */
export interface Delegation {
  id: string;
  /** approve, ratify or signoff:<name>. */
  verdicts: string[];
  steps?: string[];
  items?: string[];
  kinds?: string[];
  milestone?: string;
  all?: boolean;
  until: string;
  note?: string;
  proposed: DecisionStamp;
  granted?: DecisionStamp;
  revoked?: DecisionStamp;
}

/** A delegation with where it stands now. */
export interface DelegationView extends Delegation {
  state: "proposed" | "granted" | "ended" | "revoked";
  in_force: boolean;
  /** What a grant is pinned to; sent back with it. */
  rev: string;
  /** Granted, but not through kanbanr: Claude's verdicts under it are refused. */
  not_granted_through_kanbanr?: boolean;
  /** Granted, but the board's history cannot be read, so the grant cannot be checked: why,
   * naming the repository (DEF-080). Claude's verdicts under it are refused. */
  grant_unverifiable?: string;
  /** The Claude Code allow rules the delegation's verdicts need to run in auto mode (DEF-060). */
  permission_rules?: string[];
  /** The monitor knows the repository and the mode is delegated: the next field is meaningful. */
  permission_rules_checked?: boolean;
  /** The rules none of the project's or the user's Claude settings allows. */
  permission_rules_missing?: string[];
}

/** A decision Claude took under a delegation, waiting for the person to keep or undo it. */
export interface DelegatedDecision {
  /** `<CODE>/<approve|ratify|signoff>/<name or ->/<record at>`. */
  decision: string;
  code: string;
  title: string;
  verdict: string;
  signoff?: string;
  delegation: string;
  at: string;
  status: string;
  undo_route: string;
}

/** Something waiting for a decision, and where. */
export interface Waiting {
  code: string;
  what: string;
  place: "monitor" | "conversation" | "delegated";
  delegation?: string;
  url?: string;
}

/** `GET /projects/{p}/decisions` (FTR-028). */
export interface DecisionsView {
  mode: DecisionMode;
  modes: ModeChange[];
  delegations: DelegationView[];
  to_review: DelegatedDecision[];
  waiting: Waiting[];
}

/** The answer to a mode change: what waits where now. */
export interface ModeChanged {
  mode: DecisionMode;
  already_recorded?: boolean;
  waiting: Waiting[];
}

// ---- the monitor's compact index and live slices (DEF-081) ----------------------------------

/** The index's cadence: which planning the project uses. */
export interface IndexCadence {
  sprints: boolean;
  releases: boolean;
  git_model: boolean;
}

/** The configuration the shell, links and cards need; the whole of it is `GET …/config`. */
export interface IndexConfig {
  name: string;
  description: string;
  statuses?: string[];
  displayed_states?: string[];
  no_op_states?: string[];
  terminal_states?: string[];
  cadence: IndexCadence;
  codes: CodesConfig;
}

/** One item as a card: its fields, no bodies. `done`/`total` count tasks across its todo-lists. */
export interface IndexItem {
  code: string;
  /** Earlier codes of the item, absent when it was never renamed (FTR-056). */
  aliases?: string[];
  title: string;
  kind?: string;
  status: string;
  milestone: string;
  sprint?: string;
  release?: string;
  priority?: string;
  assignee?: string;
  team?: string;
  labels?: string[];
  depends_on?: string[];
  goals?: string[];
  requirements?: string[];
  done: number;
  total: number;
  /** When it was created, to the whole second: the board's newest-first order. */
  created_at: string;
}

export interface IndexMilestone {
  code: string;
  name: string;
  description?: string;
  depends_on?: string[];
}

export interface IndexSprint {
  code: string;
  name: string;
  start: string;
  end: string;
  state: "planned" | "active" | "closed";
}

export interface IndexRelease {
  version: string;
  name?: string;
  state: string;
  stage?: string;
  target?: string;
}

export interface IndexAdr {
  id: string;
  path: string;
}

export interface IndexGoal {
  id: string;
}

/** `GET /projects/{p}/index`: what the shell, the link index and the item lists read. */
export interface CompactIndex {
  project: string;
  config: IndexConfig;
  items: IndexItem[];
  milestones: IndexMilestone[];
  sprints?: IndexSprint[];
  releases?: IndexRelease[];
  adrs?: IndexAdr[];
  docs: string[];
  goals: IndexGoal[];
}

/** The `slices` SSE event: what changed on a project's board, by tag. */
export interface Slices {
  project: string;
  gen: number;
  tags: string[];
}

/** The revisions and time a background drift result was computed at (FTR-045). */
export interface DriftAsOf {
  /** The board's HEAD commit, read before the run. */
  board_rev: string;
  /** The code repository's HEAD commit, read before the run. */
  code_rev: string;
  /** When the run finished, RFC 3339. */
  finished_at: string;
  /** How long the run took, in milliseconds. */
  took_ms: number;
}

/** The board-wide drift report a background run produced (FTR-013, FTR-045). */
export interface DriftReport {
  generated_at: string;
  since: string;
  flags: DriftFlag[];
}

/** The latest background drift result for a project, from the daemon's memory (FTR-045):
 * `GET /projects/{p}/drift/latest[?flags=false]`. */
export interface DriftLatest {
  state: "fresh" | "stale" | "computing" | "unavailable" | "failed";
  /** Absent before the first result. */
  as_of?: DriftAsOf;
  /** A run is going beside the last result. */
  recomputing?: boolean;
  /** The flags in the last result; 0 when there is none. */
  count: number;
  /** Absent with `?flags=false` or before the first result. */
  report?: DriftReport;
  /** Why drift is unavailable, or the last run's error. */
  reason?: string;
  /** When the last run failed, RFC 3339, even beside a result. */
  failed_at?: string;
}

// ---- The Overview, Needs you and the Items list (FTR-048) ----

/** Where an attention line sits in the Needs-you drawer. */
export type AttentionSection = "decisions" | "releases" | "health" | "records";

/** One thing waiting: a review group, a release at a stage, or a health or records line. */
export interface AttentionLine {
  /** A review group's key (as FTR-038 names them), or release, red-tests, doctor-errors, retro-due, git-doctor, read-error. */
  kind: string;
  section: AttentionSection;
  label: string;
  count: number;
  /** The monitor path that shows it; absent when no page does. */
  to?: string;
  /** The "Ask Claude to …" sentence, when there is no page. */
  ask?: string;
  tone: string;
  /** Counted in `total`: the person's own decision. */
  yours: boolean;
}

/** What needs the person: `GET /projects/{p}/attention`. */
export interface Attention {
  /** The sum of the `yours` lines' counts: the badge's number. */
  total: number;
  /** Absent when nothing is waiting. */
  lines?: AttentionLine[];
}

export interface SprintNow {
  code: string;
  name?: string;
  goal?: string;
  start: string;
  end: string;
  days_left: number;
  committed: number;
  done: number;
}

export interface StepFact {
  key: string;
  label: string;
  /** done | current | next */
  state: string;
  at?: string;
  by?: string;
}

export interface ReleaseNow {
  version: string;
  name?: string;
  stage?: string;
  waits_on_tag: boolean;
  steps: StepFact[];
  items: number;
  finished: number;
}

export interface MilestoneNow {
  code: string;
  name: string;
  done: number;
  total: number;
}

export interface Now {
  sprint?: SprintNow;
  releases?: ReleaseNow[];
  milestones?: MilestoneNow[];
}

export interface ItemCounts {
  total: number;
  by_kind?: Record<string, number>;
  untyped: number;
}

export interface RequirementCounts {
  total: number;
  functional: number;
  nfr: number;
  by_pattern?: Record<string, number>;
  iso_tagged: number;
}

export interface TestCounts {
  total: number;
  by_state?: Record<string, number>;
  by_kind?: Record<string, number>;
}

export interface CoverageCounts {
  proven: number;
  total: number;
  /** Distinct items with stale evidence, as `?evidence=stale` lists them. */
  stale_items: number;
}

export interface FlowCounts {
  completed: number;
  cycle_p50_days?: number;
  cycle_p90_days?: number;
  rework: number;
  defects: number;
  escaped: number;
}

export interface ArchCounts {
  /** ADRs by status. */
  adrs?: Record<string, number>;
  design_docs: number;
  component_docs: number;
}

export interface RecordCounts {
  retros: number;
  release_notes: number;
  lessons: number;
  releases: number;
}

export interface Holds {
  items: ItemCounts;
  requirements: RequirementCounts;
  tests: TestCounts;
  coverage: CoverageCounts;
  flow: FlowCounts;
  architecture: ArchCounts;
  records: RecordCounts;
}

export interface GoalFacts {
  id: string;
  title: string;
  items_done: number;
  items_open: number;
  req_proven: number;
  req_total: number;
}

export interface Recent {
  at: string;
  /** adr | approval | signoff | ratify | release-stage | shipped */
  kind: string;
  code?: string;
  words: string;
  to: string;
}

export interface ActivityLine {
  at: string;
  item?: string;
  words: string;
}

/** The project's Overview: `GET /projects/{p}/overview`. */
export interface Overview {
  project: string;
  board_rev?: string;
  generated_at: string;
  attention: Attention;
  now: Now;
  holds: Holds;
  goals?: GoalFacts[];
  recent?: Recent[];
  activity?: ActivityLine[];
}

/** One row of the Items list: `GET /projects/{p}/items?…`. */
export interface ItemRow {
  code: string;
  title: string;
  kind?: string;
  status: string;
  milestone: string;
  sprint?: string;
  release?: string;
  req_total: number;
  req_proven: number;
  tests_red: number;
  tests_planned: number;
  stale: boolean;
}

/** The Items list's filters, each optional; `kind: "none"` is untyped. */
export interface ItemFilter {
  kind?: string;
  status?: string;
  milestone?: string;
  goal?: string;
  sprint?: string;
  release?: string;
  evidence?: "red" | "planned" | "stale" | "unproven" | "proven" | "untested" | "";
}

// ---- The engineering baseline (FTR-033): `GET /projects/{p}/baseline` and `…/profile` ----------

/**
 * One chosen option of a baseline answer: the bare option id when it carries no value, else the
 * option with its value. Read it through `baseline.ts`'s `chosenId`.
 */
export type BaselineChosen = string | { option: string; value?: string };

/** One answer of a baseline revision. */
export interface BaselineAnswer {
  area: string;
  options?: BaselineChosen[];
  later?: boolean;
  not_applicable?: string;
  explained?: boolean;
}

/** One baseline entry: a requirement, constraint, non-goal or practice an answer records. */
export interface BaselineEntry {
  id: string;
  pack: string;
  area: string;
  option: string;
  kind: "nfr" | "constraint" | "non_goal" | "practice";
  text: string;
  why?: string;
  iso25010?: string[];
  /** The ADR written at acceptance, for a constraint. */
  adr?: string;
  revision: string;
  superseded_by?: string;
}

/** An entry as the Baseline page shows it under its area. */
export interface BaselineEntryView extends BaselineEntry {
  in_force: boolean;
  pack_removed: boolean;
  /** Live items whose Design names it. */
  named_by: string[];
  /** Its acceptance is in the file but did not come through kanbanr, so it is not honoured. */
  not_accepted_through_kanbanr: boolean;
}

/** One area of the followed packs, with its standing answer. */
export interface BaselineArea {
  id: string;
  pack: string;
  title: string;
  question: string;
  status: "open" | "later" | "proposed" | "answered" | "not_applicable";
  options: BaselineChosen[];
  value: string;
  reason: string;
  item: string;
  revision: string;
  entries: BaselineEntryView[];
}

/** One revision of the baseline, with what accepting it does. */
export interface BaselineRevision {
  id: string;
  /** The item whose design asked; empty at setup. */
  item?: string;
  note?: string;
  answers: BaselineAnswer[];
  adds?: string[];
  replaces?: string[];
  proposed: DecisionStamp;
  accepted?: DecisionStamp;
  withdrawn?: DecisionStamp;
  state: "proposed" | "accepted" | "withdrawn";
  /** What the person pins when accepting. */
  rev: string;
  entries: BaselineEntry[];
  replaces_entries: BaselineEntry[];
  /** (stage, gate) acceptance will require. */
  stage_gates: [string, string][];
  /** Items naming a replaced entry, their ADRs, and docs holding its id. */
  depends_on: string[];
  /** `kanbanr baseline accept REV-n`. */
  command: string;
  warnings?: string[];
  /** Accepted, but whether through kanbanr cannot be checked: why. */
  acceptance_unverifiable?: string;
}

/** A pack the project follows or followed, with its drift as sentences. */
export interface BaselinePack {
  name: string;
  library: "board" | "personal" | "builtin";
  version: number;
  rev: string;
  adopted: DecisionStamp;
  removed?: DecisionStamp;
  messages?: string[];
}

/** The Baseline page's whole read: `GET /projects/{p}/baseline`. */
export interface BaselineView {
  packs: BaselinePack[];
  areas: BaselineArea[];
  revisions: BaselineRevision[];
  /** The charter's constraints, shown read-only beside the baseline. */
  charter_constraints: string[];
}

/** The packs of a project with their drift: `GET /projects/{p}/profile`. Internal. */
export interface ProfileStatus {
  packs: {
    name: string;
    library: "board" | "personal" | "builtin";
    version: number;
    removed: boolean;
    drift?: { newer: number | null; gone: boolean; new_areas: string[]; changed_recommendations: string[] };
    messages: string[];
  }[];
}

/** The person's planning decision, as a page recorded it (FTR-049): it changes no plan by itself. */
export interface PlanningDecision {
  /** `PD-<n>`. */
  id: string;
  action: "into-sprint" | "into-release" | "out-of-sprint";
  /** The sprint code or the release version; empty for `out-of-sprint`. */
  target: string;
  items: string[];
  by: string;
  /** Where it was taken (`monitor`, `terminal`), stamped by the daemon. */
  origin?: string;
  at: string;
  note?: string;
}

/** A move a declined decision could have been instead (FTR-049). */
export interface PlanningNearest {
  action: string;
  target: string;
}

/** What became of a planning decision: carried out by Claude, or declined with a reason (FTR-049). */
export interface PlanningOutcome {
  decision: string;
  outcome: "carried" | "declined";
  by: string;
  origin?: string;
  at: string;
  reason?: string;
  nearest?: PlanningNearest;
}

/** One planning decision with its derived state: `GET /projects/{p}/planning` (FTR-049). */
export interface PlanningView {
  decision: PlanningDecision;
  state: "pending" | "carried" | "declined";
  outcome: PlanningOutcome | null;
}

/** What `GET /projects/{p}/planning/{id}/check` answers (FTR-049). */
export interface PlanningAssessment {
  decision: string;
  /** Each a reason to decline. */
  problems: string[];
  /** Each for Claude to judge. */
  warnings: string[];
  nearest: PlanningNearest | null;
}

/** What `POST /projects/{p}/planning` takes (FTR-049). */
export interface PlanningRequest {
  action: PlanningDecision["action"];
  target: string;
  items: string[];
}

// ---- the Plan view (FTR-043) ----

/** The tab a Plan row falls in: the same three words the Releases page uses (FTR-041). */
export type PlanScope = "current" | "planned" | "past";

/** The Plan page's counts per tab and the tab it opens on: `GET /projects/{p}/plan/summary`. */
export interface PlanSummary {
  current: number;
  planned: number;
  past: number;
  /** `current` when anything runs today, else `planned`. */
  open: string;
  /** The day the counts are for, `YYYY-MM-DD` (UTC). */
  today: string;
}

/** A span on a Plan lane: a sprint's dates, or a milestone's reach. */
export interface PlanBand {
  from: string;
  to: string;
  label: string;
  /** `ok`, `info`, `warn`, `error` or `neutral`. */
  tone: string;
  /** The finished part, 0..1; null when there is nothing to count. */
  progress?: number | null;
}

/** A point on a Plan lane: a release's target or ship date (`diamond`), or a stage change (`dot`). */
export interface PlanMark {
  at: string;
  label: string;
  kind: string;
}

/** One row of the Plan page: a sprint, a release or a milestone. */
export interface PlanLane {
  /** `sprint:SP-004`, `release:v0.8.0` or `milestone:MS-004`: the row's id in the URL. */
  key: string;
  /** `sprint`, `release` or `milestone`. */
  kind: string;
  label: string;
  state: string;
  /** The lane's first and last day; empty on an undated lane. */
  from: string;
  to: string;
  bands: PlanBand[];
  marks: PlanMark[];
  done: number;
  total: number;
  points_done?: number | null;
  points_total?: number | null;
  carried: number;
  /** The running sprint's burndown; null on every other lane. */
  burndown?: SprintReport | null;
}

/** One Plan tab: `GET /projects/{p}/plan?scope=`. */
export interface PlanSlice {
  scope: PlanScope;
  today: string;
  from: string;
  to: string;
  lanes: PlanLane[];
  /** Rows with no date to place them by. */
  undated: PlanLane[];
}

/** One item of an opened Plan row, placed by the first rule that fits; never at a made-up date. */
export interface PlacedItem {
  code: string;
  title: string;
  status: string;
  /** The item's first and last day; empty when undated. */
  from: string;
  to: string;
  placed: "sprint" | "moves" | "start" | "undated" | string;
  /** On the project's dependency critical path. */
  critical: boolean;
  /** The planning decisions waiting on the item (FTR-049); empty until then. */
  pending: string[];
}

/** An opened Plan row: `GET /projects/{p}/plan/rows/{key}`. */
export interface RowBody {
  key: string;
  items: PlacedItem[];
  /** A milestone's item codes per displayed status, dependencies first; empty otherwise. */
  by_status: [string, string[]][];
  carried: { code: string; to: string; why?: string }[];
}

/** One project's row on the portfolio's timeline. */
export interface PortfolioLane {
  project: string;
  label: string;
  bands: PlanBand[];
  marks: PlanMark[];
}

/** The portfolio's timeline, its window worked out by the daemon: `GET /portfolio/plan`. */
export interface PortfolioPlan {
  from: string;
  to: string;
  today: string;
  lanes: PortfolioLane[];
}

// ---- The flow plan (FTR-061): `GET /projects/{p}/plan/flow` ----

/** Why an open task cannot start now (FTR-060). */
export type TaskWait =
  | { Dependency: string }
  | { Conflict: { task: string; paths: string[] } }
  | { ItemNotAtBuild: string }
  | { UnknownFootprint: string };

/** A task free to start (FTR-060's ReadyTask). */
export interface FlowReady {
  id: string;
  text: string;
  estimate_h?: number | null;
  /** On the critical path. */
  critical: boolean;
  /** The longest remaining chain starting at it, in agent-hours. */
  chain_h: number;
}

/** A task in progress. */
export interface FlowRunning {
  id: string;
  text: string;
  /** The live agent scope working it. */
  scope?: string | null;
  elapsed_h: number;
  estimate_h?: number | null;
  /** It has run longer than its estimate. */
  over: boolean;
}

/** A task that cannot start now, and every reason why. */
export interface FlowBlocked {
  id: string;
  waits: TaskWait[];
}

/** How long the remaining work takes with `lanes` agents and `slots` builds at once. */
export interface FlowForecast {
  lanes: number;
  slots: number;
  /** The lanes that help, bounded by the build slots. */
  effective_lanes: number;
  /** Hours from now. */
  finish_h: number;
}

/** A date a person set: a running sprint's last day, or a release's target. */
export interface PersonDate {
  what: string;
  date: string;
}

/** A scope's work in agent-hours over its task graph. */
export interface FlowPlan {
  scope: string;
  total_h: number;
  done_h: number;
  remaining_h: number;
  critical_path: string[];
  critical_h: number;
  widest: number;
  running: FlowRunning[];
  ready: FlowReady[];
  blocked: FlowBlocked[];
  forecasts: FlowForecast[];
  build_share: number;
  build_share_measured: boolean;
  unestimated: string[];
  untasked_items: string[];
  outside: string[];
  /** The only dates in the plan, each set by a person. */
  dates: PersonDate[];
}

// ---- The project's facts and its content catalog (FTR-005): `GET /projects/{p}/facts`, `…/catalog` ----

/** What the project is: its answers to the five facts questions (`kanbanr facts vocabulary`). */
export interface Facts {
  schema: number;
  languages: string[];
  runtimes: string[];
  production: string[];
  parts: string[];
  data: string[];
  /** When the record was last saved. */
  set_at?: string;
}

/**
 * `GET /projects/{p}/facts`: the facts record, or `{}` when none is set. Read it through
 * `catalogView.ts`'s `hasFacts`.
 */
export type FactsOrEmpty = Facts | {};

/** A (question, perspective) pair no applying row answers. */
export interface Cell {
  question: string;
  perspective: string;
}

/** Whether a row applies, why not, and what decided it: `always`, `facts` or `override`. */
export interface CatalogApplicability {
  applies: boolean;
  why?: string;
  by: string;
}

/** Where a row's content comes from: a written doc, a generated view, or an enforcing test. */
export interface CatalogSource {
  kind: "written" | "generated" | "enforced";
  /** Written: a board doc, a board folder ending `/`, or `repo:<path>`. */
  path?: string;
  /** Generated: `kanbanr:<view>` or `katalogr:<op>`. */
  generator?: string;
  /** Enforced: the test or rule that keeps the content true. */
  test?: string;
}

/** What a row's source holds right now. A written folder carries how many docs sit under it. */
export type SourceState =
  | "present"
  | "stub"
  | "missing"
  | { folder: { docs: number } }
  | "available"
  | "needs_katalogr"
  | "declared"
  | "no_repository";

/** One protection of a row and whether the project has it in force. */
export interface ProtectionStatus {
  name: string;
  in_force: boolean;
  why?: string;
}

/** One catalog row with its derived state. */
export interface RowStatus {
  id: string;
  title: string;
  level: "system" | "component" | "item";
  answers: string[];
  for: string[];
  /** Free text; the starter uses prose, table, mermaid, bpmn, openapi, yaml, adr, code. */
  notation?: string;
  source: CatalogSource;
  state: SourceState;
  applies: CatalogApplicability;
  protections: ProtectionStatus[];
  href?: string;
}

/** `GET /projects/{p}/catalog`: every row's state, the cells no applying row fills, and what is wrong. */
export interface CatalogStatus {
  facts_rev: string;
  /** The starter version the catalog began from; 0 with no catalog. */
  starter: number;
  rows: RowStatus[];
  gaps: Cell[];
  problems?: string[];
  docs_read: number;
}

// ---- Docs read at a position (FTR-004) ----

/** One repository's position (CD-2): a commit, and the hash of the uncommitted changes when there are any. */
export interface PositionRef {
  repo: string;
  commit: string | null;
  dirty: string | null;
}

/** A pick resolved to a position: what the banner says and where the bytes come from. */
export interface Picked {
  /** The pick's text: `main`, `worktree`, an item code, a sprint code, a version or a commit. */
  pick: string;
  /** `main`, `worktree`, `item`, `sprint`, `release` or `commit`. */
  kind: string;
  position: PositionRef;
  /** `git` reads objects at a commit; `disk` reads the working tree. */
  source: "git" | "disk";
  /** `FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4`, `main @ 6a973c6`. */
  label: string;
  /** The rule that found the commit: `branch tip`, `tag`, `merge commit recorded on the item` … */
  resolved_from: string;
}

/** A repository doc at a position: its text, or why it is not there. */
export interface RepoDoc extends Picked {
  path: string;
  text: string | null;
  blob: string | null;
  missing: boolean;
  why: string;
  /** For a missing doc, whether main has it. */
  at_main: boolean | null;
}

/** The `.md` files under `root` at a position. */
export interface RepoTree extends Picked {
  root: string;
  files: string[];
}

/** One entry of the position picker: whether it resolves here, and why not. */
export interface PickView {
  pick: string;
  kind: string;
  label: string;
  available: boolean;
  why: string;
}

/** The Charter page's derived read (FTR-076): `GET /projects/{p}/charter/progress`. */
export interface CharterProgress {
  project: string;
  board_rev?: string;
  /** When the answer was read, RFC 3339. */
  generated_at: string;
  releases_on: boolean;
  goals: GoalProgress[];
  route?: RouteStop[];
  marks?: GoalMark[];
  series?: GoalSeries[];
  lessons: LessonGrowth;
  wins?: Win[];
  finished?: Finished[];
}

/** The Overview's own goal counts, and the ids of the lessons that name the goal. */
export interface GoalProgress extends GoalFacts {
  lessons?: string[];
}

/** One release on the way travelled. */
export interface RouteStop {
  version: string;
  name?: string;
  state: "shipped" | "next";
  shipped_at?: string;
  /** The target date as the person set it; absent when none was set. */
  target?: string;
  patch_of?: string;
  items: number;
}

/** Items serving `goal` in `release`; only non-zero cells are sent. */
export interface GoalMark {
  goal: string;
  release: string;
  items: number;
}

export interface GoalSeries {
  goal: string;
  points: SeriesPoint[];
}

/** A goal's proof as of one shipped release, read from the evidence recorded today. */
export interface SeriesPoint {
  release: string;
  items: number;
  req_proven: number;
  req_total: number;
}

export interface LessonGrowth {
  in_force: number;
  total: number;
  retired: number;
  themes?: Theme[];
  cycles?: Cycle[];
  rows?: LessonRow[];
}

/** A group of lessons sharing a tag; `lessons` are lesson ids. */
export interface Theme {
  key: string;
  label: string;
  lessons: string[];
}

/** Lessons recorded in one release cycle. */
export interface Cycle {
  key: string;
  label: string;
  recorded: number;
  cumulative: number;
}

export interface LessonRow {
  id: string;
  lesson: string;
  kind: "practice" | "pitfall" | "decision";
  status: "candidate" | "active" | "retired";
  at: string;
  confidence_now: number;
  in_force: boolean;
  from_item?: string;
  from_retro?: string;
  /** sprint | milestone | absent */
  retro_kind?: string;
  evidence?: string;
  tags?: string[];
  goals?: string[];
  affirmed: number;
  contradicted: number;
  cited_by?: string[];
}

/** A recently finished item that serves a goal. */
export interface Win {
  code: string;
  title: string;
  at: string;
  goals: string[];
  release?: string;
}

/** A finished item that serves a goal, for "since you last looked". */
export interface Finished {
  code: string;
  at: string;
  goals: string[];
}
