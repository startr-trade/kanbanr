// The Needs-you badge in the top bar (FTR-047, FTR-048). The shell renders it between the project
// navigation and the build label, outside the menu, so it is always visible; FTR-048 fills it, and
// never edits App.tsx to do so. Until then it draws nothing.

/** The count of things waiting on the person, as a link to them; `null` until FTR-048. */
export function NeedsYouBadge(_p: { project: string }): JSX.Element | null {
  return null;
}
