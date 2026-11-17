#!/usr/bin/env node
//
// FTR-033: the Baseline page, checked without a browser.
//   web::baseline_page_shows_areas_and_accepts_a_revision (R-3) — revisionAsPrepared gives a
//     proposed revision's fields and its command; areaRows groups the areas by status (accepted,
//     proposed, open with the deferred ones marked, not applicable); entryLine says an entry in one
//     line; the rendered page shows the areas and offers Accept on FTR-034's prepared panel only
//     while it may write, else the command; it posts only `…/accept` and `…/withdraw` through
//     writePost, consults isReadOnly, and has no <input of its own outside the panel.
//   web::baseline_page_sits_under_the_architecture_area (R-6) — nav.ts's Architecture area holds
//     the Baseline entry, linking /p/:project/baseline, no other area does, main.tsx routes it, and
//     the page marks that entry active.
//   web::releases_page_shows_the_baselines_stage_gates (R-3) — stageGateLines names each gate the
//     baseline requires at a release stage as "requires `sast` (baseline B-007)", in stage order,
//     and the Releases page renders them.
//
// Usage: npm run check:baseline
import { readFileSync } from "node:fs";
import { NAV, activeEntry } from "../src/nav.ts";
import { areaRows, chosenId, entryLine, revisionAsPrepared, stageGateLines } from "../src/baseline.ts";
import { closeRender, render } from "./lib/render.mjs";

let failed = false;
function report(name, problems, ok) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ${ok}`);
  }
}
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);

const stamp = (by, at, origin = "conversation") => ({ by, at, origin });
const entry = (id, area, option, kind, text, extra = {}) => ({
  id,
  pack: "data",
  area,
  option,
  kind,
  text,
  revision: "REV-001",
  ...extra,
});
const viewEntry = (e, extra = {}) => ({
  ...e,
  in_force: true,
  pack_removed: false,
  named_by: [],
  not_accepted_through_kanbanr: false,
  ...extra,
});
const area = (id, status, extra = {}) => ({
  id,
  pack: "data",
  title: id,
  question: `Which ${id}?`,
  status,
  options: [],
  value: "",
  reason: "",
  item: "",
  revision: "",
  entries: [],
  ...extra,
});

const b1 = entry("B-001", "database", "postgresql", "constraint", "PostgreSQL is the database", { adr: "ADR-0004" });
const b2 = entry("B-002", "database", "postgresql", "non_goal", "No second database engine");
const b3 = entry("B-003", "database", "sqlite", "constraint", "SQLite is the database", { revision: "REV-002" });
const proposed = {
  id: "REV-002",
  item: "FTR-012",
  note: "",
  answers: [
    { area: "database", options: ["sqlite"] },
    { area: "kubernetes", later: true },
    { area: "ui-style", not_applicable: "no screens" },
    { area: "component-library", options: [{ option: "named", value: "MUI" }] },
  ],
  adds: ["B-003"],
  replaces: ["B-001"],
  proposed: stamp("Claude", "2026-11-17T09:00:00Z"),
  state: "proposed",
  rev: "7c1e00aa",
  entries: [b3],
  replaces_entries: [b1],
  stage_gates: [["main", "sast"]],
  depends_on: ["FTR-020", "ADR-0004"],
  command: "kanbanr baseline accept REV-002",
};
const accepted = {
  id: "REV-001",
  item: "",
  note: "",
  answers: [{ area: "database", options: ["postgresql"] }],
  adds: ["B-001", "B-002"],
  replaces: [],
  proposed: stamp("Claude", "2026-11-10T09:00:00Z"),
  accepted: { ...stamp("Ada", "2026-11-10T10:00:00Z", "monitor"), rev: "3f9a" },
  state: "accepted",
  rev: "3f9a",
  entries: [b1, b2],
  replaces_entries: [],
  stage_gates: [],
  depends_on: [],
  command: "kanbanr baseline accept REV-001",
};
const view = {
  packs: [
    { name: "core", library: "builtin", version: 1, rev: "aa", adopted: stamp("Claude", "2026-11-10T09:00:00Z"), messages: [] },
    {
      name: "data",
      library: "builtin",
      version: 1,
      rev: "bb",
      adopted: stamp("Claude", "2026-11-10T09:00:00Z"),
      messages: ["pack data v2 is available: `kanbanr profile update data`"],
    },
    {
      name: "web",
      library: "builtin",
      version: 1,
      rev: "cc",
      adopted: stamp("Claude", "2026-11-10T09:00:00Z"),
      removed: stamp("Claude", "2026-11-12T09:00:00Z"),
      messages: [],
    },
  ],
  areas: [
    area("ui-style", "not_applicable", { reason: "no screens", pack: "web" }),
    area("kubernetes", "later", { pack: "kubernetes" }),
    area("observability", "open", { pack: "observability" }),
    area("component-library", "proposed", { pack: "web" }),
    area("database", "answered", {
      options: ["postgresql"],
      item: "",
      revision: "REV-001",
      entries: [viewEntry(b1, { named_by: ["FTR-020"] }), viewEntry(b2, { not_accepted_through_kanbanr: true, pack_removed: true })],
    }),
  ],
  revisions: [accepted, proposed],
  charter_constraints: ["Local-first: plain YAML and markdown in a git repo"],
};

// R-3: the areas, the revision as a prepared record, and the page's one decision.
{
  const problems = [];
  const expect = (name, got, want) => {
    if (!same(got, want)) problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  };

  const p = revisionAsPrepared(proposed);
  expect("revisionAsPrepared keeps the revision's id", p.id, "REV-002");
  expect("revisionAsPrepared pins the revision's rev", p.rev, "7c1e00aa");
  expect("a proposed revision is open", p.state, "open");
  expect("its command is the CLI's accept", p.command, "kanbanr baseline accept REV-002");
  expect("it is prepared by whoever proposed it", [p.by, p.origin, p.at], ["Claude", "conversation", "2026-11-17T09:00:00Z"]);
  expect("it links the item whose design asked", p.sources, [{ label: "FTR-012", url: "item:FTR-012" }]);
  expect("its fields say what accepting it does", p.fields, {
    Answers: "database: sqlite; kubernetes: decide later; ui-style: not applicable (no screens); component-library: named (MUI)",
    "Entries added": "B-003 constraint: SQLite is the database",
    "Entries replaced": "B-001 constraint: PostgreSQL is the database",
    "Stage gates added": "main requires sast",
    "Depends on": "FTR-020, ADR-0004",
  });
  expect("an accepted revision is no open record", revisionAsPrepared(accepted).state, "confirmed");
  expect("a withdrawn revision is no open record", revisionAsPrepared({ ...proposed, state: "withdrawn" }).state, "rejected");
  expect("nothing to replace leaves the field out", "Entries replaced" in revisionAsPrepared(accepted).fields, false);

  expect("a bare option is its id", chosenId("postgresql"), "postgresql");
  expect("an option with a value says it", chosenId({ option: "named", value: "MUI" }), "named (MUI)");
  expect("entryLine names the id, the kind and the text", entryLine(b2), "B-002 non-goal: No second database engine");

  const rows = areaRows(view);
  expect(
    "areaRows groups accepted, proposed, open and not applicable, in that order",
    rows.map((r) => [r.group, r.areas.map((a) => a.id)]),
    [
      ["answered", ["database"]],
      ["proposed", ["component-library"]],
      ["open", ["observability", "kubernetes"]],
      ["not_applicable", ["ui-style"]],
    ],
  );
  expect("areaRows drops an empty group", areaRows({ ...view, areas: [area("x", "open")] }).map((r) => r.group), ["open"]);

  // The page as rendered: writable, then read-only.
  const props = (writable) => ({
    project: "shop",
    view,
    adrs: [{ id: "ADR-0004", path: "decisions/0004-postgresql.md" }],
    writable,
    by: writable ? "Ada" : null,
  });
  const { document } = await render("src/pages/BaselinePage.tsx", "BaselineBody", props(true), { url: "/p/shop/baseline" });
  const text = document.body.textContent ?? "";
  const h1 = [...document.querySelectorAll("h1")].map((h) => h.textContent?.trim());
  expect("the page has one h1, Baseline", h1, ["Baseline"]);
  const h2 = [...document.querySelectorAll("h2")].map((h) => h.textContent?.trim());
  for (const want of ["Packs", "Proposed revisions", "Areas", "Also in the charter", "History"]) {
    if (!h2.includes(want)) problems.push(`the page has no "${want}" section (h2s: ${h2.join(", ")})`);
  }
  const buttons = [...document.querySelectorAll("button")].map((b) => b.textContent?.trim());
  if (!buttons.includes("Accept")) problems.push(`a writable page offers no Accept (buttons: ${buttons.join(", ")})`);
  if (!document.querySelector("#REV-002 .prepared-panel")) problems.push("REV-002 is not shown on a prepared panel under its id");
  for (const want of ["decide later", "not applicable", "no screens", "pack removed", "not accepted through kanbanr", "changed", "current"]) {
    if (!text.includes(want)) problems.push(`the page does not say "${want}"`);
  }
  if (!text.includes("Local-first: plain YAML")) problems.push("the charter's constraints are not shown");
  const links = [...document.querySelectorAll("a")].map((a) => a.getAttribute("href"));
  for (const want of ["/p/shop/docs/file/decisions/0004-postgresql.md", "/p/shop/feature/FTR-020", "/p/shop/charter"]) {
    if (!links.includes(want)) problems.push(`the page does not link ${want} (links: ${links.join(", ")})`);
  }
  if (!text.includes("accepted by Ada")) problems.push("the history does not say who accepted REV-001");

  const ro = (await render("src/pages/BaselinePage.tsx", "BaselineBody", props(false), { url: "/p/shop/baseline" })).document;
  const roButtons = [...ro.querySelectorAll("button")].map((b) => b.textContent?.trim());
  if (roButtons.includes("Accept")) problems.push("a read-only page offers Accept");
  if (!(ro.body.textContent ?? "").includes("kanbanr baseline accept REV-002")) problems.push("a read-only page does not show the command");

  // The source: the one decision goes through the panel and writePost, gated on isReadOnly.
  const page = read("pages/BaselinePage.tsx");
  const api = read("api.ts");
  if (!/<PreparedPanel[\s\S]*?label="Accept"/.test(page)) problems.push("BaselinePage.tsx does not render PreparedPanel with label Accept");
  if (!/\bisReadOnly\(\)/.test(page)) problems.push("BaselinePage.tsx never consults isReadOnly()");
  if (/<input\b/.test(page)) problems.push("BaselinePage.tsx has an <input of its own; the reason is the panel's");
  if (!/api\.acceptBaseline\(/.test(page) || !/api\.withdrawBaseline\(/.test(page)) {
    problems.push("BaselinePage.tsx does not call api.acceptBaseline and api.withdrawBaseline");
  }
  const posts = [...page.matchAll(/api\.([A-Za-z]+)\(/g)].map((m) => m[1]).filter((n) => !/^get|^approver$/.test(n));
  for (const n of posts) {
    if (n !== "acceptBaseline" && n !== "withdrawBaseline") problems.push(`BaselinePage.tsx calls api.${n}, a write it has no business with`);
  }
  for (const [fn, tail] of [
    ["acceptBaseline", "/accept`"],
    ["withdrawBaseline", "/withdraw`"],
  ]) {
    const m = api.match(new RegExp(`${fn}: \\([^)]*\\) =>\\s*writePost\\(\\s*\`/projects/\\$\\{encodeURIComponent\\(\\w+\\)\\}/baseline/revisions/\\$\\{encodeURIComponent\\(\\w+\\)\\}([^\`]*\`)`));
    if (!m) problems.push(`api.ts: ${fn} does not post through writePost to /projects/{p}/baseline/revisions/{id}…`);
    else if (m[1] !== tail) problems.push(`api.ts: ${fn} posts to …${m[1]}, wanted …${tail}`);
  }
  if (!/getBaseline: \([^)]*\) =>\s*getJson<BaselineView>\(/.test(api)) problems.push("api.ts: getBaseline does not read a BaselineView");

  report(
    "web::baseline_page_shows_areas_and_accepts_a_revision",
    problems,
    "the areas are grouped, a proposed revision is Accept on the prepared panel (the command when read-only), posted through writePost only.",
  );
}

// R-6: the Baseline entry sits in the Architecture area.
{
  const problems = [];
  const holders = NAV.filter((a) => a.entries.some((e) => e.key === "baseline" || e.path("x") === "/p/x/baseline"));
  if (holders.length !== 1 || holders[0].key !== "architecture") {
    problems.push(`the Baseline entry is in ${holders.map((a) => a.key).join(", ") || "no area"}, wanted architecture only`);
  }
  const e = NAV.find((a) => a.key === "architecture")?.entries.find((x) => x.key === "baseline");
  if (!e) problems.push("the Architecture area has no entry keyed baseline");
  else {
    if (e.label !== "Baseline") problems.push(`the entry is labelled ${JSON.stringify(e.label)}, wanted "Baseline"`);
    if (e.path("a b") !== "/p/a%20b/baseline") problems.push(`the entry links ${e.path("a b")}, wanted /p/a%20b/baseline`);
    if (e.when) problems.push("the entry is shown only under a condition; the Baseline page is always there");
  }
  const at = activeEntry(NAV, "x", "/p/x/baseline");
  if (!same(at, { area: "architecture", entry: "baseline" })) problems.push(`/p/x/baseline marks ${JSON.stringify(at)} active`);
  const main = read("main.tsx");
  if (!/path: "p\/:project\/baseline", element: <BaselinePage \/>/.test(main)) problems.push("main.tsx does not route p/:project/baseline to BaselinePage");
  report("web::baseline_page_sits_under_the_architecture_area", problems, "Architecture › Baseline links /p/:project/baseline, routed and marked active.");
}

// R-3: the gates the baseline requires at release stages, as the Releases page shows them.
{
  const problems = [];
  const cadence = {
    release_stages: ["candidate", "main", "production"],
    release_gates: {
      production: { requires: ["pushed"], gates: [{ gate: "sast", from: "B-007" }] },
      main: { gates: [{ gate: "ci", from: "B-002" }] },
      candidate: { requires: ["items_merged"] },
    },
  };
  const got = stageGateLines(cadence);
  const want = [
    { stage: "main", text: "requires `ci` (baseline B-002)" },
    { stage: "production", text: "requires `sast` (baseline B-007)" },
  ];
  if (!same(got, want)) problems.push(`stageGateLines gave ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  if (stageGateLines({ release_stages: ["a"], release_gates: { a: {} } }).length !== 0) problems.push("a stage with no baseline gates gave a line");
  if (stageGateLines(undefined).length !== 0) problems.push("no cadence gave a line");
  const page = read("pages/ReleasesPage.tsx");
  if (!/<BaselineStageGates\b/.test(page)) problems.push("ReleasesPage.tsx does not render BaselineStageGates");
  const part = read("pages/releases/BaselineStageGates.tsx");
  if (!/stageGateLines\(/.test(part)) problems.push("BaselineStageGates.tsx does not use stageGateLines");
  report(
    "web::releases_page_shows_the_baselines_stage_gates",
    problems,
    "each baseline stage gate reads \"requires `<gate>` (baseline B-n)\" on the Releases page, in stage order.",
  );
}

await closeRender();
process.exit(failed ? 1 : 0);
