#!/bin/sh
# Write the two CycloneDX SBOMs for a release (MNT-004):
#   <outdir>/kanbanr-<tag>.sbom.cargo.cdx.json   the Rust dependencies, from api/Cargo.lock
#   <outdir>/kanbanr-<tag>.sbom.npm.cdx.json     the monitor's production npm dependencies,
#                                                from web/package-lock.json
# Usage: scripts/make-sbom.sh <tag> <outdir>
# Needs cargo-cyclonedx (cargo install cargo-cyclonedx --locked) and npm 10+. Each file is
# written to a temporary name in <outdir> and renamed, so a failed run leaves no partial file.
# Failure of any step stops it with the step named on stderr.
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: scripts/make-sbom.sh <tag> <outdir>" >&2
  exit 2
fi
tag="$1"
out="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"

step() { echo "make-sbom: $*" >&2; }
fail() { echo "make-sbom: FAILED: $*" >&2; exit 1; }

mkdir -p "$out"
tmp="$(mktemp -d "$out/.sbom.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

step "checking Cargo.lock is current"
cargo metadata --locked --format-version 1 --manifest-path "$root/api/Cargo.toml" >/dev/null \
  || fail "cargo metadata --locked (is api/Cargo.lock stale?)"

step "the Rust SBOM"
cargo cyclonedx --manifest-path "$root/api/crates/kanbanr-cli/Cargo.toml" \
  --format json --spec-version 1.5 --target all \
  || fail "cargo cyclonedx"
found="$(find "$root/api/crates/kanbanr-cli" -maxdepth 1 -name '*.cdx.json' | wc -l | tr -d ' ')"
[ "$found" = 1 ] || fail "cargo cyclonedx wrote $found *.cdx.json files beside the crate, expected exactly one"
mv "$root/api/crates/kanbanr-cli/"*.cdx.json "$tmp/cargo.json"

step "the npm SBOM"
(cd "$root/web" && npm sbom --sbom-format cyclonedx --sbom-type application --omit dev --package-lock-only) \
  > "$tmp/npm.json" || fail "npm sbom"

step "checking both are CycloneDX with components"
python3 "$root/scripts/check_supply_chain.py" --sbom "$tmp/cargo.json" "$tmp/npm.json" \
  || fail "an SBOM is not valid CycloneDX"

mv "$tmp/cargo.json" "$out/kanbanr-$tag.sbom.cargo.cdx.json"
mv "$tmp/npm.json" "$out/kanbanr-$tag.sbom.npm.cdx.json"
step "wrote $out/kanbanr-$tag.sbom.cargo.cdx.json and $out/kanbanr-$tag.sbom.npm.cdx.json"
