import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { api } from "./api";
import { type BoardIndex, makeIndex } from "./boardRefs";
import type { DocFolder } from "./types";

/**
 * What a project's references can point at, for linking its markdown (FEAT-181). Built from the
 * routes the monitor already reads, once per project and shared by every `Markdown` on the page;
 * refetched when older than a few seconds, so a new item soon links without a live connection per
 * rendered block.
 */
const FRESH_MS = 5000;
const cache = new Map<string, { at: number; index: Promise<BoardIndex | null> }>();

function docPaths(folder: DocFolder): string[] {
  return [...folder.docs.map((d) => d.path), ...folder.folders.flatMap(docPaths)];
}

async function load(project: string): Promise<BoardIndex | null> {
  try {
    const [p, charter, adrs, docs] = await Promise.all([
      api.getProject(project),
      api.getCharter(project).catch(() => null),
      api.getAdrs(project).catch(() => []),
      api.getDocTree(project).catch(() => null),
    ]);
    return makeIndex({
      project,
      items: p.features.map((f) => ({
        code: f.code,
        requirements: (f.definition?.requirements ?? []).map((r) => r.id),
      })),
      milestones: p.milestones.map((m) => m.code),
      adrs: adrs.filter((a) => a.path).map((a) => ({ id: a.id, path: a.path as string })),
      goals: (charter?.goals ?? []).map((g) => g.id),
      docs: docs ? docPaths(docs) : [],
      codes: p.config.codes,
    });
  } catch {
    return null;
  }
}

export function useBoardIndex(project: string | undefined): BoardIndex | null {
  const [index, setIndex] = useState<BoardIndex | null>(null);
  useEffect(() => {
    if (!project) {
      setIndex(null);
      return;
    }
    let entry = cache.get(project);
    if (!entry || Date.now() - entry.at > FRESH_MS) {
      entry = { at: Date.now(), index: load(project) };
      cache.set(project, entry);
    }
    let live = true;
    entry.index.then((i) => live && setIndex(i));
    return () => {
      live = false;
    };
  }, [project]);
  return index;
}

/** Bring `#R-3` or `#G-1` into view once the page has drawn it — the router does not scroll. */
export function useScrollToHash(ready: boolean) {
  const { hash } = useLocation();
  useEffect(() => {
    if (!ready || !hash) return;
    document.getElementById(decodeURIComponent(hash.slice(1)))?.scrollIntoView({ block: "start" });
  }, [ready, hash]);
}
