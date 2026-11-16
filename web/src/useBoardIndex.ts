import { useEffect, useMemo } from "react";
import { useLocation } from "react-router-dom";
import { type BoardIndex, makeIndex } from "./boardRefs";
import { useQuery } from "./data/useQuery";
import { TAGS, urls } from "./data/urls";
import type { CompactIndex } from "./types";

/**
 * What a project's references can point at, for linking its markdown (FEAT-181). Built from the
 * project's compact index (DEF-081), read once through the shared cache however many `Markdown`
 * blocks ask, and re-read when a live change names one of the index's tags, so a new item soon
 * links without a timer or a whole-board fetch.
 */
export function useBoardIndex(project: string | undefined): BoardIndex | null {
  const p = project ?? null;
  const { data } = useQuery<CompactIndex>(p, p ? urls.index(p) : null, TAGS.index);
  return useMemo(() => {
    if (!p || !data) return null;
    return makeIndex({
      project: p,
      items: data.items.map((i) => ({ code: i.code, requirements: i.requirements ?? [] })),
      milestones: data.milestones.map((m) => m.code),
      adrs: (data.adrs ?? []).filter((a) => a.path).map((a) => ({ id: a.id, path: a.path })),
      goals: data.goals.map((g) => g.id),
      docs: data.docs,
      codes: data.config.codes,
    });
  }, [p, data]);
}

/** Bring `#R-3` or `#G-1` into view once the page has drawn it — the router does not scroll. */
export function useScrollToHash(ready: boolean) {
  const { hash } = useLocation();
  useEffect(() => {
    if (!ready || !hash) return;
    document.getElementById(decodeURIComponent(hash.slice(1)))?.scrollIntoView({ block: "start" });
  }, [ready, hash]);
}
