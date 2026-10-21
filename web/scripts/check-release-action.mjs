#!/usr/bin/env node
//
// web::release_button_says_what_it_does (FEAT-212 R-12, R-17): each unshipped release has one
// button named for its next step, with what it will do beside it and, for the cut, its preview;
// it is enabled only when the plan is ready, and otherwise the page says what is missing. A
// read-only monitor offers no button, only the CLI command; a release with nothing planned into it
// offers none at all. The same function the page renders with is checked here, case by case.
//
// Usage: npm run check:release-action
import { releaseAction, NO_REPO } from "../src/releaseAction.ts";

const base = {
  version: "v0.3.0",
  from: "candidate",
  to: "main",
  direction: "forward",
  cut: false,
  button: "Promote to main",
  conditions: [{ check: "items_finished", stage: "main", holds: true, detail: "2 planned item(s), all finished" }],
  steps: [{ action: "move_main", stage: "main", does: "fast-forwards main to release/v0.3.0 (abc1234)", noop: false }],
  release_tip: "abc1234def",
  missing: [],
  ready: true,
  needs_reason: false,
  after: "push main, wait for CI, then Cut v0.3.0",
  repo: true,
  empty: false,
};
const cut = {
  ...base,
  from: "main",
  to: "production",
  cut: true,
  button: "Cut v0.3.0",
  conditions: [
    { check: "main_at_release", stage: "production", holds: true, detail: "main is at release/v0.3.0" },
    { check: "pushed", stage: "production", holds: true, detail: "abc1234 is on origin/main" },
  ],
  steps: [{ action: "tag", stage: "production", does: 'tags abc1234 "Release v0.3.0" as v0.3.0', noop: false }],
  preview: [
    { code: "FEAT-1", ships: true },
    { code: "FEAT-2", ships: false, why: "2 task(s) still open" },
  ],
  tag_commit: { sha: "abc1234def5678", subject: "Release v0.3.0: version, changelog and notes" },
};
const unpushed = {
  ...cut,
  ready: false,
  conditions: [
    cut.conditions[0],
    {
      check: "pushed",
      stage: "production",
      holds: false,
      detail: "abc1234 is not on origin/main",
      fix: "push main (`git push origin main`), then wait for CI",
    },
  ],
  missing: ["push main (`git push origin main`), then wait for CI"],
};
const noRepo = {
  ...unpushed,
  repo: false,
  conditions: [
    { check: "main_at_release", stage: "production", holds: false, detail: "needs the code repository", fix: "x" },
    { check: "pushed", stage: "production", holds: false, detail: "needs the code repository", fix: "x" },
  ],
  steps: [{ action: "tag", stage: "production", does: "tags the release tip", noop: false, blocked: "needs the code repository" }],
};
const empty = {
  ...base,
  from: "",
  to: "candidate",
  button: "Promote to candidate",
  conditions: [],
  steps: [],
  ready: false,
  empty: true,
  missing: ["v0.3.0 has no items: plan some in with `kanbanr release plan v0.3.0 <CODE>…`"],
};

const problems = [];
const expect = (name, cond, got) => {
  if (!cond) problems.push(`${name}: ${JSON.stringify(got)}`);
};

let a = releaseAction(base, true);
expect("middle stage: one button named for the step", a.button && a.label === "Promote to main" && a.enabled, a);
expect("middle stage: says what it does", a.does.join() === "fast-forwards main to release/v0.3.0 (abc1234)", a);
expect("middle stage: no preview, nothing missing", !a.preview && a.missing.length === 0, a);

a = releaseAction(cut, true);
expect("cut ready: the button says Cut", a.label === "Cut v0.3.0" && a.enabled, a);
expect("cut ready: preview inline", a.preview?.tag.startsWith("abc1234 Release v0.3.0") && a.preview.ships.join() === "FEAT-1" && a.preview.carries[0]?.code === "FEAT-2", a);
expect("cut ready: says it tags", a.does[0]?.includes("tags abc1234"), a);

a = releaseAction(unpushed, true);
expect("cut missing pushed: disabled", a.button && !a.enabled && a.label === "Cut v0.3.0", a);
expect(
  "cut missing pushed: names the push",
  a.missing.some((m) => m.fix === "push main (`git push origin main`), then wait for CI"),
  a,
);

a = releaseAction(cut, false);
expect("read-only: no button, the CLI command", !a.button && !a.enabled && a.command === "kanbanr release promote v0.3.0 production", a);
expect("read-only: still the plan's lines", a.does.length === 1 && a.preview != null, a);

a = releaseAction(noRepo, true);
expect("no repo: disabled", !a.enabled, a);
expect("no repo: says the monitor was not started in the project folder", a.missing[0]?.text === NO_REPO && a.missing[0].fix.includes("kanbanr release promote v0.3.0"), a);

a = releaseAction(empty, true);
expect("empty release: no button", !a.button && !a.enabled && a.label === "", a);
expect("empty release: only the sentence", a.note === empty.missing[0] && a.command === "kanbanr release plan v0.3.0 <CODE>…", a);

if (problems.length) {
  for (const p of problems) console.error(`web::release_button_says_what_it_does — ${p}`);
  process.exit(1);
}
console.log(
  "web::release_button_says_what_it_does — one button per release, named for its step, enabled only when ready, saying what is missing otherwise; none for an empty release.",
);
