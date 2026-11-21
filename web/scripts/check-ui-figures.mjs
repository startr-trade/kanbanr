#!/usr/bin/env node
//
// The UI kit's figures and states (FTR-046):
//   web::check_progress_percentages_sum_to_100 (R-4) — `percentages` rounds by largest remainder so
//     the shares add up, gives zeros for a total of 0, and clamps segments over the total;
//   web::check_pills_and_tiles_carry_text_not_colour_alone (R-4) — a pill whose text does not say
//     its tone gets a hidden word, a warn tile shows "warning", the progress bar is named segment by
//     segment with a visible legend;
//   web::check_states_are_distinct_and_empty_offers_a_sentence (R-6) — loading is role=status, an
//     error role=alert, empty neither; Empty with `ask` shows the sentence and a Copy button.
//
// Usage: npm run check:ui-figures
import { percentages } from "../src/ui/progress.ts";
import { closeRender, render } from "./lib/render.mjs";

function report(name, problems) {
  if (problems.length) {
    for (const p of problems) console.error(`${name} — ${p}`);
    return false;
  }
  console.log(`${name} — ok`);
  return true;
}

const eq = (problems, what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
};

const seg = (...values) => values.map((value, i) => ({ key: `s${i}`, label: `S${i}`, value, tone: "neutral" }));

let ok = true;
let current = "web::check_progress_percentages_sum_to_100";

try {
  // R-4: percentages add up.
  {
    const name = current;
    const problems = [];
    eq(problems, "[1,1,1] of 3", percentages(seg(1, 1, 1), 3), [34, 33, 33]);
    eq(problems, "[1,2] of 3", percentages(seg(1, 2), 3), [33, 67]);
    eq(problems, "[2,1] of 3 (the larger remainder gets the point)", percentages(seg(2, 1), 3), [67, 33]);
    eq(problems, "[1,1,1,1,1,1,1] of 7 sums to 100", percentages(seg(1, 1, 1, 1, 1, 1, 1), 7).reduce((a, b) => a + b, 0), 100);
    eq(problems, "[5,5] of 10", percentages(seg(5, 5), 10), [50, 50]);
    eq(problems, "total 0 gives zeros", percentages(seg(1, 2, 3), 0), [0, 0, 0]);
    eq(problems, "a negative total gives zeros", percentages(seg(1), -4), [0]);
    eq(problems, "no segments", percentages([], 10), []);
    eq(problems, "segments over the total are clamped", percentages(seg(3, 3), 4), [75, 25]);
    eq(problems, "one segment over the total is clamped to 100", percentages(seg(9), 4), [100]);
    eq(problems, "segments short of the total cover only their part", percentages(seg(1, 2), 10), [10, 20]);
    eq(problems, "a negative segment counts as 0", percentages(seg(-2, 4), 4), [0, 100]);
    ok = report(name, problems) && ok;
  }

  // R-4: pills, tiles and bars carry text, not colour alone.
  current = "web::check_pills_and_tiles_carry_text_not_colour_alone";
  {
    const name = current;
    const problems = [];
    const pill = await render("src/ui/SeverityPill.tsx", "SeverityPill", { tone: "error", children: "DEF-066 blocked" });
    const span = pill.document.querySelector("span.ui-pill");
    if (!span) problems.push(`no span.ui-pill: ${pill.html}`);
    else {
      eq(problems, "the error pill's classes", span.className, "ui-pill ui-pill-error");
      eq(problems, "an error pill without the word gets it hidden", span.querySelector(".ui-sr-only")?.textContent, "error ");
      eq(problems, "the error pill reads", span.textContent, "error DEF-066 blocked");
    }
    const said = await render("src/ui/SeverityPill.tsx", "SeverityPill", { tone: "error", children: "2 errors" });
    if (said.document.querySelector(".ui-sr-only")) problems.push("a pill that says its tone gets a second word");
    const warnPill = await render("src/ui/SeverityPill.tsx", "SeverityPill", { tone: "warn", children: "stale", title: "computed 3 days ago" });
    eq(problems, "a warn pill's hidden word", warnPill.document.querySelector(".ui-sr-only")?.textContent, "warning ");
    eq(problems, "a pill's title", warnPill.document.querySelector(".ui-pill")?.getAttribute("title"), "computed 3 days ago");
    const okPill = await render("src/ui/SeverityPill.tsx", "SeverityPill", { tone: "ok", children: "passed" });
    if (okPill.document.querySelector(".ui-sr-only")) problems.push("an ok pill gets a hidden word");

    const tile = await render("src/ui/StatTile.tsx", "StatTile", { label: "blocked items", value: 3, tone: "warn", to: "/p/x/review?blocked=1", sub: "since Monday", spark: [1, 3, 2, 3] });
    const a = tile.document.querySelector("a.ui-tile");
    if (!a) problems.push(`a tile with to is no a.ui-tile: ${tile.html}`);
    else {
      eq(problems, "the tile's link", a.getAttribute("href"), "/p/x/review?blocked=1");
      eq(problems, "a warn tile shows its word before the label", a.querySelector(".ui-tile-label")?.textContent, "warning blocked items");
      eq(problems, "the tile's value", a.querySelector(".ui-tile-value")?.textContent, "3");
      eq(problems, "the tile's name", a.getAttribute("aria-label"), "3 warning blocked items");
      const sub = tile.document.getElementById(a.getAttribute("aria-describedby") ?? "");
      eq(problems, "the tile is described by its sub line", sub?.textContent, "since Monday");
      const spark = a.querySelector("svg.ui-tile-spark");
      if (!spark) problems.push("no sparkline");
      else if (spark.getAttribute("aria-hidden") !== "true") problems.push("the sparkline is not aria-hidden");
      else if (!spark.querySelector("polyline")?.getAttribute("points")) problems.push("the sparkline has no points");
    }
    const plain = await render("src/ui/StatTile.tsx", "StatTile", { label: "items", value: 48 });
    const div = plain.document.querySelector("div.ui-tile");
    if (!div) problems.push(`a tile without to is no div.ui-tile: ${plain.html}`);
    else {
      eq(problems, "a neutral tile's text (its name)", div.textContent, "48 items");
      if (plain.document.querySelector("a")) problems.push("a tile without to is a link");
    }
    const ok_ = await render("src/ui/StatTile.tsx", "StatTile", { label: "passed", value: 9, tone: "ok" });
    eq(problems, "an ok tile adds no word", ok_.document.querySelector(".ui-tile-label")?.textContent, "passed");

    const segments = [
      { key: "done", label: "Done", value: 12, tone: "ok" },
      { key: "wip", label: "In progress", value: 3, tone: "info" },
      { key: "blocked", label: "Blocked", value: 0, tone: "error" },
    ];
    const bar = await render("src/ui/ProgressBar.tsx", "ProgressBar", { segments, total: 20, label: "Items" });
    const img = bar.document.querySelector('[role="img"]');
    if (!img) problems.push(`no role=img: ${bar.html}`);
    else {
      eq(problems, "the bar's name names each segment", img.getAttribute("aria-label"), "Items: Done 12, In progress 3, Blocked 0, of 20");
      eq(problems, "the drawn segments' widths", [...img.querySelectorAll("span")].map((s) => s.getAttribute("style")), ["width:60%", "width:15%"]);
    }
    eq(problems, "the legend lists each non-zero segment", [...bar.document.querySelectorAll(".ui-progress-legend li")].map((li) => li.textContent), ["Done 12", "In progress 3"]);
    const linked = await render("src/ui/ProgressBar.tsx", "ProgressBar", { segments, total: 20, label: "Items", to: "/p/x/board" });
    eq(problems, "a bar with to is a link", linked.document.querySelector("a.ui-progress")?.getAttribute("href"), "/p/x/board");
    ok = report(name, problems) && ok;
  }

  // R-6: the three states are distinct; Empty offers a sentence to copy.
  current = "web::check_states_are_distinct_and_empty_offers_a_sentence";
  {
    const name = current;
    const problems = [];
    const roles = (doc) => [...doc.querySelectorAll("[role]")].map((el) => el.getAttribute("role"));
    const loading = await render("src/ui/States.tsx", "LoadingState", {});
    eq(problems, "LoadingState's roles", roles(loading.document), ["status"]);
    eq(problems, "LoadingState's text", loading.document.querySelector('[role="status"]')?.textContent, "Loading…");
    const labelled = await render("src/ui/States.tsx", "LoadingState", { label: "Loading releases…" });
    eq(problems, "LoadingState's label", labelled.document.querySelector('[role="status"]')?.textContent, "Loading releases…");

    const error = await render("src/ui/States.tsx", "ErrorState", { error: "the board could not be read", retry: () => {} });
    eq(problems, "ErrorState's roles", roles(error.document), ["alert"]);
    const alert = error.document.querySelector('[role="alert"]');
    if (!alert || !alert.textContent?.includes("the board could not be read")) problems.push(`the alert does not hold the error: ${error.html}`);
    eq(problems, "ErrorState with retry offers Try again", [...error.document.querySelectorAll("button")].map((b) => b.textContent), ["Try again"]);
    const noRetry = await render("src/ui/States.tsx", "ErrorState", { error: "gone" });
    eq(problems, "ErrorState without retry has no button", noRetry.document.querySelectorAll("button").length, 0);

    const ask = "Ask your agent to plan the next sprint";
    const empty = await render("src/ui/States.tsx", "Empty", { title: "No sprint is planned", ask, children: "Plan one from the backlog." });
    eq(problems, "Empty's roles (none)", roles(empty.document), []);
    const box = empty.document.querySelector("div.ui-empty");
    if (!box) problems.push(`no div.ui-empty: ${empty.html}`);
    else {
      eq(problems, "Empty's title", box.querySelector("p.ui-empty-title")?.textContent, "No sprint is planned");
      if (!box.textContent?.includes("Plan one from the backlog.")) problems.push("Empty does not render its children");
      eq(problems, "Empty's sentence, in quotes", box.querySelector("q")?.textContent, ask);
      eq(problems, "Empty's Copy button", [...box.querySelectorAll("button")].map((b) => b.textContent), ["Copy"]);
      if (!box.querySelector('[aria-live="polite"]')) problems.push("the Copy button has no live region to say Copied");
    }
    const bare = await render("src/ui/States.tsx", "Empty", { title: "Nothing here" });
    if (bare.document.querySelector("button, q")) problems.push("Empty without ask offers a sentence");
    ok = report(name, problems) && ok;
  }
} catch (e) {
  ok = report(current, [`failed: ${e?.stack ?? e}`]) && ok;
} finally {
  await closeRender();
}

if (!ok) process.exit(1);
