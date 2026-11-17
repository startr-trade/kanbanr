import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../../api";
import { useAsync } from "../../live";
import { ErrorBox, Loading } from "../../components/bits";
import { releaseAction } from "../../releaseAction";
import type { PreparedView, Release, ReleasePlan, TagResult } from "../../types";
import { PreparedPanel } from "../../components/PreparedPanel";
import { AskFirst } from "../../components/AskFirst";
import { openFor } from "../../prepared";

/**
 * A release's one button (FEAT-212): named for its next step, with what the step will do beside it,
 * the cut's preview inline, and — until the plan is ready — what is missing and how to fix it.
 */
export function ReleaseButton({
  project,
  release,
  tick,
  writable,
  busy,
  onPromote,
  outcome,
  gates,
  who,
  prepared,
  onShown,
}: {
  project: string;
  release: Release;
  tick: number;
  writable: boolean;
  gates: string[];
  who: string | null;
  busy: string | null;
  onPromote: (r: Release, to: string, expect: string, note?: string, acceptUntested?: string[]) => void;
  outcome?: string;
  prepared: PreparedView[];
  onShown: (version: string, ids: string[]) => void;
}) {
  // A confirmation after the tag asks what the user checked before it goes (FTR-026).
  const [asking, setAsking] = useState(false);
  const [note, setNote] = useState("");
  // "Check again" re-reads the plan: a `git push` updates the remote-tracking ref locally, so no
  // fetch is needed (the monitor never fetches).
  const [again, setAgain] = useState(0);
  // The last promotion's note says what came next ("push main…"); once the plan has been read
  // again it may no longer hold, so it is put away until the next promotion.
  const [hideOutcome, setHideOutcome] = useState(false);
  useEffect(() => setHideOutcome(false), [outcome]);
  const [preparedSaid, setPreparedSaid] = useState<string | null>(null);
  // The person's acceptance of what the cut ships untested (DEF-043), for the list the plan showed.
  const [accepted, setAccepted] = useState(false);
  const plan = useAsync(() => api.getReleasePlan(project, release.version), [project, release.version, tick, again]);
  // Which prepared records this button's forms show (DEF-095), told to the page once the plan is
  // read, so the page lists the others under "Prepared, not required". None when the plan fails.
  const shownKey = plan.error ? "" : plan.data ? shownByButton(plan.data, writable, release.version, gates, prepared).join(",") : null;
  useEffect(() => {
    if (shownKey != null) onShown(release.version, shownKey ? shownKey.split(",") : []);
  }, [shownKey, release.version, onShown]);
  if (plan.error) return <ErrorBox error={plan.error} />;
  if (!plan.data) return <Loading />;
  const action = releaseAction(plan.data, writable);
  // Ticking the acceptance is the one thing the cut still waits for (DEF-043).
  const enabled = action.enabled || (action.accept != null && action.button && accepted);
  // Why the button is disabled, beside it and as its title (DEF-071); none once the acceptance is ticked.
  const why = enabled ? "" : action.disabledWhy;
  if (action.note) {
    return (
      <p className="muted small">
        <Inline text={action.note} />
      </p>
    );
  }
  const recheck = () => {
    setHideOutcome(true);
    setAgain((n) => n + 1);
  };
  // A confirmation after the tag that Claude prepared (FTR-034): what was checked, filled in, beside
  // the one button, pinned to the release tip this plan showed.
  const preparedPromotion = action.confirm && !plan.data.awaiting
    ? openFor(prepared, "promotion", { release: release.version, stage: plan.data.to })
    : undefined;
  const tip = plan.data.release_tip;
  // A direct press asks first (DEF-034); the question is dropped when an outcome arrives or the plan
  // names another stage or another again offer, and kept across live ticks.
  const askKey = `${outcome ?? ""}|${plan.data.to}|${plan.data.again?.stage ?? ""}|${plan.data.again?.button ?? ""}`;
  // The release waiting at its last stage is shipped with no stage named (DEF-049).
  const promoteTo = plan.data.awaiting ? "" : plan.data.to;
  const confirmPromotion = async (view: PreparedView, edits: Record<string, unknown>) => {
    if (!who) return;
    try {
      await api.confirmPrepared(project, view, who, edits, { expect: tip });
      setPreparedSaid(`${action.label}: done`);
    } catch (e) {
      setPreparedSaid(e instanceof Error ? e.message : String(e));
    }
    recheck();
  };
  const rejectPromotion = async (view: PreparedView, reason: string) => {
    if (!who) return;
    try {
      await api.rejectPrepared(project, view.id ?? "", who, reason);
      setPreparedSaid("rejected what Claude prepared");
    } catch (e) {
      setPreparedSaid(e instanceof Error ? e.message : String(e));
    }
    recheck();
  };
  return (
    <div className="release-action">
      {plan.data.tag ? (
        <TagPanel
          project={project}
          release={release}
          tag={plan.data.tag}
          gates={gates}
          writable={writable}
          who={who}
          secondary={action.secondary}
          onChanged={recheck}
          prepared={prepared}
        />
      ) : null}
      {plan.data.warnings?.length ? (
        <ul className="dep-list small">
          {plan.data.warnings.map((w) => (
            <li key={w}>
              <span className="chip warn">warning</span> <Inline text={w} />
            </li>
          ))}
        </ul>
      ) : null}
      {action.returned ? (
        <div className="small release-returned">
          <p>
            <span className="chip warn">sent back</span> to <strong>{release.stage ?? "its first stage"}</strong>:{" "}
            <Inline text={action.returned.item} /> was merged into <code>{release.branch}</code> on{" "}
            {action.returned.at.slice(0, 10)}
            {action.returned.from ? `, when it was at ${action.returned.from}` : ""}.
          </p>
          <ol>
            {action.returned.steps.map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ol>
        </div>
      ) : null}
      {preparedPromotion && action.button ? (
        <PreparedPanel
          project={project}
          view={preparedPromotion}
          writable={writable}
          busy={busy != null}
          label={action.label}
          onConfirm={(edits) => confirmPromotion(preparedPromotion, edits)}
          onReject={(reason) => rejectPromotion(preparedPromotion, reason)}
        />
      ) : null}
      {preparedSaid ? (
        <p className="small">
          <span className="chip">{preparedSaid}</span>
        </p>
      ) : null}
      <div className="review-actions">
        {preparedPromotion && action.button ? null : action.button && action.confirm ? (
          <button
            className="btn btn-primary"
            onClick={() => setAsking(true)}
            disabled={!enabled || busy != null}
            title={why || action.does.join("; ")}
          >
            {busy === release.version ? "promoting…" : action.label}
          </button>
        ) : action.button ? (
          <AskFirst
            key={`primary|${askKey}`}
            className="btn btn-primary"
            label={busy === release.version ? "promoting…" : action.label}
            ask={action.ask ?? `${action.label}?`}
            onConfirm={() => onPromote(release, promoteTo, plan.data!.release_tip, "", accepted ? action.accept : undefined)}
            disabled={!enabled || busy != null}
            disabledWhy={why}
          />
        ) : (
          <code>{action.command}</code>
        )}
        {why && action.button && !preparedPromotion ? (
          <span className="small release-why" role="note">
            <Inline text={why} />
          </span>
        ) : null}
        {outcome && !hideOutcome ? (
          <span className={`chip ${outcome.startsWith("promoted") ? "done" : "warn"}`}>
            <Inline text={outcome} />
          </span>
        ) : null}
      </div>
      {action.again ? (
        <div className="small release-again">
          {writable ? (
            <AskFirst
              key={`again|${askKey}`}
              label={action.again.label}
              ask={action.again.ask}
              onConfirm={() => onPromote(release, action.again!.stage, plan.data!.release_tip)}
              disabled={!action.again.enabled || busy != null}
              disabledWhy={action.again.disabledWhy}
            />
          ) : null}
          {writable && action.again.disabledWhy ? (
            <>
              {" "}
              <span className="small release-why" role="note">
                <Inline text={action.again.disabledWhy} />
              </span>
            </>
          ) : null}
          {action.again.does[0] ? (
            <>
              {" "}
              <Inline text={action.again.does[0]} />
            </>
          ) : null}
          {writable ? null : (
            <>
              {" "}
              <code>{action.again.command}</code>
            </>
          )}
          {action.again.missing.length ? (
            <>
              <p>
                <strong>Needs:</strong>
              </p>
              <ul className="dep-list">
                {action.again.missing.map((m) => (
                  <li key={m.text}>
                    <Inline text={m.text} />
                  </li>
                ))}
              </ul>
            </>
          ) : null}
        </div>
      ) : null}
      {asking && action.confirm ? (
        <div className="small">
          <p>
            <label>
              What you checked (optional){" "}
              <input
                value={note}
                onChange={(e) => setNote(e.target.value)}
                placeholder="e.g. smoke test passed on staging"
                aria-label="What you checked"
              />
            </label>
          </p>
          <p className="review-actions">
            <button
              className="btn btn-primary"
              disabled={busy != null}
              onClick={() => {
                setAsking(false);
                onPromote(release, promoteTo, plan.data!.release_tip, note.trim());
                setNote("");
              }}
            >
              {action.label}
            </button>
            <button className="btn" onClick={() => setAsking(false)}>
              Cancel
            </button>
          </p>
        </div>
      ) : null}
      {action.does.length ? (
        <ul className="dep-list small">
          {action.does.map((d) => (
            <li key={d}>
              <Inline text={d} />
            </li>
          ))}
        </ul>
      ) : null}
      {action.preview ? <CutPreviewList preview={action.preview} /> : null}
      {action.accept ? (
        <div className="small release-untested">
          <p>
            <strong>Not verified:</strong> <Inline text={action.acceptDetail ?? ""} />
          </p>
          <ul className="dep-list">
            {action.accept.map((r) => (
              <li key={r}>
                <code>{r}</code>
              </li>
            ))}
          </ul>
          {action.button ? (
            <p>
              <label>
                <input
                  type="checkbox"
                  checked={accepted}
                  onChange={(e) => setAccepted(e.target.checked)}
                  aria-label="Ship these unverified; the notes will name them"
                />{" "}
                Ship these unverified; the notes will name them
              </label>
            </p>
          ) : null}
        </div>
      ) : null}
      {action.missing.length ? (
        <div className="small">
          <p>
            <strong>Needs:</strong>
          </p>
          <ul className="dep-list">
            {action.missing.map((m) => (
              <li key={m.text}>
                <Inline text={m.text} />
                {m.fix ? (
                  <>
                    {" — "}
                    <Inline text={m.fix} />
                  </>
                ) : null}
              </li>
            ))}
          </ul>
          {action.gate ? (
            <RecordResult
              project={project}
              commit={action.gate.commit}
              gates={gates}
              writable={writable}
              who={who}
              command={action.gate.record}
              prepared={prepared}
              onRecorded={() => {
                setHideOutcome(true);
                setAgain((n) => n + 1);
              }}
            />
          ) : null}
          <button
            className="btn"
            onClick={() => {
              setHideOutcome(true);
              setAgain((n) => n + 1);
            }}
            disabled={plan.loading}
          >
            Check again
          </button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * A tagged release waiting on its later stages (FTR-026): the commit the tag names and the result
 * recorded for it. With none, Record result; when it failed, check again after a rerun or start a
 * patch release. kanbanr asks no server: the user's CI ran, and the user says what it showed.
 */
export function TagPanel({
  project,
  release,
  tag,
  gates,
  writable,
  who,
  secondary,
  onChanged,
  prepared,
}: {
  project: string;
  release: Release;
  tag: TagResult;
  gates: string[];
  writable: boolean;
  who: string | null;
  secondary: ("runAgain" | "startPatch")[];
  onChanged: () => void;
  prepared: PreparedView[];
}) {
  const sha = tag.commit.slice(0, 7);
  const r = tag.result;
  const [patching, setPatching] = useState(false);
  const [version, setVersion] = useState("");
  const [link, setLink] = useState(r?.url ?? "");
  const [notes, setNotes] = useState("");
  const [said, setSaid] = useState<{ ok: boolean; text: string; defect?: string } | null>(null);
  const start = async () => {
    if (!who) return;
    try {
      const made = await api.patchFromFailedTag(project, release.version, version.trim(), who, link.trim(), notes.trim());
      setSaid({ ok: true, text: `${made.release.version} started, with ${made.defect} for the failed run`, defect: made.defect });
      setPatching(false);
      onChanged();
    } catch (e) {
      setSaid({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  };
  const base = `/p/${encodeURIComponent(project)}`;
  return (
    <div className="small">
      <p>
        Tagged <code>{release.version}</code> at <code>{sha}</code>
        {tag.state === "none" ? (
          <>
            . No result recorded since the tag.
          </>
        ) : r ? (
          <>
            . {tag.gate} <strong>{tag.state}</strong>, recorded by {r.by} on {r.at.slice(0, 10)}
            {r.url ? (
              <>
                {" "}
                (<a href={r.url} target="_blank" rel="noreferrer">run</a>)
              </>
            ) : null}
            {r.note ? <> — {r.note}</> : null}
          </>
        ) : null}
      </p>
      {tag.state !== "passed" && tag.gate ? (
        <>
          {tag.state === "failed" ? (
            <p>
              Run your CI again on <code>{sha}</code>, record the new result, then check again.
            </p>
          ) : null}
          <RecordResult
            project={project}
            commit={tag.commit}
            gates={gates}
            writable={writable}
            who={who}
            command={`kanbanr gate record ${tag.gate} --passed --commit ${tag.commit}`}
            prepared={prepared}
            onRecorded={onChanged}
          />
        </>
      ) : null}
      {secondary.length ? (
        <p className="review-actions">
          {secondary.includes("runAgain") ? (
            <button className="btn" onClick={onChanged}>
              Check again
            </button>
          ) : null}
          {secondary.includes("startPatch") ? (
            writable && who ? (
              <button className="btn" onClick={() => setPatching((p) => !p)}>
                Start a patch release
              </button>
            ) : (
              <code>kanbanr release add &lt;version&gt; --patch-of {release.version} --from-failed-tag</code>
            )
          ) : null}
        </p>
      ) : null}
      {patching ? (
        <div>
          <p>
            <input
              value={version}
              onChange={(e) => setVersion(e.target.value)}
              placeholder="new version, e.g. v1.0.1"
              aria-label="New version"
            />{" "}
            <input
              type="url"
              value={link}
              onChange={(e) => setLink(e.target.value)}
              placeholder="link to the failed run"
              aria-label="Link to the failed run"
            />
          </p>
          <p>
            <input
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              placeholder="what you saw"
              aria-label="Notes"
            />
          </p>
          <p className="review-actions">
            <button className="btn btn-primary" onClick={start} disabled={!version.trim()}>
              Start {version.trim() || "the patch release"}
            </button>
            <button className="btn" onClick={() => setPatching(false)}>
              Cancel
            </button>
          </p>
        </div>
      ) : null}
      {said ? (
        <p>
          <span className={`chip ${said.ok ? "done" : "warn"}`}>{said.text}</span>
          {said.defect ? (
            <>
              {" "}
              <Link to={`${base}/feature/${encodeURIComponent(said.defect)}`}>{said.defect}</Link>
            </>
          ) : null}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The ids of the prepared records a release's button shows (DEF-095), by the same rules it renders
 * them: the promotion beside the button, and the gate result in the tag's and the plan's Record
 * result forms.
 */
function shownByButton(plan: ReleasePlan, writable: boolean, version: string, gates: string[], prepared: PreparedView[]): string[] {
  const action = releaseAction(plan, writable);
  if (action.note) return [];
  const ids: string[] = [];
  const promotion =
    action.confirm && !plan.awaiting ? openFor(prepared, "promotion", { release: version, stage: plan.to }) : undefined;
  if (promotion?.id && action.button) ids.push(promotion.id);
  if (plan.tag && plan.tag.state !== "passed" && plan.tag.gate) {
    const v = gateResultFor(prepared, gates, plan.tag.commit);
    if (v?.id) ids.push(v.id);
  }
  if (action.missing.length && action.gate) {
    const v = gateResultFor(prepared, gates, action.gate.commit);
    if (v?.id) ids.push(v.id);
  }
  return ids;
}

/**
 * Record what your CI said about the commit a promotion needs (FTR-002): which gate, passed or
 * failed, a link to the run and a note. kanbanr asks no server; a person records the verdict. On a
 * read-only monitor it shows the command instead.
 */
export function RecordResult({
  project,
  commit,
  gates,
  writable,
  who,
  command,
  onRecorded,
  prepared,
}: {
  project: string;
  commit: string;
  gates: string[];
  writable: boolean;
  who: string | null;
  command: string;
  onRecorded: () => void;
  prepared: PreparedView[];
}) {
  const [gate, setGate] = useState(gates[0] ?? "ci");
  const [url, setUrl] = useState("");
  const [note, setNote] = useState("");
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
  // A result Claude read from the CI and prepared (FTR-034): shown filled in, confirmed with one
  // click; Edit offers the same fields as the form.
  const view = gateResultFor(prepared, gates, commit);
  if (view) {
    const passed = view.fields?.passed === true;
    const answer = async (go: () => Promise<unknown>, text: string) => {
      try {
        await go();
        setSaid({ ok: true, text });
        onRecorded();
      } catch (e) {
        setSaid({ ok: false, text: e instanceof Error ? e.message : String(e) });
      }
    };
    return (
      <div className="small">
        <PreparedPanel
          project={project}
          view={view}
          writable={writable && who != null}
          busy={false}
          label={`Record ${view.subject.gate} ${passed ? "passed" : "failed"}`}
          onConfirm={(edits) =>
            who &&
            answer(
              () => api.confirmPrepared(project, view, who, edits),
              `${view.subject.gate} recorded on ${commit.slice(0, 7)}`,
            )
          }
          onReject={(reason) =>
            who && answer(() => api.rejectPrepared(project, view.id ?? "", who, reason), "rejected what Claude prepared")
          }
        />
        {said ? <span className={`chip ${said.ok ? "done" : "warn"}`}>{said.text}</span> : null}
      </div>
    );
  }
  if (!writable || !who) {
    return (
      <p className="small">
        Record your CI's verdict from the project folder: <code>{command}</code>
      </p>
    );
  }
  const record = async (passed: boolean) => {
    try {
      await api.recordGate(project, gate, commit, passed, who, url.trim(), note.trim());
      setSaid({ ok: true, text: `${gate} ${passed ? "passed" : "failed"} on ${commit.slice(0, 7)}` });
      setUrl("");
      setNote("");
      onRecorded();
    } catch (e) {
      setSaid({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  };
  return (
    <div className="small">
      <p>
        <strong>Record result</strong> for <code>{commit.slice(0, 7)}</code>, as your CI showed it:
      </p>
      <p>
        <select value={gate} onChange={(e) => setGate(e.target.value)} aria-label="Gate">
          {(gates.length ? gates : ["ci"]).map((g) => (
            <option key={g}>{g}</option>
          ))}
        </select>{" "}
        <input
          type="url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="link to the run (optional)"
          aria-label="Link to the run"
        />{" "}
        <input
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="note (optional)"
          aria-label="Note"
        />
      </p>
      <p className="review-actions">
        <button className="btn btn-primary" onClick={() => record(true)}>
          Passed
        </button>
        <button className="btn" onClick={() => record(false)}>
          Failed
        </button>
        {said ? <span className={`chip ${said.ok ? "done" : "warn"}`}>{said.text}</span> : null}
      </p>
    </div>
  );
}

/** The prepared gate result RecordResult shows for `commit`: the newest open one for its gates. */
function gateResultFor(prepared: PreparedView[], gates: string[], commit: string): PreparedView | undefined {
  return (gates.length ? gates : ["ci"])
    .map((g) => openFor(prepared, "gate-result", { gate: g, commit }))
    .find((v) => v != null);
}

/** Exactly what the cut would tag, ship and carry over, from the plan (FEAT-185, FEAT-212). */
export function CutPreviewList({
  preview,
}: {
  preview: { tag: string; ships: string[]; carries: { code: string; why: string }[] };
}) {
  return (
    <div className="small">
      {preview.tag ? (
        <p>
          Tags: <code>{preview.tag}</code>
        </p>
      ) : null}
      <p>Ships: {preview.ships.length ? preview.ships.join(", ") : "nothing"}</p>
      {preview.carries.length ? (
        <ul className="dep-list">
          {preview.carries.map((p) => (
            <li key={p.code}>
              carries {p.code}: {p.why}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** A message from the plan, with its `code` spans shown as code rather than as backticks. */
export function Inline({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>))}
    </>
  );
}
