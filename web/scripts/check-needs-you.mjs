#!/usr/bin/env node
//
// FTR-048: the Needs-you badge and drawer, rendered.
//   web::check_needs_you_badge_and_drawer (R-3) — the badge shows the person's total in a pill with
//     aria-expanded false and controls the drawer; the drawer has an h2, one h3 per section, lines
//     with a page as links, a line without one (doctor errors) as its sentence with a Copy button,
//     and the drift line from the daemon's latest result; with total 0 the badge shows no number and
//     the drawer says "Nothing needs you right now."; while loading it never says so; Escape and a
//     press outside close it, Escape returning focus to the badge.
//
// The components render through scripts/lib/render.mjs (FTR-046); the open/close rules are the
// pure reducer in src/needsYouState.ts.
//
// Usage: npm run check:needs-you
import { readFileSync } from "node:fs";
import { CLOSED, needsYouReducer, drawerSections, endingWords, seenKey } from "../src/needsYouState.ts";
import { closeRender, render } from "./lib/render.mjs";

const NAME = "web::check_needs_you_badge_and_drawer";
// Printed per failing check; the three DEF-104 checks name themselves in their messages.
const problems = [];
const expect = (ok, what) => {
  if (!ok) problems.push(what);
};
const noop = () => {};
const text = (el) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();

const attention = {
  total: 3,
  lines: [
    { kind: "approve", section: "decisions", label: "Approve", count: 2, to: "/p/x/review?group=approve", tone: "info", yours: true },
    { kind: "release", section: "releases", label: "v0.8.0 at candidate", count: 1, to: "/p/x/releases?tab=current#v0.8.0", tone: "info", yours: true },
    { kind: "red-tests", section: "health", label: "Red tests", count: 4, to: "/p/x/items?evidence=red", tone: "error", yours: false },
    {
      kind: "doctor-errors",
      section: "health",
      label: "Doctor errors",
      count: 22,
      ask: "Ask your agent to fix the 22 doctor errors.",
      tone: "error",
      yours: false,
    },
    {
      kind: "retro-due",
      section: "records",
      label: "Retro due: MS-003",
      count: 1,
      ask: "Ask your agent to write the retro for MS-003 from its facts.",
      tone: "info",
      yours: false,
    },
  ],
};
const drift = {
  state: "fresh",
  count: 5,
  as_of: { board_rev: "aaaaaaaaaa", code_rev: "3f2a1c9bbbb", finished_at: new Date().toISOString(), took_ms: 10 },
};
/** An Overview fixture whose Needs you is the delegated attention below; the rest is empty. */
let overviewAttention = null;
const overviewData = () => ({
  project: "x",
  board_rev: "abcdef0123456789",
  generated_at: "2026-11-17T10:00:00Z",
  attention: overviewAttention,
  now: { releases: [], milestones: [] },
  holds: {
    items: { total: 1, by_kind: {}, untyped: 0 },
    requirements: { total: 0, functional: 0, nfr: 0, by_pattern: {}, iso_tagged: 0 },
    tests: { total: 0, by_state: {}, by_kind: {} },
    coverage: { proven: 0, total: 0, stale_items: 0 },
    flow: { completed: 0, rework: 0, defects: 0, escaped: 0 },
    architecture: { adrs: {}, design_docs: 0, component_docs: 0 },
    records: { retros: 0, release_notes: 0, lessons: 0, releases: 0 },
  },
  goals: [],
  recent: [],
  activity: [],
});
const overviewProps = () => ({
  project: "x",
  name: "X",
  overview: { loading: false, error: null, data: overviewData(), reload: noop },
  drift: null,
  cadence: { sprints: false, releases: false },
  today: "2026-11-17",
});
const view = (props) => render("src/components/NeedsYou.tsx", "NeedsYouView", { onToggle: noop, onClose: noop, drift: null, open: false, project: "x", seen: null, ...props });

try {
  // The closed badge with three decisions waiting.
  {
    const { document } = await view({ attention });
    const b = document.querySelector("button.needs-you-badge");
    expect(b, "the badge is not a button");
    expect(b?.getAttribute("aria-expanded") === "false", `closed badge has aria-expanded=${b?.getAttribute("aria-expanded")}, expected false`);
    expect(b?.getAttribute("aria-controls") === "needs-you", "the badge does not control #needs-you");
    expect(b?.getAttribute("aria-label") === "Needs you: 3 decisions waiting", `badge label is "${b?.getAttribute("aria-label")}"`);
    expect(text(b?.querySelector(".needs-you-count")) === "3", `badge number is "${text(b?.querySelector(".needs-you-count"))}", expected 3`);
    expect(!document.getElementById("needs-you"), "the drawer is drawn while closed");
  }

  // The open drawer.
  {
    const { document } = await view({ attention, drift, open: true });
    const b = document.querySelector("button.needs-you-badge");
    expect(b?.getAttribute("aria-expanded") === "true", "open badge does not say aria-expanded=true");
    const d = document.getElementById("needs-you");
    expect(d, "no drawer #needs-you when open");
    expect(d?.getAttribute("role") === "dialog" && d?.getAttribute("aria-modal") === "false", "the drawer is not a non-modal dialog");
    const h2 = d && document.getElementById(d.getAttribute("aria-labelledby") ?? "");
    expect(h2?.tagName === "H2" && text(h2) === "Needs you", "the drawer is not labelled by its h2 \"Needs you\"");
    const h3s = [...(d?.querySelectorAll("h3") ?? [])].map(text);
    expect(JSON.stringify(h3s) === JSON.stringify(["Decisions", "Releases", "Health", "Records"]), `sections are ${JSON.stringify(h3s)}`);
    for (const l of attention.lines.filter((l) => l.to)) {
      const a = [...(d?.querySelectorAll("a") ?? [])].find((a) => a.getAttribute("href") === l.to);
      expect(a && text(a).includes(l.label) && text(a).includes(String(l.count)), `${l.kind} is not a link to ${l.to} with its label and count`);
    }
    const lis = [...(d?.querySelectorAll("li") ?? [])];
    const doctor = lis.find((li) => text(li).includes("Doctor errors"));
    expect(doctor && !doctor.querySelector("a"), "the doctor line is a link, but no page shows it");
    expect(doctor && [...doctor.querySelectorAll("button")].some((b) => text(b) === "Copy"), "the doctor line has no Copy button");
    expect(doctor && text(doctor).includes("Ask your agent to fix the 22 doctor errors."), "the doctor line does not show its sentence");
    const driftLi = lis.find((li) => text(li).includes("Drifted docs"));
    expect(driftLi && text(driftLi).includes("5") && text(driftLi).includes("as of 3f2a1c9"), `the drift line is "${text(driftLi)}"`);
    expect(driftLi && text(driftLi).includes("Ask your agent to settle the drifted docs."), "the drift line does not offer its sentence");
    expect(!text(d).includes("Nothing needs you"), "says nothing needs you with three waiting");
  }

  // Nothing waiting: no number, and the drawer says so.
  {
    const none = { total: 0 };
    let { document } = await view({ attention: none });
    expect(!document.querySelector(".needs-you-count"), "a number is shown with total 0");
    ({ document } = await view({ attention: none, open: true }));
    expect(text(document.getElementById("needs-you")).includes("Nothing needs you right now."), "the empty drawer does not say \"Nothing needs you right now.\"");
  }

  // While loading it never says nothing is waiting.
  {
    let { document } = await view({ attention: null });
    expect(!document.querySelector(".needs-you-count"), "a number is shown while loading");
    ({ document } = await view({ attention: null, open: true }));
    const d = document.getElementById("needs-you");
    expect(d?.querySelector('[role="status"]'), "the loading drawer has no role=status");
    expect(!text(d).includes("Nothing needs you"), "the loading drawer says nothing needs you");
  }

  // Health lines are not counted, and the drift line is left out without flags.
  {
    const s = drawerSections(attention, { state: "fresh", count: 0 }, new Date());
    expect(!s.flatMap((x) => x.lines).some((l) => l.kind === "drift"), "a drift line without flags");
  }

  // Open and close, as the navigation's menus do.
  {
    let s = needsYouReducer(CLOSED, { type: "toggle" });
    expect(s.open, "toggle does not open");
    const esc = needsYouReducer(s, { type: "escape" });
    expect(!esc.open && esc.returnFocus, "Escape does not close and return focus to the badge");
    expect(needsYouReducer(CLOSED, { type: "escape" }) === CLOSED, "Escape on a closed drawer changes state");
    expect(!needsYouReducer(s, { type: "outside" }).open, "a press outside does not close");
    expect(!needsYouReducer(s, { type: "navigate" }).open, "reaching another page does not close");
    s = needsYouReducer(s, { type: "toggle" });
    expect(!s.open, "toggle does not close an open drawer");
  }

  // ---- DEF-104: Needs you in delegated mode ------------------------------------------------
  const dec = (code, verdict, at, extra = {}) => ({ decision: `${code}/${verdict}/-/${at}`, code, title: `Title of ${code}`, verdict, delegation: "DLG-001", at, status: "applied", undo_route: "", ...extra });
  const delegated = {
    in_force: ["DLG-001", "DLG-002"],
    decided: [dec("FTR-043", "approve", "2026-11-17T09:30:00Z"), dec("FTR-042", "signoff", "2026-11-16T14:02:00Z", { signoff: "design-review" }), dec("FTR-041", "ratify", "2026-11-15T10:00:00Z")],
    decided_total: 3,
    holds: [
      { delegation: "DLG-001", until: "2026-12-04T23:59:59Z", total: 6, verdicts: [{ verdict: "signoff:design-review", count: 4 }, { verdict: "approve", count: 2 }] },
      { delegation: "DLG-002", until: "2026-12-10T23:59:59Z", total: 1, verdicts: [{ verdict: "ratify", count: 1 }] },
    ],
    ending: [],
  };
  const dAttention = {
    total: 3,
    lines: [
      { kind: "signoff-docs-folded", section: "decisions", label: "Sign off docs-folded", count: 3, to: "/p/x/review?group=docs-folded", tone: "info", yours: true },
      { kind: "delegated", section: "decisions", label: "The agent's decisions to keep or undo", count: 3, to: "/p/x/review?group=delegated", tone: "info", yours: false },
      { kind: "red-tests", section: "health", label: "Red tests", count: 4, to: "/p/x/items?evidence=red", tone: "error", yours: false },
    ],
    delegated,
  };
  const SEEN_16 = "2026-11-16T00:00:00Z";

  // R-4: what a delegation holds is not Yours, not counted, in the drawer and on the Overview.
  {
    const NAME4 = "web::check_needs_you_leaves_out_delegated_gaps";
    const { document: closed } = await view({ attention: dAttention, seen: SEEN_16 });
    expect(text(closed.querySelector(".needs-you-count")) === "3", `${NAME4}: the badge says "${text(closed.querySelector(".needs-you-count"))}", expected 3`);
    const { document } = await view({ attention: dAttention, open: true, seen: SEEN_16 });
    const yours = document.getElementById("needs-you-yours")?.closest("section");
    const lis = [...(yours?.querySelectorAll("li") ?? [])].map(text);
    expect(lis.some((t) => t.includes("Sign off docs-folded")), `${NAME4}: Yours lacks docs-folded: ${JSON.stringify(lis)}`);
    expect(!lis.some((t) => /design-review|design review|keep or undo/.test(t)), `${NAME4}: Yours lists something the agent holds: ${JSON.stringify(lis)}`);
    overviewAttention = dAttention;
    const body = await render("src/pages/OverviewPage.tsx", "OverviewBody", { ...overviewProps(), seen: SEEN_16 }, { url: "/p/x" });
    const cards = [...body.document.querySelectorAll(".overview-card")].map(text);
    expect(cards.some((t) => t.includes("Sign off docs-folded")), `${NAME4}: Overview lacks docs-folded`);
    expect(!cards.some((t) => /design review|keep or undo/.test(t)), `${NAME4}: an Overview card names what the agent holds: ${JSON.stringify(cards)}`);
  }

  // R-5: three parts, in order, then Health.
  {
    const NAME5 = "web::check_needs_you_in_delegated_mode_reads_yours_decided_held";
    const { document } = await view({ attention: dAttention, drift: null, open: true, seen: SEEN_16 });
    const h3s = [...document.querySelectorAll("#needs-you h3")].map(text);
    expect(JSON.stringify(h3s) === JSON.stringify(["Yours", "The agent decided, since you last looked", "The agent holds", "Health"]), `${NAME5}: headings are ${JSON.stringify(h3s)}`);
    const decided = document.getElementById("needs-you-decided")?.closest("section");
    const fresh = [...(decided?.querySelectorAll(":scope > ul > li") ?? [])];
    expect(fresh.length === 2, `${NAME5}: ${fresh.length} fresh decisions, expected 2`);
    expect(fresh[0]?.querySelector("a")?.getAttribute("href") === "/p/x/feature/FTR-043" && fresh[1]?.querySelector("a")?.getAttribute("href") === "/p/x/feature/FTR-042", `${NAME5}: fresh decisions are not FTR-043 then FTR-042, each linked`);
    expect(text(fresh[0]).includes("approved FTR-043") && text(fresh[0]).includes("17 Nov 09:30") && text(fresh[0]).includes("DLG-001"), `${NAME5}: the line reads "${text(fresh[0])}"`);
    expect(text(fresh[1]).includes("signed off design-review"), `${NAME5}: the sign-off line reads "${text(fresh[1])}"`);
    expect(text(decided?.querySelector("details summary")) === "1 earlier, already seen", `${NAME5}: earlier reads "${text(decided?.querySelector("details summary"))}"`);
    const held = document.getElementById("needs-you-held")?.closest("section");
    const det = held?.querySelector("details");
    expect(det && !det.hasAttribute("open"), `${NAME5}: Agent holds is not a closed details`);
    expect(text(det?.querySelector("summary")).includes("7 decisions under DLG-001, DLG-002"), `${NAME5}: the summary reads "${text(det?.querySelector("summary"))}"`);
    const heldLines = [...(det?.querySelectorAll("li") ?? [])].map(text);
    expect(heldLines[0] === "DLG-001: 4 design reviews, 2 approvals \u00B7 until 4 Dec", `${NAME5}: the first hold reads "${heldLines[0]}"`);
    expect(heldLines[1] === "DLG-002: 1 ratification \u00B7 until 10 Dec", `${NAME5}: the second hold reads "${heldLines[1]}"`);
    for (const id of ["needs-you-decided", "needs-you-held"]) {
      const sec = document.getElementById(id)?.closest("section");
      expect(!sec?.querySelector(".ui-pill-warn, .ui-pill-error"), `${NAME5}: ${id} uses a warn or error pill`);
    }
    // Yours empty: names the delegations.
    const bare = { ...dAttention, total: 0, lines: [] };
    const empty = await view({ attention: bare, open: true, seen: SEEN_16 });
    expect(text(empty.document.querySelector(".needs-you-empty")) === "Nothing needs you \u2014 the agent is working under DLG-001 and DLG-002", `${NAME5}: empty Yours reads "${text(empty.document.querySelector(".needs-you-empty"))}"`);
    // Seen after every decision: the count line only.
    const later = await view({ attention: dAttention, open: true, seen: "2026-11-18T00:00:00Z" });
    const sec = later.document.getElementById("needs-you-decided")?.closest("section");
    expect(sec && !sec.querySelector(":scope > ul") && text(sec.querySelector("summary")) === "3 decided, already seen", `${NAME5}: after a look, part 2 is not the count line only: "${text(sec)}"`);
    // The Overview reads the same.
    overviewAttention = dAttention;
    const body = await render("src/pages/OverviewPage.tsx", "OverviewBody", { ...overviewProps(), seen: SEEN_16 }, { url: "/p/x" });
    const oh3 = [...body.document.querySelectorAll("h3")].map(text);
    expect(oh3.includes("The agent decided, since you last looked") && oh3.includes("The agent holds"), `${NAME5}: the Overview lacks the decided or held part: ${JSON.stringify(oh3)}`);
    // The key, and the one writer.
    expect(seenKey("x") === "kanbanr-needs-you-seen:x", `${NAME5}: seenKey("x") is ${seenKey("x")}`);
    const parts = readFileSync(new URL("../src/components/DelegatedParts.tsx", import.meta.url), "utf8");
    const drawer = readFileSync(new URL("../src/components/NeedsYou.tsx", import.meta.url), "utf8");
    const writes = (src) => (src.match(/localStorage\.setItem/g) ?? []).length;
    expect(writes(drawer) === 0 && writes(parts) === 1, `${NAME5}: the key is written ${writes(drawer)} time(s) in NeedsYou.tsx and ${writes(parts)} in DelegatedParts.tsx, expected 0 and 1 (useLastLook)`);
    expect(/export function useLastLook[\s\S]*localStorage\.setItem/.test(parts), `${NAME5}: the write is not inside useLastLook`);
  }

  // R-6: a delegation that ends soon is told, first, under Yours, and not counted.
  {
    const NAME6 = "web::check_needs_you_warns_when_a_delegation_ends_soon";
    const e = (state, returns, at) => ({ delegation: "DLG-001", state, at, returns });
    expect(endingWords(e("granted", 6, "2026-12-04T23:59:59Z")) === "DLG-001 ends on 4 Dec: 6 decisions return to you", `${NAME6}: granted reads "${endingWords(e("granted", 6, "2026-12-04T23:59:59Z"))}"`);
    expect(endingWords(e("ended", 6, "2026-12-04T23:59:59Z")) === "DLG-001 ended on 4 Dec: 6 decisions came back to you", `${NAME6}: ended reads "${endingWords(e("ended", 6, "2026-12-04T23:59:59Z"))}"`);
    expect(endingWords(e("revoked", 0, "2026-11-17T10:00:00Z")) === "DLG-001 was revoked on 17 Nov: nothing returns to you", `${NAME6}: revoked reads "${endingWords(e("revoked", 0, "2026-11-17T10:00:00Z"))}"`);
    const withEnding = { ...dAttention, delegated: { ...delegated, ending: [e("granted", 6, "2026-12-04T23:59:59Z")] } };
    const { document } = await view({ attention: withEnding, open: true, seen: SEEN_16 });
    const yours = document.getElementById("needs-you-yours")?.closest("section");
    const first = yours?.querySelector("li");
    expect(first && text(first).includes("DLG-001 ends on 4 Dec: 6 decisions return to you") && first.querySelector("a")?.getAttribute("href") === "/p/x/review#DLG-001", `${NAME6}: the first Yours line is "${text(first)}" linking ${first?.querySelector("a")?.getAttribute("href")}`);
    const badge = await view({ attention: withEnding, seen: SEEN_16 });
    expect(text(badge.document.querySelector(".needs-you-count")) === "3", `${NAME6}: an ending line changed the badge to "${text(badge.document.querySelector(".needs-you-count"))}"`);
    // Without a delegation in force the ending line still comes first, above today's sections.
    const noForce = { total: 3, lines: attention.lines, delegated: { ...delegated, in_force: [], ending: [e("revoked", 2, "2026-11-17T10:00:00Z")] } };
    const plain = await view({ attention: noForce, open: true });
    const drawer = plain.document.getElementById("needs-you");
    expect(text(drawer?.querySelector("li")).includes("DLG-001 was revoked on 17 Nov: 2 decisions came back to you"), `${NAME6}: a revoked delegation's line is not first without one in force`);
    expect(JSON.stringify([...(drawer?.querySelectorAll("h3") ?? [])].map(text)) === JSON.stringify(["Decisions", "Releases", "Health", "Records"]), `${NAME6}: without a delegation in force the sections changed`);
  }
} catch (e) {
  problems.push(`render failed: ${e?.stack ?? e}`);
} finally {
  await closeRender();
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — ok`);
for (const n of ["web::check_needs_you_leaves_out_delegated_gaps", "web::check_needs_you_in_delegated_mode_reads_yours_decided_held", "web::check_needs_you_warns_when_a_delegation_ends_soon"]) console.log(`${n} — ok`);
