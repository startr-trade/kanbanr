#!/usr/bin/env node
//
// FTR-057: the Review page groups manual checks by item and puts the release being shipped first.
// The pure grouping the page uses (manualGroups, src/manualTests.ts) is checked case by case:
//   web::check_manual_checks_group_under_their_item (R-1) — one item with three checks gives one
//     group with three rows and count 3; two items give two groups.
//   web::check_manual_checks_put_the_release_being_shipped_first (R-2) — a holding release's item
//     comes before others, under its own section, in the order the server names the releases;
//     `other` holds the rest; a check not due yet is kept apart.
//   web::check_manual_checks_mark_a_check_that_blocks_promote (R-3) — a row that holds a promotion
//     is counted on its item, and that item sorts first within its section.
//
// Usage: npm run check:manual-groups
import { manualGroups } from "../src/manualTests.ts";

const waiting = (code, requirement, test, extra = {}) => ({
  row: {
    code,
    title: `Item ${code}`,
    status: "Completed",
    requirement,
    requirement_text: "THE SYSTEM SHALL do it.",
    test,
    state: "planned",
    checked_rev: "",
    last: null,
    ...extra,
  },
});
const prepared = { id: "p", kind: "manual-test" };

const results = [];
const check = (name, problems) => results.push({ name, problems });
const expect = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};

// R-1: one item with three checks is one group; two items are two groups.
{
  const problems = [];
  const one = manualGroups(
    [
      waiting("SPK-002", "R-1", "manual: one", { release: "v0.9.0" }),
      { ...waiting("SPK-002", "R-2", "manual: two", { release: "v0.9.0" }), prepared },
      waiting("SPK-002", "R-3", "manual: three", { release: "v0.9.0" }),
    ],
    [],
  );
  expect(problems, "groups for one item", one.other.length, 1);
  expect(problems, "its code", one.other[0]?.code, "SPK-002");
  expect(problems, "its rows, in the server's order", one.other[0]?.rows.map((r) => r.row.requirement), ["R-1", "R-2", "R-3"]);
  expect(problems, "its count", one.other[0]?.rows.length, 3);
  expect(problems, "its prepared count", one.other[0]?.prepared, 1);
  expect(problems, "its heading's fields", [one.other[0]?.title, one.other[0]?.status, one.other[0]?.release], ["Item SPK-002", "Completed", "v0.9.0"]);
  const two = manualGroups(
    [waiting("FEAT-150", "R-1", "manual: a"), waiting("FTR-033", "R-1", "manual: b"), waiting("FEAT-150", "R-2", "manual: c")],
    [],
  );
  expect(problems, "groups for two items", two.other.map((g) => [g.code, g.rows.length]), [["FEAT-150", 2], ["FTR-033", 1]]);
  check("web::check_manual_checks_group_under_their_item", problems);
}

// R-2: the release being shipped first.
{
  const problems = [];
  const s = manualGroups(
    [
      waiting("FEAT-001", "R-1", "manual: other", { release: "v0.9.0" }),
      waiting("FEAT-002", "R-1", "manual: none"),
      waiting("FEAT-003", "R-1", "manual: candidate", { release: "v0.8.0" }),
      waiting("FEAT-004", "R-1", "manual: at main", { release: "v0.7.1" }),
      waiting("FEAT-005", "R-1", "manual: later", { release: "v0.8.0", opens_at: "Build" }),
    ],
    ["v0.8.0", "v0.7.1"],
  );
  expect(problems, "holding sections, in the server's order", s.holding.map((h) => h.version), ["v0.8.0", "v0.7.1"]);
  expect(problems, "the candidate's items", s.holding[0]?.items.map((g) => g.code), ["FEAT-003"]);
  expect(problems, "the other release's items", s.holding[1]?.items.map((g) => g.code), ["FEAT-004"]);
  expect(problems, "other items", s.other.map((g) => g.code), ["FEAT-001", "FEAT-002"]);
  expect(problems, "not due yet", s.notDue.map((r) => r.row.code), ["FEAT-005"]);
  const none = manualGroups([waiting("FEAT-001", "R-1", "manual: x", { release: "v0.9.0" })], []);
  expect(problems, "with nothing holding", [none.holding.length, none.other.length], [0, 1]);
  check("web::check_manual_checks_put_the_release_being_shipped_first", problems);
}

// R-3: a check that holds Promote is counted and its item sorts first within its section.
{
  const problems = [];
  const holds = "Promote v0.8.0 to main";
  const s = manualGroups(
    [
      waiting("FEAT-010", "R-1", "manual: free", { release: "v0.8.0" }),
      waiting("FEAT-011", "R-1", "manual: free too", { release: "v0.8.0" }),
      waiting("FEAT-011", "R-2", "manual: blocks", { release: "v0.8.0", holds }),
      waiting("FEAT-012", "R-1", "manual: elsewhere", { release: "v0.9.0" }),
    ],
    ["v0.8.0"],
  );
  expect(problems, "the holding item sorts first", s.holding[0]?.items.map((g) => g.code), ["FEAT-011", "FEAT-010"]);
  expect(problems, "its holds count", s.holding[0]?.items.map((g) => g.holds), [1, 0]);
  expect(problems, "its rows keep their order", s.holding[0]?.items[0]?.rows.map((r) => r.row.holds ?? ""), ["", holds]);
  expect(problems, "an item of another release holds nothing", s.other.map((g) => [g.code, g.holds]), [["FEAT-012", 0]]);
  check("web::check_manual_checks_mark_a_check_that_blocks_promote", problems);
}

let failed = false;
for (const { name, problems } of results) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}
if (failed) process.exit(1);
