// The board's Sprint filter (FEAT-119, FTR-032): every item, the items in no sprint, or one sprint's
// items. Pure, so the page and web::sprint_filter (scripts/check-sprint-filter.mjs) use the same code.

/** Every item, sprint or not. */
export const ALL = "all";
/** Only the items in no sprint: the work still waiting for one. Sprint codes are SP-NNN, so this
 * never names a sprint. */
export const UNSCHEDULED = "unscheduled";

/** Whether an item shows under the filter `choice` (ALL, UNSCHEDULED or a sprint code). A project
 * that doesn't use sprints shows every item. */
export function inSprintFilter(f: { sprint?: string | null }, usesSprints: boolean, choice: string): boolean {
  if (!usesSprints || choice === ALL) return true;
  if (choice === UNSCHEDULED) return !f.sprint;
  return f.sprint === choice;
}
