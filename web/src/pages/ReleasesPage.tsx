import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { isReadOnly } from "../runtimeConfig";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import { releaseAction } from "../releaseAction";
import { notesSection } from "../releaseNotes";
import Markdown from "../components/Markdown";
import type { Release, TagResult } from "../types";

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
  // Promote (FEAT-185): which release is being promoted, and what came of it.
  const [busy, setBusy] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Record<string, string>>({});

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
    return at + 1 < stages.length ? stages[at + 1] : null;
  };
  // The same core promotion as the CLI, through the daemon's promote route (FEAT-183), carrying
  // the release tip the plan showed (FEAT-212).
  const promote = async (r: Release, to: string, expect: string, note = "") => {
    if (!who) return;
    setBusy(r.version);
    try {
      const done = (await api.promoteRelease(project, r.version, who, to, expect, note)) as { next?: string };
      setOutcome((o) => ({ ...o, [r.version]: `promoted to ${to}${done?.next ? ` — ${done.next}` : ""}` }));
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
                            .map((c) => `${c.stage} · ${c.by} · ${c.at.slice(0, 10)}${c.reason ? ` — ${c.reason}` : ""}`)
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
                  />
                ) : null}
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
}: {
  project: string;
  release: Release;
  tick: number;
  writable: boolean;
  gates: string[];
  who: string | null;
  busy: string | null;
  onPromote: (r: Release, to: string, expect: string, note?: string) => void;
  outcome?: string;
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
  const plan = useAsync(() => api.getReleasePlan(project, release.version), [project, release.version, tick, again]);
  if (plan.error) return <ErrorBox error={plan.error} />;
  if (!plan.data) return <Loading />;
  const action = releaseAction(plan.data, writable);
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
      <div className="review-actions">
        {action.button ? (
          <button
            className="btn btn-primary"
            onClick={() =>
              action.confirm ? setAsking(true) : onPromote(release, plan.data!.to, plan.data!.release_tip)
            }
            disabled={!action.enabled || busy != null}
            title={action.enabled ? action.does.join("; ") : "not ready: see what is missing"}
          >
            {busy === release.version ? "promoting…" : action.label}
          </button>
        ) : (
          <code>{action.command}</code>
        )}
        {outcome && !hideOutcome ? (
          <span className={`chip ${outcome.startsWith("promoted") ? "done" : "warn"}`}>
            <Inline text={outcome} />
          </span>
        ) : null}
      </div>
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
                onPromote(release, plan.data!.to, plan.data!.release_tip, note.trim());
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
}: {
  project: string;
  release: Release;
  tag: TagResult;
  gates: string[];
  writable: boolean;
  who: string | null;
  secondary: ("runAgain" | "startPatch")[];
  onChanged: () => void;
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
}: {
  project: string;
  commit: string;
  gates: string[];
  writable: boolean;
  who: string | null;
  command: string;
  onRecorded: () => void;
}) {
  const [gate, setGate] = useState(gates[0] ?? "ci");
  const [url, setUrl] = useState("");
  const [note, setNote] = useState("");
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
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
