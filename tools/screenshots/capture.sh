#!/usr/bin/env bash
# Capture screenshots of the kanbanr web monitor through a Selenium Grid (Chromium) in Docker.
#
#   1. ensures a `kanbanr serve` monitor is reachable (starts a temporary one if not),
#   2. starts a selenium/standalone-chromium container on the host network — OR reuses one that's
#      already running, so repeated runs are fast (the grid is left UP between runs),
#   3. runs the Rust WebDriver tool (src/main.rs) to write the PNGs into docs/images/.
#
# The grid is intentionally kept running for fast iteration. Stop it when finished with:
#   tools/screenshots/capture.sh --down
#
# Usage:  tools/screenshots/capture.sh         (or:  make screenshots)
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"

KANBANR_URL="${KANBANR_URL:-http://localhost:8080}"
SELENIUM_IMAGE="${SELENIUM_IMAGE:-selenium/standalone-chromium:latest}"
CONTAINER="kanbanr-selenium"
OUT_DIR="${OUT_DIR:-$ROOT/docs/images}"

# `capture.sh --down` (or `down`/`stop`) tears the persistent grid down — the only thing that does.
case "${1:-}" in
  --down|down|stop)
    docker rm -f "$CONTAINER" >/dev/null 2>&1 && echo "stopped $CONTAINER" || echo "$CONTAINER not running"
    exit 0 ;;
esac

TEMP_SERVE_PID=""
# Only tears down a temporary monitor we started — NOT the grid (kept alive for fast iteration).
cleanup() { [[ -n "$TEMP_SERVE_PID" ]] && kill "$TEMP_SERVE_PID" 2>/dev/null || true; }
trap cleanup EXIT

# 1. Ensure the monitor is up; start a temporary one from the repo if needed.
if ! curl -fsS -o /dev/null "$KANBANR_URL/healthz" 2>/dev/null; then
  echo "kanbanr serve not reachable at $KANBANR_URL — starting a temporary one…"
  ( cd "$ROOT/api" && cargo build --release -p kanbanr-cli )
  ( cd "$ROOT/web" && npm install && npm run build )
  # Ask the binary where the board is — the `.kanbanr` marker is the authority, and a board may be
  # named for its owner rather than for the repo. Guessing `<repo>.kanbanr` broke when one was
  # renamed; the guess survives only as a fallback.
  BOARD="${KANBANR_DATA_DIR:-$("$ROOT/api/target/release/kanbanr" where 2>/dev/null)}"
  BOARD="${BOARD:-$ROOT/../$(basename "$ROOT").kanbanr}"
  KANBANR_DATA_DIR="$BOARD" \
      "$ROOT/api/target/release/kanbanr" serve \
      --bind 127.0.0.1:8080 --ui-dir "$ROOT/web/dist" &
  TEMP_SERVE_PID=$!
  for _ in $(seq 1 30); do curl -fsS -o /dev/null "$KANBANR_URL/healthz" 2>/dev/null && break; sleep 1; done
fi

# 2. Reuse the grid if it's already running; otherwise start it (and leave it up afterwards).
if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
  echo "reusing running $CONTAINER"
else
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  echo "starting $SELENIUM_IMAGE (first run pulls the image, ~1GB)…"
  docker run -d --rm --name "$CONTAINER" --network host --shm-size 2g "$SELENIUM_IMAGE" >/dev/null
  echo "waiting for the grid on :4444 …"
  for _ in $(seq 1 90); do
    if curl -fsS "http://localhost:4444/status" 2>/dev/null | grep -qE '"ready"[[:space:]]*:[[:space:]]*true'; then
      break
    fi
    sleep 1
  done
fi

# 3. Run the capture tool (builds kanbanr-screenshots on first run).
OUT_DIR="$OUT_DIR" KANBANR_URL="$KANBANR_URL" cargo run --quiet

echo "screenshots written to $OUT_DIR"
echo "(grid '$CONTAINER' left running — stop it with: tools/screenshots/capture.sh --down)"
