#!/usr/bin/env bash
# Fail if any source file in a context crate imports a deep path from another
# context crate's internals (`use reeve_X::submodule::...`).
#
# Acceptable:
#   use reeve_card::Card;
#   use reeve_card::WorkspaceLifecycle;
# Forbidden:
#   use reeve_card::repo::SqliteCardRepo;
#   use reeve_card::repo::*;
#   use reeve_workspace::lifecycle::LiveWorkspaceLifecycle;
#
# reeved (the composition root) is exempt by design — it does need to construct
# concrete adapters via factory functions exposed at lib level. Even reeved
# should not reach past mod.rs of context crates, though.

set -euo pipefail

# Crates whose internals are off-limits to other context crates.
contexts=(card workspace household)

# Build the regex alternation: card|workspace|household
alt="$(IFS='|'; echo "${contexts[*]}")"

violations=$(git grep -nE "use reeve_(${alt})::[a-z_]+::" \
    -- 'crates/reeve-card/src/*' 'crates/reeve-workspace/src/*' 'crates/reeve-household/src/*' \
    || true)

if [[ -n "$violations" ]]; then
    echo "Context-isolation violations detected:" >&2
    echo "$violations" >&2
    echo >&2
    echo "Cross-context imports must go through the crate's lib.rs facade." >&2
    echo "Move the path past mod.rs or add the needed type to lib.rs's pub use." >&2
    exit 1
fi

echo "Context isolation OK."
