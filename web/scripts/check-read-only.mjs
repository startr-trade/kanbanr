#!/usr/bin/env node
//
// web::standalone_is_read_only (FEAT-178 R-3): WHERE the monitor runs from the standalone bundle
// with its shipped configuration, every decision action is hidden and the CLI command shown.
//
// Loads config.standalone.json — the file the release packs into the bundle as config.json — into
// the same runtime-config module the monitor runs, against a daemon that WOULD allow writes, and
// asserts that:
//   - the monitor considers itself read-only, and the daemon's capabilities are reported as
//     `writes: false`, which is what every page gates its decision buttons on;
//   - every write call is refused without sending anything, naming the CLI command instead;
//   - every page that reads the daemon's `writes` also consults the read-only setting.
//
// Usage: npm run check:read-only
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { setConfig, isReadOnly, routing } from "../src/runtimeConfig.ts";
import { api } from "../src/api.ts";

const problems = [];
const shipped = JSON.parse(readFileSync(new URL("../config.standalone.json", import.meta.url), "utf8"));
setConfig(shipped);

if (!isReadOnly()) problems.push("the shipped standalone configuration is not read-only");
if (routing() !== "hash") problems.push(`the shipped standalone configuration routes by ${routing()}, not hash`);

const sent = [];
globalThis.fetch = async (url, init) => {
  sent.push({ url: String(url), method: init?.method ?? "GET" });
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    // A daemon started with --allow-writes, with an identity: everything a button needs.
    json: async () => ({ writes: true, identity: { name: "Reviewer", email: "r@example.test" } }),
    text: async () => "",
  };
};

const meta = await api.getMeta();
if (meta.writes !== false) problems.push(`getMeta reports writes: ${meta.writes} — the pages would offer decisions`);

const writes = {
  approve: () => api.approve("demo", "FEAT-1", "Reviewer"),
  ratify: () => api.ratify("demo", "FEAT-1", "Reviewer"),
  unapprove: () => api.unapprove("demo", "FEAT-1", "Reviewer", "changed"),
  signoff: () => api.signoff("demo", "FEAT-1", "design-review", "Reviewer"),
  decideAdr: () => api.decideAdr("demo", "ADR-0001", "accept", "Reviewer"),
};
for (const [name, call] of Object.entries(writes)) {
  sent.length = 0;
  try {
    await call();
    problems.push(`api.${name} succeeded on a read-only monitor`);
  } catch (e) {
    if (!/`kanbanr [a-z]/.test(String(e))) problems.push(`api.${name} refused without naming the CLI command: ${e}`);
  }
  const posted = sent.filter((r) => r.method !== "GET");
  if (posted.length) problems.push(`api.${name} sent ${posted.map((r) => `${r.method} ${r.url}`).join(", ")}`);
}

// A page that decides from the daemon's `writes` alone would show a button the setting should hide.
const root = new URL("../src", import.meta.url).pathname;
const walk = (dir) =>
  readdirSync(dir).flatMap((e) => {
    const p = join(dir, e);
    return statSync(p).isDirectory() ? walk(p) : /\.tsx$/.test(p) ? [p] : [];
  });
for (const file of walk(root)) {
  const src = readFileSync(file, "utf8");
  if (/\.writes\b/.test(src) && !/\bisReadOnly\(\)/.test(src)) {
    problems.push(`${file.slice(root.length + 1)} gates an action on the daemon's writes but never on isReadOnly()`);
  }
}

if (problems.length) {
  for (const p of problems) console.error(`web::standalone_is_read_only — ${p}`);
  process.exit(1);
}
console.log("web::standalone_is_read_only — the shipped standalone config hides every decision, sends no write, and names the CLI command.");
