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
export interface FeatureDefinition {
  statement?: string;
  /** Charter goal ids this item serves — the link that makes "why" checkable. */
  goals?: string[];
  zachman?: Zachman;
  design_doc?: string;
  requirements?: Requirement[];
  approval?: Approval | null;
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
  approval: "missing" | "lapsed";
  started_unapproved?: string;
  definition: FeatureDefinition;
}

export function definitionGaps(feature: Feature): string[] {
  const def = feature.definition;
  if (!def) return ["no definition"];
  if (def.exempt?.trim()) return [];
  const gaps: string[] = [];
  if (!def.statement?.trim()) gaps.push("statement");
  for (const { column, answer } of zachmanColumns(def.zachman)) {
    if (!answer.trim()) gaps.push(column);
  }
  if (!(def.goals ?? []).length) gaps.push("goal link");
  const reqs = def.requirements ?? [];
  if (!reqs.length) gaps.push("requirements");
  if (reqs.some((r) => !(r.tests ?? []).length)) gaps.push("tests");
  return gaps;
}

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
