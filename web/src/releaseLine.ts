/**
 * Which release an item shipped in, or will (FTR-001): the same wording as the CLI's
 * `release_line`, worked out from the item and the project's releases. Null when the project does
 * not work in releases, so a Completed item is never mistaken for a released one.
 */
import type { Feature, Release } from "./types";

export function releaseLine(
  feature: Pick<Feature, "code" | "release">,
  releases: Release[],
  usesReleases: boolean,
): string | null {
  if (!usesReleases) return null;
  const shipped = releases.find((r) => (r.shipped ?? []).includes(feature.code));
  if (shipped) return `Released in ${shipped.version}`;
  const planned = feature.release;
  if (planned && releases.some((r) => r.version === planned)) {
    return `Planned for ${planned}, not released yet`;
  }
  return "Not in a release";
}
