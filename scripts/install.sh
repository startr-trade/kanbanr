#!/bin/sh
# kanbanr installer.
#
#   curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh
#
# Downloads the `kanbanr` binary for this platform from a GitHub release, VERIFIES its SHA-256
# against the release's own SHA256SUMS, and installs it — and, when Claude Code (the `claude`
# command) is on PATH, installs the kanbanr skill that binary carries into ~/.claude/skills/kanbanr
# (FEAT-141). With --vscode it also installs the release's VS Code extension into the editors it
# finds (FEAT-158). Nothing else: no shell profile is edited, no package manager is invoked, no
# daemon is started. --no-skill leaves Claude Code alone.
#
# The binary carries the web monitor and the skill inside it (FEAT-084, FEAT-141), so this is the
# whole install, and the skill always matches the program it drives.
#
# Knobs (env or flag):
#   KANBANR_VERSION=v0.1.0        --version <tag>   pin a release (default: latest, incl. pre-release)
#   KANBANR_INSTALL_DIR=~/.local/bin  --dir <path>  install location (default: see below)
#   KANBANR_NO_VERIFY=1                             skip checksum verification (discouraged)
#   KANBANR_NO_SKILL=1                --no-skill    do not install the Claude Code skill
#   KANBANR_VSCODE=1                  --vscode      also install the VS Code extension into every
#                                                   editor found (code, codium, cursor, windsurf);
#   KANBANR_VSCODE=codium             --vscode=codium   …or only into the one named
#
# Default install dir: $KANBANR_INSTALL_DIR, else /usr/local/bin when writable (or sudo is
# available and we are interactive), else ~/.local/bin.
#
# Exit codes: 0 ok · 1 usage/args · 2 unsupported platform · 3 download/network · 4 checksum.
#
# Structure and most of the hard-won details here follow an installer that has already been
# through the failures this kind of script hits: absent
# timeouts hanging forever, `/releases/latest` 404ing while every release is a pre-release, and a
# helper that reported "no token" as a failure under `set -e`. Copying the shape of something that
# has survived contact is cheaper than rediscovering each one.

set -eu

REPO="startr-trade/kanbanr"
API="https://api.github.com/repos/${REPO}"
DL="https://github.com/${REPO}/releases/download"

VERSION="${KANBANR_VERSION:-}"
INSTALL_DIR="${KANBANR_INSTALL_DIR:-}"
NO_VERIFY="${KANBANR_NO_VERIFY:-}"
NO_SKILL="${KANBANR_NO_SKILL:-}"
VSCODE="${KANBANR_VSCODE:-}"

die() { printf 'kanbanr-install: %s\n' "$1" >&2; exit "${2:-1}"; }
info() { printf '  %s\n' "$1" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?--version needs a tag}"; shift 2 ;;
        --dir)     INSTALL_DIR="${2:?--dir needs a path}"; shift 2 ;;
        --no-verify) NO_VERIFY=1; shift ;;
        --no-skill)  NO_SKILL=1; shift ;;
        --vscode)    VSCODE=1; shift ;;
        --vscode=*)  VSCODE="${1#--vscode=}"; shift ;;
        -h|--help)
            sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

# ---- platform ---------------------------------------------------------------------------
# These four targets are what .github/workflows/release.yml publishes. Anything else must build
# from source rather than receive a silently wrong binary.
os="$(uname -s)"
arch="$(uname -m)"
case "${os}/${arch}" in
    Linux/x86_64|Linux/amd64)   TARGET="x86_64-unknown-linux-gnu" ;;
    Linux/aarch64|Linux/arm64)  TARGET="aarch64-unknown-linux-gnu" ;;
    Darwin/arm64)               TARGET="aarch64-apple-darwin" ;;
    Darwin/x86_64)              TARGET="x86_64-apple-darwin" ;;
    *) die "unsupported platform: ${os}/${arch}
Published targets: linux x86_64/aarch64, macOS arm64/x86_64, windows x86_64 (see install.ps1).
Build from source instead:
    git clone https://github.com/${REPO}.git && cd kanbanr
    make install" 2 ;;
esac

# ---- fetching ---------------------------------------------------------------------------
# Timeouts and retries are not garnish: without them a stalled connection hangs with no output at
# all, and GitHub's hosts do throttle, which a couple of spaced retries usually rides out.
#
# A token, when the environment has one, moves api.github.com from 60 requests an hour to 5000. It
# is sent ONLY to api.github.com — the download host needs no credential and must not receive one.
GH_AUTH="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
auth_header() {
    case "$1" in
        https://api.github.com/*)
            if [ -n "$GH_AUTH" ]; then
                printf 'Authorization: Bearer %s' "$GH_AUTH"
            fi
            ;;
    esac
    # ALWAYS 0. Written as `[ -n "$GH_AUTH" ] && printf …` this returns 1 when no token is set,
    # and under `set -e` that aborts the CALLER at `h="$(auth_header "$1")"`. A helper whose job is
    # to produce optional output must not report absence as failure.
    return 0
}

# https ONLY, on the request AND on any redirect (FEAT-089). `-L` follows redirects, and a release
# download IS one — github.com answers a 302 to objects.githubusercontent.com — so without
# --proto-redir a server could send this script to a plaintext host and it would fetch the binary
# from there. A checksum does not rescue that: SHA256SUMS arrives over the same channel, so anyone
# who can rewrite one can rewrite the other. Verification only holds over an authenticated channel.
CURL_SAFE="--proto =https --proto-redir =https"
# wget's equivalent. Older wget has no --https-only, so it is probed rather than assumed: a flag
# that makes wget exit with a usage error would break every install on those systems.
if have wget && wget --https-only --help >/dev/null 2>&1; then
    WGET_SAFE="--https-only"
else
    WGET_SAFE=""
fi

# Refuse before sending, so a bad URL is a clear message rather than a transport error.
require_https() {
    case "$1" in
        https://*) ;;
        *) die "refusing to fetch $1 — this installer uses https only" 3 ;;
    esac
}

if have curl; then
    fetch() {
        require_https "$1"
        h="$(auth_header "$1")"
        if [ -n "$h" ]; then
            # shellcheck disable=SC2086
            curl -fsSL $CURL_SAFE --connect-timeout 10 --max-time 300 --retry 3 --retry-delay 2 -H "$h" "$1"
        else
            # shellcheck disable=SC2086
            curl -fsSL $CURL_SAFE --connect-timeout 10 --max-time 300 --retry 3 --retry-delay 2 "$1"
        fi
    }
    fetch_to() {
        require_https "$1"
        h="$(auth_header "$1")"
        if [ -n "$h" ]; then
            # shellcheck disable=SC2086
            curl -fsSL $CURL_SAFE --connect-timeout 10 --max-time 300 --retry 3 --retry-delay 2 -H "$h" "$1" -o "$2"
        else
            # shellcheck disable=SC2086
            curl -fsSL $CURL_SAFE --connect-timeout 10 --max-time 300 --retry 3 --retry-delay 2 "$1" -o "$2"
        fi
    }
elif have wget; then
    fetch() {
        require_https "$1"
        h="$(auth_header "$1")"
        # shellcheck disable=SC2086
        if [ -n "$h" ]; then wget -qO- $WGET_SAFE --timeout=30 --tries=3 --header="$h" "$1"
        else wget -qO- $WGET_SAFE --timeout=30 --tries=3 "$1"; fi
    }
    fetch_to() {
        require_https "$1"
        h="$(auth_header "$1")"
        # shellcheck disable=SC2086
        if [ -n "$h" ]; then wget -qO "$2" $WGET_SAFE --timeout=30 --tries=3 --header="$h" "$1"
        else wget -qO "$2" $WGET_SAFE --timeout=30 --tries=3 "$1"; fi
    }
else
    die "need curl or wget on PATH" 3
fi

# The first `"tag_name": "…"` of a release payload.
tag_from_json() {
    tr ',' '\n' | grep '"tag_name"' | head -n 1 | sed 's/.*"tag_name": *"//; s/".*//'
}

# `sort -V` is GNU; fall back to a plain reverse sort where it is absent (macOS).
if printf '1\n' | sort -V >/dev/null 2>&1; then SORT_DESC="sort -Vr"; else SORT_DESC="sort -r"; fi

# ---- resolve the release ----------------------------------------------------------------
# THREE sources, tried in order, because each has a failure mode the next covers:
#   1. /releases/latest — right once a STABLE release exists, 404 while every release is a
#      pre-release (as every 0.x is), so it cannot be the only source;
#   2. the release LIST — includes pre-releases, but has been observed returning an empty array
#      while the release was fetchable by tag. A miss here is not authoritative;
#   3. git TAGS — a different endpoint, which answered when the list did not.
# If all three come up empty, the message names the escape hatch instead of guessing.
if [ -z "$VERSION" ]; then
    info "resolving the latest release…"
    VERSION="$(fetch "${API}/releases/latest" 2>/dev/null | tag_from_json)" || true
    [ -n "$VERSION" ] || VERSION="$(fetch "${API}/releases?per_page=1" 2>/dev/null | tag_from_json)" || true
    if [ -z "$VERSION" ]; then
        info "release list empty — falling back to tags"
        for candidate in $(fetch "${API}/tags?per_page=100" 2>/dev/null \
            | tr ',' '\n' | grep '"name"' | sed 's/.*"name": *"//; s/".*//' \
            | grep '^v' | $SORT_DESC); do
            if fetch "${API}/releases/tags/${candidate}" >/dev/null 2>&1; then
                VERSION="$candidate"
                break
            fi
        done
    fi
    [ -n "$VERSION" ] || die "could not resolve a release tag from ${API}
(rate-limited, or nothing published yet). Pin one explicitly:
    ... | sh -s -- --version v0.1.0
and set GH_TOKEN to lift the API rate limit if you are retrying." 3
fi
info "version: ${VERSION}"

ASSET="kanbanr-${VERSION}-${TARGET}.tar.gz"

# ---- install dir ------------------------------------------------------------------------
SUDO=""
if [ -z "$INSTALL_DIR" ]; then
    if [ -w /usr/local/bin ] 2>/dev/null; then
        INSTALL_DIR=/usr/local/bin
    elif [ -t 0 ] && have sudo && [ -d /usr/local/bin ]; then
        INSTALL_DIR=/usr/local/bin
        SUDO="sudo"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi
fi
mkdir -p "$INSTALL_DIR" 2>/dev/null || $SUDO mkdir -p "$INSTALL_DIR"

# ---- download + verify ------------------------------------------------------------------
tmp="$(mktemp -d "${TMPDIR:-/tmp}/kanbanr-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT INT TERM

info "downloading ${ASSET}…"
fetch_to "${DL}/${VERSION}/${ASSET}" "${tmp}/${ASSET}" \
    || die "download failed: ${DL}/${VERSION}/${ASSET}
(check the tag exists and this platform has an asset)" 3

if [ -n "$NO_VERIFY" ]; then
    info "checksum verification SKIPPED (KANBANR_NO_VERIFY)"
elif have sha256sum || have shasum; then
    fetch_to "${DL}/${VERSION}/SHA256SUMS" "${tmp}/SHA256SUMS" \
        || die "could not download SHA256SUMS (set KANBANR_NO_VERIFY=1 to bypass)" 3
    want="$(grep " ${ASSET}\$" "${tmp}/SHA256SUMS" | awk '{print $1}' | head -n 1)"
    [ -n "$want" ] || die "SHA256SUMS carries no entry for ${ASSET}" 4
    if have sha256sum; then
        got="$(sha256sum "${tmp}/${ASSET}" | awk '{print $1}')"
    else
        got="$(shasum -a 256 "${tmp}/${ASSET}" | awk '{print $1}')"
    fi
    [ "$want" = "$got" ] || die "CHECKSUM MISMATCH for ${ASSET}
  expected ${want}
  got      ${got}
Do not use this download." 4
    info "checksum ok"
else
    info "no sha256sum/shasum on PATH — checksum NOT verified"
fi

# ---- install ----------------------------------------------------------------------------
tar -xzf "${tmp}/${ASSET}" -C "$tmp" || die "could not extract ${ASSET}" 3
bin="${tmp}/kanbanr"
[ -f "$bin" ] || bin="$(find "$tmp" -type f -name kanbanr -perm -u+x | head -n 1)"
[ -f "$bin" ] || die "the archive did not contain a 'kanbanr' binary" 3

chmod +x "$bin"
$SUDO mv "$bin" "${INSTALL_DIR}/kanbanr" || die "could not install into ${INSTALL_DIR}" 1
info "installed ${INSTALL_DIR}/kanbanr"

# ---- the skill (FEAT-141) ----------------------------------------------------------------
# The program carries the skill it matches; Claude Code needs it in its skills folder. Written by
# the program, as the user — never with sudo, which would leave it in root's home. A skill folder
# kanbanr did not write (a link to a clone, a plugin's copy) is left alone, and the program says so.
if [ -n "$NO_SKILL" ]; then
    info "Claude Code skill: skipped (--no-skill) — later: kanbanr skill install"
elif have claude; then
    "${INSTALL_DIR}/kanbanr" skill install \
        || info "Claude Code skill: not installed — run: kanbanr skill install"
else
    info "Claude Code skill: no claude command on PATH — once Claude Code is installed, run:"
    info "    kanbanr skill install"
fi

# ---- the editor extension (FEAT-158) ----------------------------------------------------------
# Opt-in: an editor is the user's, and many people with `code` installed don't want an extension
# added to it unasked. The program does the work — download, checksum against SHA256SUMS, install —
# so this script, install.ps1 and self-update share one implementation. As the user, never sudo.
case "$VSCODE" in
    "") editors_found=""
        for e in code codium cursor windsurf; do
            if have "$e"; then editors_found="${editors_found} ${e}"; fi
        done
        if [ -n "$editors_found" ]; then
            info "VS Code extension: found${editors_found} — rerun with --vscode to install it, or later: kanbanr editor install"
        fi ;;
    1|yes|true|all)
        "${INSTALL_DIR}/kanbanr" editor install \
            || info "VS Code extension: not installed — run: kanbanr editor install" ;;
    *)
        "${INSTALL_DIR}/kanbanr" editor install --editor "$VSCODE" \
            || info "VS Code extension: not installed — run: kanbanr editor install --editor ${VSCODE}" ;;
esac

# ---- report -----------------------------------------------------------------------------
printf '\n'
"${INSTALL_DIR}/kanbanr" --version 2>/dev/null || true
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *) printf '\n%s is not on your PATH. Add it:\n    export PATH="%s:$PATH"\n' \
           "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac

# The monitor and the skill are inside the binary, so "next" is Claude Code and nothing to build.
cat <<'NEXT'

Next — open Claude Code in a project you want to track and say:

    set up kanbanr for this project

Or from a terminal, inside the project:

    kanbanr init                   # asks where to keep the board, suggesting <repo>.kanbanr
                                   #   beside it, and records the choice in a .kanbanr marker
    kanbanr serve                  # the monitor on http://127.0.0.1:8080

`serve` needs no --ui-dir: the web monitor is built into this binary. It finds the board from the
.kanbanr marker; to point it elsewhere use `kanbanr serve --data-dir <path>` or KANBANR_DATA_DIR.

Docs: https://kanbanr.startr.trade
NEXT
