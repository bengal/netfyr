#!/bin/bash
# tags: meta docs
# Each file is matched against a marker only a genuine version of it would carry,
# so that a placeholder does not pass.

set -uo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
status=0

check() {
    local file="$1" pattern="$2" what="$3"
    if [ ! -s "$repo_root/$file" ]; then
        echo "FAIL: $file is missing or empty" >&2
        status=1
    elif ! grep -qE "$pattern" "$repo_root/$file"; then
        echo "FAIL: $file does not $what" >&2
        status=1
    fi
}

check README.md 'cargo build' "explain how to build"
check LICENSE 'Permission is hereby granted' "carry license terms"
check CHANGELOG.md '^## \[Unreleased\]' "have an Unreleased section"
check CONTRIBUTING.md '# tags:' "document the test tag convention"

exit "$status"
