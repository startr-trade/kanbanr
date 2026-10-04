#!/bin/sh
# Refuse to release a commit that has not passed CI (FEAT-163).
#
#   GITHUB_REPOSITORY=owner/repo GITHUB_SHA=<commit> GH_TOKEN=… sh scripts/release-ci-gate.sh
#
# Run by release.yml's tag job before anything is built. It asks GitHub for the CI workflow's run on
# the tagged commit: success proceeds, a failure or no run at all refuses, and a run still in
# progress is waited for (KANBANR_CI_WAIT_TRIES × KANBANR_CI_WAIT_SECONDS, 45 minutes by default).
# v0.1.5 was published while CI was failing on macOS for the same commit; the release built and
# verified its own artifacts but never asked whether the code had passed its tests.
#
# GH can name a stand-in for the `gh` command, which is how the waiting is tested.
set -eu
: "${GITHUB_REPOSITORY:?owner/repo}"
: "${GITHUB_SHA:?the tagged commit}"
gh_cmd="${GH:-gh}"
tries="${KANBANR_CI_WAIT_TRIES:-90}"
pause="${KANBANR_CI_WAIT_SECONDS:-30}"

i=0
while :; do
  i=$((i + 1))
  run="$("$gh_cmd" api "repos/${GITHUB_REPOSITORY}/actions/workflows/ci.yml/runs?head_sha=${GITHUB_SHA}&per_page=1" \
          -q '.workflow_runs[0] | select(.) | [.status, (.conclusion // ""), .html_url] | @tsv')"
  if [ -z "$run" ]; then
    echo "::error::CI has not run on ${GITHUB_SHA}: push the commit to main and let CI pass before tagging." >&2
    exit 1
  fi
  status="$(printf '%s' "$run" | cut -f1)"
  conclusion="$(printf '%s' "$run" | cut -f2)"
  url="$(printf '%s' "$run" | cut -f3)"
  if [ "$status" = completed ]; then
    if [ "$conclusion" = success ]; then
      echo "CI passed on ${GITHUB_SHA}: ${url}"
      exit 0
    fi
    echo "::error::CI ${conclusion} on ${GITHUB_SHA} — nothing is released from a commit that failed its tests: ${url}" >&2
    exit 1
  fi
  if [ "$i" -ge "$tries" ]; then
    echo "::error::CI is still ${status} on ${GITHUB_SHA} after waiting; re-run this release when it has passed: ${url}" >&2
    exit 1
  fi
  echo "CI is ${status} on ${GITHUB_SHA}; waiting (${i}/${tries})…"
  sleep "$pause"
done
