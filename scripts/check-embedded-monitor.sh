#!/bin/sh
# Is the web monitor in this binary, and stored compressed? (FEAT-172, for FEAT-084/R-5)
#
#   scripts/check-embedded-monitor.sh <binary> <web/dist>
#
# Checked by looking, not by size. The step used to bracket the binary between 12 and 20 MB, and
# code growth made both bounds wrong: a binary with no monitor is ~19 MB, and FEAT-169's code alone
# crossed 20 MB with the assets compressed. Two direct questions instead:
#   - embedded: the asset table names every file under web/dist (build.rs stores each path);
#   - compressed: a stretch of the largest script's plain text is nowhere in the binary.
# POSIX sh: release.yml runs it under bash on Linux, macOS and Windows, and make ci runs it too.
set -eu
LC_ALL=C
export LC_ALL

bin=$1
dist=$2
[ -f "$bin" ] || bin="$bin.exe"
[ -f "$bin" ] || { echo "::error::no binary at $1" >&2; exit 1; }
[ -f "$dist/index.html" ] || {
  echo "::error::$dist has no index.html — build the monitor before checking for it" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Embedded: every published path, found in one pass over the binary.
(cd "$dist" && find . -type f | sed 's|^\./||' | sort) > "$work/want"
grep -a -o -F -f "$work/want" "$bin" | sort -u > "$work/found" || true
missing=$(comm -23 "$work/want" "$work/found")
if [ -n "$missing" ]; then
  echo "::error::$bin does not embed the monitor; missing:" >&2
  echo "$missing" | head -20 >&2
  exit 1
fi
echo "embedded: all $(wc -l < "$work/want" | tr -d ' ') monitor files are named in the binary"

# Compressed: 120 characters from the middle of the largest script's longest line. Minified, that
# line is long and unlike anything else; stored raw, it would be in the binary verbatim.
largest=$(find "$dist" -type f -name '*.js' -exec wc -c {} + | grep -v ' total$' | sort -n | tail -1 \
  | awk '{print $2}')
sample=$(awk '{ if (length($0) > length(best)) best = $0 } END {
  start = int(length(best) / 2) - 60; if (start < 1) start = 1; print substr(best, start, 120) }' "$largest")
if [ "${#sample}" -lt 120 ]; then
  echo "::error::$largest has no line long enough to sample" >&2
  exit 1
fi
if grep -a -q -F -e "$sample" "$bin"; then
  echo "::error::$bin carries $largest's text verbatim — the assets are not compressed (FEAT-084/R-5)" >&2
  exit 1
fi
echo "compressed: $(basename "$largest")'s text is not in the binary as plain text"
