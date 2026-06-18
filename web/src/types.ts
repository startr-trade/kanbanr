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
  created_at: string;
  updated_at: string;
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
