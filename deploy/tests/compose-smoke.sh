#!/usr/bin/env bash
# Compose mode (b) serves a local board read-only and reports healthy
# (FTR-030 R-10).
#
#   deploy/tests/compose-smoke.sh
#
# Makes a board in a temp dir, starts deploy/compose in mode (b)
# (compose.yaml + compose.local.yaml) with the kanbanr:e2e image on a free
# port, as the caller's uid and gid, and checks that the monitor turns healthy
# within 60 s, serves the board's item, and cannot write to /boards. The
# settings come from a temp env file, never a person's deploy/compose/.env.
# Prints `ok: <name>` or `FAIL: <name>`; `down -v` runs on exit.
# Needs Docker with compose, git, curl and python3.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=deploy/tests/lib.sh
. "$here/lib.sh"

name="compose: mode (b) serves a local board read-only and reports healthy"
site=$LIB_REPO/deploy/compose
project=kanbanr-smoke-$$
tmp=$(mktemp -d)
env_file=$tmp/smoke.env

compose() {
    docker compose -p "$project" --env-file "$env_file" -f "$site/compose.yaml" -f "$site/compose.local.yaml" "$@"
}
cleanup() {
    compose down -v >/dev/null 2>&1 || true
    rm -rf "$tmp"
}
trap cleanup EXIT

ensure_image
make_board "$tmp/board" e2e
port=$(free_port)
cat >"$env_file" <<EOF
KANBANR_IMAGE=$E2E_IMAGE
KANBANR_BOARDS_PATH=$tmp/board
KANBANR_LOCAL_BOARDS=
KANBANR_PORT=$port
KANBANR_PUBLISH_ADDRESS=127.0.0.1
KANBANR_ALLOWED_HOSTS=
KANBANR_UID=$(id -u)
KANBANR_GID=$(id -g)
EOF

problems=()
if ! compose up -d >"$tmp/up.log" 2>&1; then
    problems+=("docker compose up failed:" "$(cat "$tmp/up.log")")
else
    container=$(compose ps -q monitor)
    healthy() { [ "$(docker inspect -f '{{.State.Health.Status}}' "$container")" = healthy ]; }
    if ! wait_for 60 healthy; then
        problems+=("the monitor is $(docker inspect -f '{{.State.Health.Status}}' "$container" 2>&1) after 60 s, not healthy")
        problems+=("$(compose logs --tail=20 monitor 2>&1)")
    fi
    if ! curl -sS -H 'Host: localhost' "http://127.0.0.1:$port/api/projects/e2e" | grep -F '"First item"' >/dev/null; then
        problems+=("GET /api/projects/e2e does not list \"First item\"")
    fi
    if compose exec -T monitor bash -c 'touch /boards/x' >/dev/null 2>&1; then
        problems+=("touch /boards/x succeeded in the monitor")
    fi
fi

if [ ${#problems[@]} -eq 0 ]; then
    echo "ok: $name"
else
    echo "FAIL: $name"
    for why in "${problems[@]}"; do echo "$why" | sed 's/^/  /'; done
    exit 1
fi
