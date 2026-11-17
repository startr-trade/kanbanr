// The Baseline page's pure part (FTR-033): how the areas group, how a proposed revision reads on
// FTR-034's prepared panel, and how an entry reads in one line. No React and no browser API, so
// web::baseline_page_shows_areas_and_accepts_a_revision (scripts/check-baseline.mjs) imports it.
import type { BaselineArea, BaselineChosen, BaselineEntry, BaselineRevision, BaselineView, PreparedView } from "./types.ts";

/**
 * A chosen option as words: its id, and its value in brackets when it carries one. The daemon
 * writes an option with no value as the bare id, so either form is read.
 */
export function chosenId(c: BaselineChosen | string): string {
  if (typeof c === "string") return c;
  return c.value ? `${c.option} (${c.value})` : c.option;
}

/** What an area's answer says, as words: "postgresql", "decide later", "not applicable (why)". */
export function answerText(a: { options?: (BaselineChosen | string)[]; later?: boolean; not_applicable?: string }): string {
  if (a.later) return "decide later";
  if (a.not_applicable != null) return a.not_applicable ? `not applicable (${a.not_applicable})` : "not applicable";
  return (a.options ?? []).map(chosenId).join(", ");
}

const KIND_WORD: Record<BaselineEntry["kind"], string> = {
  nfr: "nfr",
  constraint: "constraint",
  non_goal: "non-goal",
  practice: "practice",
};

/** One entry in one line: "B-003 constraint: PostgreSQL is the database". */
export function entryLine(e: Pick<BaselineEntry, "id" | "kind" | "text">): string {
  return `${e.id} ${KIND_WORD[e.kind] ?? e.kind}: ${e.text}`;
}

/** A group of areas on the page, under its heading. */
export interface AreaGroup {
  group: "answered" | "proposed" | "open" | "not_applicable";
  title: string;
  areas: BaselineArea[];
}

const GROUPS: { group: AreaGroup["group"]; title: string; holds: BaselineArea["status"][] }[] = [
  { group: "answered", title: "Accepted", holds: ["answered"] },
  { group: "proposed", title: "Proposed", holds: ["proposed"] },
  // A deferred area is still open: it is asked again at the next design that touches it.
  { group: "open", title: "Open", holds: ["open", "later"] },
  { group: "not_applicable", title: "Not applicable", holds: ["not_applicable"] },
];

/**
 * The view's areas grouped as the page lists them: accepted, proposed, open (the never-asked ones
 * first, then those deferred), not applicable. Each group keeps the view's order; an empty group
 * is left out.
 */
export function areaRows(view: Pick<BaselineView, "areas">): AreaGroup[] {
  const out: AreaGroup[] = [];
  for (const g of GROUPS) {
    const areas = g.holds.flatMap((s) => view.areas.filter((a) => a.status === s));
    if (areas.length) out.push({ group: g.group, title: g.title, areas });
  }
  return out;
}

/**
 * A revision as FTR-034's prepared panel shows a prepared record: what it answers, the entries it
 * adds and replaces, the stage gates accepting it adds, and what depends on it; pinned to its
 * `rev`, with the CLI command a person would run instead. Only a proposed revision is open.
 */
export function revisionAsPrepared(rev: BaselineRevision): PreparedView {
  const fields: Record<string, unknown> = {};
  fields.Answers = rev.answers.map((a) => `${a.area}: ${answerText(a)}`).join("; ");
  if (rev.entries.length) fields["Entries added"] = rev.entries.map(entryLine).join("; ");
  if (rev.replaces_entries.length) fields["Entries replaced"] = rev.replaces_entries.map(entryLine).join("; ");
  if (rev.stage_gates.length) fields["Stage gates added"] = rev.stage_gates.map(([stage, gate]) => `${stage} requires ${gate}`).join("; ");
  if (rev.depends_on.length) fields["Depends on"] = rev.depends_on.join(", ");
  if (rev.note) fields.Note = rev.note;
  return {
    id: rev.id,
    kind: "baseline-revision",
    subject: rev.item ? { item: rev.item } : {},
    fields,
    sources: rev.item ? [{ label: rev.item, url: `item:${rev.item}` }] : [],
    by: rev.proposed.by,
    origin: rev.proposed.origin,
    at: rev.proposed.at,
    rev: rev.rev,
    state: rev.state === "proposed" ? "open" : rev.state === "accepted" ? "confirmed" : "rejected",
    resolution: null,
    stale_why: rev.state === "accepted" ? "accepted" : rev.state === "withdrawn" ? "withdrawn" : "",
    page: "baseline",
    command: rev.command || `kanbanr baseline accept ${rev.id}`,
  };
}

/** The accepted revisions, newest acceptance first: the page's History. */
export function history(view: Pick<BaselineView, "revisions">): BaselineRevision[] {
  return view.revisions
    .filter((r) => r.state === "accepted")
    .sort((a, b) => (b.accepted?.at ?? "").localeCompare(a.accepted?.at ?? "") || b.id.localeCompare(a.id));
}

/** Where an ADR id is on this monitor, from the index's ADRs; null when the index does not list it. */
export function adrHref(project: string, adr: string, adrs: { id: string; path: string }[] | null | undefined): string | null {
  const found = (adrs ?? []).find((a) => a.id === adr);
  if (!found || found.path.split("/").includes("..")) return null;
  return `/p/${encodeURIComponent(project)}/docs/file/${found.path.split("/").map(encodeURIComponent).join("/")}`;
}
