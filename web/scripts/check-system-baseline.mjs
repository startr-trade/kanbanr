#!/usr/bin/env node
//
// FTR-006: the system baseline's acceptance, checked without a browser.
//   web::system_baseline (R-10, R-12) — systemBaselineView's cardFor gives the group body for an
//     awaiting or prepared baseline and none for an accepted or changed one; a read-only monitor
//     gets the command, never a button; the entry rows word pinned → now; the Catalog chips read
//     pinned, changed since <date>, not pinned and awaiting; the Review page renders the acceptance
//     inside the `system-baseline` group (the rendered group shows its sentence, and on a read-only
//     monitor the command and no button), and the page never mounts it as a page-level panel.
//
// Usage: npm run check:system-baseline
import { readFileSync } from "node:fs";
import {
  ACCEPT_COMMAND,
  ACCEPT_SENTENCE,
  GROUP_KEY,
  cardFor,
  catalogChip,
  catalogRowHref,
  entryRowsLine,
  rowLine,
  viewsChanged,
} from "../src/systemBaselineView.ts";
import { closeRender, render } from "./lib/render.mjs";

const problems = [];
const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);
const eq = (what, got, want) => {
  if (!same(got, want)) problems.push(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
const read = (rel) => readFileSync(new URL(`../src/${rel}`, import.meta.url), "utf8");

const state = (extra = {}) => ({
  kind: "awaiting",
  rows: ["context", "decisions", "code-structure"],
  accepted_count: 0,
  can_accept: true,
  pins_rev: "abcdef1234567",
  unpinned: ["code-structure"],
  ...extra,
});
const accepted = { at: "2026-11-21T10:00:00Z", by: "Ada", origin: "monitor", rev: "r1", pins: [] };
const prepared = { id: "P-1", kind: "system-baseline", subject: { target: "system-baseline" }, fields: { note: "agreed in the call" } };

try {
  // The group body: awaiting and prepared show it; accepted and changed do not.
  const card = cardFor(state(), null, true);
  if (!card) problems.push("an awaiting baseline has no card");
  else {
    eq("the card's heading", card.heading, "System baseline");
    if (!card.body.startsWith("System baseline — 3 rows, 2 pinned; accepting says the system design is agreed before the first build")) {
      problems.push(`the card's body reads ${JSON.stringify(card.body)}`);
    }
    eq("each row is listed with where it stands", card.rows.map((r) => r.text), [
      "context: pinned",
      "decisions: pinned",
      "code-structure: not pinned (kept by its own tool)",
    ]);
    eq("a writable monitor offers Accept, pinned to the rev it showed", card.action, { kind: "button", label: "Accept", rev: "abcdef1234567" });
  }
  eq("an accepted baseline has no group body", cardFor(state({ kind: "accepted", accepted, accepted_count: 1 }), null, true), null);
  eq("a changed baseline has no group body", cardFor(state({ kind: "changed", accepted, accepted_count: 1, changes: [{ row: "context", source: "a.md", pinned: "1", now: "2", missing: false }] }), null, true), null);
  const again = cardFor(state({ kind: "changed", accepted, accepted_count: 1 }), prepared, true);
  eq("a prepared re-acceptance brings the body back, with its note", [again?.prepared, again?.note], ["P-1", "agreed in the call"]);
  eq("a read-only monitor shows the command, not a button", cardFor(state(), null, false)?.action, { kind: "command", command: ACCEPT_COMMAND });
  const blocked = cardFor(state({ can_accept: false, cannot_accept_why: "write the system docs first: context", missing: ["context"] }), null, true);
  eq("a baseline that cannot be accepted says why", blocked?.action, { kind: "blocked", why: "write the system docs first: context" });
  eq("a missing row reads as no source yet", blocked?.rows[0].text, "context: no source yet");

  // The entry rows word pinned → now.
  eq("a row pinned and changed", rowLine({ row: "context", pinned: "91ab0000", now: "4c120000" }), "context (pinned 91ab… → now 4c12…)");
  eq("a row never pinned", rowLine({ row: "context", pinned: "", now: "4c120000" }), "context (not pinned before → now 4c12…)");
  eq("the entry's sentence", entryRowsLine([{ row: "context", source: "a.md", pinned: "91ab0000", now: "4c120000" }]), "Changes the system baseline: context (pinned 91ab… → now 4c12…)");
  eq("a design changing no row says nothing", entryRowsLine([]), "");
  eq("views are the derived ones then the named, once", viewsChanged(["context", "components"], ["components", "code-structure"]), ["context", "components", "code-structure"]);
  eq("a views link goes to the Catalog row", catalogRowHref("demo", "code-structure"), "/p/demo/catalog#row-code-structure");

  // The Catalog page's chips.
  const acc = state({ kind: "accepted", accepted, accepted_count: 1 });
  eq("a pinned row", catalogChip("context", acc)?.label, "pinned");
  eq("an unpinned row", catalogChip("code-structure", acc)?.label, "not pinned");
  eq("an awaiting row", catalogChip("context", state())?.label, "awaiting");
  eq("a changed row", catalogChip("context", state({ kind: "changed", accepted, accepted_count: 1, changes: [{ row: "context", source: "a.md", pinned: "1", now: "2", missing: false }] }))?.label, "changed since 2026-11-21");
  eq("a row the baseline does not hold has no chip", catalogChip("security", acc), null);
  eq("no chip before init", catalogChip("context", state({ kind: "not_initialised" })), null);

  // The Review page renders the acceptance inside the group, never as a page-level panel.
  const row = {
    key: GROUP_KEY,
    code: GROUP_KEY,
    title: "System baseline",
    what: "3 rows, 2 pinned: awaiting acceptance",
    records: "Records your acceptance of the system baseline as shown (pins rev `abcdef1`)",
    release: "",
    sprint: "",
    changed_at: "",
    blocked: "",
    pills: [],
    prepared: "",
    batch: "",
    rev: "abcdef1234567",
    spec_rev: "",
    signoff: "",
    requirement: "",
    test: "",
  };
  const slice = { key: GROUP_KEY, label: "System baseline", action: "Accept", rows: [row], choices: [] };
  const panel = (writable) =>
    render(
      "src/pages/review/GroupView.tsx",
      "GroupPanel",
      { project: "x", slice, writable, who: "Ada <a@example.invalid>", tick: 0, body: (r) => `evidence of ${r.key}` },
      { url: "/p/x/review?group=system-baseline" },
    );
  const write = await panel(true);
  if (!write.document.querySelector(".review-group-head")?.textContent.includes(ACCEPT_SENTENCE)) problems.push("the group's sentence is not the acceptance's");
  eq("the writable group's button asks first", [...write.document.querySelectorAll(".review-group-head button")].map((b) => b.textContent.trim()).includes("Accept 0"), true);
  eq("the row sits in the group", write.document.getElementById(GROUP_KEY)?.tagName, "SECTION");
  const ro = await panel(false);
  const head = ro.document.querySelector(".review-group-head");
  if (!head?.textContent.includes(ACCEPT_COMMAND)) problems.push("a read-only monitor does not show the command");
  if ([...(head?.querySelectorAll("button") ?? [])].some((b) => /Accept/.test(b.textContent))) problems.push("a read-only monitor offers the Accept button");

  const page = read("pages/ReviewPage.tsx");
  const uses = page.match(/<SystemBaselineEvidence/g) ?? [];
  if (uses.length !== 1) problems.push(`ReviewPage mounts SystemBaselineEvidence ${uses.length} times, expected once, inside a group`);
  const at = page.indexOf("<SystemBaselineEvidence");
  if (at >= 0 && !page.slice(0, at).includes("baselineEvidence = () =>")) problems.push("the acceptance is not built as a group row body");
  if (!/slice\.key === SYSTEM_BASELINE_GROUP/.test(page)) problems.push("the page does not draw the card inside the system-baseline group");
  const evidence = read("pages/review/SystemBaselineEvidence.tsx");
  if ((evidence.match(/api\.acceptSystemBaseline\(/g) ?? []).length !== 1) problems.push("the card does not post acceptSystemBaseline once");
  if (!/<AskFirst/.test(evidence)) problems.push("the card's Accept does not ask first");
  const api = read("api.ts");
  if (/systemBaselineInit|baseline\/init/.test(api)) problems.push("api.ts has an init: the CLI does it");
} catch (e) {
  problems.push(`threw: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

if (problems.length) {
  for (const p of problems) console.error(`web::system_baseline — ${p}`);
  process.exit(1);
}
console.log("web::system_baseline — the group body, the read-only command, the entry rows and the catalog chips read as designed");
