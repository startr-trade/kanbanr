/**
 * The monitor's runtime configuration (FEAT-178).
 *
 * The monitor ships two ways: embedded in the binary, served beside the read API it talks to, and
 * as a standalone static bundle attached to each release, which any static host can serve and
 * point at a monitor's read API somewhere else. Both read the same `config.json`, fetched at
 * startup from beside `index.html`:
 *
 * - `apiBase`: where the read API is. Every request the monitor makes goes through `apiUrl`, so
 *   this is the one place a URL to the daemon is formed — no other source file spells `/api`.
 * - `routing`: `browser` (clean paths, needs a server that answers every route with index.html —
 *   the daemon does) or `hash` (`#/p/…`, for a plain static host with no rewrite rule).
 * - `readOnly`: hide every decision action and show the CLI command instead, whatever the daemon
 *   would allow. The standalone bundle ships with it on: reviewers view, decisions happen in
 *   Claude Code or a writes-enabled local monitor.
 *
 * A missing or unreadable file means the defaults, which are exactly the embedded behaviour.
 */
export type Routing = "browser" | "hash";

export interface RuntimeConfig {
  apiBase: string;
  routing: Routing;
  readOnly: boolean;
}

/** The embedded binary's behaviour: same origin, `/api`, clean paths, buttons when allowed. */
export const DEFAULTS: RuntimeConfig = { apiBase: "/api", routing: "browser", readOnly: false };

let current: RuntimeConfig = { ...DEFAULTS };

/** Validate a parsed `config.json`: a field that is missing or of the wrong type keeps its default. */
export function normalise(raw: unknown): RuntimeConfig {
  const obj = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const base =
    typeof obj.apiBase === "string" && obj.apiBase.trim() !== "" ? obj.apiBase.trim() : DEFAULTS.apiBase;
  return {
    // A trailing slash would double up with the paths `apiUrl` appends.
    apiBase: base.replace(/\/+$/, "") || DEFAULTS.apiBase,
    routing: obj.routing === "hash" || obj.routing === "browser" ? obj.routing : DEFAULTS.routing,
    readOnly: typeof obj.readOnly === "boolean" ? obj.readOnly : DEFAULTS.readOnly,
  };
}

/** Replace the active configuration. `loadConfig` calls it; the check scripts call it directly. */
export function setConfig(raw: unknown): RuntimeConfig {
  current = normalise(raw);
  return current;
}

/**
 * Fetch `./config.json` relative to the page's base URI and make it the active configuration. Never
 * rejects: a missing or invalid file leaves the defaults in place, so the monitor still starts.
 */
export async function loadConfig(
  fetchImpl: typeof fetch = fetch,
  baseURI: string = document.baseURI,
): Promise<RuntimeConfig> {
  try {
    const res = await fetchImpl(new URL("config.json", baseURI).toString(), { cache: "no-store" });
    if (!res.ok) return setConfig({});
    return setConfig(await res.json());
  } catch {
    return setConfig({});
  }
}

/** The active configuration. */
export function config(): RuntimeConfig {
  return current;
}

/** The URL of an API path (`/projects/x`), under the configured base. */
export function apiUrl(path: string): string {
  return current.apiBase + (path.startsWith("/") ? path : `/${path}`);
}

/** Whether decision actions are hidden regardless of what the daemon allows. */
export function isReadOnly(): boolean {
  return current.readOnly;
}

/** Which router the monitor builds. */
export function routing(): Routing {
  return current.routing;
}

/**
 * The href for an in-app path such as `/p/x/feature/Y`, for links the router does not draw itself
 * (rendered markdown). In hash routing that is `#/p/…`; in browser routing it is the path itself.
 */
export function appHref(path: string): string {
  return current.routing === "hash" ? `#${path}` : path;
}
