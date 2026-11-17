import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, viaOrigin } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { releaseAction } from "../releaseAction";
import { notesSection } from "../releaseNotes";
import Markdown from "../components/Markdown";
import type { PreparedView, Release, ReleasePlan, TagResult } from "../types";
import { PreparedPanel } from "../components/PreparedPanel";
import { AskFirst } from "../components/AskFirst";
import { openFor, unshownFor } from "../prepared";

/** The stages a release moves through when the project names none (FEAT-183). */
const DEFAULT_STAGES = ["candidate", "test", "staging", "production"];

/**
 * Releases (FEAT-120, FEAT-123): each release's planned scope, how much of it is finished, its
 * target, and — once cut — its notes. Only for projects that use releases; the tab is not offered
 * elsewhere.
 */
export default function ReleasesPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const board = useAsync(() => api.getProject(project), [project, tick]);
  const releases = useAsync(() => api.getReleases(project), [project, tick]);
  const meta = useAsync(() => api.getMeta(), []);
  // What Claude prepared for these decisions (FTR-034): CI results and confirmations after the tag.
  const prepared = useAsync(() => api.getPrepared(project), [project, tick]);
  // Promote (FEAT-185): which release is being promoted, and what came of it.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});
  // The prepared records each release's forms already show (DEF-095), by version, as the release's
  // button reports them; the rest are listed under "Prepared, not required".
  const [shown, setShown] = useState<Record<string, string[]>>({});
  const onShown = useCallback(
    (version: string, ids: string[]) =>
      setShown((s) => (s[version]?.join(",") === ids.join(",") ? s : { ...s, [version]: ids })),
    [],
  );

  if ((board.loading && !board.data) || (releases.loading && !releases.data)) return <Loading />;
  if (board.error) return <ErrorBox error={board.error} />;
  if (releases.error) return <ErrorBox error={releases.error} />;
  if (!board.data) return null;

  const config = board.data.config;
  if (!config.cadence?.releases) {
    return (
      <div className="releases-page">
        <h1>Releases</h1>
        <p className="muted">
          This project does not use releases. Switch them on with{" "}
          <code>kanbanr config cadence --releases on</code>.
        </p>
      </div>
    );
  }
  const list = releases.data ?? [];
  const base = `/p/${encodeURIComponent(project)}`;
  const stages = config.cadence.release_stages?.length ? config.cadence.release_stages : DEFAULT_STAGES;
  const last = stages[stages.length - 1];
  // Recorded as the board's commit identity, as `kanbanr release promote` records it.
  const who = api.approver(meta.data ?? null);
  // A read-only copy of the monitor (FEAT-178) never offers a promotion, whatever the daemon says.
  const writable = !isReadOnly() && meta.data?.writes === true && who != null;
  // The stage after the one it is at; the first, before it has one. None once it is at the last.
  const nextStage = (r: Release) => {
    // A superseded release never ships: its tag stays, and the patch carries on (FTR-026).
    if (r.state === "shipped" || r.superseded_by) return null;
    const at = r.stage ? stages.indexOf(r.stage) : -1;
    // Tagged at the last stage, it waits for its tag workflow's result and ships on it (DEF-049):
    // the stage it is at is the one the button still speaks for.
    if (r.tagged_at && at >= 0 && at + 1 === stages.length) return stages[at];
    return at + 1 < stages.length ? stages[at + 1] : null;
  };
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183), carrying
  // the release tip the plan showed (FEAT-212).
  const promote = async (r: Release, to: string, expect: string, note = "", acceptUntested?: string[]) => {
    if (!who) return;
    setBusy(r.version);
    try {
      const done = (await api.promoteRelease(project, r.version, who, to, expect, note, acceptUntested)) as {
        next?: string;
      };
      // A stage taken by mistake is stepped back from the CLI, by the person (DEF-034): the page
      // offers no Undo button, so the chip names the command.
      const undo = to ? ` — undo with \`kanbanr release undo ${r.version} --reason …\` while nothing is pushed` : "";
      setOutcome((o) => ({ ...o, [r.version]: `${to ? `promoted to ${to}` : `promoted: ${r.version} shipped`}${done?.next ? ` — ${done.next}` : ""}${undo}` }));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({
        ...o,
        [r.version]: `${message} — from the project folder: kanbanr release promote ${r.version} ${to}${note ? ` --note "${note}"` : ""}`,
      }));
    } finally {
      setBusy(null);
    }
  };

  // Offered only for a release that holds nothing and has not started (FTR-022): the same rule the
  // daemon applies, so the button never offers what would be refused.
  const removable = (r: Release) =>
    r.state === "planned" &&
    !r.stage &&
    !r.branch &&
    !r.notes_doc &&
    (r.items ?? []).length === 0 &&
    !list.some((o) => o.parent === r.version);
  const remove = async (r: Release) => {
    if (!window.confirm(`Remove release ${r.version}? It holds nothing.`)) return;
    setBusy(r.version);
    try {
      await api.removeRelease(project, r.version);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setOutcome((o) => ({ ...o, [r.version]: `${message} — from the project folder: kanbanr release remove ${r.version}` }));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="releases-page">
      <div className="page-head">
        <h1>Releases</h1>
        <LiveDot />
      </div>
      {list.length === 0 ? (
        <p className="muted">
          No releases yet — <code>kanbanr release add v0.1.0 --target YYYY-MM-DD</code>.
        </p>
      ) : (
        <div className="tiles">
          {list.map((r) => {
            const holds = new Set(r.items ?? []);
            const planned = board.data!.features.filter((f) => holds.has(f.code));
            // Counted by the daemon, by the same rule as the sprint's (FEAT-138): this page used
            // to keep its own copy, which counted only the end status.
            const finished = r.finished ?? 0;
            const pct = planned.length ? Math.round((finished / planned.length) * 100) : 0;
            const next = nextStage(r);
            return (
              <div className="tile" key={r.version}>
                <div className="tile-title">
                  {r.version} {r.name ? `· ${r.name}` : ""}{" "}
                  <span className={`chip ${r.state === "shipped" ? "done" : ""}`}>{r.state}</span>{" "}
                  <span
                    className={`chip ${r.stage === last ? "done" : ""}`}
                    title={
                      (r.stages ?? []).length > 0
                        ? (r.stages ?? [])
                            .map((c) => `${c.stage} · ${c.by} · ${c.at.slice(0, 10)}${viaOrigin(c.origin)}${c.reason ? ` — ${c.reason}` : ""}`)
                            .join("\n")
                        : "not yet promoted"
                    }
                  >
                    {r.stage ? `stage ${r.stage}` : "not yet a candidate"}
                  </span>
                </div>
                <div className="tile-states">
                  {r.target ? <span className="chip">target {r.target}</span> : null}
                  {r.tagged_at && r.state !== "shipped" && !r.superseded_by ? (
                    <span className="chip warn">tagged, not released yet</span>
                  ) : null}
                  {r.superseded_by ? (
                    <span className="chip warn">Superseded by {r.superseded_by}</span>
                  ) : null}
                  {r.shipped_at ? <span className="chip">shipped {r.shipped_at.slice(0, 10)}</span> : null}
                  <span className="chip tasks">
                    {finished}/{planned.length} finished ({pct}%)
                  </span>
                  {(r.carried ?? []).length > 0 ? (
                    <span className="chip warn" title={(r.carried ?? []).map((c) => `${c.code} → ${c.to}`).join(", ")}>
                      {(r.carried ?? []).length} carried over
                    </span>
                  ) : null}
                </div>
                <ul className="dep-list">
                  {planned.map((f) => (
                    <li key={f.code}>
                      <Link to={`${base}/feature/${encodeURIComponent(f.code)}`}>{f.code}</Link> {f.title}{" "}
                      <span className="muted small">({f.status})</span>
                    </li>
                  ))}
                </ul>
                {writable && removable(r) ? (
                  <p>
                    <button className="btn" onClick={() => remove(r)} disabled={busy === r.version}>
                      Remove
                    </button>
                    {outcome[r.version] ? <span className="muted small"> {outcome[r.version]}</span> : null}
                  </p>
                ) : null}
                {next ? (
                  <ReleaseButton
                    project={project}
                    release={r}
                    tick={tick}
                    writable={writable}
                    gates={config.cadence?.commit_gates ?? []}
                    who={who}
                    busy={busy}
                    onPromote={promote}
                    outcome={outcome[r.version]}
                    prepared={prepared.data ?? []}
                    onShown={onShown}
                  />
                ) : null}
                {/* Until the button has said what its forms show, nothing is listed twice. */}
                {next && shown[r.version] === undefined ? null : (
                  <PreparedNotRequired
                    project={project}
                    views={unshownFor(prepared.data, r.version, new Set(next ? shown[r.version] : []))}
                    writable={writable}
                    who={who}
                  />
                )}
                {r.branch && config.cadence?.release_prepare?.notes ? (
                  <ReleaseNotesSection
                    project={project}
                    release={r}
                    notesPath={config.cadence.release_prepare.notes}
                    tick={tick}
                    writable={writable}
                    who={who}
                  />
                ) : null}
                {r.notes_doc ? (
                  <p className="small">
                    <Link to={`${base}/docs/file/${r.notes_doc.split("/").map(encodeURIComponent).join("/")}`}>
                      Release notes
                    </Link>
                  </p>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/**
 * A release's one button (FEAT-212): named for its next step, with what the step will do beside it,
 * the cut's preview inline, and — until the plan is ready — what is missing and how to fix it.
 */
function ReleaseButton({
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
function TagPanel({
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
 * The open prepared records about a release that no form on its entry shows (DEF-095): a gate
 * result for a gate no stage asks for, a promotion for a stage that is not the plan's next. Each is
 * shown in its own PreparedPanel, confirmed or rejected there, so nothing waits unseen. On a
 * read-only monitor the panel names the command.
 */
function PreparedNotRequired({
  project,
  views,
  writable,
  who,
}: {
  project: string;
  views: PreparedView[];
  writable: boolean;
  who: string | null;
}) {
  const [said, setSaid] = useState<Record<string, { ok: boolean; text: string }>>({});
  if (!views.length) return null;
  const answer = async (id: string, go: () => Promise<unknown>, text: string) => {
    try {
      await go();
      setSaid((s) => ({ ...s, [id]: { ok: true, text } }));
    } catch (e) {
      setSaid((s) => ({ ...s, [id]: { ok: false, text: e instanceof Error ? e.message : String(e) } }));
    }
  };
  return (
    <div className="small">
      <p>
        <strong>Prepared, not required</strong>
      </p>
      {views.map((view) => {
        const id = view.id ?? "";
        const gate = view.subject.gate ?? "";
        const isGate = view.kind === "gate-result";
        const passed = view.fields?.passed === true;
        const label = isGate
          ? `Record ${gate} ${passed ? "passed" : "failed"} on ${(view.subject.commit ?? "").slice(0, 7)}`
          : `Promote ${view.subject.release ?? view.release ?? ""} to ${view.subject.stage ?? ""}`;
        return (
          <div key={id}>
            {isGate ? (
              <p className="muted">
                {(view.required_by ?? []).length ? `asked for by ${(view.required_by ?? []).join(", ")}` : `no stage asks for ${gate}`}
              </p>
            ) : null}
            <PreparedPanel
              project={project}
              view={view}
              writable={writable && who != null}
              busy={false}
              label={label}
              onConfirm={(edits) => who && answer(id, () => api.confirmPrepared(project, view, who, edits), `${label}: done`)}
              onReject={(reason) =>
                who && answer(id, () => api.rejectPrepared(project, id, who, reason), "rejected what Claude prepared")
              }
            />
            {said[id] ? <span className={`chip ${said[id].ok ? "done" : "warn"}`}>{said[id].text}</span> : null}
          </div>
        );
      })}
    </div>
  );
}

/**
 * Record what your CI said about the commit a promotion needs (FTR-002): which gate, passed or
 * failed, a link to the run and a note. kanbanr asks no server; a person records the verdict. On a
 * read-only monitor it shows the command instead.
 */
function RecordResult({
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

/**
 * A release's notes as they are on its branch (FEAT-216), collapsed until opened: read from the
 * daemon's repository, rendered as markdown, and — on a write-enabled monitor, before the cut —
 * edited and saved onto `release/<V>`. Elsewhere it names the CLI command.
 */
function ReleaseNotesSection({
  project,
  release,
  notesPath,
  tick,
  writable,
  who,
}: {
  project: string;
  release: Release;
  notesPath: string;
  tick: number;
  writable: boolean;
  who: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
  const [again, setAgain] = useState(0);
  // Read only once opened: a daemon started elsewhere has no repository to read them from.
  const notes = useAsync(
    () => (open ? api.getReleaseNotes(project, release.version) : Promise.resolve(null)),
    [project, release.version, open, tick, again],
  );
  const section = notesSection(release, notesPath, notes.data ?? null, writable);
  const save = async () => {
    if (draft == null || !who) return;
    setSaving(true);
    try {
      const done = await api.saveReleaseNotes(project, release.version, draft, who);
      const saved = `saved: ${done.notes.branch} @ ${done.sha.slice(0, 7)}`;
      setSaid({ ok: true, text: done.main_moved ? `${saved}. ${done.main_moved}` : saved });
      setDraft(null);
      setAgain((n) => n + 1);
    } catch (e) {
      setSaid({ ok: false, text: e instanceof Error ? e.message : String(e) });
    } finally {
      setSaving(false);
    }
  };
  return (
    <details className="release-notes" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>
        <strong>Release notes</strong> <span className="muted small">{section.heading}</span>
      </summary>
      {!open ? null : notes.error ? (
        <ErrorBox error={notes.error} />
      ) : !notes.data ? (
        <Loading />
      ) : (
        <div className="small">
          <p className="muted">
            <code>{notes.data.path}</code>
            {notes.data.why_not ? ` — ${notes.data.why_not}` : ""}
          </p>
          {draft != null ? (
            <>
              <textarea
                className="reason notes-editor"
                rows={18}
                autoFocus
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                aria-label={`${release.version} release notes`}
              />
              <div className="review-actions">
                <button className="btn btn-primary" onClick={save} disabled={saving}>
                  {saving ? "saving…" : `Save to ${notes.data.branch}`}
                </button>
                <button className="btn" onClick={() => setDraft(null)} disabled={saving}>
                  Cancel
                </button>
              </div>
            </>
          ) : (
            <>
              <Markdown source={notes.data.text} />
              {section.unmentioned ? (
                <p className="muted release-unmentioned">
                  <Inline text={section.unmentioned} />
                </p>
              ) : null}
              {section.changelogHeading ? (
                <div className="release-changelog">
                  <p className="muted">{section.changelogHeading}</p>
                  {section.changelogText ? <Markdown source={section.changelogText} /> : <p>{section.changelogEmpty}</p>}
                </div>
              ) : null}
              <div className="review-actions">
                {section.edit ? (
                  <button
                    className="btn"
                    onClick={() => {
                      setSaid(null);
                      setDraft(notes.data!.text);
                    }}
                  >
                    Edit notes
                  </button>
                ) : (
                  <code>{section.command}</code>
                )}
              </div>
            </>
          )}
          {said ? (
            <p>
              <span className={`chip ${said.ok ? "done" : "warn"}`}>
                <Inline text={said.text} />
              </span>
            </p>
          ) : null}
        </div>
      )}
    </details>
  );
}

/** Exactly what the cut would tag, ship and carry over, from the plan (FEAT-185, FEAT-212). */
function CutPreviewList({
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
function Inline({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>))}
    </>
  );
}
