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
import { CLOSED, needsYouReducer, drawerSections } from "../src/needsYouState.ts";
import { closeRender, render } from "./lib/render.mjs";

const NAME = "web::check_needs_you_badge_and_drawer";
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
      ask: "Ask Claude to fix the 22 doctor errors.",
      tone: "error",
      yours: false,
    },
    {
      kind: "retro-due",
      section: "records",
      label: "Retro due: MS-003",
      count: 1,
      ask: "Ask Claude to write the retro for MS-003 from its facts.",
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
const view = (props) => render("src/components/NeedsYou.tsx", "NeedsYouView", { onToggle: noop, onClose: noop, drift: null, open: false, ...props });

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
    expect(doctor && text(doctor).includes("Ask Claude to fix the 22 doctor errors."), "the doctor line does not show its sentence");
    const driftLi = lis.find((li) => text(li).includes("Drifted docs"));
    expect(driftLi && text(driftLi).includes("5") && text(driftLi).includes("as of 3f2a1c9"), `the drift line is "${text(driftLi)}"`);
    expect(driftLi && text(driftLi).includes("Ask Claude to settle the drifted docs."), "the drift line does not offer its sentence");
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
