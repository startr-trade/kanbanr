/**
 * Render a monitor component to markup, for the web checks (FTR-046).
 *
 * The checks used to import pure `.ts` modules or read `.tsx` sources as text, so none could see
 * what a component actually draws. This loads any `.tsx` through Vite's SSR loader — the same
 * transform the build uses, so no new dependency — renders it with `react-dom/server` inside a
 * `MemoryRouter` (components that read or write the URL need one), and parses the markup with
 * `jsdom` so a check can query roles and attributes.
 *
 * One Vite server is started per process and reused. Call `closeRender()` when done; it is also
 * closed on `process.exit` as a fallback.
 *
 *   const { html, document } = await render("src/ui/States.tsx", "LoadingState", {});
 */
import { createServer } from "vite";
import { JSDOM } from "jsdom";
import { existsSync } from "node:fs";
import { isAbsolute } from "node:path";

const webRoot = new URL("../..", import.meta.url).pathname;

let serverPromise = null;

function server() {
  if (!serverPromise) {
    serverPromise = createServer({
      root: webRoot,
      logLevel: "error",
      appType: "custom",
      server: { middlewareMode: true, hmr: false, watch: null },
      // Dependencies stay external and are loaded by Node itself, so the component and this module
      // share one React. react-router 7 ships CommonJS under the "node" condition, whose named
      // exports Vite cannot see; "module-sync" picks its ES module, as Node 24 itself does.
      ssr: { resolve: { externalConditions: ["module-sync"] } },
    });
    process.once("exit", () => {
      serverPromise?.then((s) => s.close()).catch(() => {});
    });
  }
  return serverPromise;
}

/**
 * Render `exportName` from `modulePath` (relative to `web/`, or absolute) with `props`.
 * `url` is the router's location, so `useSearchParams` and `useLocation` see it.
 * @returns {Promise<{ html: string, document: Document }>}
 */
export async function render(modulePath, exportName, props, { url = "/" } = {}) {
  const vite = await server();
  // An absolute file path is passed through; anything else is relative to `web/`.
  const path = isAbsolute(modulePath) && existsSync(modulePath) ? modulePath : `/${modulePath.replace(/^\.?\//, "")}`;
  // React and the router are imported by Node, the same instances Vite's external loader gives the
  // component, so every element is created by one React.
  const [mod, React, { renderToStaticMarkup }, { MemoryRouter }] = await Promise.all([
    vite.ssrLoadModule(path),
    import("react"),
    import("react-dom/server"),
    import("react-router-dom"),
  ]);
  const Component = mod[exportName];
  if (typeof Component !== "function" && typeof Component !== "object") {
    throw new Error(`render: ${modulePath} has no export named ${exportName}`);
  }
  const h = React.createElement ?? React.default.createElement;
  // MemoryRouter uses useLayoutEffect, which React 18 warns about on every server render; that one
  // warning is noise for a static check, so it is dropped and everything else passes through.
  const error = console.error;
  console.error = (msg, ...rest) => {
    if (typeof msg === "string" && msg.includes("useLayoutEffect does nothing on the server")) return;
    error(msg, ...rest);
  };
  let html;
  try {
    html = renderToStaticMarkup(h(MemoryRouter, { initialEntries: [url] }, h(Component, props ?? {})));
  } finally {
    console.error = error;
  }
  const { document } = new JSDOM(`<!DOCTYPE html><body>${html}</body>`).window;
  return { html, document };
}

/**
 * Load `modulePath` through the same Vite server `render` uses, so a check shares the instances the
 * rendered components see (the shared data cache, which a check seeds or records reads from).
 */
export async function load(modulePath) {
  const vite = await server();
  const path = isAbsolute(modulePath) && existsSync(modulePath) ? modulePath : `/${modulePath.replace(/^\.?\//, "")}`;
  return vite.ssrLoadModule(path);
}

/** Close the shared Vite server, so the check's process can exit. */
export async function closeRender() {
  if (!serverPromise) return;
  const s = await serverPromise;
  serverPromise = null;
  await s.close();
}
