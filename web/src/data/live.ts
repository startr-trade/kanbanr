// Live updates by slice (DEF-081, A1 shared API §2–3): one EventSource per project, shared by
// every subscriber on the page. Each `slices` event invalidates the shared cache once, for the
// tags it names, then tells the subscribers. A `changed` event that no `slices` event follows
// within FALLBACK_MS (an older daemon, or a stream that lagged and dropped it) invalidates
// everything of the project, so a page never stays on data a change has passed.
import { cache, type Tag } from "./cache.ts";
import { apiUrl } from "../runtimeConfig.ts";

/** How long a `changed` event waits for its `slices` before everything is re-read. */
export const FALLBACK_MS = 1000;

interface Stream {
  es: EventSource;
  subs: Set<(tags: Tag[]) => void>;
  fallback?: ReturnType<typeof setTimeout>;
  /** The close scheduled when the last subscriber left; a new subscriber cancels it. */
  closing?: ReturnType<typeof setTimeout>;
}

const streams = new Map<string, Stream>();

function deliver(project: string, s: Stream, tags: Tag[]) {
  cache.invalidate(project, tags);
  for (const fn of [...s.subs]) fn(tags);
}

function open(project: string): Stream {
  const es = new EventSource(apiUrl(`/projects/${encodeURIComponent(project)}/events`));
  const s: Stream = { es, subs: new Set() };
  es.addEventListener("slices", (ev) => {
    if (s.fallback !== undefined) clearTimeout(s.fallback);
    s.fallback = undefined;
    let tags: Tag[] = ["*"];
    try {
      const body = JSON.parse((ev as MessageEvent<string>).data) as { project?: string; tags?: Tag[] };
      if (Array.isArray(body.tags)) tags = body.tags;
    } catch {
      /* an unreadable event is treated as "everything changed" */
    }
    deliver(project, s, tags);
  });
  es.addEventListener("changed", () => {
    if (s.fallback !== undefined) return;
    s.fallback = setTimeout(() => {
      s.fallback = undefined;
      deliver(project, s, ["*"]);
    }, FALLBACK_MS);
  });
  es.onerror = () => {
    /* EventSource reconnects by itself */
  };
  return s;
}

/**
 * Call `fn` with the tags of every change to `project`'s board, after the shared cache has been
 * invalidated for them. One EventSource per project serves every subscriber. When the last one
 * leaves, the stream is closed once the current render has settled (a page replaced by another of
 * the same project keeps it), and the project's entries are then marked stale, since changes are
 * no longer heard: they are re-read (a cheap 304 when unchanged) on the next mount.
 */
export function onSlices(project: string, fn: (tags: Tag[]) => void): () => void {
  let s = streams.get(project);
  if (!s) {
    s = open(project);
    streams.set(project, s);
  }
  if (s.closing !== undefined) clearTimeout(s.closing);
  s.closing = undefined;
  s.subs.add(fn);
  const stream = s;
  return () => {
    stream.subs.delete(fn);
    if (stream.subs.size > 0 || streams.get(project) !== stream) return;
    if (stream.closing !== undefined) clearTimeout(stream.closing);
    stream.closing = setTimeout(() => {
      stream.closing = undefined;
      if (stream.subs.size > 0 || streams.get(project) !== stream) return;
      if (stream.fallback !== undefined) clearTimeout(stream.fallback);
      stream.es.close();
      streams.delete(project);
      cache.invalidate(project, ["*"]);
    }, 0);
  };
}
