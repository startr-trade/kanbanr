import { useState } from "react";
import { useParams } from "react-router-dom";
import { Link } from "react-router-dom";
import { alreadySaid, api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import { zachmanColumns } from "../types";
import type { Adr, DriftFlag, GroupCount, GroupSlice, PendingReview, PlannedRelease, ReviewRow as Row, RowEvidence, SettledText, SpecBrief } from "../types";
import { docsFoldedOffer } from "../textsToCheck";
import { approveBlocker, blockedSentence, type DiagramProblem } from "../diagramCheck";
import { PreparedPanel } from "../components/PreparedPanel";
import { type Batch, batchChecks, batches, editedWhileFlagged, openFor } from "../prepared";
import { lastResultLine, manualCommand, notDueLine, outcomeLine, type ManualRow, type Outcome } from "../manualTests";
import type { DecisionsView, DelegationView, PreparedView, Waiting } from "../types";
import { useQuery } from "../data/useQuery";
import { urls } from "../data/urls";
import { GROUP_PARAM, groupTabs } from "../reviewGroups";
import { Empty, ErrorState, LoadingState, Tabs } from "../ui";
import { GroupView, REVIEW_TAGS } from "./review/GroupView";
import { useDiagramCheck } from "./review/ReviewRow";

/**
 * What to show once a verdict went through. A repeat of the latest verdict records nothing; the
 * answer then says who recorded it first and from where, and that replaces "approved" (DEF-028).
 */
function alreadyOr(answer: unknown, done: string): string {
  const said = alreadySaid(answer);
  return said ? `already ${said}` : done;
}

/** A call that failed, as the outcome shown where its form was (DEF-090). */
function failure(e: unknown): Outcome {
  return { ok: false, text: e instanceof Error ? e.message : String(e) };
}

/**
 * Where the form was, once its call returned (DEF-090): the outcome, a rejection included, and for
 * a failed call its error with "Try again", which brings the form back.
 */
function OutcomeChip({ outcome, onRetry }: { outcome: Outcome; onRetry: () => void }) {
  return (
    <>
      <span className={`chip ${outcome.ok ? "done" : "warn"}`}>{outcomeLine(outcome)}</span>
      {outcome.ok ? null : (
        <>
          {" "}
          <a
            href="#try-again"
            className="small"
            onClick={(e) => {
              e.preventDefault();
              onRetry();
            }}
          >
            Try again
          </a>
        </>
      )}
    </>
  );
}

/** "planned into v1", and, when a sprint runs that the item is not in, the command that plans it in (DEF-029). */
function plannedText(version: string, code: string, planned: PlannedRelease): string {
  const out = planned.not_in_sprint?.find((o) => o.code === code);
  return out
    ? `planned into ${version}. ${code} is in no sprint and ${out.sprint} is running: kanbanr sprint plan ${out.sprint} ${code}`
    : `planned into ${version}`;
}

/**
 * The Review page (FEAT-067, regrouped by FTR-038): what waits on the person, grouped by the
 * decision it needs, one group open at a time.
 *
 * The approval gate exists to stop work being built on reasoning nobody agreed to, and its premise
 * is that agreeing has to be **cheap**. So the page is a shell: the groups as tabs with their counts
 * (`?group=<key>`, the largest open when the URL names none), and the open group's rows, each
 * selectable inside that group only, with one action that asks first and records one verdict per
 * row. A row opens to its evidence, read for that row alone, with the single actions that need a
 * reason or the person's own observation.
 *
 * The daemon decides what is pending and what "agreed" means; this page only renders and posts. It
 * offers a decision only when the daemon accepts writes, because a button that fails on click is
 * worse than no button.
 */
export default function ReviewPage() {
  const { project = "" } = useParams();
  // The diagram checks and the briefs re-read on the live tick.
  const tick = useLiveTick(api.projectEvents(project), 400);
  // What each decision came to once its call returned (DEF-090); any outcome hides its form.
  const [done, setDone] = useState<Record<string, Outcome>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const meta = useAsync(() => api.getMeta(), []);
  // The decision groups, counted (FTR-038): the tabs.
  const summary = useQuery<GroupCount[]>(project, urls.reviewSummary(project), REVIEW_TAGS);
  // What Claude prepared for these decisions (FTR-034), shown filled in beside their buttons.
  const prepared = useAsync(() => api.getPrepared(project), [project, tick]);
  // Where this project's decisions are taken, and what was delegated (FTR-028).
  const decisions = useAsync(() => api.getDecisions(project), [project, tick]);

  // Who a yes here is recorded as (FEAT-077). The board's commit identity, exactly as
  // `kanbanr approve` defaults it — not the string "reviewed in the monitor", which named the place
  // the click happened and left twenty-eight approvals on this board attributable to nobody.
  const who = api.approver(meta.data ?? null);
  // A monitor configured read-only (the standalone bundle, FEAT-178) offers no decision at all; each
  // row shows the CLI command that records it instead.
  const writable = !isReadOnly() && meta.data?.writes === true && who != null;
  // In conversation mode core refuses the monitor's item verdicts, so none is offered (FTR-028).
  const inConversation = decisions.data?.mode === "conversation";

  // One verdict per item, and which one follows from how it got here: work built under a recorded
  // bypass is ratified — agreed after the fact, and recorded as such (FEAT-109) — anything else is
  // approved before the work.
  const agree = async (item: PendingReview) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      if (item.approval === "unratified") {
        // Pinned to the definition this page showed (FEAT-159): if it changed meanwhile, the
        // daemon refuses rather than recording a yes to text nobody read.
        const answer = await api.ratify(project, code, who, item.rev ?? "");
        setDone((d) => ({ ...d, [code]: { ok: true, text: alreadyOr(answer, "ratified") } }));
      } else {
        const answer = await api.approve(project, code, who, item.rev ?? "");
        setDone((d) => ({ ...d, [code]: { ok: true, text: alreadyOr(answer, "approved") } }));
      }
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A verdict on a proposed decision (FEAT-153), recorded as the same identity.
  const decide = async (adr: Adr, verdict: "accept" | "reject", reason = "") => {
    if (!who) return;
    setBusy(adr.id);
    try {
      await api.decideAdr(project, adr.id, verdict, who, reason);
      setDone((d) => ({ ...d, [adr.id]: { ok: true, text: verdict === "accept" ? "accepted" : "rejected" } }));
    } catch (e) {
      setDone((d) => ({ ...d, [adr.id]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A named sign-off the item's next stage is waiting on (FEAT-117), recorded as the same identity.
  const signoff = async (item: PendingReview, name: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      const answer = await api.signoff(project, code, name, who, item.rev ?? "", item.spec_rev ?? "");
      setDone((d) => ({ ...d, [code]: { ok: true, text: alreadyOr(answer, `signed off ${name}`) } }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A record Claude prepared, confirmed as the same identity (FTR-034): the server makes the
  // verdict it prepares, pinned to the definition and spec this page showed.
  const confirmPrepared = async (item: PendingReview, view: PreparedView, edits: Record<string, unknown>, done: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      const answer = await api.confirmPrepared(project, view, who, edits, { rev: item.rev ?? "", spec_rev: item.spec_rev ?? "" });
      setDone((d) => ({ ...d, [code]: { ok: true, text: alreadyOr(answer, done) } }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };
  const rejectPrepared = async (item: PendingReview, view: PreparedView, reason: string) => {
    if (!who) return;
    const code = item.code;
    setBusy(code);
    try {
      await api.rejectPrepared(project, view.id ?? "", who, reason);
      setDone((d) => ({ ...d, [code]: { ok: true, text: "Rejected what Claude prepared; ask Claude to prepare it again" } }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A manual test's result, recorded as the person who checked it (DEF-040): the route
  // `kanbanr test` uses, so the board's record is the same.
  const recordManual = async (m: ManualRow, passed: boolean, note: string) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.recordManual(project, m.row.code, m.row.requirement, m.row.test, passed, who, note);
      setDone((d) => ({ ...d, [key]: { ok: true, text: passed ? "recorded as passed" : "recorded as failed" } }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };
  const confirmManual = async (m: ManualRow, view: PreparedView, edits: Record<string, unknown>) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.confirmPrepared(project, view, who, edits);
      setDone((d) => ({ ...d, [key]: { ok: true, text: "recorded" } }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };
  const rejectManual = async (m: ManualRow, view: PreparedView, reason: string) => {
    if (!who) return;
    const key = manualKey(m);
    setBusy(key);
    try {
      await api.rejectPrepared(project, view.id ?? "", who, reason);
      setDone((d) => ({ ...d, [key]: { ok: true, text: "Rejected" } }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };
  // A whole batch at once (FTR-037): a walkthrough run's checks, confirmed or rejected with one
  // click; the single buttons stay for one result alone.
  const answerBatch = async (batch: Batch, verdict: "confirm" | "reject") => {
    if (!who) return;
    const key = `batch:${batch.name}`;
    setBusy(key);
    try {
      const answer =
        verdict === "confirm"
          ? await api.confirmPreparedBatch(project, batch.name, who)
          : await api.rejectPreparedBatch(project, batch.name, who);
      const failed = answer.results.filter((r) => r.error);
      setDone((d) => ({
        ...d,
        [key]: {
          ok: failed.length === 0,
          text:
            failed.length === 0
              ? `${verdict === "confirm" ? "confirmed" : "rejected"} all ${answer.results.length}`
              : `${answer.results.length - failed.length} of ${answer.results.length} ${verdict === "confirm" ? "confirmed" : "rejected"}; ${failed
                  .map((r) => `${r.id}: ${r.error}`)
                  .join("; ")}`,
        },
      }));
    } catch (e) {
      setDone((d) => ({ ...d, [key]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // Which release a ready item ships in (FEAT-185): the same plan route `kanbanr release plan` uses.
  const planRelease = async (item: PendingReview, version: string) => {
    const code = item.code;
    setBusy(code);
    try {
      const planned = await api.planIntoRelease(project, version, code);
      setDone((d) => ({ ...d, [code]: { ok: true, text: plannedText(version, code, planned) } }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // A release that does not exist yet: added, as `kanbanr release add` adds it, then planned into.
  const newRelease = async (item: PendingReview, version: string, target: string) => {
    const code = item.code;
    setBusy(code);
    try {
      await api.addRelease(project, version, target);
      const planned = await api.planIntoRelease(project, version, code);
      setDone((d) => ({ ...d, [code]: { ok: true, text: plannedText(version, code, planned) } }));
    } catch (e) {
      setDone((d) => ({ ...d, [code]: failure(e) }));
    } finally {
      setBusy(null);
    }
  };

  // "Try again" after a failed call (DEF-090): the outcome is dropped, so the form comes back.
  const retry = (key: string) =>
    setDone((d) => {
      const rest = { ...d };
      delete rest[key];
      return rest;
    });

  // A row's evidence, once read (GET /review/groups/{key}/rows/{row}): the brief of an item, a
  // manual check, a proposed ADR, or a delegation, with the single actions each keeps in its row.
  const evidence = (row: Row, e: RowEvidence) => {
    if (e.entry) {
      const item = e.entry;
      return (
        <Brief
          project={project}
          item={item}
          tick={tick}
          writable={writable && !inConversation}
          busy={busy === item.code}
          outcome={done[item.code]}
          onRetry={() => retry(item.code)}
          defaultOpen
          onApprove={() => agree(item)}
          onSignoff={(name) => signoff(item, name)}
          prepared={prepared.data ?? []}
          onConfirmPrepared={(view, edits, done) => confirmPrepared(item, view, edits, done)}
          onRejectPrepared={(view, reason) => rejectPrepared(item, view, reason)}
          onRelease={(version) => planRelease(item, version)}
          onNewRelease={(version, target) => newRelease(item, version, target)}
        />
      );
    }
    if (e.manual) {
      const m: ManualRow = { row: e.manual, prepared: e.prepared ?? undefined };
      return (
        <ManualCheck
          project={project}
          m={m}
          writable={writable}
          busy={busy === manualKey(m)}
          outcome={done[manualKey(m)]}
          onRetry={() => retry(manualKey(m))}
          onRecord={(passed, note) => recordManual(m, passed, note)}
          onConfirm={(view, edits) => confirmManual(m, view, edits)}
          onReject={(view, reason) => rejectManual(m, view, reason)}
        />
      );
    }
    if (e.adr) {
      const adr = e.adr;
      return (
        <DecisionBrief
          project={project}
          adr={adr}
          writable={writable}
          busy={busy === adr.id}
          outcome={done[adr.id]}
          onRetry={() => retry(adr.id)}
          onDecide={(verdict, reason) => decide(adr, verdict, reason)}
        />
      );
    }
    return (
      <p className="small">
        {row.what}. {row.records}. Its full record, and the actions that take a reason, are under{" "}
        <strong>Decisions here</strong> above.
      </p>
    );
  };

  const groups = summary.data ?? [];
  return (
    <div className="review-page">
      <div className="page-head">
        <h1>Review</h1>
        <LiveDot />
      </div>

      <p className="muted">
        {isReadOnly() ? (
          <>
            This copy of the monitor is <strong>read-only</strong> — record a verdict in Claude Code, or
            run the command each row shows.
          </>
        ) : meta.data?.writes !== true ? (
          <>
            This monitor is <strong>read-only</strong> — restart it with <code>kanbanr review --ui</code> to
            decide from here, or run the command each row shows.
          </>
        ) : who == null ? (
          <>
            This board has <strong>no commit identity</strong>, so an approval could not say who gave it —
            and one that cannot is not evidence of agreement. Set one with{" "}
            <code>kanbanr identity --name "You" --email you@example.com</code>, then reload.
          </>
        ) : (
          <>
            A yes is recorded as <strong>{who}</strong>.
          </>
        )}
      </p>
      {inConversation ? <p className="muted">Decisions are taken in the conversation in this project.</p> : null}

      <details className="review-decisions-here">
        <summary>Decisions here{decisions.data ? `: ${decisions.data.mode} mode` : ""}</summary>
        <DecisionsHere project={project} view={decisions.data ?? null} error={decisions.error} writable={writable} who={who} />
      </details>

      {batches(prepared.data).map((batch) => (
        <BatchBar
          key={batch.name}
          batch={batch}
          writable={writable}
          busy={busy === `batch:${batch.name}`}
          outcome={done[`batch:${batch.name}`]?.text}
          onAnswer={(verdict) => answerBatch(batch, verdict)}
        />
      ))}

      {summary.error && !summary.data ? (
        <ErrorState error={`What is waiting could not be read: ${summary.error}`} retry={summary.reload} />
      ) : !summary.data ? (
        <LoadingState label="Reading what is waiting…" />
      ) : groups.every((g) => g.count === 0) ? (
        <Empty title="Nothing is waiting for a decision.">
          <p className="muted small">
            An item appears here when its definition has never been approved, when approved text was
            changed or removed, which lapses the approval, or when text was added once the item is being
            built; and so do sign-offs, manual checks, delegations and proposed decisions.
          </p>
        </Empty>
      ) : (
        <Tabs tabs={groupTabs(groups)} param={GROUP_PARAM} label="Decisions waiting, by the decision">
          {(key) => (
            <GroupView
              key={key}
              project={project}
              group={key}
              writable={writable}
              who={who}
              tick={tick}
              body={(row, slice: GroupSlice) => <RowBody project={project} group={slice.key} row={row} render={evidence} />}
              onDecided={summary.reload}
            />
          )}
        </Tabs>
      )}
    </div>
  );
}

/** A row's evidence, read for that row alone when it opens; an error says so and offers a retry. */
function RowBody({
  project,
  group,
  row,
  render,
}: {
  project: string;
  group: string;
  row: Row;
  render: (row: Row, e: RowEvidence) => JSX.Element;
}) {
  const ev = useQuery<RowEvidence>(project, urls.reviewRow(project, group, row.key), REVIEW_TAGS);
  if (ev.error && !ev.data) return <ErrorState error={`${row.code}'s evidence could not be read: ${ev.error}`} retry={ev.reload} />;
  if (!ev.data) return <LoadingState label="Reading the evidence…" />;
  return render(row, ev.data);
}

/** Where a waiting decision is taken, in words. */
function placeWords(w: Waiting): string {
  if (w.place === "delegated") return `for Claude, under ${w.delegation ?? "a delegation"}`;
  if (w.place === "conversation") return "in the conversation";
  return "here, on the monitor";
}

/** A proposed delegation as FTR-034's prepared panel shows a prepared record (FTR-028). */
function asPrepared(g: DelegationView): PreparedView {
  const fields: Record<string, unknown> = { verdicts: g.verdicts.join(", ") };
  if (g.steps?.length) fields.steps = g.steps.join(", ");
  if (g.items?.length) fields.items = g.items.join(", ");
  if (g.kinds?.length) fields.kinds = g.kinds.join(", ");
  if (g.milestone) fields.milestone = g.milestone;
  if (g.all) fields.covers = "every item";
  fields.until = g.until;
  if (g.note) fields.note = g.note;
  return {
    id: g.id,
    kind: "delegation",
    subject: {},
    fields,
    sources: [],
    by: g.proposed.by,
    origin: g.proposed.origin,
    at: g.proposed.at,
    rev: g.rev,
    state: "open",
    resolution: null,
    stale_why: "",
    page: "review",
    command: `kanbanr decisions grant ${g.id}`,
  };
}

/**
 * Decisions here (FTR-028): where this project's decisions are taken, the delegations the person
 * granted or Claude proposed, and the decisions Claude took under one, to keep or undo.
 */
function DecisionsHere({
  project,
  view,
  error,
  writable,
  who,
}: {
  project: string;
  view: DecisionsView | null;
  error: unknown;
  writable: boolean;
  who: string | null;
}) {
  const [mode, setMode] = useState<string>("");
  const [said, setSaid] = useState<Record<string, string>>({});
  const [waiting, setWaiting] = useState<Waiting[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  if (error) {
    return (
      <section className="decisions-here">
        <h2>Decisions here</h2>
        <p className="error">Where decisions are taken could not be read: {String(error)}</p>
      </section>
    );
  }
  if (!view) return null;
  const run = async (key: string, f: () => Promise<unknown>, ok: string) => {
    setBusy(key);
    try {
      await f();
      setSaid((d) => ({ ...d, [key]: ok }));
    } catch (e) {
      setSaid((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const by = who ?? "";
  const chosen = mode || view.mode;
  const proposed = view.delegations.filter((g) => g.state === "proposed");
  const granted = view.delegations.filter((g) => g.state === "granted");
  const reason = (k: string) => reasons[k] ?? "";
  const setReason = (k: string, v: string) => setReasons((r) => ({ ...r, [k]: v }));
  return (
    <section className="decisions-here">
      <h2>Decisions here</h2>
      <p className="muted">
        Decisions are taken in <strong>{view.mode}</strong> mode.{" "}
        {view.mode === "delegated"
          ? "You decide here; Claude may take a verdict a delegation you granted covers, and it comes back below for you to keep or undo."
          : view.mode === "conversation"
            ? "Decisions are taken in the conversation in this project."
            : "Every verdict is yours, taken here."}
      </p>
      {writable ? (
        <p>
          <label>
            Mode{" "}
            <select value={chosen} onChange={(e) => setMode(e.target.value)} aria-label="Decision mode">
              <option value="monitor">monitor</option>
              <option value="delegated">delegated</option>
            </select>
          </label>{" "}
          <button
            disabled={busy === "mode" || chosen === view.mode}
            onClick={() =>
              run(
                "mode",
                async () => {
                  const answer = await api.setDecisionMode(project, chosen, by);
                  setWaiting(answer.waiting);
                },
                `now ${chosen}`,
              )
            }
          >
            Change
          </button>{" "}
          {said.mode ? <span className="small">{said.mode}</span> : null}
        </p>
      ) : (
        <p className="muted small">
          Change it with <code>kanbanr decisions mode &lt;monitor|delegated&gt;</code> in your own terminal.
        </p>
      )}
      {waiting && waiting.length > 0 ? (
        <ul className="small">
          {waiting.map((w, i) => (
            <li key={`${w.code}-${i}`}>
              <strong>{w.code}</strong> {w.what}: {placeWords(w)}
            </li>
          ))}
        </ul>
      ) : null}

      <h3>Delegations</h3>
      {proposed.length === 0 && granted.length === 0 ? (
        <p className="muted small">None in force or proposed.</p>
      ) : null}
      {proposed.map((g) => (
        <div key={g.id}>
          <p className="small">
            <strong>{g.id}</strong> proposed
          </p>
          {said[g.id] ? (
            <p className="small">{said[g.id]}</p>
          ) : (
            <PreparedPanel
              project={project}
              view={asPrepared(g)}
              writable={writable}
              busy={busy === g.id}
              label="Grant"
              onConfirm={() => run(g.id, () => api.grantDelegation(project, g.id, g.rev, by), "granted")}
              onReject={(why) =>
                run(
                  g.id,
                  () => api.revokeDelegation(project, g.id, by, why || "discarded on the Review page"),
                  "discarded",
                )
              }
            />
          )}
        </div>
      ))}
      {granted.map((g) => (
        <div key={g.id} className="small">
          <p>
            <strong>{g.id}</strong> granted: {g.verdicts.join(", ")}
            {g.kinds?.length ? ` · ${g.kinds.join(", ")}` : ""}
            {g.items?.length ? ` · ${g.items.join(", ")}` : ""}
            {g.milestone ? ` · ${g.milestone}` : ""}
            {g.all ? " · every item" : ""}
            {g.steps?.length ? ` · at ${g.steps.join(", ")}` : ""} · until {g.until.slice(0, 10)}
            {g.not_granted_through_kanbanr ? (
              <>
                {" "}
                · <strong>not granted through kanbanr</strong>: nothing is taken under it
              </>
            ) : null}
            {g.grant_unverifiable ? (
              <>
                {" "}
                · <strong>can't be checked</strong>: {g.grant_unverifiable}; nothing is taken under it until it can
              </>
            ) : null}
            {view.mode === "delegated" && g.permission_rules_checked ? (
              g.permission_rules_missing?.length ? (
                <>
                  {" "}
                  · auto mode: add <code>{g.permission_rules_missing.join(", ")}</code> to{" "}
                  <code>.claude/settings.local.json</code> (ask Claude, or run{" "}
                  <code>kanbanr claude permissions --write</code> in your own terminal)
                </>
              ) : (
                " · auto mode: allowed"
              )
            ) : null}
          </p>
          {said[g.id] ? (
            <p>{said[g.id]}</p>
          ) : writable ? (
            <p>
              <input
                aria-label={`Why revoke ${g.id}`}
                placeholder="why"
                value={reason(g.id)}
                onChange={(e) => setReason(g.id, e.target.value)}
              />{" "}
              <button
                disabled={busy === g.id || !reason(g.id).trim()}
                onClick={() => run(g.id, () => api.revokeDelegation(project, g.id, by, reason(g.id)), "revoked")}
              >
                Revoke
              </button>
            </p>
          ) : (
            <p className="muted">
              Revoke with <code>kanbanr decisions revoke {g.id} --reason "…"</code>.
            </p>
          )}
        </div>
      ))}

      <h3>Delegated decisions to review</h3>
      {view.to_review.length === 0 ? (
        <p className="muted small">Nothing Claude decided under a delegation is waiting.</p>
      ) : null}
      {view.to_review.map((d) => (
        <div key={d.decision} className="small">
          <p>
            <strong>{d.code}</strong> {d.title}: {d.verdict}
            {d.signoff ? ` ${d.signoff}` : ""}, under {d.delegation}, on {d.at.slice(0, 10)}
          </p>
          {said[d.decision] ? (
            <p>{said[d.decision]}</p>
          ) : writable ? (
            <p>
              <input
                aria-label={`Reason for ${d.decision}`}
                placeholder="reason (optional)"
                value={reason(d.decision)}
                onChange={(e) => setReason(d.decision, e.target.value)}
              />{" "}
              <button
                disabled={busy === d.decision}
                onClick={() =>
                  run(d.decision, () => api.reviewDelegated(project, d.decision, "keep", by, reason(d.decision)), "kept")
                }
              >
                Keep
              </button>{" "}
              <button
                disabled={busy === d.decision}
                onClick={() =>
                  run(d.decision, () => api.reviewDelegated(project, d.decision, "undo", by, reason(d.decision)), "undone")
                }
              >
                Undo
              </button>
            </p>
          ) : (
            <p className="muted">
              <code>kanbanr decisions review {d.decision} --keep</code> or <code>--undo</code>
            </p>
          )}
        </div>
      ))}
    </section>
  );
}

const manualKey = (m: ManualRow) => `manual:${m.row.code}:${m.row.requirement}:${m.row.test}`;

/**
 * One batch of prepared records (FTR-037): what a walkthrough run observed, offered as one click —
 * "Confirm all N" or "Reject all N" — beside the single buttons each record keeps below. Nothing
 * to fill in; where the monitor cannot write, it says what the batch is and that the terminal
 * records one result at a time.
 */
function BatchBar({
  batch,
  writable,
  busy,
  outcome,
  onAnswer,
}: {
  batch: Batch;
  writable: boolean;
  busy: boolean;
  outcome?: string;
  onAnswer: (verdict: "confirm" | "reject") => void;
}) {
  const n = batch.views.length;
  const kinds = Array.from(new Set(batch.views.map((v) => v.kind ?? "record")));
  const items = Array.from(new Set(batch.views.map((v) => v.subject.item).filter(Boolean)));
  const checks = batchChecks(batch);
  return (
    <section className="review-item batch-bar">
      <h2>Batch {batch.name}</h2>
      <p className="muted">
        {n} {kinds.join(", ")} record{n === 1 ? "" : "s"} prepared together
        {items.length ? ` for ${items.join(", ")}` : ""}
        {checks.length ? ` (checks ${checks.join(", ")})` : ""}, by {batch.views[0]?.by || "Claude"}. Confirm or
        reject them all with one click, or one alone below.
      </p>
      {outcome ? (
        <span className={`chip ${outcome.startsWith("confirmed all") || outcome.startsWith("rejected all") ? "done" : "warn"}`}>
          {outcome}
        </span>
      ) : writable ? (
        <div className="review-actions">
          <button className="btn btn-primary" disabled={busy} onClick={() => onAnswer("confirm")}>
            {busy ? "recording…" : `Confirm all ${n}`}
          </button>
          <button className="btn" disabled={busy} onClick={() => onAnswer("reject")}>
            {busy ? "recording…" : `Reject all ${n}`}
          </button>
        </div>
      ) : (
        <p className="small muted">This monitor is read-only; each result is recorded one at a time from the terminal.</p>
      )}
    </section>
  );
}


function ManualCheck({
  project,
  m,
  writable,
  busy,
  outcome,
  onRetry,
  onRecord,
  onConfirm,
  onReject,
}: {
  project: string;
  m: ManualRow;
  writable: boolean;
  busy: boolean;
  outcome?: Outcome;
  onRetry: () => void;
  onRecord: (passed: boolean, note: string) => void;
  onConfirm: (view: PreparedView, edits: Record<string, unknown>) => Promise<void>;
  onReject: (view: PreparedView, reason: string) => Promise<void>;
}) {
  const { row, prepared } = m;
  const [noting, setNoting] = useState(false);
  const [note, setNote] = useState("");
  const last = lastResultLine(row.last);
  // Before its item reaches its build stage a check offers no result (DEF-076).
  const notDue = notDueLine(row);
  // Any outcome, a rejection or an error included, replaces the form (DEF-090).
  const finished = outcome != null;
  const failed = row.state === "red";
  return (
    <div className="review-item manual-check">
      <p>
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(row.code)}`}>
          <code className="taskkey">{row.code}</code> {row.title}
        </Link>{" "}
        <code className="taskkey">{row.requirement}</code>{" "}
        <span className={`chip ${failed ? "warn" : ""}`}>{failed ? "failed" : "not run"}</span>
      </p>
      <p className="small">{row.requirement_text}</p>
      <p>
        <strong>Check:</strong> {row.test}
      </p>
      {last ? <p className="small muted">Last result: {last}.</p> : null}
      {failed ? (
        <p className="small muted">It is recorded as failed. To have a defect filed, ask Claude to file one.</p>
      ) : null}
      {finished ? (
        <OutcomeChip outcome={outcome} onRetry={onRetry} />
      ) : notDue ? (
        <p className="small muted">{notDue}</p>
      ) : prepared && writable ? (
        <PreparedPanel
          project={project}
          view={prepared}
          writable={writable}
          busy={busy}
          label={prepared.fields?.state === "red" ? "Record failed" : "Record passed"}
          onConfirm={(edits) => onConfirm(prepared, edits)}
          onReject={(reason) => onReject(prepared, reason)}
        />
      ) : writable ? (
        <>
          <div className="review-actions">
            <button className="btn btn-primary" disabled={busy} onClick={() => onRecord(true, note.trim())}>
              Passed
            </button>
            <button className="btn" disabled={busy} onClick={() => onRecord(false, note.trim())}>
              Failed
            </button>
            {noting ? null : (
              <a
                href="#add-a-note"
                className="small"
                onClick={(e) => {
                  e.preventDefault();
                  setNoting(true);
                }}
              >
                Add a note
              </a>
            )}
          </div>
          {noting ? (
            <p className="small">
              <label>
                Note (optional){" "}
                <input value={note} onChange={(e) => setNote(e.target.value)} maxLength={4000} aria-label="Note" />
              </label>
            </p>
          ) : null}
        </>
      ) : (
        <p className="small">
          To record it in your own terminal: <code>{manualCommand(row, true)}</code> (or <code>red</code>)
        </p>
      )}
    </div>
  );
}

/** One proposed decision: its text, what rests on it, and the two verdicts (FEAT-153). */
function DecisionBrief({
  project,
  adr,
  writable,
  busy,
  outcome,
  onRetry,
  onDecide,
}: {
  project: string;
  adr: Adr;
  writable: boolean;
  busy: boolean;
  outcome?: Outcome;
  onRetry: () => void;
  onDecide: (verdict: "accept" | "reject", reason?: string) => void;
}) {
  const [reason, setReason] = useState("");
  const decided = outcome != null;
  const missing = adr.missing ?? [];
  // The title is the summary's; the body below starts after it.
  const body = (adr.body ?? "").replace(/^#\s[^\n]*\n/, "");

  return (
    <details className="review-item" id={adr.id}>
      <summary>
        <code className="taskkey">{adr.id}</code>
        <span className="review-title">{adr.title}</span>
        <span className={`chip ${missing.length > 0 ? "warn" : ""}`}>
          {missing.length > 0 ? `unanswered: ${missing.join(", ")}` : "proposed"}
        </span>
        {decided ? <span className={`chip ${outcome.ok ? "done" : "warn"}`}>{outcome.text}</span> : null}
      </summary>

      <div className="review-body">
        {adr.path ? (
          <p className="muted small">
            <Link
              to={`/p/${encodeURIComponent(project)}/docs/file/${adr.path.split("/").map(encodeURIComponent).join("/")}`}
            >
              open {adr.path}
            </Link>
            {adr.date ? ` · proposed ${adr.date}` : null}
          </p>
        ) : null}
        {(adr.affects ?? []).length > 0 && (
          <div className="tile-states">
            {(adr.affects ?? []).map((code) => (
              <span className="chip tasks" key={code}>
                affects {code}
              </span>
            ))}
          </div>
        )}
        <Markdown source={body} />

        <div className="review-actions">
          {decided ? (
            <OutcomeChip outcome={outcome} onRetry={onRetry} />
          ) : writable ? (
            <>
              <button
                className="btn btn-primary"
                onClick={() => onDecide("accept")}
                disabled={busy || missing.length > 0}
                title={missing.length > 0 ? `Answer ${missing.join(", ")} first` : undefined}
              >
                {busy ? "recording…" : `Accept ${adr.id}`}
              </button>
              <input
                className="review-reason"
                placeholder="Why reject it?"
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                aria-label={`Reason for rejecting ${adr.id}`}
              />
              <button
                className="btn"
                onClick={() => onDecide("reject", reason)}
                disabled={busy || reason.trim() === ""}
              >
                {busy ? "recording…" : `Reject ${adr.id}`}
              </button>
            </>
          ) : (
            <code>
              kanbanr adr accept {adr.id} · kanbanr adr reject {adr.id} --reason "…"
            </code>
          )}
        </div>
      </div>
    </details>
  );
}

/** Where a diagram problem's source is: the item for its spec, the doc page for a doc. */
function problemHref(project: string, code: string, p: DiagramProblem): string {
  const enc = encodeURIComponent;
  return p.doc
    ? `/p/${enc(project)}/docs/file/${p.doc.split("/").map(enc).join("/")}`
    : `/p/${enc(project)}/feature/${enc(code)}`;
}

/** Context kept around each change, and the shortest run of unchanged lines worth folding (FTR-023). */
const DIFF_CONTEXT = 2;

type DiffRow = { op: "same" | "added" | "removed" | "fold"; text: string };

/** The changed lines with two lines of context; each longer unchanged run folds to a count. */
function foldDiff(diff: SpecBrief["diff"]): DiffRow[] {
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < diff.length) {
    if (diff[i].op !== "same") {
      rows.push(diff[i]);
      i++;
      continue;
    }
    let j = i;
    while (j < diff.length && diff[j].op === "same") j++;
    const run = diff.slice(i, j);
    const head = i === 0 ? 0 : DIFF_CONTEXT;
    const tail = j === diff.length ? 0 : DIFF_CONTEXT;
    if (run.length > 4) {
      rows.push(...run.slice(0, head));
      const hidden = run.length - head - tail;
      rows.push({ op: "fold", text: `… ${hidden} unchanged ${hidden === 1 ? "line" : "lines"}` });
      rows.push(...run.slice(run.length - tail));
    } else {
      rows.push(...run);
    }
    i = j;
  }
  return rows;
}

/** The spec a sign-off pins: only what changed since the last sign-off, the whole spec on request. */
function SpecDiff({ spec, changed }: { spec: SpecBrief; changed: boolean }) {
  const whole = <Markdown source={spec.text} />;
  if (spec.diff.length > 0) {
    return (
      <div className="spec-diff">
        <h3>The spec, changed since sign-off</h3>
        <pre className="diff">
          {foldDiff(spec.diff).map((r, n) => (
            <div key={n} className={`diff-${r.op}`} style={{ color: r.op === "added" ? "var(--ok, #1a7f37)" : r.op === "removed" ? "var(--bad, #cf222e)" : undefined }}>
              {r.op === "added" ? "+ " : r.op === "removed" ? "− " : "  "}
              {r.text}
            </div>
          ))}
        </pre>
        <details>
          <summary>Show the whole spec</summary>
          {whole}
        </details>
      </div>
    );
  }
  return (
    <div className="spec-diff">
      <h3>The spec this sign-off covers</h3>
      {changed && !spec.previous_kept ? (
        <p className="muted small">
          The text signed off on {spec.signed_at.slice(0, 10)} wasn't kept on this board (it was signed before
          kanbanr kept it), so the whole spec is shown.
        </p>
      ) : null}
      {whole}
    </div>
  );
}

function Brief({
  project,
  item,
  tick,
  writable,
  busy,
  outcome,
  onRetry,
  defaultOpen,
  onApprove,
  onSignoff,
  onRelease,
  onNewRelease,
  prepared,
  onConfirmPrepared,
  onRejectPrepared,
}: {
  project: string;
  item: PendingReview;
  tick: number;
  writable: boolean;
  busy: boolean;
  outcome?: Outcome;
  onRetry: () => void;
  defaultOpen: boolean;
  onApprove: () => void;
  onSignoff: (name: string) => void;
  onRelease: (version: string) => void;
  onNewRelease: (version: string, target: string) => void;
  prepared: PreparedView[];
  onConfirmPrepared: (view: PreparedView, edits: Record<string, unknown>, done: string) => Promise<void>;
  onRejectPrepared: (view: PreparedView, reason: string) => Promise<void>;
}) {
  const [adding, setAdding] = useState(false);
  const [version, setVersion] = useState("");
  const [target, setTarget] = useState("");
  const def = item.definition ?? {};
  const requirements = def.requirements ?? [];
  const dimensions = zachmanColumns(def.zachman).filter((d) => d.answer.trim());
  const ratify = item.approval === "unratified";
  const waitingOnSignoff = item.approval === "signoff";
  // The design was signed off, then the spec changed (FTR-023): signing again is the answer.
  const specChanged = item.approval === "spec_changed";
  // Ready for release and in none (FEAT-185); asked beside any other question this item has.
  const releaseChoices = item.release_choices;
  const whichRelease = item.approval === "release";
  const verb = ratify ? "Ratify" : "Approve";
  // Any outcome, a rejected prepared record or an error included, replaces the form and the
  // prepared panels (DEF-090).
  const approved = outcome != null;
  const signoffs = item.signoffs_needed ?? [];
  // The texts that name what this item changed (FTR-013): Claude settles each before docs-folded
  // is offered (DEF-039). The person is shown what is left and what Claude checked, never a form.
  const docsFolded = signoffs.includes("docs-folded")
    ? docsFoldedOffer(item)
    : { offer: true, left: [] as DriftFlag[], settled: [] as SettledText[] };
  const flags = docsFolded.left;
  const diagrams = useDiagramCheck(project, item.code, tick);
  const blocker = approveBlocker(diagrams);
  // What Claude prepared (FTR-034): a sign-off's note and doc, a ratification's reason. A
  // docs-folded is not offered while texts are left, so nothing prepared for it shows either.
  const preparedSignoff = (name: string) =>
    name === "docs-folded" && !docsFolded.offer ? undefined : openFor(prepared, "signoff", { item: item.code, signoff: name });
  const preparedRatify = ratify ? openFor(prepared, "ratification", { item: item.code }) : undefined;
  const panels = [
    ...(preparedRatify ? [{ view: preparedRatify, label: `Ratify ${item.code}`, done: "ratified" }] : []),
    ...signoffs.flatMap((name) => {
      const view = preparedSignoff(name);
      return view ? [{ view, label: `Sign off ${name}`, done: `signed off ${name}` }] : [];
    }),
  ];

  // Parts added or changed since the last decision lead; the rest is collapsed (FTR-018).
  const changes =
    item.changes && (item.changes.added.length > 0 || item.changes.changed.length > 0 || item.changes.removed.length > 0)
      ? item.changes
      : undefined;
  const definitionBody = (
    <>
      {def.statement ? <Markdown source={`> ${def.statement}`} /> : null}
      {(def.goals ?? []).length > 0 && (
        <div className="tile-states">
          {(def.goals ?? []).map((g) => (
            <span className="chip tasks" key={g}>
              serves {g}
            </span>
          ))}
        </div>
      )}
      {item.started_unapproved?.trim() ? (
        <p className="muted small">
          Started without approval: {item.started_unapproved}
        </p>
      ) : null}

      {dimensions.length > 0 && (
        <ul className="dep-list">
          {dimensions.map((d) => (
            <li key={d.column}>
              <strong>{d.column}.</strong> {d.answer}
            </li>
          ))}
        </ul>
      )}

      {requirements.length > 0 && (
        <div className="tiles">
          {requirements.map((r) => (
            <div className="tile" key={r.id}>
              <div className="tile-title">
                <code className="taskkey">{r.id}</code> {r.text}
              </div>
              {r.scenario?.measure ? (
                <div className="tile-desc">Measure: {r.scenario.measure}</div>
              ) : null}
              <div className="tile-states">
                <span className="chip">{r.kind ?? "functional"}</span>
                {(r.iso25010 ?? []).map((q) => (
                  <span className="chip" key={q}>
                    {q}
                  </span>
                ))}
                {(r.tests ?? []).map((t) => (
                  <span className={`chip ${t.state === "green" ? "tasks" : "warn"}`} key={t.name}>
                    {t.state} · {t.name}
                  </span>
                ))}
                {(r.tests ?? []).length === 0 && <span className="chip warn">no test</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );

  return (
    <details className="review-item" id={item.code} open={defaultOpen}>
      {/* The summary is the row you scan; the brief is what you expand to read. */}
      <summary>
        <code className="taskkey">{item.code}</code>
        <span className="review-title">{item.title}</span>
        <span className={`chip ${item.approval === "missing" ? "" : "warn"}`}>
          {ratify
            ? `${item.status} without agreement`
            : waitingOnSignoff
              ? `waiting on sign-off: ${signoffs.join(", ")}`
              : whichRelease
                ? "ready for release: which one?"
              : specChanged
                ? item.finished
                  ? `${item.status}: spec changed since sign-off`
                  : "spec changed since sign-off"
              : item.approval === "added"
                ? item.finished
                  ? `${item.status}: added since approval`
                  : "added since approval"
              : item.approval === "lapsed"
                ? item.finished
                  ? `${item.status}: approval lapsed`
                  : "approval lapsed"
                : "never approved"}
        </span>
        {approved ? <span className={`chip ${outcome.ok ? "done" : "warn"}`}>{outcome.text}</span> : null}
      </summary>

      <div className="review-body">
      <p className="muted small">
        <Link to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(item.code)}`}>
          open {item.code}
        </Link>
        {specChanged
          ? ` · signed off by ${item.spec?.signed_by ?? ""} on ${(item.spec?.signed_at ?? "").slice(0, 10)}; since then the spec changed. Signing again covers it as it stands now.`
          : item.approval === "added"
          ? " · approved once; since then text was only added. Approving again covers what it says now."
          : item.approval === "lapsed"
          ? ` · approved once, then the definition changed — agreeing again covers what it says now${
              item.finished ? ". The work is done." : ""
            }`
          : ratify
            ? " · built under a recorded bypass — ratifying agrees to it after the fact, and the record says so"
            : null}
      </p>

      {changes ? (
        <Changes
          item={item}
          changes={changes}
          heading={
            waitingOnSignoff
              ? "Also reviewed by this sign-off: added since approval"
              : "Since your last decision"
          }
        />
      ) : null}
      {changes ? (
        <details className="review-unchanged">
          <summary>The approved text, unchanged</summary>
          {definitionBody}
        </details>
      ) : (
        definitionBody
      )}

      {item.spec ? <SpecDiff spec={item.spec} changed={specChanged} /> : null}

      {diagrams.failed || diagrams.problems.length > 0 ? (
        <div className="diagram-problems">
          <h3>Diagrams</h3>
          {diagrams.failed ? (
            <p className="diagram-error">The diagram check could not run: {diagrams.failed}</p>
          ) : (
            <ul>
              {diagrams.problems.map((p) => (
                <li key={`${p.file}#${p.block}`}>
                  <Link to={problemHref(project, item.code, p)}>{p.file}</Link>, diagram #{p.block}, line{" "}
                  {p.line}: {p.message}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {/* The action lives in the body, not the summary: approving should follow reading. */}
      {item.signoff_lapsed ? <p>The earlier docs-folded sign-off lapsed: {item.signoff_lapsed}.</p> : null}
      {item.changes_no_documents && signoffs.includes("docs-folded") ? (
        <p>This item changes no documents; signing off confirms that.</p>
      ) : null}
      {signoffs.includes("docs-folded") && item.drift_skipped ? (
        <p className="drift-skipped">Drift check skipped: {item.drift_skipped}.</p>
      ) : null}
      {flags.length ? (
        <div className="drift-flags">
          <h4>Left to check before docs-folded</h4>
          <p>
            Claude reads each text against what {item.code} changed, then confirms it is still right or files a
            correction. Ask Claude to settle them; docs-folded appears here once none is left.
          </p>
          <ul>
            {flags.map((f) => (
              <li key={driftKey(f)}>
                <Link to={driftHref(project, f)}>{f.target.title || f.target.id}</Link> names{" "}
                {f.hits.map((h, i) => (
                  <span key={i}>
                    {i ? "; " : ""}
                    <code>{h.name.text}</code> ({h.lines.length ? `line ${h.lines.join(", ")}` : "no longer in the text"})
                  </span>
                ))}
                {f.commits[0] ? (
                  <span>
                    {" "}
                    — changed by {f.commits[0].item}, {f.commits[0].sha.slice(0, 7)}
                  </span>
                ) : null}
                <EditedWhileFlagged flag={f} />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {ratify && item.overrides?.length ? (
        <ul className="dep-list small">
          {item.overrides.map((o) => (
            <li key={`${o.at}-${o.to}`}>
              Moved {o.from} → {o.to} on {o.at.slice(0, 10)} by Claude, past its gate: {o.reason}
            </li>
          ))}
        </ul>
      ) : null}
      {docsFolded.settled.length ? (
        <details className="drift-settled">
          <summary>Checked by Claude ({docsFolded.settled.length})</summary>
          <ul>
            {docsFolded.settled.map((t) => (
              <li key={driftKey(t.flag)}>
                <Link to={driftHref(project, t.flag)}>{t.flag.target.title || t.flag.target.id}</Link>:{" "}
                {t.check.outcome === "corrected" ? (
                  <>
                    corrected by <Link to={`/p/${project}/feature/${t.check.correction}`}>{t.check.correction}</Link>
                  </>
                ) : (
                  <>still right: {t.check.why}</>
                )}{" "}
                — {t.check.by}
                {t.check.at ? `, ${t.check.at.slice(0, 10)}` : ""}
                <EditedWhileFlagged flag={t.flag} />
              </li>
            ))}
          </ul>
        </details>
      ) : null}
      {!approved
        ? panels.map((p) => (
            <PreparedPanel
              key={p.view.id}
              project={project}
              view={p.view}
              writable={writable}
              busy={busy}
              label={p.label}
              onConfirm={(edits) => onConfirmPrepared(p.view, edits, p.done)}
              onReject={(reason) => onRejectPrepared(p.view, reason)}
            />
          ))
        : null}
      <div className="review-actions">
        {approved ? (
          <OutcomeChip outcome={outcome} onRetry={onRetry} />
        ) : writable ? (
          <>
            {waitingOnSignoff || specChanged || whichRelease || preparedRatify ? null : (
              <>
                <button
                  className="btn btn-primary"
                  onClick={onApprove}
                  disabled={busy || blocker != null}
                  title={blocker ?? undefined}
                >
                  {busy ? "recording…" : `${verb} ${item.code}`}
                </button>
                {blocker ? <p className="muted small review-held">{blockedSentence(verb, diagrams)}</p> : null}
              </>
            )}
            {signoffs.filter((name) => !preparedSignoff(name) && (name !== "docs-folded" || docsFolded.offer)).map((name) => (
              <button
                key={name}
                className={`btn ${waitingOnSignoff || specChanged ? "btn-primary" : ""}`}
                onClick={() => onSignoff(name)}
                disabled={busy}
              >
                {busy ? "recording…" : `Sign off ${name}`}
              </button>
            ))}
            {releaseChoices
              ? releaseChoices.map((v) => (
                  <button
                    key={v}
                    className={`btn ${whichRelease ? "btn-primary" : ""}`}
                    onClick={() => onRelease(v)}
                    disabled={busy}
                  >
                    {busy ? "recording…" : `In release ${v}`}
                  </button>
                ))
              : null}
            {releaseChoices && !adding ? (
              <button className="btn" onClick={() => setAdding(true)} disabled={busy}>
                New release…
              </button>
            ) : null}
            {releaseChoices && adding ? (
              <>
                <input
                  className="review-reason"
                  placeholder="Version, e.g. v0.2.0"
                  value={version}
                  onChange={(e) => setVersion(e.target.value)}
                  aria-label={`Version of the new release for ${item.code}`}
                />
                <input
                  className="review-reason"
                  type="date"
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  aria-label={`Target date of the new release for ${item.code}`}
                />
                <button
                  className="btn btn-primary"
                  onClick={() => onNewRelease(version.trim(), target)}
                  disabled={busy || version.trim() === ""}
                >
                  {busy ? "recording…" : `Add ${version.trim() || "release"} with ${item.code}`}
                </button>
                <button className="btn" onClick={() => setAdding(false)} disabled={busy}>
                  Cancel
                </button>
              </>
            ) : null}
          </>
        ) : (
          <code>
            {[
              ...(waitingOnSignoff
                ? signoffs
                    .filter((n) => !(n === "docs-folded" && flags.length > 0))
                    .map((n) => `kanbanr signoff ${item.code} ${n}`)
                : whichRelease
                  ? []
                  : [`kanbanr ${verb.toLowerCase()} ${item.code}`]),
              ...(releaseChoices
                ? [
                    releaseChoices.length > 0
                      ? `kanbanr release plan ${releaseChoices.join("|")} ${item.code}`
                      : `kanbanr release add <VERSION> --target YYYY-MM-DD && kanbanr release plan <VERSION> ${item.code}`,
                  ]
                : []),
            ].join("  ·  ")}
          </code>
        )}
      </div>
      </div>
    </details>
  );
}

/** The text of one definition part, by its key (FTR-018): `statement`, `goals/<id>`,
 * `zachman/<column>`, `requirements/<id>` and `requirements/<id>/tests/<name>`. */
function partText(def: NonNullable<PendingReview["definition"]>, key: string): string {
  if (key === "statement") return def.statement ?? "";
  if (key.startsWith("goals/")) return key.slice("goals/".length);
  if (key.startsWith("zachman/")) {
    const column = key.slice("zachman/".length);
    const found = zachmanColumns(def.zachman).find((d) => d.column.toLowerCase() === column);
    return found ? `${found.column}. ${found.answer}` : column;
  }
  if (key.startsWith("requirements/")) {
    const rest = key.slice("requirements/".length);
    // A test's name may hold "/", so split on the tests marker and never on every slash.
    const at = rest.indexOf("/tests/");
    const id = at < 0 ? rest : rest.slice(0, at);
    const requirement = (def.requirements ?? []).find((r, i) => (r.id || `#${i}`) === id);
    if (at < 0) return requirement ? `${requirement.id} ${requirement.text}` : id;
    const name = rest.slice(at + "/tests/".length);
    const test = (requirement?.tests ?? []).find((t) => t.name === name);
    return test ? `${id} test ${test.name}${test.kind ? ` (${test.kind})` : ""}` : name;
  }
  return key;
}

/** What was added or changed since the last decision, first on the brief (FTR-018). */
function Changes({
  item,
  changes,
  heading,
}: {
  item: PendingReview;
  changes: NonNullable<PendingReview["changes"]>;
  heading: string;
}) {
  const def = item.definition ?? {};
  return (
    <div className="review-changes">
      <h3>{heading}</h3>
      <ul className="dep-list">
        {changes.added.map((k) => (
          <li key={`a-${k}`}>
            <span className="chip tasks">added</span> {partText(def, k)}
          </li>
        ))}
        {changes.changed.map((k) => (
          <li key={`c-${k}`}>
            <span className="chip warn">changed</span> {partText(def, k)}
          </li>
        ))}
        {changes.removed.map((k) => (
          <li key={`r-${k}`}>
            <code>{k}</code> removed
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * The edits made to a flagged text while its flag was open, with their diff (DEF-050). Shown so the
 * person who signs sees a text that changed while flagged; the edit settles nothing.
 */
function EditedWhileFlagged({ flag }: { flag: DriftFlag }) {
  const edited = editedWhileFlagged(flag);
  if (!edited) return null;
  return (
    <div className="drift-edited">
      <strong>Edited while flagged</strong>: {edited.count} {edited.count === 1 ? "edit" : "edits"} (
      {flag.edits?.map((e, i) => (
        <span key={`${e.sha}-${e.at}`}>
          {i ? "; " : ""}
          <code>{edited.shas[i]}</code> {e.at.slice(0, 10)} {e.subject}
        </span>
      ))}
      ). Only Claude&apos;s check settles this flag, not the edit.
      {edited.diff ? (
        <details>
          <summary>Diff of the edits</summary>
          <pre>{edited.diff}</pre>
          {edited.truncated ? <p>The diff is cut short.</p> : null}
        </details>
      ) : null}
    </div>
  );
}

/** How a flag is named in a sign-off: `doc:<path>`, `adr:<id>` or `item:<CODE>` (FTR-013). */
function driftKey(f: DriftFlag): string {
  return `${f.target.kind}:${f.target.id}`;
}

/** Where a flagged text is read: a board doc or an item. */
function driftHref(project: string, f: DriftFlag): string {
  return f.target.kind === "item"
    ? `/p/${project}/feature/${f.target.id}`
    : `/p/${project}/docs/file/${f.target.path ?? f.target.id}`;
}
