#!/usr/bin/env bash
# The hosted site, end to end on a throwaway kind cluster (FTR-030 R-1, R-3,
# R-5, R-6 and the freshness NFR).
#
#   deploy/tests/e2e-kind.sh
#
# Makes two boards (projects e2e and e2e2) and pushes them to bare remotes in a
# temp dir, which a kind node mounts and an in-cluster BusyBox httpd serves
# (e2e/remote.yaml). Applies the e2e root (e2e/main.tf) with the kanbanr:e2e
# image, port-forwards the site's Service and runs the cases. Each case prints
# `ok: <name>` or `FAIL: <name>`; the script exits 1 if any failed. The cluster,
# the port-forward, the temp dir and the root's state are removed on exit.
# Needs Docker, kind, kubectl, git, curl and python3.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=deploy/tests/lib.sh
. "$here/lib.sh"

cluster=kanbanr-e2e-$$
tmp=$(mktemp -d)
pf_pid=
sse_pid=

cleanup() {
    [ -n "$sse_pid" ] && kill "$sse_pid" 2>/dev/null || true
    [ -n "$pf_pid" ] && kill "$pf_pid" 2>/dev/null || true
    kind delete cluster --name "$cluster" >/dev/null 2>&1 || true
    rm -f "$here/e2e/terraform.tfstate" "$here/e2e/terraform.tfstate.backup"
    rm -rf "$tmp"
}
trap cleanup EXIT

failed=0
ok() { echo "ok: $1"; }
fail() {
    echo "FAIL: $1"
    shift
    for why in "$@"; do echo "  $why"; done
    failed=1
}

# 1. The image.
ensure_image

# 2. Two boards, each pushed to its bare remote; the post-update hook keeps the
# dumb-HTTP files current.
mkdir -p "$tmp/remote"
# new_board <working dir> <project> <bare repository name>
new_board() {
    local dir=$1 project=$2 bare=$tmp/remote/$3
    make_board "$dir" "$project"
    git init -q --bare -b main "$bare"
    printf '#!/bin/sh\nexec git update-server-info\n' >"$bare/hooks/post-update"
    chmod +x "$bare/hooks/post-update"
    git -C "$dir" remote add origin "$bare"
    git -C "$dir" push -q origin main
}
new_board "$tmp/board-one" e2e board.git
new_board "$tmp/board-two" e2e2 board2.git
# The httpd runs as nobody; every file it serves must be world-readable.
chmod -R a+rX "$tmp/remote"

# 3. The cluster, with the remotes mounted into its node.
cat >"$tmp/kind.yaml" <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    extraMounts:
      - hostPath: $tmp/remote
        containerPath: /e2e-remote
        readOnly: true
EOF
export KUBECONFIG=$tmp/kubeconfig
kind create cluster --name "$cluster" --config "$tmp/kind.yaml" --kubeconfig "$KUBECONFIG" --wait 120s >/dev/null
kind load docker-image "$E2E_IMAGE" --name "$cluster" >/dev/null

# 4. The remote.
kubectl apply -f "$here/e2e/remote.yaml" >/dev/null
kubectl -n e2e-remote rollout status deploy/board-remote --timeout=120s >/dev/null

# 5. The site.
export KUBECONFIG_FILE=$KUBECONFIG
rm -f "$here/e2e/terraform.tfstate" "$here/e2e/terraform.tfstate.backup"
if ! "$here/tf.sh" tofu tests/e2e init -input=false -no-color >"$tmp/tf.log" 2>&1 ||
    ! "$here/tf.sh" tofu tests/e2e apply -auto-approve -input=false -no-color -var image="$E2E_IMAGE" >>"$tmp/tf.log" 2>&1; then
    echo "FAIL: deploy e2e: the site applies"
    sed 's/^/  /' "$tmp/tf.log"
    kubectl -n e2e get pods -o wide || true
    for c in $(kubectl -n e2e get pods -o jsonpath='{.items[*].spec.initContainers[*].name} {.items[*].spec.containers[*].name}' 2>/dev/null); do
        echo "  --- logs -c $c"
        kubectl -n e2e logs deploy/kanbanr-site -c "$c" --tail=20 2>&1 | sed 's/^/  /' || true
    done
    exit 1
fi

# 6. The port-forward.
port=$(free_port)
kubectl -n e2e port-forward svc/kanbanr-site "$port:80" >"$tmp/port-forward.log" 2>&1 &
pf_pid=$!
base=http://127.0.0.1:$port
if ! wait_for 30 curl -fsS -H 'Host: localhost' "$base/healthz"; then
    echo "FAIL: deploy e2e: the site's Service answers through a port-forward"
    sed 's/^/  /' "$tmp/port-forward.log"
    exit 1
fi

get() { curl -sS -H 'Host: localhost' "$base$1"; }

# 7. The cases.

case_two_boards() {
    local name="deploy e2e: one site serves two boards' projects" why
    get /api/projects >"$tmp/projects.json"
    get /api/boards >"$tmp/boards.json"
    if why=$(python3 -I - "$tmp/projects.json" "$tmp/boards.json" \
        "$(git -C "$tmp/remote/board.git" rev-parse main)" \
        "$(git -C "$tmp/remote/board2.git" rev-parse main)" <<'PY'
import json, sys

projects = json.load(open(sys.argv[1]))
boards = json.load(open(sys.argv[2]))
heads = {"board-one": sys.argv[3], "board-two": sys.argv[4]}
expect = {"board-one": ["e2e"], "board-two": ["e2e2"]}
problems = []

ids = sorted(p.get("id") for p in projects)
if ids != ["e2e", "e2e2"]:
    problems.append(f"/api/projects lists {ids}, expected ['e2e', 'e2e2']")
names = [b.get("name") for b in boards]
if names != ["board-one", "board-two"]:
    problems.append(f"/api/boards lists {names}, expected ['board-one', 'board-two']")
for b in boards:
    n = b.get("name")
    if n not in heads:
        continue
    if b.get("projects") != expect[n]:
        problems.append(f"{n} serves {b.get('projects')}, expected {expect[n]}")
    if b.get("head") != heads[n]:
        problems.append(f"{n}'s head is {b.get('head')}, its remote's tip is {heads[n]}")
    if b.get("history_unreadable"):
        problems.append(f"{n}: history can't be read: {b['history_unreadable']}")
print("\n".join(problems))
sys.exit(1 if problems else 0)
PY
    ); then
        ok "$name"
    else
        fail "$name" "$why"
    fi
}

case_read_only() {
    local name="deploy e2e: the monitor serves its boards from a read-only mount as a non-root user"
    local problems=() uid writes code
    if kubectl -n e2e exec deploy/kanbanr-site -c monitor -- touch /boards/board-one/x >/dev/null 2>&1; then
        problems+=("touch /boards/board-one/x succeeded in the monitor")
    fi
    uid=$(kubectl -n e2e exec deploy/kanbanr-site -c monitor -- id -u 2>&1 || true)
    [ "$uid" = 10001 ] || problems+=("the monitor runs as uid '$uid', expected 10001")
    writes=$(get /api/meta | python3 -I -c 'import json, sys; print(json.load(sys.stdin).get("writes"))')
    [ "$writes" = False ] || problems+=("/api/meta has writes: $writes, expected false")
    code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: localhost' "$base/")
    [ "$code" = 200 ] || problems+=("GET / answered $code, expected 200")
    if [ ${#problems[@]} -eq 0 ]; then ok "$name"; else fail "$name" "${problems[@]}"; fi
}

case_host_check() {
    local name="deploy e2e: the ingress host is answered and any other host is refused"
    local problems=() site other
    site=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: site.e2e.test' "$base/")
    other=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: elsewhere.test' "$base/")
    [ "$site" = 200 ] || problems+=("Host: site.e2e.test answered $site, expected 200")
    [ "$other" = 403 ] || problems+=("Host: elsewhere.test answered $other, expected 403")
    if [ ${#problems[@]} -eq 0 ]; then ok "$name"; else fail "$name" "${problems[@]}"; fi
}

# The push case: R-3 and the NFR (push to listed within 5 + 15 s), and the live
# refresh's `changed` event for the pulled change.
case_push() {
    local fresh="deploy e2e: a pushed board change shows on the page within one pull interval"
    local live="deploy e2e: the page's live refresh fires for a pulled change"
    local nonce title start elapsed listed=no
    curl -sN -H 'Host: localhost' "$base/api/projects/e2e/events" >"$tmp/sse.log" 2>&1 &
    sse_pid=$!
    sleep 1
    nonce=$$-$RANDOM
    title="Pushed $nonce"
    board_cli "$tmp/board-one" e2e feature add --title "$title" --milestone MS-001
    start=$(date +%s)
    git -C "$tmp/board-one" push -q origin main
    for _ in $(seq 1 20); do
        sleep 1
        if get /api/projects/e2e | grep -F "$title" >/dev/null; then
            listed=yes
            break
        fi
    done
    elapsed=$(($(date +%s) - start))
    if [ "$listed" = yes ] && [ "$elapsed" -le 20 ]; then
        ok "$fresh"
        echo "  listed ${elapsed}s after the push (interval 5 s, budget 20 s)"
    else
        fail "$fresh" "\"$title\" not listed within 20 s of the push"
        kubectl -n e2e logs deploy/kanbanr-site -c sync-board-one --tail=10 2>&1 | sed 's/^/  /' || true
    fi
    wait_for 5 grep -q '^event: changed' "$tmp/sse.log" || true
    kill "$sse_pid" 2>/dev/null || true
    sse_pid=
    if grep -q '^event: changed' "$tmp/sse.log"; then
        ok "$live"
    else
        fail "$live" "no 'event: changed' on /api/projects/e2e/events after the pull"
    fi
}

case_two_boards
case_read_only
case_host_check
case_push

exit "$failed"
