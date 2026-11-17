// The monitor's one shared data cache (DEF-081, A1 shared API §3): every read is kept by its URL
// with the slice tags it depends on, so pages share a request instead of each fetching the whole
// board, and a live change re-reads only the entries whose tags it names. Pure: no React and no
// browser API, so node checks import it directly (web/scripts/check-data-cache.mjs).

/** A slice of a board a read depends on: "items", "item:FTR-046", "releases", or "*" for all. */
export type Tag = string;

/** One cached read. */
export interface Entry<T = unknown> {
  data: T | null;
  error: string | null;
  /** The slices this read depends on. */
  tags: readonly Tag[];
  /** A change named one of its tags since it was read: it is re-read when next used. */
  stale: boolean;
  /** The read in flight, shared by every caller asking for this URL meanwhile. */
  inflight?: Promise<T>;
  /** When the read in flight started (ms since the epoch); cleared when it ends. */
  startedAt?: number;
  /** Called when the entry changes: data arrived, or it went stale. */
  subs: Set<() => void>;
  /** Whether a read has ever finished, so a `null` body is not taken for "not loaded yet". */
  loaded: boolean;
  /** Bumped by every invalidation, so a read that started before one finishes still stale. */
  epoch: number;
}

/** Whether a change to `changed` affects a read of `depends`: they share a tag, or either has "*". */
export function tagsMatch(depends: readonly Tag[], changed: readonly Tag[]): boolean {
  if (depends.includes("*") || changed.includes("*")) return true;
  return depends.some((t) => changed.includes(t));
}

/** The project a monitor URL reads, from its `/projects/<p>` segment; null for a root read. */
export function projectOf(url: string): string | null {
  const m = /\/projects\/([^/?#]+)/.exec(url);
  if (!m) return null;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return m[1];
  }
}

export class DataCache {
  private entries = new Map<string, Entry>();

  private entry(url: string): Entry {
    let e = this.entries.get(url);
    if (!e) {
      e = { data: null, error: null, tags: [], stale: true, subs: new Set(), loaded: false, epoch: 0 };
      this.entries.set(url, e);
    }
    return e;
  }

  private notify(e: Entry) {
    for (const fn of [...e.subs]) fn();
  }

  /** The entry for `url`, if it was ever read or subscribed to. */
  read<T>(url: string): Entry<T> | undefined {
    return this.entries.get(url) as Entry<T> | undefined;
  }

  /**
   * Read `url` with `load`, keeping the result under `tags`. A read already in flight for the URL
   * is shared: `load` runs once however many callers ask meanwhile. The old data stays in the
   * entry until the new arrives; a failed read keeps it too and records the error.
   */
  fetch<T>(url: string, tags: readonly Tag[], load: () => Promise<T>): Promise<T> {
    const e = this.entry(url) as Entry<T>;
    e.tags = tags;
    if (e.inflight) return e.inflight;
    const epoch = e.epoch;
    e.startedAt = Date.now();
    const run = (async () => {
      try {
        const data = await load();
        e.data = data;
        e.error = null;
        e.loaded = true;
        return data;
      } catch (err) {
        e.error = err instanceof Error ? err.message : String(err);
        e.loaded = true;
        throw err;
      } finally {
        e.inflight = undefined;
        e.startedAt = undefined;
        // A change that landed while the read ran may not be in what it read.
        e.stale = e.epoch !== epoch;
        this.notify(e);
      }
    })();
    e.inflight = run;
    this.notify(e);
    return run;
  }

  /**
   * A change to `changed` on `project`'s board: mark every entry of that project's URLs whose
   * tags match stale, tell their subscribers, and return their URLs.
   */
  invalidate(project: string, changed: readonly Tag[]): string[] {
    if (changed.length === 0) return [];
    const hit: string[] = [];
    for (const [url, e] of this.entries) {
      if (projectOf(url) !== project || !tagsMatch(e.tags, changed)) continue;
      e.stale = true;
      e.epoch++;
      hit.push(url);
    }
    for (const url of hit) this.notify(this.entries.get(url)!);
    return hit;
  }

  /** Call `fn` whenever the entry for `url` changes; returns the unsubscribe. */
  subscribe(url: string, fn: () => void): () => void {
    const e = this.entry(url);
    e.subs.add(fn);
    return () => {
      e.subs.delete(fn);
    };
  }
}

/** The one instance every page shares. */
export const cache = new DataCache();

/** What a page draws from an entry. */
export interface QueryState<T> {
  data: T | null;
  error: string | null;
  /** True only while there is no data and no error yet. */
  loading: boolean;
  /** A stale entry being re-read: the old data stays on screen meanwhile. */
  refreshing: boolean;
  /** When the read in flight started (ms since the epoch), else null: a page says how long it has been working. */
  since: number | null;
}

/**
 * The state a page draws from `e` (R-9): `loading` while nothing has arrived, `refreshing` while
 * stale data is shown and re-read. A page draws its empty state only when `loading` is false and
 * `error` is null, so "not loaded yet" never reads as "nothing there".
 */
export function queryState<T>(e: Entry<T> | undefined): QueryState<T> {
  if (!e || !e.loaded) return { data: null, error: null, loading: true, refreshing: false, since: e?.startedAt ?? null };
  return {
    data: e.data,
    error: e.error,
    loading: false,
    refreshing: e.stale || e.inflight !== undefined,
    since: e.inflight !== undefined ? (e.startedAt ?? null) : null,
  };
}
