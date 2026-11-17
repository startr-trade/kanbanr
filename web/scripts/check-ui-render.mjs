#!/usr/bin/env node
//
// render.mjs renders a component through Vite's SSR loader (FTR-046 T2): the self-check the kit's
// markup checks stand on. It renders a loading state and asserts `role=status`.
//
// The kit's `LoadingState` (web/src/ui/States.tsx) lands in FTR-046 T6, which completes
// web::check_states_are_distinct_and_empty_offers_a_sentence; until then this renders the monitor's
// existing `Loading` (components/bits.tsx), which has the same role.
//
// Usage: npm run check:ui-render
import { existsSync } from "node:fs";
import { closeRender, render } from "./lib/render.mjs";

const problems = [];
const kit = existsSync(new URL("../src/ui/States.tsx", import.meta.url).pathname);
const [file, name] = kit ? ["src/ui/States.tsx", "LoadingState"] : ["src/components/bits.tsx", "Loading"];

try {
  const { html, document } = await render(file, name, {});
  const status = document.querySelectorAll('[role="status"]');
  if (status.length !== 1) problems.push(`${name} rendered ${status.length} role=status element(s), expected 1: ${html}`);
  else if (!/Loading/.test(status[0].textContent ?? "")) problems.push(`${name}'s status says "${status[0].textContent}", expected "Loading…"`);

  // The server is reused: a second render in the same process works and sees the given URL.
  const again = await render(file, name, {}, { url: "/p/x?tab=a" });
  if (again.html !== html) problems.push(`a second render differs from the first: ${again.html}`);

  // A missing export is an error, not an empty render.
  let threw = false;
  try {
    await render(file, "NoSuchExport", {});
  } catch {
    threw = true;
  }
  if (!threw) problems.push("rendering a missing export did not throw");
} catch (e) {
  problems.push(`render failed: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

// DEF-106 R-2: a read that is still going says how long it has been, once it has run a second.
let working = true;
{
  const wp = [];
  try {
    const { elapsedText, WORKING_AFTER_MS } = await import("../src/ui/working.ts");
    const want = (what, got, exp) => {
      if (got !== exp) wp.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(exp)}`);
    };
    want("elapsedText 0", elapsedText(1000, 1000), "0 s");
    want("elapsedText 59", elapsedText(0, 59_000), "59 s");
    want("elapsedText 119", elapsedText(0, 119_900), "119 s");
    want("elapsedText 125", elapsedText(0, 125_000), "2 min 5 s");
    want("WORKING_AFTER_MS", WORKING_AFTER_MS, 1000);
    const f = "src/ui/States.tsx";
    const none = await render(f, "WorkingState", { label: "Counting…", since: null });
    want("null since renders nothing", none.document.querySelectorAll('[role="status"]').length, 0);
    const early = await render(f, "WorkingState", { label: "Counting…", since: Date.now() - 200 });
    const earlyEl = early.document.querySelector('[role="status"]');
    want("before a second: role=status", Boolean(earlyEl), true);
    want("before a second: only the label", earlyEl?.textContent, "Counting…");
    const late = await render(f, "WorkingState", { label: "Counting…", since: Date.now() - 12_000 });
    const lateEl = late.document.querySelector('[role="status"]');
    if (!/^Counting… · 1[23] s/.test(lateEl?.textContent ?? "")) wp.push(`after a second: "${lateEl?.textContent}" does not show the label and the seconds`);
    const secs = lateEl?.querySelector('[aria-hidden="true"]');
    if (!secs || !/1[23] s/.test(secs.textContent ?? "")) wp.push("the seconds are not in an aria-hidden span");
    if (!lateEl?.querySelector(".ui-sr-only")) wp.push("there is no visually hidden copy for a screen reader");
    const mins = await render(f, "WorkingState", { label: "Counting…", since: Date.now() - 125_000 });
    if (!/2 min 5 s|2 min 6 s/.test(mins.document.querySelector('[role="status"]')?.textContent ?? "")) wp.push("after two minutes the text is not in minutes");
  } catch (e) {
    wp.push(`render failed: ${e?.stack ?? e}`);
  } finally {
    await closeRender();
  }
  if (wp.length) {
    for (const p of wp) console.error(`web::check_still_working_shows_elapsed_seconds — ${p}`);
    working = false;
  } else console.log("web::check_still_working_shows_elapsed_seconds — ok");
}
if (!working) process.exit(1);

const label = `render.mjs renders ${name} (${file})`;
if (problems.length) {
  for (const p of problems) console.error(`check:ui-render — ${label} — ${p}`);
  process.exit(1);
}
console.log(`check:ui-render — ${label} with role=status — ok`);
