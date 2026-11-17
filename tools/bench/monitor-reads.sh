#!/usr/bin/env bash
# Time the monitor's reads on a copy of a board, against their budgets (DEF-081 R-11).
#
#   tools/bench/monitor-reads.sh <board-copy> <port>
#
# <board-copy> is a data folder (the one holding projects/), copied somewhere temporary first:
#
#   cp -a <your board's data folder> /tmp/board-copy
#   tools/bench/monitor-reads.sh /tmp/board-copy 8199
#
# It starts `kanbanr serve` (read-only) on the copy, reads each route once to warm it, then takes
# 5 samples with curl, gzip accepted as the monitor's browser does, and prints a TSV of
# `route  p95_ms  bytes_gzip  budget_ms`. It exits non-zero when any read is over its budget or
# does not answer 200. The p95 of 5 samples is the slowest one (nearest rank).
#
# Budgets (DEF-081): the summary reads (the compact index and the configuration) under 50 ms, a
# slice under 150 ms, both p95 and warm.
#
# Environment: KANBANR (the binary, default `kanbanr`), PROJECT (default: the copy's first
# project), CODE (an item to read, default: the project's first item file), STATUS (the status
# slice, default `In Progress`).
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <board-copy> <port>" >&2
  exit 2
fi
COPY="$(cd "$1" && pwd)"
PORT="$2"
KANBANR="${KANBANR:-kanbanr}"
SAMPLES=5

if [ ! -d "$COPY/projects" ]; then
  echo "$COPY holds no projects/ folder: give the data folder of a board copy" >&2
  exit 2
fi
PROJECT="${PROJECT:-$(find "$COPY/projects" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort | head -n 1)}"
if [ -z "$PROJECT" ] || [ ! -d "$COPY/projects/$PROJECT" ]; then
  echo "no project ${PROJECT:+$PROJECT }in $COPY/projects" >&2
  exit 2
fi
CODE="${CODE:-$(find "$COPY/projects/$PROJECT/features" -mindepth 2 -maxdepth 2 -name '*.yaml' -printf '%f\n' | sort | head -n 1 | sed 's/\.yaml$//')}"
STATUS="${STATUS:-In Progress}"
STATUS_Q="$(printf '%s' "$STATUS" | sed 's/ /%20/g')"

BASE="http://127.0.0.1:$PORT/api/projects/$PROJECT"

# The fixed route list: path below the project, and its budget in milliseconds.
ROUTES=(
  "/index 50"
  "/config 50"
  "/features?status=$STATUS_Q 150"
  "/features/$CODE 150"
  "/readiness 150"
  "/charter 150"
  "/sprints 150"
)

LOG="$(mktemp "${TMPDIR:-/tmp}/kanbanr-bench-serve.XXXXXX")"
# Started from inside the copy, so nothing walks up to a real board's .kanbanr marker.
(cd "$COPY" && exec "$KANBANR" serve --data-dir "$COPY" --project "$PROJECT" \
  --bind "127.0.0.1:$PORT") >"$LOG" 2>&1 &
SERVE=$!
trap 'kill "$SERVE" 2>/dev/null || true; wait "$SERVE" 2>/dev/null || true; rm -f "$LOG"' EXIT

for _ in $(seq 1 200); do
  if curl -fs "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$SERVE" 2>/dev/null; then
    echo "kanbanr serve exited:" >&2
    cat "$LOG" >&2
    exit 1
  fi
  sleep 0.05
done
curl -fs "http://127.0.0.1:$PORT/healthz" >/dev/null || {
  echo "kanbanr serve did not answer on port $PORT" >&2
  cat "$LOG" >&2
  exit 1
}

# One read: "<http_code> <seconds> <bytes>", the body as sent (gzipped), not decoded.
read_once() {
  curl -s -o /dev/null -H 'Accept-Encoding: gzip' \
    -w '%{http_code} %{time_total} %{size_download}\n' "$BASE$1"
}

over=0
printf 'route\tp95_ms\tbytes_gzip\tbudget_ms\n'
for entry in "${ROUTES[@]}"; do
  route="${entry% *}"
  budget="${entry##* }"
  read -r code _ _ <<<"$(read_once "$route")" # warm
  if [ "$code" != "200" ]; then
    printf '%s\t-\t-\t%s\tHTTP %s\n' "$route" "$budget" "$code"
    over=1
    continue
  fi
  samples=()
  bytes=0
  for _ in $(seq 1 "$SAMPLES"); do
    read -r code secs size <<<"$(read_once "$route")"
    if [ "$code" != "200" ]; then
      bytes="HTTP $code"
      break
    fi
    samples+=("$secs")
    bytes="$size"
  done
  if [ "$code" != "200" ]; then
    printf '%s\t-\t-\t%s\t%s\n' "$route" "$budget" "$bytes"
    over=1
    continue
  fi
  # p95 by nearest rank over the samples, in milliseconds.
  p95="$(printf '%s\n' "${samples[@]}" | sort -g | awk -v n="$SAMPLES" '
    { v[NR] = $1 }
    END { r = int(0.95 * n); if (r < 0.95 * n) r++; printf "%.1f", v[r] * 1000 }')"
  verdict=""
  if awk -v p="$p95" -v b="$budget" 'BEGIN { exit !(p >= b) }'; then
    verdict="over budget"
    over=1
  fi
  printf '%s\t%s\t%s\t%s%s\n' "$route" "$p95" "$bytes" "$budget" "${verdict:+	$verdict}"
done

exit "$over"
