#!/usr/bin/env node
//
// The UI kit's charts (FTR-046, R-5):
//   web::check_matrix_cells_link_to_their_lists — a matrix is a table with a caption and scoped
//     headers; a cell with `to` is a link named "row, col: value"; a warn or error cell says so in
//     text, not by colour alone;
//   web::check_timeline_scales_and_clamps_to_its_window — `scale` places dates in 0..1 and clamps;
//     `clampWindow` keeps at least 14 days around today; the hidden list holds every band and mark;
//   web::check_stepper_marks_done_current_and_next — `stepsFromStages` for no current stage, the
//     middle one, the last one and shipped; the current step carries aria-current.
//
// Usage: npm run check:ui-charts
import { clampWindow, scale } from "../src/ui/timeline.ts";
import { stepsFromStages } from "../src/ui/stepper.ts";
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

let ok = true;
let current = "web::check_matrix_cells_link_to_their_lists";

try {
  // R-5: matrix cells link to their lists.
  {
    const name = current;
    const problems = [];
    const values = { "v0.8.0|Done": { value: 12, to: "/p/x/review?release=v0.8.0" }, "v0.8.0|Blocked": { value: 2, tone: "error", to: "/p/x/review?blocked=1" }, "v0.7.0|Blocked": { value: 1, tone: "warn" } };
    const { html, document } = await render("src/ui/Matrix.tsx", "Matrix", {
      rows: ["v0.8.0", "v0.7.0"],
      cols: ["Done", "Blocked"],
      cell: (r, c) => values[`${r}|${c}`] ?? { value: 0 },
      caption: "Items per release",
    });
    const table = document.querySelector("table");
    if (!table) problems.push(`no table: ${html}`);
    eq(problems, "the caption", document.querySelector("table > caption")?.textContent, "Items per release");
    eq(problems, "column headers", [...document.querySelectorAll('th[scope="col"]')].map((th) => th.textContent), ["Done", "Blocked"]);
    eq(problems, "row headers", [...document.querySelectorAll('th[scope="row"]')].map((th) => th.textContent), ["v0.8.0", "v0.7.0"]);
    const unscoped = [...document.querySelectorAll("th")].filter((th) => !th.getAttribute("scope"));
    if (unscoped.length) problems.push(`${unscoped.length} header(s) without scope`);
    const links = [...document.querySelectorAll("td a")];
    eq(problems, "links' names", links.map((a) => a.getAttribute("aria-label")), ["v0.8.0, Done: 12", "v0.8.0, Blocked: 2"]);
    eq(problems, "links' targets", links.map((a) => a.getAttribute("href")), ["/p/x/review?release=v0.8.0", "/p/x/review?blocked=1"]);
    eq(problems, "links' text is the value", links.map((a) => a.textContent), ["12", "2"]);
    const cells = [...document.querySelectorAll("tbody td")];
    eq(problems, "cell count", cells.length, 4);
    const plain = cells[2];
    if (plain && plain.querySelector("a")) problems.push("a cell without `to` is a link");
    // Tone: an error and a warn cell show ⚠ (hidden from the reader) and the word, in text.
    const toned = [cells[1], cells[3]];
    eq(problems, "toned cells' hidden words", toned.map((c) => c?.querySelector(".ui-sr-only")?.textContent), ["error", "warning"]);
    eq(problems, "toned cells' ⚠ is aria-hidden", toned.map((c) => c?.querySelector('[aria-hidden="true"]')?.textContent), ["⚠", "⚠"]);
    eq(problems, "tone classes", cells.map((c) => c.className), ["ui-matrix-cell ui-matrix-neutral", "ui-matrix-cell ui-matrix-error", "ui-matrix-cell ui-matrix-neutral", "ui-matrix-cell ui-matrix-warn"]);
    if (cells[0]?.querySelector(".ui-sr-only")) problems.push("a neutral cell carries a tone word");
    ok = report(name, problems) && ok;
  }

  // R-5: the timeline scales, clamps and lists every band.
  current = "web::check_timeline_scales_and_clamps_to_its_window";
  {
    const name = current;
    const problems = [];
    eq(problems, "scale at from", scale("2026-11-01", "2026-11-21", "2026-11-01"), 0);
    eq(problems, "scale at the middle", scale("2026-11-01", "2026-11-21", "2026-11-11"), 0.5);
    eq(problems, "scale at to", scale("2026-11-01", "2026-11-21", "2026-11-21"), 1);
    eq(problems, "scale before from clamps to 0", scale("2026-11-01", "2026-11-21", "2026-10-01"), 0);
    eq(problems, "scale after to clamps to 1", scale("2026-11-01", "2026-11-21", "2027-01-01"), 1);
    eq(problems, "scale reads a time as its day (UTC)", scale("2026-11-01", "2026-11-21", "2026-11-11T23:30:00Z"), 0.5);
    eq(problems, "scale over no window is 0", scale("2026-11-01", "2026-11-01", "2026-11-01"), 0);
    eq(problems, "scale of an unreadable date is 0", scale("2026-11-01", "2026-11-21", "soon"), 0);
    eq(problems, "clampWindow widens a short window to 14 days around today", clampWindow("2026-11-15", "2026-11-18", "2026-11-17", 14), { from: "2026-11-10", to: "2026-11-24" });
    eq(problems, "clampWindow never cuts the given window", clampWindow("2026-11-01", "2026-11-18", "2026-11-17", 14), { from: "2026-11-01", to: "2026-11-24" });
    eq(problems, "clampWindow keeps a wide window", clampWindow("2026-10-01", "2026-12-31", "2026-11-17", 14), { from: "2026-10-01", to: "2026-12-31" });
    eq(problems, "clampWindow crosses a month end", clampWindow("2026-11-29", "2026-11-30", "2026-11-30", 14), { from: "2026-11-23", to: "2026-12-07" });
    eq(problems, "clampWindow leaves a window today is outside", clampWindow("2026-12-01", "2026-12-05", "2026-11-17", 14), { from: "2026-12-01", to: "2026-12-05" });

    const lanes = [
      {
        key: "sprints",
        label: "Sprints",
        bands: [
          { from: "2026-11-10", to: "2026-11-24", label: "SP-004", tone: "info", progress: 0.4, to_url: "/p/x/sprints/SP-004" },
          { from: "2026-11-24", to: "2026-12-08", label: "SP-005", pending: true },
        ],
        marks: [],
      },
      { key: "releases", label: "Releases", bands: [], marks: [{ at: "2026-11-20", label: "v0.8.0", kind: "diamond" }] },
    ];
    const { html, document } = await render("src/ui/Timeline.tsx", "Timeline", {
      from: "2026-11-10",
      to: "2026-12-08",
      today: "2026-11-17",
      lanes,
      caption: "Sprints and releases",
    });
    const svg = document.querySelector("svg");
    if (!svg) problems.push(`no svg: ${html}`);
    else {
      eq(problems, "the svg's role", svg.getAttribute("role"), "img");
      eq(problems, "the svg's name", svg.getAttribute("aria-label"), "Sprints and releases");
    }
    const items = [...document.querySelectorAll("ul.ui-sr-only li li")].map((li) => li.textContent);
    eq(problems, "the hidden list holds every band and mark", items, ["SP-004: 2026-11-10 to 2026-11-24, 40% done", "SP-005: 2026-11-24 to 2026-12-08, pending", "v0.8.0: 2026-11-20"]);
    eq(problems, "the hidden list names the lanes", [...document.querySelectorAll("ul.ui-sr-only > li")].map((li) => li.firstChild?.textContent), ["Sprints", "Releases"]);
    const today = document.querySelector(".ui-tl-today");
    if (!today) problems.push("no today line");
    else {
      eq(problems, "today's label", today.querySelector("text")?.textContent, "today");
      eq(problems, "today's line is 2 px", today.querySelector("line")?.getAttribute("stroke-width"), "2");
    }
    if (!document.querySelector(".ui-tl-band .ui-tl-done") || !document.querySelector(".ui-tl-band .ui-tl-rest")) problems.push("a band with progress has no done and rest parts");
    const pending = document.querySelector(".ui-tl-pending");
    if (!pending) problems.push("no pending band");
    else {
      if (!pending.querySelector("rect")?.getAttribute("stroke-dasharray")) problems.push("the pending band is not dashed");
      if (!/pending/.test(pending.querySelector("text")?.textContent ?? "")) problems.push("the pending band is not labelled pending");
    }
    if (!document.querySelector(".ui-tl-diamond polygon")) problems.push("the release mark is not a diamond");
    const linked = [...svg?.querySelectorAll("a") ?? []].map((a) => a.getAttribute("href"));
    eq(problems, "a band with to_url is a link", linked, ["/p/x/sprints/SP-004"]);

    // Without today inside the window, no today line is drawn.
    const past = await render("src/ui/Timeline.tsx", "Timeline", { from: "2026-01-01", to: "2026-02-01", today: "2026-11-17", lanes, caption: "Past" });
    if (past.document.querySelector(".ui-tl-today")) problems.push("a today line is drawn outside the window");
    ok = report(name, problems) && ok;
  }

  // R-5: the stepper marks done, current and next.
  current = "web::check_stepper_marks_done_current_and_next";
  {
    const name = current;
    const problems = [];
    const stages = [
      { key: "dev", label: "Dev" },
      { key: "candidate", label: "Candidate" },
      { key: "main", label: "Main" },
    ];
    const history = [
      { stage: "dev", at: "2026-11-10T09:00:00Z", by: "claude" },
      { stage: "dev", at: "2026-11-12T09:00:00Z", by: "the person" },
      { stage: "candidate", at: "2026-11-15T10:00:00Z", by: "the person" },
    ];
    const states = (steps) => steps.map((s) => s.state);
    eq(problems, "no current stage", states(stepsFromStages(stages, null, history, false)), ["next", "next", "next"]);
    eq(problems, "an unknown current stage", states(stepsFromStages(stages, "nowhere", history, false)), ["next", "next", "next"]);
    eq(problems, "the first stage", states(stepsFromStages(stages, "dev", history, false)), ["current", "next", "next"]);
    const middle = stepsFromStages(stages, "candidate", history, false);
    eq(problems, "the middle stage", states(middle), ["done", "current", "next"]);
    eq(problems, "a done step's latest move", [middle[0].at, middle[0].by], ["2026-11-12T09:00:00Z", "the person"]);
    eq(problems, "the last stage", states(stepsFromStages(stages, "main", history, false)), ["done", "done", "current"]);
    const noHistory = stepsFromStages(stages, "main", [], false);
    eq(problems, "a done step without history has no date", [noHistory[0].at, noHistory[0].by], [undefined, undefined]);
    eq(problems, "shipped", states(stepsFromStages(stages, "main", history, true)), ["done", "done", "done"]);
    eq(problems, "shipped with no current stage", states(stepsFromStages(stages, null, [], true)), ["done", "done", "done"]);
    eq(problems, "no stages", stepsFromStages([], "dev", history, false), []);

    const { html, document } = await render("src/ui/StageStepper.tsx", "StageStepper", { stages: middle, label: "v0.8.0's stages" });
    const ol = document.querySelector("ol");
    if (!ol) problems.push(`no ol: ${html}`);
    else eq(problems, "the list's name", ol.getAttribute("aria-label"), "v0.8.0's stages");
    const lis = [...document.querySelectorAll("ol > li")];
    eq(problems, "aria-current per step", lis.map((li) => li.getAttribute("aria-current")), [null, "step", null]);
    eq(problems, "state in text", lis.map((li) => li.querySelector(".ui-step-state")?.textContent), ["done", "now", "next"]);
    eq(problems, "a done step's date and who", lis[0]?.querySelector(".ui-step-when")?.textContent, "2026-11-12 · the person");
    if (lis[1]?.querySelector(".ui-step-when") || lis[2]?.querySelector(".ui-step-when")) problems.push("a step that is not done shows a date");
    if (lis.some((li) => li.querySelector(".ui-step-dot")?.getAttribute("aria-hidden") !== "true")) problems.push("a dot is not aria-hidden");
    ok = report(name, problems) && ok;
  }
} catch (e) {
  ok = report(current, [`failed: ${e?.stack ?? e}`]) && ok;
} finally {
  await closeRender();
}

if (!ok) process.exit(1);
