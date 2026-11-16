#!/usr/bin/env bash
# Runs OpenTofu or Terraform from a pinned image against a folder of deploy/
# (FTR-030), so the checks need only Docker on the machine.
#
#   deploy/tests/tf.sh <tofu|terraform> <dir> <args...>
#
# <dir> is relative to deploy/ (modules/kanbanr-site, examples/site, tests/e2e).
# The images are pinned by digest, read with `docker buildx imagetools inspect`
# on 17 Nov 2026 (multi-arch index digests). Providers are cached in
# ${XDG_CACHE_HOME:-~/.cache}/kanbanr-tf. When KUBECONFIG_FILE is set, that file
# is mounted read-only and KUBE_CONFIG_PATH names it inside the container.

set -euo pipefail

TOFU_IMAGE="ghcr.io/opentofu/opentofu:1.8.8@sha256:dd7d334e94e37ebfeadc272b96d900b44585967fab838e420a70b5ee2ff7dbe3"
TERRAFORM_IMAGE="docker.io/hashicorp/terraform:1.17.2@sha256:6e9c1e647c31a960dc9b59e13272f9111d90010c10b717f77ffb324767660475"

if [ "$#" -lt 2 ]; then
    echo "usage: deploy/tests/tf.sh <tofu|terraform> <dir> <args...>" >&2
    exit 2
fi

tool=$1
dir=$2
shift 2

case "$tool" in
    tofu) image=$TOFU_IMAGE ;;
    terraform) image=$TERRAFORM_IMAGE ;;
    *)
        echo "tf.sh: the tool is tofu or terraform, not '$tool'" >&2
        exit 2
        ;;
esac

repo=$(cd "$(dirname "$0")/../.." && pwd)
if [ ! -d "$repo/deploy/$dir" ]; then
    echo "tf.sh: no folder deploy/$dir" >&2
    exit 2
fi

cache=${XDG_CACHE_HOME:-$HOME/.cache}/kanbanr-tf
mkdir -p "$cache"

opts=(
    --rm
    --network host
    --user "$(id -u):$(id -g)"
    -e HOME=/tmp
    -e TF_IN_AUTOMATION=1
    -e TF_PLUGIN_CACHE_DIR=/plugin-cache
    -v "$repo/deploy:/deploy"
    -v "$cache:/plugin-cache"
    -w "/deploy/$dir"
)

if [ -n "${KUBECONFIG_FILE:-}" ]; then
    if [ ! -f "$KUBECONFIG_FILE" ]; then
        echo "tf.sh: KUBECONFIG_FILE names no file: $KUBECONFIG_FILE" >&2
        exit 2
    fi
    opts+=(-v "$(cd "$(dirname "$KUBECONFIG_FILE")" && pwd)/$(basename "$KUBECONFIG_FILE"):/kubeconfig:ro"
           -e KUBE_CONFIG_PATH=/kubeconfig)
fi

exec docker run "${opts[@]}" "$image" "$@"
