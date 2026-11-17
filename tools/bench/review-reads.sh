#!/usr/bin/env bash
# Time the Review page's reads on a copy of a board, with its code repository (DEF-106).
#
#   tools/bench/review-reads.sh <board-dir> <code-dir> [port]
#
# <board-dir> is a data folder (the one holding projects/), copied somewhere temporary first; the
# daemon reads it and never writes. <code-dir> is a checkout of the project's code repository: the
# daemon starts there, as the person's does, so the reads that judge the code run for real. The
# port defaults to 8098.
#
#   cp -a <your board's data folder> /tmp/board-copy
#   tools/bench/review-reads.sh /tmp/board-copy <your code checkout>
#
# It starts `kanbanr serve` (read-only) on 127.0.0.1:<port>, and times, once and cold:
#
#   /review/outline    the groups from the config alone
#   /review/summary    the groups, counted: the first read is the cold one, then 20 warm ones
#   the largest group's slice (by count in the summary)
#   /review            the whole queue
#   /decisions         where decisions are taken, and what waits
#
# and prints a table of `read  cold_ms  warm_median_ms  bytes`. It stops the daemon by the pid it
# started and never by name. Run it with a release binary: KANBANR=<path> (default `kanbanr`), and
# PROJECT=<id> (default: the copy's first project). Keep the table: it is the item's evidence.
set -euo pipefail

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <board-dir> <code-dir> [port]" >&2
  exit 2
fi
COPY="$(cd "$1" && pwd)"
CODE_DIR="$(cd "$2" && pwd)"
PORT="${3:-8098}"
KANBANR="${KANBANR:-kanbanr}"
WARM=20

if [ ! -d "$COPY/projects" ]; then
  echo "$COPY holds no projects/ folder: give the data folder of a board copy" >&2
  exit 2
fi
PROJECT="${PROJECT:-$(find "$COPY/projects" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort | head -n 1)}"
BASE="http://127.0.0.1:$PORT/api/projects/$PROJECT"

LOG="$(mktemp "${TMPDIR:-/tmp}/kanbanr-bench-review.XXXXXX")"
BODY="$(mktemp "${TMPDIR:-/tmp}/kanbanr-bench-review-body.XXXXXX")"
(cd "$CODE_DIR" && exec "$KANBANR" serve --data-dir "$COPY" --project "$PROJECT" \
  --bind "127.0.0.1:$PORT") >"$LOG" 2>&1 &
SERVE=$!
trap 'kill "$SERVE" 2>/dev/null || true; wait "$SERVE" 2>/dev/null || true; rm -f "$LOG" "$BODY"' EXIT

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
  exit 1
}

# One read: "<http_code> <milliseconds> <bytes>"; the body is kept in $BODY.
read_once() {
  curl -s -o "$BODY" -H 'Accept-Encoding: gzip' \
    -w '%{http_code} %{time_total} %{size_download}\n' "$BASE$1" |
    awk '{ printf "%s %.0f %s\n", $1, $2 * 1000, $3 }'
}

# The median of $WARM further reads, in milliseconds.
warm_median() {
  local i
  for i in $(seq 1 "$WARM"); do
    read_once "$1" | awk '{ print $2 }'
  done | sort -n | awk '{ v[NR] = $1 } END { print v[int((NR + 1) / 2)] }'
}

bad=0
row() { # <label> <path> <warm: 1 to take the warm median>
  local code ms bytes warm="-"
  read -r code ms bytes <<<"$(read_once "$2")"
  if [ "$code" != "200" ]; then
    printf '%s\t%s\t-\t-\tHTTP %s\n' "$1" "-" "$code"
    bad=1
    return
  fi
  if [ "$3" = 1 ]; then
    warm="$(warm_median "$2")"
  fi
  printf '%s\t%s\t%s\t%s\n' "$1" "$ms" "$warm" "$bytes"
}

printf 'read\tcold_ms\twarm_median_ms\tbytes_gzip\n'
row "/review/outline" "/review/outline" 0
row "/review/summary" "/review/summary" 1
# The largest group, from the summary just read (the body of the last read is the summary's).
curl -s "$BASE/review/summary" >"$BODY"
LARGEST="$(python3 -I -c 'import json,sys; g=json.load(open(sys.argv[1])); print(max(g,key=lambda x:x["count"])["key"] if g else "")' "$BODY")"
if [ -n "$LARGEST" ]; then
  row "/review/groups/$LARGEST" "/review/groups/$LARGEST" 1
fi
row "/review" "/review" 0
row "/decisions" "/decisions" 0
exit "$bad"
