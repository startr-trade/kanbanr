#!/bin/sh
# The image reports a serving monitor healthy under docker, through its own HEALTHCHECK (DEF-079).
#
#   scripts/check-image-health.sh IMAGE
#
# The image's health check asks for /healthz as localhost; without a Host line the monitor's host
# check refused it, and docker reported the container unhealthy from v0.5.0. Only the interval and
# start period are overridden, so the image's own command is what runs. The board is a read-only
# temporary folder: the image's anonymous /data volume belongs to root and serve cannot write it.
# Run by release.yml and make ci, from the repository root.
set -eu
image="$1"
board="$(mktemp -d)"
mkdir "$board/projects"
chmod -R a+rX "$board"
name="kanbanr-health-$$"
trap 'docker rm -f "$name" >/dev/null 2>&1 || true; rm -rf "$board"' EXIT
docker run -d --name "$name" --health-interval 2s --health-start-period 0s \
  -v "$board:/data:ro" "$image" >/dev/null

status=""
i=0
while [ "$i" -lt 60 ]; do
  status="$(docker inspect -f '{{.State.Health.Status}}' "$name" 2>/dev/null || true)"
  case "$status" in
    healthy)
      echo "ok: image: a serving monitor reports healthy"
      exit 0
      ;;
    unhealthy) break ;;
  esac
  sleep 1
  i=$((i + 1))
done
echo "::error::the image's health check did not report healthy: $(docker inspect -f '{{json .State.Health}}' "$name" 2>&1 || true)" >&2
exit 1
