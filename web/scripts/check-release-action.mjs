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
  confirm: false,
  ships: false,
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
  ships: true,
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

// FTR-002: an unmet gate_passed offers recording a result for the release tip, never reading one.
const ungated = {
  ...cut,
  ready: false,
  conditions: [
    cut.conditions[0],
    {
      check: "gate_passed",
      stage: "production",
      holds: false,
      detail: "ci has no recorded result for abc1234",
      fix: "look at your CI for abc1234, then record its verdict: kanbanr gate record <gate> --passed --commit <sha>",
    },
  ],
  missing: ["look at your CI for abc1234, then record its verdict"],
};
a = releaseAction(ungated, true);
expect("gate missing: disabled, names the result", !a.enabled && a.missing.some((m) => m.text.startsWith("gate_passed: ci has no recorded result")), a);
expect("gate missing: the form is for the release tip", a.gate?.commit === "abc1234def" && a.gate.record.includes("kanbanr gate record <gate> --passed --commit abc1234def"), a);
a = releaseAction({ ...ungated, repo: false }, true);
expect("gate missing, no repo: still shown with the tip", a.missing.some((m) => m.text.startsWith("gate_passed:")), a);
a = releaseAction({ ...ungated, release_tip: "" }, true);
expect("gate missing, no tip: no form", a.gate === undefined, a);
a = releaseAction(cut, true);
expect("no gate condition: no form", a.gate === undefined, a);

// FTR-026: a stage after the tag is a confirmation, offered only after a passed result for the tag.
const confirm = {
  ...base,
  from: "tag",
  to: "staging",
  confirm: true,
  ships: true,
  button: "Confirm staging: ship v0.3.0",
  conditions: [{ check: "tag_passed", stage: "staging", holds: true, detail: "ci passed on abc1234 (recorded by Ada L)" }],
  steps: [],
  preview: [{ code: "FEAT-1", ships: true }],
  tag: { commit: "abc1234def5678", gate: "ci", state: "passed", result: { passed: true, by: "Ada L" } },
};
a = releaseAction(confirm, true);
expect("confirm after a pass: the button confirms and is enabled", a.confirm && a.enabled && a.label === "Confirm staging: ship v0.3.0" && a.secondary.length === 0, a);
const red = {
  ...confirm,
  ready: false,
  conditions: [{ check: "tag_passed", stage: "staging", holds: false, detail: "ci failed on abc1234", fix: "start a patch release" }],
  missing: ["start a patch release"],
  tag: { commit: "abc1234def5678", gate: "ci", state: "failed", result: { passed: false, by: "Ada L", note: "two red" } },
};
a = releaseAction(red, true);
expect("confirm after a failure: not enabled, run again or patch", !a.enabled && a.secondary.join() === "runAgain,startPatch" && a.missing.some((m) => m.text.startsWith("tag_passed")), a);
const none = { ...red, ready: true, conditions: [], missing: [], tag: { commit: "abc1234def5678", gate: "ci", state: "none" } };
a = releaseAction(none, true);
expect("confirm with no result: not enabled even if the plan has no condition", !a.enabled && a.missing.length === 1 && a.secondary.length === 0, a);
a = releaseAction({ ...none, tag: { commit: "abc1234def5678", gate: "", state: "none" } }, true);
expect("confirm with no gate to judge by: enabled", a.enabled, a);

const inert = { ...base, to: "test", button: "Promote to test", conditions: [], steps: [] };
a = releaseAction(inert, true);
expect(
  "a promotion to an inert stage says nothing happens",
  a.button && a.enabled && a.note === undefined && a.does.includes("Nothing happens when you promote to test: it only records that the release passed it."),
  a,
);
a = releaseAction({ ...none, conditions: [], steps: [] }, true);
expect("a confirmation stage gets no such line", a.does.every((d) => !d.startsWith("Nothing happens")), a);
a = releaseAction(base, true);
expect("a promotion with a step gets no such line", a.does.every((d) => !d.startsWith("Nothing happens")), a);

// DEF-029: a promotion held for a merged item that would carry over says so, also with no repository.
const heldMerged = {
  ...cut,
  ready: false,
  conditions: [
    {
      check: "merged_items_ship",
      stage: "production",
      holds: false,
      detail: "FEAT-1 is merged into v0.3.0 but would carry over: needs sign-off release",
      fix: "FEAT-1: give what it lacks on the Review page (kanbanr check FEAT-1 lists it), or plan it into another release",
    },
  ],
  missing: ["FEAT-1: give what it lacks on the Review page (kanbanr check FEAT-1 lists it), or plan it into another release"],
};
a = releaseAction(heldMerged, true);
expect(
  "held for a merged item: disabled, names the item and the Review page",
  !a.enabled && a.missing.some((m) => m.text.startsWith("merged_items_ship: FEAT-1") && m.fix.includes("Review page")),
  a,
);
a = releaseAction({ ...heldMerged, repo: false }, true);
expect(
  "held for a merged item, no repo: still named beside the repository line",
  a.missing[0]?.text === NO_REPO && a.missing.some((m) => m.text.startsWith("merged_items_ship: FEAT-1")),
  a,
);

// DEF-031: a release sent back by a merge shows the item and the steps from candidate, until cut.
const returned = {
  item: "DEF-030",
  by: "Ada L",
  at: "2026-11-07T10:00:00Z",
  from: "main",
  steps: ["Update the release notes", "Promote to main", "Push main and wait for CI", "Cut v0.3.0"],
};
a = releaseAction({ ...base, from: "candidate", returned }, true);
expect(
  "web::release_returned_banner: names the item and the steps in order",
  a.returned?.item === "DEF-030" && a.returned.from === "main" && a.returned.steps.join("|") === returned.steps.join("|"),
  a,
);
expect("web::release_returned_banner: the button stays the ordinary next step", a.label === "Promote to main" && a.enabled, a);
a = releaseAction(base, true);
expect("web::release_returned_banner: no banner for a release that was not sent back", a.returned === undefined, a);
a = releaseAction({ ...base, returned }, false);
expect("web::release_returned_banner: shown on a read-only monitor too", a.returned?.item === "DEF-030" && !a.button, a);

if (problems.length) {
  for (const p of problems) console.error(`web::release_button_says_what_it_does — ${p}`);
  process.exit(1);
}
console.log(
  "web::release_button_says_what_it_does — one button per release, named for its step, enabled only when ready, saying what is missing otherwise; none for an empty release.",
);
