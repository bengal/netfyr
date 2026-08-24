#!/bin/bash
# tags: meta policy
# Every test must fail loudly on a missing prerequisite. A silent skip is
# indistinguishable from a pass in CI, so the missing dependency stays hidden.

set -uo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

shopt -s nullglob
tests=("$repo_root"/tests/*.sh)
scripts=("${tests[@]}" "$repo_root"/scripts/*.sh)
shopt -u nullglob

if [ ${#scripts[@]} -eq 0 ]; then
    echo "FAIL: found no scripts to check under $repo_root" >&2
    exit 1
fi

status=0

guard='(command -v|which |\[ +-[xfnedrs] )[^|]*\|\|'

for script in "${scripts[@]}"; do
    name="${script##*/}"

    # Both rules are judged per guard, not per file: one FAIL: elsewhere in the
    # script would otherwise vouch for every silent guard in it.
    mapfile -t lines < "$script"
    for i in "${!lines[@]}"; do
        [[ ${lines[i]} =~ $guard ]] || continue
        window="${lines[i]}"
        # Only a guard whose block opens at the end of the line continues below it;
        # otherwise a neighbouring FAIL: would vouch for a silent guard.
        [[ ${lines[i]} =~ \{[[:space:]]*$ ]] && window+="${lines[i + 1]:-}${lines[i + 2]:-}"

        if [[ $window == *"exit 0"* ]]; then
            echo "FAIL: $name exits 0 from a prerequisite check" >&2
            status=1
        fi
        # `fail ...` is the repo's helper printing the same message.
        [[ $window =~ (^|[\;\{\&\|][[:space:]]*)fail[[:space:]] ]] && continue
        [[ $window == *FAIL:* ]] && continue
        echo "FAIL: $name has a prerequisite check that prints no FAIL message" >&2
        status=1
    done
done

# An undeclared test silently drops out of every filtered run.
for script in "${tests[@]}"; do
    if ! sed -n '1,10p' "$script" | grep -qE '^#[[:space:]]*tags:[[:space:]]*[^[:space:]]'; then
        echo "FAIL: ${script##*/} has no '# tags:' declaration in its first 10 lines" >&2
        status=1
    fi
done

exit "$status"
