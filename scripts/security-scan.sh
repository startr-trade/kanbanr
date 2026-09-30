#!/usr/bin/env bash
# Local runs of the scanners the GitHub workflows run (FEAT-130), so a finding surfaces before a
# push instead of on a public Security tab.
#
#   security-scan.sh deps  [DIR]    Trivy over a source tree (default: a clean copy of the tracked
#                                   files): lockfiles, committed secrets, Dockerfile misconfig.
#                                   Fails on ANY finding not in .trivyignore.yaml — the workflow
#                                   only reports, so this is the stricter of the two on purpose.
#   security-scan.sh image [IMAGE]  Trivy over an image, as release.yml scans it before a push:
#                                   fails on a HIGH or CRITICAL finding that has a fix.
#   security-scan.sh codeql         CodeQL for Rust, JavaScript/TypeScript and the workflows, with
#                                   .github/codeql/codeql-config.yml. Several minutes; fails on
#                                   any result.
#
# Tools come from pinned upstream artefacts, so nothing is installed system-wide: Trivy as its
# container image, CodeQL as the CLI bundle cached under ~/.cache/codeql. Needs docker, git, curl,
# tar and jq.
set -euo pipefail

# Keep in step with the workflows: trivy-action v0.36.0 runs Trivy 0.70.0; codeql-action v4.38.2
# runs bundle v2.27.1 (see the action's defaults.json at that release).
TRIVY_VERSION=${TRIVY_VERSION:-0.70.0}
CODEQL_BUNDLE=${CODEQL_BUNDLE:-codeql-bundle-v2.27.1}
root=$(git rev-parse --show-toplevel)
cache=${XDG_CACHE_HOME:-$HOME/.cache}

trivy() {
    mkdir -p "$cache/trivy"
    docker run --rm -v "$cache/trivy:/root/.cache/trivy" "$@"
}

# What a checkout contains: the tracked files, nothing built or installed.
tracked_copy() {
    local dir
    dir=$(mktemp -d)
    git -C "$root" ls-files -z | tar -C "$root" --null -T - -cf - | tar -C "$dir" -xf -
    echo "$dir"
}

case "${1:-}" in
deps)
    tree=${2:-}
    cleanup=
    if [ -z "$tree" ]; then tree=$(tracked_copy); cleanup=$tree; fi
    status=0
    trivy -v "$tree:/src:ro" -w /src "aquasec/trivy:$TRIVY_VERSION" fs \
        --scanners vuln,secret,misconfig --severity UNKNOWN,LOW,MEDIUM,HIGH,CRITICAL \
        --ignorefile .trivyignore.yaml --exit-code 1 . || status=$?
    [ -z "$cleanup" ] || rm -rf "$cleanup"
    exit "$status"
    ;;
image)
    image=${2:-kanbanr:ci-local}
    trivy -v /var/run/docker.sock:/var/run/docker.sock -v "$root/.trivyignore.yaml:/.trivyignore.yaml:ro" \
        "aquasec/trivy:$TRIVY_VERSION" image --severity HIGH,CRITICAL --ignore-unfixed \
        --ignorefile /.trivyignore.yaml --exit-code 1 "$image"
    ;;
codeql)
    dir="$cache/codeql/$CODEQL_BUNDLE"
    if [ ! -x "$dir/codeql/codeql" ]; then
        mkdir -p "$dir"
        curl -fsSL --proto '=https' --proto-redir '=https' \
            "https://github.com/github/codeql-action/releases/download/$CODEQL_BUNDLE/codeql-bundle-linux64.tar.gz" \
            | tar -xz -C "$dir"
    fi
    codeql="$dir/codeql/codeql"
    tree=$(tracked_copy)
    out=$(mktemp -d)
    trap 'rm -rf "$tree" "$out"' EXIT
    found=0
    for lang in rust javascript-typescript actions; do
        "$codeql" database create "$out/db-$lang" --language="$lang" --build-mode=none \
            --source-root="$tree" --codescanning-config="$root/.github/codeql/codeql-config.yml" \
            --overwrite >/dev/null
        "$codeql" database analyze "$out/db-$lang" --format=sarif-latest \
            --output="$out/$lang.sarif" >/dev/null
        n=$(jq '[.runs[].results[]] | length' "$out/$lang.sarif")
        echo "codeql $lang: $n result(s)"
        if [ "$n" -gt 0 ]; then
            jq -r '.runs[].results[] | "  \(.ruleId): \(.locations[0].physicalLocation.artifactLocation.uri):\(.locations[0].physicalLocation.region.startLine) \(.message.text | split("\n")[0])"' "$out/$lang.sarif"
            found=1
        fi
    done
    exit "$found"
    ;;
*)
    sed -n '2,17p' "$0"
    exit 2
    ;;
esac
