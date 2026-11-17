// The Review page's decision groups (FTR-038): the tabs and the group the page opens on, the
// person's selection inside one group, the group button's label and question, and the decision it
// posts. Pure, so the page and web/scripts/check-review-groups.mjs use the same code.
import type { DecisionItem, GroupCount, GroupDecision, GroupOutline, GroupSlice, ReviewRow } from "./types";
import { pickTab, type TabDef } from "./ui/tabs.ts";

/** The URL parameter that names the open group: `?group=docs-folded`. */
export const GROUP_PARAM = "group";

/** The group table's `DataTable` state key: its filter and sort are `?r.q=`, `?r.sort=`, `?r.dir=`. */
export const TABLE_STATE_KEY = "r.";

/** How many items a question names before it says "and N more". */
const NAMED = 5;

/**
 * One tab per group, in the summary's order: the group's label, with its count shown beside it
 * by `Tabs` (so the tab reads "Approve 3"). A group with nothing waiting and nothing Claude holds is hidden; a group Claude holds rows of says so
 * ("Approve · 4 Claude's").
 */
export function groupTabs(groups: GroupCount[]): TabDef[] {
  return groups.map((g) => {
    const held = g.held ?? 0;
    return {
      key: g.key,
      label: held > 0 ? `${g.label} · ${held} Claude's` : g.label,
      count: g.count,
      // A group of only what Claude holds stays listed, with count 0: the person can still decide it.
      hidden: g.count === 0 && held === 0,
    };
  });
}

/**
 * The tabs the page draws (DEF-106): once the summary has counted, it decides, as `groupTabs`. Before
 * that, the outline (the groups the config can have, in order) gives one tab per group with its count
 * still pending, so the groups are on screen at once. With neither read there is nothing to draw.
 */
export function reviewTabs(outline: GroupOutline[] | null, summary: GroupCount[] | null): TabDef[] {
  if (summary) return groupTabs(summary);
  if (outline) return outline.map((g) => ({ key: g.key, label: g.label, pending: true }));
  return [];
}

/** The group the page opens on: the one the URL names if it is shown, else the largest. */
export function defaultGroup(groups: GroupCount[], requested: string | null): string {
  return pickTab(groupTabs(groups), requested);
}

/** The person's selection: row keys of one group only. */
export interface Selection {
  group: string;
  keys: Set<string>;
}

/** Nothing selected in `group`: what opening a group starts with. */
export function emptySelection(group: string): Selection {
  return { group, keys: new Set() };
}

/**
 * Can `row` be selected now? Not when it is blocked, and not while its diagram check (approve and
 * ratify groups) has not finished clean: `pending` holds the keys still being checked.
 */
function selectable(row: ReviewRow, pending: ReadonlySet<string>): boolean {
  return row.blocked === "" && !pending.has(row.key);
}

/**
 * `sel` with `key` ticked (`on`) or unticked. Returned unchanged when `group` is not the
 * selection's group, the key is not one of `rows`, or (ticking) the row cannot be selected.
 */
export function select(
  sel: Selection,
  group: string,
  key: string,
  on: boolean,
  rows: ReviewRow[],
  pending: ReadonlySet<string> = new Set(),
): Selection {
  if (group !== sel.group) return sel;
  const row = rows.find((r) => r.key === key);
  if (!row) return sel;
  if (on && !selectable(row, pending)) return sel;
  if (on === sel.keys.has(key)) return sel;
  const keys = new Set(sel.keys);
  if (on) keys.add(key);
  else keys.delete(key);
  return { group: sel.group, keys };
}

/** Select-all: every row that can be selected now (`on`), or none. */
export function selectAll(
  sel: Selection,
  rows: ReviewRow[],
  on: boolean,
  pending: ReadonlySet<string> = new Set(),
): Selection {
  if (!on) return emptySelection(sel.group);
  return { group: sel.group, keys: new Set(rows.filter((r) => selectable(r, pending)).map((r) => r.key)) };
}

/**
 * The selection a group opens with: the rows Claude prepared a record for that can be decided.
 * The person can untick any of it, so the selection is always the person's.
 */
export function preselected(slice: GroupSlice, pending: ReadonlySet<string> = new Set()): Selection {
  return {
    group: slice.key,
    keys: new Set(slice.rows.filter((r) => r.prepared !== "" && selectable(r, pending)).map((r) => r.key)),
  };
}

/** The selected rows of `slice`'s group, in `rows`' order. */
function chosen(slice: GroupSlice, sel: Selection, rows: ReviewRow[]): ReviewRow[] {
  if (sel.group !== slice.key) return [];
  return rows.filter((r) => sel.keys.has(r.key));
}

/**
 * The batch the selection is exactly: every row of the slice in one batch is selected, and
 * nothing else is. The page posts it to the batch route (`api.confirmPreparedBatch`), so FTR-037's
 * one batch commit is kept. Null for any other selection.
 */
export function batchOf(slice: GroupSlice, sel: Selection): string | null {
  const picked = chosen(slice, sel, slice.rows);
  if (picked.length === 0 || picked.length !== sel.keys.size) return null;
  const batch = picked[0].batch;
  if (batch === "" || picked.some((r) => r.batch !== batch)) return null;
  const whole = slice.rows.filter((r) => r.batch === batch);
  return whole.length === picked.length ? batch : null;
}

/** Is this a sign-off group, whose label ("Sign off docs-folded") names what is signed? */
function isSignoff(slice: GroupSlice): boolean {
  return slice.action.toLowerCase() === "sign off";
}

/** Is this the which-release group, whose action plans the rows into a release? */
function isPlan(slice: GroupSlice): boolean {
  return slice.action.toLowerCase() === "plan into";
}

/**
 * The group button: the action and how many rows it applies to ("Approve 5", "Sign off docs-folded
 * 3", "Plan 2 into v0.8.0"), or "Confirm all N" when the selection is exactly one prepared batch.
 */
export function actionLabel(slice: GroupSlice, sel: Selection, target = ""): string {
  const n = chosen(slice, sel, slice.rows).length;
  if (batchOf(slice, sel) !== null) return `Confirm all ${n}`;
  if (isSignoff(slice)) return `${slice.label} ${n}`;
  if (isPlan(slice)) return target ? `Plan ${n} into ${target}` : `Plan ${n}`;
  return `${slice.action} ${n}`;
}

/** How a row is named in a question: its code, and a manual check's requirement beside it. */
function nameOf(row: ReviewRow): string {
  const code = row.code || row.key;
  return row.requirement ? `${code} ${row.requirement}` : code;
}

/** "A", "A and B", "A, B and C"; past six, the first five and "and N more". */
function listOf(names: string[]): string {
  if (names.length > NAMED + 1) return `${names.slice(0, NAMED).join(", ")} and ${names.length - NAMED} more`;
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * The ask-first question, naming the selected items in row order: "Approve FTR-041, FTR-043 and
 * DEF-071?", "Sign off docs-folded for DEF-066?", "Plan FTR-041 into v0.8.0?".
 */
export function askText(slice: GroupSlice, sel: Selection, rows: ReviewRow[], target = ""): string {
  const names = listOf(chosen(slice, sel, rows).map(nameOf));
  if (slice.key === "recode") return `Confirm ${chosen(slice, sel, rows).map((r) => r.title.toLowerCase()).join(" and ")} to their kinds' prefixes?`;
  if (isSignoff(slice)) return `${slice.label} for ${names}?`;
  if (isPlan(slice)) return target ? `Plan ${names} into ${target}?` : `Plan ${names}?`;
  return `${slice.action} ${names}?`;
}

/**
 * The decision the second press posts (`api.decideGroup`): each selected row with the pins the
 * slice showed, and its prepared record when it has one, so that record resolves. `action`
 * defaults to the group's; the delegated group's second button sends `undo`, and turning prepared
 * records down sends `reject`.
 */
export function decisionBody(
  slice: GroupSlice,
  sel: Selection,
  rows: ReviewRow[],
  by: string,
  target: string,
  action: string = slice.action,
): GroupDecision {
  const items: DecisionItem[] = chosen(slice, sel, rows).map((r) => ({
    key: r.key,
    code: r.code,
    rev: r.rev,
    spec_rev: r.spec_rev,
    signoff: r.signoff,
    requirement: r.requirement,
    test: r.test,
    prepared: r.prepared,
  }));
  return { group: slice.key, action, by, target, reason: "", items };
}
