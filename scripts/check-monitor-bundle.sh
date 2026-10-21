#!/usr/bin/env bash
# release::ui_bundle_attached (FEAT-178 R-1): the monitor bundle a release attaches is one a static
# host can serve.
#
#   scripts/check-monitor-bundle.sh <kanbanr-monitor-vX.Y.Z.tar.gz>
#
# It must contain:
#   - index.html, whose every src/href is relative — no leading `/`, no scheme — and which loads its
#     bundle from ./assets/, every one of which is in the archive (an absolute path would only work
#     at a host's root; a missing asset is a blank page);
#   - config.json, which parses and names apiBase (without it the monitor cannot find the API), and
#     is the standalone configuration: read-only, hash routing;
#   - the assets themselves.
# release.yml runs it before staging the archive, and `make ci` runs it on a locally packed one.
set -euo pipefail

archive="${1:?usage: check-monitor-bundle.sh <archive.tar.gz>}"
[ -f "$archive" ] || { echo "no archive at $archive" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$archive" -C "$work"

fail=0
problem() { echo "release::ui_bundle_attached — $*" >&2; fail=1; }

index="$work/index.html"
if [ ! -f "$index" ]; then
  problem "no index.html at the archive's root"
else
  refs="$(grep -oE '(src|href)="[^"]*"' "$index" | sed -E 's/^(src|href)="(.*)"$/\2/' || true)"
  assets=0
  while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    case "$ref" in
      /*|*://*) problem "index.html references $ref, which is not relative" ;;
      ./assets/*)
        assets=$((assets + 1))
        [ -f "$work/${ref#./}" ] || problem "index.html loads $ref, which is not in the archive" ;;
    esac
  done <<<"$refs"
  [ "$assets" -gt 0 ] || problem "index.html loads nothing from ./assets/"
  if grep -qi '<base' "$index"; then
    problem "index.html carries a <base>, so it would only work at one path"
  fi
fi

if [ ! -f "$work/config.json" ]; then
  problem "no config.json at the archive's root"
elif ! python3 -c 'import json,sys; c=json.load(open(sys.argv[1])); sys.exit(0 if isinstance(c,dict) and isinstance(c.get("apiBase"),str) and c["apiBase"] else 1)' "$work/config.json"; then
  problem "config.json does not parse, or names no apiBase: $(head -c 200 "$work/config.json")"
# The bundle ships read-only with hash routing (FEAT-178 Q1), not the embedded defaults that
# web/dist carries: packing the dist as it is would hand reviewers decision buttons.
elif ! python3 -c 'import json,sys; c=json.load(open(sys.argv[1])); sys.exit(0 if c.get("readOnly") is True and c.get("routing")=="hash" else 1)' "$work/config.json"; then
  problem "config.json is not the standalone configuration (readOnly, hash routing): $(head -c 200 "$work/config.json")"
fi

if [ -z "$(find "$work/assets" -type f 2>/dev/null | head -n 1)" ]; then
  problem "no assets/ in the archive"
fi

[ "$fail" -eq 0 ] || exit 1
echo "release::ui_bundle_attached — $archive: index.html loads its relative ./assets, config.json names apiBase ($(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["apiBase"])' "$work/config.json"))."
