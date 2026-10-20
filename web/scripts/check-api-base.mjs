#!/usr/bin/env node
//
// web::api_base_url_configurable (FEAT-178 R-2): WHERE an API base URL is configured, every request
// the monitor makes goes to it.
//
// Configures `apiBase` the way a host's config.json would, then calls every URL builder and every
// request in api.ts — through the SAME modules the monitor runs — and fails if any URL, EventSource
// URLs included, does not start with the configured base. It also fails if any source file other
// than runtimeConfig.ts spells a literal "/api, because a URL formed anywhere else would not follow
// the setting.
//
// Usage: npm run check:api-base
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { setConfig, apiUrl } from "../src/runtimeConfig.ts";
import { api } from "../src/api.ts";

const problems = [];
const BASE = "https://example.test/x/api";

const requested = [];
globalThis.fetch = async (url, init) => {
  requested.push({ url: String(url), method: init?.method ?? "GET" });
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    json: async () => ({ writes: true, identity: null }),
    text: async () => "",
  };
};

/** Every URL `api` forms for one call: a returned URL, or what it fetched. */
async function urlsOf(name, fn) {
  requested.length = 0;
  let out;
  try {
    out = fn("demo", "FEAT-1", "accept", "Reviewer", "a reason");
    if (out && typeof out.then === "function") out = await out;
  } catch (e) {
    problems.push(`${name}: threw ${e}`);
  }
  // A request made is what counts; a call that made none is a URL builder, and returns the URL.
  if (requested.length) return requested.map((r) => r.url);
  return typeof out === "string" ? [out] : [];
}

async function checkAll(base, label) {
  const names = Object.keys(api).filter((k) => typeof api[k] === "function" && k !== "approver");
  if (names.length < 20) problems.push(`only ${names.length} functions found on api — this check has gone stale`);
  for (const name of names) {
    const urls = await urlsOf(name, api[name]);
    if (urls.length === 0) problems.push(`${label}: api.${name} formed no URL`);
    for (const url of urls) {
      if (!url.startsWith(base + "/")) problems.push(`${label}: api.${name} -> ${url} (not under ${base})`);
    }
  }
  // The live streams are what pages hand to `new EventSource(...)`.
  for (const url of [api.allEvents(), api.projectEvents("demo")]) {
    if (!url.startsWith(base + "/")) problems.push(`${label}: EventSource URL ${url} is not under ${base}`);
  }
}

// A configured base, given with a trailing slash as a host might type it.
setConfig({ apiBase: BASE + "/" });
if (apiUrl("/projects") !== `${BASE}/projects`) problems.push(`apiUrl("/projects") = ${apiUrl("/projects")}`);
await checkAll(BASE, "configured");

// No configuration at all is the embedded binary's behaviour: same origin, /api.
setConfig({});
await checkAll("/api", "default");

// The setting is only honoured if no URL is formed anywhere else.
const root = new URL("../src", import.meta.url).pathname;
const walk = (dir) =>
  readdirSync(dir).flatMap((e) => {
    const p = join(dir, e);
    return statSync(p).isDirectory() ? walk(p) : /\.tsx?$/.test(p) ? [p] : [];
  });
for (const file of walk(root)) {
  if (file.endsWith("/runtimeConfig.ts")) continue;
  const src = readFileSync(file, "utf8");
  src.split("\n").forEach((line, i) => {
    if (/["'`]\/api\b/.test(line)) {
      problems.push(`${file.slice(root.length + 1)}:${i + 1} spells a literal /api URL — use apiUrl()\n    ${line.trim()}`);
    }
    if (/new EventSource\(\s*["'`]/.test(line)) {
      problems.push(`${file.slice(root.length + 1)}:${i + 1} opens an EventSource on a literal URL — use api.*Events()`);
    }
  });
}

if (problems.length) {
  for (const p of problems) console.error(`web::api_base_url_configurable — ${p}`);
  process.exit(1);
}
console.log("web::api_base_url_configurable — every request and stream goes to the configured API base; no other file spells /api.");
