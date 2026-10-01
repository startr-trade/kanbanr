#!/bin/sh
# The image's kanbanr names the commit and date it was built from (FEAT-144).
#
#   scripts/check-image-stamp.sh IMAGE [COMMIT]     # COMMIT defaults to HEAD
#
# The release archives are checked for this; the image is the same binary built a second way, in a
# context with no .git, so it is checked too — before it is pushed. Run by release.yml and make ci.
set -eu
image="$1"
want="$(git rev-parse --short=12 "${2:-HEAD}")"
date="$(git log -1 --format=%cd --date=short "${2:-HEAD}")"
v="$(docker run --rm --entrypoint kanbanr "$image" --version)"
echo "$v"
case "$v" in
  *unknown*) echo "::error::the image's kanbanr cannot say which build it is: $v" >&2; exit 1 ;;
esac
echo "$v" | grep -q "$want" \
  || { echo "::error::the image's kanbanr does not name commit $want: $v" >&2; exit 1; }
echo "$v" | grep -q "built $date" \
  || { echo "::error::the image's kanbanr does not name the commit's date $date: $v" >&2; exit 1; }
