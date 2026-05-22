#!/bin/bash
set -euo pipefail

# Promote the inline `lore/` to a separate sibling git repo.
# Usage: ./scripts/split-lore.sh [REMOTE_URL]
#        REMOTE_URL is optional; if omitted, prompts interactively.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
LORE="$WORKSPACE/lore"
REMOTE="${1:-}"

if [ ! -d "$LORE" ]; then
    echo "ERROR: No lore/ directory at $LORE" >&2
    exit 1
fi
if [ -d "$LORE/.git" ]; then
    echo "ERROR: $LORE already has its own .git/ — already a separate sibling" >&2
    exit 1
fi

# --- Resolve remote ---
if [ -z "$REMOTE" ]; then
    echo "Inline lore/ is currently tracked in this workspace's git history."
    echo "Where should the extracted lore repo's origin point?"
    echo ""
    echo "  [1] local-only (no remote; you'll set one up later)"
    echo "  [2] create a new GitHub repo via 'gh repo create' (requires gh CLI)"
    echo "  [3] paste a remote URL I already have"
    echo "  [q] cancel"
    echo ""
    read -p "Choice: " CHOICE
    case "$CHOICE" in
        1) REMOTE="" ;;
        2)
            command -v gh >/dev/null 2>&1 || { echo "gh CLI not installed; aborting" >&2; exit 1; }
            read -p "Target repo name (e.g. you/my-workspace-lore): " GH_NAME
            read -p "Visibility (public/private) [private]: " GH_VIS
            GH_VIS="${GH_VIS:-private}"
            echo "Will run: gh repo create $GH_NAME --$GH_VIS"
            read -p "Proceed? [y/N]: " CONFIRM
            [ "$CONFIRM" = "y" ] || { echo "Aborted."; exit 1; }
            gh repo create "$GH_NAME" --"$GH_VIS" >/dev/null
            REMOTE="git@github.com:$GH_NAME.git"
            echo "Created $GH_NAME"
            ;;
        3)
            read -p "Remote URL: " REMOTE
            ;;
        q|Q) echo "Cancelled."; exit 0 ;;
        *) echo "Invalid choice." >&2; exit 1 ;;
    esac
fi

# --- Confirm before destructive ops ---
echo ""
echo "About to:"
echo "  1. Preserve lore content (cp -r lore lore.split-backup)"
echo "  2. Remove inline lore from this workspace's git history"
echo "  3. Restore lore as a fresh sibling repo"
[ -n "$REMOTE" ] && echo "  4. Set origin = $REMOTE and push"
echo "  5. Update parent .gitignore and household.json"
echo ""
read -p "Proceed? [y/N]: " CONFIRM
[ "$CONFIRM" = "y" ] || { echo "Aborted."; exit 1; }

# --- Execute ---
cd "$WORKSPACE"

echo ""
echo "[1/5] Preserving lore content..."
cp -r lore lore.split-backup

echo "[2/5] Removing from workspace history..."
rm -rf lore
git add -A
git commit -m "split: remove inline lore (becomes a sibling repo)"

echo "[3/5] Restoring as fresh sibling..."
mv lore.split-backup lore
cd lore
git init -b main --quiet
git add -A
git commit -m "initial lore" --quiet
cd "$WORKSPACE"

if [ -n "$REMOTE" ]; then
    echo "[4/5] Wiring remote: $REMOTE"
    git -C lore remote add origin "$REMOTE"
    git -C lore push -u origin main --quiet
fi

echo "[5/5] Updating parent files..."

# .gitignore: remove the `!/lore/` allowlist line (catch-all /* will gitignore lore now)
if grep -q '^!/lore/$' .gitignore 2>/dev/null; then
    grep -v '^!/lore/$' .gitignore > .gitignore.tmp && mv .gitignore.tmp .gitignore
fi

# household.json: if a remote was provided, set the lore entry's url
if [ -n "$REMOTE" ]; then
    REMOTE="$REMOTE" node -e "
        const fs = require('fs');
        const path = './household.json';
        const m = JSON.parse(fs.readFileSync(path, 'utf8'));
        const kb = m.knowledge_base || 'lore';
        const entry = m.repos.find(r => r.name === kb);
        if (entry) entry.url = process.env.REMOTE;
        fs.writeFileSync(path, JSON.stringify(m, null, 2) + '\n');
    "
fi

git add .gitignore household.json
git commit -m "split: lore is now a sibling repo"

echo ""
echo "Done. lore/ is now a sibling git repo."
if [ -n "$REMOTE" ]; then
    echo "  Remote: $REMOTE"
fi
