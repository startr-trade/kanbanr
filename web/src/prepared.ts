// What Claude prepared for a decision (FTR-034), as the monitor shows it. Pure, so the pages, the
// panel and web::prepared (scripts/check-prepared.mjs) use the same code.
import type { PreparedSource, PreparedSubject, PreparedView } from "./types";

/**
 * The newest open record of `kind` whose subject has every key `subject` names. Stale, superseded,
 * answered and unknown records are never offered.
 */
export function openFor(
  views: PreparedView[] | null | undefined,
  kind: string,
  subject: Partial<PreparedSubject>,
): PreparedView | undefined {
  const matches = (v: PreparedView) =>
    v.state === "open" &&
    v.kind === kind &&
    Object.entries(subject).every(([k, want]) => (v.subject as Record<string, string | undefined>)[k] === want);
  const found = (views ?? []).filter(matches);
  // Newest first, whatever order the list came in.
  found.sort((a, b) => (b.at ?? "").localeCompare(a.at ?? "") || (b.id ?? "").localeCompare(a.id ?? ""));
  return found[0];
}

/**
 * Where a source links: a board doc or an item on this monitor, an http(s) link outside it, or
 * nothing (a commit is shown as code; a bare label or anything else is text, never a link).
 */
export function sourceHref(project: string, s: PreparedSource): { href: string; external: boolean } | null {
  const url = (s.url ?? "").trim();
  const p = `/p/${encodeURIComponent(project)}`;
  if (url.startsWith("doc:")) {
    const path = url.slice(4);
    if (!path || path.split("/").includes("..")) return null;
    return { href: `${p}/docs/file/${path}`, external: false };
  }
  if (url.startsWith("item:")) {
    const code = url.slice(5);
    if (!/^[A-Za-z0-9_-]+$/.test(code)) return null;
    return { href: `${p}/feature/${code}`, external: false };
  }
  if (/^https?:\/\/[^\s/?#@]+([/?#][^\s]*)?$/.test(url)) return { href: url, external: true };
  return null;
}

/** The fields the person may change before confirming, per kind; the server checks them again. */
export const EDITABLE: Record<string, string[]> = {
  signoff: ["note", "doc"],
  promotion: ["checked"],
  "gate-result": ["passed", "url", "note"],
  ratification: ["reason"],
};

const LABELS: Record<string, string> = {
  "signoff.note": "Note",
  "signoff.doc": "Record doc",
  "promotion.checked": "What was checked",
  "gate-result.passed": "Result",
  "gate-result.url": "Link to the run",
  "gate-result.note": "Note",
  "ratification.reason": "Why agree after the fact",
};

/** The label a kind's field is shown with. */
export function fieldLabel(kind: string, field: string): string {
  return LABELS[`${kind}.${field}`] ?? field;
}

function day(at: string | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at.slice(0, 10);
  return d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
}

/** A source as words: its label, or for a commit its short hash. */
function sourceWords(s: PreparedSource): string {
  const url = s.url ?? "";
  if (url.startsWith("commit:")) {
    const sha = url.slice(7, 14);
    return s.label && s.label !== "commit" ? `${s.label} ${sha}` : sha;
  }
  return s.label || url;
}

/** "Prepared by Claude on 12 Nov 2026 from CI run 4512, release tip 9f078bd". */
export function preparedLine(view: PreparedView): string {
  const from = (view.sources ?? []).map(sourceWords).filter(Boolean);
  return `Prepared by ${view.by || "someone"}${view.at ? ` on ${day(view.at)}` : ""}${
    from.length ? ` from ${from.join(", ")}` : ""
  }`;
}
