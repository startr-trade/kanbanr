#!/usr/bin/env node
//
// web::review_blocked (DEF-064 R-3): WHILE the Approve or Ratify button on the Review page is
// disabled, the page shows beside it one sentence that says what is missing and who provides it.
// The pure function the page uses (src/diagramCheck.ts) is checked case by case against
// approveBlocker; the page is read for where it renders the sentence.
//
// Usage: npm run check:review-blocked
import { readFileSync } from "node:fs";
import { approveBlocker, blockedSentence } from "../src/diagramCheck.ts";

const problems = [];
const expect = (name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const problem = (i) => ({ file: "spec.md", doc: null, block: i, line: 10 + i, message: "Parse error (syntax)" });
const states = {
  loading: { loading: true, failed: null, problems: [] },
  failed: { loading: false, failed: "500 Internal Server Error", problems: [] },
  "one problem": { loading: false, failed: null, problems: [problem(1)] },
  "three problems": { loading: false, failed: null, problems: [problem(1), problem(2), problem(3)] },
  clean: { loading: false, failed: null, problems: [] },
};

for (const verb of ["Approve", "Ratify"]) {
  for (const [name, state] of Object.entries(states)) {
    const s = blockedSentence(verb, state);
    const label = `${verb}, ${name}`;
    expect(`${label}: the sentence is null exactly when approveBlocker is`, s === null, approveBlocker(state) === null);
    if (s === null) continue;
    if (/[\r\n]/.test(s)) problems.push(`${label}: the sentence is more than one line: ${JSON.stringify(s)}`);
    if (!s.endsWith(".")) problems.push(`${label}: the sentence does not end with a full stop: ${JSON.stringify(s)}`);
    if (s.length >= 220) problems.push(`${label}: the sentence is ${s.length} characters, not under 220`);
    if (!s.includes(verb)) problems.push(`${label}: the sentence does not name ${verb}`);
    if (!s.includes("diagram")) problems.push(`${label}: the sentence does not name the diagram check`);
    if (!/Claude|reload the page|this page/.test(s)) {
      problems.push(`${label}: the sentence does not say who provides what is missing: ${JSON.stringify(s)}`);
    }
  }
  const one = blockedSentence(verb, states["one problem"]);
  const three = blockedSentence(verb, states["three problems"]);
  expect(
    `${verb}: one diagram is singular`,
    one,
    `${verb} is held until the 1 diagram listed above parses: Claude fixes it in the spec or the doc, and this page checks again.`,
  );
  expect(
    `${verb}: three diagrams are plural`,
    three,
    `${verb} is held until the 3 diagrams listed above parse: Claude fixes them in the spec or the doc, and this page checks again.`,
  );
  if (one === three) problems.push(`${verb}: the single and plural forms are the same`);
}

// The page: the sentence is text in a <p>, in the same review-actions block as the button whose
// `disabled` names the blocker, not only a tooltip.
const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const page = read("../src/pages/ReviewPage.tsx");
const blocks = page.split('<div className="review-actions">').slice(1);
const actions = blocks.find((b) => /disabled=\{busy \|\| blocker != null\}/.test(b));
if (!actions) {
  problems.push("ReviewPage.tsx: no review-actions block holds the button whose disabled names blocker");
} else {
  const at = actions.indexOf("disabled={busy || blocker != null}");
  const end = actions.indexOf("</div>");
  const sentence = actions.indexOf("blockedSentence(");
  if (sentence < 0 || (end >= 0 && sentence > end)) {
    problems.push("ReviewPage.tsx: blockedSentence( is not rendered in the button's review-actions block");
  } else {
    const before = actions.slice(0, sentence);
    const open = before.lastIndexOf("<p");
    if (open < 0 || open < at || before.slice(open).includes("</p>")) {
      problems.push("ReviewPage.tsx: blockedSentence( is not rendered inside a <p> element after the button");
    }
  }
  if (!/title=\{blocker \?\? undefined\}/.test(actions)) problems.push("ReviewPage.tsx: the button's title no longer names the blocker");
}
if (!/import \{[^}]*\bblockedSentence\b[^}]*\} from "\.\.\/diagramCheck"/.test(page)) {
  problems.push("ReviewPage.tsx: blockedSentence is not imported from ../diagramCheck");
}

if (problems.length) {
  for (const p of problems) console.error(`web::review_blocked — ${p}`);
  process.exit(1);
}
console.log("web::review_blocked — a disabled Approve or Ratify button says beneath it what is missing and who provides it.");
