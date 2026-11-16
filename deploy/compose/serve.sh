#!/usr/bin/env bash
# kanbanr hosted site, compose: start one read-only `kanbanr serve` over the
# site's boards, one --board per board, in order (FTR-030).
#
# Mode (a), remote URLs: /config/boards.conf exists; one /boards/<name> per
#   board line (`<name> <url> [<interval>] [<branch>]`), as sync.sh cloned them.
# Mode (b), local folders: one /boards/<name> per comma-separated name in
#   KANBANR_LOCAL_BOARDS; when that is empty, /boards itself is the one board
#   (and its name is `boards`).
#
# Exits 2 with a message on an empty list or a board folder without projects/.
# For tests only: SERVE_CONFIG (the conf, /config/boards.conf) and BOARDS_ROOT
# (the boards' parent folder, /boards).

set -euo pipefail

conf=${SERVE_CONFIG:-/config/boards.conf}
root=${BOARDS_ROOT:-/boards}
boards=()

if [[ -e $conf ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
        read -r name _ <<<"$line" || true
        [[ -z ${name:-} || $name == '#'* ]] && continue
        boards+=("$root/$name")
    done <"$conf"
elif [[ -n ${KANBANR_LOCAL_BOARDS:-} ]]; then
    IFS=',' read -r -a names <<<"$KANBANR_LOCAL_BOARDS"
    for name in "${names[@]}"; do
        name=${name//[[:space:]]/}
        [[ -n $name ]] && boards+=("$root/$name")
    done
else
    boards=("$root")
fi

if ((${#boards[@]} == 0)); then
    echo "serve.sh: no boards to serve: list them in $conf (remote URLs) or KANBANR_LOCAL_BOARDS (local folders)" >&2
    exit 2
fi

args=()
for board in "${boards[@]}"; do
    if [[ ! -d $board/projects ]]; then
        echo "serve.sh: $board is not a board folder (it has no projects/)" >&2
        exit 2
    fi
    args+=(--board "$board")
done

echo "serve.sh: serving ${#boards[@]} board(s): ${boards[*]}" >&2
exec kanbanr serve "${args[@]}"
