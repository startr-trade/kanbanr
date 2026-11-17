/**
 * The still-working line's model (DEF-106): when it appears and how its elapsed time reads. Pure, so
 * the checks import it directly; `States.tsx` draws it.
 */

/** How long a read runs before the line says how long it has been. */
export const WORKING_AFTER_MS = 1000;

/** `"N s"` under two minutes, else `"M min N s"`. */
export function elapsedText(since: number, now: number): string {
  const secs = Math.max(0, Math.floor((now - since) / 1000));
  return secs < 120 ? `${secs} s` : `${Math.floor(secs / 60)} min ${secs % 60} s`;
}

/** The seconds a screen reader is told: at 10, 30 and 60 s, then each minute; none before 10. */
export function announcedSeconds(secs: number): number {
  if (secs < 10) return 0;
  if (secs < 30) return 10;
  if (secs < 60) return 30;
  return Math.floor(secs / 60) * 60;
}
