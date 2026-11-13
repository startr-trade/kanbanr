// The docs-folded sign-off under the texts to check (DEF-039). Claude settles each text; the
// person is offered the sign-off only when none is left. Pure, so the page and
// web::texts_to_check (scripts/check-texts-to-check.mjs) use the same code.

import type { DriftFlag, SettledText } from "./types.ts";

/** What the Review page does about docs-folded for one entry. `offer` is false while `left` is
 * non-empty; a skipped check (`to_check` absent) offers it, as FTR-013 R-9 keeps. */
export function docsFoldedOffer(item: { to_check?: DriftFlag[]; settled?: SettledText[] }): {
  offer: boolean;
  left: DriftFlag[];
  settled: SettledText[];
} {
  const left = item.to_check ?? [];
  return { offer: left.length === 0, left, settled: item.settled ?? [] };
}
