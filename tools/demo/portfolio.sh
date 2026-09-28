#!/usr/bin/env bash
# Build a throwaway three-project portfolio with real cross-project dependencies.
#
#   tools/demo/portfolio.sh [DATA_DIR]      # default: a temp dir, printed at the end
#
# Why this exists: kanbanr's own board is ONE project, so nothing in this repository exercises the
# portfolio views or shows what a cross-project dependency looks like. This builds a small, honest
# example — a shared identity service two other products depend on — which is both the worked
# example in the docs and the fixture the portfolio screenshots are taken against.
#
# It writes to its own data folder and never touches your board.
set -euo pipefail

KANBANR="${KANBANR:-kanbanr}"
DATA_DIR="${1:-$(mktemp -d "${TMPDIR:-/tmp}/kanbanr-demo.XXXXXX")/board}"
mkdir -p "$DATA_DIR"
export KANBANR_DATA_DIR="$DATA_DIR"

k() { "$KANBANR" --data-dir "$DATA_DIR" "$@"; }

echo "building a demo portfolio in $DATA_DIR"

# Three projects. `identity` is the shared one; the other two rest on it, which is the whole point
# of a portfolio view — the dependency that matters is the one you cannot see from inside a project.
for p in identity checkout mobile; do
    k project init "$p" --description "demo: the $p product" >/dev/null
    k --project "$p" milestone add --name "v1" --code MS-001 >/dev/null
done

add() { # add <project> <title> [depends-on...]
    local proj="$1" title="$2"; shift 2
    if [ $# -gt 0 ]; then
        k --project "$proj" feature add --title "$title" --milestone MS-001 \
            --spec "# $title" --depends-on "$1" >/dev/null
    else
        k --project "$proj" feature add --title "$title" --milestone MS-001 --spec "# $title" >/dev/null
    fi
}

# identity — the thing everything else waits on.
add identity "Store a user record"                     # FEAT-001
add identity "Issue a session token"                   # FEAT-002
add identity "Revoke a session"                        # FEAT-003

# checkout — cannot take a payment until identity can say who is paying.
add checkout "Price a basket"                          # FEAT-001
add checkout "Take a card payment" "identity:FEAT-002" # FEAT-002, ACROSS projects
add checkout "Refund a payment" "checkout:FEAT-002"    # FEAT-003, within

# mobile — rests on both.
add mobile "Sign in on device" "identity:FEAT-002"     # FEAT-001, across
add mobile "Pay in app" "checkout:FEAT-002"            # FEAT-002, across a second project

# Move a few along so the rollups and the cross-project board are not all one colour.
k --project identity move FEAT-001 Completed >/dev/null 2>&1 || {
    k --project identity move FEAT-001 Scheduled >/dev/null
    k --project identity move FEAT-001 Completed >/dev/null
}
k --project identity move FEAT-002 Scheduled >/dev/null
k --project checkout move FEAT-001 Scheduled >/dev/null

# The program that groups them. Without this the projects are just neighbours in a folder.
k portfolio add-program platform --name "Platform" \
    --description "The shared identity service and the two products that rest on it" \
    --projects identity,checkout,mobile >/dev/null

echo
k portfolio show
echo
echo "What is worth looking at:"
echo "  kanbanr --data-dir $DATA_DIR portfolio rollups     # milestone% -> project% -> program%"
echo "  kanbanr --data-dir $DATA_DIR portfolio board       # every project's work in one set of lanes"
echo "  kanbanr --data-dir $DATA_DIR --project identity impact FEAT-002"
echo "                                                     # what breaks in OTHER projects if this slips"
echo "  kanbanr --data-dir $DATA_DIR --project mobile blocked"
echo "                                                     # waiting on work it does not own"
echo
echo "  kanbanr serve --data-dir $DATA_DIR --bind 127.0.0.1:8081   # then open /portfolio"
