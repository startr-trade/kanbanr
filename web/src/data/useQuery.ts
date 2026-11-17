// A page's read through the shared cache (DEF-081, A1 shared API §3): one request per URL however
// many components ask, re-read only when a live change names one of its tags.
import { useCallback, useEffect, useReducer, useRef } from "react";
import { cache, queryState, type Tag } from "./cache.ts";
import { onSlices } from "./live.ts";
import { getJson } from "../api.ts";

export interface Query<T> {
  data: T | null;
  error: string | null;
  /** True only while there is no data yet: draw the empty state only when this is false. */
  loading: boolean;
  /** A stale read being re-read; the old data stays meanwhile. */
  refreshing: boolean;
  /** When the read in flight started (ms since the epoch), else null. */
  since: number | null;
  reload(): void;
}

/**
 * Read `url` (JSON) for `project`, depending on `tags`. While mounted, the project's live stream
 * is held open, and the read is fetched again whenever a change marks it stale. With `url` null
 * nothing is read and the query stays loading. `load` reads a body that is not plain JSON (a text
 * read, or one the monitor adjusts); every caller of one URL must pass the same one, since the
 * cache keeps one entry per URL.
 */
export function useQuery<T>(
  project: string | null,
  url: string | null,
  tags: readonly Tag[],
  load?: (url: string) => Promise<T>,
): Query<T> {
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const tagsKey = tags.join("\u0000");
  const loadRef = useRef(load);
  loadRef.current = load;
  const loader = useCallback((u: string) => (loadRef.current ? loadRef.current(u) : getJson<T>(u)), []);

  useEffect(() => {
    if (!project) return;
    return onSlices(project, () => {});
  }, [project]);

  useEffect(() => {
    if (!url) return;
    const tagList = tagsKey === "" ? [] : tagsKey.split("\u0000");
    const refetch = () => {
      const e = cache.read<T>(url);
      if (!e || (e.stale && !e.inflight)) {
        cache.fetch<T>(url, tagList, () => loader(url)).catch(() => {
          /* the error is kept in the entry */
        });
      }
    };
    const unsubscribe = cache.subscribe(url, () => {
      refetch();
      rerender();
    });
    refetch();
    rerender();
    return unsubscribe;
  }, [url, tagsKey, loader]);

  const reload = useCallback(() => {
    if (!url) return;
    cache.fetch<T>(url, tagsKey === "" ? [] : tagsKey.split("\u0000"), () => loader(url)).catch(() => {});
  }, [url, tagsKey, loader]);

  const state = queryState(url ? cache.read<T>(url) : undefined);
  return { ...state, reload };
}
