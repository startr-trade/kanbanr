#!/bin/sh
# Run kanbanr's Rust tests: through cargo-nextest when it is installed, else
# cargo test (MNT-010). nextest skips doctests, so they run after it, always.
#
#   scripts/test.sh [--ignored] [cargo args…]
#
# KANBANR_TEST_RUNNER=nextest|cargo forces the runner. CARGO_BUILD_JOBS, when
# set, caps compile parallelism (nextest's -j means test threads, so it gets
# --build-jobs). The exit status is the first non-zero status of the commands.
set -u

cd "$(dirname "$0")/../api" || exit 2

ignored=0
if [ "${1:-}" = "--ignored" ]; then
    ignored=1
    shift
fi

runner="${KANBANR_TEST_RUNNER:-}"
probe() {
    version=$(cargo nextest --version 2>/dev/null) || return 1
    version=$(printf '%s\n' "$version" | sed -n '1s/^cargo-nextest //p')
}
case "$runner" in
    nextest)
        if ! probe; then
            echo "cargo-nextest is not installed: cargo install cargo-nextest --locked" >&2
            exit 2
        fi
        ;;
    cargo) ;;
    "")
        if probe; then
            runner=nextest
        else
            runner=cargo
        fi
        ;;
    *)
        echo "KANBANR_TEST_RUNNER is nextest or cargo" >&2
        exit 2
        ;;
esac

if [ "$runner" = nextest ]; then
    echo "test.sh: cargo-nextest ${version}"
elif [ -n "${KANBANR_TEST_RUNNER:-}" ]; then
    echo "test.sh: cargo test (KANBANR_TEST_RUNNER=cargo)"
else
    echo "test.sh: cargo test (cargo-nextest not installed)"
fi

jobs="${CARGO_BUILD_JOBS:-}"
status=0

# Keep the first non-zero status; later commands still run.
note() {
    if [ "$1" -ne 0 ] && [ "$status" -eq 0 ]; then
        status=$1
    fi
}

if [ "$runner" = nextest ]; then
    if [ "$ignored" -eq 1 ]; then
        cargo nextest run --locked --no-fail-fast ${jobs:+--build-jobs "$jobs"} --run-ignored only "$@"
        note $?
    else
        cargo nextest run --locked --workspace --no-fail-fast ${jobs:+--build-jobs "$jobs"} "$@"
        note $?
        cargo test --locked --workspace --doc ${jobs:+--jobs "$jobs"}
        note $?
    fi
else
    if [ "$ignored" -eq 1 ]; then
        cargo test --locked ${jobs:+--jobs "$jobs"} "$@" -- --ignored
        note $?
    else
        cargo test --locked --workspace --no-fail-fast ${jobs:+--jobs "$jobs"} "$@"
        note $?
    fi
fi

exit "$status"
