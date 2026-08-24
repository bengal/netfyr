#!/bin/bash
# tags: build meta

set -uo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$repo_root/Cargo.toml"

command -v cargo >/dev/null || { echo "FAIL: cargo not found" >&2; exit 1; }

metadata="$(cd "$repo_root" && cargo metadata --format-version 1 --no-deps 2>&1)" || {
    echo "FAIL: cargo metadata rejected the workspace manifest:" >&2
    echo "$metadata" >&2
    exit 1
}

# Scoped to the [workspace] section: resolver is ignored anywhere else.
sed -n '/^\[workspace\]/,/^\[/p' "$manifest" \
    | grep -qE '^resolver[[:space:]]*=[[:space:]]*"2"' || {
    echo "FAIL: [workspace] does not set resolver = \"2\"" >&2
    exit 1
}

# cargo build refuses to run while members is empty, so the manifest checks above are
# all there is to assert. Delete this branch once a crate lands.
if [[ "$metadata" == *'"workspace_members":[]'* ]]; then
    echo "note: workspace has no members, checked the manifest only"
    exit 0
fi

(cd "$repo_root" && cargo build) || { echo "FAIL: cargo build failed" >&2; exit 1; }
