import type { Meta } from "./api";

/** The kanbanr this monitor runs, as a short label: `v0.6.0 · 1138833`. Empty when the daemon
 * did not say (an older server, or the call failed). (FTR-036) */
export function buildLabel(meta: Meta | null): string {
  const version = meta?.version?.trim();
  if (!version) return "";
  const commit = meta?.commit?.trim();
  return commit && commit !== "unknown" ? `v${version} · ${commit}` : `v${version}`;
}

/** The warning shown when the board was last written by a newer kanbanr than this monitor runs,
 * naming both; null otherwise. (FTR-036) */
export function behindWarning(meta: Meta | null): string | null {
  const bw = meta?.board_writer;
  if (!meta?.writer_newer || !bw) return null;
  return (
    `This monitor runs kanbanr ${meta.version ?? "unknown"} (${meta.commit ?? "unknown"}); ` +
    `the board was last written by ${bw.version} (${bw.commit}). ` +
    `Decisions taken here follow older rules than the work was built for. ` +
    `Restart the monitor from the newer kanbanr.`
  );
}
