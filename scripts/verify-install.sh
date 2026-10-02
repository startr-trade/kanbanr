#!/bin/sh
# Check an installed kanbanr is the release it claims to be, and that it works (FEAT-149).
#
#   GITHUB_REF_NAME=v0.1.2 GITHUB_SHA=<commit> sh scripts/verify-install.sh
#
# Run by release.yml after each installer has installed the just-published release — in Linux
# containers and on macOS — and by `make ci` against the latest published release. POSIX sh: a
# container job runs it under dash (FEAT-143). scripts/verify-install.ps1 is the Windows twin.
set -eu
: "${GITHUB_REF_NAME:?the release tag, e.g. v0.1.2}"
: "${GITHUB_SHA:?the commit the tag points at}"

# It installed, and it names both the version and the commit — the same SHA the release page
# shows, so an installed binary can be matched against it.
v="$(kanbanr --version)"
echo "$v"
echo "$v" | grep -q "${GITHUB_REF_NAME#v}" \
  || { echo "::error::--version does not name the released version: $v" >&2; exit 1; }
sha="$(printf '%s' "$GITHUB_SHA" | cut -c1-12)"
echo "$v" | grep -q "$sha" \
  || { echo "::error::--version does not name the released commit: $v" >&2; exit 1; }

# The monitor is served with no --ui-dir and no build step. --data-dir given, so init does not
# ask; --no-hooks because there is no Claude Code here. Run from a fresh folder of its own: the
# release starts this script in the repository checkout, whose .kanbanr marker makes init refuse
# (FEAT-155).
work="$(mktemp -d)"
cd "$work"
board="$work/board"
port="${KANBANR_VERIFY_PORT:-8080}"
kanbanr init ci-check --data-dir "$board" --no-hooks --author "CI" --email "ci@kanbanr.local"
kanbanr serve --data-dir "$board" --bind "127.0.0.1:$port" &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  curl -fsS -o /dev/null "http://127.0.0.1:$port/healthz" 2>/dev/null && break
  sleep 1
done
body="$(curl -fsS "http://127.0.0.1:$port/")"
echo "$body" | grep -q "/assets/" \
  || { echo "::error::the installed binary served no monitor — the embed did not ship" >&2; exit 1; }
asset="$(echo "$body" | sed -n 's|.*src="/assets/\([^"]*\)".*|\1|p' | head -n1)"
[ -n "$asset" ] || { echo "::error::index.html names no bundle" >&2; exit 1; }
curl -fsS -o /dev/null "http://127.0.0.1:$port/assets/${asset}" \
  || { echo "::error::the bundle the index names is not served" >&2; exit 1; }
echo "installed, names ${GITHUB_REF_NAME} at ${sha}, monitor served, bundle intact."
