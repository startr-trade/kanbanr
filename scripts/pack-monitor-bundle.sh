#!/usr/bin/env bash
# Pack the built monitor as the standalone bundle a release attaches (FEAT-178).
#
#   scripts/pack-monitor-bundle.sh <web/dist> <kanbanr-monitor-vX.Y.Z.tar.gz>
#
# The archive holds web/dist as it is (relative asset paths, no <base>), except that config.json is
# the standalone configuration — web/config.standalone.json: the API at localhost:8080, hash
# routing for a plain static host, and read-only. A host unpacks it, edits config.json and serves
# the folder. One script, so `make monitor-bundle` and release.yml pack it the same way.
set -euo pipefail

dist="${1:?usage: pack-monitor-bundle.sh <web/dist> <out.tar.gz>}"
out="${2:?usage: pack-monitor-bundle.sh <web/dist> <out.tar.gz>}"
here="$(cd "$(dirname "$0")/.." && pwd)"
config="$here/web/config.standalone.json"

[ -f "$dist/index.html" ] || { echo "no built monitor at $dist (cd web && npm run build)" >&2; exit 1; }
[ -f "$config" ] || { echo "missing $config" >&2; exit 1; }

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$dist"/. "$stage"/
cp "$config" "$stage/config.json"

case "$out" in /*) abs="$out" ;; *) abs="$PWD/$out" ;; esac
# Entries at the archive's root (index.html, config.json, assets/…), like the binary archives.
(cd "$stage" && tar -czf "$abs" -- *)
echo "packed $out"
