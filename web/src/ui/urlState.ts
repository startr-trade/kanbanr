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
