#!/bin/bash
set -euo pipefail

# Bootstrap the witan-household: clone declared sibling repos.
# Reads the manifest at ./household.json (Node parser; no jq dependency).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$WORKSPACE/household.json"

# --- Flag parsing ---
TAG_FILTER=""
REPOS_FILTER=""

for arg in "$@"; do
    case "$arg" in
        --tag=*)    TAG_FILTER="${arg#*=}" ;;
        --repos=*)  REPOS_FILTER="${arg#*=}" ;;
        -h|--help)
            cat <<EOF
Usage: $0 [--tag=foo] [--repos=name1,name2]

No args: clone every sibling repo that has a 'url' field in household.json,
         excluding the workspace entry itself.

Flags:
  --tag=foo       Only clone repos tagged 'foo'.
  --repos=a,b,c   Only clone the named repos.
EOF
            exit 0
            ;;
        *)
            echo "Unknown flag: $arg" >&2
            exit 2
            ;;
    esac
done

# --- Prereq check ---
echo "[1/3] Checking prerequisites..."
MISSING=""
for tool in git node; do
    if ! command -v "$tool" &>/dev/null; then
        MISSING="$MISSING $tool"
    fi
done
if [ -n "$MISSING" ]; then
    echo "ERROR: Missing required tools:$MISSING" >&2
    exit 1
fi
if [ ! -f "$MANIFEST" ]; then
    echo "ERROR: No household.json at $MANIFEST. Run this from inside a witan-household workspace." >&2
    exit 1
fi
echo "  OK"

# --- Parse manifest (Node, no jq dep) ---
echo ""
echo "[2/3] Selecting repos..."

SELECTED=$(TAG_FILTER="$TAG_FILTER" REPOS_FILTER="$REPOS_FILTER" node -e "
    const m = require('$MANIFEST');
    const tag = process.env.TAG_FILTER;
    const repos = process.env.REPOS_FILTER.split(',').filter(Boolean);
    let result = m.repos.filter(r => r.name !== m.meta_repo && r.url);
    if (tag)          result = result.filter(r => (r.tags || []).includes(tag));
    if (repos.length) result = result.filter(r => repos.includes(r.name));
    result.forEach(r => console.log(r.name + ' ' + r.url));
")

if [ -z "$SELECTED" ]; then
    echo "  No siblings selected (manifest may declare none with 'url', or filters excluded all)."
    exit 0
fi

COUNT=$(echo "$SELECTED" | wc -l | xargs)
echo "  $COUNT repo(s) selected."

# --- Clone ---
echo ""
echo "[3/3] Cloning..."
SUCCESS=0
SKIPPED=0
FAILED=0
while IFS=' ' read -r name url; do
    if [ -d "$WORKSPACE/$name" ]; then
        echo "  SKIP $name (already exists at $WORKSPACE/$name)"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi
    echo "  CLONE $name from $url"
    if git -C "$WORKSPACE" clone --quiet "$url" "$name"; then
        SUCCESS=$((SUCCESS + 1))
    else
        echo "    FAILED: git clone exited non-zero" >&2
        FAILED=$((FAILED + 1))
    fi
done <<< "$SELECTED"

echo ""
echo "Done. $SUCCESS cloned, $SKIPPED skipped, $FAILED failed."
[ $FAILED -eq 0 ] || exit 1
