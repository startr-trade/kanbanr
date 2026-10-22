#!/usr/bin/env node
//
// web::release_notes_section (FEAT-216 R-4): a release with a branch and a configured notes path
// has a Release notes section, headed "draft on release/<V> @ <sha>" before the cut and "as
// shipped on …" after it; Edit is offered only on a write-enabled monitor for an unshipped
// release, and otherwise the CLI command is shown. On a read-only monitor, saving sends nothing
// and names the command. The same function the page renders with is checked here, case by case.
//
// Usage: npm run check:release-notes
import { notesSection } from "../src/releaseNotes.ts";
import { setConfig } from "../src/runtimeConfig.ts";
import { api } from "../src/api.ts";

const draft = { version: "v0.3.0", state: "planned", stage: "candidate", branch: "release/v0.3.0" };
const shipped = { ...draft, state: "shipped", stage: "production" };
const notes = {
  version: "v0.3.0",
  branch: "release/v0.3.0",
  path: "docs/src/releases/v0.3.0.md",
  text: "# v0.3.0\n",
  at: "8f995e1",
  editable: true,
};
const shippedNotes = { ...notes, editable: false, why_not: "v0.3.0 has shipped: its notes are as they shipped" };
const path = "docs/src/releases/{version}.md";

const problems = [];
const expect = (name, cond, got) => {
  if (!cond) problems.push(`${name}: ${JSON.stringify(got)}`);
};

let s = notesSection(draft, path, notes, true);
expect("draft: a section", s.show, s);
expect("draft: headed with the branch and its tip", s.heading === "draft on release/v0.3.0 @ 8f995e1", s);
expect("draft, writable: Edit offered", s.edit, s);

s = notesSection(draft, path, notes, false);
expect("draft, read-only: no Edit, the CLI command", !s.edit && s.command === "kanbanr release notes v0.3.0 --edit", s);

s = notesSection(draft, path, null, true);
expect("not yet read: headed with the branch, no Edit", s.heading === "draft on release/v0.3.0" && !s.edit, s);

s = notesSection(shipped, path, shippedNotes, true);
expect("shipped: headed as shipped", s.heading === "as shipped on release/v0.3.0 @ 8f995e1", s);
expect("shipped, writable: no Edit", !s.edit && s.command === "kanbanr release notes v0.3.0", s);

s = notesSection({ ...draft, branch: undefined }, path, null, true);
expect("no branch yet: no section", !s.show && !s.edit, s);

s = notesSection(draft, undefined, notes, true);
expect("no notes path configured: no section", !s.show && !s.edit, s);

// A read-only monitor never sends the save, and names the command instead.
setConfig({ readOnly: true });
const sent = [];
globalThis.fetch = async (url, init) => {
  sent.push(`${init?.method ?? "GET"} ${url}`);
  return { ok: true, status: 200, json: async () => ({}), text: async () => "" };
};
try {
  await api.saveReleaseNotes("demo", "v0.3.0", "# v0.3.0\n", "Reviewer");
  problems.push("saveReleaseNotes succeeded on a read-only monitor");
} catch (e) {
  expect("read-only save names the command", String(e).includes("`kanbanr release notes v0.3.0 --edit`"), String(e));
}
expect("read-only save sends nothing", sent.length === 0, sent);

// A write-enabled monitor saves with PUT to the write route.
setConfig({ readOnly: false });
sent.length = 0;
await api.saveReleaseNotes("demo", "v0.3.0", "# v0.3.0\n", "Reviewer");
expect(
  "writable save: PUT to the notes write route",
  sent.length === 1 && /^PUT .*\/write\/projects\/demo\/releases\/v0\.3\.0\/notes$/.test(sent[0]),
  sent,
);

if (problems.length) {
  for (const p of problems) console.error(`web::release_notes_section — ${p}`);
  process.exit(1);
}
console.log(
  "web::release_notes_section — a section per release with a branch, headed draft or as shipped; Edit only when writable and unshipped; a read-only save sends nothing.",
);
