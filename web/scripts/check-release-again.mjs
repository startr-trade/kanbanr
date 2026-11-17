#!/usr/bin/env node
//
// web::the_releases_page_offers_promote_to_candidate_again (DEF-057 R-1): a release whose plan
// carries `again` shows a second, secondary button named for what the plan says (never a literal:
// the label is the plan's own `button`), which promotes to the stage it is at. The primary button
// is not touched by it. A read-only monitor shows the command, not a button.
//
// Usage: npm run check:release-again
import { readFileSync } from "node:fs";
import { releaseAction } from "../src/releaseAction.ts";

const base = {
  version: "v0.3.0",
  from: "candidate",
  to: "main",
  direction: "forward",
  cut: false,
  confirm: false,
  ships: false,
  button: "Promote v0.3.0 to main",
  conditions: [{ check: "items_finished", stage: "main", holds: true, detail: "2 planned item(s), all finished" }],
  steps: [{ action: "move_main", stage: "main", does: "fast-forwards main to release/v0.3.0 (abc1234)", noop: false }],
  release_tip: "abc1234def",
  missing: [],
  ready: true,
  needs_reason: false,
  repo: true,
  empty: false,
};
const offer = {
  stage: "candidate",
  button: "Promote v0.3.0 to candidate again",
  does: ["commits the version 0.3.0 to release/v0.3.0"],
  ready: true,
  missing: [],
};
const withAgain = { ...base, again: offer };

const problems = [];
const expect = (name, cond, got) => {
  if (!cond) problems.push(`${name}: ${JSON.stringify(got)}`);
};

const plain = releaseAction(base, true);
expect("no offer, no second button", plain.again === undefined, plain);

let a = releaseAction(withAgain, true);
expect("the label is the plan's button", a.again?.label === withAgain.again.button, a.again);
expect("enabled when writable and ready", a.again?.enabled === true, a.again);
expect("carries what it does", a.again?.does[0] === offer.does[0], a.again);
expect("promotes to the stage it is at", a.again?.stage === "candidate", a.again);
expect("the command names the stage", a.again?.command === "kanbanr release promote v0.3.0 candidate", a.again);
expect("the primary label is unchanged", a.label === plain.label && a.enabled === plain.enabled, a);

a = releaseAction(withAgain, false);
expect("read-only: disabled", a.again?.enabled === false, a.again);

a = releaseAction({ ...base, again: { ...offer, ready: false, missing: ["release/v0.3.0 is checked out in a worktree"] } }, true);
expect("not ready: disabled", a.again?.enabled === false, a.again);
expect("not ready: says what is missing", a.again?.missing[0]?.text === "release/v0.3.0 is checked out in a worktree", a.again);

a = releaseAction({ ...withAgain, repo: false }, true);
expect("no repository: disabled", a.again?.enabled === false, a.again);

// A renamed label passes through (DEF-034).
a = releaseAction({ ...base, again: { ...offer, button: "Promote v0.3.0 to candidate again" } }, true);
expect("a renamed label passes through", a.again?.label === "Promote v0.3.0 to candidate again", a.again);

const page = readFileSync(new URL("../src/pages/releases/ReleaseActions.tsx", import.meta.url), "utf8");
expect("the page renders action.again", page.includes("action.again") && page.includes("release-again"), "releases/ReleaseActions.tsx");
expect(
  "the button promotes to the stage, pinned to the tip",
  /onPromote\(release,\s*action\.again!?\.stage,\s*plan\.data!?\.release_tip\)/.test(page),
  "releases/ReleaseActions.tsx",
);
const types = readFileSync(new URL("../src/types.ts", import.meta.url), "utf8");
expect("types.ts declares again?", /again\?:\s*AgainOffer/.test(types), "types.ts");

if (problems.length) {
  for (const p of problems) console.error(`web::the_releases_page_offers_promote_to_candidate_again — ${p}`);
  process.exit(1);
}
console.log("web::the_releases_page_offers_promote_to_candidate_again — ok");
