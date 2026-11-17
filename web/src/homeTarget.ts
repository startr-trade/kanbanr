// Where Home sends the person (FTR-048 R-12): with exactly one project the monitor opens that
// project's Overview, since a list of one is a step that says nothing; with none or several, Home
// lists them. Pure, so node checks import it, and the one place FTR-040 adds its one-board condition.

/** `/p/<id>` when there is exactly one project; otherwise `null` (Home shows the list). */
export function homeTarget(projects: { id: string }[]): string | null {
  return projects.length === 1 ? `/p/${encodeURIComponent(projects[0].id)}` : null;
}
