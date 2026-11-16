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

const label = `render.mjs renders ${name} (${file})`;
if (problems.length) {
  for (const p of problems) console.error(`check:ui-render — ${label} — ${p}`);
  process.exit(1);
}
console.log(`check:ui-render — ${label} with role=status — ok`);
