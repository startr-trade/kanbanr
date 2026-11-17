#!/usr/bin/env node
//
// web::check_home_groups_projects_by_board (FTR-040 R-9) and
// web::check_feature_page_marks_a_dependency_on_another_board_external (R-5): Home groups the
// projects by board in the server's order and keeps an empty board; a dependency on a project held
// by another board is external, and the pages use the functions checked here.
//
// Usage: npm run check:boards
import { readFileSync } from "node:fs";
import { externalBoard, groupByBoard } from "../src/boards.ts";

const problems = [];
const expect = (name, got, want) => {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) problems.push(`${name}: got ${g}, wanted ${w}`);
};

const a = { name: "a", projects: ["alpha"] };
const b = { name: "b", projects: ["beta"] };
const empty = { name: "c", projects: [] };
const projects = [{ id: "alpha", board: "a" }, { id: "beta", board: "b" }];
const names = (groups) => groups.map((g) => [g.board?.name ?? null, g.projects.map((p) => p.id)]);

// R-9
expect("no boards: one group, no board", names(groupByBoard(projects, [])), [[null, ["alpha", "beta"]]]);
expect("one board: one group, no board", names(groupByBoard(projects, [a])), [[null, ["alpha", "beta"]]]);
expect("two boards: groups in board order", names(groupByBoard(projects, [b, a])), [["b", ["beta"]], ["a", ["alpha"]]]);
expect("an empty board keeps its group", names(groupByBoard(projects, [a, b, empty])), [
  ["a", ["alpha"]],
  ["b", ["beta"]],
  ["c", []],
]);

// R-5
const boards = [a, b];
expect("another board's project is external", externalBoard("beta:FTR-001", "alpha", boards), "b");
expect("the same board is not", externalBoard("alpha:FTR-002", "alpha", boards), null);
expect("a bare code is not", externalBoard("FTR-001", "alpha", boards), null);
expect("a project on no board is not", externalBoard("zed:FTR-001", "alpha", boards), null);
expect("no boards: nothing is external", externalBoard("beta:FTR-001", "alpha", []), null);
expect("splits on the first colon", externalBoard("beta:FTR:1", "alpha", boards), "b");

const read = (f) => readFileSync(new URL(`../src/pages/${f}`, import.meta.url), "utf8");
const home = read("HomePage.tsx");
if (!/groupByBoard\(/.test(home)) problems.push("HomePage.tsx does not call groupByBoard");
if (!/<h2>\{g\.board\.name\}<\/h2>/.test(home)) problems.push("HomePage.tsx: no level-2 heading per board");
if (!/last changed /.test(home)) problems.push("HomePage.tsx: no 'last changed' line under a board's heading");
const feature = read("FeaturePage.tsx");
if (!/externalBoard\(/.test(feature)) problems.push("FeaturePage.tsx does not call externalBoard");
if (!/not resolved here\)/.test(feature)) problems.push("FeaturePage.tsx: no '(board <name>, not resolved here)' note");
const portfolio = read("PortfolioPage.tsx");
if (!/get\("board"\)/.test(portfolio)) problems.push("PortfolioPage.tsx does not read ?board");

if (problems.length) {
  for (const p of problems) console.error(`web::check_boards — ${p}`);
  process.exit(1);
}
console.log("web::check_home_groups_projects_by_board — one group for zero or one boards, board order for several, an empty board kept; Home uses groupByBoard.");
console.log("web::check_feature_page_marks_a_dependency_on_another_board_external — another board's project is external, same-board and bare codes are not; the item page uses externalBoard.");
