#!/usr/bin/env node
//
// web::check_pages_never_fetch_the_whole_board (DEF-081 R-1): the shell, the link index, the
// components and the pages read the compact index and their own slices, never the whole board
// (`GET /projects/{p}`, about 4 MB on this project's board). A file fails when it calls
// `api.getProject(` or spells a `/projects/${…}` URL with nothing after the project.
//
// Usage: npm run check:whole-board
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const NAME = "web::check_pages_never_fetch_the_whole_board";
const src = join(dirname(fileURLToPath(import.meta.url)), "../src");

/**
 * Pages still on the whole board until the item named beside each rebuilds or removes them; that
 * item removes its entry.
 */
const EXEMPT = new Map([
  ["pages/ReviewPage.tsx", "rebuilt by FTR-038"],
  ["pages/ReleasesPage.tsx", "rebuilt by FTR-041"],
  ["pages/SchedulePage.tsx", "removed by FTR-043"],
  ["pages/GanttPage.tsx", "removed by FTR-043"],
]);

/** What a whole-board read looks like in source. */
const PATTERNS = [
  { re: /\bapi\.getProject\s*\(/, what: "calls api.getProject(" },
  // A template URL that ends right after the project segment: `/projects/${p}` then a quote,
  // a backtick, or a query string.
  { re: /\/projects\/\$\{[^}]*\}(?=[`'"?])/, what: "builds a /projects/${…} URL with no sub-path" },
];

function files(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...files(path));
    else if (/\.(ts|tsx)$/.test(name)) out.push(path);
  }
  return out;
}

/** The problems in `text`, the source of `rel`. Pure, so the self-test below can feed it. */
export function wholeBoardReads(rel, text) {
  const problems = [];
  const lines = text.split("\n");
  lines.forEach((line, i) => {
    for (const { re, what } of PATTERNS) {
      if (re.test(line)) problems.push(`${rel}:${i + 1} ${what}`);
    }
  });
  return problems;
}

const problems = [];

// The check must catch what it is for: a page spelling either form fails.
const caught = [
  wholeBoardReads("x.tsx", "useAsync(() => api.getProject(project), [project]);"),
  wholeBoardReads("x.tsx", "fetch(apiUrl(`/projects/${enc(p)}`));"),
];
if (caught.some((p) => p.length !== 1)) problems.push("the self-test did not catch a whole-board read");
if (wholeBoardReads("x.tsx", "apiUrl(`/projects/${enc(p)}/index`)").length !== 0) {
  problems.push("the self-test flagged a sub-path read as the whole board");
}

const scanned = [
  join(src, "App.tsx"),
  join(src, "useBoardIndex.ts"),
  ...files(join(src, "components")),
  ...files(join(src, "pages")),
];
for (const path of scanned) {
  const rel = relative(src, path).split("\\").join("/");
  if (EXEMPT.has(rel)) continue;
  problems.push(...wholeBoardReads(rel, readFileSync(path, "utf8")));
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — ok`);
