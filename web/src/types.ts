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
}

export interface TodoList {
  code: string;
  description: string;
  tasks: Task[];
  created_at: string;
}

export interface Feature {
  code: string;
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
}

export interface FeatureDefinition {
  statement?: string;
  /** Charter goal ids this item serves — the link that makes "why" checkable. */
  goals?: string[];
  zachman?: Zachman;
  design_doc?: string;
  /** Board docs this item's design changes (FEAT-180); `docs_updated` checks each changed. */
  affects_docs?: string[];
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
  note?: string;
  doc?: string;
  status?: string;
  /** Absent means given. */
  verdict?: "given" | "withdrawn";
  reason?: string;
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

export interface TestRef {
  name: string;
  kind?: string;
  state: "planned" | "red" | "green";
  checked_rev?: string;
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
  approval: "missing" | "lapsed" | "unratified" | "signoff" | "release";
  /** Sign-offs the item's next stage is waiting on (FEAT-117). */
  signoffs_needed?: string[];
  /** Present when the item is ready for release and in none: the planned releases, in file order
   * (FEAT-185). Empty means there is none yet, and only a new release can be chosen. */
  release_choices?: string[];
  /** The definition revision this brief shows; a verdict sent with it is refused if it moved on (FEAT-159). */
  rev?: string;
  started_unapproved?: string;
  definition: FeatureDefinition;
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
  { gaps: Gap[]; next: { status: string; missing: number; signoffs: string[]; tasks_open: number; ends: boolean }[] }
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
export function featuresNewestFirst(features: Feature[]): Feature[] {
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
    integration_branch?: string;
    main_branch?: string;
    /** Branching follows the git model the cadence implies (FEAT-189). */
    git_model?: boolean;
    /** Each release stage's conditions and entry actions, keyed by stage name (FEAT-212). */
    release_gates?: Record<string, { purpose?: string; requires?: string[]; on_enter?: string[] }>;
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
  stages?: { stage: string; by: string; at: string; reason?: string }[];
  /** The code branch, made on entering the first stage. */
  branch?: string;
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

/** One condition of a release stage a promotion enters, and whether it holds (FEAT-212). */
export interface PlanCondition {
  check: "items_finished" | "prepared" | "main_at_release" | "pushed";
  stage: string;
  holds: boolean;
  detail: string;
  fix?: string;
}

/** One step a promotion takes (FEAT-212). */
export interface PlanStep {
  action: "branch" | "prepare" | "move_main" | "tag";
  stage: string;
  does: string;
  noop: boolean;
  blocked?: string;
}

/** What promoting a release would do, with nothing changed (FEAT-212). */
export interface ReleasePlan {
  version: string;
  from: string;
  to: string;
  direction: string;
  cut: boolean;
  /** What the one button says: "Promote to main", "Cut v0.2.1". */
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
export function displayedStates(config: ProjectConfig): string[] {
  return config.displayed_states.length ? config.displayed_states : config.statuses;
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
