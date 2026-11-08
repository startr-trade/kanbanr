#!/usr/bin/env node
//
// web::live_ticks_settle_before_refetch (DEF-022): the Review page waits for a burst of board
// changes to settle and refetches once, not once per change. The same timer the page's live hook
// uses is checked here, without React.
//
// Usage: npm run check:live-settle
import { settle } from "../src/live.ts";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const problems = [];

// Five calls 50 ms apart fire once, after the quiet period.
let fired = 0;
const burst = settle(() => fired++, 200);
for (let i = 0; i < 5; i++) {
  burst.call();
  await sleep(50);
}
if (fired !== 0) problems.push(`fired ${fired} time(s) during the burst, expected 0`);
await sleep(300);
if (fired !== 1) problems.push(`fired ${fired} time(s) after the burst settled, expected 1`);

// cancel() fires none.
fired = 0;
const cancelled = settle(() => fired++, 100);
cancelled.call();
cancelled.cancel();
await sleep(200);
if (fired !== 0) problems.push(`fired ${fired} time(s) after cancel(), expected 0`);

// ms = 0 keeps today's behaviour: every call fires at once.
fired = 0;
const now = settle(() => fired++, 0);
now.call();
now.call();
now.call();
if (fired !== 3) problems.push(`ms = 0 fired ${fired} time(s) for 3 calls, expected 3`);

if (problems.length) {
  for (const p of problems) console.error(`web::live_ticks_settle_before_refetch — ${p}`);
  process.exit(1);
}
console.log("web::live_ticks_settle_before_refetch — ok");
