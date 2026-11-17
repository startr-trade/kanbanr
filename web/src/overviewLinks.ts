// The one place an Overview or Needs-you link is spelled (FTR-048): every number the Overview
// draws leads to the page that lists it, through these functions. Pure, so node checks import it.

const enc = encodeURIComponent;

/** `a=1&b=2` from the filters that are set, in the order given; "" when none is. */
function search(q: Record<string, string>): string {
  return Object.entries(q)
    .filter(([, v]) => v !== "")
    .map(([k, v]) => `${enc(k)}=${enc(v)}`)
    .join("&");
}

export const to = {
  /** One Review group (FTR-038). */
  review: (p: string, group: string) => `/p/${enc(p)}/review?group=${enc(group)}`,
  /** A release on the Releases page's current tab (FTR-041). */
  release: (p: string, v: string) => `/p/${enc(p)}/releases?tab=current#${enc(v)}`,
  /** A sprint on the Plan page's current tab (FTR-043). */
  sprint: (p: string, s: string) => `/p/${enc(p)}/plan?tab=current#${enc(s)}`,
  milestone: (p: string, m: string) => `/p/${enc(p)}/milestone/${enc(m)}`,
  /** The Items list with its filters; with none, every item. */
  items: (p: string, q: Record<string, string> = {}) => {
    const s = search(q);
    return s ? `/p/${enc(p)}/items?${s}` : `/p/${enc(p)}/items`;
  },
  /** A board docs folder, e.g. "decisions" or "design/components"; its slashes stay path separators. */
  docsFolder: (p: string, f: string) => `/p/${enc(p)}/docs/folder/${f.split("/").map(enc).join("/")}`,
  lessons: (p: string) => `/p/${enc(p)}/charter#lessons`,
  releases: (p: string) => `/p/${enc(p)}/releases`,
  item: (p: string, c: string) => `/p/${enc(p)}/feature/${enc(c)}`,
  /** An ADR (or any board doc) by its path; its slashes stay path separators. */
  adr: (p: string, path: string) => `/p/${enc(p)}/docs/file/${path.split("/").map(enc).join("/")}`,
};
