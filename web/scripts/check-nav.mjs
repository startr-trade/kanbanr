#!/usr/bin/env node
//
// FTR-047: the monitor's grouped navigation, checked without React or a browser.
//   web::check_nav_lists_every_page_route_once — every project page route in main.tsx is the
//     path of exactly one NAV entry, and no two entries share a path or a key (R-1);
//   web::check_nav_hides_releases_without_cadence_and_empty_areas — Releases shows only once the
//     index says the project uses releases, and an area with no entry is not shown (R-1);
//   web::check_nav_marks_the_active_area — an item, milestone, status or document page marks the
//     entry it was reached from, compared on whole path segments (R-2);
//   web::check_nav_menus_open_one_at_a_time_and_close_on_escape — the menu reducer (R-3);
//   web::check_shell_links_are_not_duplicated — no path twice in NAV plus the global nav (R-5);
//   web::check_nav_puts_review_second_under_work — Work lists Board, then Review (R-7);
//   web::check_nav_menus_are_disclosure_buttons — the rendered shell: an area with several pages is
//     a disclosure button, marked current when it holds the page you are on (R-3);
//   web::check_nav_collapses_behind_one_button_at_the_narrow_breakpoint — at 720 px and below the
//     areas fold behind one ≡ button, and the badge and the theme toggle stay outside them (R-4);
//   web::check_nav_areas_with_several_pages_are_menu_buttons — one row of menu buttons above 720 px,
//     no second row and no sidebar (R-6);
//   web::check_nav_renders_a_one_page_area_as_a_link_named_for_the_area — never a one-entry menu (R-8);
//   web::check_narrow_top_bar_keeps_its_controls_on_one_row — at 390 px the brand's word is clipped
//     away, the here label shrinks, and the toggle, badge and theme keep their size (R-4);
//   web::check_narrow_pages_keep_wide_content_in_its_own_frame — the stylesheets give wide tables,
//     the timeline, code and pills their own frame at 720 px and below, so no page scrolls
//     sideways (R-4; the person's decision of 18 Nov 2026).
//
// The last four render the shell through scripts/lib/render.mjs (FTR-046).
//
// Usage: npm run check:nav
import { readFileSync } from "node:fs";
import { NAV, activeEntry, visibleAreas } from "../src/nav.ts";
import { navReducer } from "../src/navState.ts";
import { closeRender, render } from "./lib/render.mjs";

let failed = false;
function report(name, problems) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}

const P = "x";
const root = `/p/${P}`;
const entries = NAV.flatMap((a) => a.entries.map((e) => ({ area: a.key, ...e, at: e.path(P) })));

// R-1: every top-level project page route is exactly one entry's path.
{
  const problems = [];
  const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
  // Detail pages are reached from an entry, not listed in the nav. The repository docs pages
  // (FTR-004) are reached from Docs, whose entry matches /repo.
  const detail = [/^feature\/:/, /^milestone\/:/, /^state\/:/, /^docs\/file\//, /^docs\/folder\//, /^ui(\/|$)/, /^repo(\/|$)/];
  // An old URL kept as a redirect (`element: <Navigate …/>`, FTR-043's /schedule and /gantt) is not a page.
  const routes = [...main.matchAll(/path:\s*"p\/:project(?:\/([^"]*))?"(?:,\s*element:\s*<(\w+))?/g)]
    .filter((m) => m[2] !== "Navigate")
    .map((m) => m[1] ?? "")
    .filter((r) => !detail.some((d) => d.test(r)));
  if (routes.length === 0) problems.push("found no project routes in main.tsx");
  for (const r of routes) {
    const url = r === "" ? root : `${root}/${r}`;
    const hits = entries.filter((e) => e.at === url);
    if (hits.length !== 1) problems.push(`route p/:project/${r} is the path of ${hits.length} NAV entries, expected 1`);
  }
  for (const e of entries) {
    const r = e.at === root ? "" : e.at.slice(root.length + 1);
    if (!routes.includes(r)) problems.push(`NAV entry ${e.key} (${e.at}) has no page route in main.tsx`);
  }
  const dup = (xs) => xs.filter((x, i) => xs.indexOf(x) !== i);
  for (const k of new Set(dup(entries.map((e) => e.key)))) problems.push(`two NAV entries have the key ${k}`);
  for (const p of new Set(dup(entries.map((e) => e.at)))) problems.push(`two NAV entries have the path ${p}`);
  report("web::check_nav_lists_every_page_route_once", problems);
}

// R-1: conditional entries and empty areas.
{
  const problems = [];
  const ix = (releases) => ({ config: { cadence: { sprints: false, releases, git_model: false } } });
  const keys = (areas) => areas.flatMap((a) => a.entries.map((e) => e.key));
  const cases = [
    ["the index is not loaded", null, false],
    ["the project does not use releases", ix(false), false],
    ["the project uses releases", ix(true), true],
  ];
  for (const [what, index, shown] of cases) {
    const areas = visibleAreas(NAV, index);
    if (keys(areas).includes("releases") !== shown) problems.push(`Releases ${shown ? "hidden" : "shown"} when ${what}`);
    for (const a of areas) if (a.entries.length === 0) problems.push(`area ${a.key} shown with no entries when ${what}`);
    if (areas.some((a) => a.key === "evidence")) problems.push(`Evidence shown with no entries when ${what}`);
    if (!keys(areas).includes("board")) problems.push(`Board missing when ${what}`);
  }
  const fixture = [{ key: "plan", label: "Plan", entries: [{ key: "releases", label: "Releases", path: (p) => `/p/${p}/releases`, when: (i) => i.config.cadence.releases }] }];
  if (visibleAreas(fixture, ix(false)).length !== 0) problems.push("an area whose only entry is hidden is still shown");
  report("web::check_nav_hides_releases_without_cadence_and_empty_areas", problems);
}

// R-2: the active area and entry.
{
  const problems = [];
  const cases = [
    ["/p/x", { area: "overview", entry: "overview" }],
    ["/p/x/", { area: "overview", entry: "overview" }],
    ["/p/x/board", { area: "work", entry: "board" }],
    ["/p/x/feature/F-1", { area: "work", entry: "board" }],
    ["/p/x/state/Planned", { area: "work", entry: "board" }],
    ["/p/x/review", { area: "work", entry: "review" }],
    ["/p/x/milestones", { area: "work", entry: "milestones" }],
    ["/p/x/milestone/MS-1", { area: "work", entry: "milestones" }],
    ["/p/x/releases", { area: "plan", entry: "releases" }],
    ["/p/x/docs", { area: "records", entry: "docs" }],
    ["/p/x/docs/file/a.md", { area: "records", entry: "docs" }],
    ["/p/x/docs/folder/design", { area: "records", entry: "docs" }],
    ["/p/x/docsx", null],
    ["/p/xy/review", null],
    ["/portfolio", null],
    ["/", null],
  ];
  for (const [path, want] of cases) {
    const got = activeEntry(NAV, P, path);
    if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${path} → ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
  report("web::check_nav_marks_the_active_area", problems);
}

// R-3: one menu at a time; Escape closes and returns focus; choosing a page closes.
{
  const problems = [];
  const expect = (what, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${what}: ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  };
  const closed = { open: null, mobile: false, returnFocus: null };
  let s = navReducer(closed, { type: "toggle", area: "work" });
  expect("toggle work", s, { open: "work", mobile: false, returnFocus: null });
  s = navReducer(s, { type: "toggle", area: "plan" });
  expect("toggle work then plan", s, { open: "plan", mobile: false, returnFocus: null });
  s = navReducer(s, { type: "escape" });
  expect("escape from plan", s, { open: null, mobile: false, returnFocus: "plan" });
  s = navReducer({ ...s, returnFocus: null }, { type: "toggle", area: "plan" });
  s = navReducer(s, { type: "toggle", area: "plan" });
  expect("toggle plan twice", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "navigate" });
  expect("navigate", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "outside" });
  expect("outside click", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "records" }), { type: "close" });
  expect("close", s, closed);
  s = navReducer(navReducer(closed, { type: "toggle", area: "work" }), { type: "toggle-mobile" });
  expect("toggle-mobile with a menu open", s, { open: null, mobile: true, returnFocus: null });
  expect("escape from the mobile menu", navReducer(s, { type: "escape" }), { open: null, mobile: false, returnFocus: "mobile" });
  expect("toggle-mobile twice", navReducer(s, { type: "toggle-mobile" }), closed);
  expect("navigate from the mobile menu", navReducer(s, { type: "navigate" }), closed);
  expect("escape with nothing open", navReducer(closed, { type: "escape" }), closed);
  report("web::check_nav_menus_open_one_at_a_time_and_close_on_escape", problems);
}

// R-5: each destination once across NAV and the global nav (the literal NavLinks in App.tsx).
{
  const problems = [];
  const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
  const global = [...app.matchAll(/<NavLink\b[^>]*?\bto="([^"]+)"/g)].map((m) => m[1]);
  if (global.length === 0) problems.push("found no global nav links in App.tsx");
  const all = [...entries.map((e) => e.at), ...global];
  const seen = new Map();
  for (const p of all) seen.set(p, (seen.get(p) ?? 0) + 1);
  for (const [p, n] of seen) if (n > 1) problems.push(`${p} appears ${n} times in NAV and the global nav`);
  report("web::check_shell_links_are_not_duplicated", problems);
}

// R-7: Review is Work's second entry, after Board.
{
  const problems = [];
  const work = NAV.find((a) => a.key === "work");
  const keys = work ? work.entries.map((e) => e.key) : [];
  if (keys[0] !== "board" || keys[1] !== "review") problems.push(`Work lists ${JSON.stringify(keys)}, expected board then review first`);
  report("web::check_nav_puts_review_second_under_work", problems);
}

// The rendered shell. ThemeToggle reads the saved theme while rendering, so the browser globals it
// touches are stubbed; effects do not run on the server, so nothing else is reached.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };
globalThis.window ??= { matchMedia: () => ({ matches: false }) };

const stylesCss = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** The bodies of `@media (max-width: 720px)` blocks, and the stylesheet with them removed. */
function splitNarrow(css) {
  const narrow = [];
  let wide = "";
  let i = 0;
  const at = /@media\s*\(max-width:\s*720px\)\s*\{/g;
  let m;
  while ((m = at.exec(css)) !== null) {
    wide += css.slice(i, m.index);
    let depth = 1;
    let j = m.index + m[0].length;
    for (; j < css.length && depth > 0; j++) {
      if (css[j] === "{") depth++;
      else if (css[j] === "}") depth--;
    }
    narrow.push(css.slice(m.index + m[0].length, j - 1));
    i = j;
    at.lastIndex = j;
  }
  wide += css.slice(i);
  return { narrow: narrow.join("\n"), wide };
}

/** The declarations of every rule whose selector list includes exactly `selector`. */
function declarations(css, selector) {
  const out = [];
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selectors = m[1].split(",").map((x) => x.trim());
    if (selectors.includes(selector)) out.push(m[2]);
  }
  return out.join(";");
}

const { narrow, wide } = splitNarrow(stylesCss);

/** The body of every `@media (max-width: <px>px)` block in `css`. */
function narrowBlock(css, px) {
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
    out.push(css.slice(m.index + m[0].length, j - 1));
    at.lastIndex = j;
  }
  return out.join("\n");
}

try {
  // R-3: at /p/x/milestones, Work is a closed disclosure button marked current, and Milestones is the page.
  {
    const problems = [];
    const { document } = await render("src/App.tsx", "ShellNav", { project: P, index: null }, { url: `${root}/milestones` });
    const work = document.querySelector('button.nav-area[aria-controls="nav-work"]');
    if (!work) problems.push("no Work button with aria-controls=nav-work");
    else {
      if (work.getAttribute("aria-expanded") !== "false") problems.push(`Work's aria-expanded is ${work.getAttribute("aria-expanded")}, expected false`);
      if (!work.classList.contains("current")) problems.push(`Work's class is "${work.className}", expected it to include current`);
      if (!/^Work\s*▾$/.test(work.textContent.trim())) problems.push(`Work's button reads "${work.textContent}", expected "Work ▾"`);
      const caret = [...work.querySelectorAll("span")].find((x) => x.textContent.includes("▾"));
      if (caret?.getAttribute("aria-hidden") !== "true") problems.push("the ▾ is not in an aria-hidden span");
    }
    const list = document.getElementById("nav-work");
    if (!list) problems.push("no list with id nav-work");
    else if (!list.hasAttribute("hidden")) problems.push("Work's list is not hidden while its menu is closed");
    const current = [...document.querySelectorAll('[aria-current="page"]')];
    if (current.length !== 1 || current[0].textContent.trim() !== "Milestones" || current[0].getAttribute("href") !== `${root}/milestones`)
      problems.push(`aria-current=page is on ${JSON.stringify(current.map((x) => x.textContent))}, expected only Milestones`);
    for (const b of document.querySelectorAll("button.nav-area")) {
      if (b !== work && b.classList.contains("current")) problems.push(`${b.textContent} is marked current too`);
    }
    report("web::check_nav_menus_are_disclosure_buttons", problems);
  }

  // R-4: the narrow breakpoint folds the areas behind one button; the badge and the theme stay out.
  {
    const problems = [];
    const hideAreas = declarations(narrow, ".nav-areas");
    if (!/display:\s*none/.test(hideAreas)) problems.push("@media (max-width: 720px) does not hide .nav-areas");
    if (!/display:\s*(inline-block|inline-flex|block|flex)/.test(declarations(narrow, ".nav-toggle"))) problems.push("@media (max-width: 720px) does not show .nav-toggle");
    if (!/display:\s*none/.test(declarations(wide, ".nav-toggle"))) problems.push(".nav-toggle is not hidden above 720 px");
    for (const m of stylesCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (/\.nav\b|\.nav-/.test(m[1]) && /overflow-x:\s*auto/.test(m[2])) problems.push(`${m[1].trim()} scrolls sideways (overflow-x: auto)`);
    }
    if (/\.tabs\b/.test(stylesCss)) problems.push("styles.css still has .tabs rules");
    const { document } = await render("src/App.tsx", "default", {}, { url: `${root}/review` });
    const toggle = document.querySelector("button.nav-toggle");
    if (!toggle) problems.push("no .nav-toggle in the shell");
    else {
      if (toggle.getAttribute("aria-controls") !== "nav-areas") problems.push(`.nav-toggle's aria-controls is ${toggle.getAttribute("aria-controls")}`);
      if (toggle.getAttribute("aria-expanded") !== "false") problems.push(".nav-toggle is not aria-expanded=false while closed");
      if (toggle.getAttribute("aria-label") !== "Menu") problems.push(".nav-toggle has no Menu label");
      if (toggle.closest(".nav-areas")) problems.push(".nav-toggle is inside the panel it opens");
    }
    const areas = document.getElementById("nav-areas");
    if (!areas || !areas.classList.contains("nav-areas")) problems.push("no .nav-areas with id nav-areas");
    else {
      const theme = document.querySelector(".theme-toggle");
      if (!theme) problems.push("no theme toggle in the shell");
      else if (areas.contains(theme)) problems.push("the theme toggle is inside .nav-areas");
      // NeedsYouBadge draws nothing until FTR-048; it is rendered after the nav, outside the panel.
      const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
      const shellAt = app.indexOf("<ShellNav ");
      const badgeAt = app.indexOf("<NeedsYouBadge ");
      const buildAt = app.indexOf('className="build"');
      if (shellAt < 0 || badgeAt < 0 || !(shellAt < badgeAt && badgeAt < buildAt)) problems.push("App.tsx does not render NeedsYouBadge between the nav and the build label");
      const here = document.querySelector(".nav-here");
      if (!here || here.textContent.replace(/[·\s]/g, "") !== "Review") problems.push(`the here label reads "${here?.textContent}", expected "· Review"`);
      const global = [...areas.querySelectorAll(".global-nav a")].map((a) => a.textContent.trim());
      if (JSON.stringify(global) !== JSON.stringify(["Projects", "Portfolio"])) problems.push(`the panel's global views are ${JSON.stringify(global)}`);
    }
    const navs = document.querySelectorAll("nav.nav");
    if (navs.length !== 1) problems.push(`${navs.length} nav.nav elements, expected one serving both widths`);
    report("web::check_nav_collapses_behind_one_button_at_the_narrow_breakpoint", problems);
  }

  // R-6: above 720 px, each area with several pages is one menu button on a single row.
  {
    const problems = [];
    const ix = { config: { cadence: { sprints: false, releases: true, git_model: false } } };
    const { document } = await render("src/App.tsx", "ShellNav", { project: P, index: ix }, { url: `${root}/docs` });
    for (const area of visibleAreas(NAV, ix)) {
      if (area.entries.length < 2) continue;
      const buttons = document.querySelectorAll(`button.nav-area[aria-controls="nav-${area.key}"]`);
      if (buttons.length !== 1) {
        problems.push(`${area.label} has ${buttons.length} menu buttons, expected 1`);
        continue;
      }
      if (!buttons[0].hasAttribute("aria-expanded")) problems.push(`${area.label}'s button has no aria-expanded`);
      const list = document.getElementById(`nav-${area.key}`);
      if (!list || list.tagName !== "UL" || list.parentElement !== buttons[0].parentElement) problems.push(`${area.label}'s list is not a ul beside its button`);
      else {
        const links = [...list.querySelectorAll("a")].map((a) => a.getAttribute("href"));
        const want = area.entries.map((e) => e.path(P));
        if (JSON.stringify(links) !== JSON.stringify(want)) problems.push(`${area.label}'s list links ${JSON.stringify(links)}, expected ${JSON.stringify(want)}`);
      }
    }
    const navRule = declarations(wide, ".nav");
    if (!/display:\s*flex/.test(navRule) || !/flex-wrap:\s*nowrap/.test(navRule)) problems.push(`.nav above 720 px is not one flex row with flex-wrap: nowrap (${navRule.trim()})`);
    if (!/flex-wrap:\s*nowrap/.test(declarations(wide, ".nav-areas"))) problems.push(".nav-areas above 720 px may wrap to a second row");
    // A page may keep its own side panel (the docs tree); the shell's navigation never becomes one.
    for (const m of stylesCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (/sidebar/i.test(m[1]) && /\bnav\b|\.nav-/.test(m[1])) problems.push(`${m[1].trim()} is a sidebar rule for the navigation`);
    }
    for (const sel of [".nav", ".nav-areas"]) {
      if (/flex-direction:\s*column/.test(declarations(wide, sel))) problems.push(`${sel} is a column above 720 px`);
    }
    report("web::check_nav_areas_with_several_pages_are_menu_buttons", problems);
  }

  // R-8: an area with one page is a plain link named for the area.
  {
    const problems = [];
    const fixture = [
      { key: "work", label: "Work", entries: [{ key: "board", label: "Board", path: (p) => `/p/${p}` }, { key: "review", label: "Review", path: (p) => `/p/${p}/review` }] },
      { key: "architecture", label: "Architecture", entries: [{ key: "workflow", label: "Workflow", path: (p) => `/p/${p}/workflow` }] },
    ];
    const { document } = await render("src/App.tsx", "ShellNav", { project: P, index: null, nav: fixture }, { url: `${root}/workflow` });
    const links = [...document.querySelectorAll("a")].filter((a) => a.getAttribute("href") === `${root}/workflow`);
    if (links.length !== 1) problems.push(`${links.length} links to ${root}/workflow, expected 1`);
    else {
      const a = links[0];
      if (a.textContent.trim() !== "Architecture") problems.push(`the link reads "${a.textContent}", expected "Architecture"`);
      if (a.getAttribute("aria-current") !== "page") problems.push("the one-page area's link is not aria-current=page on its page");
      if (!a.classList.contains("current")) problems.push("the one-page area is not marked current on its page");
    }
    if (document.querySelector('button[aria-controls="nav-architecture"]') || document.getElementById("nav-architecture")) problems.push("Architecture is rendered as a menu");
    // The narrow layout's here label names the page you are on ("· Workflow"); the area item never does.
    document.querySelector(".nav-here")?.remove();
    if (/Workflow/.test(document.body.textContent)) problems.push("the area item shows the page's own label, Workflow");
    if (!document.querySelector('button[aria-controls="nav-work"]')) problems.push("the fixture's two-page Work area is not a menu button");
    report("web::check_nav_renders_a_one_page_area_as_a_link_named_for_the_area", problems);
  }

  // R-4 (18 Nov 2026): at 390 px the top bar keeps one row.
  {
    const problems = [];
    const { document } = await render("src/App.tsx", "default", {}, { url: `${root}/review` });
    const word = document.querySelector(".brand a .brand-word");
    if (!word) problems.push("the brand link has no .brand-word span around the word kanbanr");
    else if (word.textContent.trim() !== "kanbanr") problems.push(`.brand-word reads "${word.textContent}", expected "kanbanr"`);
    const brand = document.querySelector(".brand a");
    if (brand && brand.getAttribute("aria-label") !== "kanbanr — all projects") problems.push("the brand link lost its accessible name");
    const small = narrowBlock(stylesCss, 480);
    const hide = declarations(small, ".brand-word");
    if (!hide) problems.push("no .brand-word rule at @media (max-width: 480px)");
    else {
      if (/display:\s*none/.test(hide)) problems.push(".brand-word is display: none at 480 px, which drops the link's name; clip it instead");
      if (!/clip(-path)?:/.test(hide) || !/position:\s*absolute/.test(hide)) problems.push(".brand-word is not hidden with the clip pattern (position: absolute plus clip)");
    }
    const here = declarations(narrow, ".nav-here");
    if (!/min-width:\s*0/.test(here)) problems.push(".nav-here is not min-width: 0 at 720 px");
    if (!/text-overflow:\s*ellipsis/.test(here)) problems.push(".nav-here has no ellipsis at 720 px");
    if (!/flex:\s*1 1 auto/.test(here)) problems.push(".nav-here is not flex: 1 1 auto at 720 px");
    for (const sel of [".nav-toggle", ".needs-you", ".theme-toggle"]) {
      const d = declarations(narrow, sel) + ";" + declarations(wide, sel);
      if (!/flex:\s*none/.test(d)) problems.push(`${sel} is not flex: none`);
    }
    report("web::check_narrow_top_bar_keeps_its_controls_on_one_row", problems);
  }

  // R-4 (18 Nov 2026): wide content scrolls in its own frame, never the page.
  {
    const problems = [];
    const sheets = {
      "styles.css": stylesCss,
      "ui/ui.css": readFileSync(new URL("../src/ui/ui.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, ""),
      "pages/board/board.css": readFileSync(new URL("../src/pages/board/board.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, ""),
    };
    // [file, selector, declaration pattern, where: "narrow" (inside 720 px) or "any"]
    const want = [
      ["styles.css", ".content", /min-width:\s*0/, "any"],
      ["styles.css", ".overview-page", /min-width:\s*0/, "any"],
      ["styles.css", ".overview-col", /min-width:\s*0/, "any"],
      ["ui/ui.css", ".ui-table-wrap", /max-width:\s*100%/, "any"],
      ["ui/ui.css", ".ui-table-wrap", /min-width:\s*0/, "any"],
      ["ui/ui.css", ".ui-matrix", /display:\s*block/, "narrow"],
      ["ui/ui.css", ".ui-matrix", /overflow-x:\s*auto/, "narrow"],
      ["ui/ui.css", ".ui-matrix", /max-width:\s*100%/, "narrow"],
      ["ui/ui.css", ".ui-pill", /max-width:\s*100%/, "any"],
      ["ui/ui.css", ".ui-pill", /text-overflow:\s*ellipsis/, "any"],
      ["ui/ui.css", ".ui-gallery-themes", /grid-template-columns:\s*minmax\(0,\s*1fr\)/, "narrow"],
      ["ui/ui.css", ".ui-gallery-theme", /min-width:\s*0/, "narrow"],
      ["styles.css", ".plan-tab .ui-timeline", /overflow-x:\s*auto/, "narrow"],
      ["styles.css", ".plan-tab .ui-timeline", /max-width:\s*100%/, "narrow"],
      ["styles.css", ".markdown table", /display:\s*block/, "narrow"],
      ["styles.css", ".markdown table", /overflow-x:\s*auto/, "narrow"],
      ["styles.css", ".markdown table", /max-width:\s*100%/, "narrow"],
      ["styles.css", ".markdown code", /overflow-wrap:\s*anywhere/, "any"],
      ["styles.css", ".ui-table code", /overflow-wrap:\s*anywhere/, "any"],
      ["styles.css", ".markdown pre", /overflow-x:\s*auto/, "any"],
      ["styles.css", ".markdown pre", /max-width:\s*100%/, "any"],
      ["styles.css", ".tasktext", /overflow-wrap:\s*anywhere/, "any"],
      ["styles.css", ".tasktext", /min-width:\s*0/, "any"],
      ["styles.css", ".per-page", /flex-wrap:\s*wrap/, "any"],
      ["styles.css", ".per-page select", /max-width:\s*100%/, "any"],
      ["styles.css", ".board", /grid-template-columns:\s*minmax\(0,\s*1fr\)/, "narrow"],
      ["styles.css", ".column", /min-width:\s*0/, "any"],
      ["styles.css", ".activity-meta", /white-space:\s*normal/, "narrow"],
      ["styles.css", ".todo-tile", /overflow-wrap:\s*anywhere/, "narrow"],
      ["styles.css", ".activity-list", /overflow-wrap:\s*anywhere/, "narrow"],
      ["styles.css", ".plan-flow", /overflow-wrap:\s*anywhere/, "narrow"],
    ];
    const parts = Object.fromEntries(Object.entries(sheets).map(([k, v]) => [k, splitNarrow(v)]));
    for (const [file, sel, re, where] of want) {
      const { narrow: n, wide: w } = parts[file];
      const hit = where === "narrow" ? declarations(n, sel) : declarations(n, sel) + ";" + declarations(w, sel);
      if (!re.test(hit)) problems.push(`${file}: ${sel} lacks ${re.source}${where === "narrow" ? " at 720 px and below" : ""}`);
    }
    // Review is FTR-038's: this task adds no Review rule to the shared stylesheets.
    for (const [file, css] of Object.entries(sheets)) {
      if (file === "styles.css" || file === "ui/ui.css") continue;
      if (/\.review-|\.ui-row-/.test(css)) problems.push(`${file} holds a Review or row rule`);
    }
    report("web::check_narrow_pages_keep_wide_content_in_its_own_frame", problems);
  }
} catch (e) {
  failed = true;
  console.error(`check:nav — rendering the shell failed: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

if (failed) process.exit(1);
