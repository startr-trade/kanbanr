#!/usr/bin/env node
//
// A prepared answer shows its outcome once (DEF-090).
//
// web::check_review_page_shows_a_rejected_outcome_and_hides_the_form (R-1): once a call on the
// Review page returns, any outcome, a rejection or an error included, replaces the form; a failed
// call offers "Try again", which brings the form back.
//
// web::check_prepared_panel_sends_one_request_per_click_until_it_returns (R-2): a press while a
// call is in flight is dropped, never queued, so a second click sends no second request.
//
// The rendered case (a ManualCheck with a rejected outcome has no Reject button) joins R-1 once
// scripts/render.mjs exists; until then the page's source is read.
//
// Usage: npm run check:review-outcome
import { readFileSync } from "node:fs";
import { guardInFlight } from "../src/inFlight.ts";
import { outcomeLine } from "../src/manualTests.ts";

const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");

const R1 = "web::check_review_page_shows_a_rejected_outcome_and_hides_the_form";
const R2 = "web::check_prepared_panel_sends_one_request_per_click_until_it_returns";

const expect = (problems, name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

// R-2: guardInFlight, with a fake handler whose promise is held open.
async function checkInFlight() {
  const problems = [];
  let calls = 0;
  let release = () => {};
  const held = guardInFlight(() => {
    calls += 1;
    return new Promise((resolve) => {
      release = resolve;
    });
  });
  expect(problems, "nothing is in flight before the first press", held.busy(), false);
  const first = held.run();
  held.run();
  held.run();
  expect(problems, "three presses while the call is open send one call", calls, 1);
  expect(problems, "busy while the call is open", held.busy(), true);
  release();
  await first;
  expect(problems, "not busy once the call returns", held.busy(), false);
  const next = held.run();
  expect(problems, "a press after the call returns sends again", calls, 2);
  release();
  await next;

  // A failed call ends the flight too; run passes the error on rather than swallowing it.
  const failing = guardInFlight(() => Promise.reject(new Error("refused")));
  let passedOn = "";
  await failing.run().catch((e) => {
    passedOn = e.message;
  });
  expect(problems, "a failed call's error is passed on", passedOn, "refused");
  expect(problems, "a failed call is not left busy", failing.busy(), false);

  // A synchronous handler counts as settled at once.
  let syncCalls = 0;
  const sync = guardInFlight(() => {
    syncCalls += 1;
  });
  sync.run();
  expect(problems, "a synchronous handler is never left busy", sync.busy(), false);
  sync.run();
  expect(problems, "a synchronous handler runs on every press", syncCalls, 2);

  // PreparedPanel: one guard per panel; the primary button and the second Reject go through it
  // and are disabled while its call is in flight.
  const panel = read("../src/components/PreparedPanel.tsx");
  const has = (name, re) => {
    if (!re.test(panel)) problems.push(`PreparedPanel.tsx: ${name}`);
  };
  has("does not import guardInFlight", /import \{ guardInFlight \} from "\.\.\/inFlight"/);
  has("keeps no guard per panel in a ref", /useRef<[\s\S]*?\(null\)[\s\S]*guard\.current = guardInFlight\(/);
  has("onConfirm is still typed to return void, so its promise is not awaited", /onConfirm: \(edits: Record<string, unknown>\) => unknown;/);
  has("onReject is still typed to return void, so its promise is not awaited", /onReject: \(reason: string\) => unknown;/);
  has("the primary button does not go through the guard", /onClick=\{\(\) => once\(\(\) => onConfirm\(/);
  has("the second Reject does not go through the guard", /onClick=\{\(\) => once\(\(\) => onReject\(/);
  has("the primary button is not disabled while in flight", /disabled=\{busy \|\| inFlight \|\| mode === "reject"\}\s*onClick=\{\(\) => once\(\(\) => onConfirm\(/);
  has("the second Reject is not disabled while in flight", /disabled=\{busy \|\| inFlight\} onClick=\{\(\) => once\(\(\) => onReject\(/);
  if (/onClick=\{\(\) => on(Confirm|Reject)\(/.test(panel)) {
    problems.push("PreparedPanel.tsx: a button calls onConfirm or onReject without the guard");
  }
  return problems;
}

// R-1: the outcome's line, and the page's source.
function checkOutcome() {
  const problems = [];
  expect(
    problems,
    "a rejection's line",
    outcomeLine({ ok: true, text: "Rejected" }),
    "Rejected — it leaves this list on the next refresh",
  );
  expect(problems, "an error's line is the error alone", outcomeLine({ ok: false, text: "409 Conflict: already answered" }), "409 Conflict: already answered");

  const page = read("../src/pages/ReviewPage.tsx");
  const part = (from, to) => {
    const a = page.indexOf(from);
    const b = page.indexOf(to, a);
    if (a < 0 || b < 0) {
      problems.push(`ReviewPage.tsx: no ${from.trim()} section`);
      return "";
    }
    return page.slice(a, b);
  };
  const parts = {
    ManualCheck: part("function ManualCheck(", "/** One proposed decision"),
    DecisionBrief: part("function DecisionBrief(", "function useDiagramCheck("),
    Brief: part("function Brief(", "function partText("),
  };
  const finishedBy = { ManualCheck: "finished", DecisionBrief: "decided", Brief: "approved" };
  for (const [name, src] of Object.entries(parts)) {
    if (!new RegExp(`const ${finishedBy[name]} = outcome != null;`).test(src)) {
      problems.push(`ReviewPage.tsx: ${name} does not decide finished as outcome != null`);
    }
    if (/startsWith\(/.test(src) || /(?<![.\w])outcome === "/.test(src) || /\(outcome \?\? ""\)/.test(src)) {
      problems.push(`ReviewPage.tsx: ${name} still decides finished by the outcome's words`);
    }
    if (!/<OutcomeChip outcome=\{outcome\} onRetry=\{onRetry\} \/>/.test(src)) {
      problems.push(`ReviewPage.tsx: ${name} does not show the outcome where the form was`);
    }
  }
  if (!/outcome\?: Outcome;[\s\S]*outcome\?: Outcome;[\s\S]*outcome\?: Outcome;/.test(page)) {
    problems.push("ReviewPage.tsx: the outcome is not an Outcome in ManualCheck, DecisionBrief and Brief");
  }
  if (!/useState<Record<string, Outcome>>\(\{\}\)/.test(page)) problems.push("ReviewPage.tsx: done is not a Record<string, Outcome>");

  const handler = (name) => part(`const ${name} = async`, "};\n");
  if (!/\{ ok: true, text: "Rejected" \}/.test(handler("rejectManual"))) {
    problems.push('ReviewPage.tsx: rejectManual does not write {ok: true, text: "Rejected"}');
  }
  if (!/\{ ok: true, text: "Rejected what Claude prepared; ask Claude to prepare it again" \}/.test(handler("rejectPrepared"))) {
    problems.push("ReviewPage.tsx: rejectPrepared does not write its rejected outcome");
  }
  if (/setDone\(\(d\) => \(\{ \.\.\.d, \[[^\]]+\]: (e instanceof|"|`|alreadyOr|plannedText)/.test(page)) {
    problems.push("ReviewPage.tsx: a setDone still writes a bare string");
  }

  // The outcome and "Try again": the chip is done or warn by ok, and the link clears the key.
  const chip = part("function OutcomeChip(", "\n}\n");
  if (!/chip \$\{outcome\.ok \? "done" : "warn"\}/.test(chip) || !/outcomeLine\(outcome\)/.test(chip)) {
    problems.push("ReviewPage.tsx: OutcomeChip does not show outcomeLine as a done or warn chip");
  }
  if (!/outcome\.ok \? null : [\s\S]*onRetry\(\)[\s\S]*Try again/.test(chip)) {
    problems.push('ReviewPage.tsx: a failed outcome offers no "Try again"');
  }
  if (!/const retry = \(key: string\) =>\s*setDone\(\(d\) => \{[\s\S]*?delete rest\[key\];/.test(page)) {
    problems.push('ReviewPage.tsx: "Try again" does not clear the outcome, so the form would not return');
  }
  return problems;
}

const failures = [];
const r1 = checkOutcome();
if (r1.length) {
  for (const p of r1) console.error(`${R1} — ${p}`);
  failures.push(R1);
} else {
  console.log(`${R1} — any outcome, a rejection included, replaces the form; a failed call offers Try again.`);
}
const r2 = await checkInFlight();
if (r2.length) {
  for (const p of r2) console.error(`${R2} — ${p}`);
  failures.push(R2);
} else {
  console.log(`${R2} — a press while a call is in flight sends nothing; the next press after it returns does.`);
}

if (failures.length) process.exit(1);
