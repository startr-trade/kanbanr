#!/usr/bin/env bash
# The module, the example root and the e2e root validate under OpenTofu and
# Terraform, and the module's tests pass under both (FTR-030 R-1).
#
#   deploy/tests/validate.sh
#
# For each tool: `init -backend=false` and `validate` in each root, then `test`
# in the module; `fmt -check -recursive` over deploy/ with tofu only. A failing
# step prints its output and exits 1. The tests' `run "..."... pass` lines are
# passed through. Needs Docker (deploy/tests/tf.sh).

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
tf=$here/tf.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

roots=(modules/kanbanr-site examples/site tests/e2e)

# step <description> <tool> <dir> <args...>: runs quietly, prints the output on failure.
step() {
    local what=$1
    shift
    if ! "$tf" "$@" -no-color >"$tmp/out" 2>&1; then
        echo "FAIL: deploy: $what"
        sed 's/^/  /' "$tmp/out"
        exit 1
    fi
}

for tool in tofu terraform; do
    for dir in "${roots[@]}"; do
        step "$tool init in $dir" "$tool" "$dir" init -backend=false -input=false
        step "$tool validate in $dir" "$tool" "$dir" validate
    done
    step "$tool test in modules/kanbanr-site" "$tool" modules/kanbanr-site test
    grep -E '^[[:space:]]*run ".*"\.\.\. ' "$tmp/out" || true
done

step "tofu fmt -check -recursive" tofu . fmt -check -recursive

echo "ok: deploy: the module, the example root and the e2e root validate under OpenTofu and Terraform"
