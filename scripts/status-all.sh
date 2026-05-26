#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$WORKSPACE/household.json"

if [ ! -f "$MANIFEST" ]; then
    echo "ERROR: No household.json. Run from inside a witan-household workspace." >&2
    exit 1
fi

SIBLINGS=$(node -e "
    const m = require('$MANIFEST');
    m.repos.filter(r => r.name !== m.meta_repo).forEach(r => console.log(r.name));
")

for name in $SIBLINGS; do
    DIR="$WORKSPACE/$name"
    if [ ! -d "$DIR/.git" ]; then
        printf "%-30s (no .git/)\n" "$name"
        continue
    fi
    BRANCH=$(git -C "$DIR" symbolic-ref --short HEAD 2>/dev/null || echo "?")
    DIRTY=$(git -C "$DIR" status --porcelain | wc -l | xargs)
    AHEAD=$(git -C "$DIR" rev-list --count "@{u}..HEAD" 2>/dev/null || echo "0")
    BEHIND=$(git -C "$DIR" rev-list --count "HEAD..@{u}" 2>/dev/null || echo "0")
    printf "%-30s %s  %d dirty  %d ahead  %d behind\n" "$name" "$BRANCH" "$DIRTY" "$AHEAD" "$BEHIND"
done
