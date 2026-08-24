#!/bin/bash
# A hook already present is preserved by pre-commit as <hook>.legacy and still runs.

set -uo pipefail

command -v pre-commit >/dev/null || {
    echo "FAIL: pre-commit not found; install it from https://pre-commit.com" >&2
    exit 1
}

cd "$(dirname "${BASH_SOURCE[0]}")/.." \
    || { echo "FAIL: cannot reach the repository root" >&2; exit 1; }

# Which hook types get installed is declared in .pre-commit-config.yaml, so adding
# one there does not also mean editing a flag here.
pre-commit install
