#!/usr/bin/env bash
# The compose site's files validate in both modes (FTR-030 R-10).
#
# Copies deploy/compose and sync.sh to a temp dir, writes a boards.conf and an
# env file per mode from the committed examples (with a temp board folder for
# mode b), runs `docker compose config` on each, and checks the merged result:
# mode (a) has clone, sync and monitor; mode (b) has the monitor only; in both
# the monitor's /boards mount is read-only. Needs docker compose and python3.
#
#   deploy/tests/compose-config.sh
#
# Prints `ok: <name>` or `FAIL: <name>` per case and exits 1 if any failed.

set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

site=$tmp/deploy/compose
mkdir -p "$tmp/deploy" "$tmp/deploy/modules/kanbanr-site/files" "$tmp/board/projects"
cp -R "$repo/deploy/compose" "$site"
# A person's own settings never take part in the check.
rm -f "$site/.env" "$site/boards.conf"
cp "$repo/deploy/modules/kanbanr-site/files/sync.sh" "$tmp/deploy/modules/kanbanr-site/files/sync.sh"
cp "$site/boards.conf.example" "$site/boards.conf"

mode_a=$tmp/a.env
mode_b=$tmp/b.env
sed -e "s|^KANBANR_BOARDS_PATH=.*|KANBANR_BOARDS_PATH=$tmp/board|" "$site/.env.example" >"$mode_a"
# Mode (b): the example's commented line becomes the one in force.
sed -e 's|^COMPOSE_FILE=|# COMPOSE_FILE=|' \
    -e 's|^# COMPOSE_FILE=compose.yaml:compose.local.yaml$|COMPOSE_FILE=compose.yaml:compose.local.yaml|' \
    "$mode_a" >"$mode_b"

failed=0

# check <name> <env file> <mode>
check() {
    local name=$1 env=$2 mode=$3 json=$tmp/config-$3.json
    if ! (cd "$site" && docker compose --env-file "$env" config --format json) >"$json" 2>"$tmp/err"; then
        echo "FAIL: $name"
        sed 's/^/  /' "$tmp/err"
        failed=1
        return
    fi
    if python3 -I - "$json" "$mode" "$tmp/board" <<'PY'; then
import json, sys

config = json.load(open(sys.argv[1]))
mode, board = sys.argv[2], sys.argv[3]
services = config.get("services", {})
problems = []

expected = {"a": {"clone", "sync", "monitor"}, "b": {"monitor"}}[mode]
if set(services) != expected:
    problems.append(f"services are {sorted(services)}, expected {sorted(expected)}")

mounts = [v for v in services.get("monitor", {}).get("volumes", []) if v.get("target") == "/boards"]
if len(mounts) != 1:
    problems.append(f"the monitor has {len(mounts)} /boards mounts, expected one")
elif not mounts[0].get("read_only"):
    problems.append("the monitor's /boards mount is not read-only")
elif mode == "b" and (mounts[0].get("type"), mounts[0].get("source")) != ("bind", board):
    problems.append(f"the /boards mount is {mounts[0]}, expected a bind of {board}")
elif mode == "a" and (mounts[0].get("type"), mounts[0].get("source")) != ("volume", "boards"):
    problems.append(f"the /boards mount is {mounts[0]}, expected the boards volume")

for p in problems:
    print("  " + p)
sys.exit(1 if problems else 0)
PY
        echo "ok: $name"
    else
        echo "FAIL: $name"
        failed=1
    fi
}

check "compose: mode (a), remote URLs, validates" "$mode_a" a
check "compose: mode (b), local board folders, validates" "$mode_b" b

exit "$failed"
