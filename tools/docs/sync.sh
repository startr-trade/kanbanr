#!/usr/bin/env bash
# Copy the root files the book also publishes into docs/src, so there is ONE source for each.
#
# The changelog is canonically at the repository root (where every tool and every reader expects
# it) and is also a chapter of the book. Rather than maintain two, this copies it in — and CI runs
# the same script and fails if the result differs from what is committed, so the chapter cannot
# quietly go stale.
set -euo pipefail
cd "$(dirname "$0")/../.."

cp CHANGELOG.md docs/src/project/changelog.md
echo "synced: CHANGELOG.md -> docs/src/project/changelog.md"
