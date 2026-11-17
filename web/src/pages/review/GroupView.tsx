/**
 * One decision group of the Review page (FTR-038): its rows, the person's selection inside it, and
 * the one group action, asked first (R-3) and recorded as one verdict per selected row in one
 * commit (`POST /review/decide`).
 *
 * The header is sticky at the top of the open group (R-9): the group's sentence, select-all, the
 * filter and the sort (kept in the URL as `?r.q=`, `?r.sort=`, `?r.dir=`), and the action button,
 * neutral until something is selected. Each row is a `ReviewRow` (an `AccordionRow`, so its header
 * is a heading, R-10); its body, the row's evidence, is drawn by the page only while it is open.
 * A row that cannot be decided says why beside a disabled checkbox (R-2); in the approve and ratify
 * groups a row is selectable only once its diagram check has finished clean.
 *
 * The selection is never in the URL: a link never carries a pending decision.
 */
import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "../../api";
import { AskFirst } from "../../components/AskFirst";
import { dayMonth } from "../../cardText";
import { blockedSentence } from "../../diagramCheck";
import { guardInFlight } from "../../inFlight";
import { type Batch, batchChecks } from "../../prepared";
import { useQuery } from "../../data/useQuery";
import { urls } from "../../data/urls";
import type { Tag } from "../../data/cache";
import {
  TABLE_STATE_KEY,
  actionLabel,
  askText,
  batchOf,
  decisionBody,
  isTestsOnly,
  preselected,
  select,
  selectAll,
  selectTestsOnly,
  type Selection,
} from "../../reviewGroups";
import { Empty, ErrorState, LoadingState, WorkingState, filterRows, sortRows, useOpenRows, useUrlState, type Column } from "../../ui";
import type { BatchAnswer } from "../../api";
import type { GroupDecisionAnswer, GroupSlice, ReviewRow as Row } from "../../types";
import { ACCEPT_COMMAND, ACCEPT_SENTENCE } from "../../systemBaselineView";
import { ManualGroupLive } from "./ManualGroup";
import { DiagramWatch, ReviewRow, type DiagramState } from "./ReviewRow";

/**
 * What the Review page's reads depend on. A waiting decision follows from items, their specs
 * (`spec:<CODE>`, which no single tag covers), sign-offs, gates, prepared records, manual tests,
 * delegations, ADRs, releases and sprints, so any change of the board re-reads them.
 */
export const REVIEW_TAGS: readonly Tag[] = ["*"];

/** A group action's word once done: "approved 4 of 5". */
const PAST: Record<string, string> = {
  approve: "approved",
  ratify: "ratified",
  "sign off": "signed off",
  "plan into": "planned",
  confirm: "confirmed",
  grant: "granted",
  keep: "kept",
  accept: "accepted",
  undo: "undone",
  reject: "rejected",
  retire: "retired",
};
const past = (action: string) => PAST[action.toLowerCase()] ?? `${action.toLowerCase()}: done`;

/** The group's sentence above its rows: what its action records. */
function sentenceOf(slice: GroupSlice): string {
  const held = slice.rows.filter((r) => r.held_by);
  const first = held[0];
  const tail = first
    ? ` ${held.length} of these Claude holds under ${first.held_by}${first.held_until ? ` until ${dayMonth(first.held_until)}` : ""}; you can still decide them.`
    : "";
  return baseSentenceOf(slice) + tail;
}

function baseSentenceOf(slice: GroupSlice): string {
  switch (slice.key) {
    case "approve":
      return "Approving records agreement pinned to the definition below: change or remove approved text afterwards and the approval lapses; added text is reviewed at the next decision.";
    case "ratify":
      return "Ratifying agrees, after the fact, to work built under a recorded bypass, and the record says so. One ratification closes every override waiting on that item.";
    case "which-release":
      return "Each selected item is planned into the release you choose, as `kanbanr release plan` plans it.";
    case "manual":
      return "Confirming records the result Claude prepared as yours. A check with nothing prepared is your own observation: do it, then record it in its row.";
    case "delegation":
      return "Granting lets Claude take the verdicts a delegation names, within its scope and until its date.";
    case "delegated":
      return "Keep lets a decision Claude took under a delegation stand; Undo withdraws it. To give a reason, open the row.";
    case "adr":
      return "Accepting needs every section answered. Rejecting needs a reason, so it is done in the row.";
    case "system-baseline":
      return ACCEPT_SENTENCE;
    case "recode":
      return "Confirming renames every item in the plan to its kind's prefix, in one board commit. Each keeps its old code as an alias, so nothing that names it stops resolving. A plan the board has changed since is refused.";
    default:
      return `Signing off records the ${slice.label.replace(/^Sign off /, "")} sign-off for each selected item, pinned to the definition and spec as shown.`;
  }
}

/** The sort choices: the slice's own order first, then by release, last change, sprint or code. */
const SORTS: { key: string; label: string; value(r: Row): string | null }[] = [
  { key: "release", label: "Release", value: (r) => r.release || null },
  { key: "changed", label: "Last changed", value: (r) => r.changed_at || null },
  { key: "sprint", label: "Sprint", value: (r) => r.sprint || null },
  { key: "code", label: "Code", value: (r) => r.code || r.key },
];
const COLUMNS: Column<Row>[] = [
  ...SORTS.map((s) => ({ key: s.key, label: s.label, value: s.value, sortable: true })),
  { key: "title", label: "Title", value: (r) => r.title },
  { key: "what", label: "What", value: (r) => r.what },
  { key: "pills", label: "Pills", value: (r) => r.pills.join(" ") },
];

/** The outcome under the header once a decision returned. */
interface GroupOutcome {
  ok: boolean;
  text: string;
  failed: { key: string; error: string }[];
}

function outcomeOf(answer: GroupDecisionAnswer, word: string, n: number): GroupOutcome {
  const failed = answer.results.filter((r) => r.error).map((r) => ({ key: r.key, error: r.error ?? "" }));
  const made = answer.results.length - failed.length;
  return { ok: failed.length === 0, text: `${word} ${made} of ${n}`, failed };
}

/** A whole batch's answer (FTR-037), each record by its id: "confirmed all 3", or how many went through. */
function batchOutcomeOf(answer: BatchAnswer, word: string): GroupOutcome {
  const failed = answer.results.filter((r) => r.error).map((r) => ({ key: r.id, error: r.error ?? "" }));
  const n = answer.results.length;
  const made = n - failed.length;
  return { ok: failed.length === 0, text: failed.length === 0 ? `${word} all ${n}` : `${word} ${made} of ${n}`, failed };
}

/** The open group, read through the shared cache; a read that fails says so, never "empty". */
export function GroupView(p: {
  project: string;
  group: string;
  writable: boolean;
  who: string | null;
  /** The open prepared records by batch (FTR-037), for each batch's checks (DEF-089). */
  batches?: Batch[];
  body(row: Row, slice: GroupSlice): ReactNode;
  onDecided?(): void;
}): JSX.Element {
  const q = useQuery<GroupSlice>(p.project, urls.reviewGroup(p.project, p.group), REVIEW_TAGS);
  if (q.error && !q.data) return <ErrorState error={`This group could not be read: ${q.error}`} retry={q.reload} />;
  if (!q.data) return q.since === null ? <LoadingState label="Reading this group…" /> : <WorkingState label="Reading this group…" since={q.since} />;
  const reload = q.reload;
  return (
    <GroupPanel
      project={p.project}
      slice={q.data}
      writable={p.writable}
      who={p.who}
      batches={p.batches}
      body={(row) => p.body(row, q.data as GroupSlice)}
      onDecided={() => {
        reload();
        p.onDecided?.();
      }}
    />
  );
}

/** The group drawn from its slice: the sticky header, the rows, and the decision. */
export function GroupPanel(p: {
  project: string;
  slice: GroupSlice;
  /** The monitor may write, as a known identity, outside conversation mode. */
  writable: boolean;
  who: string | null;
  /** The open prepared records by batch (FTR-037), for each batch's checks (DEF-089). */
  batches?: Batch[];
  body(row: Row): ReactNode;
  onDecided?(): void;
}): JSX.Element {
  const { slice } = p;
  const needsDiagrams = slice.key === "approve" || slice.key === "ratify";
  const verb = slice.key === "ratify" ? "Ratify" : "Approve";
  const [checks, setChecks] = useState<Record<string, DiagramState>>({});
  const onCheck = useCallback((key: string, state: DiagramState) => setChecks((c) => ({ ...c, [key]: state })), []);

  // Rows whose diagram check has not finished clean cannot be selected yet.
  const pending = useMemo(() => {
    if (!needsDiagrams) return new Set<string>();
    return new Set(
      slice.rows
        .filter((r) => {
          const c = checks[r.key];
          return !c || c.loading || c.failed != null || c.problems.length > 0;
        })
        .map((r) => r.key),
    );
  }, [needsDiagrams, slice.rows, checks]);
  const reasonFor = (row: Row): string => {
    if (row.blocked) return row.blocked;
    if (!needsDiagrams) return "";
    const c = checks[row.key];
    if (!c || c.loading) return "checking diagrams…";
    return blockedSentence(verb, c) ?? "";
  };

  const [sel, setSel] = useState<Selection>(() => preselected(slice, pending));
  // A live update re-reads the slice; the selection is kept, minus rows no longer selectable.
  useEffect(() => {
    setSel((s) => {
      const keep = [...s.keys].filter((k) => slice.rows.some((r) => r.key === k && r.blocked === "") && !pending.has(k));
      return keep.length === s.keys.size ? s : { group: s.group, keys: new Set(keep) };
    });
  }, [slice, pending]);

  const [table, setTable] = useUrlState({ sort: "", dir: "asc", q: "" }, TABLE_STATE_KEY);
  const shown = useMemo(() => {
    const filtered = filterRows(slice.rows, COLUMNS, table.q);
    const col = COLUMNS.find((c) => c.key === table.sort);
    return col ? sortRows(filtered, col, table.dir === "desc" ? "desc" : "asc") : filtered;
  }, [slice.rows, table.q, table.sort, table.dir]);
  const [openRows, setOpen] = useOpenRows();

  const [target, setTarget] = useState(slice.choices[0] ?? "");
  const [inFlight, setInFlight] = useState(false);
  const [outcome, setOutcome] = useState<GroupOutcome | null>(null);
  // One call at a time (DEF-090): a second press while the first is in flight is dropped.
  const guard = useRef<ReturnType<typeof guardInFlight<[() => Promise<void>]>> | null>(null);
  if (!guard.current) guard.current = guardInFlight((call: () => Promise<void>) => call());

  // The second press of an ask-first step: one decision over the chosen rows. A selection that is
  // exactly one prepared batch, confirmed or turned down, goes through FTR-037's batch route, so
  // the batch is still one action and one commit (R-5); anything else is one group decision.
  const send = (action: string, picked: Selection, reason = "") => {
    // Only rows the person can see are sent (T12): a hidden selected row stays selected, unsent.
    const chosen = shownOnly(picked);
    const batch = action === slice.action || action === "reject" ? batchOf(slice, chosen) : null;
    const body = decisionBody(slice, chosen, slice.rows, p.who ?? "", target, action, reason);
    const n = body.items.length;
    const by = p.who ?? "";
    void guard.current?.run(async () => {
      setInFlight(true);
      try {
        if (batch !== null) {
          const answer =
            action === "reject"
              ? await api.rejectPreparedBatch(p.project, batch, by)
              : await api.confirmPreparedBatch(p.project, batch, by);
          setOutcome(batchOutcomeOf(answer, action === "reject" ? "rejected" : "confirmed"));
        } else {
          const answer = await api.decideGroup(p.project, body);
          setOutcome(outcomeOf(answer, past(action), n));
        }
        // What was sent leaves the selection; a selected row the filter hid stays.
        setSel((s) => ({ group: s.group, keys: new Set([...s.keys].filter((k) => !chosen.keys.has(k))) }));
        p.onDecided?.();
      } catch (e) {
        setOutcome({ ok: false, text: e instanceof Error ? e.message : String(e), failed: [] });
      } finally {
        setInFlight(false);
      }
    });
  };

  // A group action takes the selected rows that are shown (selection ∩ shown after the filter);
  // a selected row the filter hides stays selected, is not counted and is never sent (T12).
  const shownKeys = useMemo(() => new Set(shown.map((r) => r.key)), [shown]);
  const shownOnly = (s: Selection): Selection => ({ group: s.group, keys: new Set([...s.keys].filter((k) => shownKeys.has(k))) });
  const visible = shownOnly(sel);
  const n = [...visible.keys].filter((k) => slice.rows.some((r) => r.key === k)).length;
  const selectable = shown.filter((r) => reasonFor(r) === "");
  const testsOnlyN = shown.filter((r) => isTestsOnly(r) && !pending.has(r.key) && r.blocked === "").length;
  const allOn = selectable.length > 0 && selectable.every((r) => sel.keys.has(r.key));
  const preparedSel: Selection = {
    group: sel.group,
    keys: new Set([...visible.keys].filter((k) => slice.rows.some((r) => r.key === k && r.prepared !== ""))),
  };
  const nothing = n === 0 ? "Select the rows to decide first" : "Recording…";
  // Turning down every open record of one batch, and nothing else, is the batch's "Reject all N".
  const rejectLabel =
    batchOf(slice, preparedSel) !== null ? `Reject all ${preparedSel.keys.size}` : `Reject ${preparedSel.keys.size} prepared`;
  const plan = slice.key === "which-release";
  // Each batch in this group, with the runbook checks its results carry (DEF-089): one line each,
  // and the checks named on the action that confirms exactly that batch.
  const checksOf = (name: string) => {
    const b = p.batches?.find((x) => x.name === name);
    const checks = b ? batchChecks(b) : [];
    return checks.length ? ` (checks ${checks.join(", ")})` : "";
  };
  const batchNames = [...new Set(slice.rows.map((r) => r.batch).filter((b) => b !== ""))];
  const chosenBatch = batchOf(slice, visible);
  const label = actionLabel(slice, visible, target) + (chosenBatch !== null ? checksOf(chosenBatch) : "");

  return (
    <div className="review-group">
      <div className="review-group-head">
        <p className="muted small review-group-sentence">{sentenceOf(slice)}</p>
        <div className="review-group-tools">
          {p.writable ? (
            <label className="review-select-all">
              <input
                type="checkbox"
                checked={allOn}
                disabled={selectable.length === 0 ? true : undefined}
                onChange={(e) => {
                  const on = e.currentTarget.checked;
                  setSel((s) => selectAll(s, shown, on, pending));
                }}
              />{" "}
              Select all
            </label>
          ) : null}
          {p.writable && slice.key === "approve" && shown.some(isTestsOnly) ? (
            <button
              type="button"
              className="btn"
              disabled={testsOnlyN === 0}
              onClick={() => setSel((s) => selectTestsOnly(s, shown, pending))}
            >
              Select tests only ({testsOnlyN})
            </button>
          ) : null}
          <input
            className="review-reason review-group-filter"
            type="search"
            placeholder="Filter"
            aria-label={`Filter ${slice.label}`}
            value={table.q}
            onChange={(e) => setTable({ q: e.target.value })}
          />
          <label className="small">
            Sort{" "}
            <select value={table.sort} onChange={(e) => setTable({ sort: e.target.value })} aria-label="Sort the rows">
              <option value="">Release in flight first</option>
              {SORTS.map((s) => (
                <option key={s.key} value={s.key}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
          {table.sort ? (
            <button
              type="button"
              className="btn"
              aria-label={table.dir === "desc" ? "Sort ascending" : "Sort descending"}
              onClick={() => setTable({ dir: table.dir === "desc" ? "asc" : "desc" })}
            >
              {table.dir === "desc" ? "↓" : "↑"}
            </button>
          ) : null}
          {p.writable && plan ? (
            <label className="small">
              Into{" "}
              <select value={target} onChange={(e) => setTarget(e.target.value)} aria-label="The release to plan into">
                {slice.choices.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          {p.writable ? (
            <span className="review-group-actions">
              <AskFirst
                label={label}
                ask={askText(slice, visible, slice.rows, target)}
                className={n > 0 ? "btn btn-primary" : "btn"}
                disabled={n === 0 || inFlight || (plan && target === "")}
                disabledWhy={plan && target === "" ? "No release is planned: ask Claude to add one" : nothing}
                onConfirm={() => send(slice.action, visible)}
              />
              {slice.key === "delegated" ? (
                <AskFirst
                  label={`Undo ${n}`}
                  ask={askText({ ...slice, action: "Undo" }, visible, slice.rows)}
                  className="btn"
                  disabled={n === 0 || inFlight}
                  disabledWhy={nothing}
                  onConfirm={() => send("undo", visible)}
                />
              ) : null}
              {preparedSel.keys.size > 0 ? (
                <AskFirst
                  label={rejectLabel}
                  ask={askText({ ...slice, action: "Reject what Claude prepared for" }, preparedSel, slice.rows)}
                  className="btn"
                  disabled={inFlight}
                  disabledWhy="Recording…"
                  onConfirm={() => send("reject", preparedSel)}
                />
              ) : null}
            </span>
          ) : null}
        </div>
        {slice.key === "system-baseline" && !p.writable ? (
          // A read-only monitor shows the person's command, never a button (FTR-006).
          <p className="muted small">
            To accept it, run <code>{ACCEPT_COMMAND}</code> in your own terminal.
          </p>
        ) : null}
        {batchNames.map((name) => (
          <p className="muted small review-group-batch" key={name}>
            {slice.rows.filter((r) => r.batch === name).length} results from {name}
            {checksOf(name)}
          </p>
        ))}
        {outcome ? (
          <div className={`review-group-outcome ${outcome.ok ? "done" : "warn"}`} role="status">
            <span className={`chip ${outcome.ok ? "done" : "warn"}`}>{outcome.text}</span>
            {outcome.failed.length > 0 ? (
              <ul className="small">
                {outcome.failed.map((f) => (
                  <li key={f.key}>
                    {f.key}: {f.error}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="review-group-rows">
        {slice.rows.length === 0 ? (
          <Empty title="Nothing is waiting in this group." />
        ) : shown.length === 0 ? (
          <Empty title="No row matches the filter." />
        ) : slice.key === "manual" ? (
          // Manual checks grouped under their item, the release being shipped first (FTR-057).
          <ManualGroupLive
            project={p.project}
            slice={slice}
            rows={shown}
            writable={p.writable}
            sel={sel}
            onSelect={(key, v) => setSel((s) => select(s, slice.key, key, v, slice.rows, pending))}
            onSelectItem={setSel}
            onConfirmItem={(chosen) => send(slice.action, chosen)}
            onRetire={(chosen, reason) => send("retire", chosen, reason)}
            reasonFor={reasonFor}
            openRows={openRows}
            onOpen={setOpen}
            inFlight={inFlight}
            body={p.body}
          />
        ) : (
          shown.map((row) => {
            const why = reasonFor(row);
            return (
              <Fragment key={row.key}>
                {needsDiagrams ? (
                  <DiagramWatch project={p.project} rowKey={row.key} code={row.code} onState={onCheck} />
                ) : null}
                <ReviewRow
                  project={p.project}
                  row={row}
                  open={openRows.has(row.key)}
                  onToggle={(o) => setOpen(row.key, o)}
                  select={
                    p.writable
                      ? {
                          checked: sel.keys.has(row.key),
                          disabled: why || undefined,
                          onChange: (v) => setSel((s) => select(s, slice.key, row.key, v, slice.rows, pending)),
                        }
                      : undefined
                  }
                >
                  {() => p.body(row)}
                </ReviewRow>
              </Fragment>
            );
          })
        )}
      </div>
    </div>
  );
}
