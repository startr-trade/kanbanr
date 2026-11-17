#!/usr/bin/env bash
# Shared helpers for the hosted site's end-to-end checks (FTR-030):
# deploy/tests/e2e-kind.sh and deploy/tests/compose-smoke.sh source this file.
#
#   deploy/tests/lib.sh install-tools     install kind and kubectl (CI runners)
#
# Functions:
#   install_tools                 kind v0.24.0 and kubectl v1.31.14 from pinned
#                                 URLs, each checked against its SHA-256, into
#                                 ~/.local/bin (added to $GITHUB_PATH on Actions)
#   ensure_image                  the kanbanr:e2e image: on GitHub Actions the one
#                                 buildx loaded; elsewhere always rebuilt from
#                                 docker/Dockerfile, so a stale image is never tested
#   board_cli <dir> <project> <args...>
#                                 run the image's kanbanr CLI on the board <dir>
#   make_board <dir> <project>    a new board in <dir>: a git repository on main
#                                 with project <project>, milestone MS-001 and one
#                                 item, "First item", each committed
#   wait_for <seconds> <cmd...>   run <cmd> once a second until it succeeds;
#                                 return 1 when <seconds> pass first
#   free_port                     print a free TCP port on 127.0.0.1

set -euo pipefail

LIB_REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
E2E_IMAGE=kanbanr:e2e

KIND_VERSION=v0.24.0
KIND_URL=https://github.com/kubernetes-sigs/kind/releases/download/$KIND_VERSION/kind-linux-amd64
KIND_SHA256=b89aada5a39d620da3fcd16435b7f28d858927dd53f92cbac77686b0588b600d
KUBECTL_VERSION=v1.31.14
KUBECTL_URL=https://dl.k8s.io/release/$KUBECTL_VERSION/bin/linux/amd64/kubectl
KUBECTL_SHA256=8791ec7c8966b61420d55103a5fb948de9f0ca3d7306d789734975ad9704bdb0

# install_one <name> <url> <sha256> <bin dir>
install_one() {
    local name=$1 url=$2 sum=$3 bin=$4 tmp
    tmp=$(mktemp)
    curl -fsSL --retry 3 -o "$tmp" "$url"
    if ! echo "$sum  $tmp" | sha256sum -c --status; then
        rm -f "$tmp"
        echo "lib.sh: $name from $url does not match its pinned SHA-256" >&2
        return 1
    fi
    install -m 0755 "$tmp" "$bin/$name"
    rm -f "$tmp"
}

install_tools() {
    local bin=$HOME/.local/bin
    mkdir -p "$bin"
    install_one kind "$KIND_URL" "$KIND_SHA256" "$bin"
    install_one kubectl "$KUBECTL_URL" "$KUBECTL_SHA256" "$bin"
    if [ -n "${GITHUB_PATH:-}" ]; then
        echo "$bin" >>"$GITHUB_PATH"
    fi
    "$bin/kind" version
    "$bin/kubectl" version --client
}

ensure_image() {
    if [ "${GITHUB_ACTIONS:-}" = true ]; then
        if ! docker image inspect "$E2E_IMAGE" >/dev/null 2>&1; then
            echo "lib.sh: $E2E_IMAGE is missing; the workflow's build step loads it" >&2
            return 1
        fi
        return 0
    fi
    local build=(docker build -q -f "$LIB_REPO/docker/Dockerfile" -t "$E2E_IMAGE" "$LIB_REPO")
    if command -v flock >/dev/null 2>&1; then
        mkdir -p "$HOME/.cache"
        flock "$HOME/.cache/heavy-build.lock" "${build[@]}" >/dev/null
    else
        "${build[@]}" >/dev/null
    fi
}

board_cli() {
    local dir=$1 project=$2
    shift 2
    docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -e KANBANR_DATA_DIR=/board -e KANBANR_PUSH=off -e KANBANR_PROJECT="$project" \
        -v "$dir:/board" "$E2E_IMAGE" kanbanr "$@" >/dev/null
}

make_board() {
    local dir=$1 project=$2
    mkdir -p "$dir"
    git -C "$dir" init -q -b main
    git -C "$dir" config user.name "kanbanr e2e"
    git -C "$dir" config user.email "e2e@example.invalid"
    mkdir -p "$dir/projects"
    board_cli "$dir" "$project" project init "$project"
    board_cli "$dir" "$project" milestone add --name M1
    board_cli "$dir" "$project" feature add --title "First item" --milestone MS-001
}

wait_for() {
    local seconds=$1 start
    shift
    start=$(date +%s)
    until "$@" >/dev/null 2>&1; do
        if [ $(($(date +%s) - start)) -ge "$seconds" ]; then
            return 1
        fi
        sleep 1
    done
}

free_port() {
    python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    case "${1:-}" in
        install-tools) install_tools ;;
        *)
            echo "usage: deploy/tests/lib.sh install-tools (or source it)" >&2
            exit 2
            ;;
    esac
fi
