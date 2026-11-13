#!/usr/bin/env node
//
// web::prepared (FTR-034 R-2): WHILE a decision has a prepared record, the monitor shows it filled
// in beside the decision's button, with its sources as links, and lets the person edit or reject it
// before confirming. The pure functions the pages use are checked case by case; the panel and the
// pages are read for where they render it.
//
// Usage: npm run check:prepared
import { readFileSync } from "node:fs";
import { EDITABLE, openFor, preparedLine, sourceHref } from "../src/prepared.ts";

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const view = (id, at, state, subject, extra = {}) => ({
  id,
  kind: "gate-result",
  subject,
  fields: { passed: true },
  sources: [],
  by: "Claude",
  at,
  state,
  resolution: null,
  stale_why: "",
  page: "releases",
  command: "",
  ...extra,
});
const sha = "9f078bd0c1e2aaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const views = [
  view("a", "2026-11-12T10:00:00Z", "superseded", { gate: "ci", commit: sha }),
  view("b", "2026-11-12T10:05:00Z", "open", { gate: "ci", commit: sha }),
  view("c", "2026-11-12T10:10:00Z", "stale", { gate: "ci", commit: sha }),
  view("d", "2026-11-12T09:00:00Z", "open", { gate: "ci", commit: sha }),
  view("e", "2026-11-12T11:00:00Z", "open", { gate: "smoke", commit: sha }),
  view("f", "2026-11-12T12:00:00Z", "confirmed", { gate: "ci", commit: sha }),
];

// openFor: the newest open view for the subject; stale, superseded and answered ones never.
expect("openFor picks the newest open view for a subject", openFor(views, "gate-result", { gate: "ci", commit: sha })?.id, "b");
expect("openFor keeps to the subject", openFor(views, "gate-result", { gate: "smoke", commit: sha })?.id, "e");
expect("openFor keeps to the kind", openFor(views, "signoff", { gate: "ci" }), undefined);
expect(
  "openFor ignores stale and superseded views",
  openFor(views.filter((v) => v.state !== "open"), "gate-result", { gate: "ci", commit: sha }),
  undefined,
);
expect("openFor on nothing", openFor(undefined, "gate-result", {}), undefined);

// sourceHref: board references stay on the monitor, links go out, and nothing else is a link.
expect("doc: is a board doc", sourceHref("shop", { label: "minutes", url: "doc:minutes/tuesday.md" }), {
  href: "/p/shop/docs/file/minutes/tuesday.md",
  external: false,
});
expect("item: is an item", sourceHref("shop", { label: "FTR-034", url: "item:FTR-034" }), {
  href: "/p/shop/feature/FTR-034",
  external: false,
});
expect("commit: is shown as code, not a link", sourceHref("shop", { label: "tip", url: `commit:${sha}` }), null);
expect("https: is an outside link", sourceHref("shop", { label: "run", url: "https://ci.example/run/4512" }), {
  href: "https://ci.example/run/4512",
  external: true,
});
for (const url of ["javascript:alert(1)", "data:text/html,x", "https://user:token@ci.example/", "doc:../secret", "item:<x>", "ftp://x/y", ""]) {
  expect(`${JSON.stringify(url)} is not a link`, sourceHref("shop", { label: "x", url }), null);
}
expect("a gate result's person may change its result, link and note", EDITABLE["gate-result"], ["passed", "url", "note"]);
const line = preparedLine(
  view("g", "2026-11-12T10:15:00Z", "open", {}, {
    sources: [
      { label: "CI run 4512", url: "https://ci.example/run/4512" },
      { label: "release tip", url: `commit:${sha}` },
    ],
  }),
);
expect("the line says who prepared it, when and from what", line, "Prepared by Claude on 12 Nov 2026 from CI run 4512, release tip 9f078bd");

// The panel: sources through sourceHref, one primary button, no input until Edit or Reject.
const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const panel = read("../src/components/PreparedPanel.tsx");
if (!/sourceHref\(project, s\)/.test(panel)) problems.push("PreparedPanel.tsx: sources are not rendered through sourceHref");
if (!/target="_blank" rel="noreferrer"/.test(panel)) problems.push("PreparedPanel.tsx: an outside link does not open apart, without a referrer");
const primaries = panel.match(/btn-primary/g) ?? [];
if (primaries.length !== 1) problems.push(`PreparedPanel.tsx: ${primaries.length} primary buttons, wanted one`);
if (/dangerouslySetInnerHTML|<Markdown/.test(panel)) problems.push("PreparedPanel.tsx: prepared text is rendered as markup");
/** The spans of the source between each `{mode === "<m>" ? (` and its `) : null}`. */
const branches = (src, mode) => {
  const out = [];
  const open = `{mode === "${mode}" ? (`;
  for (let i = src.indexOf(open); i >= 0; i = src.indexOf(open, i + 1)) {
    const end = src.indexOf(") : null}", i);
    out.push([i, end < 0 ? src.length : end]);
  }
  return out;
};
const allowed = [...branches(panel, "edit"), ...branches(panel, "reject")];
if (!branches(panel, "edit").length || !branches(panel, "reject").length) {
  problems.push("PreparedPanel.tsx: no Edit or no Reject branch");
}
for (const m of panel.matchAll(/<(input|textarea|select)\b/g)) {
  if (!allowed.some(([a, b]) => m.index > a && m.index < b)) {
    const lineNo = panel.slice(0, m.index).split("\n").length;
    problems.push(`PreparedPanel.tsx:${lineNo}: a <${m[1]}> is rendered outside the Edit and Reject branches`);
  }
}

// The pages render it beside their buttons.
const review = read("../src/pages/ReviewPage.tsx");
if (!/openFor\(prepared, "signoff"/.test(review)) problems.push("ReviewPage.tsx: a sign-off does not look for its prepared record");
if (!/openFor\(prepared, "ratification"/.test(review)) problems.push("ReviewPage.tsx: Ratify does not look for its prepared record");
if (!/<PreparedPanel/.test(review)) problems.push("ReviewPage.tsx: renders no PreparedPanel");
if (!/label: `Sign off \$\{name\}`/.test(review) || !/label: `Ratify \$\{item\.code\}`/.test(review)) {
  problems.push("ReviewPage.tsx: the panels are not labelled Sign off <name> and Ratify <code>");
}
const releases = read("../src/pages/ReleasesPage.tsx");
const between = (src, from, to) => {
  const a = src.indexOf(from);
  const b = src.indexOf(to, a + 1);
  return a < 0 ? "" : src.slice(a, b < 0 ? src.length : b);
};
const recordResult = between(releases, "function RecordResult(", "function ReleaseNotesSection");
if (!recordResult) problems.push("ReleasesPage.tsx: RecordResult no longer comes before ReleaseNotesSection");
if (!/openFor\(prepared, "gate-result"/.test(recordResult) || !/<PreparedPanel/.test(recordResult)) {
  problems.push("ReleasesPage.tsx: RecordResult does not render a prepared CI result in a PreparedPanel");
}
const button = between(releases, "function ReleaseButton(", "function TagPanel(");
if (!/openFor\(prepared, "promotion"/.test(button) || !/<PreparedPanel/.test(button)) {
  problems.push("ReleasesPage.tsx: the confirmation after the tag does not render a prepared promotion in a PreparedPanel");
}
if (!/expect: tip/.test(button)) problems.push("ReleasesPage.tsx: a prepared promotion is not pinned to the release tip shown");

if (problems.length) {
  for (const p of problems) console.error(`web::prepared — ${p}`);
  process.exit(1);
}
console.log("web::prepared — a prepared record shows filled in beside its button, with its sources as links, and is edited or rejected only on request.");
