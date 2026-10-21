/**
 * What a release's **Release notes** section shows (FEAT-216), decided from the release, the
 * project's notes path and the notes the daemon read on `release/<V>`: whether there is a section
 * at all, its heading, and whether it offers Edit. A pure function, so the check script can hold it
 * to each case.
 */
import type { Release, ReleaseNotes } from "./types";

export interface NotesSection {
  /** A section is offered: the release has a branch and the project configures a notes path. */
  show: boolean;
  /** "draft on release/v0.3.0 @ 8f995e1", or "as shipped on …" after the cut. */
  heading: string;
  /** Edit is offered: a write-enabled monitor, and the release not yet cut. */
  edit: boolean;
  /** The CLI command that does the same thing. */
  command: string;
}

export function notesSection(
  release: Release,
  notesPath: string | undefined,
  notes: ReleaseNotes | null,
  writable: boolean,
): NotesSection {
  const shipped = release.state === "shipped";
  const show = !!release.branch && !!notesPath;
  const where = notes ? `${notes.branch} @ ${notes.at}` : (release.branch ?? "");
  const heading = show ? `${shipped ? "as shipped on" : "draft on"} ${where}` : "";
  const edit = show && writable && !shipped && (notes?.editable ?? false);
  const command = `kanbanr release notes ${release.version}${shipped ? "" : " --edit"}`;
  return { show, heading, edit, command };
}
