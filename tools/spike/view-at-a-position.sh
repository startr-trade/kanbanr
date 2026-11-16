#!/usr/bin/env bash
# SPK-001's proof (R-5): one written doc and the generated `docs` view at main, at a branch and at
# an uncommitted position, each with its position text and view ID, and a second read at each
# position that repeats the bytes and the ID. Run from the repository root. Exits non-zero when the
# proof is not ok. Everything happens in scratch repositories under $TMPDIR: nothing is written
# into any repository it is given, and the real board is never touched.
#
# SPIKE_BIN=<path to the spike_view example> skips the build; otherwise it is built and run with
# cargo from api/.
set -euo pipefail

scratch="$(mktemp -d "${TMPDIR:-/tmp}/spk-001-proof.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

# A git that no global setting bends: identity set, no signing, no line-ending conversion.
export GIT_CONFIG_GLOBAL="$scratch/empty-config" GIT_CONFIG_SYSTEM="$scratch/empty-config"
: > "$scratch/empty-config"
g() { git -c user.name=Proof -c user.email=proof@example.invalid -c commit.gpgsign=false \
        -c core.autocrlf=false "$@"; }

spike() {
  if [ -n "${SPIKE_BIN:-}" ]; then "$SPIKE_BIN" "$@"
  else (cd api && cargo run -q -p kanbanr-core --example spike_view -- "$@"); fi
}

repo="$scratch/repo"; board="$scratch/board"
mkdir -p "$repo/docs" "$board"

# A stand-in board: a second repository with one commit.
( cd "$board" && g init -q --initial-branch=main && echo board > board.txt \
  && g add -A && g commit -q -m "board" )

# main: two docs.
( cd "$repo" && g init -q --initial-branch=main \
  && printf '# Overview\n\nThe overview.\n' > docs/overview.md \
  && printf '# Guide\n\nThe guide.\n' > docs/guide.md \
  && g add -A && g commit -q -m "main" \
  && g checkout -q -b feat/SPK-001-proof \
  && printf '# Guide\n\nThe guide, edited on the branch.\n' > docs/guide.md \
  && g commit -q -am "branch edit" \
  && printf '# Overview\n\nThe overview, edited and uncommitted.\n\n```mermaid\nflowchart LR\n  A --> B\n```\n' > docs/overview.md \
  && printf '# New\n\nUntracked.\n' > docs/new.md \
  && rm docs/guide.md )

echo "scratch repository: main, branch feat/SPK-001-proof (checked out) with an uncommitted edit to"
echo "docs/overview.md, an untracked docs/new.md and docs/guide.md deleted; written doc: docs/guide.md"
echo

common=("$repo" --doc docs/guide.md --board "$board")
spike "${common[@]}" --at main
spike "${common[@]}" --at feat/SPK-001-proof
spike "${common[@]}" --live
echo "######## the three together"
spike "${common[@]}" --at main --at feat/SPK-001-proof --live | sed -n '/== proof ==/,$p'
