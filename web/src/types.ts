export type TaskState = "NotStarted" | "InProgress" | "Completed";

/** One entry in a project's activity changelog. */
export interface Activity {
  time: string;
  actor: string;
  message: string;
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
  verdict: "approved" | "withdrawn";
  reason?: string;
  rev?: string;
}

export interface FeatureDefinition {
  statement?: string;
  /** Charter goal ids this item serves — the link that makes "why" checkable. */
  goals?: string[];
  zachman?: Zachman;
  design_doc?: string;
  requirements?: Requirement[];
  approval?: Approval | null;
  /** Every agreement and withdrawal, oldest first (FEAT-069). Nothing is ever removed. */
  approvals?: ApprovalEvent[];
  /** A recorded reason work started without approval. */
  started_unapproved?: string;
  /** A recorded reason this item is exempt from gap reporting. */
  exempt?: string;
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
  approval: "missing" | "lapsed" | "unratified" | "signoff";
  /** Sign-offs the item's next stage is waiting on (FEAT-117). */
  signoffs_needed?: string[];
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

/** Per live item, keyed by code: what its card counts, and the next stage with how much it lacks. */
export type Readiness = Record<string, { gaps: Gap[]; next: { status: string; missing: number }[] }>;

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
  /** Days or story points (FEAT-121). */
  estimate_unit?: "days" | "points";
  /** Whether the project uses sprints and releases at all (FEAT-121). */
  cadence?: { sprints?: boolean; releases?: boolean; sprint_length_days?: number; release?: string };
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
}

/** A check a gate names, or the Zachman check narrowed to some columns. */
export type Condition = string | { zachman: string[] };

/** What entering a status asks for (FEAT-113, FEAT-114). */
export interface Gate {
  purpose?: string;
  requires?: Condition[];
  warns?: Condition[];
  signoffs?: string[];
  enforce?: "block" | "warn";
  kinds?: string[];
  on_enter?: string[];
}

/** Readable text for a gate condition. */
export function conditionText(c: Condition): string {
  return typeof c === "string" ? c.replace(/_/g, " ") : `zachman ${c.zachman.join("/")}`;
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
