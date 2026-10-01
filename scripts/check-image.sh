#!/bin/sh
# The image's kanbanr is the released program: it names the commit and date it was built from
# (FEAT-144), and it carries the repository's skill (FEAT-145).
#
#   scripts/check-image.sh IMAGE [COMMIT]     # COMMIT defaults to HEAD
#
# The release archives are checked for this; the image is the same binary built a second way, in a
# context with no .git and only what the Dockerfile copies, so it is checked too — before it is
# pushed. Run by release.yml and make ci, from the repository root.
set -eu
image="$1"
want="$(git rev-parse --short=12 "${2:-HEAD}")"
date="$(git log -1 --format=%cd --date=short "${2:-HEAD}")"
v="$(docker run --rm --entrypoint kanbanr "$image" --version)"
echo "$v"
case "$v" in
  *unknown*) echo "::error::the image's kanbanr cannot say which build it is: $v" >&2; exit 1 ;;
esac
echo "$v" | grep -q "$want" \
  || { echo "::error::the image's kanbanr does not name commit $want: $v" >&2; exit 1; }
echo "$v" | grep -q "built $date" \
  || { echo "::error::the image's kanbanr does not name the commit's date $date: $v" >&2; exit 1; }

# The skill it installs is the repository's, file for file (the stamp kanbanr adds aside).
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
docker run --rm -e CLAUDE_CONFIG_DIR=/tmp/claude --entrypoint sh "$image" -c \
  'kanbanr skill install >&2 && cd /tmp/claude/skills/kanbanr && tar cf - --exclude=./.kanbanr-skill .' \
  | tar xf - -C "$out" \
  || { echo "::error::the image's kanbanr carries no skill" >&2; exit 1; }
diff -r -x '.*' -x __pycache__ skill/kanbanr "$out" \
  || { echo "::error::the image's kanbanr carries a different skill than skill/kanbanr" >&2; exit 1; }
echo "the image's kanbanr carries the repository's skill"
