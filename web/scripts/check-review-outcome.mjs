#!/usr/bin/env node
//
// A prepared answer shows its outcome once (DEF-090).
//
// web::check_prepared_panel_sends_one_request_per_click_until_it_returns (R-2): a press while a
// call is in flight is dropped, never queued, so a second click sends no second request.
//
// Usage: npm run check:review-outcome
import { guardInFlight } from "../src/inFlight.ts";

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
  return problems;
}

const failures = [];
const r2 = await checkInFlight();
if (r2.length) {
  for (const p of r2) console.error(`${R2} — ${p}`);
  failures.push(R2);
} else {
  console.log(`${R2} — a press while a call is in flight sends nothing; the next press after it returns does.`);
}

if (failures.length) process.exit(1);
