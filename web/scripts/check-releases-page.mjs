#!/usr/bin/env node
//
// FTR-041: the Releases page, checked without a browser. Each check prints its name.
//   web::check_releases_tabs_carry_counts_and_open_current_first (R-1) — releaseTabs labels
//     Current, Planned and Released with the summary's counts and hides an empty tab unless all are
//     empty; defaultTab opens Current when a release is in flight, the URL's tab when it names one,
//     and maps the route's `past` to the page's Released.
//   web::check_releases_first_release_is_open_unless_the_url_names_others (R-2) — initiallyOpen;
//     rendered, a tab opens its first (newest) release alone, or the hash's releases when it names
//     any in the tab.
//   web::check_release_header_carries_chips_stepper_and_next_step (R-3) — headerChips and
//     stepperSteps (an undone stage shows as not reached); rendered, a Current release's header
//     holds its button, the step's first line, and why when it is disabled; a closed Planned
//     release shows "next: …" and reads no plan.
//   web::check_release_item_table_sorts_filters_and_narrows (R-4) — ITEM_COLUMNS with sortRows and
//     filterRows: Tests sorts by its ratio, an item naming none last; narrow keeps ID, Title and
//     Status; an open release draws its table with a linked ID. At 390 px the open release's table
//     draws only ID, Title and Status, with the rest under each row's "More"; below 720 px its cells
//     wrap, and below 640 px the header stacks to one column with a stepper that may shrink (T6).
//   web::check_releases_url_names_the_tab_and_open_releases (R-5) — `?tab=planned#v0.8.0` selects
//     Planned and opens v0.8.0 alone; the filter box shows and writes `?q=`.
//   web::check_releases_page_reads_only_what_is_open (R-6) — the sources read the summary, one
//     tab's slice, an open release's items and plans through the shared cache with their tags;
//     ReleaseBody is drawn only inside AccordionRow's `children`; nothing under the page reads the
//     whole board.
//   web::check_release_header_is_two_lines_with_a_dot_stepper (R-8) — `.release-head`'s header is a
//     two-row grid: the chips and an `ol` stepper whose reached stages show date and who, then the
//     button and its line; the stepper is dots joined by a line, no pills.
//   web::check_releases_read_plans_only_for_current_and_open_releases (R-9) — with the cache's reads
//     recorded: Current reads one plan per release, a closed Planned release none and an open one
//     one, Released none.
//   web::check_releases_filter_reads_no_closed_table (R-10) — with `?q=v0.9`: a closed release
//     named otherwise is hidden, the open release's table gets the filter, and no closed release's
//     items are read.
//   web::check_release_blocking_reason_is_shown_once (R-3) — notShownAbove drops the lines already
//     shown in the header; a release whose header shows a reason and its first step shows each of
//     them once in the whole release, a distinct missing line still shows under Needs, and with only
//     the duplicate missing there is no Needs heading (UX-5).
//   web::check_releases_filter_wears_the_kit_search_style (R-10) — the page filter's input sits
//     inside `.ui-table-tools`, as each release table's filter does.
//
// Usage: npm run check:releases-page
import { ITEM_COLUMNS, TAB_OF, defaultTab, headerChips, initiallyOpen, releaseTabs, stepperSteps } from "../src/releaseTabs.ts";
import { filterRows, sortRows } from "../src/ui/table.ts";
import { writeState } from "../src/ui/urlState.ts";
import { readFileSync, readdirSync } from "node:fs";
import { closeRender, load, render } from "./lib/render.mjs";

let failed = false;
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);

/** Run one named check: its problems, or one line saying what it proved. */
async function check(name, proved, body) {
  const problems = [];
  const expect = (what, got, want) => {
    if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  };
  try {
    await body(expect, problems);
  } catch (e) {
    problems.push(`threw: ${e?.stack ?? e}`);
  }
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else console.log(`${name} — ${proved}`);
}

const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
const src = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");
/** Each row's id and whether its header says it is open. */
const expanded = (document) =>
  Object.fromEntries(
    [...document.querySelectorAll("section.ui-row")].map((s) => [s.id, s.querySelector("button[aria-expanded]")?.getAttribute("aria-expanded") === "true"]),
  );

const STAGES = ["candidate", "main", "production"];
const summary = (current, planned, past) => ({
  current,
  planned,
  past,
  open: current > 0 ? "current" : planned > 0 ? "planned" : "past",
});
const release = (version, extra = {}) => ({ version, state: "planned", items: [], finished: 0, ...extra });

const CURRENT = [
  release("v0.8.0", {
    name: "Plans",
    stage: "main",
    branch: "release/v0.8.0",
    target: "2026-11-24",
    items: ["FTR-1", "FTR-2", "FTR-3", "FTR-4"],
    finished: 3,
    carried: [{ code: "FTR-9", to: "v0.9.0" }],
    stages: [
      { stage: "candidate", by: "Ann", at: "2026-11-16T10:00:00Z" },
      { stage: "main", by: "Ann", at: "2026-11-17T09:00:00Z" },
    ],
  }),
  release("v0.7.1", { stage: "candidate", branch: "release/v0.7.1", parent: "v0.7.0", items: ["DEF-1"], stages: [{ stage: "candidate", by: "Bo", at: "2026-11-15T08:00:00Z" }] }),
];
const PLANNED = [release("v0.10.0", { name: "Later" }), release("v0.9.0", { name: "Next", items: ["FTR-9"] }), release("v0.8.5")];
const PAST = [
  release("v0.7.0", { state: "shipped", stage: "production", shipped_at: "2026-11-13T12:00:00Z", items: ["FTR-5"], finished: 1 }),
  release("v0.6.0", { state: "shipped", stage: "production", shipped_at: "2026-11-01T12:00:00Z" }),
];

// A plan ready to promote, and one whose branch step is blocked (DEF-071's fixture).
const READY = {
  version: "v0.8.0",
  from: "main",
  to: "production",
  direction: "forward",
  cut: false,
  confirm: false,
  ships: false,
  button: "Promote v0.8.0 to production",
  conditions: [{ check: "items_finished", stage: "production", holds: true, detail: "4 planned item(s), all finished" }],
  steps: [{ action: "move_main", stage: "production", does: "fast-forwards main to release/v0.8.0 (abc1234)", noop: false }],
  release_tip: "abc1234def",
  missing: [],
  ready: true,
  needs_reason: false,
  repo: true,
  empty: false,
};
const REASON = "release/v0.7.1 already exists in /work/shop — nothing was changed";
const BLOCKED = {
  ...READY,
  version: "v0.7.1",
  from: "",
  to: "candidate",
  button: "Promote v0.7.1 to candidate",
  conditions: [],
  steps: [{ action: "branch", stage: "candidate", does: "makes release/v0.7.1", noop: false, blocked: REASON }],
  missing: [REASON],
  ready: false,
};
// UX-5: a plan whose blocking reason and first step would each show twice (header and body).
const DUP = {
  ...READY,
  version: "v1.1.0",
  from: "",
  to: "candidate",
  button: "Promote v1.1.0 to candidate",
  conditions: [],
  steps: [{ action: "branch", stage: "candidate", does: "makes A from B", noop: false }],
  missing: ["X does not exist", "Y"],
  ready: false,
};
const DUP_ONLY = { ...DUP, version: "v1.2.0", missing: ["X does not exist"] };
const ITEMS = [
  { code: "FTR-1", title: "One", kind: "feature", status: "Done", sprint: "SP-004", tests_green: 3, tests_named: 4, approval: "approved", design_review: "signed", docs_folded: "", carried_to: "", finished: true },
  { code: "FTR-2", title: "Two", kind: "feature", status: "In Progress", sprint: "SP-004", tests_green: 1, tests_named: 2, approval: "lapsed", design_review: "", docs_folded: "", carried_to: "", finished: false },
  { code: "FTR-3", title: "Three", kind: "defect", status: "Planned", sprint: "", tests_green: 0, tests_named: 0, approval: "none", design_review: "", docs_folded: "", carried_to: "", finished: false },
  { code: "FTR-9", title: "Nine", kind: "feature", status: "Planned", sprint: "", tests_green: 2, tests_named: 2, approval: "added", design_review: "", docs_folded: "lapsed", carried_to: "v0.9.0", finished: false },
];

// The shared cache the rendered components read through, loaded by the same Vite server, so a
// check seeds it and records the URLs a render reads.
const { cache } = await load("src/data/cache.ts");
const actions = await load("src/pages/releases/ReleaseActions.tsx");
const { TAGS, urls } = await load("src/data/urls.ts");
const P = "shop";
await cache.fetch(urls.releasePlan(P, "v0.8.0"), [], async () => READY);
await cache.fetch(urls.releasePlan(P, "v0.7.1"), [], async () => BLOCKED);
await cache.fetch(urls.releasePlan(P, "v1.1.0"), [], async () => DUP);
await cache.fetch(urls.releasePlan(P, "v1.2.0"), [], async () => DUP_ONLY);
await cache.fetch(urls.releaseItems(P, "v0.8.0"), [], async () => ITEMS);
const reads = [];
const read = cache.read.bind(cache);
cache.read = (url) => {
  reads.push(url);
  return read(url);
};
/** Render, returning the document and the distinct URLs read meanwhile. */
async function drawn(module, name, props, url) {
  reads.length = 0;
  const out = await render(module, name, props, { url });
  return { ...out, reads: [...new Set(reads)] };
}
const PAGE = { project: P, stages: STAGES, gates: ["ci"], notesPath: "", writable: true, who: "Ann" };
const list = (scope, slice, url) => drawn("src/pages/ReleasesPage.tsx", "ReleaseListBody", { ...PAGE, scope, slice }, url);
const ctx = (extra = {}) => ({
  ...PAGE,
  busy: null,
  outcome: {},
  onPromote: () => {},
  removable: () => false,
  onRemove: () => {},
  prepared: [],
  shown: {},
  onShown: () => {},
  tick: 0,
  q: "",
  ...extra,
});
const row = (r, scope, next, open, url = "/p/shop/releases") =>
  drawn("src/pages/releases/ReleaseRow.tsx", "ReleaseRow", { ctx: ctx(), release: r, scope, next, open, onToggle: () => {} }, url);
const plans = (r) => r.filter((u) => u.endsWith("/plan")).map((u) => u.split("/").slice(-2, -1)[0]);
const itemReads = (r) => r.filter((u) => u.endsWith("/items")).map((u) => u.split("/").slice(-2, -1)[0]);

/** Render with the browser's width at `px`: `matchMedia` answers `(max-width: N px)` for that width. */
async function atWidth(px, draw) {
  const had = Object.prototype.hasOwnProperty.call(globalThis, "window");
  const before = globalThis.window;
  globalThis.window = {
    innerWidth: px,
    matchMedia: (q) => {
      const max = /max-width:\s*(\d+)px/.exec(q);
      return { matches: max ? px <= Number(max[1]) : false, addEventListener() {}, removeEventListener() {} };
    },
  };
  try {
    return await draw();
  } finally {
    if (had) globalThis.window = before;
    else delete globalThis.window;
  }
}
/** The declarations of `selector`'s rules inside `@media (max-width: <px>px)` blocks of `css`. */
function narrowRule(css, px, selector) {
  const out = [];
  const at = new RegExp(`@media\\s*\\(max-width:\\s*${px}px\\)\\s*\\{`, "g");
  let m;
  while ((m = at.exec(css)) !== null) {
    let depth = 1;
    let j = m.index + m[0].length;
    for (; j < css.length && depth > 0; j++) {
      if (css[j] === "{") depth++;
      else if (css[j] === "}") depth--;
    }
    for (const r of css.slice(m.index + m[0].length, j - 1).matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (r[1].split(",").map((x) => x.trim()).includes(selector)) out.push(r[2]);
    }
    at.lastIndex = j;
  }
  return out.join(";");
}

await check(
  "web::check_releases_tabs_carry_counts_and_open_current_first",
  "the tabs are Current, Planned and Released with the summary's counts, an empty one hidden unless all are, and the page opens on Current when a release is in flight.",
  (expect) => {
    expect(
      "the tabs, in order, with their counts",
      releaseTabs(summary(2, 4, 7)).map((t) => [t.key, t.label, t.count, t.hidden]),
      [
        ["current", "Current", 2, false],
        ["planned", "Planned", 4, false],
        ["released", "Released", 7, false],
      ],
    );
    expect("an empty tab is hidden", releaseTabs(summary(0, 4, 7)).map((t) => t.hidden), [true, false, false]);
    expect("all empty: none hidden", releaseTabs(summary(0, 0, 0)).map((t) => t.hidden), [false, false, false]);
    expect("in flight: Current", defaultTab(summary(2, 4, 7), null), "current");
    expect("nothing in flight: Planned", defaultTab(summary(0, 4, 7), null), "planned");
    expect("only shipped: Released, from the route's past", defaultTab(summary(0, 0, 7), null), "released");
    expect("the URL's tab wins", defaultTab(summary(2, 4, 7), "released"), "released");
    expect("an unknown tab falls back to open", defaultTab(summary(2, 4, 7), "past"), "current");
    expect("the page tab Released reads the route's past", [TAB_OF.current, TAB_OF.planned, TAB_OF.released], ["current", "planned", "past"]);
  },
);

await check(
  "web::check_releases_first_release_is_open_unless_the_url_names_others",
  "a tab opens its first release alone unless the hash names releases in it, and then those.",
  async (expect) => {
    expect("no hash: the first", [...initiallyOpen(PLANNED, new Set())], ["v0.10.0"]);
    expect("the hash names two here", [...initiallyOpen(PLANNED, new Set(["v0.9.0", "v0.8.5"]))], ["v0.9.0", "v0.8.5"]);
    expect("the hash names only another tab's", [...initiallyOpen(PLANNED, new Set(["v0.7.0"]))], ["v0.10.0"]);
    expect("an empty tab opens nothing", [...initiallyOpen([], new Set())], []);
    const first = await list("planned", PLANNED, "/p/shop/releases?tab=planned");
    expect("rendered: the first open", expanded(first.document), { "v0.10.0": true, "v0.9.0": false, "v0.8.5": false });
    const named = await list("planned", PLANNED, "/p/shop/releases?tab=planned#v0.9.0,v0.8.5");
    expect("rendered: the hash's", expanded(named.document), { "v0.10.0": false, "v0.9.0": true, "v0.8.5": true });
  },
);

await check(
  "web::check_release_header_carries_chips_stepper_and_next_step",
  "the header carries the tile's chips and a stepper whose undone stage is not reached; a Current release shows its button, its first line and why it is disabled; a closed Planned one says next and reads no plan.",
  async (expect, problems) => {
    expect(
      "the chips",
      headerChips(CURRENT[0], STAGES).map((c) => c.text),
      ["v0.8.0", "Plans", "planned", "stage main", "target 2026-11-24", "3/4 finished (75%)", "1 carried over"],
    );
    expect(
      "a waiting, superseded and shipped release's chips",
      headerChips(release("v1", { stage: "production", tagged_at: "x", superseded_by: "v1.0.1" }), STAGES).map((c) => [c.key, c.tone]),
      [
        ["version", "neutral"],
        ["state", "neutral"],
        ["stage", "ok"],
        ["superseded", "warn"],
        ["finished", "neutral"],
      ],
    );
    expect("before the first stage", headerChips(PLANNED[2], STAGES).find((c) => c.key === "stage")?.text, "not yet a candidate");
    expect(
      "the stepper at main",
      stepperSteps(CURRENT[0], STAGES).map((s) => [s.key, s.state, s.at ?? "", s.by ?? ""]),
      [
        ["candidate", "done", "2026-11-16T10:00:00Z", "Ann"],
        ["main", "current", "", ""],
        ["production", "next", "", ""],
      ],
    );
    const undone = release("v2", {
      stage: "candidate",
      stages: [
        { stage: "candidate", by: "Ann", at: "2026-11-10T00:00:00Z" },
        { stage: "main", by: "Ann", at: "2026-11-11T00:00:00Z" },
        { stage: "candidate", by: "Ann", at: "2026-11-12T00:00:00Z", undoes: "main" },
      ],
    });
    expect("an undone stage is not reached", stepperSteps(undone, STAGES).map((s) => s.state), ["current", "next", "next"]);
    expect("shipped: every stage done", stepperSteps(PAST[0], STAGES).map((s) => s.state), ["done", "done", "done"]);

    const ready = await row(CURRENT[0], "current", "production", false);
    const actions = ready.document.querySelector(".ui-row-actions");
    expect("a Current release's button", text(actions?.querySelector(".review-actions button")), "Promote v0.8.0 to production");
    expect("its first line", text(actions?.querySelector(".release-does")), "fast-forwards main to release/v0.8.0 (abc1234)");
    if (actions?.querySelector(".release-why")) problems.push("a ready release says why it is disabled");
    const blocked = await row(CURRENT[1], "current", "candidate", false);
    const why = blocked.document.querySelector(".ui-row-actions .release-why");
    expect("a disabled button says why beside it", text(why), REASON);
    expect("and is disabled", blocked.document.querySelector(".ui-row-actions .review-actions button")?.hasAttribute("disabled"), true);
    const closed = await row(PLANNED[1], "planned", "candidate", false);
    expect("a closed Planned release says next", text(closed.document.querySelector(".ui-row-actions")), "next: Promote v0.9.0 to candidate");
    expect("and reads no plan", plans(closed.reads), []);
    const shipped = await row(PAST[0], "past", null, false);
    expect("a shipped release has no next step", shipped.document.querySelector(".ui-row-actions"), null);
  },
);

await check(
  "web::check_release_item_table_sorts_filters_and_narrows",
  "Tests sorts by its ratio with an item naming none last, the filter matches any column, narrow keeps ID, Title and Status, an open release draws its table with a linked ID, and at 390 px the table keeps ID, Title and Status, its cells wrap and the header stacks.",
  async (expect, problems) => {
    const tests = ITEM_COLUMNS.find((c) => c.key === "tests");
    expect("Tests ascending by ratio", sortRows(ITEM_COLUMNS.length ? ITEMS : [], tests, "asc").map((r) => r.code), ["FTR-2", "FTR-1", "FTR-9", "FTR-3"]);
    expect("Tests descending, none still last", sortRows(ITEMS, tests, "desc").map((r) => r.code), ["FTR-9", "FTR-1", "FTR-2", "FTR-3"]);
    expect("Tests reads green/named", tests.render(ITEMS[0]), "3/4");
    expect("filter by status", filterRows(ITEMS, ITEM_COLUMNS, "progress").map((r) => r.code), ["FTR-2"]);
    expect("filter by where it was carried", filterRows(ITEMS, ITEM_COLUMNS, "v0.9").map((r) => r.code), ["FTR-9"]);
    expect("narrow keeps", ITEM_COLUMNS.filter((c) => c.narrow === "keep").map((c) => c.label), ["ID", "Title", "Status"]);
    expect(
      "the other columns",
      ITEM_COLUMNS.filter((c) => c.narrow === "hide").map((c) => c.label),
      ["Kind", "Sprint", "Tests", "Approval", "Design review", "Docs folded", "Carried to"],
    );
    const open = await row(CURRENT[0], "current", "production", true);
    const table = open.document.querySelector(".release-body table.ui-table");
    if (!table) return problems.push("the open release draws no item table");
    expect("its caption", text(table.querySelector("caption")), "Items in v0.8.0");
    expect("one row per item", table.querySelectorAll("tbody tr").length, ITEMS.length);
    expect("the ID links to the item", table.querySelector("tbody tr td a")?.getAttribute("href"), "/p/shop/feature/FTR-1");
    // T6: at 390 px the table keeps ID, Title and Status, and each row's first cell folds the rest.
    const phone = await atWidth(390, () => row(CURRENT[0], "current", "production", true));
    const small = phone.document.querySelector(".release-body table.ui-table");
    if (!small) return problems.push("at 390 px the open release draws no item table");
    expect("at 390 px: the columns", [...small.querySelectorAll("thead th")].map(text), ["ID", "Title", "Status"]);
    expect(
      "at 390 px: the rest under More",
      [...(small.querySelector("tbody tr td details.ui-table-detail")?.querySelectorAll("dt") ?? [])].map(text),
      ["Kind", "Sprint", "Tests", "Approval", "Design review", "Docs folded", "Carried to"],
    );
    const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    if (!/white-space:\s*normal/.test(narrowRule(css, 720, ".release-body .ui-table td"))) problems.push("below 720 px the item table's cells do not wrap");
    const head = narrowRule(css, 640, ".release-head > .ui-row > .ui-row-head");
    if (!/grid-template-columns:\s*minmax\(0,\s*1fr\)\s*(;|$)/.test(head)) problems.push(`below 640 px the release header is not one column: ${head}`);
    if (!/min-width:\s*0/.test(narrowRule(css, 640, ".release-head .ui-stepper"))) problems.push("below 640 px the stepper keeps its minimum width");
  },
);

await check(
  "web::check_releases_url_names_the_tab_and_open_releases",
  "?tab=planned#v0.8.0 selects Planned and opens v0.8.0 alone, and the filter box shows and writes ?q=.",
  async (expect) => {
    const url = "/p/shop/releases?tab=planned&q=v0.8#v0.8.0";
    const page = await drawn("src/pages/ReleasesPage.tsx", "ReleasesBody", { ...PAGE, summary: summary(2, 3, 2) }, url);
    const selected = [...page.document.querySelectorAll('[role="tab"][aria-selected="true"]')].map((t) => text(t));
    expect("?tab=planned selects Planned", selected, ["Planned 3"]);
    expect("the page has one h1, Releases", [...page.document.querySelectorAll("h1")].map(text), ["Releases"]);
    expect("the filter box shows ?q=", page.document.querySelector('input[type="search"]')?.getAttribute("value"), "v0.8");
    expect("the filter writes q and keeps the tab", writeState("?tab=planned", { q: "v0.9" }, { q: "" }), "?tab=planned&q=v0.9");
    const slice = [release("v0.9.0"), release("v0.8.0")];
    const opened = await list("planned", slice, "/p/shop/releases?tab=planned#v0.8.0");
    expect("#v0.8.0 opens it alone", expanded(opened.document), { "v0.9.0": false, "v0.8.0": true });
    expect("each release header is an h2", [...opened.document.querySelectorAll("section.ui-row > .ui-row-head > h2")].length, 2);
  },
);

await check(
  "web::check_releases_page_reads_only_what_is_open",
  "the page reads the summary, one tab, an open release's items and plans through the shared cache with their tags, the body is drawn only inside AccordionRow's children, and nothing reads the whole board.",
  async (expect, problems) => {
    const sources = { "pages/ReleasesPage.tsx": src("pages/ReleasesPage.tsx") };
    for (const f of readdirSync(new URL("../src/pages/releases", import.meta.url))) sources[`pages/releases/${f}`] = src(`pages/releases/${f}`);
    for (const [rel, body] of Object.entries(sources)) {
      for (const bad of [/\bapi\.getProject\s*\(/, /\bapi\.getReleases\s*\(/, /\bapi\.getReleasePlan\s*\(/, /\bapi\.getReleaseItems\s*\(/]) {
        if (bad.test(body)) problems.push(`${rel} has ${bad}: reads go through useQuery and urls`);
      }
    }
    const want = [
      ["pages/ReleasesPage.tsx", /useQuery<ReleasesSummary>\(\s*project\s*,\s*urls\.releasesSummary\(project\)\s*,\s*\["releases", "items"\]\s*\)/, "the summary"],
      ["pages/ReleasesPage.tsx", /useQuery<Release\[\]>\(\s*project\s*,\s*urls\.releases\(project\s*,\s*scope\)\s*,\s*\["releases", "items", "gates"\]\s*\)/, "one tab's slice"],
      ["pages/releases/ReleaseBody.tsx", /useQuery<ReleaseItemRow\[\]>\(\s*project\s*,\s*urls\.releaseItems\(project\s*,\s*r\.version\)\s*,\s*\["releases", "items"\]\s*\)/, "an open release's items"],
      ["pages/releases/ReleaseActions.tsx", /useQuery<ReleasePlan>\(\s*project\s*,\s*urls\.releasePlan\(project\s*,\s*release\.version\)\s*,\s*TAGS\.releasePlan\s*\)/, "a release's plan"],
    ];
    for (const [rel, re, what] of want) if (!re.test(sources[rel])) problems.push(`${rel} does not read ${what} through useQuery with its tags`);
    // The plan reads the code repository, which sends no events, so it is re-read on every change (DEF-081).
    if (!TAGS.releasePlan.includes("*")) problems.push(`TAGS.releasePlan is ${JSON.stringify(TAGS.releasePlan)}: the plan reads the code repository, so it is tagged "*"`);
    if (!/<Tabs\b[^>]*param="tab"/.test(sources["pages/ReleasesPage.tsx"])) problems.push("ReleasesPage.tsx does not keep its tab in ?tab=");
    const r = sources["pages/releases/ReleaseRow.tsx"];
    if (!/<AccordionRow\b/.test(r)) problems.push("ReleaseRow.tsx is not an AccordionRow");
    if (!/\{\(\)\s*=>\s*<ReleaseBody\b/.test(r)) problems.push("ReleaseRow.tsx does not draw ReleaseBody only inside AccordionRow's children function");
    const all = Object.values(sources).join("\n");
    expect("ReleaseBody is drawn in one place", (all.match(/<ReleaseBody\b/g) ?? []).length, 1);

    const closed = await list("past", PAST, "/p/shop/releases?tab=released#v9");
    const bodies = [...closed.document.querySelectorAll(".ui-row-body")].filter((b) => b.innerHTML !== "").map((b) => b.id);
    expect("only the open release has a body", bodies, ["v0.7.0-body"]);
    expect("only it reads its items", itemReads(closed.reads), ["v0.7.0"]);
    expect("the page reads no whole board", closed.reads.filter((u) => /\/projects\/[^/]+$/.test(u)), []);
  },
);

await check(
  "web::check_release_header_is_two_lines_with_a_dot_stepper",
  "the header is a two-row grid: chips and a dot stepper with date and who first, the button and its line second.",
  async (expect, problems) => {
    const { document } = await row(CURRENT[0], "current", "production", false);
    const head = document.querySelector(".release-head > section.ui-row > .ui-row-head");
    if (!head) return problems.push("no .release-head header");
    const first = head.querySelector(":scope > .ui-row-meta");
    const second = head.querySelector(":scope > .ui-row-actions");
    if (!first || !second) return problems.push(`the header has no two lines: ${head.innerHTML.slice(0, 200)}`);
    expect("line one: the chips", [...first.querySelectorAll(".release-chips .ui-pill")].map(text).slice(0, 2), ["planned", "stage main"]);
    const stepper = first.querySelector("ol.ui-stepper");
    if (!stepper) return problems.push("line one has no ol stepper");
    expect("one step per stage", [...stepper.querySelectorAll("li")].map((li) => text(li.querySelector(".ui-step-label"))), STAGES);
    expect("a reached stage shows date and who", text(stepper.querySelector(".ui-step-done .ui-step-when")), "2026-11-16 · Ann");
    if (stepper.querySelector(".ui-pill")) problems.push("the stepper draws pills");
    expect("line two: the button and its line", [text(second.querySelector("button")), text(second.querySelector(".release-does"))], [
      "Promote v0.8.0 to production",
      "fast-forwards main to release/v0.8.0 (abc1234)",
    ]);
    const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
    const rule = /\.release-head > \.ui-row > \.ui-row-head\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    if (!/display:\s*grid/.test(rule) || !/grid-template-rows:\s*auto auto/.test(rule)) problems.push(`.release-head's header is not a two-row grid: ${rule}`);
    if (!/\.ui-row-actions\s*\{[^}]*grid-row:\s*2/.test(css)) problems.push("the actions are not on the second row");
    const ui = readFileSync(new URL("../src/ui/ui.css", import.meta.url), "utf8");
    if (!/\.ui-step-dot\s*\{[^}]*border-radius:\s*50%/.test(ui)) problems.push("the steps are not dots");
    if (!/\.ui-step::before\s*\{[^}]*height:\s*2px/.test(ui)) problems.push("the dots are not joined by a line");
  },
);

await check(
  "web::check_releases_read_plans_only_for_current_and_open_releases",
  "Current reads one plan per release, a closed Planned release none and an open one one, and Released none.",
  async (expect) => {
    const current = await list("current", CURRENT, "/p/shop/releases?tab=current");
    expect("Current: one plan per release", plans(current.reads).sort(), ["v0.7.1", "v0.8.0"]);
    const closed = await row(PLANNED[1], "planned", "candidate", false);
    expect("Planned, closed: none", plans(closed.reads), []);
    const opened = await row(PLANNED[1], "planned", "candidate", true);
    expect("Planned, opened: one", plans(opened.reads), ["v0.9.0"]);
    const planned = await list("planned", PLANNED, "/p/shop/releases?tab=planned");
    expect("the Planned tab: only its open first release", plans(planned.reads), ["v0.10.0"]);
    const past = await list("past", PAST, "/p/shop/releases?tab=released");
    expect("Released: none", plans(past.reads), []);
  },
);

await check(
  "web::check_releases_filter_reads_no_closed_table",
  "with ?q=v0.9 a closed release named otherwise is hidden, the open release's table is filtered, and no closed release's items are read.",
  async (expect, problems) => {
    const slice = [release("v0.10.0", { name: "Later" }), release("v0.9.0", { name: "Next" }), CURRENT[0]];
    const out = await list("planned", slice, "/p/shop/releases?tab=planned&q=v0.9#v0.8.0");
    expect("shown: the match and the open one", [...out.document.querySelectorAll("section.ui-row")].map((s) => s.id), ["v0.9.0", "v0.8.0"]);
    const rows = [...out.document.querySelectorAll("#v0\\.8\\.0-body table.ui-table tbody tr")].map((tr) => text(tr.querySelector("td")).split(" ")[0]);
    if (!rows.length) problems.push("the open release draws no table");
    expect("the open table gets the filter", rows, ["FTR-9"]);
    expect("only the open release's items are read", itemReads(out.reads), ["v0.8.0"]);
  },
);

await check(
  "web::check_release_blocking_reason_is_shown_once",
  "a reason and a first step the header shows are not repeated in the body; a different missing line still shows under Needs, and with none left there is no Needs heading.",
  async (expect) => {
    const notShownAbove = actions.notShownAbove ?? (() => ["notShownAbove is not exported"]);
    expect("drops a line already shown, trimmed", notShownAbove(["a", " b ", "c"], ["b", "x"]), ["a", "c"]);
    expect("keeps all when none was shown", notShownAbove(["a"], []), ["a"]);
    const count = (document, what) => text(document.body).split(what).length - 1;
    const both = await row(release("v1.1.0", { stage: "", branch: "release/v1.1.0" }), "current", "candidate", true);
    expect("the reason shows once", count(both.document, "X does not exist"), 1);
    expect("the first step shows once", count(both.document, "makes A from B"), 1);
    expect("the other missing line still shows", count(both.document, "Y"), 1);
    expect("Needs still shows", count(both.document, "Needs:"), 1);
    const only = await row(release("v1.2.0", { stage: "", branch: "release/v1.2.0" }), "current", "candidate", true);
    expect("only the duplicate: the reason once", count(only.document, "X does not exist"), 1);
    expect("only the duplicate: no Needs heading", count(only.document, "Needs:"), 0);
  },
);

await check(
  "web::check_releases_filter_wears_the_kit_search_style",
  "the page filter's input sits inside .ui-table-tools, the class each release table's filter wears.",
  async (expect) => {
    const out = await drawn(
      "src/pages/ReleasesPage.tsx",
      "ReleasesBody",
      { ...PAGE, summary: summary(1, 0, 0), cadence: null },
      "/p/shop/releases",
    );
    const input = out.document.querySelector(".releases-filter input[type=search]");
    expect("the filter input is drawn", input != null, true);
    expect("it sits inside .ui-table-tools", input?.closest(".ui-table-tools") != null, true);
  },
);

await closeRender();

if (failed) process.exit(1);
