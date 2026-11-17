#!/usr/bin/env node
//
// FTR-004: docs read at the position you pick, in the monitor.
//   web::position (R-3, R-4, R-7, R-8) — positionLine names path, position and source for each shape
//     a pick resolves to; missingLine says whether main has a missing doc; withAt, currentPick and
//     the session's remembered pick carry a position from page to page; markdown rewrites a
//     relative docs/ link to the repository doc page at the position and leaves a board link, a
//     board doc key and another site alone; RepoDocBody puts the banner before the text (read from its source); a
//     sign-off's docs link to the commit it was signed at; DocPage says a board doc has no position.
//   web::position_picker_sits_in_the_docs_page_header_as_a_select (R-10) — DocsBody, RepoDocsBody
//     and RepoDocBody each draw the picker as a <select> with its typed field in the page header,
//     unavailable positions disabled with their reason; no other page imports it, so the item page
//     has none; the picker's rules use the Tabs' tokens (control height, focus ring, accent
//     underline).
//
// The pages render through scripts/lib/render.mjs (FTR-046).
//
// Usage: npm run check:position
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import { markdownToSafeHtml } from "../src/markdownHtml.ts";
import { makeIndex } from "../src/boardRefs.ts";
import {
  AT_KEY,
  BOARD_DOC_BANNER,
  currentPick,
  missingLine,
  positionLine,
  rememberPick,
  rememberedPick,
  repoDocHref,
  rewriteRepoLinks,
  signoffDocs,
  withAt,
} from "../src/position.ts";
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
const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
const srcDir = new URL("../src/", import.meta.url).pathname;
const read = (rel) => readFileSync(join(srcDir, rel), "utf8");

const C = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d";
const D = "f".repeat(64);
const pos = (commit, dirty = null) => ({ repo: "x", commit, dirty });
const picked = (over) => ({
  pick: "main",
  kind: "main",
  position: pos(C),
  source: "git",
  label: "main @ 1a2b3c4",
  resolved_from: "branch tip",
  ...over,
});
const doc = (over) => ({
  ...picked({}),
  path: "docs/guide/using/x.md",
  text: "# Using x\n\nSee [the store](docs/system/store.md) and [FTR-004](/p/x/feature/FTR-004).\n",
  blob: "b".repeat(40),
  missing: false,
  why: "",
  at_main: null,
  ...over,
});
const picks = [
  { pick: "main", kind: "main", label: "main @ 1a2b3c4", available: true, why: "" },
  { pick: "worktree", kind: "worktree", label: "working tree @ 1a2b3c4+ffff", available: true, why: "" },
  { pick: "SP-003", kind: "sprint", label: "SP-003 · closed @ 77ab001", available: true, why: "" },
  { pick: "v0.9.0", kind: "release", label: "v0.9.0", available: false, why: "v0.9.0 has no branch or tag yet" },
];
const fakeStore = () => {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v) };
};

try {
  // web::position
  {
    const problems = [];
    const eq = (what, got, want) => {
      if (got !== want) problems.push(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
    };

    // R-3: the banner names the path, the position and where the bytes came from.
    const path = "docs/guide/using/x.md";
    eq(
      "item",
      positionLine(picked({ pick: "FTR-004", kind: "item", label: "FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4", resolved_from: "merge commit recorded on the item" }), path),
      `${path} · at FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4 · from git (merge commit recorded on the item)`,
    );
    eq("main", positionLine(picked({}), path), `${path} · at main @ 1a2b3c4 · from git (branch tip)`);
    eq(
      "release",
      positionLine(picked({ pick: "v0.6.0", kind: "release", label: "v0.6.0 · tag v0.6.0 @ 9fe1d20", resolved_from: "tag" }), path),
      `${path} · at v0.6.0 · tag v0.6.0 @ 9fe1d20 · from git (tag)`,
    );
    eq(
      "sprint",
      positionLine(picked({ pick: "SP-003", kind: "sprint", label: "SP-003 · closed @ 77ab001", resolved_from: "closing commit recorded on the sprint" }), path),
      `${path} · at SP-003 · closed @ 77ab001 · from git (closing commit recorded on the sprint)`,
    );
    eq(
      "working tree, uncommitted",
      positionLine(picked({ pick: "worktree", kind: "worktree", source: "disk", position: pos(C, D), label: "working tree @ 1a2b3c4+ffffffff…", resolved_from: "HEAD and the uncommitted changes" }), path),
      `${path} · working tree @ 1a2b3c4+ffffffff… · from disk, with uncommitted changes`,
    );
    eq(
      "working tree, clean",
      positionLine(picked({ pick: "worktree", kind: "worktree", source: "disk", label: "working tree @ 1a2b3c4" }), path),
      `${path} · working tree @ 1a2b3c4 · from disk`,
    );
    eq("a doc carries its own path", positionLine(doc({})), positionLine(picked({}), "docs/guide/using/x.md"));
    eq("a tree names its root", positionLine(picked({}), "docs/"), "docs/ · at main @ 1a2b3c4 · from git (branch tip)");

    // R-4: a missing doc says so, and whether main has it.
    const missing = { path: "docs/system/store.md", label: "v0.6.0 · tag v0.6.0 @ 9fe1d20" };
    eq("missing, main has it", missingLine({ ...missing, at_main: true }), "docs/system/store.md is not in the repository at v0.6.0 (tag v0.6.0 @ 9fe1d20). main has it.");
    eq("missing, main lacks it", missingLine({ ...missing, at_main: false }), "docs/system/store.md is not in the repository at v0.6.0 (tag v0.6.0 @ 9fe1d20). main does not have it either.");
    eq("missing at main", missingLine({ path: "docs/a.md", label: "main @ 1a2b3c4", at_main: null }), "docs/a.md is not in the repository at main @ 1a2b3c4.");

    // The pick travels: ?at= first, then the one kept for the session, then main.
    const store = fakeStore();
    eq("default", currentPick("", rememberedPick(store)), "main");
    rememberPick("FTR-004", store);
    eq("remembered under the key", store.getItem(AT_KEY), "FTR-004");
    eq("remembered", currentPick("", rememberedPick(store)), "FTR-004");
    eq("the URL wins", currentPick("?at=SP-003", rememberedPick(store)), "SP-003");
    eq("an empty at is no pick", currentPick("?at=", "v0.6.0"), "v0.6.0");
    rememberPick("  ", store);
    eq("a blank pick is not kept", store.getItem(AT_KEY), "FTR-004");
    eq("a browser with no storage", rememberedPick(null), "");
    eq("withAt", withAt("/p/x/repo/docs/a.md", "FTR-004"), "/p/x/repo/docs/a.md?at=FTR-004");
    eq("withAt keeps the hash", withAt("/p/x/repo/docs/a.md#s", "SP-003"), "/p/x/repo/docs/a.md?at=SP-003#s");
    eq("withAt replaces", withAt("/p/x/repo?at=main&q=1", "v0.6.0"), "/p/x/repo?at=v0.6.0&q=1");
    eq("withAt encodes a position text", withAt("/p/x/repo", `${C}+${D}`), `/p/x/repo?at=${C}%2B${D}`);
    eq("repoDocHref", repoDocHref("x", "docs/guide/a b.md", "FTR-004"), "/p/x/repo/docs/guide/a%20b.md?at=FTR-004");

    // R-8: markdown. A relative link under docs/ opens the repository doc at the pick; a board link,
    // a board doc key and another site are left alone, and a docs/ link is no board reference.
    const { window } = new JSDOM("");
    const purifier = createDOMPurify(window);
    const index = makeIndex({
      project: "x",
      items: [{ code: "FTR-004", requirements: [] }],
      milestones: [],
      adrs: [],
      goals: [],
      docs: ["architecture/data.md", "guide/using/x.md"],
      codes: ["FTR"],
    });
    const source = [
      "[a](docs/guide/using/x.md)",
      "[b](./docs/system/store.md#s)",
      "[c](/p/x/docs/file/architecture/data.md)",
      "[d](architecture/data.md)",
      "[e](https://example.com/docs/x.md)",
      "[f](../docs/x.md)",
      "[g](docs/../secret.md)",
    ].join("\n\n");
    const html = rewriteRepoLinks(markdownToSafeHtml(source, undefined, purifier, index, 1), "x", "FTR-004");
    const hrefs = [...html.matchAll(/<a\b[^>]*?\bhref="([^"]*)"/g)].map((m) => m[1]);
    const want = [
      "/p/x/repo/docs/guide/using/x.md?at=FTR-004",
      "/p/x/repo/docs/system/store.md?at=FTR-004#s",
      "/p/x/docs/file/architecture/data.md",
      "/p/x/docs/file/architecture/data.md",
      "https://example.com/docs/x.md",
      "../docs/x.md",
      "docs/../secret.md",
    ];
    if (JSON.stringify(hrefs) !== JSON.stringify(want)) problems.push(`markdown links: got ${JSON.stringify(hrefs)}, expected ${JSON.stringify(want)}`);
    if (/board-ref broken/.test(html.split("\n").find((l) => l.includes("docs/guide/using/x.md")) ?? "")) problems.push("a docs/ link is marked as a broken board reference");
    // The component hands the pick (`at`, else the URL's or the session's) to that rewrite, and the
    // repository doc page draws its banner before the text. Markdown cannot be drawn here (DOMPurify
    // needs a browser's window and React cannot share it), so the source is what is read.
    const mdSrc = read("components/Markdown.tsx");
    if (!/const pick = at \?\? currentPick\(search\)/.test(mdSrc) || !/rewriteRepoLinks\(/.test(mdSrc)) problems.push("Markdown does not rewrite repository doc links at its pick");
    const pageSrc = read("pages/RepoDocPage.tsx");
    const bannerAt = pageSrc.indexOf("positionLine(found)");
    const textAt = pageSrc.indexOf("<Markdown");
    if (bannerAt < 0 || textAt < 0 || bannerAt > textAt) problems.push("RepoDocBody does not draw the banner before the text");
    if (!/<Markdown[^>]*\bat=\{pick\}/s.test(pageSrc)) problems.push("RepoDocBody's text does not open its links at the page's pick");
    if (!/<Markdown[^>]*\bat=\{feature\.code\}/s.test(read("pages/FeaturePage.tsx"))) problems.push("the item page's spec does not open repository docs at the item");

    // R-4: a missing doc says so on the page, and keeps the picker to choose another position.
    const gone = await render("src/pages/RepoDocPage.tsx", "RepoDocBody", {
      project: "x", path: "docs/system/store.md", pick: "v0.6.0", picks,
      doc: { data: doc({ path: "docs/system/store.md", pick: "v0.6.0", kind: "release", label: "v0.6.0 · tag v0.6.0 @ 9fe1d20", text: null, blob: null, missing: true, why: "gone", at_main: true }), error: null, loading: false },
    }, { url: "/p/x/repo/docs/system/store.md?at=v0.6.0" });
    if (!text(gone.document.querySelector(".ui-position-banner")).startsWith("docs/system/store.md is not in the repository at v0.6.0 (tag v0.6.0 @ 9fe1d20). main has it."))
      problems.push(`a missing doc says ${JSON.stringify(text(gone.document.querySelector(".ui-position-banner")))}`);
    if (gone.document.querySelector(".doc-article")) problems.push("a missing doc still draws text");
    if (!gone.document.querySelector("select")) problems.push("a missing doc has no picker to choose another position");

    // R-3: a board doc says it has no position.
    if (!/BOARD_DOC_BANNER/.test(read("pages/DocPage.tsx"))) problems.push("DocPage draws no board banner");
    eq("board banner", BOARD_DOC_BANNER, "board doc · one record, the same at every position");

    // R-7: the sign-off line, and its links open what was signed.
    eq("no position, no line", signoffDocs("x", { position: "", docs_read: {} }), null);
    eq("absent position", signoffDocs("x", {}), null);
    const so = signoffDocs("x", { position: C, docs_read: { "docs/guide/b.md": "1", "docs/guide/a.md": "2" } });
    eq("sign-off line", so?.line, "docs read at 1a2b3c4");
    eq("sign-off links", JSON.stringify(so?.links), JSON.stringify([
      { path: "docs/guide/a.md", href: `/p/x/repo/docs/guide/a.md?at=${C}` },
      { path: "docs/guide/b.md", href: `/p/x/repo/docs/guide/b.md?at=${C}` },
    ]));
    eq("dirty sign-off opens at its commit", signoffDocs("x", { position: `${C}+${D}`, docs_read: { "docs/a.md": "1" } })?.links[0].href, `/p/x/repo/docs/a.md?at=${C}`);
    const line = await render("src/pages/FeaturePage.tsx", "SignoffDocs", { project: "x", signoff: { id: "docs-folded", by: "p", at: "2026-11-17T00:00:00Z", rev: "r", position: C, docs_read: { "docs/guide/a.md": "2" } } });
    const link = line.document.querySelector("a");
    if (!/docs read at 1a2b3c4/.test(text(line.document.body))) problems.push(`the sign-off line says ${JSON.stringify(text(line.document.body))}`);
    if (link?.getAttribute("href") !== `/p/x/repo/docs/guide/a.md?at=${C}`) problems.push(`the sign-off's doc links to ${link?.getAttribute("href")}`);
    const none = await render("src/pages/FeaturePage.tsx", "SignoffDocs", { project: "x", signoff: { id: "spec", by: "p", at: "2026-11-17T00:00:00Z", rev: "r" } });
    if (text(none.document.body) !== "") problems.push("a sign-off with no position draws a line");

    // The routes.
    const main = read("main.tsx");
    if (!/path:\s*"p\/:project\/repo",\s*element:\s*<RepoDocsPage\s*\/>/.test(main)) problems.push("main.tsx does not route p/:project/repo");
    if (!/path:\s*"p\/:project\/repo\/\*",\s*element:\s*<RepoDocPage\s*\/>/.test(main)) problems.push("main.tsx does not route p/:project/repo/*");
    if (!/path:\s*"p\/:project\/docs\/file\/\*",\s*element:\s*<DocPage\s*\/>/.test(main)) problems.push("main.tsx lost the board doc route");
    if (!/path:\s*"p\/:project",\s*element:\s*<OverviewPage\s*\/>/.test(main)) problems.push("main.tsx lost the Overview route (FTR-048)");
    if (!/path:\s*"p\/:project\/board",\s*element:\s*<BoardPage\s*\/>/.test(main)) problems.push("main.tsx lost the Board route (FTR-048)");
    report("web::position", problems);
  }

  // web::position_picker_sits_in_the_docs_page_header_as_a_select
  {
    const problems = [];
    const views = [
      ["DocsBody", "src/pages/DocsPage.tsx", { project: "x", folderPath: "", tree: { data: { name: "docs", path: "", description: "", folders: [], docs: [] }, error: null, loading: false }, pick: "main", picks }],
      ["RepoDocsBody", "src/pages/RepoDocsPage.tsx", { project: "x", pick: "SP-003", tree: { data: { ...picked({ pick: "SP-003", kind: "sprint", label: "SP-003 · closed @ 77ab001" }), root: "docs/", files: ["docs/guide/a.md"] }, error: null, loading: false }, picks }],
      ["RepoDocBody", "src/pages/RepoDocPage.tsx", { project: "x", path: "docs/guide/a.md", pick: "FTR-004", doc: { data: doc({ text: null, blob: null, missing: true, at_main: true }), error: null, loading: false }, picks }],
    ];
    for (const [name, file, props] of views) {
      const { document } = await render(file, name, props, { url: "/p/x/docs" });
      const head = document.querySelector(".page-head");
      const select = head?.querySelector("select.ui-position-select");
      if (!head) problems.push(`${name} has no page header`);
      else if (!select) problems.push(`${name}'s header has no picker <select>`);
      else {
        const opts = [...select.querySelectorAll("option")];
        for (const key of ["main", "worktree", "SP-003", "v0.9.0"]) if (!opts.some((o) => o.getAttribute("value") === key)) problems.push(`${name}'s picker lacks ${key}`);
        const off = opts.find((o) => o.getAttribute("value") === "v0.9.0");
        if (off && !(off.hasAttribute("disabled") && /no branch or tag yet/.test(off.getAttribute("title") ?? ""))) problems.push(`${name}'s picker does not disable an unavailable position with its reason`);
        const current = props.pick;
        if (select.value !== undefined && !opts.some((o) => o.getAttribute("value") === current)) problems.push(`${name}'s picker does not offer the current pick ${current}`);
        const selected = opts.filter((o) => o.hasAttribute("selected")).map((o) => o.getAttribute("value"));
        if (selected.length !== 1 || selected[0] !== current) problems.push(`${name}'s picker has ${JSON.stringify(selected)} selected, expected ${current}`);
        if (!head.querySelector('input[type="text"].ui-position-typed')) problems.push(`${name}'s header has no typed field`);
        if (!document.querySelector('label[for="position-select"]')) problems.push(`${name}'s select has no label`);
      }
      if (document.querySelectorAll("h1").length !== 1) problems.push(`${name} has ${document.querySelectorAll("h1").length} h1s`);
    }
    // Nowhere else: the item page carries no picker, and no other file imports it.
    const files = [];
    const walk = (d) => {
      for (const f of readdirSync(d)) {
        const p = join(d, f);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.tsx?$/.test(f)) files.push(p);
      }
    };
    walk(srcDir);
    const users = files.filter((f) => /\bPositionPicker\b/.test(readFileSync(f, "utf8"))).map((f) => f.slice(srcDir.length)).sort();
    const allowed = ["components/PositionPicker.tsx", "pages/DocsPage.tsx", "pages/RepoDocPage.tsx", "pages/RepoDocsPage.tsx"];
    if (JSON.stringify(users) !== JSON.stringify(allowed)) problems.push(`the picker is used in ${JSON.stringify(users)}, expected ${JSON.stringify(allowed)}`);
    if (/PositionPicker|usePick/.test(read("pages/FeaturePage.tsx"))) problems.push("FeaturePage carries a picker");
    // The tab tokens: a control's height, the focus ring and an accent underline on the current value.
    const css = read("ui/ui.css");
    // Every block whose selector list names `sel` as a whole class, bodies joined.
    const rule = (sel) =>
      [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
        .filter((m) => m[1].split(",").some((x) => x.trim() === sel))
        .map((m) => m[2])
        .join(" ");
    const sel = rule(".ui-position-select");
    if (!/min-height:\s*var\(--row-h\)/.test(sel)) problems.push("the select does not take the control height token");
    if (!/inset 0 -2px 0 var\(--accent\)/.test(sel)) problems.push("the select has no accent underline");
    if (!/\.ui-position-select:focus-visible[^{]*\{[^}]*outline:\s*2px solid var\(--accent\)/.test(css)) problems.push("the select has no focus ring");
    report("web::position_picker_sits_in_the_docs_page_header_as_a_select", problems);
  }
} finally {
  await closeRender();
}
process.exit(failed ? 1 : 0);
