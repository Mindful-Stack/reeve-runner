#!/bin/bash
set -euo pipefail

# Substitute the workspace placeholder name into household.json + CLAUDE.md.
# Usage: ./scripts/rename.sh <new-name>

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
NEW_NAME="${1:-}"

if [ -z "$NEW_NAME" ]; then
    echo "Usage: $0 <new-name>" >&2
    echo "Example: $0 my-cool-project" >&2
    exit 1
fi

# Reject names with characters that would break sed substitutions or be invalid
# as identifiers / filenames. Allowed: alphanumerics, hyphens, underscores.
if ! [[ "$NEW_NAME" =~ ^[A-Za-z0-9_-]+$ ]]; then
    echo "ERROR: workspace name must match [A-Za-z0-9_-]+ (got: '$NEW_NAME')" >&2
    exit 1
fi

cd "$WORKSPACE"

OLD_NAME=$(node -e "console.log(require('./household.json').workspace)")

if [ "$OLD_NAME" = "$NEW_NAME" ]; then
    echo "Workspace name is already '$NEW_NAME'; nothing to do."
    exit 0
fi

# household.json: rewrite the `workspace` field AND the matching repos[] entry's name.
# Pass NEW_NAME via env (process.env) to avoid shell-injection via the Node string interpolation.
NEW_NAME="$NEW_NAME" node -e "
    const fs = require('fs');
    const path = './household.json';
    const m = JSON.parse(fs.readFileSync(path, 'utf8'));
    const old = m.workspace;
    const next = process.env.NEW_NAME;
    const entry = m.repos.find(r => r.name === old);
    m.workspace = next;
    if (entry) entry.name = next;
    fs.writeFileSync(path, JSON.stringify(m, null, 2) + '\n');
"

# CLAUDE.md: substitute the old name. The regex character class above guarantees
# NEW_NAME has no sed metacharacters; OLD_NAME comes from the manifest, which
# the JSON manifest validator constrains. Still, prefer a non-greedy literal.
if [ -f CLAUDE.md ]; then
    # Use | as delimiter to avoid conflicts with / in URLs (defensive).
    sed -i "s|$OLD_NAME|$NEW_NAME|g" CLAUDE.md
fi

echo "Renamed workspace: $OLD_NAME → $NEW_NAME"
echo "  Updated: household.json"
[ -f CLAUDE.md ] && echo "  Updated: CLAUDE.md"
echo ""
echo "Review the diff and commit:"
echo "  git diff household.json CLAUDE.md"
echo "  git add household.json CLAUDE.md && git commit -m 'rename: workspace → $NEW_NAME'"
