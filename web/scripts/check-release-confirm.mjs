#!/usr/bin/env node
//
// web::release_button_names_the_release_and_asks_first (DEF-034 R-3): a release's button names
// the release and the stage ("Promote v0.3.0 to main"), and a direct press asks first: the first
// press shows "Promote v0.3.0 to main?" with the same button and Cancel, the second press
// promotes, Cancel changes nothing. A read-only monitor and an empty release offer no question.
// The same function the page renders with is checked case by case, and AskFirst is rendered in
// jsdom and pressed.
//
// Usage: npm run check:release-confirm
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { transformWithEsbuild } from "vite";
import { JSDOM } from "jsdom";
import { releaseAction } from "../src/releaseAction.ts";

const NAME = "web::release_button_names_the_release_and_asks_first";
const base = {
  version: "v0.3.0",
  from: "candidate",
  to: "main",
  direction: "forward",
  cut: false,
  confirm: false,
  ships: false,
  button: "Promote v0.3.0 to main",
  conditions: [{ check: "items_finished", stage: "main", holds: true, detail: "2 planned item(s), all finished" }],
  steps: [{ action: "move_main", stage: "main", does: "fast-forwards main to release/v0.3.0 (abc1234)", noop: false }],
  release_tip: "abc1234def",
  missing: [],
  ready: true,
  needs_reason: false,
  repo: true,
  empty: false,
};
const withAgain = {
  ...base,
  again: {
    stage: "candidate",
    button: "Promote v0.3.0 to candidate again",
    does: ["commits the version 0.3.0 to release/v0.3.0"],
    ready: true,
    missing: [],
  },
};

const problems = [];
const expect = (name, cond, got) => {
  if (!cond) problems.push(`${name}: ${JSON.stringify(got)}`);
};

// The plan's label, as the daemon writes it (DEF-034 R-2), and the question built from it.
let a = releaseAction(base, true);
expect("the label names the release and the stage", a.label === "Promote v0.3.0 to main", a.label);
expect("a direct press asks first", a.ask === "Promote v0.3.0 to main?", a.ask);
a = releaseAction(base, false);
expect("read-only: no question", a.ask === undefined && !a.button, a);
a = releaseAction({ ...base, empty: true, missing: ["v0.3.0 has no items"] }, true);
expect("empty release: no question", a.ask === undefined && !a.button, a);
a = releaseAction(withAgain, true);
expect("the again button asks its own question", a.again?.ask === "Promote v0.3.0 to candidate again?", a.again);
expect("the primary question is unchanged by the again offer", a.ask === "Promote v0.3.0 to main?", a.ask);

// The page renders each direct-path button through AskFirst; confirmations keep their own path.
const page = readFileSync(new URL("../src/pages/ReleasesPage.tsx", import.meta.url), "utf8");
const asks = page.match(/<AskFirst\b/g) ?? [];
expect("the primary and the again button go through AskFirst", asks.length === 2, asks.length);
expect("the primary asks the plan's question", /ask=\{action\.ask\b/.test(page), "ReleasesPage.tsx");
expect("the again button asks its own question", /ask=\{action\.again\.ask\}/.test(page), "ReleasesPage.tsx");
expect("a confirmation after the tag keeps its note form", page.includes("setAsking(true)"), "ReleasesPage.tsx");
expect("the outcome chip names release undo", page.includes("kanbanr release undo ${r.version} --reason"), "ReleasesPage.tsx");

// AskFirst itself, rendered and pressed.
const dom = new JSDOM("<!doctype html><div id=root></div>", { url: "http://localhost/" });
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const src = readFileSync(new URL("../src/components/AskFirst.tsx", import.meta.url), "utf8");
const out = await transformWithEsbuild(src, "AskFirst.tsx", { jsx: "automatic", format: "esm", loader: "tsx" });
// The compiled module lives outside the tree, so its bare imports are resolved from here.
const code = out.code
  .replace(/from\s+"react\/jsx-runtime"/g, `from "${import.meta.resolve("react/jsx-runtime")}"`)
  .replace(/from\s+"react"/g, `from "${import.meta.resolve("react")}"`);
const dir = mkdtempSync(join(tmpdir(), "askfirst-"));
const file = join(dir, "AskFirst.mjs");
writeFileSync(file, code);
try {
  const React = (await import("react")).default;
  const { createRoot } = await import("react-dom/client");
  const { AskFirst } = await import(pathToFileURL(file).href);
  const root = createRoot(document.getElementById("root"));
  const act = React.act;
  let confirmed = 0;
  const render = (props) =>
    act(() => root.render(React.createElement(AskFirst, { onConfirm: () => void confirmed++, ...props })));
  const buttons = () => [...document.querySelectorAll("button")].map((b) => b.textContent);
  const press = (text) =>
    act(() => {
      const b = [...document.querySelectorAll("button")].find((x) => x.textContent === text);
      if (!b) throw new Error(`no button "${text}" among ${JSON.stringify(buttons())}`);
      b.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    });
  const text = () => document.getElementById("root").textContent;
  const props = { label: "Promote v0.3.0 to main", ask: "Promote v0.3.0 to main?" };

  await render(props);
  expect("at rest: one button, its label", buttons().join("|") === "Promote v0.3.0 to main", buttons());
  await press("Promote v0.3.0 to main");
  expect("the first press does not act", confirmed === 0, confirmed);
  expect("the first press asks", text().includes("Promote v0.3.0 to main?"), text());
  expect("the question has the same button and Cancel", buttons().join("|") === "Promote v0.3.0 to main|Cancel", buttons());
  await press("Cancel");
  expect("Cancel changes nothing", confirmed === 0 && buttons().join("|") === "Promote v0.3.0 to main", { confirmed, b: buttons() });
  await press("Promote v0.3.0 to main");
  await press("Promote v0.3.0 to main");
  expect("the second press promotes, once", confirmed === 1, confirmed);
  expect("after the second press the question is gone", !text().includes("?"), text());
  await press("Promote v0.3.0 to main");
  await render({ label: "Promote v0.3.0 to production", ask: "Promote v0.3.0 to production?" });
  expect("a new label drops the question", !text().includes("?") && confirmed === 1, text());
  await render({ ...props, disabled: true, disabledWhy: "not ready" });
  const b = document.querySelector("button");
  expect("disabled: says why", b?.disabled === true && b?.title === "not ready", b?.outerHTML);
  await act(() => root.unmount());
} finally {
  rmSync(dir, { recursive: true, force: true });
}

if (problems.length) {
  for (const p of problems) console.error(`${NAME} — ${p}`);
  process.exit(1);
}
console.log(`${NAME} — ok`);
