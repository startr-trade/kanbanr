/**
 * Docs read at a position (FTR-004): the pick the person made, how it travels from page to page,
 * and the sentences that say what a page read. Pure, so `scripts/check-position.mjs` imports it.
 *
 * A pick travels as the query parameter `at=<pick text>` and is kept for the session, so following
 * a link keeps the position. A repository doc lives at `/p/<project>/repo/<path>?at=<pick>`; a
 * board doc stays at `/p/<project>/docs/file/<path>` and has no position.
 */
import type { Picked, RepoDoc, Signoff } from "./types";

/** The session key the last pick is kept under. */
export const AT_KEY = "kanbanr.at";

/** The pick a page shows when the URL names none and none was remembered. */
export const DEFAULT_PICK = "main";

/** The banner a board doc carries: its text is one record, whatever position is picked. */
export const BOARD_DOC_BANNER = "board doc · one record, the same at every position";

/** The pick `search` names (`?at=`), trimmed; "" when it names none. */
export function atOf(search: string): string {
  try {
    return (new URLSearchParams(search).get("at") ?? "").trim();
  } catch {
    return "";
  }
}

type Store = Pick<Storage, "getItem" | "setItem">;

function session(): Store | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

/** The last pick this session kept; "" when none, or when the browser keeps nothing. */
export function rememberedPick(store: Store | null = session()): string {
  try {
    return (store?.getItem(AT_KEY) ?? "").trim();
  } catch {
    return "";
  }
}

/** Keep `pick` for the session. A browser that refuses storage just forgets it. */
export function rememberPick(pick: string, store: Store | null = session()): void {
  if (!pick.trim()) return;
  try {
    store?.setItem(AT_KEY, pick.trim());
  } catch {
    /* the pick is still in the URL */
  }
}

/** The pick a page reads at: the URL's `at`, else the one remembered, else main. */
export function currentPick(search: string, remembered: string = rememberedPick()): string {
  return atOf(search) || remembered.trim() || DEFAULT_PICK;
}

/** `href` with `at=<pick>` set, ahead of any `#hash`; an `at` already there is replaced. */
export function withAt(href: string, pick: string): string {
  if (!pick.trim()) return href;
  const hashAt = href.indexOf("#");
  const hash = hashAt >= 0 ? href.slice(hashAt) : "";
  const head = hashAt >= 0 ? href.slice(0, hashAt) : href;
  const q = head.indexOf("?");
  const path = q >= 0 ? head.slice(0, q) : head;
  const params = new URLSearchParams(q >= 0 ? head.slice(q + 1) : "");
  params.set("at", pick.trim());
  return `${path}?${params.toString()}${hash}`;
}

const enc = encodeURIComponent;
const encPath = (path: string) => path.split("/").map(enc).join("/");

/** A repository doc's page at a pick. */
export function repoDocHref(project: string, path: string, pick: string): string {
  return withAt(`/p/${enc(project)}/repo/${encPath(path)}`, pick);
}

/** The page listing the repository's docs at a pick. */
export function repoDocsHref(project: string, pick: string): string {
  return withAt(`/p/${enc(project)}/repo`, pick);
}

/**
 * The repository path a markdown link target names, and its `#anchor`: a relative target (no scheme,
 * not `/…`, not `#…`) that starts with `docs/` once any `./` is dropped. A board link
 * (`/p/x/docs/file/…`) is absolute, so it is never taken for one.
 */
export function repoDocTarget(target: string): { path: string; hash: string } | null {
  let t = target.trim();
  if (t === "" || t.startsWith("/") || t.startsWith("#") || t.startsWith("?") || /^[a-z][a-z0-9+.-]*:/i.test(t)) return null;
  while (t.startsWith("./")) t = t.slice(2);
  if (!t.startsWith("docs/")) return null;
  const hashAt = t.indexOf("#");
  const hash = hashAt >= 0 ? t.slice(hashAt) : "";
  const noHash = hashAt >= 0 ? t.slice(0, hashAt) : t;
  const q = noHash.indexOf("?");
  const path = (q >= 0 ? noHash.slice(0, q) : noHash).replace(/&amp;/g, "&");
  if (path === "docs/" || path.split("/").includes("..")) return null;
  return { path, hash };
}

/**
 * `html` with every anchor whose relative `href` is under `docs/` pointing at the repository doc's
 * page at `pick`. Anchors to the board, to other sites and to anchors in the page are untouched.
 */
export function rewriteRepoLinks(html: string, project: string, pick: string): string {
  return html.replace(/(<a\b[^>]*?\bhref=")([^"]*)(")/g, (all, pre: string, href: string, post: string) => {
    const hit = repoDocTarget(href);
    if (!hit) return all;
    return pre + repoDocHref(project, hit.path, pick) + hit.hash + post;
  });
}

/** The first seven characters of a commit. */
export function shortCommit(commit: string): string {
  return commit.slice(0, 7);
}

/**
 * The sentence a page opens with, saying what it read: the path, the position, and where the bytes
 * came from. "docs/guide/x.md · at FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4 · from git (merge
 * commit recorded on the item)", or "… · working tree @ 6a973c6+1f3e… · from disk, with uncommitted
 * changes". `path` defaults to the one a `RepoDoc` carries.
 */
export function positionLine(at: Picked & { path?: string }, path: string | undefined = at.path): string {
  const where = at.kind === "worktree" ? at.label : `at ${at.label}`;
  const from =
    at.source === "disk"
      ? `from disk${at.position.dirty ? ", with uncommitted changes" : ""}`
      : `from git${at.resolved_from ? ` (${at.resolved_from})` : ""}`;
  return [path ?? "", where, from].filter(Boolean).join(" · ");
}

/**
 * What a doc that is not at the position says: "docs/system/store.md is not in the repository at
 * v0.6.0 (tag v0.6.0 @ 9fe1d20). main has it." The label's first part names the pick and the rest
 * is how it resolved; `main has it` or `main does not have it either` follows when main was asked.
 */
export function missingLine(doc: Pick<RepoDoc, "path" | "label" | "at_main">): string {
  const cut = doc.label.indexOf(" · ");
  const where = cut >= 0 ? `${doc.label.slice(0, cut)} (${doc.label.slice(cut + 3)})` : doc.label;
  const main = doc.at_main === true ? " main has it." : doc.at_main === false ? " main does not have it either." : "";
  return `${doc.path} is not in the repository at ${where}.${main}`;
}

/**
 * What a sign-off says it read: "docs read at 1a2b3c4" and a link to each doc it read, opened at the
 * commit it was signed at, so the reviewer sees exactly what was signed. Null when it recorded none.
 */
export function signoffDocs(
  project: string,
  s: Pick<Signoff, "position" | "docs_read">,
): { line: string; links: { path: string; href: string }[] } | null {
  const position = (s.position ?? "").trim();
  if (!position) return null;
  const [commit, dirty] = position.split("+");
  const links = Object.keys(s.docs_read ?? {})
    .sort()
    .map((path) => ({ path, href: repoDocHref(project, path, commit) }));
  return { line: `docs read at ${shortCommit(commit)}${dirty ? " (with uncommitted changes)" : ""}`, links };
}
