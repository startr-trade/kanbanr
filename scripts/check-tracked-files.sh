#!/bin/sh
# Refuse tracked merge leftovers: a file ending in .orig or .rej is a merge tool's backup, not
# source (MNT-006). Looks at tracked files only, so an untracked or ignored leftover passes.
# Run from anywhere inside the repository.
set -eu

cd "$(git rev-parse --show-toplevel)"

found=$(git ls-files -z | tr '\0' '\n' | grep -E '\.(orig|rej)$' || true)
if [ -n "$found" ]; then
    printf '%s\n' "$found" | while IFS= read -r path; do
        echo "tracked merge leftover: $path"
    done
    echo "Remove them with git rm; a merge tool made them."
    exit 1
fi
echo "no tracked *.orig or *.rej files"
