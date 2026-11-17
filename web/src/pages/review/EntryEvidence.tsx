/**
 * An item row's evidence on the Review page (FTR-038): the body of what was the item's brief. What
 * was added or changed since the last decision leads (FTR-018); then the definition, the spec a
 * sign-off pins (FTR-023), the diagram check (FEAT-200), the texts left to check and those Claude
 * checked (DEF-039, DEF-050), the overrides a ratification closes, and what Claude prepared
 * (FTR-034). Read for this row alone (`urls.reviewRow`) when it opens.
 *
 * The row's own buttons stay here: Approve or Ratify (held, with its sentence, while a diagram
 * does not parse), each sign-off, and the which-release choice. The group's header takes the same
 * decision for several rows at once.
 */
import { useState } from "react";
import { Link } from "react-router-dom";
import Markdown from "../../components/Markdown";
import { PreparedPanel } from "../../components/PreparedPanel";
import { approveBlocker, blockedSentence, type DiagramProblem } from "../../diagramCheck";
import { outcomeLine, type Outcome } from "../../manualTests";
import { editedWhileFlagged, openFor } from "../../prepared";
import { catalogRowHref, entryRowsLine } from "../../systemBaselineView";
import { docsFoldedOffer } from "../../textsToCheck";
import { zachmanColumns } from "../../types";
import type { DriftFlag, PendingReview, PreparedView, SettledText, SpecBrief } from "../../types";
import { useDiagramCheck } from "./ReviewRow";

/**
 * Where the form was, once its call returned (DEF-090): the outcome, a rejection included, and for
 * a failed call its error with "Try again", which brings the form back.
 */
export function OutcomeChip({ outcome, onRetry }: { outcome: Outcome; onRetry: () => void }) {
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

export function EntryEvidence({
  project,
  item,
  tick,
  writable,
  busy,
  outcome,
  onRetry,
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

      {entryRowsLine(item.system_baseline_rows) ? (
        // What this design changes in the system baseline the person accepted (FTR-006).
        <p className="small review-baseline-rows">
          {entryRowsLine(item.system_baseline_rows)}
          {(item.system_baseline_rows ?? []).some((r) => r.href) ? (
            <>
              {" · "}
              {(item.system_baseline_rows ?? [])
                .filter((r) => r.href)
                .map((r) => (
                  <Link key={r.row} to={r.href ?? ""}>
                    {r.row}
                  </Link>
                ))}
            </>
          ) : null}
        </p>
      ) : null}
      {(item.views_changed ?? []).length > 0 ? (
        <div className="provenance">
          <span className="muted">Views changed</span>
          {(item.views_changed ?? []).map((v) => (
            <Link key={v} className="chip" to={catalogRowHref(project, v)}>
              {v}
            </Link>
          ))}
        </div>
      ) : null}
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
