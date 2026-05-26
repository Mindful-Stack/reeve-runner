#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$WORKSPACE/household.json"

if [ ! -f "$MANIFEST" ]; then
    echo "ERROR: No household.json. Run from inside a witan-household workspace." >&2
    exit 1
fi

# List non-workspace repo names from manifest.
SIBLINGS=$(node -e "
    const m = require('$MANIFEST');
    m.repos.filter(r => r.name !== m.meta_repo).forEach(r => console.log(r.name));
")

for name in $SIBLINGS; do
    DIR="$WORKSPACE/$name"
    if [ ! -d "$DIR/.git" ]; then
        echo "[SKIP] $name (no .git/)"
        continue
    fi
    echo "[FETCH] $name"
    git -C "$DIR" fetch --prune --quiet
    BRANCH=$(git -C "$DIR" symbolic-ref --short HEAD 2>/dev/null || echo "")
    if [ "$BRANCH" = "main" ] || [ "$BRANCH" = "master" ]; then
        if [ -z "$(git -C "$DIR" status --porcelain)" ]; then
            git -C "$DIR" pull --ff-only --quiet && echo "  pulled" || echo "  ff-pull failed"
        else
            echo "  dirty; skipping pull"
        fi
    else
        echo "  on '$BRANCH'; not pulling"
    fi
done
