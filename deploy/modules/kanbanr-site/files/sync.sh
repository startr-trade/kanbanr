#!/bin/sh
# kanbanr hosted site: keep one board's checkout current from its git remote.
#
# Kubernetes runs one process per container (`clone` as the init container,
# `follow` as the sidecar, `alive` as its liveness probe); compose runs
# `clone-all`, `follow-all` and `alive-all` over a boards.conf. The script is
# the same file in both (FTR-030).
#
#   sync.sh clone              clone $BOARD_URL into $BOARD_DIR (full clone)
#   sync.sh follow             fetch and fast-forward $BOARD_DIR in place
#   sync.sh alive              exit 0 while the follow loop is making rounds
#   sync.sh clone-all  <conf>  clone each board of <conf>, in order
#   sync.sh follow-all <conf>  one follow loop per board of <conf>
#   sync.sh alive-all  <conf>  alive for each board of <conf>
#
# <conf> lines are `<name> <url> [<interval>] [<branch>]`; `#` comments and
# blank lines are skipped, and a duplicate name is an error.
#
# Environment (per board): BOARD_NAME, BOARD_URL, BOARD_BRANCH, BOARD_DIR,
# PULL_INTERVAL, MAX_BACKOFF, FETCH_TIMEOUT, CLONE_TIMEOUT, HOME, HEARTBEAT,
# CREDENTIAL_KIND (none | token | ssh), CREDENTIAL_USERNAME, CREDENTIAL_DIR,
# KNOWN_HOSTS_FILE. For tests only: SYNC_ROUNDS (rounds of `follow`, 0 means
# forever) and BOARDS_ROOT (where the *-all commands put checkouts, /boards).
#
# The checkout is never reset, cleaned or re-cloned by `follow`: a good
# checkout is kept through any failure, and a restart clones afresh.

set -eu

PULL_INTERVAL=${PULL_INTERVAL:-60}
MAX_BACKOFF=${MAX_BACKOFF:-600}
FETCH_TIMEOUT=${FETCH_TIMEOUT:-120}
CLONE_TIMEOUT=${CLONE_TIMEOUT:-900}
HEARTBEAT=${HEARTBEAT:-/tmp/sync-heartbeat}
CREDENTIAL_KIND=${CREDENTIAL_KIND:-none}
CREDENTIAL_USERNAME=${CREDENTIAL_USERNAME:-git}
CREDENTIAL_DIR=${CREDENTIAL_DIR:-/credential}
SYNC_ROUNDS=${SYNC_ROUNDS:-0}
BOARDS_ROOT=${BOARDS_ROOT:-/boards}
BOARD_BRANCH=${BOARD_BRANCH:-}
BOARD_NAME=${BOARD_NAME:-}

# `<UTC ISO time> sync: [<board>] <text>` on stderr.
log() {
    printf '%s sync: [%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$BOARD_NAME" "$*" >&2
}

head_sha() {
    git -C "$BOARD_DIR" rev-parse --short HEAD
}

# Credentials for this board's git, from its own mounted secret. The token
# never appears in a URL, .git/config, a process argument or a log line: the
# helper reads the file when git asks.
setup_git() {
    case "$CREDENTIAL_KIND" in
        none) ;;
        token)
            install -d -m 700 "$HOME"
            # Single quotes: the variables are expanded by git's shell when
            # the helper runs, never written into the config.
            # shellcheck disable=SC2016
            git config --global credential.helper \
                '!f() { test "$1" = get || return 0; IFS= read -r t < "$CREDENTIAL_DIR/token" || test -n "$t"; printf "username=%s\npassword=%s\n" "$CREDENTIAL_USERNAME" "$t"; }; f'
            ;;
        ssh)
            install -d -m 700 "$HOME" "$HOME/.ssh"
            install -m 600 "$CREDENTIAL_DIR/ssh-privatekey" "$HOME/.ssh/id"
            KNOWN_HOSTS_FILE=${KNOWN_HOSTS_FILE:-/scripts/known_hosts-$BOARD_NAME}
            GIT_SSH_COMMAND="ssh -i $HOME/.ssh/id -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$KNOWN_HOSTS_FILE"
            export GIT_SSH_COMMAND
            ;;
        *)
            log "unknown CREDENTIAL_KIND '$CREDENTIAL_KIND' (none, token or ssh)"
            exit 2
            ;;
    esac
}

cmd_clone() {
    : "${BOARD_DIR:?BOARD_DIR is not set}" "${BOARD_URL:?BOARD_URL is not set}"
    if [ -e "$BOARD_DIR/.git" ]; then
        log "checkout present at $(head_sha); not cloning"
        return 0
    fi
    setup_git
    rm -rf "$BOARD_DIR.partial"
    rc=0
    if [ -n "$BOARD_BRANCH" ]; then
        timeout "$CLONE_TIMEOUT" git clone --no-tags --single-branch --branch "$BOARD_BRANCH" \
            -- "$BOARD_URL" "$BOARD_DIR.partial" || rc=$?
    else
        timeout "$CLONE_TIMEOUT" git clone --no-tags --single-branch \
            -- "$BOARD_URL" "$BOARD_DIR.partial" || rc=$?
    fi
    if [ "$rc" -ne 0 ]; then
        rm -rf "$BOARD_DIR.partial"
        log "clone failed (exit $rc); retrying is the caller's"
        exit 1
    fi
    mv "$BOARD_DIR.partial" "$BOARD_DIR"
    branch=$BOARD_BRANCH
    if [ -z "$branch" ]; then
        branch=$(git -C "$BOARD_DIR" rev-parse --abbrev-ref HEAD)
    fi
    log "cloned $branch at $(head_sha)"
}

backoff() {
    next=$((delay * 2))
    if [ "$next" -gt "$MAX_BACKOFF" ]; then next=$MAX_BACKOFF; fi
    if [ "$next" -lt "$PULL_INTERVAL" ]; then next=$PULL_INTERVAL; fi
    delay=$next
}

cmd_follow() {
    : "${BOARD_DIR:?BOARD_DIR is not set}"
    if [ ! -e "$BOARD_DIR/.git" ]; then
        log "no checkout at $BOARD_DIR; the clone runs first"
        exit 1
    fi
    setup_git
    branch=$BOARD_BRANCH
    if [ -z "$branch" ]; then
        branch=$(git -C "$BOARD_DIR" rev-parse --abbrev-ref HEAD)
    fi
    delay=$PULL_INTERVAL
    round=0
    while [ "$SYNC_ROUNDS" -eq 0 ] || [ "$round" -lt "$SYNC_ROUNDS" ]; do
        round=$((round + 1))
        touch "$HEARTBEAT"
        sleep "$delay"
        touch "$HEARTBEAT"
        old=$(head_sha)
        rc=0
        timeout "$FETCH_TIMEOUT" git -C "$BOARD_DIR" fetch --no-tags --quiet origin \
            "+refs/heads/$branch:refs/remotes/origin/$branch" || rc=$?
        if [ "$rc" -eq 0 ]; then
            if git -C "$BOARD_DIR" merge --ff-only --quiet "refs/remotes/origin/$branch"; then
                new=$(head_sha)
                if [ "$new" != "$old" ]; then
                    log "updated $old..$new"
                fi
                delay=$PULL_INTERVAL
            else
                backoff
                log "the remote's $branch is not a fast-forward of the checkout at $old; keeping it (restart to clone afresh); next try in ${delay}s"
            fi
        else
            backoff
            log "fetch failed (exit $rc); keeping $old; next try in ${delay}s"
        fi
    done
}

cmd_alive() {
    limit=$MAX_BACKOFF
    if [ "$PULL_INTERVAL" -gt "$limit" ]; then limit=$PULL_INTERVAL; fi
    limit=$((limit + FETCH_TIMEOUT + 60))
    now=$(date +%s)
    if [ -e "$HEARTBEAT" ]; then
        age=$((now - $(stat -c %Y "$HEARTBEAT")))
    else
        age=$now
    fi
    if [ "$age" -le "$limit" ]; then
        return 0
    fi
    if [ -e "$HEARTBEAT" ]; then
        printf 'no sync round for %ss\n' "$age"
    else
        printf 'no sync round yet (no heartbeat at %s)\n' "$HEARTBEAT"
    fi
    exit 1
}

# Print the conf's board lines as `<name> <url> <interval> <branch>`, with the
# interval defaulting to 60 and an empty branch as `-`. A line without a URL
# or a duplicate name is an error.
conf_lines() {
    conf=$1
    if [ ! -r "$conf" ]; then
        printf 'sync: cannot read %s\n' "$conf" >&2
        exit 2
    fi
    seen=" "
    out=""
    set -f
    while IFS= read -r line || [ -n "$line" ]; do
        # shellcheck disable=SC2086
        set -- $line
        case "${1:-#}" in '#'*) continue ;; esac
        if [ $# -lt 2 ]; then
            printf 'sync: %s: "%s" needs a name and a URL\n' "$conf" "$line" >&2
            exit 2
        fi
        case "$seen" in
            *" $1 "*)
                printf 'sync: %s: board %s is listed twice\n' "$conf" "$1" >&2
                exit 2
                ;;
        esac
        seen="$seen$1 "
        out="$out$1 $2 ${3:-60} ${4:--}
"
    done < "$conf"
    set +f
    printf '%s' "$out"
}

# Set one conf board's env: <name> <url> <interval> <branch>.
board_env() {
    BOARD_NAME=$1
    BOARD_URL=$2
    PULL_INTERVAL=$3
    BOARD_BRANCH=$4
    if [ "$BOARD_BRANCH" = "-" ]; then BOARD_BRANCH=; fi
    BOARD_DIR=$BOARDS_ROOT/$BOARD_NAME
    HEARTBEAT=${TMPDIR:-/tmp}/sync-heartbeat-$BOARD_NAME
}

cmd_clone_all() {
    lines=$(conf_lines "${1:?usage: sync.sh clone-all <conf>}")
    # Each clone runs in its own subshell; under set -e the first failure
    # ends this script with exit 1.
    while read -r name url interval branch; do
        [ -n "$name" ] || continue
        ( board_env "$name" "$url" "$interval" "$branch"; cmd_clone )
    done <<EOF
$lines
EOF
}

cmd_follow_all() {
    lines=$(conf_lines "${1:?usage: sync.sh follow-all <conf>}")
    pids=""
    # A child that exits for any reason signals this shell, which stops the
    # rest and exits 1, so compose restarts the service.
    trap 'trap - USR1; stop_children; printf "sync: a follow loop exited; stopping the others\n" >&2; exit 1' USR1
    trap 'trap - USR1; stop_children; exit 143' TERM INT
    parent=$$
    while read -r name url interval branch; do
        [ -n "$name" ] || continue
        (
            trap 'kill -USR1 '"$parent" EXIT
            board_env "$name" "$url" "$interval" "$branch"
            cmd_follow
        ) &
        pids="$pids $!"
    done <<EOF
$lines
EOF
    # `wait` returns early when the USR1 trap fires; if it returns on its
    # own, every loop has ended.
    wait || :
    stop_children
    exit 1
}

stop_children() {
    if [ -n "$pids" ]; then
        # shellcheck disable=SC2086
        kill $pids 2>/dev/null || :
    fi
}

cmd_alive_all() {
    lines=$(conf_lines "${1:?usage: sync.sh alive-all <conf>}")
    failed=0
    while read -r name url interval branch; do
        [ -n "$name" ] || continue
        # In the background, so set -e still holds inside the check.
        ( board_env "$name" "$url" "$interval" "$branch"; cmd_alive ) &
        wait "$!" || failed=1
    done <<EOF
$lines
EOF
    exit "$failed"
}

case "${1:-}" in
    clone) cmd_clone ;;
    follow) cmd_follow ;;
    alive) cmd_alive ;;
    clone-all) shift; cmd_clone_all "$@" ;;
    follow-all) shift; cmd_follow_all "$@" ;;
    alive-all) shift; cmd_alive_all "$@" ;;
    *)
        printf 'usage: sync.sh clone|follow|alive|clone-all <conf>|follow-all <conf>|alive-all <conf>\n' >&2
        exit 2
        ;;
esac
