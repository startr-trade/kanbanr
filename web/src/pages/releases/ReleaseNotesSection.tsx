import { useState } from "react";
import { api } from "../../api";
import { useAsync } from "../../live";
import { ErrorBox, Loading } from "../../components/bits";
import { notesSection } from "../../releaseNotes";
import Markdown from "../../components/Markdown";
import type { Release } from "../../types";
import { Inline } from "./ReleaseActions";

/**
 * A release's notes as they are on its branch (FEAT-216), collapsed until opened: read from the
 * daemon's repository, rendered as markdown, and — on a write-enabled monitor, before the cut —
 * edited and saved onto `release/<V>`. Elsewhere it names the CLI command.
 */
export function ReleaseNotesSection({
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
