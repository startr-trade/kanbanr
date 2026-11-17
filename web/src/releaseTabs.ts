/**
 * The Releases page's model (FTR-041): its tabs from the summary read, the tab it opens on, which
 * releases start open, a release header's chips and stage stepper, and the item table's columns.
 * Pure, so the checks import it directly.
 */
import { pickTab, type TabDef } from "./ui/tabs.ts";
import { stepsFromStages, type Step } from "./ui/stepper.ts";
import type { Column } from "./ui/table.ts";
import type { Tone } from "./ui/tone.ts";
import type { Release, ReleaseItemRow, ReleasesSummary, Scope } from "./types";

/** A page tab: the route calls the third one `past`, the page labels it Released. */
export type ReleaseTab = "current" | "planned" | "released";

/** The route scope each page tab reads. */
export const TAB_OF: Record<ReleaseTab, Scope> = { current: "current", planned: "planned", released: "past" };

const TABS: { key: ReleaseTab; label: string; scope: Scope }[] = [
  { key: "current", label: "Current", scope: "current" },
  { key: "planned", label: "Planned", scope: "planned" },
  { key: "released", label: "Released", scope: "past" },
];

/**
 * The page's tabs (R-1): Current, Planned and Released, each with its count from the summary. An
 * empty tab is hidden, unless every tab is empty.
 */
export function releaseTabs(s: ReleasesSummary): TabDef[] {
  const all = TABS.every((t) => s[t.scope] === 0);
  return TABS.map((t) => ({ key: t.key, label: t.label, count: s[t.scope], hidden: !all && s[t.scope] === 0 }));
}

/**
 * The tab the page opens on (R-1): the one the URL asks for, else the summary's `open` (Current when
 * a release is in flight, else Planned, else Released), through `pickTab`.
 */
export function defaultTab(s: ReleasesSummary, requested: string | null): string {
  return pickTab(releaseTabs(s), requested, s.open === "past" ? "released" : s.open);
}

/**
 * The releases open when a tab is drawn (R-2): those of the hash's versions that are in `slice`, when
 * any is; else the first release (the newest) alone; none for an empty slice.
 */
export function initiallyOpen(slice: Release[], hashOpen: Set<string>): Set<string> {
  const named = slice.filter((r) => hashOpen.has(r.version)).map((r) => r.version);
  if (named.length > 0) return new Set(named);
  return new Set(slice.length > 0 ? [slice[0].version] : []);
}

/** One chip in a release's header. */
export interface HeaderChip {
  key: string;
  text: string;
  tone: Tone;
}

/**
 * A release header's chips (R-3), as the old tile showed them: version, name, state, stage ("not yet
 * a candidate" before the first), target, shipped date, how much is finished, how many items were
 * carried over, what superseded it, and "tagged, not released yet" while its tag waits.
 */
export function headerChips(r: Release, stages: string[]): HeaderChip[] {
  const chips: HeaderChip[] = [{ key: "version", text: r.version, tone: "neutral" }];
  if (r.name) chips.push({ key: "name", text: r.name, tone: "neutral" });
  chips.push({ key: "state", text: r.state, tone: r.state === "shipped" ? "ok" : "neutral" });
  const last = stages[stages.length - 1];
  chips.push({
    key: "stage",
    text: r.stage ? `stage ${r.stage}` : "not yet a candidate",
    tone: r.stage && r.stage === last ? "ok" : "neutral",
  });
  if (r.target) chips.push({ key: "target", text: `target ${r.target}`, tone: "neutral" });
  if (r.tagged_at && r.state !== "shipped" && !r.superseded_by) {
    chips.push({ key: "tagged", text: "tagged, not released yet", tone: "warn" });
  }
  if (r.superseded_by) chips.push({ key: "superseded", text: `Superseded by ${r.superseded_by}`, tone: "warn" });
  if (r.shipped_at) chips.push({ key: "shipped", text: `shipped ${r.shipped_at.slice(0, 10)}`, tone: "neutral" });
  const n = (r.items ?? []).length;
  const finished = r.finished ?? 0;
  const pct = n ? Math.round((finished / n) * 100) : 0;
  chips.push({ key: "finished", text: `${finished}/${n} finished (${pct}%)`, tone: "neutral" });
  const carried = (r.carried ?? []).length;
  if (carried > 0) chips.push({ key: "carried", text: `${carried} carried over`, tone: "warn" });
  return chips;
}

/**
 * A release's stage stepper (R-3, R-8): the configured stages through `stepsFromStages`, every
 * stage done once it has shipped. Each reached stage carries the date and who of the newest change
 * entering it. A stage undone (DEF-034's `undoes`) is after the stage the release went back to, so
 * it shows as not reached.
 */
export function stepperSteps(r: Release, stages: string[]): Step[] {
  const history = (r.stages ?? []).map((c) => ({ stage: c.stage, at: c.at, by: c.by }));
  return stepsFromStages(
    stages.map((s) => ({ key: s, label: s })),
    r.stage ? r.stage : null,
    history,
    r.state === "shipped",
  );
}

/** A value that sorts last when empty. */
const orNull = (v: string) => (v === "" ? null : v);

/**
 * The item table's columns (R-4). ID, Title and Status stay on a narrow screen; the rest move into
 * a row's detail. Tests reads "3/4" and sorts by its ratio; an item naming none sorts last.
 */
export const ITEM_COLUMNS: Column<ReleaseItemRow>[] = [
  { key: "code", label: "ID", value: (r) => r.code, sortable: true, narrow: "keep" },
  { key: "title", label: "Title", value: (r) => r.title, sortable: true, narrow: "keep" },
  { key: "status", label: "Status", value: (r) => r.status, sortable: true, narrow: "keep" },
  { key: "kind", label: "Kind", value: (r) => orNull(r.kind), sortable: true, narrow: "hide" },
  { key: "sprint", label: "Sprint", value: (r) => orNull(r.sprint), sortable: true, narrow: "hide" },
  {
    key: "tests",
    label: "Tests",
    value: (r) => (r.tests_named > 0 ? r.tests_green / r.tests_named : null),
    render: (r) => `${r.tests_green}/${r.tests_named}`,
    sortable: true,
    align: "end",
    narrow: "hide",
  },
  { key: "approval", label: "Approval", value: (r) => orNull(r.approval), sortable: true, narrow: "hide" },
  { key: "design_review", label: "Design review", value: (r) => orNull(r.design_review), sortable: true, narrow: "hide" },
  { key: "docs_folded", label: "Docs folded", value: (r) => orNull(r.docs_folded), sortable: true, narrow: "hide" },
  { key: "carried_to", label: "Carried to", value: (r) => orNull(r.carried_to), sortable: true, narrow: "hide" },
];
