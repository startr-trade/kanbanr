#!/bin/sh
# Run locally every CI check that can run off GitHub (FEAT-131). See scripts/ci_local.py.
exec python3 "$(dirname "$0")/ci_local.py" "$@"
