/**
 * Sample data for the /ui gallery (FTR-046, R-8). Fixed values, dates included, so the gallery
 * reads the same on every machine and a walkthrough can compare screenshots of it.
 */
import type { Column } from "./table.ts";
import type { Lane } from "./timeline.ts";
import type { History, StageInput } from "./stepper.ts";
import type { Segment } from "./progress.ts";
import type { MatrixCell } from "./Matrix.tsx";
import type { TabDef } from "./tabs.ts";
import type { Tone } from "./tone.ts";

/** Tabs with counts: one empty, one hidden, one disabled with its reason. */
export const TABS: TabDef[] = [
  { key: "approve", label: "Approve", count: 3 },
  { key: "signoff", label: "Sign off", count: 7 },
  { key: "docs", label: "Docs folded", count: 0 },
  { key: "manual", label: "Manual checks", pending: true },
  { key: "later", label: "Later", count: 2, hidden: true },
  { key: "cut", label: "The cut", disabled: "waits for main to be pushed" },
];

/** What each tab's panel says in the gallery. */
export const TAB_TEXT: Record<string, string> = {
  approve: "Three designs wait for approval.",
  signoff: "Seven items wait for sign-off.",
  docs: "Nothing waits for docs to be folded.",
  manual: "The manual checks are still being counted.",
};

/** Accordion rows: one plain, one with a select that cannot be ticked, and why. */
export interface RowFixture {
  id: string;
  title: string;
  pills: { tone: Tone; text: string }[];
  body: string;
  selectDisabled?: string;
}

export const ROWS: RowFixture[] = [
  {
    id: "DEF-066",
    title: "DEF-066 · The review page loses its place after a decision",
    pills: [
      { tone: "error", text: "blocked" },
      { tone: "info", text: "3 tasks" },
    ],
    body: "The page reloads the whole queue after each decision, so the open row closes.",
  },
  {
    id: "FTR-041",
    title: "FTR-041 · The release page shows each release's stages as a stepper",
    pills: [
      { tone: "ok", text: "ready" },
      { tone: "warn", text: "2 checks waiting" },
    ],
    body: "Each release shows where it is in the cadence and who moved it there.",
    selectDisabled: "its checks are not green yet",
  },
];

/** A table row: an item with its stage, task count and size. */
export interface ItemRow {
  code: string;
  title: string;
  stage: string;
  tasks: number | null;
  size: string;
}

export const ITEM_ROWS: ItemRow[] = [
  { code: "FTR-9", title: "Plan a sprint from the ready list", stage: "Build", tasks: 4, size: "M" },
  { code: "FTR-10", title: "Show a release's notes before the cut", stage: "Design", tasks: 7, size: "L" },
  { code: "DEF-066", title: "The review page loses its place", stage: "Build", tasks: 3, size: "S" },
  { code: "MNT-002", title: "Rename the book's source folder", stage: "Done", tasks: null, size: "S" },
  { code: "FTR-046", title: "The monitor's shared components", stage: "Build", tasks: 9, size: "L" },
];

export const ITEM_COLUMNS: Column<ItemRow>[] = [
  { key: "code", label: "Item", value: (r) => r.code, sortable: true },
  { key: "title", label: "Title", value: (r) => r.title, sortable: true },
  { key: "stage", label: "Stage", value: (r) => r.stage, sortable: true, narrow: "hide" },
  { key: "tasks", label: "Tasks", value: (r) => r.tasks, sortable: true, align: "end" },
  { key: "size", label: "Size", value: (r) => r.size, narrow: "hide" },
];

/** Stat tiles: plain, warn, error and ok, one with a sparkline. */
export const TILES: { label: string; value: number; tone: Tone; to?: string; sub?: string; spark?: number[] }[] = [
  { label: "items in build", value: 12, tone: "neutral", to: "/ui", spark: [3, 5, 4, 8, 9, 12] },
  { label: "waiting for review", value: 7, tone: "warn", sub: "oldest 3 days" },
  { label: "checks failing", value: 2, tone: "error" },
  { label: "released this month", value: 5, tone: "ok" },
];

/** A progress bar's parts, of a total of 20. */
export const SEGMENTS: Segment[] = [
  { key: "done", label: "Done", value: 12, tone: "ok" },
  { key: "wip", label: "In progress", value: 3, tone: "info" },
  { key: "blocked", label: "Blocked", value: 1, tone: "error" },
  { key: "todo", label: "Not started", value: 4, tone: "neutral" },
];
export const SEGMENTS_TOTAL = 20;

/** Every tone, for the pills. */
export const TONES: Tone[] = ["ok", "info", "warn", "error", "neutral"];

/** A matrix: stages by kind of item. */
export const MATRIX_ROWS = ["Design", "Build", "Review"];
export const MATRIX_COLS = ["Features", "Defects", "Chores"];
const MATRIX: Record<string, MatrixCell> = {
  "Design/Features": { value: 4, to: "/ui" },
  "Design/Defects": { value: 1 },
  "Design/Chores": { value: 0 },
  "Build/Features": { value: 6, to: "/ui", tone: "info" },
  "Build/Defects": { value: 3, to: "/ui", tone: "warn" },
  "Build/Chores": { value: 2 },
  "Review/Features": { value: 2, to: "/ui", tone: "ok" },
  "Review/Defects": { value: 1, to: "/ui", tone: "error" },
  "Review/Chores": { value: 0 },
};
export function matrixCell(r: string, c: string): MatrixCell {
  return MATRIX[`${r}/${c}`] ?? { value: 0 };
}

/** A timeline: two sprint lanes and a release lane, with "today" fixed. */
export const TIMELINE = {
  from: "2026-11-01",
  to: "2026-12-31",
  today: "2026-11-17",
  lanes: [
    {
      key: "sp4",
      label: "SP-004",
      bands: [{ from: "2026-11-03", to: "2026-11-28", label: "v0.8.0 build", tone: "info", progress: 0.6, to_url: "/ui" }],
      marks: [],
    },
    {
      key: "sp5",
      label: "SP-005",
      bands: [{ from: "2026-12-01", to: "2026-12-20", label: "v0.9.0 build", tone: "neutral", pending: true }],
      marks: [],
    },
    {
      key: "releases",
      label: "Releases",
      bands: [],
      marks: [
        { at: "2026-11-10", label: "v0.7.0", kind: "diamond" },
        { at: "2026-12-01", label: "v0.8.0", kind: "diamond" },
      ],
    },
  ] as Lane[],
};

/** A release cadence and its recorded moves. */
export const STAGES: StageInput[] = [
  { key: "planned", label: "Planned" },
  { key: "candidate", label: "Candidate" },
  { key: "gates", label: "Gates" },
  { key: "cut", label: "Cut" },
  { key: "shipped", label: "Shipped" },
];
export const STAGE_HISTORY: History[] = [
  { stage: "planned", at: "2026-11-03T09:00:00Z", by: "Claude" },
  { stage: "candidate", at: "2026-11-14T16:20:00Z", by: "the person" },
];
export const STAGE_CURRENT = "gates";

/** The sentence an empty state offers to copy. */
export const ASK = "Ask Claude to plan the next sprint from the ready list";
