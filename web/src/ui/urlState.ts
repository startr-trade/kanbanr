/**
 * Page state kept in the URL (FTR-046): the selected tab, a table's sort and filter. A link to the
 * page then opens it as the person left it, and a reload keeps it.
 *
 * The pure parsers are what the checks import; `useUrlState` is the thin hook over them. It writes
 * with `replace`, so the back button does not step through every tab click, and it keeps the hash
 * (the open rows live there). react-router's `setSearchParams` would drop the hash, so the hook
 * navigates with the search and the hash together, which is what `setSearchParams` does underneath.
 */
import { useCallback, useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";

/**
 * The values of `defaults`' keys in `search` (each read as `<prefix><key>`). A key that is absent,
 * empty or unreadable gives its default; keys not in `defaults` are ignored.
 */
export function parseState<T extends Record<string, string>>(search: string, defaults: T, prefix = ""): T {
  const out: Record<string, string> = { ...defaults };
  let params: URLSearchParams;
  try {
    params = new URLSearchParams(search);
  } catch {
    return out as T;
  }
  for (const key of Object.keys(defaults)) {
    const value = params.get(prefix + key);
    if (value !== null && value !== "") out[key] = value;
  }
  return out as T;
}

/**
 * `search` with `next` applied (each key written as `<prefix><key>`). A value equal to its default,
 * or undefined, is removed; every other parameter is kept as it was. Returns `"?…"`, or `""` when
 * nothing is left.
 */
export function writeState<T extends Record<string, string>>(
  search: string,
  next: Partial<T>,
  defaults: T,
  prefix = "",
): string {
  let params: URLSearchParams;
  try {
    params = new URLSearchParams(search);
  } catch {
    params = new URLSearchParams();
  }
  for (const [key, value] of Object.entries(next)) {
    if (value === undefined || value === defaults[key]) params.delete(prefix + key);
    else params.set(prefix + key, value as string);
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

/**
 * The page's state for `defaults`' keys, read from the query string, and a setter that writes a
 * patch with `replace` and keeps the hash. `prefix` namespaces the keys (a table's `stateKey`).
 */
export function useUrlState<T extends Record<string, string>>(
  defaults: T,
  prefix = "",
): [T, (patch: Partial<T>) => void] {
  const location = useLocation();
  const navigate = useNavigate();
  // The defaults are usually a literal, so they are keyed by their content, not their identity.
  const key = JSON.stringify(defaults);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const state = useMemo(() => parseState(location.search, defaults, prefix), [location.search, key, prefix]);
  const set = useCallback(
    (patch: Partial<T>) => {
      navigate(
        { pathname: location.pathname, search: writeState(location.search, patch, defaults, prefix), hash: location.hash },
        { replace: true, preventScrollReset: true },
      );
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [navigate, location.pathname, location.search, location.hash, key, prefix],
  );
  return [state, set];
}

/*
 * Open rows kept in the URL hash (FTR-046, R-2): `#open=v0.8.0,v0.7.0`. A plain `#DEF-066` is read
 * as one open id, so a link to an anchor (`?group=docs-folded#DEF-066`) opens that row. The hash
 * is a `key=value&…` list, so another list under a different `param` is kept when one is written.
 */

/** Decode one id; a malformed escape leaves it as written. */
function decodeId(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

/** Encode one id so `,`, `&`, `=`, `#` and spaces cannot break the list; `:` and `@` stay readable. */
function encodeId(id: string): string {
  return encodeURIComponent(id).replace(/%3A/gi, ":").replace(/%40/gi, "@");
}

/** The hash's `key=value` parts in order, or null for a plain anchor (`#DEF-066`). */
function hashParts(hash: string): [string, string][] | null {
  const text = hash.replace(/^#/, "");
  if (text === "") return [];
  if (!text.includes("=")) return null;
  return text
    .split("&")
    .filter((part) => part !== "")
    .map((part) => {
      const at = part.indexOf("=");
      return at < 0 ? [part, ""] : [part.slice(0, at), part.slice(at + 1)];
    });
}

/**
 * The open ids in `hash`: the comma list under `param` (`#open=a,b`), or a plain anchor (`#DEF-066`)
 * as one id. Empty ids are dropped; a hash that is neither gives no open rows.
 */
export function parseOpen(hash: string, param = "open"): Set<string> {
  const parts = hashParts(hash);
  if (parts === null) {
    const id = decodeId(hash.replace(/^#/, ""));
    return new Set(id ? [id] : []);
  }
  const entry = parts.find(([key]) => key === param);
  if (!entry) return new Set();
  return new Set(entry[1].split(",").map(decodeId).filter((id) => id !== ""));
}

/**
 * `hash` with the list under `param` set to `open` (in its order), other `key=value` parts kept.
 * A plain anchor is replaced: it was read as an open id, and `open` now carries the whole list.
 * Returns `"#…"`, or `""` when nothing is left.
 */
export function writeOpen(hash: string, open: Set<string>, param = "open"): string {
  const parts = (hashParts(hash) ?? []).filter(([key]) => key !== param);
  const ids = [...open].filter((id) => id !== "");
  if (ids.length > 0) parts.push([param, ids.map(encodeId).join(",")]);
  const text = parts.map(([key, value]) => `${key}=${value}`).join("&");
  return text ? `#${text}` : "";
}

/**
 * The open row ids, read from the hash list under `param` (default `open`), and a setter that opens
 * or closes one id. It writes with `replace` and keeps the path and the query string.
 */
export function useOpenRows(param = "open"): [Set<string>, (id: string, open: boolean) => void] {
  const location = useLocation();
  const navigate = useNavigate();
  const open = useMemo(() => parseOpen(location.hash, param), [location.hash, param]);
  const set = useCallback(
    (id: string, isOpen: boolean) => {
      const next = new Set(parseOpen(location.hash, param));
      if (isOpen) next.add(id);
      else next.delete(id);
      navigate(
        { pathname: location.pathname, search: location.search, hash: writeOpen(location.hash, next, param) },
        { replace: true, preventScrollReset: true },
      );
    },
    [navigate, location.pathname, location.search, location.hash, param],
  );
  return [open, set];
}
