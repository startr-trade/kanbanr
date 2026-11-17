#!/usr/bin/env node
//
// A batch Claude prepared, decided on the Review page's groups (FTR-038 T7). Two names:
//
// web::check_review_batch_selection_confirms_the_batch_after_asking (FTR-038 R-5): a batch's
// records arrive selected in their group; the selection that is exactly that batch reads
// "Confirm all N" (and the prepared ones "Reject all N"); the first press only asks, naming the
// batch's items, and posts nothing; the second posts the batch route once, as the monitor's
// identity, with nothing typed.
//
// web::check_review_page_offers_one_click_for_a_batch (FTR-037 R-5, kept by FTR-038's X5): the
// batch is one action with nothing typed, posting the batch confirm route once (and the reject
// route once); the pure grouping of open records into batches is checked case by case.
//
// The group is rendered through scripts/lib/render.mjs; GroupView's source is read for where it
// posts. AskFirst's own press behaviour is checked by check-release-confirm.mjs.
//
// Usage: npm run check:review-batch
import { readFileSync } from "node:fs";
import { batches } from "../src/prepared.ts";
import { actionLabel, askText, batchOf, preselected } from "../src/reviewGroups.ts";
import { closeRender, render } from "./lib/render.mjs";

const R5 = "web::check_review_batch_selection_confirms_the_batch_after_asking";
const FTR037 = "web::check_review_page_offers_one_click_for_a_batch";
const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const expect = (problems, name, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(`${name}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  }
};

const row = (key, extra = {}) => ({
  key,
  code: key.split("/")[0],
  title: `Title of ${key}`,
  what: "R-1: manual: x",
  records: "Records the prepared result as you",
  release: "",
  sprint: "",
  changed_at: "",
  blocked: "",
  pills: [],
  prepared: "",
  batch: "",
  rev: "",
  spec_rev: "",
  signoff: "",
  requirement: "R-1",
  test: "manual: x",
  ...extra,
});
const rows = [
  row("DEF-040/R-1/manual: x", { prepared: "P-1", batch: "agile-1", pills: ["prepared", "batch agile-1"] }),
  row("FTR-034/R-1/manual: x", { prepared: "P-2", batch: "agile-1", pills: ["prepared", "batch agile-1"] }),
  row("FTR-028/R-1/manual: x", { blocked: "Do the check, then press Passed or Failed in the row." }),
];
const manual = { key: "manual", label: "Manual checks", action: "Confirm", rows, choices: [] };

const src = read("../src/pages/review/GroupView.tsx");
const sendAt = src.indexOf("const send = ");
const sendEnd = src.indexOf("\n  };\n", sendAt);
const send = sendAt < 0 || sendEnd < 0 ? "" : src.slice(sendAt, sendEnd);
const count = (re, s = src) => (s.match(re) ?? []).length;

// R-5: the selection that is one batch confirms it, after asking.
{
  const problems = [];
  const pre = preselected(manual);
  expect(problems, "the batch's records arrive selected", [...pre.keys].sort(), ["DEF-040/R-1/manual: x", "FTR-034/R-1/manual: x"]);
  expect(problems, "the selection is the batch", batchOf(manual, pre), "agile-1");
  expect(problems, "it reads Confirm all N", actionLabel(manual, pre), "Confirm all 2");
  expect(problems, "the question names the batch's items", askText(manual, pre, rows), "Confirm DEF-040 R-1 and FTR-034 R-1?");
  try {
    const { document, html } = await render("src/pages/review/GroupView.tsx", "GroupPanel", {
      project: "x",
      slice: manual,
      writable: true,
      who: "Alice <a@example.invalid>",
      tick: 0,
      body: () => "",
    });
    const head = document.querySelector(".review-group-head");
    const buttons = [...(head?.querySelectorAll("button") ?? [])].map((b) => b.textContent.trim());
    if (!buttons.includes("Confirm all 2")) problems.push(`the group button does not read "Confirm all 2": ${JSON.stringify(buttons)} ${html.slice(0, 200)}`);
    if (!buttons.includes("Reject all 2")) problems.push(`the prepared button does not read "Reject all 2": ${JSON.stringify(buttons)}`);
    if (document.querySelector(".ask-first")) problems.push("the question shows before the first press");
  } catch (e) {
    problems.push(`rendering the group failed: ${e?.stack ?? e}`);
  } finally {
    await closeRender();
  }
  // The batch goes through send, the second press of AskFirst, and never a first-press onClick.
  if (!send) problems.push("GroupView.tsx: no send");
  if (!/batchOf\(slice, chosen\)/.test(send)) problems.push("GroupView.tsx: send does not ask whether the selection is one batch (batchOf)");
  if (!/api\.confirmPreparedBatch\(p\.project, batch, by\)/.test(send)) problems.push("GroupView.tsx: send does not post the batch confirm route as the monitor's identity");
  if (!/api\.rejectPreparedBatch\(p\.project, batch, by\)/.test(send)) problems.push("GroupView.tsx: send does not post the batch reject route as the monitor's identity");
  if (!/<AskFirst[\s\S]*?onConfirm=\{\(\) => send\(/.test(src)) problems.push("GroupView.tsx: send is not AskFirst's onConfirm (the second press)");
  if (/onClick=\{[^}]*\bsend\(/.test(src)) problems.push("GroupView.tsx: a button posts on its first press (onClick calls send)");
  if (!/label=\{rejectLabel\}/.test(src) || !/`Reject all \$\{preparedSel\.keys\.size\}`/.test(src)) problems.push('GroupView.tsx: the prepared button does not read "Reject all N" for a whole batch');
  if (problems.length) for (const p of problems) console.error(`${R5} — ${p}`);
  else console.log(`${R5} — ok`);
  if (problems.length) process.exitCode = 1;
}

// FTR-037 R-5, kept: one action, nothing typed, one batch post.
{
  const problems = [];
  const view = (id, at, state, batch, item) => ({
    id,
    kind: "manual-test",
    subject: { item, requirement: "R-1", test: "manual: x" },
    fields: { state: "green" },
    sources: [],
    by: "Claude",
    at,
    state,
    resolution: null,
    stale_why: "",
    page: "review",
    command: "",
    ...(batch ? { batch } : {}),
  });
  const views = [
    view("c", "2026-11-15T10:02:00Z", "open", "togaf-2026-11-15", "DEF-040"),
    view("a", "2026-11-15T10:00:00Z", "open", "togaf-2026-11-15", "FTR-034"),
    view("b", "2026-11-15T10:01:00Z", "open", undefined, "FTR-028"),
    view("d", "2026-11-15T09:00:00Z", "confirmed", "togaf-2026-11-15", "DEF-039"),
    view("e", "2026-11-15T11:00:00Z", "open", "agile-2026-11-15", "DOC-008"),
    view("f", "2026-11-15T11:30:00Z", "open", "  ", "DOC-008"),
  ];
  const grouped = batches(views);
  expect(problems, "batches come in the order their first record was prepared", grouped.map((b) => b.name), ["togaf-2026-11-15", "agile-2026-11-15"]);
  expect(problems, "a batch holds its open records, oldest first", grouped[0].views.map((v) => v.id), ["a", "c"]);
  expect(problems, "an answered record is not offered again", grouped[0].views.some((v) => v.id === "d"), false);
  expect(
    problems,
    "a record prepared alone, or with a blank batch, is in no batch",
    grouped.flatMap((b) => b.views.map((v) => v.id)).some((id) => id === "b" || id === "f"),
    false,
  );
  expect(problems, "nothing makes no batch", batches(undefined), []);

  expect(problems, "GroupView posts the batch confirm route once", count(/api\.confirmPreparedBatch\(/g), 1);
  expect(problems, "GroupView posts the batch reject route once", count(/api\.rejectPreparedBatch\(/g), 1);
  // Nothing typed: the batch posts carry the identity and no reason the person would type.
  if (/rejectPreparedBatch\(p\.project, batch, by,/.test(send)) problems.push("GroupView.tsx: Reject all asks for a reason");
  if (/<(textarea)\b/.test(src)) problems.push("GroupView.tsx: the group asks the person to type");

  const apiTs = read("../src/api.ts");
  if (!/\/prepared\/batch\/\$\{encodeURIComponent\(batch\)\}\/confirm`/.test(apiTs)) problems.push("api.ts: no batch confirm route");
  if (!/\/prepared\/batch\/\$\{encodeURIComponent\(batch\)\}\/reject`/.test(apiTs)) problems.push("api.ts: no batch reject route");
  const types = read("../src/types.ts");
  if (!/batch\?: string;/.test(types)) problems.push("types.ts: Prepared has no batch");
  if (problems.length) {
    for (const p of problems) console.error(`✗ ${FTR037} — ${p}`);
    process.exitCode = 1;
  } else {
    console.log(`✓ ${FTR037}`);
    console.log("web::review-batch — a batch of prepared records is confirmed or rejected in one action, after asking, with nothing typed.");
  }
}
