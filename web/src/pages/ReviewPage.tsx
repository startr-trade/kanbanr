import { useState } from "react";
import { useParams } from "react-router-dom";
import { alreadySaid, api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { LiveDot } from "../components/bits";
import type { Adr, GroupCount, GroupSlice, PendingReview, PlannedRelease, ReviewRow as Row, RowEvidence } from "../types";
import { batches } from "../prepared";
import type { ManualRow, Outcome } from "../manualTests";
import type { PreparedView } from "../types";
import { useQuery } from "../data/useQuery";
import { urls } from "../data/urls";
import { GROUP_PARAM, groupTabs } from "../reviewGroups";
import { Empty, ErrorState, LoadingState, Tabs } from "../ui";
import { AdrEvidence } from "./review/AdrEvidence";
import { DecisionsHere } from "./review/DecisionsHere";
import { DelegationEvidence } from "./review/DelegationEvidence";
import { EntryEvidence } from "./review/EntryEvidence";
import { GroupView, REVIEW_TAGS } from "./review/GroupView";
import { ManualEvidence } from "./review/ManualEvidence";
import { SystemBaselineEvidence } from "./review/SystemBaselineEvidence";
import { GROUP_KEY as SYSTEM_BASELINE_GROUP } from "../systemBaselineView";

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

  // A row's evidence, once read (GET /review/groups/{key}/rows/{row}): an item's (EntryEvidence),
  // a manual check's, a proposed ADR's, or a delegation's, each with the single actions it keeps in
  // its row (R-11): Passed and Failed, Reject with a reason, Undo with a reason.
  const evidence = (row: Row, e: RowEvidence) => {
    if (e.entry) {
      const item = e.entry;
      return (
        <EntryEvidence
          project={project}
          item={item}
          tick={tick}
          writable={writable && !inConversation}
          busy={busy === item.code}
          outcome={done[item.code]}
          onRetry={() => retry(item.code)}
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
        <ManualEvidence
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
        <AdrEvidence
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
    if (e.delegation || e.delegated) {
      return (
        <DelegationEvidence
          project={project}
          delegation={e.delegation}
          delegated={e.delegated}
          writable={writable}
          who={who}
        />
      );
    }
    return (
      <p className="small">
        {row.what}. {row.records}.
      </p>
    );
  };
  // The system baseline's own row (FTR-006): the project's one baseline, never an item's entry.
  const baselineEvidence = () => (
    <SystemBaselineEvidence project={project} writable={writable} who={who} onDecided={summary.reload} />
  );

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
              batches={batches(prepared.data)}
              body={(row, slice: GroupSlice) =>
                slice.key === SYSTEM_BASELINE_GROUP ? (
                  baselineEvidence()
                ) : (
                  <RowBody project={project} group={slice.key} row={row} render={evidence} />
                )
              }
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

const manualKey = (m: ManualRow) => `manual:${m.row.code}:${m.row.requirement}:${m.row.test}`;
