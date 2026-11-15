#!/usr/bin/env node
//
// web::check_build_info (FTR-036): the page header shows the kanbanr version and commit the
// monitor runs (R-1), and a warning naming both versions appears when the board was last written
// by a newer kanbanr (R-2). The pure functions the page renders are checked case by case; the
// shell is read to require that it renders them in the header and asks `getMeta` once.
//
// Usage: npm run check:build-info
import { readFileSync } from "node:fs";
import { buildLabel, behindWarning } from "../src/buildInfo.ts";

const problems = [];
const expect = (test, name, got, want) => {
  if (got !== want) problems.push([test, `${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`]);
};
const H = "web::check_build_info_header_shows_the_running_version_and_commit";
const W = "web::check_build_info_the_warning_names_both_versions";

// R-1
expect(H, "version and commit", buildLabel({ writes: false, identity: null, version: "0.6.0", commit: "1138833" }), "v0.6.0 · 1138833");
expect(H, "version only", buildLabel({ writes: false, identity: null, version: "0.6.0" }), "v0.6.0");
expect(H, "unknown commit", buildLabel({ writes: false, identity: null, version: "0.6.0", commit: "unknown" }), "v0.6.0");
expect(H, "an older server", buildLabel({ writes: false, identity: null }), "");
expect(H, "no meta", buildLabel(null), "");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const header = app.slice(app.indexOf('<header className="topbar">'), app.indexOf("</header>"));
if (!/buildLabel\(/.test(header) && !/\{label\}/.test(header)) {
  problems.push([H, "App.tsx: the topbar header does not render the build label"]);
}
if (!/className="build"/.test(header)) problems.push([H, "App.tsx: no .build element in the topbar header"]);
const calls = (app.match(/getMeta\(/g) ?? []).length;
if (calls !== 1) problems.push([H, `App.tsx: getMeta is called ${calls} times, wanted once`]);

// R-2
const behind = {
  writes: false, identity: null, version: "0.5.0", commit: "aaa1111",
  board_writer: { version: "0.6.0", commit: "bbb2222", at: "2026-11-13T09:12:00Z" }, writer_newer: true,
};
const text = behindWarning(behind) ?? "";
for (const part of ["0.5.0", "aaa1111", "0.6.0", "bbb2222"]) {
  if (!text.includes(part)) problems.push([W, `the warning does not name ${part}: ${text}`]);
}
expect(W, "not newer", behindWarning({ ...behind, writer_newer: false }), null);
expect(W, "no writer", behindWarning({ ...behind, board_writer: null }), null);
expect(W, "no meta", behindWarning(null), null);
if (!/behindWarning\(/.test(app) || !/role="alert"/.test(app)) {
  problems.push([W, "App.tsx: the warning is not rendered as an alert"]);
}

if (problems.length) {
  for (const [t, p] of problems) console.error(`${t} — ${p}`);
  process.exit(1);
}
console.log(`${H} — the header label shows version and commit.`);
console.log(`${W} — the warning names both versions and both commits, and only when the writer is newer.`);
