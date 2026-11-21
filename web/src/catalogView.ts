// The Catalog page's pure part (FTR-005): how the rows group by level, how the gaps read as one
// sentence, where a row's source links, and the chips a row line shows. No React and no browser
// API, so web::catalog (scripts/check-catalog.mjs) imports it directly.
import type { CatalogStatus, Cell, Facts, FactsOrEmpty, RowStatus, SourceState } from "./types.ts";

/** Whether `GET …/facts` answered with a facts record rather than `{}`. */
export function hasFacts(f: FactsOrEmpty | null | undefined): f is Facts {
  return !!f && Array.isArray((f as Facts).languages);
}

/** Whether the project has a catalog: rows, a starter version, or a catalog file with problems. */
export function hasCatalog(s: CatalogStatus): boolean {
  return s.rows.length > 0 || s.starter > 0 || (s.problems ?? []).length > 0;
}

export type Level = RowStatus["level"];

/** A group of rows on the page, under its heading. */
export interface RowGroup {
  level: Level;
  title: string;
  rows: RowStatus[];
}

const GROUPS: [Level, string][] = [
  ["system", "System"],
  ["component", "Components"],
  ["item", "Items"],
];

/** The rows by level, System then Components then Items, each in the catalog's order; empty groups dropped. */
export function groupRows(rows: RowStatus[]): RowGroup[] {
  return GROUPS.map(([level, title]) => ({ level, title, rows: rows.filter((r) => r.level === level) })).filter(
    (g) => g.rows.length > 0,
  );
}

/**
 * The gaps as one sentence, question by question in the order given:
 * "Nobody answers: when for the operator; who for the builder and the user." "" when there is none.
 */
export function gapSentence(gaps: Cell[]): string {
  if (gaps.length === 0) return "";
  const byQuestion = new Map<string, string[]>();
  for (const g of gaps) {
    const list = byQuestion.get(g.question) ?? [];
    if (!list.includes(g.perspective)) list.push(g.perspective);
    byQuestion.set(g.question, list);
  }
  const parts = [...byQuestion].map(([q, ps]) => `${q} for ${andList(ps.map((p) => `the ${p}`))}`);
  return `Nobody answers: ${parts.join("; ")}.`;
}

function andList(words: string[]): string {
  if (words.length <= 1) return words.join("");
  return `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

/** The monitor page each known generated view is shown on. */
const GENERATOR_PAGE: Record<string, string> = {
  // The charter page shows the goals and the work behind each: the trace.
  "kanbanr:charter": "charter",
  "kanbanr:trace": "charter",
  "kanbanr:releases": "releases",
  "kanbanr:gantt": "gantt",
};

/**
 * Where a row's source links on the monitor: a board doc to its page, a board folder to the Docs
 * page at that folder, a known generated view to the page that shows it. `null` for a repository
 * doc (text until FTR-004 gives it a page), a katalogr view, an enforced source or an empty path.
 */
export function sourceHref(project: string, row: Pick<RowStatus, "source">): string | null {
  const p = `/p/${encodeURIComponent(project)}`;
  const s = row.source;
  if (s.kind === "written") {
    const path = (s.path ?? "").trim();
    if (!path || path.startsWith("repo:")) return null;
    const enc = (x: string) => x.split("/").map(encodeURIComponent).join("/");
    if (path.endsWith("/")) return `${p}/docs/folder/${enc(path.replace(/\/+$/, ""))}`;
    return `${p}/docs/file/${enc(path)}`;
  }
  if (s.kind === "generated") {
    const page = GENERATOR_PAGE[(s.generator ?? "").trim()];
    return page ? `${p}/${page}` : null;
  }
  return null;
}

/** The source in words: "written architecture/context.md", "generated kanbanr:charter", "enforced <test>". */
export function sourceText(row: Pick<RowStatus, "source">): string {
  const s = row.source;
  if (s.kind === "written") return s.path ? `written ${s.path}` : "written in each item's spec";
  if (s.kind === "generated") return `generated ${s.generator ?? ""}`.trim();
  return `enforced ${s.test ?? ""}`.trim();
}

/** A chip's tone: never carried by colour alone, since the label says it in words. */
export type ChipTone = "ok" | "warn" | "neutral" | "off";

export interface Chip {
  kind: "applies" | "state" | "notation" | "protection";
  label: string;
  tone: ChipTone;
  /** Shown as the chip's title: why a protection is or is not in force. */
  title?: string;
}

/** A source state in words, with its tone. */
export function stateChip(state: SourceState): Chip {
  if (typeof state === "object") {
    const n = state.folder.docs;
    return n === 0
      ? { kind: "state", label: "missing: no docs in the folder", tone: "warn" }
      : { kind: "state", label: `${n} doc${n === 1 ? "" : "s"}`, tone: "ok" };
  }
  const words: Record<string, [string, ChipTone]> = {
    present: ["present", "ok"],
    stub: ["stub", "warn"],
    missing: ["missing", "warn"],
    available: ["available", "ok"],
    needs_katalogr: ["needs Katalogr", "neutral"],
    declared: ["declared", "neutral"],
    no_repository: ["no repository", "neutral"],
  };
  const [label, tone] = words[state] ?? [String(state), "neutral"];
  return { kind: "state", label, tone };
}

/**
 * The chips of one row line, in order: whether it applies (or "off: <why>"), its source state,
 * its notations, and each protection with whether it is in force.
 */
export function chips(row: RowStatus): Chip[] {
  const out: Chip[] = [];
  const a = row.applies;
  if (a.applies) out.push({ kind: "applies", label: "applies", tone: "ok" });
  else if (a.by === "override") out.push({ kind: "applies", label: a.why ? `off: ${a.why}` : "off", tone: "off" });
  else out.push({ kind: "applies", label: a.why ? `does not apply: ${a.why}` : "does not apply", tone: "off" });
  out.push(stateChip(row.state));
  for (const n of (row.notation ?? "").split(",").map((x) => x.trim()).filter(Boolean)) {
    out.push({ kind: "notation", label: n, tone: "neutral" });
  }
  for (const p of row.protections) {
    out.push({
      kind: "protection",
      label: `${p.name}: ${p.in_force ? "in force" : "not configured"}`,
      tone: p.in_force ? "ok" : "warn",
      title: p.why || undefined,
    });
  }
  return out;
}

/** Whether a row line is greyed: it does not apply. */
export function isOff(row: RowStatus): boolean {
  return !row.applies.applies;
}

/** The facts record in one line: "rust, typescript · native, browser · …", by key, empty keys left out. */
export function factsLine(f: Facts): string {
  const keys: (keyof Facts)[] = ["languages", "runtimes", "production", "parts", "data"];
  return keys
    .map((k) => [k, (f[k] as string[]) ?? []] as const)
    .filter(([, v]) => v.length > 0)
    .map(([k, v]) => `${k}: ${v.join(", ")}`)
    .join(" · ");
}
