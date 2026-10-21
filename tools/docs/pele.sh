#!/usr/bin/env bash
# Copy the pinned Pele's browser build into the book (FEAT-203).
#
# The book draws its diagrams with Pele in the reader's browser. Its build is committed as
# docs/pele.global.js, so building the book needs no npm — and so it must stay the exact build of
# the version web/package.json pins. Run this after changing that pin; `--check` changes nothing
# and fails when the committed copy differs (check:docs runs the same comparison).
set -euo pipefail
cd "$(dirname "$0")/../.."

src=web/node_modules/pele/dist/pele.global.js
dst=docs/pele.global.js
# Pele is MIT: its licence travels with the copy committed here.
lic_src=web/node_modules/pele/LICENSE
lic_dst=docs/pele.LICENSE
[ -f "$src" ] || { echo "no $src — run npm install in web/ first" >&2; exit 1; }
if [ "${1:-}" = "--check" ]; then
  cmp -s "$src" "$dst" && cmp -s "$lic_src" "$lic_dst" \
    || { echo "$dst or $lic_dst is not the pinned Pele's — run tools/docs/pele.sh" >&2; exit 1; }
  echo "$dst is the pinned Pele's build"
else
  cp "$src" "$dst"
  cp "$lic_src" "$lic_dst"
  echo "copied: $src -> $dst ($(head -c 60 "$dst" | tr -d '/*!' | sed 's/ |.*//;s/^ *//'))"
fi
